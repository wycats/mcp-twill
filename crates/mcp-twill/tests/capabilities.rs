//! RFC 0010 acceptance tests: declared preconditions (capabilities) wired
//! into registration validation, catalog projection, help, plan-time
//! diagnostics, runtime denial enrichment, and contract checks.

use mcp_twill::{
    ArgSpec, CapabilityDecl, CommandExample, CommandOutput, CommandRegistry, CommandSpec,
    FrameworkError, HelpRequest, ResponseEnvelope, RunRequest,
};
use serde_json::json;

fn request(command: &str, args: serde_json::Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args).expect("test args must be a JSON object of values"),
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    }
}

fn session_capability() -> CapabilityDecl {
    CapabilityDecl::new("session", "An active browser session").carried_by("session_id")
}

fn provider_spec() -> CommandSpec {
    CommandSpec::new(
        ["session", "start"],
        "Start session",
        "Starts a browser session and returns its id.",
    )
    .provides("session")
}

fn consumer_spec() -> CommandSpec {
    let mut example = CommandExample::new(
        "tabs list --session $args.session_id",
        "List tabs in a session",
    );
    example
        .args
        .insert("session_id".to_string(), json!("sess-1"));
    CommandSpec::new(
        ["tabs", "list"],
        "List tabs",
        "Lists tabs in an active session.",
    )
    .with_arg(ArgSpec::string("session_id", "Session to inspect"))
    .with_example(example)
    .requires("session")
}

fn registry() -> CommandRegistry {
    CommandRegistry::new("capability-test", "Capability integration test server")
        .declare_capability(session_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({ "session_id": "sess-1" })))
        })
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({ "tabs": [] })))
        })
}

// Acceptance: a fully declared capability graph passes validation.
#[test]
fn declared_capability_graph_passes_validation() {
    registry().validate_capabilities().expect("valid graph");
}

// Acceptance: the catalog carries capability declarations and per-operation
// requires/provides.
#[test]
fn catalog_projects_capabilities_and_operation_edges() {
    let catalog = registry().catalog();

    assert_eq!(catalog.capabilities.len(), 1);
    let decl = &catalog.capabilities[0];
    assert_eq!(decl.name, "session");
    assert_eq!(decl.summary, "An active browser session");
    assert_eq!(decl.carrier, "session_id");

    let provider = catalog
        .operations
        .iter()
        .find(|op| op.id == "session.start")
        .expect("provider operation");
    assert_eq!(provider.provides, vec!["session".to_string()]);
    assert!(provider.requires.is_empty());

    let consumer = catalog
        .operations
        .iter()
        .find(|op| op.id == "tabs.list")
        .expect("consumer operation");
    assert_eq!(consumer.requires, vec!["session".to_string()]);
    assert!(consumer.provides.is_empty());
}

// Acceptance: command help names the required capability and derives the
// establishing command from `provides` declarations.
#[test]
fn command_help_renders_requires_with_derived_establishers() {
    let help = registry().help(HelpRequest {
        command: Some("tabs list".to_string()),
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("Requires:"), "help: {}", help.text);
    assert!(
        help.text
            .contains("`session`: An active browser session (carried by `session_id`; establish with `session start`)"),
        "help: {}",
        help.text
    );

    let provider_help = registry().help(HelpRequest {
        command: Some("session start".to_string()),
        topic: None,
        detail: None,
    });
    assert!(
        provider_help.text.contains("Provides:"),
        "help: {}",
        provider_help.text
    );
    assert!(
        provider_help.text.contains("`session`"),
        "help: {}",
        provider_help.text
    );
}

// Acceptance: server-level help lists declared capabilities.
#[test]
fn server_help_lists_declared_capabilities() {
    let help = registry().help(HelpRequest {
        command: None,
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("Capabilities:"), "help: {}", help.text);
    assert!(help.text.contains("`session`"), "help: {}", help.text);
}

// Acceptance: requiring an undeclared capability fails validation.
#[test]
fn requiring_undeclared_capability_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server").register(
        consumer_spec(),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `tabs list` requires capability `session`, which is not declared on the server"
        ),
        "error: {error}"
    );
}

// Acceptance: providing an undeclared capability fails validation.
#[test]
fn providing_undeclared_capability_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server").register(
        provider_spec(),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `session start` provides capability `session`, which is not declared on the server"
        ),
        "error: {error}"
    );
}

// Acceptance: a declared capability with no providing command fails
// validation.
#[test]
fn capability_without_provider_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(session_capability())
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "capability `session` has no providing command; declare `provides` on the command that establishes it"
        ),
        "error: {error}"
    );
}

// Acceptance: a declared capability with no requiring command fails
// validation.
#[test]
fn capability_without_consumer_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(session_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "capability `session` has no requiring command; remove the declaration or declare `requires` on the commands that need it"
        ),
        "error: {error}"
    );
}

// Acceptance: a requiring command without the carrier argument fails
// validation.
#[test]
fn requiring_command_without_carrier_argument_fails_validation() {
    let missing_carrier = CommandSpec::new(
        ["tabs", "list"],
        "List tabs",
        "Lists tabs in an active session.",
    )
    .requires("session");
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(session_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(missing_carrier, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `tabs list` requires capability `session` but has no `session_id` argument to carry it"
        ),
        "error: {error}"
    );
}

// Acceptance: an optional carrier argument fails validation; proof of a
// precondition cannot be optional.
#[test]
fn optional_carrier_argument_fails_validation() {
    let optional_carrier = CommandSpec::new(
        ["tabs", "list"],
        "List tabs",
        "Lists tabs in an active session.",
    )
    .with_arg({
        let mut arg = ArgSpec::string("session_id", "Session to inspect");
        arg.required = false;
        arg
    })
    .requires("session");
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(session_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(optional_carrier, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `tabs list` requires capability `session` but its carrier argument `session_id` is optional; a carrier must be required"
        ),
        "error: {error}"
    );
}

