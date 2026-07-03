use crate::{CliMcpServer, CommandRegistry, EffectLane, HelpRequest, HelpTopic, RunRequest};

/// One failed framework promise, named precisely enough to repair the drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    /// The catalog operation id, when the violation is operation-scoped.
    pub operation: Option<String>,
    /// The projection that disagrees with the catalog (help, resources, schema, lanes, examples).
    pub projection: &'static str,
    pub message: String,
}

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.operation {
            Some(operation) => write!(f, "[{}] {}: {}", self.projection, operation, self.message),
            None => write!(f, "[{}] {}", self.projection, self.message),
        }
    }
}

fn violation(
    operation: Option<&str>,
    projection: &'static str,
    message: impl Into<String>,
) -> ContractViolation {
    ContractViolation {
        operation: operation.map(ToOwned::to_owned),
        projection,
        message: message.into(),
    }
}

/// Every catalog operation appears in command resources and command help.
pub fn check_discovery(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    for operation in registry.operation_specs() {
        let name = operation.name();
        let resource_uri = format!("cli://commands/{}", operation.path.join("/"));
        // resource_text returns unknown-command error text rather than None for
        // a drifted path, so validate the content is this command's usage text.
        let usage_header = format!("# `{name}`");
        match registry.resource_text(&resource_uri) {
            Some(text) if text.contains(&usage_header) => {}
            Some(_) => {
                violations.push(violation(
                    Some(&operation.id),
                    "resources",
                    format!("`{resource_uri}` does not render usage text for `{name}`"),
                ));
            }
            None => {
                violations.push(violation(
                    Some(&operation.id),
                    "resources",
                    format!("`{name}` has no `{resource_uri}` resource"),
                ));
            }
        }
        let help = registry.help(HelpRequest {
            command: Some(name.clone()),
            topic: Some(HelpTopic::Usage),
            detail: None,
        });
        if help.title == "Unknown command" {
            violations.push(violation(
                Some(&operation.id),
                "help",
                format!("`{name}` is not reachable through command help"),
            ));
        }
    }
    violations
}

/// Every required argument appears in generated help and the serialized
/// catalog projection agents read from `cli://catalog`.
pub fn check_required_arguments(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    let catalog_json: serde_json::Value = registry
        .resource_text("cli://catalog")
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let empty = Vec::new();
    let projected_operations = catalog_json["operations"].as_array().unwrap_or(&empty);
    for operation in registry.operation_specs() {
        let name = operation.name();
        let help = registry.help(HelpRequest {
            command: Some(name.clone()),
            topic: Some(HelpTopic::Arguments),
            detail: None,
        });
        let projected = projected_operations
            .iter()
            .find(|candidate| candidate["id"] == operation.id.as_str());
        for arg in operation.args.iter().filter(|arg| arg.required) {
            // Match the exact rendered token from arguments_text, including the
            // closing backtick, so `foo` does not match a `foo2` line.
            if !help.text.contains(&format!("`$args.{}`", arg.name)) {
                violations.push(violation(
                    Some(&operation.id),
                    "help",
                    format!("required argument `{}` missing from arguments help", arg.name),
                ));
            }
            let projected_arg = projected.and_then(|operation| {
                operation["args"]
                    .as_array()?
                    .iter()
                    .find(|candidate| candidate["name"] == arg.name.as_str())
            });
            match projected_arg {
                Some(projection) if projection["required"] == serde_json::json!(true) => {}
                Some(_) => {
                    violations.push(violation(
                        Some(&operation.id),
                        "catalog",
                        format!(
                            "required argument `{}` is not marked required in the catalog projection",
                            arg.name
                        ),
                    ));
                }
                None => {
                    violations.push(violation(
                        Some(&operation.id),
                        "catalog",
                        format!(
                            "required argument `{}` missing from the catalog projection",
                            arg.name
                        ),
                    ));
                }
            }
        }
    }
    violations
}

