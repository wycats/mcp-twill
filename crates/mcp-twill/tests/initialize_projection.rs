use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, CliMcpServer, CliMcpServerConfig,
    CommandRegistry, CommandSpec, DynamicCommandFailure, FrameworkHelpProjection,
    McpProtocolTarget, NativeConfirmationRoute, NativeMcpInitializeProjection,
    NativeMcpServerIdentity, NativeToolSurface, OutputContract,
};
use rmcp::{
    ClientHandler, ServerHandler, ServiceExt,
    model::{ErrorCode, GetPromptRequestParams, ReadResourceRequestParams},
    service::ServiceError,
};
use serde_json::json;

struct NullClient;

impl ClientHandler for NullClient {}

fn assert_method_not_found(error: ServiceError) {
    match error {
        ServiceError::McpError(error) => assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND),
        other => panic!("expected MCP method-not-found error, got {other:?}"),
    }
}

fn native_server(projection: NativeMcpInitializeProjection) -> anyhow::Result<CliMcpServer> {
    let work = CommandSpec::new(["work"], "Work", "Perform work").with_output(OutputContract {
        application: Some(ApplicationResultContract::new(json!({
            "type": "object",
            "properties": {
                "worked": { "type": "boolean" }
            },
            "required": ["worked"],
            "additionalProperties": false
        }))),
        ..OutputContract::default()
    });
    let registry = CommandRegistry::new("projection-test", "Projection test server")
        .register_dynamic(work, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "worked": true
            })))
        });
    let surface = NativeToolSurface::builder("projection-test")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("work", "work")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    Ok(CliMcpServer::builder(registry)
        .config(CliMcpServerConfig::default())
        .surface(surface)
        .native_mcp_initialize_projection(projection)
        .build()?)
}

#[test]
fn default_initialize_projection_preserves_twill_capabilities_and_identity() -> anyhow::Result<()> {
    let server = native_server(NativeMcpInitializeProjection::default())?;
    let info = server.get_info();

    assert!(info.capabilities.tools.is_some());
    assert!(info.capabilities.resources.is_some());
    assert!(info.capabilities.prompts.is_some());
    assert!(info.capabilities.experimental.is_none());
    assert_eq!(info.server_info.name, "projection-test");
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.server_info.title.as_deref(), Some("MCP Twill"));
    assert_eq!(
        info.server_info.description.as_deref(),
        Some("Projection test server")
    );
    Ok(())
}

#[test]
fn initialize_projection_can_preserve_long_existing_server_instructions() -> anyhow::Result<()> {
    let instructions = "Existing server guidance. ".repeat(80);
    assert!(instructions.chars().count() > 1_024);
    let server = native_server(
        NativeMcpInitializeProjection::default().with_instructions(instructions.clone()),
    )?;

    assert_eq!(
        server.get_info().instructions.as_deref(),
        Some(instructions.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn native_initialize_projection_can_preserve_a_tools_only_server_contract()
-> anyhow::Result<()> {
    let projection = NativeMcpInitializeProjection::default()
        .with_resources(false)
        .with_prompts(false)
        .with_experimental_capability("codex/sandbox-state-meta", Default::default())
        .with_server_identity(NativeMcpServerIdentity::new("rmcp", "1.7.0"));
    let server = native_server(projection)?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = NullClient.serve(client_transport).await?;

    let info = client.peer_info().expect("server initialize info");
    assert!(info.capabilities.tools.is_some());
    assert!(info.capabilities.resources.is_none());
    assert!(info.capabilities.prompts.is_none());
    assert_eq!(
        info.capabilities
            .experimental
            .as_ref()
            .and_then(|capabilities| capabilities.get("codex/sandbox-state-meta")),
        Some(&Default::default())
    );
    assert_eq!(info.server_info.name, "rmcp");
    assert_eq!(info.server_info.version, "1.7.0");
    assert!(info.server_info.title.is_none());
    assert!(info.server_info.description.is_none());

    assert_method_not_found(client.list_resources(Default::default()).await.unwrap_err());
    assert_method_not_found(
        client
            .read_resource(ReadResourceRequestParams::new("cli://catalog"))
            .await
            .unwrap_err(),
    );
    assert_method_not_found(client.list_prompts(Default::default()).await.unwrap_err());
    assert_method_not_found(
        client
            .get_prompt(GetPromptRequestParams::new("getting_started"))
            .await
            .unwrap_err(),
    );
    assert_eq!(client.list_tools(Default::default()).await?.tools.len(), 1);

    client.cancel().await?;
    server_handle.await??;
    Ok(())
}
