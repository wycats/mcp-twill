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
                    format!(
                        "required argument `{}` missing from arguments help",
                        arg.name
                    ),
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
                Ok(plan) if plan.idempotent != operation.idempotent => {
                    violations.push(violation(
                        Some(&operation.id),
                        "plan",
                        format!(
                            "example `{}` plans with idempotent={}, but the operation declares {}",
                            example.command, plan.idempotent, operation.idempotent
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
/// MCP adapter can actually serve, and idempotency declarations project into
/// the catalog agents read.
pub fn check_effect_metadata(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    if let Err(error) = registry.validate_effects() {
        violations.push(violation(None, "permissions", error.to_string()));
    }
    let catalog_json: serde_json::Value = registry
        .resource_text("cli://catalog")
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let empty = Vec::new();
    let projected_operations = catalog_json["operations"].as_array().unwrap_or(&empty);
    for operation in registry.operation_specs() {
        if operation.permissions.is_empty() && operation.effect != crate::EffectSpec::Pure {
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
        // An idempotency declaration only helps a supervisor if it survives
        // into the projection agents actually read.
        if operation.idempotent {
            let projected = projected_operations
                .iter()
                .find(|candidate| candidate["id"] == operation.id.as_str());
            if projected.map(|op| &op["idempotent"]) != Some(&serde_json::json!(true)) {
                violations.push(violation(
                    Some(&operation.id),
                    "catalog",
                    "idempotent declaration missing from the catalog projection",
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

    let mut required: std::collections::BTreeSet<EffectLane> = std::collections::BTreeSet::new();
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
                format!(
                    "lane tool `{}` is not advertised through list_tools",
                    lane_spec.tool_name
                ),
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
    if let Err(error) = registry.validate_workspaces() {
        violations.push(violation(None, "workspaces", error.to_string()));
    }
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
    violations.extend(check_type_projection(registry));
    violations.extend(check_capability_projection(registry));
    violations.extend(check_runtime_identity(registry));
    violations
}

/// Named types honor the registration promises (every referenced type
/// exists, no dead types) and every argument schema is fully inlined:
/// unions appear only as property-level `oneOf`, never behind `$ref`,
/// `$defs`, or a top-level `oneOf`.
pub fn check_type_projection(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    if let Err(error) = registry.validate_types() {
        violations.push(violation(None, "types", error.to_string()));
        // The schema inliner assumes a validated type graph (no cycles,
        // no dangling references); projecting anyway could recurse forever.
        return violations;
    }
    for command in registry.command_specs() {
        let schema = registry.arg_schema(command);
        if schema.get("oneOf").is_some() {
            violations.push(violation(
                Some(&command.path.join(" ")),
                "schema",
                "argument schema has a top-level `oneOf`; unions must inline at the property level",
            ));
        }
        for forbidden in ["$ref", "$defs"] {
            if schema_contains_key(&schema, forbidden) {
                violations.push(violation(
                    Some(&command.path.join(" ")),
                    "schema",
                    format!(
                        "argument schema contains `{forbidden}`; named types must be fully inlined"
                    ),
                ));
            }
        }
    }
    violations
}

/// Whether any object in the JSON tree has `key` as a property name.
/// Matching on keys (not the rendered string) avoids false positives when
/// the text appears inside a description.
fn schema_contains_key(value: &serde_json::Value, key: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.contains_key(key) || map.values().any(|child| schema_contains_key(child, key))
        }
        serde_json::Value::Array(items) => {
            items.iter().any(|child| schema_contains_key(child, key))
        }
        _ => false,
    }
}

/// Resource declarations honor the registration promises: every resource
/// with lifecycle edges renders in server help, every command's resource
/// fields render in its command help, listing producers carry the reference
/// array in structured output, and grant URIs round-trip through the
/// derived reference type (mint → parse → same id).
pub fn check_resource_projection(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    if let Err(error) = registry.validate_resources() {
        violations.push(violation(None, "resources", error.to_string()));
        return violations;
    }
    let server_help = registry.help(crate::HelpRequest {
        command: None,
        topic: None,
        detail: None,
    });
    for decl in registry.resource_decls() {
        let name = &decl.name;
        let has_edges = !registry.resource_granters(name).is_empty()
            || !registry.resource_releasers(name).is_empty()
            || !registry.resource_enumerators(name).is_empty()
            || !registry.resource_requirers(name).is_empty();
        if has_edges && !server_help.text.contains(&format!("`{name}`")) {
            violations.push(violation(
                None,
                "resources",
                format!("server help does not render resource `{name}`"),
            ));
        }
        // Mint → parse must be exact inverses for the derived reference type
        // to treat ids and URIs as interchangeable.
        let probe = "probe-id_0.~";
        match decl.mint_uri(probe) {
            Ok(uri) => {
                if decl.parse_uri(&uri) != Some(probe) {
                    violations.push(violation(
                        None,
                        "resources",
                        format!("resource `{name}` URI template does not round-trip minted ids"),
                    ));
                }
            }
            Err(error) => violations.push(violation(
                None,
                "resources",
                format!("resource `{name}` cannot mint a URI from a conforming id: {error}"),
            )),
        }
    }
    for command in registry.command_specs() {
        let resource_fields: Vec<(&str, &Vec<String>)> = [
            ("requires", &command.requires_resources),
            ("grants", &command.grants),
            ("releases", &command.releases),
            ("enumerates", &command.enumerates),
        ]
        .into_iter()
        .filter(|(_, names)| !names.is_empty())
        .collect();
        if resource_fields.is_empty() {
            continue;
        }
        let name = command.path.join(" ");
        let help = registry.help(crate::HelpRequest {
            command: Some(name.clone()),
            topic: None,
            detail: None,
        });
        for (field, resources) in resource_fields {
            for resource in resources {
                if !help.text.contains(&format!("`{resource}`")) {
                    violations.push(violation(
                        Some(&name),
                        "resources",
                        format!("command help does not render {field} edge for `{resource}`"),
                    ));
                }
            }
        }
        if !command.enumerates.is_empty() {
            // Structured output is the shared CommandOutput envelope; the
            // reference array must be a schema-visible field, not folklore.
            let schema = serde_json::to_value(schemars::schema_for!(crate::CommandOutput))
                .unwrap_or(serde_json::Value::Null);
            if !schema_contains_key(&schema, "listings") {
                violations.push(violation(
                    Some(&name),
                    "resources",
                    "listing producer's output schema does not carry the reference array",
                ));
            }
        }
    }
    violations
}

/// Capability declarations honor the registration promises (declared,
/// provided, consumed, carried by a required argument) and every command
/// that requires a capability names it in its rendered help.
pub fn check_capability_projection(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    if let Err(error) = registry.validate_capabilities() {
        violations.push(violation(None, "capabilities", error.to_string()));
        return violations;
    }
    let catalog = registry.catalog();
    for command in registry.command_specs() {
        if command.requires.is_empty() {
            continue;
        }
        let name = command.path.join(" ");
        let help = registry.help(crate::HelpRequest {
            command: Some(name.clone()),
            topic: None,
            detail: None,
        });
        for capability in &command.requires {
            if !help.text.contains(capability.as_str()) {
                violations.push(violation(
                    Some(&name),
                    "capabilities",
                    format!("command help does not mention required capability `{capability}`"),
                ));
            }
        }
        let operation = catalog
            .operations
            .iter()
            .find(|operation| operation.id == name.replace(' ', "."));
        match operation {
            Some(operation) if operation.requires == command.requires => {}
            Some(_) => violations.push(violation(
                Some(&name),
                "capabilities",
                "catalog operation does not project this command's `requires` declarations",
            )),
            None => violations.push(violation(
                Some(&name),
                "capabilities",
                "command missing from catalog operations",
            )),
        }
    }
    violations
}

/// Guidance declarations honor the registration promises: routing edges
/// resolve to catalog commands, every declared edge appears in rendered
/// help (including the derived reverse edges on preferred commands), the
/// catalog projects the declarations, and the server preamble does not
/// inline command names that belong in per-command guidance.
pub fn check_guidance_projection(registry: &CommandRegistry) -> Vec<ContractViolation> {
    let mut violations = Vec::new();
    if let Err(error) = registry.validate_guidance() {
        violations.push(violation(None, "guidance", error.to_string()));
        return violations;
    }
    let catalog = registry.catalog();
    if let Some(preamble) = registry.preamble() {
        for command in registry.command_specs() {
            let name = command.path.join(" ");
            if preamble.contains(&format!("`{name}`")) {
                violations.push(violation(
                    None,
                    "guidance",
                    format!(
                        "server preamble names command `{name}`; per-command steering belongs on the command, not the preamble"
                    ),
                ));
            }
        }
    }
    for command in registry.command_specs() {
        let name = command.path.join(" ");
        let has_guidance = command.use_when.is_some()
            || !command.alternatives.is_empty()
            || command.fallback.is_some();
        let reverse_edges = registry.derived_fallback_edges(&name);
        if has_guidance || !reverse_edges.is_empty() {
            let help = registry.help(crate::HelpRequest {
                command: Some(name.clone()),
                topic: None,
                detail: None,
            });
            if let Some(use_when) = &command.use_when
                && !help.text.contains(use_when.as_str())
            {
                violations.push(violation(
                    Some(&name),
                    "guidance",
                    "command help does not render the `use_when` condition",
                ));
            }
            for alternative in &command.alternatives {
                if !help.text.contains(&format!("`{}`", alternative.command)) {
                    violations.push(violation(
                        Some(&name),
                        "guidance",
                        format!(
                            "command help does not route to alternative `{}`",
                            alternative.command
                        ),
                    ));
                }
            }
            if let Some(fallback) = &command.fallback {
                for preferred in &fallback.prefer {
                    if !help.text.contains(&format!("`{preferred}`")) {
                        violations.push(violation(
                            Some(&name),
                            "guidance",
                            format!("command help does not name preferred command `{preferred}`"),
                        ));
                    }
                }
            }
            for (source, _) in &reverse_edges {
                if !help.text.contains(&format!("`{source}`")) {
                    violations.push(violation(
                        Some(&name),
                        "guidance",
                        format!(
                            "command help does not render derived fallback edge from `{source}`"
                        ),
                    ));
                }
            }
        }
        let operation = catalog
            .operations
            .iter()
            .find(|operation| operation.id == name.replace(' ', "."));
        match operation {
            Some(operation)
                if operation.use_when == command.use_when
                    && operation.alternatives == command.alternatives
                    && operation.fallback == command.fallback => {}
            Some(_) => violations.push(violation(
                Some(&name),
                "guidance",
                "catalog operation does not project this command's guidance declarations",
            )),
            None => violations.push(violation(
                Some(&name),
                "guidance",
                "command missing from catalog operations",
            )),
        }
    }
    if catalog.server.preamble.as_deref() != registry.preamble() {
        violations.push(violation(
            None,
            "guidance",
            "catalog does not project the server preamble",
        ));
    }
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
            $crate::contract::assert_no_violations($crate::contract::check_discovery(&$registry()));
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
        fn contract_type_projection() {
            $crate::contract::assert_no_violations($crate::contract::check_type_projection(
                &$registry(),
            ));
        }

        #[test]
        fn contract_capability_projection() {
            $crate::contract::assert_no_violations($crate::contract::check_capability_projection(
                &$registry(),
            ));
        }

        #[test]
        fn contract_resource_projection() {
            $crate::contract::assert_no_violations($crate::contract::check_resource_projection(
                &$registry(),
            ));
        }

        #[test]
        fn contract_guidance_projection() {
            $crate::contract::assert_no_violations($crate::contract::check_guidance_projection(
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
