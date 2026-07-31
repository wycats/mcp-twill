//! RFC 0015's test-only native-surface adoption of VBL v0.4.9.

use mcp_twill::{
    ApplicationErrorSpec, ApplicationMessageDecl, ApplicationRecoveryDecl,
    ApplicationResultContract, ApplicationSuccess, ArgSpec, ArgumentRendering, BindAmbientResource,
    CommandRegistry, CommandSpec, ConfirmationMessage, ConfirmationPredicate,
    ConfirmationPresentation, DynamicCommandFailure, ExplicitCarrierPolicy,
    FrameworkHelpProjection, McpProtocolTarget, NativeApplicationErrorDialect,
    NativeConfirmationRoute, NativeExposurePolicy, NativeGroupDescriptionDialect,
    NativeToolSurface, NativeToolSurfaceDecl, NoApplicationError, OutputContract, PermissionEffect,
    PermissionSpec, PrivateResourceReference, RecoveryCardinality, ResolveResource,
    ResourceBindingMode, ResourceRefusal, TaskDeliveryDecl,
};
use serde_json::{Value, json};

use crate::vbl::{ERROR_OWNERS, ErrorOwner, OPERATION_MAPPING, Session, ambient_session_adoption};

struct SessionResolver;
struct SessionBinder;

impl ResolveResource<Session> for SessionResolver {
    async fn resolve(
        &self,
        _reference: &str,
        _plan: &mcp_twill::InvocationPlan,
    ) -> Result<Session, ResourceRefusal> {
        Ok(Session)
    }
}

impl BindAmbientResource<Session> for SessionBinder {
    type Error = NoApplicationError;
    type ErrorFootprint = mcp_twill::AllApplicationErrorCodes<NoApplicationError>;

    async fn bind(
        &self,
        _context: mcp_twill::AmbientBindingContext<'_>,
    ) -> Result<
        PrivateResourceReference,
        mcp_twill::AmbientBindingFailure<Self::Error, Self::ErrorFootprint>,
    > {
        Ok(PrivateResourceReference::from_id("ambient-session")
            .expect("static ambient session id is valid"))
    }
}

/// Authors a Twill catalog from the released per-operation schemas.
pub fn registry(
    baseline: &Value,
    observed_surface: &Value,
    server_instructions: &str,
) -> CommandRegistry {
    registry_impl(baseline, observed_surface, server_instructions, false)
}

/// Authors the same released catalog with the RFC 0016/RFC 0018 declarations
/// consumed by RFC 0019's VS Code host profile.
pub fn host_registry(
    baseline: &Value,
    observed_surface: &Value,
    server_instructions: &str,
) -> CommandRegistry {
    registry_impl(baseline, observed_surface, server_instructions, true)
}

fn registry_impl(
    baseline: &Value,
    observed_surface: &Value,
    server_instructions: &str,
    host_adoption: bool,
) -> CommandRegistry {
    let paths = OPERATION_MAPPING
        .iter()
        .map(|(released, path, title)| (*released, (*path, *title)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut registry = CommandRegistry::new(
        "vbl-native-surface",
        "Visible Browser Lab native surface adoption fixture",
    )
    .declare_preamble(server_instructions);
    if host_adoption {
        let (resource, _) = ambient_session_adoption();
        registry = registry
            .declare_resource(resource)
            .with_resolver::<Session>(SessionResolver);
    }
    for tool in baseline.as_array().expect("VBL baseline tools") {
        let name = tool["name"].as_str().expect("VBL tool name");
        let (path, _) = paths[name];
        let annotations = &tool["annotations"];
        let title = annotations["title"].as_str().expect("VBL tool title");
        let mut output_schema = tool["outputSchema"].clone();
        restore_released_output_definitions(name, observed_surface, &mut output_schema);
        let mut application = ApplicationResultContract::new(output_schema);
        if host_adoption {
            application.errors = application_error_inventory();
        }
        let mut spec = CommandSpec::new(
            path.split_whitespace(),
            title,
            tool["description"].as_str().expect("VBL tool description"),
        )
        .with_output(OutputContract {
            application: Some(application),
            ..OutputContract::default()
        });
        let input = tool["inputSchema"].as_object().expect("VBL input schema");
        let grouped = [
            "interact_",
            "console_",
            "network_",
            "emulation_",
            "performance_",
            "memory_",
            "screencast_",
            "artifacts_",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix));
        let required = input["required"]
            .as_array()
            .expect("VBL required list")
            .iter()
            .filter_map(Value::as_str)
            .filter(|argument| !(grouped && *argument == "operation"))
            .collect::<Vec<_>>();
        let properties = input["properties"]
            .as_object()
            .expect("VBL property schemas");
        for argument in required.iter().copied().chain(
            properties
                .keys()
                .map(String::as_str)
                .filter(|argument| !required.contains(argument))
                .filter(|argument| !(grouped && *argument == "operation")),
        ) {
            let schema = properties[argument].clone();
            let summary = schema
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("VBL argument")
                .to_string();
            let mut arg = ArgSpec::inline_schema(argument, schema, summary);
            if !required.contains(&argument) {
                arg = arg.optional();
            }
            if name == "screencast_start" && argument == "max_width" {
                arg = arg.requires_argument("max_height");
            }
            if name == "screencast_start" && argument == "max_height" {
                arg = arg.requires_argument("max_width");
            }
            spec = spec.with_arg(arg);
        }
        if annotations["readOnlyHint"] == json!(true) {
            spec = spec.with_permission(PermissionSpec::new(
                PermissionEffect::Read,
                name,
                "Released VBL read effect",
            ));
        } else if annotations["destructiveHint"] == json!(true) {
            spec = spec.with_permission(PermissionSpec::new(
                PermissionEffect::Delete,
                name,
                "Released VBL destructive effect",
            ));
        } else {
            spec = spec.with_permission(PermissionSpec::new(
                PermissionEffect::Write,
                name,
                "Released VBL write effect",
            ));
        }
        if annotations["openWorldHint"] == json!(true) {
            spec = spec.with_permission(PermissionSpec::new(
                PermissionEffect::Network,
                name,
                "Released VBL open-world effect",
            ));
        }
        if annotations["idempotentHint"] == json!(true) {
            spec = spec.idempotent();
        }
        if host_adoption {
            spec = apply_host_presentation(name, spec);
            if name == "start_session" {
                spec.grants.push("session".to_string());
            } else if properties.contains_key("agent_session_id") {
                spec.requires_resources.push("session".to_string());
            }
            registry = registry.register_dynamic(spec, |_context| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
            });
        } else {
            registry = registry.register_dynamic(spec, |_context| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
            });
        }
    }
    registry
}

