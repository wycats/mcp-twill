//! Public acceptance coverage for the stateless MCP serving surface (RFC 0020).

use std::future::poll_fn;

use bytes::Bytes;
use http::{Method, Request, StatusCode, header::CONTENT_TYPE};
use http_body::Body;
use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, CliMcpServer, CommandRegistry, CommandSpec,
    DynamicCommandFailure, FrameworkHelpProjection, McpProtocolTarget, NativeConfirmationRoute,
    NativeToolSurface, OutputContract,
};
use serde_json::{Value, json};
use tower_service::Service;

const PROTOCOL_VERSION: &str = "2026-07-28";

fn public_http_service() -> mcp_twill::StatelessMcpHttpService {
    let spec = CommandSpec::new(["work"], "Work", "Perform work").with_output(OutputContract {
        application: Some(ApplicationResultContract::new(json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
            "required": ["ok"],
            "additionalProperties": false
        }))),
        ..OutputContract::default()
    });
    let registry = CommandRegistry::new("stateless-acceptance", "Stateless acceptance")
        .register_dynamic(spec, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({ "ok": true })))
        });
    let surface = NativeToolSurface::builder("stateless-acceptance")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("work", "work")
        .build(&registry, McpProtocolTarget::V2026_07_28)
        .unwrap();
    CliMcpServer::builder(registry)
        .surface(surface)
        .build()
        .unwrap()
        .into_stateless_service()
        .unwrap()
        .into_http_service()
}

fn request(id: u64, method: &str, routed_name: Option<&str>, params: Value) -> Request<Bytes> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .header(CONTENT_TYPE, "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .header("Mcp-Method", method);
    if let Some(routed_name) = routed_name {
        builder = builder.header("Mcp-Name", routed_name);
    }
    builder
        .body(Bytes::from(
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }))
            .unwrap(),
        ))
        .unwrap()
}

fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {
            "name": "public-acceptance",
            "version": "1"
        },
        "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION
    })
}

async fn response_value(
    response: http::Response<mcp_twill::StatelessMcpHttpBody>,
) -> (StatusCode, Value) {
    let status = response.status();
    let mut body = Box::pin(response.into_body());
    let mut bytes = Vec::new();
    while let Some(frame) = poll_fn(|context| body.as_mut().poll_frame(context)).await {
        if let Ok(data) = frame.unwrap().into_data() {
            bytes.extend_from_slice(&data);
        }
    }
    let bytes = bytes
        .strip_prefix(b"event: message\ndata: ")
        .and_then(|bytes| bytes.strip_suffix(b"\n\n"))
        .unwrap_or(&bytes);
    (status, serde_json::from_slice(bytes).unwrap())
}

#[tokio::test]
async fn public_stateless_http_applies_framework_cache_policy_to_every_cacheable_result() {
    let mut service = public_http_service();
    let cases = [
        ("tools/list", None, json!({ "_meta": meta() })),
        ("resources/list", None, json!({ "_meta": meta() })),
        ("prompts/list", None, json!({ "_meta": meta() })),
        (
            "resources/read",
            Some("cli://catalog"),
            json!({ "_meta": meta(), "uri": "cli://catalog" }),
        ),
    ];

    for (index, (method, routed_name, params)) in cases.into_iter().enumerate() {
        let response = service
            .call(request(index as u64 + 1, method, routed_name, params))
            .await
            .unwrap();
        let (status, value) = response_value(response).await;
        assert_eq!(status, StatusCode::OK, "{method}: {value}");
        assert_eq!(value["result"]["cacheScope"], "private", "{method}");
        assert_eq!(value["result"]["ttlMs"], 0, "{method}");
    }
}

#[tokio::test]
async fn public_stateless_http_accepts_argument_free_calls_without_an_arguments_member() {
    let mut service = public_http_service();
    let response = service
        .call(request(
            1,
            "tools/call",
            Some("work"),
            json!({
                "_meta": meta(),
                "name": "work"
            }),
        ))
        .await
        .unwrap();
    let (status, value) = response_value(response).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["result"]["structuredContent"], json!({ "ok": true }));
}

#[tokio::test]
async fn public_stateless_http_rejects_malformed_extension_capabilities_at_preflight() {
    let mut service = public_http_service();
    for (id, method, routed_name, extensions) in [
        (1, "tools/list", None, json!({ "example": {} })),
        (
            2,
            "tasks/get",
            Some("task-example"),
            json!({ "io.modelcontextprotocol/tasks": { "version": 1 } }),
        ),
    ] {
        let mut metadata = meta();
        metadata["io.modelcontextprotocol/clientCapabilities"] =
            json!({ "extensions": extensions });
        let response = service
            .call(request(
                id,
                method,
                routed_name,
                json!({
                    "_meta": metadata,
                    "taskId": "task-example"
                }),
            ))
            .await
            .unwrap();
        let (status, value) = response_value(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method}: {value}");
        assert_eq!(value["error"]["code"], -32602, "{method}: {value}");
    }
}

#[tokio::test]
async fn public_stateless_http_rejects_duplicate_standardized_metadata_keys() {
    let mut service = public_http_service();
    for body in [
        br#"{
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/list",
            "params":{"_meta":{
                "io.modelcontextprotocol/clientCapabilities":{},
                "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/protocolVersion":"2099-01-01"
            }}
        }"#
        .as_slice(),
        br#"{
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/list",
            "params":{"_meta":{
                "io.modelcontextprotocol/clientCapabilities":{},
                "io.modelcontextprotocol/clientCapabilities":{"extensions":{}},
                "io.modelcontextprotocol/protocolVersion":"2026-07-28"
            }}
        }"#
        .as_slice(),
    ] {
        let response = service
            .call(
                Request::builder()
                    .method(Method::POST)
                    .header(CONTENT_TYPE, "application/json")
                    .header("Accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", PROTOCOL_VERSION)
                    .header("Mcp-Method", "tools/list")
                    .body(Bytes::copy_from_slice(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, value) = response_value(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert_eq!(value["error"]["code"], -32602, "{value}");
    }
}
