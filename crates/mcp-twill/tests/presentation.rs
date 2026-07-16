use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use mcp_twill::{
    ArgSpec, ArgumentRendering, CliMcpServer, CliMcpServerConfig, CommandOutput, CommandRegistry,
    CommandSpec, ConfirmationBranch, ConfirmationMessage, ConfirmationPredicate,
    ConfirmationPresentation, InMemoryEventSink, OperationPresentation, PermissionSpec,
    ResponseEnvelope, RunMode, RunRequest, arg,
};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ProgressNotificationParam},
};
use serde_json::{Value, json};

#[path = "support/vbl_presentation.rs"]
mod vbl_presentation;

struct TestClient;

impl ClientHandler for TestClient {
    async fn on_progress(
        &self,
        _params: ProgressNotificationParam,
        _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
    }
}

fn close_presentation() -> ConfirmationPresentation {
    ConfirmationPresentation::new(
        ConfirmationMessage::new("Close browser tab?")
            .text("Close owned tab ")
            .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
            .text("."),
    )
}

fn close_spec() -> CommandSpec {
    let mut spec = CommandSpec::new(["tabs", "close"], "Close tab", "Close an owned browser tab")
        .with_arg(ArgSpec::string("tab_id", "Owned tab id"))
        .with_permission(PermissionSpec::write("browser", "Close a browser tab"));
    spec.invocation_message = Some("Closing an owned browser tab".to_string());
    spec.confirmation = Some(close_presentation());
    spec
}

fn close_registry(dispatches: Arc<AtomicUsize>) -> CommandRegistry {
    CommandRegistry::new("presentation", "Presentation tests").register(
        close_spec(),
        move |_context| {
            let dispatches = dispatches.clone();
            async move {
                dispatches.fetch_add(1, Ordering::SeqCst);
                Ok(CommandOutput::structured(json!({ "closed": true })))
            }
        },
    )
}

fn request(mode: RunMode) -> RunRequest {
    RunRequest {
        command: "tabs close $args.tab_id".to_string(),
        args: serde_json::from_value(json!({ "tab_id": "tab-1" })).unwrap(),
        stdin: None,
        output: None,
        mode,
        approval: None,
        dry_run: false,
    }
}

fn json_object<T: serde::Serialize>(value: T) -> serde_json::Map<String, Value> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .unwrap()
        .clone()
}

#[test]
fn declaration_wire_spelling_and_catalog_normalization_are_canonical() {
    let presentation = ConfirmationPresentation::new(
        ConfirmationMessage::new("Release browser tab?").text("Release the tab."),
    )
    .case(
        ConfirmationPredicate::argument_equals("leave_visible", true),
        ConfirmationMessage::new("Leave browser tab visible?").text("Preserve the tab."),
    );
    let value = serde_json::to_value(&presentation).unwrap();
    assert_eq!(
        value["cases"][0]["when"],
        json!({ "argumentEquals": { "argument": "leave_visible", "value": true } })
    );
    assert_eq!(
        serde_json::to_value(close_presentation()).unwrap()["cases"],
        json!([])
    );

    let branch = serde_json::to_value(ConfirmationBranch::Case {
        predicate: ConfirmationPredicate::argument_present("tab_id"),
    })
    .unwrap();
    assert_eq!(
        branch,
        json!({ "case": { "predicate": { "argumentPresent": { "argument": "tab_id" } } } })
    );
    assert_eq!(
        serde_json::to_value(ArgumentRendering::TrimmedJsonString).unwrap(),
        "trimmedJsonString"
    );

    let operation = mcp_twill::OperationSpec::from_command_spec(&close_spec());
    assert_eq!(
        operation.presentation,
        Some(OperationPresentation {
            invocation_message: Some("Closing an owned browser tab".to_string()),
            confirmation: Some(close_presentation()),
        })
    );

    let mut encoded = serde_json::to_value(&operation).unwrap();
    encoded["presentation"] = json!({ "futureMember": true });
    let decoded: mcp_twill::OperationSpec = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.presentation, None);
    assert!(
        serde_json::to_value(decoded)
            .unwrap()
            .get("presentation")
            .is_none()
    );
}