// Acceptance: declaring the same capability twice fails validation.
#[test]
fn duplicate_capability_declaration_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(session_capability())
        .declare_capability(session_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("capability `session` is declared more than once"),
        "error: {error}"
    );
}

// Acceptance: the serving path rejects a registry whose capability graph is
// invalid, so a server that registers cannot serve undeclared preconditions.
#[test]
fn serving_path_rejects_invalid_capability_graph() {
    let registry = CommandRegistry::new("bad", "Bad server").register(
        consumer_spec(),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let error = match mcp_twill::CliMcpServer::new(registry) {
        Ok(_) => panic!("expected serving to reject an invalid capability graph"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("requires capability `session`"),
        "error: {error}"
    );
}

// Acceptance: an unbound carrier fails at plan time with a diagnostic
// located at the carrier argument and steering that names every
// establishing command.
#[tokio::test]
async fn missing_carrier_fails_at_plan_time_with_capability_diagnostic() {
    let response = registry()
        .run(request("tabs list", json!({})))
        .await
        .unwrap_err();

    let FrameworkError::CapabilityMissing {
        capability,
        carrier,
        providers,
    } = &response
    else {
        panic!("expected CapabilityMissing, got {response:?}");
    };
    assert_eq!(capability, "session");
    assert_eq!(carrier, "session_id");
    assert_eq!(providers, &vec!["session start".to_string()]);

    let envelope = ResponseEnvelope::framework_error(response.clone(), None, None);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["error"]["code"], json!("capability_missing"));
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("argument `session_id` carries the `session` capability"),
        "message: {}",
        value["error"]["message"]
    );
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Establish it with `session start`."),
        "message: {}",
        value["error"]["message"]
    );
    assert_eq!(
        value["diagnostics"][0]["location"],
        json!({ "type": "argument", "name": "session_id" })
    );
    assert_eq!(
        value["steering"][0]["label"],
        json!("Establish `session` with `session start`")
    );
    assert_eq!(
        value["steering"][0]["request"],
        json!({ "tool": "help", "arguments": { "command": "session start" } })
    );
}

// Acceptance: a handler-raised capability denial is enriched by the
// framework with the carrier and establishing commands from declarations.
#[tokio::test]
async fn handler_capability_denial_is_enriched_from_declarations() {
    let registry = CommandRegistry::new("capability-test", "Capability test server")
        .declare_capability(session_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({ "session_id": "sess-1" })))
        })
        .register(consumer_spec(), |_context| async {
            Err(FrameworkError::capability_denied(
                "session",
                "session `sess-9` is not active",
            ))
        });

    let error = registry
        .run(request(
            "tabs list --session $args.session_id",
            json!({ "session_id": "sess-9" }),
        ))
        .await
        .unwrap_err();

    let FrameworkError::CapabilityDenied {
        capability,
        detail,
        carrier,
        providers,
    } = &error
    else {
        panic!("expected CapabilityDenied, got {error:?}");
    };
    assert_eq!(capability, "session");
    assert_eq!(detail, "session `sess-9` is not active");
    assert_eq!(carrier.as_deref(), Some("session_id"));
    assert_eq!(providers, &vec!["session start".to_string()]);

    let envelope = ResponseEnvelope::framework_error(error.clone(), None, None);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["error"]["code"], json!("capability_denied"));
    assert_eq!(
        value["diagnostics"][0]["location"],
        json!({ "type": "argument", "name": "session_id" })
    );
    assert_eq!(
        value["steering"][0]["label"],
        json!("Establish `session` with `session start`")
    );
}

// Acceptance: capability requirements participate in catalog identity, so
// adding or removing a requirement changes the hash.
#[test]
fn capability_requirements_change_catalog_hash() {
    let with_capability = registry().catalog_identity().catalog_hash;

    let without_capability = CommandRegistry::new("capability-test", "Capability integration test server")
        .register(
            CommandSpec::new(
                ["session", "start"],
                "Start session",
                "Starts a browser session and returns its id.",
            ),
            |_context| async { Ok(CommandOutput::structured(json!({ "session_id": "sess-1" }))) },
        )
        .register(
            CommandSpec::new(
                ["tabs", "list"],
                "List tabs",
                "Lists tabs in an active session.",
            )
            .with_arg(ArgSpec::string("session_id", "Session to inspect")),
            |_context| async { Ok(CommandOutput::structured(json!({ "tabs": [] }))) },
        )
        .catalog_identity()
        .catalog_hash;

    assert_ne!(with_capability, without_capability);
}

// Acceptance: the contract check passes a registry whose declarations and
// projections agree.
#[test]
fn contract_capability_projection_passes_valid_registry() {
    let violations = mcp_twill::contract::check_capability_projection(&registry());
    assert!(violations.is_empty(), "violations: {violations:?}");
}

// Acceptance: the contract check reports a registry whose capability graph
// is invalid rather than projecting it.
#[test]
fn contract_capability_projection_reports_invalid_graph() {
    let registry = CommandRegistry::new("bad", "Bad server").register(
        consumer_spec(),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let violations = mcp_twill::contract::check_capability_projection(&registry);
    assert_eq!(violations.len(), 1);
    assert!(
        violations[0]
            .message
            .contains("requires capability `session`"),
        "violation: {:?}",
        violations[0]
    );
}

mcp_twill::contract_tests!(registry);
