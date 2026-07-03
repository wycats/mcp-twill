use mcp_twill::{
    CliMcpServer, CommandOutput, CommandRegistry, CommandSpec, PermissionEffect, PermissionSpec,
};
use mcp_twill_host::RuntimeHost;
use serde_json::json;

fn registry() -> CommandRegistry {
    CommandRegistry::new("host-test", "Runtime host test server").register(
        CommandSpec::new(["issues", "list"], "List issues", "List issues").with_permission(
            PermissionSpec::new(PermissionEffect::Read, "issues", "Reads issues"),
        ),
        |_context| async { Ok(CommandOutput::structured(json!([{ "id": 1 }]))) },
    )
}

#[test]
fn host_layers_process_facts_onto_the_server_identity() -> anyhow::Result<()> {
    let server = CliMcpServer::new(registry())?;
    let bare = server.runtime_identity();
    let host = RuntimeHost::new(&server);
    let hosted = host.identity();

    // Everything the core reports survives unchanged.
    assert_eq!(hosted.server_name, bare.server_name);
    assert_eq!(hosted.server_version, bare.server_version);
    assert_eq!(hosted.catalog_hash, bare.catalog_hash);
    assert_eq!(hosted.run_schema_hash, bare.run_schema_hash);
    assert_eq!(hosted.help_schema_hash, bare.help_schema_hash);

    // The host adds what only a process can know.
    assert_eq!(hosted.process_id, Some(std::process::id()));
    assert!(hosted.started_at_unix_ms.is_some());

    // Follow-up work, deliberately absent: replacement detection.
    assert_eq!(hosted.executable_hash, None);
    Ok(())
}

#[test]
fn retry_decisions_flow_from_the_plan_effect() -> anyhow::Result<()> {
    use mcp_twill::{RunMode, RunRequest};
    use mcp_twill_host::{Idempotency, RetryDecision};

    let server = CliMcpServer::new(registry())?;
    let host = RuntimeHost::new(&server);

    let plan = server.registry().build_plan(&RunRequest {
        command: "issues list".to_string(),
        args: Default::default(),
        stdin: None,
        output: None,
        mode: RunMode::DryRun,
        approval: None,
        dry_run: true,
    })?;

    // A read-lane plan retries with no idempotency declaration.
    assert_eq!(
        host.retry_decision(&plan.effect, Idempotency::None),
        RetryDecision::Retry
    );
    Ok(())
}
