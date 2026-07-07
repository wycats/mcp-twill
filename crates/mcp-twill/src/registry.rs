use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use async_trait::async_trait;
use mcp_workspace_resolver::{
    ResolvedWorkspaceSet, WorkspaceObservationSet, WorkspaceRequirement, normalize_file_uri,
    path_has_prefix, resolve_workspaces,
};
use schemars::schema_for;
use serde_json::{Value, json};

use crate::{
    CatalogIdentity, CommandCatalog, CommandContext, CommandGuidance, CommandOutput, CommandSpec,
    EffectLane, FrameworkError, HelpRequest, HelpResult, HelpTopic, InvocationPlan,
    InvocationToken, OperationSpec, PermissionPolicy, Result, RunRequest, RunResponse, ServerSpec,
    TemplateToken, ToolLaneSpec, WorkspaceDecl, group_namespaces, stable_hash_value,
    structured_error, value_matches_type,
};
use crate::{CommandTemplate, PermissionSpec};

pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<CommandOutput>> + Send>>;

#[async_trait]
pub trait CommandHandler: Send + Sync + 'static {
    async fn call(&self, context: CommandContext) -> Result<CommandOutput>;
}

#[async_trait]
impl<F, Fut> CommandHandler for F
where
    F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<CommandOutput>> + Send,
{
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        (self)(context).await
    }
}

#[derive(Clone)]
pub struct CommandRegistry {
    server_name: String,
    server_description: String,
    commands: BTreeMap<Vec<String>, RegisteredCommand>,
    workspaces: BTreeMap<String, WorkspaceDecl>,
    types: BTreeMap<String, crate::TypeDecl>,
    duplicate_types: Vec<String>,
    capabilities: BTreeMap<String, crate::CapabilityDecl>,
    duplicate_capabilities: Vec<String>,
    guidance: Vec<CommandGuidance>,
    policy: PermissionPolicy,
}

#[derive(Clone)]
struct RegisteredCommand {
    spec: CommandSpec,
    handler: Arc<dyn CommandHandler>,
}

impl CommandRegistry {
    pub fn new(server_name: impl Into<String>, server_description: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
            server_description: server_description.into(),
            commands: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            types: BTreeMap::new(),
            duplicate_types: Vec::new(),
            capabilities: BTreeMap::new(),
            duplicate_capabilities: Vec::new(),
            guidance: Vec::new(),
            policy: PermissionPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: PermissionPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn declare_workspace(mut self, workspace: WorkspaceDecl) -> Self {
        self.workspaces.insert(workspace.name.clone(), workspace);
        self
    }

    pub fn declare_type(mut self, decl: crate::TypeDecl) -> Self {
        if self.types.contains_key(&decl.name) {
            self.duplicate_types.push(decl.name.clone());
        }
        self.types.insert(decl.name.clone(), decl);
        self
    }

    pub fn declare_capability(mut self, decl: crate::CapabilityDecl) -> Self {
        if self.capabilities.contains_key(&decl.name) {
            self.duplicate_capabilities.push(decl.name.clone());
        }
        self.capabilities.insert(decl.name.clone(), decl);
        self
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &crate::CapabilityDecl> {
        self.capabilities.values()
    }

    pub fn types(&self) -> impl Iterator<Item = &crate::TypeDecl> {
        self.types.values()
    }

    pub fn declare_guidance(mut self, guidance: CommandGuidance) -> Self {
        self.guidance.push(guidance);
        self
    }

    pub fn guidance(&self) -> &[CommandGuidance] {
        &self.guidance
    }

    pub fn register<H>(mut self, spec: CommandSpec, handler: H) -> Self
    where
        H: CommandHandler,
    {
        self.commands.insert(
            spec.path.clone(),
            RegisteredCommand {
                spec,
                handler: Arc::new(handler),
            },
        );
        self
    }

    pub fn command_specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.values().map(|command| &command.spec)
    }

    pub fn operation_specs(&self) -> Vec<OperationSpec> {
        let mut operations = self
            .commands
            .values()
            .map(|command| OperationSpec::from_command_spec(&command.spec))
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| left.path.cmp(&right.path));
        operations
    }

