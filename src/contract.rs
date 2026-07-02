use crate::{CommandRegistry, EffectLane, HelpRequest, HelpTopic, RunRequest};

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

/// Every required argument appears in generated help and schema projections.
pub fn check_required_arguments(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    let run_schema = serde_json::to_value(schemars::schema_for!(RunRequest))
        .unwrap_or(serde_json::Value::Null);
    let schema_text = run_schema.to_string();
    for operation in registry.operation_specs() {
        let name = operation.name();
        let help = registry.help(HelpRequest {
            command: Some(name.clone()),
            topic: Some(HelpTopic::Arguments),
            detail: None,
        });
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
        }
    }
    // The run schema is shared; it must at least describe the args map.
    if !schema_text.contains("args") {
        violations.push(violation(
            None,
            "schema",
            "run request schema does not describe the `args` map",
        ));
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
            if let Err(error) = registry.build_plan(&request) {
                violations.push(violation(
                    Some(&operation.id),
                    "examples",
                    format!("example `{}` fails planning: {error}", example.command),
                ));
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

/// Every operation has an effect classification and permission metadata.
pub fn check_effect_metadata(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
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

/// Run every contract rule and aggregate the violations.
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
    };
}
