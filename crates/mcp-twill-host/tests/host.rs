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
fn retry_decisions_flow_from_the_plan() -> anyhow::Result<()> {
    use mcp_twill::{RunMode, RunRequest};
    use mcp_twill_host::RetryDecision;

    let registry = registry()
        .register(
            CommandSpec::new(["issues", "close"], "Close an issue", "Close an issue")
                .with_arg(mcp_twill::ArgSpec::string("id", "Issue id"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "issues",
                    "Closes issues",
                ))
                .idempotent(),
            |_context| async { Ok(CommandOutput::structured(json!({ "closed": true }))) },
        )
        .register(
            CommandSpec::new(["issues", "create"], "Create an issue", "Create an issue")
                .with_arg(mcp_twill::ArgSpec::string("title", "Issue title"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "issues",
                    "Creates issues",
                )),
            |_context| async { Ok(CommandOutput::structured(json!({ "id": 2 }))) },
        );
    let server = CliMcpServer::new(registry)?;
    let host = RuntimeHost::new(&server);

    let plan_for = |command: &str, args: serde_json::Value| {
        server.registry().build_plan(&RunRequest {
            command: command.to_string(),
            args: serde_json::from_value(args).expect("args must deserialize"),
            stdin: None,
            output: None,
            mode: RunMode::DryRun,
            approval: None,
            dry_run: true,
        })
    };

    // A read-lane plan retries freely.
    let read = plan_for("issues list", json!({}))?;
    assert_eq!(host.retry_decision(&read), RetryDecision::Retry);

    // A write command that declared itself idempotent retries as such.
    let close = plan_for("issues close $args.id", json!({ "id": "42" }))?;
    assert!(close.idempotent, "declaration must project into the plan");
    assert_eq!(
        host.retry_decision(&close),
        RetryDecision::RetryAsIdempotent
    );

    // An undeclared write does not retry, and the decision names the effect.
    let create = plan_for("issues create $args.title", json!({ "title": "boom" }))?;
    assert!(!create.idempotent);
    assert!(!host.retry_decision(&create).is_retryable());
    Ok(())
}