    pub fn workspaces(&self) -> impl Iterator<Item = &WorkspaceDecl> {
        self.workspaces.values()
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn server_description(&self) -> &str {
        &self.server_description
    }

    pub fn catalog(&self) -> CommandCatalog {
        let operations = self.operation_specs();
        let identity = self.catalog_identity_for(&operations);
        CommandCatalog {
            server: ServerSpec::new(&self.server_name, &self.server_description),
            namespaces: group_namespaces(&operations),
            operations,
            workspaces: self.workspaces.values().cloned().collect(),
            types: self.types.values().cloned().collect(),
            capabilities: self.capabilities.values().cloned().collect(),
            guidance: self.guidance.clone(),
            identity,
        }
    }

    pub fn catalog_identity(&self) -> CatalogIdentity {
        let operations = self.operation_specs();
        self.catalog_identity_for(&operations)
    }

    /// The identity a bare registry can report: name and the catalog and
    /// schema hashes. Process facts stay unset without a runtime host.
    pub fn runtime_identity(&self) -> crate::RuntimeIdentity {
        crate::RuntimeIdentity::for_registry(self)
    }

    fn catalog_identity_for(&self, operations: &[OperationSpec]) -> CatalogIdentity {
        // The hash preimage is this hand-built value, not the serialized
        // `CommandCatalog` served at cli://catalog (which skips empty fields
        // and embeds the identity itself). The hash is an opaque change
        // detector; clients cannot recompute it from the resource bytes.
        let catalog_value = json!({
            "server": ServerSpec::new(&self.server_name, &self.server_description),
            "namespaces": group_namespaces(operations),
            "operations": operations,
            "workspaces": self.workspaces.values().collect::<Vec<_>>(),
            "types": self.types.values().collect::<Vec<_>>(),
            "capabilities": self.capabilities.values().collect::<Vec<_>>(),
            "guidance": self.guidance,
        });
        let run_schema = serde_json::to_value(schema_for!(RunRequest)).unwrap_or(Value::Null);
        let help_schema = serde_json::to_value(schema_for!(HelpRequest)).unwrap_or(Value::Null);

        CatalogIdentity {
            catalog_hash: stable_hash_value(&catalog_value),
            run_schema_hash: stable_hash_value(&run_schema),
            help_schema_hash: stable_hash_value(&help_schema),
        }
    }

    pub fn lane_specs(&self, primary_tool_name: &str) -> Vec<ToolLaneSpec> {
        let mut lanes = BTreeMap::<EffectLane, Vec<_>>::new();
        lanes.entry(EffectLane::Primary).or_default();
        for operation in self.operation_specs() {
            lanes
                .entry(operation.lane())
                .or_default()
                .push(operation.effect);
        }

        lanes
            .into_iter()
            .map(|(lane, mut allowed_effects)| {
                allowed_effects.sort();
                allowed_effects.dedup();
                ToolLaneSpec {
                    tool_name: lane.tool_name(primary_tool_name),
                    lane,
                    allowed_effects,
                    description: lane_description(primary_tool_name, lane),
                }
            })
            .collect()
    }

    pub fn tool_lane(&self, primary_tool_name: &str, tool_name: &str) -> Option<EffectLane> {
        self.lane_specs(primary_tool_name)
            .into_iter()
            .find(|spec| spec.tool_name == tool_name)
            .map(|spec| spec.lane)
    }

    pub fn required_tool_name(&self, primary_tool_name: &str, lane: EffectLane) -> String {
        lane.tool_name(primary_tool_name)
    }

    pub fn validate_examples(&self) -> Result<()> {
        for command in self.commands.values() {
            for example in &command.spec.examples {
                let request = RunRequest {
                    command: example.command.clone(),
                    args: example.args.clone(),
                    stdin: None,
                    output: None,
                    mode: crate::RunMode::DryRun,
                    approval: None,
                    dry_run: true,
                };
                self.build_plan(&request)?;
            }
        }
        Ok(())
    }

    /// Validates the type declarations against every rule registration
    /// promises: unique names, non-empty unions, resolvable references,
    /// no cycles, no dead types, no ambiguous variant pairs.
    pub fn validate_types(&self) -> Result<()> {
        if let Some(name) = self.duplicate_types.first() {
            return Err(FrameworkError::Build(format!(
                "type `{name}` is declared more than once"
            )));
        }
        let mut arg_references = Vec::new();
        for command in self.commands.values() {
            for arg in &command.spec.args {
                if let crate::ArgType::Named(type_name) = &arg.value_type {
                    arg_references.push((
                        format!("command `{}` argument `{}`", command.spec.name(), arg.name),
                        type_name.clone(),
                    ));
                }
            }
        }
        crate::types::validate_types(&self.types, &arg_references)
    }

    /// Validates command-declared workspace requirements: every name passed
    /// to `uses_workspace` must match a server-declared workspace, and a
    /// command must not declare the same workspace twice.
    pub fn validate_workspaces(&self) -> Result<()> {
        for command in self.commands.values() {
            let mut seen = std::collections::BTreeSet::new();
            for workspace_name in &command.spec.workspaces {
                if !self.workspaces.contains_key(workspace_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` uses workspace `{workspace_name}`, which is not declared on the server",
                        command.spec.name()
                    )));
                }
                if !seen.insert(workspace_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` declares workspace `{workspace_name}` more than once",
                        command.spec.name()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Validates capability declarations: every `requires`/`provides` name
    /// must match a declared capability, a requiring command must carry the
    /// capability's carrier as a required argument, and every capability
    /// needs both a provider and a consumer.
    pub fn validate_capabilities(&self) -> Result<()> {
        if let Some(name) = self.duplicate_capabilities.first() {
            return Err(FrameworkError::Build(format!(
                "capability `{name}` is declared more than once"
            )));
        }
        for capability in self.capabilities.values() {
            if capability.carrier.is_empty() {
                return Err(FrameworkError::Build(format!(
                    "capability `{}` does not declare a carrier argument; use `carried_by` to name the argument that carries it",
                    capability.name
                )));
            }
        }
        for command in self.commands.values() {
            let mut seen = std::collections::BTreeSet::new();
            for capability_name in &command.spec.requires {
                let Some(capability) = self.capabilities.get(capability_name) else {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` requires capability `{capability_name}`, which is not declared on the server",
                        command.spec.name()
                    )));
                };
                if !seen.insert(capability_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` requires capability `{capability_name}` more than once",
                        command.spec.name()
                    )));
                }
                let carrier = command
                    .spec
                    .args
                    .iter()
                    .find(|arg| arg.name == capability.carrier);
                match carrier {
                    None => {
                        return Err(FrameworkError::Build(format!(
                            "command `{}` requires capability `{capability_name}` but has no `{}` argument to carry it",
                            command.spec.name(),
                            capability.carrier
                        )));
                    }
                    Some(arg) if !arg.required => {
                        return Err(FrameworkError::Build(format!(
                            "command `{}` requires capability `{capability_name}` but its carrier argument `{}` is optional; a carrier must be required",
                            command.spec.name(),
                            capability.carrier
                        )));
                    }
                    Some(_) => {}
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            for capability_name in &command.spec.provides {
                if !self.capabilities.contains_key(capability_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` provides capability `{capability_name}`, which is not declared on the server",
                        command.spec.name()
                    )));
                }
                if !seen.insert(capability_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` provides capability `{capability_name}` more than once",
                        command.spec.name()
                    )));
                }
            }
        }
        for capability in self.capabilities.values() {
            if self.capability_providers(&capability.name).is_empty() {
                return Err(FrameworkError::Build(format!(
                    "capability `{}` has no providing command; declare `provides` on the command that establishes it",
                    capability.name
                )));
            }
            let has_bootstrap_provider = self.commands.values().any(|command| {
                command.spec.provides.contains(&capability.name)
                    && !command.spec.requires.contains(&capability.name)
            });
            if !has_bootstrap_provider {
                return Err(FrameworkError::Build(format!(
                    "capability `{}` has only providers that also require it; declare `provides` on a command that can establish it without an existing `{}`",
                    capability.name, capability.name
                )));
            }
            let consumed = self
                .commands
                .values()
                .any(|command| command.spec.requires.contains(&capability.name));
            if !consumed {
                return Err(FrameworkError::Build(format!(
                    "capability `{}` has no requiring command; remove the declaration or declare `requires` on the commands that need it",
                    capability.name
                )));
            }
        }
        Ok(())
    }

    /// The commands that establish a capability, derived from `provides`
    /// declarations. Steering and help name these commands so guidance can
    /// never drift from the declarations.
    pub fn capability_providers(&self, capability: &str) -> Vec<String> {
        self.commands
            .values()
            .filter(|command| {
                command
                    .spec
                    .provides
                    .iter()
                    .any(|provided| provided == capability)
            })
            .map(|command| command.spec.name())
            .collect()
    }

    /// One help line for a capability: summary, carrier argument, and the
    /// commands that establish it, all derived from declarations.
    fn capability_help_line(&self, capability: &str) -> String {
        let Some(decl) = self.capabilities.get(capability) else {
            return format!("`{capability}`");
        };
        let providers = self.capability_providers(capability);
        let establish = if providers.is_empty() {
            String::new()
        } else {
            format!(
                "; establish with {}",
                providers
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "`{}`: {} (carried by `{}`{establish})",
            decl.name, decl.summary, decl.carrier
        )
    }

    /// Fills in the carrier argument and establishing commands on a
    /// handler-raised `CapabilityDenied` so the response layer can locate
    /// the failure and derive establishment steering from declarations.
    /// Values the handler already set are preserved; only missing pieces
    /// are filled from the declarations.
    fn enrich_capability_denied(&self, error: FrameworkError) -> FrameworkError {
        let FrameworkError::CapabilityDenied {
            capability,
            detail,
            carrier,
            providers,
        } = error
        else {
            return error;
        };
        let carrier = carrier.or_else(|| {
            self.capabilities
                .get(&capability)
                .map(|decl| decl.carrier.clone())
        });
        let providers = if providers.is_empty() {
            self.capability_providers(&capability)
        } else {
            providers
        };
        FrameworkError::CapabilityDenied {
            capability,
            detail,
            carrier,
            providers,
        }
    }

    /// The model-facing JSON schema for one command's arguments. Named types
    /// are fully inlined at the property site as a property-level `oneOf`
    /// (array-wrapped when repeated); no `$ref` indirection and no top-level
    /// `oneOf` appear anywhere in the output.
    pub fn arg_schema(&self, spec: &crate::CommandSpec) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for arg in &spec.args {
            let base = match &arg.value_type {
                crate::ArgType::String | crate::ArgType::Path => json!({
                    "type": "string",
                    "description": arg.summary,
                }),
                crate::ArgType::Json => json!({
                    "description": arg.summary,
                }),
                crate::ArgType::Bool => json!({
                    "type": "boolean",
                    "description": arg.summary,
                }),
                crate::ArgType::Number => json!({
                    "type": "number",
                    "description": arg.summary,
                }),
                crate::ArgType::Named(type_name) => {
                    let mut schema = crate::types::inline_type_schema(type_name, &self.types);
                    // The property site carries the command-specific summary
                    // ("Element to click"), like every other argument; the
                    // type's own description lives on its variants.
                    if let Some(object) = schema.as_object_mut() {
                        object.insert("description".into(), json!(arg.summary));
                    }
                    schema
                }
            };
            let schema = if arg.repeated {
                json!({
                    "type": "array",
                    "description": arg.summary,
                    "items": base,
                })
            } else {
                base
            };
            properties.insert(arg.name.clone(), schema);
            if arg.required {
                required.push(Value::String(arg.name.clone()));
            }
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }

    pub fn validate_effects(&self) -> Result<()> {
        for command in self.commands.values() {
            for permission in &command.spec.permissions {
                if let crate::PermissionEffect::Custom(effect) = &permission.effect {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` declares custom effect `{effect}`; \
                         custom effects are not supported. Declare read, write, \
                         delete, exec, or network permissions instead",
                        command.spec.path.join(" ")
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn validate_guidance(&self) -> Result<()> {
        for guidance in &self.guidance {
            if guidance.kind != crate::GuidanceKind::RunCommand {
                continue;
            }
            let template = CommandTemplate::parse(&guidance.text).map_err(|error| {
                FrameworkError::Build(format!(
                    "guidance `{}` is marked runCommand but does not parse: {error}",
                    guidance.id
                ))
            })?;
            let Some(registered) = self.match_command(&template) else {
                return Err(FrameworkError::Build(format!(
                    "guidance `{}` is marked runCommand but `{}` matches no catalog command",
                    guidance.id, guidance.text
                )));
            };
            for placeholder in template.placeholders() {
                if registered.spec.arg(placeholder).is_none() {
                    return Err(FrameworkError::Build(format!(
                        "guidance `{}` references `$args.{placeholder}`, which `{}` does not declare",
                        guidance.id,
                        registered.spec.name()
                    )));
                }
            }
            let placeholders: BTreeSet<_> = template.placeholders().into_iter().collect();
            for arg in &registered.spec.args {
                if arg.required && !placeholders.contains(arg.name.as_str()) {
                    return Err(FrameworkError::Build(format!(
                        "guidance `{}` omits required argument `{}` of `{}`; the guidance would fail planning",
                        guidance.id,
                        arg.name,
                        registered.spec.name()
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn build_plan(&self, request: &RunRequest) -> Result<InvocationPlan> {
        self.build_plan_with_workspaces(request, &self.resolve_declared_workspaces())
    }

    /// Projects every declared workspace into a resolver requirement. The
    /// single-root convenience selection applies only when exactly one
    /// workspace is declared; with several, each must match by name so one
    /// client root cannot satisfy unrelated requirements.
    pub fn workspace_requirements(&self) -> Vec<WorkspaceRequirement> {
        let sole = self.workspaces.len() == 1;
        self.workspaces
            .values()
            .map(|decl| decl.requirement(sole))
            .collect()
    }

    /// The server-declared workspace roots as a resolver observation set.
    /// Runtime observations (MCP roots, Codex sandbox metadata) are layered
    /// on top of this set by the caller that observed them.
    pub fn declared_observations(&self) -> WorkspaceObservationSet {
        let mut observations = WorkspaceObservationSet::new();
        for decl in self.workspaces.values() {
            observations = observations.with_declared(decl.declared_root());
        }
        observations
    }

    /// Resolves declared workspaces with no runtime observations. This is the
    /// resolution `build_plan` uses when the caller has nothing runtime to
    /// contribute; declared roots resolve through the resolver's
    /// declared-roots path.
    pub fn resolve_declared_workspaces(&self) -> ResolvedWorkspaceSet {
        resolve_workspaces(
            &self.workspace_requirements(),
            &self.declared_observations(),
        )
    }

    /// Builds an invocation plan against pre-resolved workspace roots.
    /// Planning stays synchronous; observation gathering happens in the
    /// adapter and arrives here already resolved.
    pub fn build_plan_with_workspaces(
        &self,
        request: &RunRequest,
        resolved: &ResolvedWorkspaceSet,
    ) -> Result<InvocationPlan> {
        let template = CommandTemplate::parse(&request.command)?;
        let registered =
            self.match_command(&template)
                .ok_or_else(|| FrameworkError::UnknownCommand {
                    command: request.command.clone(),
                    nearest: self.nearest_commands(&template.literal_prefix()),
                })?;
        let operation = OperationSpec::from_command_spec(&registered.spec);
        let identity = self.catalog_identity();

        match (&registered.spec.stdin, &request.stdin) {
            (None, Some(_)) => {
                return Err(FrameworkError::StdinMismatch(format!(
                    "`{}` does not accept stdin",
                    registered.spec.name()
                )));
            }
            (Some(contract), Some(stdin)) => {
                if let Some(mime_type) = &stdin.mime_type
                    && mime_type != &contract.mime_type
                {
                    return Err(FrameworkError::StdinMismatch(format!(
                        "`{}` accepts {} stdin, got {mime_type}",
                        registered.spec.name(),
                        contract.mime_type
                    )));
                }
            }
            _ => {}
        }

        let referenced: BTreeSet<_> = template
            .placeholders()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        for arg_name in &referenced {
            if registered.spec.arg(arg_name).is_none() {
                return Err(FrameworkError::UnknownArgument(arg_name.clone()));
            }
        }
        for arg_name in request.args.keys() {
            if registered.spec.arg(arg_name).is_none() {
                return Err(FrameworkError::UnknownArgument(arg_name.clone()));
            }
            if !referenced.contains(arg_name) {
                return Err(FrameworkError::PlaceholderInterpolation(format!(
                    "$args.{arg_name}"
                )));
            }
        }

        // The request shape is valid, so a missing carrier gets the
        // capability diagnostic instead of the generic missing-argument one.
        for capability_name in &registered.spec.requires {
            let Some(capability) = self.capabilities.get(capability_name) else {
                continue;
            };
            if !request.args.contains_key(&capability.carrier) {
                return Err(FrameworkError::CapabilityMissing {
                    capability: capability.name.clone(),
                    carrier: capability.carrier.clone(),
                    providers: self.capability_providers(&capability.name),
                });
            }
        }

        for arg_name in &referenced {
            if !request.args.contains_key(arg_name) {
                return Err(FrameworkError::MissingArgument(arg_name.clone()));
            }
        }

        let mut bound_args = BTreeMap::new();
        let mut used_workspaces: BTreeSet<String> = BTreeSet::new();
        for spec in &registered.spec.args {
            let Some(value) = request.args.get(&spec.name) else {
                if spec.required {
                    return Err(FrameworkError::MissingArgument(spec.name.clone()));
                }
                continue;
            };
            value_matches_type(&spec.name, value, &spec.value_type)?;
            if let Some(workspace_name) = &spec.workspace {
                if !self.workspaces.contains_key(workspace_name) {
                    return Err(FrameworkError::WorkspaceMismatch {
                        argument: spec.name.clone(),
                        workspace: workspace_name.clone(),
                        selected_root: None,
                        path: None,
                        diagnostics: Vec::new(),
                    });
                }
                let value = value.as_str().ok_or_else(|| {
                    FrameworkError::InvalidArgumentType(
                        spec.name.clone(),
                        spec.value_type.expected_name(),
                    )
                })?;
                let workspace_id =
                    mcp_workspace_resolver::WorkspaceId::from(workspace_name.as_str());
                let Some(root) = resolved.root(&workspace_id) else {
                    // Resolution failed for this requirement: surface the
                    // resolver diagnostics that explain why.
                    let diagnostics = resolved
                        .diagnostics
                        .iter()
                        .filter(|diagnostic| {
                            diagnostic
                                .requirement
                                .as_ref()
                                .is_none_or(|id| id == &workspace_id)
                        })
                        .cloned()
                        .collect();
                    return Err(FrameworkError::WorkspaceMismatch {
                        argument: spec.name.clone(),
                        workspace: workspace_name.clone(),
                        selected_root: None,
                        path: Some(value.to_string()),
                        diagnostics,
                    });
                };
                let contained = match (
                    normalize_file_uri(&root.root_uri),
                    normalize_file_uri(value),
                ) {
                    (Ok(root_path), Ok(candidate)) => path_has_prefix(&candidate, &root_path),
                    (_, Err(err)) => {
                        // The path argument itself has a non-file scheme:
                        // surface the resolver's actionable code.
                        return Err(FrameworkError::WorkspaceMismatch {
                            argument: spec.name.clone(),
                            workspace: workspace_name.clone(),
                            selected_root: Some(root.root_uri.clone()),
                            path: Some(value.to_string()),
                            diagnostics: vec![
                                mcp_workspace_resolver::WorkspaceDiagnostic::unsupported_scheme(
                                    Some(workspace_id.clone()),
                                    err.to_string(),
                                    value.to_string(),
                                ),
                            ],
                        });
                    }
                    (Err(_), _) => false,
                };
                if !contained {
                    return Err(FrameworkError::WorkspaceMismatch {
                        argument: spec.name.clone(),
                        workspace: workspace_name.clone(),
                        selected_root: Some(root.root_uri.clone()),
                        path: Some(value.to_string()),
                        diagnostics: Vec::new(),
                    });
                }
                used_workspaces.insert(workspace_name.clone());
            }
            let variants = if let crate::ArgType::Named(type_name) = &spec.value_type {
                if spec.repeated {
                    let elements = value.as_array().ok_or_else(|| {
                        FrameworkError::InvalidArgumentType(
                            spec.name.clone(),
                            "an array of values matching a declared type",
                        )
                    })?;
                    let mut matched = Vec::with_capacity(elements.len());
                    for (index, element) in elements.iter().enumerate() {
                        let label = format!("{}[{index}]", spec.name);
                        matched.push(crate::types::match_named_value(
                            &label,
                            &label,
                            type_name,
                            element,
                            &self.types,
                        )?);
                    }
                    Some(crate::ArgVariants::PerElement(matched))
                } else {
                    Some(crate::ArgVariants::Single(crate::types::match_named_value(
                        &spec.name,
                        &spec.name,
                        type_name,
                        value,
                        &self.types,
                    )?))
                }
            } else {
                None
            };
            bound_args.insert(
                spec.name.clone(),
                crate::BoundArg {
                    name: spec.name.clone(),
                    value_type: spec.value_type.clone(),
                    value: value.clone(),
                    workspace: spec.workspace.clone(),
                    variants,
                },
            );
        }

        // Command-declared workspaces resolve unconditionally: `uses_workspace`
        // is a hard requirement, unlike path-argument workspaces which only
        // resolve when a bound argument references them.
        for workspace_name in &registered.spec.workspaces {
            let workspace_id = mcp_workspace_resolver::WorkspaceId::from(workspace_name.as_str());
            if resolved.root(&workspace_id).is_none() {
                let diagnostics = resolved
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| {
                        diagnostic
                            .requirement
                            .as_ref()
                            .is_none_or(|id| id == &workspace_id)
                    })
                    .cloned()
                    .collect();
                return Err(FrameworkError::WorkspaceUnresolved {
                    workspace: workspace_name.clone(),
                    diagnostics,
                });
            }
            used_workspaces.insert(workspace_name.clone());
        }

        let tokens = template
            .tokens
            .iter()
            .map(|token| match token {
                TemplateToken::Literal(value) => InvocationToken::Literal {
                    value: value.clone(),
                },
                TemplateToken::Placeholder(name) => InvocationToken::Placeholder {
                    name: name.clone(),
                    value: request.args.get(name).cloned().unwrap_or(Value::Null),
                },
            })
            .collect();
        let output = request.output.clone().unwrap_or_default();
        let workspaces = self.workspaces.values().cloned().collect::<Vec<_>>();
        let workspace_roots: Vec<crate::PlanWorkspaceRoot> = resolved
            .roots
            .iter()
            .filter(|root| used_workspaces.contains(root.id.as_str()))
            .map(crate::PlanWorkspaceRoot::from)
            .collect();
        let stdin_fingerprint = request.stdin.as_ref().map(|stdin| {
            json!({
                "mimeType": stdin.mime_type,
                "textBytes": stdin.text.len(),
                "textHash": stable_hash_value(&json!(stdin.text)),
            })
        });
        let invocation_fingerprint = stable_hash_value(&json!({
            "normalizedCommand": {
                "path": &registered.spec.path,
                "tokens": &tokens,
                "boundArgs": &bound_args,
            },
            "stdin": stdin_fingerprint,
            "operationId": &operation.id,
            "effect": &operation.effect,
            "lane": operation.lane(),
            "permissions": &registered.spec.permissions,
            "workspaces": &workspaces,
            // Selected roots bind approvals to the roots that were previewed.
            "workspaceRoots": &workspace_roots,
            "idempotent": operation.idempotent,
            "output": &output,
        }));

        Ok(InvocationPlan {
            operation_id: operation.id.clone(),
            command_path: registered.spec.path.clone(),
            raw_command: request.command.clone(),
            catalog_hash: identity.catalog_hash,
            invocation_fingerprint,
            effect: operation.effect.clone(),
            lane: operation.lane(),
            tokens,
            bound_args,
            permissions: registered.spec.permissions.clone(),
            workspaces,
            workspace_roots,
            idempotent: operation.idempotent,
            output,
        })
    }

    pub async fn run(&self, request: RunRequest) -> Result<RunResponse> {
        self.run_with_lane(
            request,
            None,
            None,
            None,
            &self.resolve_declared_workspaces(),
        )
        .await
    }

    pub async fn run_in_lane(
        &self,
        request: RunRequest,
        current_tool: impl Into<String>,
        lane: EffectLane,
        primary_tool_name: impl AsRef<str>,
    ) -> Result<RunResponse> {
        self.run_with_lane(
            request,
            Some(current_tool.into()),
            Some(lane),
            Some(primary_tool_name.as_ref().to_string()),
            &self.resolve_declared_workspaces(),
        )
        .await
    }

    /// Like [`run_in_lane`](Self::run_in_lane), but plans against
    /// pre-resolved workspace roots gathered by the caller.
    pub async fn run_in_lane_with_workspaces(
        &self,
        request: RunRequest,
        current_tool: impl Into<String>,
        lane: EffectLane,
        primary_tool_name: impl AsRef<str>,
        resolved: &ResolvedWorkspaceSet,
    ) -> Result<RunResponse> {
        self.run_with_lane(
            request,
            Some(current_tool.into()),
            Some(lane),
            Some(primary_tool_name.as_ref().to_string()),
            resolved,
        )
        .await
    }

    async fn run_with_lane(
        &self,
        request: RunRequest,
        current_tool: Option<String>,
        current_lane: Option<EffectLane>,
        primary_tool_name: Option<String>,
        resolved: &ResolvedWorkspaceSet,
    ) -> Result<RunResponse> {
        let plan = self.build_plan_with_workspaces(&request, resolved)?;
        if let Some(current_lane) = current_lane {
            if plan.lane != current_lane {
                let required_tool = self
                    .required_tool_name(primary_tool_name.as_deref().unwrap_or("run"), plan.lane);
                return Err(FrameworkError::WrongEffectLane {
                    current_tool: current_tool.unwrap_or_else(|| current_lane.tool_name("run")),
                    required_tool,
                });
            }
        }

        if matches!(request.effective_mode(), crate::RunMode::DryRun) {
            return Ok(RunResponse {
                plan,
                output: None,
                dry_run: true,
            });
        }

        self.policy.check(&plan.permissions)?;
        let registered = self.commands.get(&plan.command_path).ok_or_else(|| {
            FrameworkError::UnknownCommand {
                command: plan.command_path.join(" "),
                nearest: Vec::new(),
            }
        })?;

        let output = registered
            .handler
            .call(CommandContext {
                plan: plan.clone(),
                stdin: request.stdin,
            })
            .await
            .map_err(|error| self.enrich_capability_denied(error))?
            .apply_output_spec(&plan.output);

        Ok(RunResponse {
            plan,
            output: Some(output),
            dry_run: false,
        })
    }

    pub fn help(&self, request: HelpRequest) -> HelpResult {
        match request.command.as_deref() {
            Some(command) => self.command_help(command, request.topic.unwrap_or(HelpTopic::Usage)),
            None => self.server_help(),
        }
    }

    pub fn resource_text(&self, uri: &str) -> Option<String> {
        match uri {
            "cli://server/overview" => Some(self.server_help().text),
            "cli://catalog" => serde_json::to_string_pretty(&self.catalog()).ok(),
            "cli://commands" => Some(self.command_catalog_text()),
            "cli://permissions" => Some(self.permissions_text()),
            other if other.starts_with("cli://commands/") => {
                let command = other
                    .trim_start_matches("cli://commands/")
                    .replace('/', " ");
                Some(self.command_help(&command, HelpTopic::Usage).text)
            }
            _ => None,
        }
    }

    fn match_command(&self, template: &CommandTemplate) -> Option<&RegisteredCommand> {
        let prefix = template.literal_prefix();
        self.commands
            .values()
            .filter(|command| {
                command.spec.path.len() <= prefix.len()
                    && command
                        .spec
                        .path
                        .iter()
                        .zip(prefix.iter())
                        .all(|(expected, actual)| expected == actual)
            })
            .max_by_key(|command| command.spec.path.len())
    }

    fn nearest_commands<S: AsRef<str>>(&self, requested: &[S]) -> Vec<String> {
        if requested.is_empty() {
            return Vec::new();
        }

        let mut scored = Vec::new();
        for path in self.commands.keys() {
            let candidate = path.join(" ").to_lowercase();
            let compared = requested
                .iter()
                .take(path.len())
                .map(|token| token.as_ref().to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let distance = edit_distance(&compared, &candidate);
            if distance <= 2.max(candidate.len() / 3) {
                scored.push((distance, path.join(" ")));
            }
        }

        if scored.is_empty() {
            let namespace = requested[0].as_ref().to_lowercase();
            scored.extend(
                self.commands
                    .keys()
                    .filter(|path| {
                        path.first()
                            .is_some_and(|first| first.to_lowercase() == namespace)
                    })
                    .map(|path| (0, path.join(" "))),
            );
        }

        scored.sort();
        scored.dedup_by(|left, right| left.1 == right.1);
        scored.into_iter().take(3).map(|(_, name)| name).collect()
    }

    fn server_help(&self) -> HelpResult {
        let identity = self.catalog_identity();
        let mut lines = vec![
            format!("# {}", self.server_name),
            self.server_description.clone(),
            String::new(),
            "Start with the primary execution tool. Use lane tools only when the framework returns structured retry data.".to_string(),
            "Command strings are typed templates, not shell programs.".to_string(),
            String::new(),
            "Runtime identity:".to_string(),
            format!("- Catalog hash: `{}`", identity.catalog_hash),
            format!("- Run schema hash: `{}`", identity.run_schema_hash),
            format!("- Help schema hash: `{}`", identity.help_schema_hash),
            String::new(),
            "Commands:".to_string(),
        ];
        for spec in self.command_specs() {
            lines.push(format!("- `{}`: {}", spec.name(), spec.summary));
        }

        if !self.capabilities.is_empty() {
            lines.push(String::new());
            lines.push("Capabilities:".to_string());
            for name in self.capabilities.keys() {
                lines.push(format!("- {}", self.capability_help_line(name)));
            }
        }

        if !self.guidance.is_empty() {
            lines.push(String::new());
            lines.push("Guidance:".to_string());
            for guidance in &self.guidance {
                match guidance.kind {
                    crate::GuidanceKind::RunCommand => {
                        lines.push(format!("- `{}` - {}", guidance.text, guidance.surface));
                    }
                    crate::GuidanceKind::HumanAction => {
                        lines.push(format!(
                            "- (human action) {} - {}",
                            guidance.text, guidance.surface
                        ));
                    }
                    crate::GuidanceKind::ExternalShell => {
                        lines.push(format!(
                            "- (external shell, not a framework command) `{}` - {}",
                            guidance.text, guidance.surface
                        ));
                    }
                }
            }
        }

        HelpResult {
            title: self.server_name.clone(),
            text: lines.join("\n"),
            structured: json!({
                "server": self.server_name,
                "catalog": self.catalog(),
                "commands": self.command_specs().collect::<Vec<_>>(),
                "workspaces": self.workspaces().collect::<Vec<_>>()
            }),
        }
    }

    fn command_help(&self, command: &str, topic: HelpTopic) -> HelpResult {
        let parsed = CommandTemplate::parse(command);
        let spec = parsed.ok().and_then(|template| {
            self.match_command(&template)
                .map(|registered| &registered.spec)
        });

        let Some(spec) = spec else {
            let tokens = command
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            let error = FrameworkError::UnknownCommand {
                command: command.to_string(),
                nearest: self.nearest_commands(&tokens),
            };
            let mut text = error.to_string();
            if let FrameworkError::UnknownCommand { nearest, .. } = &error
                && !nearest.is_empty()
            {
                text.push_str("\n\nDid you mean:\n");
                for candidate in nearest {
                    text.push_str(&format!("- `{candidate}`\n"));
                }
            }
            let mut structured = structured_error(&error);
            if let FrameworkError::UnknownCommand { nearest, .. } = &error
                && let Some(map) = structured.as_object_mut()
            {
                map.insert("nearest".to_string(), json!(nearest));
            }
            return HelpResult {
                title: "Unknown command".to_string(),
                text,
                structured,
            };
        };

        let text = match topic {
            HelpTopic::Server | HelpTopic::Usage => self.usage_text(spec),
            HelpTopic::Arguments => self.arguments_text(spec),
            HelpTopic::Permissions => format_permissions(&spec.permissions),
            HelpTopic::Examples => self.examples_text(spec),
        };

        HelpResult {
            title: spec.name(),
            text,
            structured: json!({ "command": spec, "topic": topic }),
        }
    }

    fn command_catalog_text(&self) -> String {
        self.command_specs()
            .map(|spec| format!("`{}` - {}", spec.name(), spec.summary))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn permissions_text(&self) -> String {
        let mut lines = Vec::new();
        for spec in self.command_specs() {
            lines.push(format!("## {}", spec.name()));
            lines.push(format_permissions(&spec.permissions));
        }
        lines.join("\n")
    }

    fn usage_text(&self, spec: &CommandSpec) -> String {
        let mut sections = vec![
            format!("# `{}`", spec.name()),
            spec.description.clone(),
            self.arguments_text(spec),
        ];
        if !spec.workspaces.is_empty() {
            let mut lines = vec!["Workspaces:".to_string()];
            for name in &spec.workspaces {
                let description = self
                    .workspaces
                    .get(name)
                    .and_then(|decl| decl.description.as_deref())
                    .map(|text| format!("{text} "))
                    .unwrap_or_default();
                lines.push(format!(
                    "- `{name}`: {description}Resolved by the server; not a command argument."
                ));
            }
            sections.push(lines.join("\n"));
        }
        if !spec.requires.is_empty() {
            let mut lines = vec!["Requires:".to_string()];
            for name in &spec.requires {
                lines.push(format!("- {}", self.capability_help_line(name)));
            }
            sections.push(lines.join("\n"));
        }
        if !spec.provides.is_empty() {
            let mut lines = vec!["Provides:".to_string()];
            for name in &spec.provides {
                let summary = self
                    .capabilities
                    .get(name)
                    .map(|decl| format!(": {}", decl.summary))
                    .unwrap_or_default();
                lines.push(format!("- `{name}`{summary}"));
            }
            sections.push(lines.join("\n"));
        }
        if let Some(stdin) = &spec.stdin {
            sections.push(format!("Stdin: {} ({}).", stdin.summary, stdin.mime_type));
        }
        if !spec.progress.is_empty() {
            let mut lines = vec!["Progress phases:".to_string()];
            for phase in &spec.progress {
                lines.push(format!("- {}: {}", phase.name, phase.summary));
            }
            sections.push(lines.join("\n"));
        }
        sections.push(self.examples_text(spec));
        sections.join("\n\n")
    }

    fn arguments_text(&self, spec: &CommandSpec) -> String {
        if spec.args.is_empty() {
            return "Arguments: none.".to_string();
        }
        let mut lines = vec!["Arguments:".to_string()];
        for arg in &spec.args {
            let required = if arg.required { "required" } else { "optional" };
            let workspace = arg
                .workspace
                .as_ref()
                .map(|workspace| format!(" workspace `{workspace}`"))
                .unwrap_or_default();
            let value_type = match &arg.value_type {
                crate::ArgType::Named(type_name) if arg.repeated => {
                    format!("list of `{type_name}`")
                }
                crate::ArgType::Named(type_name) => format!("`{type_name}`"),
                other => format!("{other:?}"),
            };
            lines.push(format!(
                "- `$args.{}`: {}, {}, {}{}",
                arg.name, value_type, required, arg.summary, workspace
            ));
        }
        let type_lines = self.referenced_types_text(spec);
        if !type_lines.is_empty() {
            lines.push(String::new());
            lines.extend(type_lines);
        }
        lines.join("\n")
    }

    /// Renders each type referenced by this command's arguments exactly
    /// once, in reference order, including transitively referenced types.
    fn referenced_types_text(&self, spec: &CommandSpec) -> Vec<String> {
        let mut queue: Vec<&str> = spec
            .args
            .iter()
            .filter_map(|arg| match &arg.value_type {
                crate::ArgType::Named(type_name) => Some(type_name.as_str()),
                _ => None,
            })
            .collect();
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        while let Some(name) = queue.first().copied() {
            queue.remove(0);
            if !seen.insert(name.to_string()) {
                continue;
            }
            let Some(decl) = self.types.get(name) else {
                continue;
            };
            ordered.push(decl);
            for variant in &decl.variants {
                for field in &variant.fields {
                    if let Some(referenced) = field.shape.referenced_type() {
                        queue.push(referenced);
                    }
                }
            }
        }
        let mut lines = Vec::new();
        for decl in ordered {
            lines.push(format!("Type `{}`: {}", decl.name, decl.summary));
            for variant in &decl.variants {
                lines.push(format!("  - {}: {}", variant.name, variant.summary));
                for field in &variant.fields {
                    let required = if field.required {
                        "required"
                    } else {
                        "optional"
                    };
                    lines.push(format!(
                        "    - `{}`: {}, {}, {}",
                        field.name,
                        field.shape.label(),
                        required,
                        field.summary
                    ));
                }
            }
        }
        lines
    }

    fn examples_text(&self, spec: &CommandSpec) -> String {
        if spec.examples.is_empty() {
            return "Examples: none.".to_string();
        }
        let mut lines = vec!["Examples:".to_string()];
        for example in &spec.examples {
            lines.push(format!("- `{}` - {}", example.command, example.summary));
        }
        lines.join("\n")
    }
}

fn lane_description(primary_tool_name: &str, lane: EffectLane) -> String {
    match lane {
        EffectLane::Primary => format!(
            "Primary execution tool. Start here for all command templates; the framework returns structured retry data when another effect lane is required."
        ),
        EffectLane::Write => format!(
            "Write execution lane. Use this tool when `{primary_tool_name}` returns structured retry data requiring this lane."
        ),
        EffectLane::Delete => format!(
            "Delete execution lane. Use this tool when `{primary_tool_name}` returns structured retry data requiring this lane."
        ),
        EffectLane::Exec => format!(
            "Process execution lane. Use this tool when `{primary_tool_name}` returns structured retry data requiring this lane."
        ),
        EffectLane::Network => format!(
            "Network execution lane. Use this tool when `{primary_tool_name}` returns structured retry data requiring this lane."
        ),
    }
}

fn format_permissions(permissions: &[PermissionSpec]) -> String {
    if permissions.is_empty() {
        return "Permissions: none.".to_string();
    }
    let mut lines = vec!["Permissions:".to_string()];
    for permission in permissions {
        lines.push(format!(
            "- {} `{}`: {}",
            permission.effect.as_label(),
            permission.scope,
            permission.description
        ));
    }
    lines.join("\n")
}

fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for (row, left_char) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != right_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
}