fn apply_host_presentation(method: &str, mut spec: CommandSpec) -> CommandSpec {
    spec.invocation_message = match method {
        "start_session" => Some("Starting a visible browser session"),
        "snapshot" => Some("Capturing a browser snapshot"),
        "screenshot" => Some("Capturing a browser screenshot"),
        "navigate" => Some("Navigating the owned browser tab"),
        "click" => Some("Clicking a browser element"),
        "fill" | "fill_form" => Some("Filling browser form controls"),
        "wait_for" => Some("Waiting for browser state"),
        _ => None,
    }
    .map(str::to_string);
    spec.confirmation = match method {
        "claim_tab" => Some(ConfirmationPresentation::new(
            ConfirmationMessage::new("Claim browser tab?")
                .text("Claim target ")
                .argument("target_id", ArgumentRendering::Plain, "(unknown target)")
                .text(" for this agent session."),
        )),
        "close_tab" => Some(ConfirmationPresentation::new(
            ConfirmationMessage::new("Close browser tab?")
                .text("Close owned tab ")
                .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                .text("."),
        )),
        "release_tab" => Some(
            ConfirmationPresentation::new(
                ConfirmationMessage::new("Release browser tab?")
                    .text("Release owned tab ")
                    .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                    .text("; a VBL-created target remains eligible for expiry cleanup."),
            )
            .case(
                ConfirmationPredicate::argument_equals("leave_visible", true),
                ConfirmationMessage::new("Leave browser tab visible?")
                    .text("Release owned tab ")
                    .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                    .text(" and preserve it after this session expires. User instruction: ")
                    .argument(
                        "user_instruction",
                        ArgumentRendering::TrimmedJsonString,
                        "(missing; this request will be rejected)",
                    )
                    .text("."),
            ),
        ),
        "focus_tab" => Some(ConfirmationPresentation::new(
            ConfirmationMessage::new("Bring Chrome forward?")
                .text("Focus owned tab ")
                .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                .text(" for manual inspection or handoff."),
        )),
        _ => spec.confirmation,
    };
    spec
}

fn application_error_inventory() -> Vec<ApplicationErrorSpec> {
    ERROR_OWNERS
        .into_iter()
        .filter(|(_, owner)| matches!(owner, ErrorOwner::Application))
        .map(|(code, _)| ApplicationErrorSpec {
            code: code.to_string(),
            summary: code.replace('_', " "),
            message: ApplicationMessageDecl::RuntimeBounded {
                max_scalar_values: 512,
            },
            details_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            capability: None,
            recoveries: (code == "session_required")
                .then(|| ApplicationRecoveryDecl::Operation {
                    operation_id: "session.start".to_string(),
                })
                .into_iter()
                .collect(),
            recovery_cardinality: RecoveryCardinality::AtMostOne,
        })
        .collect()
}

