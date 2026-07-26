use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, ArgSpec, CliMcpServer, CommandContext,
    CommandRegistry, CommandSpec, ConversationIdentity, DynamicCommandFailure,
    FrameworkHelpProjection, McpProtocolTarget, NativeConfirmationBridge,
    NativeConfirmationBridgeError, NativeConfirmationDecision, NativeConfirmationRequest,
    NativeConfirmationRoute, NativeToolDecl, NativeToolSurface, OutputContract,
};
use mcp_twill_host::{
    HostAdapterProfile, HostCallOutcomeV1, HostConfirmationPolicy, HostConfirmationTrigger,
    HostContextReason, HostGuidanceProjection, HostGuidanceSegment, HostInvocationContextV1,
    HostInvocationLimits, HostRecoveryAction, HostRuntimeFactsV1, UnsupportedContextPolicy,
    VsCodeVersion, generate_vscode_artifacts,
};
use serde_json::{Value, json};

#[path = "support/vbl.rs"]
mod vbl;
#[path = "support/vbl_host.rs"]
mod vbl_host;
#[path = "support/vbl_native.rs"]
mod vbl_native;

struct AllowBridge;

#[async_trait]
impl NativeConfirmationBridge for AllowBridge {
    async fn confirm(
        &self,
        _request: NativeConfirmationRequest,
    ) -> std::result::Result<NativeConfirmationDecision, NativeConfirmationBridgeError> {
        Ok(NativeConfirmationDecision::Allow)
    }
}

fn vbl_fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/vbl/v0.4.9")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).expect("read VBL fixture"))
        .expect("parse VBL fixture")
}

fn canonicalize_required_sets(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
                required.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
            }
            if let Some(singleton) = object
                .get("enum")
                .and_then(Value::as_array)
                .filter(|values| values.len() == 1)
                .and_then(|values| values.first())
                .cloned()
            {
                object.remove("enum");
                object.remove("type");
                object.insert("const".to_string(), singleton);
            }
            for nested in object.values_mut() {
                canonicalize_required_sets(nested);
            }
        }
        Value::Array(values) => {
            for nested in values {
                canonicalize_required_sets(nested);
            }
        }
        _ => {}
    }
}

fn registry() -> CommandRegistry {
    let spec = CommandSpec::new(
        ["items", "get"],
        "Get Item",
        "Read one item from the catalog.",
    )
    .with_arg(ArgSpec::string("id", "Item id"))
    .with_output(OutputContract {
        application: Some(ApplicationResultContract::new(json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "value": { "type": "string" },
                "secret": { "type": "string" }
            },
            "required": ["id", "value", "secret"],
            "additionalProperties": false
        }))),
        ..OutputContract::default()
    });
    CommandRegistry::new("host-adapter-test", "Host adapter acceptance").register_dynamic(
        spec,
        |context: CommandContext| async move {
            let id = context.plan.bound_args["id"]
                .value
                .as_str()
                .expect("validated string")
                .to_string();
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "id": id,
                "value": "found",
                "secret": "private"
            })))
        },
    )
}

fn surface(registry: &CommandRegistry) -> mcp_twill::Result<NativeToolSurface> {
    NativeToolSurface::builder("host-items")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .tool(NativeToolDecl::Direct {
            name: "item_get".to_string(),
            operation_id: "items.get".to_string(),
            title: Some("Get Item".to_string()),
            description: Some("Read one item from the catalog.".to_string()),
        })
        .build(registry, McpProtocolTarget::V2025_11_25)
}

fn surface_with_help(registry: &CommandRegistry) -> mcp_twill::Result<NativeToolSurface> {
    NativeToolSurface::builder("host-items-help")
        .framework_help(FrameworkHelpProjection::Tool {
            name: "help".to_string(),
        })
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .tool(NativeToolDecl::Direct {
            name: "item_get".to_string(),
            operation_id: "items.get".to_string(),
            title: Some("Get Item".to_string()),
            description: Some("Read one item from the catalog.".to_string()),
        })
        .build(registry, McpProtocolTarget::V2025_11_25)
}