#[test]
fn low_level_and_builder_declarations_are_equivalent() {
    let low_level = close_registry(Arc::new(AtomicUsize::new(0)));
    let builder = CommandRegistry::build("presentation", "Presentation tests", |server| {
        server.command("tabs close", |command| {
            command
                .summary("Close tab")
                .description("Close an owned browser tab")
                .arg(arg::string("tab_id").summary("Owned tab id"))
                .write("browser", "Close a browser tab")
                .invocation_message("Closing an owned browser tab")
                .confirmation(close_presentation())
                .handle(|_context| async {
                    Ok(CommandOutput::structured(json!({ "closed": true })))
                });
        });
    })
    .unwrap();
    assert_eq!(low_level.operation_specs(), builder.operation_specs());
    assert_eq!(low_level.catalog_identity(), builder.catalog_identity());
    assert!(low_level.validate_presentations().is_ok());
    assert!(builder.validate_presentations().is_ok());
}

#[test]
fn repeated_builder_slots_and_invalid_declarations_fail_before_serving() {
    let repeated = CommandRegistry::build("presentation", "Presentation tests", |server| {
        server.command("tabs close", |command| {
            command
                .summary("Close tab")
                .description("Close a tab")
                .invocation_message("Closing tab")
                .invocation_message("Closing tab")
                .handle(|_context| async {
                    Ok(CommandOutput::structured(json!({ "closed": true })))
                });
        });
    });
    let error = match repeated {
        Ok(_) => panic!("expected repeated presentation declaration to fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("more than once"));

    let mut spec = CommandSpec::new(["broken"], "Broken", "Broken presentation")
        .with_arg(ArgSpec::integer("count", "Count"));
    spec.confirmation = Some(ConfirmationPresentation::new(
        ConfirmationMessage::new("Run broken?").argument(
            "count",
            ArgumentRendering::Plain,
            "(missing)",
        ),
    ));
    let registry = CommandRegistry::new("presentation", "Presentation tests")
        .register(spec, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    assert!(registry.validate_presentations().is_err());
    assert!(CliMcpServer::new(registry).is_err());
}

#[test]
fn presentation_changes_catalog_and_invocation_identity_without_plan_state() {
    let registry = close_registry(Arc::new(AtomicUsize::new(0)));
    let first = registry.build_plan(&request(RunMode::Execute)).unwrap();
    let mut edited = close_spec();
    edited.invocation_message = Some("Closing the selected browser tab".to_string());
    let edited_registry = CommandRegistry::new("presentation", "Presentation tests")
        .register(edited, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let second = edited_registry
        .build_plan(&request(RunMode::Execute))
        .unwrap();
    assert_ne!(
        registry.catalog_identity(),
        edited_registry.catalog_identity()
    );
    assert_ne!(first.invocation_fingerprint, second.invocation_fingerprint);
    let plan_json = serde_json::to_value(first).unwrap();
    assert!(plan_json.get("presentation").is_none());
    assert!(plan_json.get("confirmation").is_none());
    assert!(plan_json.get("invocationMessage").is_none());
}

#[test]
fn case_order_is_hash_significant_but_not_selection_significant() {
    fn spec(reverse: bool) -> CommandSpec {
        let mut cases = vec![
            (
                ConfirmationPredicate::argument_equals("flag", true),
                ConfirmationMessage::new("True?").text("True."),
            ),
            (
                ConfirmationPredicate::argument_equals("flag", false),
                ConfirmationMessage::new("False?").text("False."),
            ),
        ];
        if reverse {
            cases.reverse();
        }
        let presentation = cases.into_iter().fold(
            ConfirmationPresentation::new(ConfirmationMessage::new("Default?").text("Default.")),
            |presentation, (predicate, message)| presentation.case(predicate, message),
        );
        let mut spec = CommandSpec::new(["conditional"], "Conditional", "Conditional command")
            .with_arg(
                ArgSpec::inline_schema("flag", json!({ "type": "boolean" }), "Optional flag")
                    .optional(),
            );
        spec.confirmation = Some(presentation);
        spec
    }

    let registry = |reverse| {
        CommandRegistry::new("presentation", "Presentation tests")
            .register(spec(reverse), |_context| async {
                Ok(CommandOutput::structured(json!({})))
            })
    };
    let request = RunRequest {
        command: "conditional $args.flag".to_string(),
        args: BTreeMap::from([("flag".to_string(), Value::Bool(true))]),
        stdin: None,
        output: None,
        mode: RunMode::Execute,
        approval: None,
        dry_run: false,
    };
    let first = registry(false);
    let second = registry(true);
    first.validate_presentations().unwrap();
    second.validate_presentations().unwrap();
    assert_ne!(first.catalog_identity(), second.catalog_identity());
    assert_ne!(
        first.build_plan(&request).unwrap().invocation_fingerprint,
        second.build_plan(&request).unwrap().invocation_fingerprint
    );
    let help = |registry: &CommandRegistry| {
        registry
            .help(mcp_twill::HelpRequest {
                command: Some("conditional".to_string()),
                topic: Some(mcp_twill::HelpTopic::Usage),
                detail: None,
            })
            .text
    };
    let first_help = help(&first);
    let second_help = help(&second);
    assert!(first_help.find("True?").unwrap() < first_help.find("False?").unwrap());
    assert!(second_help.find("False?").unwrap() < second_help.find("True?").unwrap());
}

#[tokio::test]
async fn live_preview_and_replay_use_one_prepared_confirmation() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(InMemoryEventSink::new());
    let server = CliMcpServer::with_config(
        close_registry(dispatches.clone()),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?
    .with_event_sink(events.clone());
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;

    let preview = client
        .call_tool(
            CallToolRequestParams::new("repo-write")
                .with_arguments(json_object(request(RunMode::Preview))),
        )
        .await?;
    let preview = preview.structured_content.unwrap();
    assert_eq!(preview["preview"]["requiresConfirmation"], true);
    assert_eq!(
        preview["preview"]["confirmation"]["operationId"],
        "tabs.close"
    );
    assert_eq!(
        preview["preview"]["confirmation"]["title"],
        "Close browser tab?"
    );
    assert_eq!(
        preview["preview"]["confirmation"]["message"],
        "Close owned tab tab-1."
    );
    assert_eq!(preview["display"]["title"], "Close browser tab?");
    assert_eq!(preview["display"]["summary"], "Close owned tab tab-1.");
    let _: ResponseEnvelope = serde_json::from_value(preview.clone())?;
    let mut mismatched_display = preview.clone();
    mismatched_display["display"]["summary"] = json!("different copy");
    assert!(serde_json::from_value::<ResponseEnvelope>(mismatched_display).is_err());
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    let required = client
        .call_tool(
            CallToolRequestParams::new("repo-write")
                .with_arguments(json_object(request(RunMode::Execute))),
        )
        .await?;
    let required = required.structured_content.unwrap();
    assert_eq!(required["status"], "permissionRequired");
    assert_eq!(
        required["preview"]["confirmation"]["message"],
        "Close owned tab tab-1."
    );
    let token = required["replay"]["token"].as_str().unwrap().to_string();
    let mut cancelled = request(RunMode::Execute);
    cancelled.approval = Some(mcp_twill::ApprovalInput {
        token: token.clone(),
        confirm: false,
    });
    let cancellation = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(cancelled)))
        .await?;
    assert_eq!(cancellation.is_error, Some(true));
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    let mut approved = request(RunMode::Execute);
    approved.approval = Some(mcp_twill::ApprovalInput {
        token,
        confirm: true,
    });
    let success = client
        .call_tool(CallToolRequestParams::new("repo-write").with_arguments(json_object(approved)))
        .await?;
    assert_eq!(success.is_error, Some(false));
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);

    let event_json = serde_json::to_string(&events.events())?;
    assert!(!event_json.contains("Close browser tab?"));
    assert!(!event_json.contains("Close owned tab tab-1."));

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn allow_preview_omits_declared_confirmation() -> anyhow::Result<()> {
    let mut spec = close_spec();
    spec.permissions.clear();
    let events = Arc::new(InMemoryEventSink::new());
    let server = CliMcpServer::with_config(
        CommandRegistry::new("presentation", "Presentation tests")
            .register(spec, |_context| async {
                Ok(CommandOutput::structured(json!({})))
            }),
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?
    .with_event_sink(events.clone());
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let preview = client
        .call_tool(
            CallToolRequestParams::new("repo")
                .with_arguments(json_object(request(RunMode::Preview))),
        )
        .await?
        .structured_content
        .unwrap();
    assert_eq!(preview["preview"]["requiresConfirmation"], false);
    assert!(preview["preview"].get("confirmation").is_none());
    let event_json = serde_json::to_string(&events.events())?;
    assert!(!event_json.contains("Close browser tab?"));
    assert!(!event_json.contains("Closing an owned browser tab"));
    client.cancel().await?;
    Ok(())
}

#[test]
fn legacy_and_prepared_response_deserialization_enforce_the_boundary() {
    let plan = close_registry(Arc::new(AtomicUsize::new(0)))
        .build_plan(&request(RunMode::Preview))
        .unwrap();
    let legacy = ResponseEnvelope::preview(plan, true);
    let legacy_json = serde_json::to_value(&legacy).unwrap();
    let round_trip: ResponseEnvelope = serde_json::from_value(legacy_json).unwrap();
    assert!(round_trip.preview.unwrap().confirmation.is_none());

    let mut invalid = serde_json::to_value(legacy).unwrap();
    invalid["preview"]["requiresConfirmation"] = json!(false);
    invalid["preview"]["confirmation"] = json!({
        "operationId": "tabs.close",
        "branch": "default",
        "title": "Close browser tab?",
        "message": "Close owned tab tab-1."
    });
    assert!(serde_json::from_value::<ResponseEnvelope>(invalid).is_err());

    let mut wrong_operation = serde_json::to_value(ResponseEnvelope::preview(
        close_registry(Arc::new(AtomicUsize::new(0)))
            .build_plan(&request(RunMode::Preview))
            .unwrap(),
        true,
    ))
    .unwrap();
    wrong_operation["preview"]["confirmation"] = json!({
        "operationId": "tabs.other",
        "branch": "default",
        "title": "Close browser tab?",
        "message": "Close owned tab tab-1."
    });
    wrong_operation["display"] = json!({
        "title": "Close browser tab?",
        "summary": "Close owned tab tab-1."
    });
    assert!(serde_json::from_value::<ResponseEnvelope>(wrong_operation).is_err());
}

#[test]
fn confirmation_projection_contract_is_green() {
    let registry = close_registry(Arc::new(AtomicUsize::new(0)));
    assert!(mcp_twill::check_confirmation_projection(&registry).is_empty());
    let help = registry.help(mcp_twill::HelpRequest {
        command: Some("tabs close".to_string()),
        topic: Some(mcp_twill::HelpTopic::Usage),
        detail: None,
    });
    assert!(help.text.contains("Closing an owned browser tab"));
    assert!(help.text.contains("Close browser tab?"));
}

#[test]
fn public_prepared_types_round_trip() {
    let prepared = mcp_twill::PreparedInvocationPresentation {
        invocation_message: "Closing an owned browser tab".to_string(),
        confirmation: Some(mcp_twill::PreparedConfirmation {
            operation_id: "tabs.close".to_string(),
            branch: ConfirmationBranch::Default,
            title: "Close browser tab?".to_string(),
            message: "Close owned tab tab-1.".to_string(),
        }),
    };
    let encoded = serde_json::to_value(&prepared).unwrap();
    assert_eq!(encoded["invocationMessage"], "Closing an owned browser tab");
    assert_eq!(encoded["confirmation"]["operationId"], "tabs.close");
    assert_eq!(
        serde_json::from_value::<mcp_twill::PreparedInvocationPresentation>(encoded).unwrap(),
        prepared
    );
}

#[test]
fn public_json_schemas_include_only_the_declared_presentation_surfaces() {
    let operation = serde_json::to_value(schemars::schema_for!(mcp_twill::OperationSpec)).unwrap();
    let preview =
        serde_json::to_value(schemars::schema_for!(mcp_twill::PermissionPreview)).unwrap();
    let envelope = serde_json::to_value(schemars::schema_for!(ResponseEnvelope)).unwrap();
    assert!(operation.to_string().contains("presentation"));
    assert!(preview.to_string().contains("confirmation"));
    assert!(envelope.to_string().contains("PreparedConfirmation"));
    assert!(
        !operation
            .to_string()
            .contains("SurfacePresentationDefaults")
    );
    assert!(!preview.to_string().contains("invocationContext"));
}

#[tokio::test]
async fn vbl_v049_portable_vectors_execute_against_the_owner_local_registry() -> anyhow::Result<()>
{
    let vectors: Value = serde_json::from_str(include_str!(
        "fixtures/vbl/v0.4.9/presentation-vectors.json"
    ))?;
    let registry = vbl_presentation::registry();
    registry.validate_presentations()?;
    let operations = registry.operation_specs();
    for vector in vectors["invocation"].as_array().unwrap() {
        let method = vector["method"].as_str().unwrap();
        if method == "console" {
            assert_eq!(vector["output"], "Running Console");
            continue;
        }
        let (path, _) = vbl_presentation::OPERATION_MAPPING
            .iter()
            .find_map(|(released, path, title)| (*released == method).then_some((*path, *title)))
            .unwrap();
        let operation = operations
            .iter()
            .find(|operation| operation.name() == path)
            .unwrap();
        assert_eq!(
            operation
                .presentation
                .as_ref()
                .and_then(|presentation| presentation.invocation_message.as_deref()),
            vector["output"].as_str()
        );
    }

    let server = CliMcpServer::with_config(
        registry,
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )?;
    let (server_transport, client_transport) = tokio::io::duplex(32 * 1024);
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    for vector in vectors["confirmation"].as_array().unwrap() {
        let method = vector["method"].as_str().unwrap();
        let (path, _) = vbl_presentation::OPERATION_MAPPING
            .iter()
            .find_map(|(released, path, title)| (*released == method).then_some((*path, *title)))
            .unwrap();
        let input = vector["input"].as_object().unwrap();
        let placeholders = input
            .keys()
            .map(|name| format!("$args.{name}"))
            .collect::<Vec<_>>()
            .join(" ");
        let command = if placeholders.is_empty() {
            path.to_string()
        } else {
            format!("{path} {placeholders}")
        };
        let request = RunRequest {
            command,
            args: input
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            stdin: None,
            output: None,
            mode: RunMode::Preview,
            approval: None,
            dry_run: false,
        };
        let tool = if vector["output"].is_null() {
            "repo"
        } else {
            "repo-write"
        };
        let response = client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(json_object(request)))
            .await?;
        let response = response.structured_content.unwrap();
        if let Some(expected) = vector["output"].as_object() {
            assert_eq!(
                response["preview"]["confirmation"]["title"], expected["title"],
                "{method}"
            );
            assert_eq!(
                response["preview"]["confirmation"]["message"], expected["message"],
                "{method}"
            );
        } else {
            assert!(response["preview"].get("confirmation").is_none());
        }
    }
    client.cancel().await?;
    Ok(())
}
