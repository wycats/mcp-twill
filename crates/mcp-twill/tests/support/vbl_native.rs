//! RFC 0015's test-only native-surface adoption of VBL v0.4.9.

use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, ArgSpec, CommandRegistry, CommandSpec,
    DynamicCommandFailure, FrameworkHelpProjection, McpProtocolTarget,
    NativeApplicationErrorDialect, NativeConfirmationRoute, NativeToolSurface, OutputContract,
    PermissionEffect, PermissionSpec,
};
use serde_json::{Value, json};

use crate::vbl::OPERATION_MAPPING;

/// Authors a Twill catalog from the released per-operation schemas.
pub fn registry(
    baseline: &Value,
    observed_surface: &Value,
    server_instructions: &str,
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
    for tool in baseline.as_array().expect("VBL baseline tools") {
        let name = tool["name"].as_str().expect("VBL tool name");
        let (path, _) = paths[name];
        let annotations = &tool["annotations"];
        let title = annotations["title"].as_str().expect("VBL tool title");
        let mut output_schema = tool["outputSchema"].clone();
        restore_released_output_definitions(name, observed_surface, &mut output_schema);
        let mut spec = CommandSpec::new(
            path.split_whitespace(),
            title,
            tool["description"].as_str().expect("VBL tool description"),
        )
        .with_output(OutputContract {
            application: Some(ApplicationResultContract::new(output_schema)),
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
        registry = registry.register_dynamic(spec, |_context| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
        });
    }
    registry
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
    let paths = OPERATION_MAPPING
        .iter()
        .map(|(released, path, _)| (*released, path.replace(' ', ".")))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut builder = NativeToolSurface::builder("vbl")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::FlatSingleRecovery)
        .confirmation_route(NativeConfirmationRoute::Bridge);
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