fn unsupported_policy() -> UnsupportedContextPolicy {
    [
        HostContextReason::UnknownTokenShape,
        HostContextReason::InvalidSessionResource,
        HostContextReason::InvalidWorkingDirectory,
        HostContextReason::ProviderFailed,
    ]
    .into_iter()
    .fold(UnsupportedContextPolicy::new(), |policy, reason| {
        policy.reason(
            reason,
            "This host did not expose compatible invocation context",
        )
    })
}

fn profile(
    surface: &mcp_twill::NativeToolSurfaceSnapshot,
) -> mcp_twill::Result<HostAdapterProfile> {
    HostAdapterProfile::vscode("items-vscode", VsCodeVersion::new(1, 120, 0))
        .tool_name_prefix("host_")
        .icon("$(tools)")
        .prompt_reference("items.get", "item")
        .confirmation(HostConfirmationPolicy::presentation_only(
            HostConfirmationTrigger::None,
        ))
        .omit_result_property("items.get", "secret")
        .unsupported_context(unsupported_policy())
        .invocation_limits(HostInvocationLimits::new(64 * 1024, 64 * 1024))
        .in_process()
        .build(surface)
}

#[test]
fn builder_and_direct_declaration_compile_identically() -> anyhow::Result<()> {
    let registry = registry();
    let surface = surface(&registry)?;
    let built = profile(surface.snapshot())?;
    let direct = built.declaration().clone().compile(surface.snapshot())?;

    assert_eq!(
        built.snapshot().canonical_json(),
        direct.snapshot().canonical_json()
    );
    assert_eq!(
        built.snapshot().host_adapter_hash(),
        direct.snapshot().host_adapter_hash()
    );
    Ok(())
}

#[tokio::test]
async fn in_process_host_uses_native_dispatch_and_projects_results() -> anyhow::Result<()> {
    let registry = registry();
    let surface = surface(&registry)?;
    let profile = profile(surface.snapshot())?;
    let server = CliMcpServer::builder(registry).surface(surface).build()?;
    let adapter = profile.bind_in_process(server)?;
    let result = adapter
        .call(
            "host_item_get",
            BTreeMap::from([("id".to_string(), json!("42"))]),
            HostInvocationContextV1::Absent {
                workspace_roots: None,
            },
            HostRuntimeFactsV1::VsCode {
                engine_version: None,
            },
        )
        .await;

    assert_eq!(
        result.outcome,
        HostCallOutcomeV1::Success {
            text: r#"{"id":"42","value":"found"}"#.to_string()
        }
    );
    Ok(())
}