/// Every example parses, binds, and plans; every operation can produce a dry-run plan.
pub fn check_examples_and_plans(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    for operation in registry.operation_specs() {
        let name = operation.name();
        for example in &operation.examples {
            let request = RunRequest {
                command: example.command.clone(),
                args: example.args.clone(),
                stdin: None,
                output: None,
                mode: crate::RunMode::DryRun,
                approval: None,
                dry_run: true,
            };
            match registry.build_plan(&request) {
                Err(error) => {
                    violations.push(violation(
                        Some(&operation.id),
                        "examples",
                        format!("example `{}` fails planning: {error}", example.command),
                    ));
                }
                Ok(plan) if plan.operation_id != operation.id => {
                    violations.push(violation(
                        Some(&operation.id),
                        "examples",
                        format!(
                            "example `{}` plans `{}`, not this operation",
                            example.command, plan.operation_id
                        ),
                    ));
                }
                Ok(_) => {}
            }
        }
        if operation.examples.is_empty() {
            // An operation with no examples must still dry-run plan from a
            // synthesized request when it has no required args to bind.
            if operation.args.iter().all(|arg| !arg.required) {
                let request = RunRequest {
                    command: name.clone(),
                    args: Default::default(),
                    stdin: None,
                    output: None,
                    mode: crate::RunMode::DryRun,
                    approval: None,
                    dry_run: true,
                };
                if let Err(error) = registry.build_plan(&request) {
                    violations.push(violation(
                        Some(&operation.id),
                        "planning",
                        format!("dry-run plan fails: {error}"),
                    ));
                }
            } else {
                violations.push(violation(
                    Some(&operation.id),
                    "examples",
                    "operation has required arguments but no examples proving they plan",
                ));
            }
        }
    }
    violations
}

/// Every operation has an effect classification and permission metadata the
/// MCP adapter can actually serve.
pub fn check_effect_metadata(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    if let Err(error) = registry.validate_effects() {
        violations.push(violation(None, "permissions", error.to_string()));
    }
    for operation in registry.operation_specs() {
        if operation.permissions.is_empty()
            && operation.effect != crate::EffectSpec::Pure
        {
            violations.push(violation(
                Some(&operation.id),
                "permissions",
                "non-pure operation declares no permissions",
            ));
        }
        for permission in &operation.permissions {
            if permission.description.trim().is_empty() {
                violations.push(violation(
                    Some(&operation.id),
                    "permissions",
                    format!("permission on `{}` has no description", permission.scope),
                ));
            }
        }
    }
    violations
}

/// Every required effect lane appears as a tool; no unused lane tool is generated;
/// annotations match worst-case truthful lane behavior.
pub fn check_effect_lanes(
    registry: &CommandRegistry,
    primary_tool_name: &str,
) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    let lanes = registry.lane_specs(primary_tool_name);

    let mut required: std::collections::BTreeSet<EffectLane> =
        std::collections::BTreeSet::new();
    required.insert(EffectLane::Primary);
    for operation in registry.operation_specs() {
        required.insert(operation.lane());
    }

    let generated: std::collections::BTreeSet<EffectLane> =
        lanes.iter().map(|lane| lane.lane).collect();

    for lane in required.difference(&generated) {
        violations.push(violation(
            None,
            "lanes",
            format!("required lane {lane:?} has no generated tool"),
        ));
    }
    for lane in generated.difference(&required) {
        violations.push(violation(
            None,
            "lanes",
            format!("lane {lane:?} generates a tool no operation needs"),
        ));
    }
    for lane in &lanes {
        if lane.tool_name != lane.lane.tool_name(primary_tool_name) {
            violations.push(violation(
                None,
                "lanes",
                format!(
                    "lane {:?} tool is named `{}`, expected `{}`",
                    lane.lane,
                    lane.tool_name,
                    lane.lane.tool_name(primary_tool_name)
                ),
            ));
        }
    }
    violations
}