fn restore_released_output_definitions(
    operation: &str,
    observed_surface: &Value,
    schema: &mut Value,
) {
    let surface_name = [
        "interact",
        "console",
        "network",
        "emulation",
        "performance",
        "memory",
        "screencast",
        "artifacts",
    ]
    .into_iter()
    .find(|group| operation.starts_with(&format!("{group}_")))
    .unwrap_or(operation);
    let Some(definitions) = observed_surface["tools"]
        .as_array()
        .and_then(|tools| tools.iter().find(|tool| tool["name"] == surface_name))
        .and_then(|tool| tool["outputSchema"]["$defs"].as_object())
    else {
        return;
    };
    let mut used = std::collections::BTreeSet::new();
    replace_released_definitions(schema, definitions, &mut used, true);
    if used.is_empty() {
        return;
    }
    let retained = used
        .into_iter()
        .map(|name| (name.clone(), definitions[&name].clone()))
        .collect();
    schema
        .as_object_mut()
        .expect("VBL result schema root")
        .insert("$defs".to_string(), Value::Object(retained));
}

fn replace_released_definitions(
    value: &mut Value,
    definitions: &serde_json::Map<String, Value>,
    used: &mut std::collections::BTreeSet<String>,
    root: bool,
) {
    if !root
        && let Some((name, _)) = definitions
            .iter()
            .find(|(_, definition)| *definition == value)
    {
        used.insert(name.clone());
        *value = json!({ "$ref": format!("#/$defs/{name}") });
        return;
    }
    match value {
        Value::Object(object) => {
            for nested in object.values_mut() {
                replace_released_definitions(nested, definitions, used, false);
            }
        }
        Value::Array(values) => {
            for nested in values {
                replace_released_definitions(nested, definitions, used, false);
            }
        }
        _ => {}
    }
}

/// Declares the released 27-tool hybrid mapping over the authored catalog.
pub fn surface(
    registry: &CommandRegistry,
    observed_surface: &Value,
) -> mcp_twill::Result<NativeToolSurface> {
    surface_impl(registry, observed_surface, false)
}

pub fn host_surface(
    registry: &CommandRegistry,
    observed_surface: &Value,
) -> mcp_twill::Result<NativeToolSurface> {
    surface_impl(registry, observed_surface, true)
}

fn surface_impl(
    registry: &CommandRegistry,
    observed_surface: &Value,
    host_adoption: bool,
) -> mcp_twill::Result<NativeToolSurface> {
    let paths = OPERATION_MAPPING
        .iter()
        .map(|(released, path, _)| (*released, path.replace(' ', ".")))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut builder = if host_adoption {
        let (_, mut binding) = ambient_session_adoption();
        binding.mode = ResourceBindingMode::Ambient {
            context: mcp_twill::AmbientContextSource::ConversationIdentity,
            explicit: ExplicitCarrierPolicy::Omitted,
            // RFC 0019 owns the profile-scoped runtime-bounded
            // `session_required` use. It rewrites this framework-owned
            // missing-source recovery without broadening RFC 0016's
            // declaration-summary-only command emitter.
            missing_error: None,
        };
        NativeToolSurface::builder_from(NativeToolSurfaceDecl {
            name: "vbl-vscode".to_string(),
            tools: Vec::new(),
            exposure: NativeExposurePolicy::Complete,
            framework_help: FrameworkHelpProjection::Omitted,
            application_errors: NativeApplicationErrorDialect::FlatSingleRecovery,
            group_description_dialect: NativeGroupDescriptionDialect::AuthoredVerbatim,
            confirmation: NativeConfirmationRoute::Bridge,
            resource_bindings: vec![binding],
            task_delivery: TaskDeliveryDecl::Disabled,
        })
        .attach_resource_binder::<Session>(SessionBinder)
    } else {
        NativeToolSurface::builder("vbl")
            .framework_help(FrameworkHelpProjection::Omitted)
            .application_errors(NativeApplicationErrorDialect::FlatSingleRecovery)
            .group_description_dialect(NativeGroupDescriptionDialect::AuthoredVerbatim)
            .confirmation_route(NativeConfirmationRoute::Bridge)
    };
    for tool in observed_surface["tools"]
        .as_array()
        .expect("VBL surface tools")
    {
        let name = tool["name"].as_str().expect("VBL surface tool name");
        let title = tool["annotations"]["title"]
            .as_str()
            .expect("VBL surface tool title")
            .to_string();
        let description = tool["description"]
            .as_str()
            .expect("VBL surface tool description")
            .to_string();
        let selectors = tool["inputSchema"]["properties"]["operation"]["enum"].as_array();
        if let Some(selectors) = selectors.filter(|selectors| selectors.len() >= 2) {
            builder = builder.group(name, |group| {
                group
                    .selector("operation")
                    .title(title)
                    .description(description);
                for selector in selectors {
                    let selector = selector.as_str().expect("VBL selector");
                    let operation = format!("{name}_{selector}");
                    group.member(selector, &paths[operation.as_str()]);
                }
            });
        } else {
            let operation = selectors
                .and_then(|selectors| selectors.first())
                .and_then(Value::as_str)
                .map(|selector| format!("{name}_{selector}"));
            let operation = operation.as_deref().unwrap_or(name);
            builder = builder.tool(mcp_twill::NativeToolDecl::Direct {
                name: name.to_string(),
                operation_id: paths[operation].clone(),
                title: Some(title),
                description: Some(description),
            });
        }
    }
    builder.build(registry, McpProtocolTarget::V2025_11_25)
}
