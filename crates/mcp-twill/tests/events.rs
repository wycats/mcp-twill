use std::sync::Arc;

use mcp_twill::{
    ArgSpec, CliMcpServer, CommandOutput, CommandRegistry, CommandSpec, InMemoryEventSink,
    PermissionEffect, PermissionSpec, ResponseStatus, RunRequest,
};
use rmcp::{
    ClientHandler, ServiceExt, handler::client::progress::ProgressDispatcher,
    model::CallToolRequestParams,
};
use serde_json::{Value, json};

fn json_object<T: serde::Serialize>(value: T) -> anyhow::Result<serde_json::Map<String, Value>> {
    match serde_json::to_value(value)? {
        Value::Object(map) => Ok(map),
        other => anyhow::bail!("expected JSON object, got {other:?}"),
    }
}

fn request(command: &str, args: serde_json::Value) -> anyhow::Result<RunRequest> {
    Ok(RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args)?,
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    })
}

fn registry() -> CommandRegistry {
    CommandRegistry::new("event-test", "Event sink test server")
        .register(
            CommandSpec::new(["issues", "list"], "List issues", "List issues").with_permission(
                PermissionSpec::new(PermissionEffect::Read, "issues", "Reads issues"),
            ),
            |_context| async { Ok(CommandOutput::structured(json!([{ "id": 1 }]))) },
        )
        .register(
            CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
                .with_arg(ArgSpec::string("title", "Issue title"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "issues",
                    "Creates issues",
                )),
            |_context| async { Ok(CommandOutput::structured(json!({ "id": 2 }))) },
        )
}

struct TestClient {
    progress: ProgressDispatcher,
}

impl TestClient {
    fn new() -> Self {
        Self {
            progress: ProgressDispatcher::new(),
        }
    }
}

impl ClientHandler for TestClient {
    async fn on_progress(
        &self,
        params: rmcp::model::ProgressNotificationParam,
        _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.progress.handle_notification(params).await;
    }
}

async fn serve_with_sink(
    registry: CommandRegistry,
) -> anyhow::Result<(
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    Arc<InMemoryEventSink>,
)> {
    let sink = Arc::new(InMemoryEventSink::new());
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(registry)?.with_event_sink(sink.clone());
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient::new().serve(client_transport).await?;
    Ok((client, sink))
}

#[tokio::test]
async fn successful_dispatch_records_an_event() -> anyhow::Result<()> {
    let (client, sink) = serve_with_sink(registry()).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(json_object(request("issues list", json!({}))?)?),
        )
        .await?;
    assert_eq!(result.is_error, Some(false));

    let events = sink.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.status, ResponseStatus::Ok);
    assert_eq!(event.operation_id.as_deref(), Some("issues.list"));
    assert_eq!(
        event.command.as_deref(),
        Some(["issues", "list"].map(String::from).as_slice())
    );
    assert_eq!(event.effects.len(), 1);
    assert!(event.diagnostics.is_empty());
    assert!(event.id.starts_with("event-"));
    assert!(event.timestamp_unix_ms > 0);

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn events_carry_the_runtime_identity() -> anyhow::Result<()> {
    let registry = registry();
    let expected = registry.runtime_identity();
    let (client, sink) = serve_with_sink(registry).await?;

    client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(json_object(request("issues list", json!({}))?)?),
        )
        .await?;
    // A failing call carries the same identity: the recording site is shared.
    client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(json_object(request("issues nonexistent", json!({}))?)?),
        )
        .await?;

    let events = sink.events();
    assert_eq!(events.len(), 2);
    for event in &events {
        let runtime = event
            .runtime
            .as_ref()
            .expect("events record the serving runtime identity");
        assert_eq!(runtime.catalog_hash, expected.catalog_hash);
        assert_eq!(runtime.run_schema_hash, expected.run_schema_hash);
        assert_eq!(runtime.help_schema_hash, expected.help_schema_hash);
        assert!(
            runtime.server_version.is_some(),
            "the adapter layers the crate version onto the registry identity"
        );
    }

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn planning_failure_records_an_event_with_diagnostics() -> anyhow::Result<()> {
    let (client, sink) = serve_with_sink(registry()).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(json_object(request("issues nonexistent", json!({}))?)?),
        )
        .await?;
    assert_eq!(result.is_error, Some(true));

    let events = sink.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_ne!(event.status, ResponseStatus::Ok);
    assert_eq!(event.operation_id, None, "planning never produced a plan");
    assert!(
        !event.diagnostics.is_empty(),
        "planning failures carry diagnostics"
    );

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn permission_required_records_an_event() -> anyhow::Result<()> {
    let (client, sink) = serve_with_sink(registry()).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("run-write").with_arguments(json_object(request(
                "issues create --title $args.title",
                json!({ "title": "T" }),
            )?)?),
        )
        .await?;
    assert_eq!(result.is_error, Some(true));

    let events = sink.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.status, ResponseStatus::PermissionRequired);
    assert_eq!(event.operation_id.as_deref(), Some("issues.create"));

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn unparseable_run_request_records_an_invalid_input_event() -> anyhow::Result<()> {
    let (client, sink) = serve_with_sink(registry()).await?;

    // `command` must be a string; a number fails RunRequest deserialization
    // before planning starts.
    let result = client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(json_object(json!({ "command": 42 }))?),
        )
        .await;
    assert!(
        result.is_err(),
        "parse failure surfaces as a protocol error"
    );

    let events = sink.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.status, ResponseStatus::InvalidInput);
    assert_eq!(event.operation_id, None);
    assert!(!event.diagnostics.is_empty(), "carries the parse message");

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn each_call_records_its_own_event() -> anyhow::Result<()> {
    let (client, sink) = serve_with_sink(registry()).await?;

    for _ in 0..2 {
        client
            .call_tool(
                CallToolRequestParams::new("run")
                    .with_arguments(json_object(request("issues list", json!({}))?)?),
            )
            .await?;
    }

    let events = sink.events();
    assert_eq!(events.len(), 2);
    assert_ne!(events[0].id, events[1].id, "event ids are unique");

    client.cancel().await?;
    Ok(())
}