#[tokio::test]
async fn unsupported_context_fails_before_handler_dispatch() -> anyhow::Result<()> {
    let registry = registry();
    let surface = surface(&registry)?;
    let profile = profile(surface.snapshot())?;
    let server = CliMcpServer::builder(registry).surface(surface).build()?;
    let adapter = profile.bind_in_process(server)?;
    let result = adapter
        .call(
            "host_item_get",
            BTreeMap::from([("id".to_string(), json!("42"))]),
            HostInvocationContextV1::Unsupported {
                reason: HostContextReason::UnknownTokenShape,
            },
            HostRuntimeFactsV1::VsCode {
                engine_version: None,
            },
        )
        .await;

    match result.outcome {
        HostCallOutcomeV1::FrameworkError { code, text } => {
            assert_eq!(code, "unsupported_host");
            assert!(text.contains("This host did not expose compatible invocation context"));
            assert!(!text.contains("42"));
        }
        other => panic!("expected framework failure, got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn generated_framework_help_discards_private_context() -> anyhow::Result<()> {
    let registry = registry();
    let surface = surface_with_help(&registry)?;
    let profile = profile(surface.snapshot())?;
    let server = CliMcpServer::builder(registry).surface(surface).build()?;
    let adapter = profile.bind_in_process(server)?;
    let identity = ConversationIdentity::new("com.example.host", "private-conversation")?;
    let result = adapter
        .call(
            "host_help",
            BTreeMap::new(),
            HostInvocationContextV1::Ambient {
                conversation_identity: identity,
                workspace_roots: None,
            },
            HostRuntimeFactsV1::VsCode {
                engine_version: None,
            },
        )
        .await;

    match result.outcome {
        HostCallOutcomeV1::Success { text } => {
            assert!(text.contains("item_get"), "{text}");
            assert!(!text.contains("private-conversation"));
            assert!(!text.contains("com.example.host"));
        }
        other => panic!("expected framework help, got {other:?}"),
    }
    Ok(())
}

#[test]
fn guidance_uses_typed_structural_references() -> anyhow::Result<()> {
    let registry = registry();
    let surface = surface(&registry)?;
    let mut raw = profile(surface.snapshot())?.declaration().clone();
    raw.guidance.tool_suffix = vec![HostGuidanceSegment::Text(
        "Prefer item_get for this lookup.".to_string(),
    )];
    assert!(raw.compile(surface.snapshot()).is_err());

    let mut typed = profile(surface.snapshot())?.declaration().clone();
    typed.guidance = HostGuidanceProjection {
        server_prefix: vec![
            HostGuidanceSegment::Text("Host entry point: ".to_string()),
            HostGuidanceSegment::Operation {
                operation_id: "items.get".to_string(),
            },
            HostGuidanceSegment::Text(".".to_string()),
        ],
        tool_suffix: vec![
            HostGuidanceSegment::Text(" Prefer ".to_string()),
            HostGuidanceSegment::Operation {
                operation_id: "items.get".to_string(),
            },
            HostGuidanceSegment::Text(".".to_string()),
        ],
        ..HostGuidanceProjection::default()
    };
    let compiled = typed.compile(surface.snapshot())?;
    assert_eq!(
        compiled.snapshot().document()["serverInstructions"],
        "Host entry point: item_get. Call the named tools directly."
    );
    let generated = generate_vscode_artifacts(compiled.snapshot())?;
    assert!(
        generated.manifest_projection()["contributes"]["languageModelTools"][0]["modelDescription"]
            .as_str()
            .is_some_and(|description| description.ends_with(" Prefer item_get."))
    );
    Ok(())
}

#[test]
fn snapshot_changes_reversibly_with_host_only_facts() -> anyhow::Result<()> {
    let registry = registry();
    let surface = surface(&registry)?;
    let first = profile(surface.snapshot())?;
    let mut changed = first.declaration().clone();
    changed.icon = Some("$(browser)".to_string());
    let changed = changed.compile(surface.snapshot())?;
    let restored = first.declaration().clone().compile(surface.snapshot())?;

    assert_ne!(
        first.snapshot().host_adapter_hash(),
        changed.snapshot().host_adapter_hash()
    );
    assert_eq!(
        first.snapshot().host_adapter_hash(),
        restored.snapshot().host_adapter_hash()
    );
    assert_eq!(
        first.snapshot().surface_hash(),
        changed.snapshot().surface_hash()
    );
    Ok(())
}

#[test]
fn vscode_artifacts_derive_manifest_identity_and_schema() -> anyhow::Result<()> {
    let registry = registry();
    let surface = surface(&registry)?;
    let profile = profile(surface.snapshot())?;
    let generated = generate_vscode_artifacts(profile.snapshot())?;
    let contribution = &generated.manifest_projection()["contributes"]["languageModelTools"][0];

    assert_eq!(
        generated.manifest_projection()["engines"]["vscode"],
        "^1.120.0"
    );
    assert_eq!(contribution["name"], "host_item_get");
    assert_eq!(contribution["displayName"], "Get Item");
    assert_eq!(contribution["toolReferenceName"], "item");
    assert_eq!(
        contribution["inputSchema"],
        serde_json::to_value(surface.snapshot().tools()[0].input_schema.as_ref())?
    );
    assert!(generated.adapter_typescript().contains(
        "export function registerGeneratedHostTools(extensionContext: vscode.ExtensionContext"
    ));
    assert!(generated.adapter_typescript().ends_with('\n'));
    assert!(!generated.adapter_typescript().ends_with("\n\n"));
    Ok(())
}

#[test]
fn public_wire_types_generate_schemas() {
    let schema = schemars::schema_for!(mcp_twill_host::HostCallEnvelopeV1);
    let value = serde_json::to_value(schema).expect("schema serializes");
    assert_eq!(
        value["properties"]["hostProfile"]["type"],
        Value::String("string".to_string())
    );
    assert_eq!(
        value["additionalProperties"],
        Value::Bool(false),
        "closed host envelope schema"
    );
    let context = HostInvocationContextV1::Ambient {
        conversation_identity: ConversationIdentity::new(
            "com.example.host",
            "private-conversation",
        )
        .expect("valid test identity"),
        workspace_roots: None,
    };
    assert_eq!(
        serde_json::to_value(&context).expect("context serializes"),
        json!({
            "kind": "ambient",
            "conversationIdentity": {
                "version": 1,
                "issuer": "com.example.host",
                "id": "private-conversation",
            },
        })
    );
    let debug = format!("{context:?}");
    assert!(!debug.contains("private-conversation"));
    assert!(!debug.contains("com.example.host"));
    assert!(
        serde_json::from_value::<HostInvocationContextV1>(json!({
            "kind": "absent",
            "unexpected": true,
        }))
        .is_err()
    );
}

#[test]
fn vbl_v049_host_profile_generates_the_released_tool_contributions() -> anyhow::Result<()> {
    let baseline = vbl_fixture("baseline-tools.json");
    let observed = vbl_fixture("surface-catalog.json");
    let released = vbl_fixture("vscode-package.json");
    let registry = vbl_native::host_registry(&baseline, &observed, vbl::PREAMBLE);
    let surface = vbl_native::host_surface(&registry, &observed)?;
    let profile = vbl_host::vscode_host_profile(surface.snapshot())?;
    let mut conflicting_recovery = profile.declaration().clone();
    conflicting_recovery.unsupported_context.recovery = Some(HostRecoveryAction {
        code: "use_chat_or_explicit_surface".to_string(),
        summary: "different summary".to_string(),
    });
    assert!(conflicting_recovery.compile(surface.snapshot()).is_err());
    let mut missing_recovery = profile.declaration().clone();
    missing_recovery
        .absent_context
        .rejections
        .get_mut("session.start")
        .expect("VBL session rejection")
        .recovery = None;
    assert!(missing_recovery.compile(surface.snapshot()).is_err());
    let generated = generate_vscode_artifacts(profile.snapshot())?;

    assert_eq!(
        generated.manifest_projection()["engines"],
        released["engines"]
    );
    let generated_tools = generated.manifest_projection()["contributes"]["languageModelTools"]
        .as_array()
        .expect("generated tool contributions");
    let released_tools = released["contributes"]["languageModelTools"]
        .as_array()
        .expect("released tool contributions");
    assert_eq!(generated_tools.len(), 27);
    assert_eq!(generated_tools.len(), released_tools.len());
    for (generated, released) in generated_tools.iter().zip(released_tools) {
        let name = generated["name"].as_str().expect("generated tool name");
        for field in [
            "name",
            "displayName",
            "userDescription",
            "modelDescription",
            "icon",
            "inputSchema",
            "canBeReferencedInPrompt",
            "toolReferenceName",
        ] {
            let mut generated_field = generated.get(field).cloned();
            let mut released_field = released.get(field).cloned();
            if field == "inputSchema" {
                if let Some(value) = &mut generated_field {
                    canonicalize_required_sets(value);
                }
                if let Some(value) = &mut released_field {
                    canonicalize_required_sets(value);
                }
            }
            assert_eq!(
                generated_field, released_field,
                "VBL contribution `{name}` differs at `{field}`"
            );
        }
    }

    // Loading the host-specific support module must not displace the already
    // accepted RFC 0011/RFC 0015 adoption paths it extends.
    let guidance_registry = vbl::registry();
    assert!(guidance_registry.preamble().is_some());
    let schema_registry = vbl::argument_schema_registry(&baseline);
    assert_eq!(
        schema_registry.operation_specs().len(),
        baseline.as_array().unwrap().len()
    );
    let ordinary_registry = vbl_native::registry(&baseline, &observed, vbl::PREAMBLE);
    let ordinary_surface = vbl_native::surface(&ordinary_registry, &observed)?;
    assert_eq!(ordinary_surface.snapshot().tools().len(), 27);
    Ok(())
}

#[tokio::test]
async fn vbl_context_gates_render_released_host_errors() -> anyhow::Result<()> {
    let baseline = vbl_fixture("baseline-tools.json");
    let observed = vbl_fixture("surface-catalog.json");
    let registry = vbl_native::host_registry(&baseline, &observed, vbl::PREAMBLE);
    let surface = vbl_native::host_surface(&registry, &observed)?;
    let process_profile = vbl_host::vscode_host_profile(surface.snapshot())?;
    let mut declaration = process_profile.declaration().clone();
    declaration.transport = mcp_twill_host::HostInvocationTransport::InProcess;
    let profile = declaration.compile(surface.snapshot())?;
    let server = CliMcpServer::builder(registry)
        .surface(surface)
        .native_confirmation_bridge(Arc::new(AllowBridge))
        .build()?;
    let adapter = profile.bind_in_process(server)?;

    let absent = adapter
        .call(
            "visible_browser_lab_start_session",
            BTreeMap::new(),
            HostInvocationContextV1::Absent {
                workspace_roots: None,
            },
            HostRuntimeFactsV1::VsCode {
                engine_version: None,
            },
        )
        .await;
    assert_eq!(
        absent.outcome,
        HostCallOutcomeV1::ApplicationError {
            code: "session_required".to_string(),
            text: "start_session failed with session_required. Global VS Code tool invocations have no conversation identity and do not expose explicit session handles. Recovery: invoke Visible Browser Lab from a supported VS Code chat, or use the explicit MCP/CLI surface".to_string(),
        }
    );

    let absent_consumer = adapter
        .call(
            "visible_browser_lab_new_tab",
            BTreeMap::new(),
            HostInvocationContextV1::Absent {
                workspace_roots: None,
            },
            HostRuntimeFactsV1::VsCode {
                engine_version: None,
            },
        )
        .await;
    assert_eq!(
        absent_consumer.outcome,
        HostCallOutcomeV1::ApplicationError {
            code: "session_required".to_string(),
            text: "new_tab failed with session_required. Global VS Code tool invocations have no conversation identity and do not expose explicit session handles. Recovery: invoke Visible Browser Lab from a supported VS Code chat, or use the explicit MCP/CLI surface".to_string(),
        }
    );

    let unsupported = adapter
        .call(
            "visible_browser_lab_new_tab",
            BTreeMap::new(),
            HostInvocationContextV1::Unsupported {
                reason: HostContextReason::UnknownTokenShape,
            },
            HostRuntimeFactsV1::VsCode {
                engine_version: None,
            },
        )
        .await;
    assert_eq!(
        unsupported.outcome,
        HostCallOutcomeV1::FrameworkError {
            code: "unsupported_host".to_string(),
            text: "new_tab failed with unsupported_host. VS Code did not expose a compatible chat session resource; Visible Browser Lab requires VS Code 1.120 or newer with the supported invocation-token shape. Recovery: update and reload VS Code, or use the explicit MCP/CLI surface".to_string(),
        }
    );
    Ok(())
}