/// The MCP server advertises every per-command resource and annotates each
/// lane tool with worst-case truthful hints. Registry-level checks validate
/// what URIs render; this validates what the server actually advertises.
pub fn check_server_projection(server: &CliMcpServer) -> Vec<ContractViolation> {
    let mut violations = Vec::new();

    let advertised: std::collections::BTreeSet<String> =
        server.resource_uris().into_iter().collect();
    for operation in server.registry().operation_specs() {
        let expected = format!("cli://commands/{}", operation.path.join("/"));
        if !advertised.contains(&expected) {
            violations.push(violation(
                Some(&operation.id),
                "resources",
                format!("`{expected}` is not advertised through list_resources"),
            ));
        }
    }

    let primary = &server.config().execution_tool_name;
    let tools = server.generated_tools();
    for lane_spec in server.registry().lane_specs(primary) {
        let Some(tool) = tools.iter().find(|tool| tool.name == lane_spec.tool_name) else {
            violations.push(violation(
                None,
                "lanes",
                format!("lane tool `{}` is not advertised through list_tools", lane_spec.tool_name),
            ));
            continue;
        };
        let Some(annotations) = tool.annotations.as_ref() else {
            violations.push(violation(
                None,
                "lanes",
                format!("lane tool `{}` has no annotations", lane_spec.tool_name),
            ));
            continue;
        };
        let (read_only, destructive, open_world) = match lane_spec.lane {
            EffectLane::Primary => (Some(true), Some(false), Some(false)),
            EffectLane::Write => (Some(false), Some(false), Some(false)),
            EffectLane::Delete => (Some(false), Some(true), Some(false)),
            EffectLane::Exec => (Some(false), Some(true), Some(true)),
            EffectLane::Network => (Some(false), Some(false), Some(true)),
        };
        let actual = (
            annotations.read_only_hint,
            annotations.destructive_hint,
            annotations.open_world_hint,
        );
        if actual != (read_only, destructive, open_world) {
            violations.push(violation(
                None,
                "lanes",
                format!(
                    "lane tool `{}` annotations {:?} do not match worst-case truthful {:?} for {:?}",
                    lane_spec.tool_name,
                    actual,
                    (read_only, destructive, open_world),
                    lane_spec.lane
                ),
            ));
        }
    }
    violations
}

/// The served `cli://catalog` resource projects the registry's identity
/// hashes faithfully. Both sides compute hashes through the same function,
/// so this cannot catch hash-computation drift; it guards the serialization
/// projection: the `identity` field staying present, its serde names, and
/// the resource route continuing to serve the catalog.
pub fn check_runtime_identity(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    let identity = registry.runtime_identity();
    let Some(catalog_text) = registry.resource_text("cli://catalog") else {
        violations.push(violation(
            None,
            "runtime_identity",
            "`cli://catalog` resource is not served",
        ));
        return violations;
    };
    let catalog_json: serde_json::Value = match serde_json::from_str(&catalog_text) {
        Ok(value) => value,
        Err(error) => {
            violations.push(violation(
                None,
                "runtime_identity",
                format!("`cli://catalog` resource is not valid JSON: {error}"),
            ));
            return violations;
        }
    };
    let served = &catalog_json["identity"];

    let checks = [
        ("catalogHash", &identity.catalog_hash),
        ("runSchemaHash", &identity.run_schema_hash),
        ("helpSchemaHash", &identity.help_schema_hash),
    ];
    for (field, expected) in checks {
        match served[field].as_str() {
            Some(actual) if actual == expected => {}
            Some(actual) => {
                violations.push(violation(
                    None,
                    "runtime_identity",
                    format!(
                        "runtime identity `{field}` is `{expected}` but the served catalog reports `{actual}`"
                    ),
                ));
            }
            None => {
                violations.push(violation(
                    None,
                    "runtime_identity",
                    format!("served catalog has no `identity.{field}`"),
                ));
            }
        }
    }
    violations
}

/// Run every contract rule and aggregate the violations.
/// Every declared workspace projects into exactly one resolver requirement
/// whose id matches the declared name and whose fallback carries the declared
/// URI. Guards the WorkspaceDecl -> WorkspaceRequirement projection against
/// drift.
pub fn check_workspace_projection(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    let requirements = registry.workspace_requirements();
    for decl in registry.workspaces() {
        let matching: Vec<_> = requirements
            .iter()
            .filter(|requirement| requirement.id == decl.name.as_str())
            .collect();
        match matching.len() {
            1 => {
                let requirement = matching[0];
                match &requirement.fallback {
                    Some(fallback) if fallback.uri == decl.uri => {}
                    Some(fallback) => {
                        violations.push(violation(
                            None,
                            "workspaces",
                            format!(
                                "workspace `{}` fallback URI `{}` does not match declared URI `{}`",
                                decl.name, fallback.uri, decl.uri
                            ),
                        ));
                    }
                    None => {
                        violations.push(violation(
                            None,
                            "workspaces",
                            format!(
                                "workspace `{}` projects a requirement without a declared fallback",
                                decl.name
                            ),
                        ));
                    }
                }
            }
            0 => {
                violations.push(violation(
                    None,
                    "workspaces",
                    format!("workspace `{}` projects no resolver requirement", decl.name),
                ));
            }
            _ => {
                violations.push(violation(
                    None,
                    "workspaces",
                    format!(
                        "workspace `{}` projects {} resolver requirements; expected exactly one",
                        decl.name,
                        matching.len()
                    ),
                ));
            }
        }
    }
    violations
}

pub fn verify_catalog_coverage(
    registry: &CommandRegistry,
    primary_tool_name: &str,
) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    violations.extend(check_discovery(registry));
    violations.extend(check_required_arguments(registry));
    violations.extend(check_examples_and_plans(registry));
    violations.extend(check_effect_metadata(registry));
    violations.extend(check_effect_lanes(registry, primary_tool_name));
    violations.extend(check_workspace_projection(registry));
    violations.extend(check_runtime_identity(registry));
    violations
}

fn render_violations(violations: &[ContractViolation]) -> String {
    violations
        .iter()
        .map(|violation| format!("- {violation}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Panic with a repair-oriented message when any violations exist.
/// Used by the `contract_tests!` macro; also callable directly.
pub fn assert_no_violations(violations: Vec<ContractViolation>) {
    if !violations.is_empty() {
        panic!(
            "catalog contract violations:\n{}",
            render_violations(&violations)
        );
    }
}

/// Generate one `#[test]` per contract rule for a registry constructor.
///
/// ```ignore
/// fn registry() -> mcp_twill::CommandRegistry { /* ... */ }
/// mcp_twill::contract_tests!(registry);
/// // or with a custom primary tool name:
/// mcp_twill::contract_tests!(registry, "repo");
/// ```
#[macro_export]
macro_rules! contract_tests {
    ($registry:path) => {
        $crate::contract_tests!($registry, "run");
    };
    ($registry:path, $primary:expr) => {
        #[test]
        fn contract_discovery() {
            $crate::contract::assert_no_violations($crate::contract::check_discovery(
                &$registry(),
            ));
        }

        #[test]
        fn contract_required_arguments() {
            $crate::contract::assert_no_violations($crate::contract::check_required_arguments(
                &$registry(),
            ));
        }

        #[test]
        fn contract_examples_and_plans() {
            $crate::contract::assert_no_violations($crate::contract::check_examples_and_plans(
                &$registry(),
            ));
        }

        #[test]
        fn contract_effect_metadata() {
            $crate::contract::assert_no_violations($crate::contract::check_effect_metadata(
                &$registry(),
            ));
        }

        #[test]
        fn contract_effect_lanes() {
            $crate::contract::assert_no_violations($crate::contract::check_effect_lanes(
                &$registry(),
                $primary,
            ));
        }

        #[test]
        fn contract_workspace_projection() {
            $crate::contract::assert_no_violations($crate::contract::check_workspace_projection(
                &$registry(),
            ));
        }

        #[test]
        fn contract_runtime_identity() {
            $crate::contract::assert_no_violations($crate::contract::check_runtime_identity(
                &$registry(),
            ));
        }

        #[test]
        fn contract_server_projection() {
            let server = $crate::CliMcpServer::with_config(
                $registry(),
                $crate::CliMcpServerConfig::default().with_execution_tool_name($primary),
            )
            .expect("registry must be servable through the MCP adapter");
            $crate::contract::assert_no_violations($crate::contract::check_server_projection(
                &server,
            ));
        }
    };
}
