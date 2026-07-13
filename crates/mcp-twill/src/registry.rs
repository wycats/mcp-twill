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
    preamble: Option<String>,
    commands: BTreeMap<Vec<String>, RegisteredCommand>,
    workspaces: BTreeMap<String, WorkspaceDecl>,
    types: BTreeMap<String, crate::TypeDecl>,
    duplicate_types: Vec<String>,
    capabilities: BTreeMap<String, crate::CapabilityDecl>,
    duplicate_capabilities: Vec<String>,
    resources: BTreeMap<String, crate::ResourceDecl>,
    duplicate_resources: Vec<String>,
    /// Capability names derived from resource declarations. These skip the
    /// provider/consumer capability rules; the resource rules (unpaired
    /// grant, unenumerable grant) own those semantics.
    resource_capabilities: BTreeSet<String>,
    resolvers: BTreeMap<String, Arc<dyn crate::resource::ErasedResolver>>,
    readers: BTreeMap<String, Arc<dyn crate::resource::ErasedReader>>,
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
            preamble: None,
            commands: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            types: BTreeMap::new(),
            duplicate_types: Vec::new(),
            capabilities: BTreeMap::new(),
            duplicate_capabilities: Vec::new(),
            resources: BTreeMap::new(),
            duplicate_resources: Vec::new(),
            resource_capabilities: BTreeSet::new(),
            resolvers: BTreeMap::new(),
            readers: BTreeMap::new(),
            guidance: Vec::new(),
            policy: PermissionPolicy::default(),
        }
    }

    /// Declares server-level operating guidance rendered in server help,
    /// the MCP `instructions` field, and the `getting_started` prompt.
    pub fn declare_preamble(mut self, text: impl Into<String>) -> Self {
        self.preamble = Some(text.into());
        self
    }

    pub fn preamble(&self) -> Option<&str> {
        self.preamble.as_deref()
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

    /// Declares a server-held resource (RFC 0012). Lifecycle edges derive
    /// from handler signatures, never from the declaration.
    pub fn declare_resource(mut self, decl: crate::ResourceDecl) -> Self {
        if self.resources.contains_key(&decl.name) {
            self.duplicate_resources.push(decl.name.clone());
        }
        self.resources.insert(decl.name.clone(), decl);
        self
    }

    /// Declares the capability derived from a resource declaration. The
    /// derived capability keeps the RFC 0010 vocabulary and projections
    /// working; the resource rules own its lifecycle semantics.
    pub(crate) fn declare_derived_capability(mut self, decl: crate::CapabilityDecl) -> Self {
        self.resource_capabilities.insert(decl.name.clone());
        self.capabilities.insert(decl.name.clone(), decl);
        self
    }

    /// Binds the resolver that turns references to `T` into live values.
    pub fn with_resolver<T: crate::Resource>(
        mut self,
        resolver: impl crate::ResolveResource<T>,
    ) -> Self {
        self.resolvers.insert(
            T::NAME.to_string(),
            Arc::new(crate::resource::ResolverAdapter::new(resolver)),
        );
        self
    }

    /// Binds a reader for `T`, enabling `resources/read` and
    /// `resource_link` emission for its grants on capable transports.
    pub fn with_reader<T: crate::Resource>(mut self, reader: impl crate::ReadResource<T>) -> Self {
        self.readers.insert(
            T::NAME.to_string(),
            Arc::new(crate::resource::ReaderAdapter::new(reader)),
        );
        self
    }

    pub fn resource_decls(&self) -> impl Iterator<Item = &crate::ResourceDecl> {
        self.resources.values()
    }

    pub fn resource_decl(&self, name: &str) -> Option<&crate::ResourceDecl> {
        self.resources.get(name)
    }

    pub fn has_reader(&self, resource: &str) -> bool {
        self.readers.contains_key(resource)
    }

    pub(crate) fn resource_reader(
        &self,
        resource: &str,
    ) -> Option<Arc<dyn crate::resource::ErasedReader>> {
        self.readers.get(resource).cloned()
    }

    /// Matches a URI against every declared resource template, returning
    /// the declaration and the extracted id.
    pub fn match_resource_uri(&self, uri: &str) -> Option<(&crate::ResourceDecl, String)> {
        self.resources
            .values()
            .find_map(|decl| decl.parse_uri(uri).map(|id| (decl, id.to_string())))
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

    pub fn register<H>(mut self, mut spec: CommandSpec, handler: H) -> Self
    where
        H: CommandHandler,
    {
        spec.workspaces.sort();
        spec.workspaces.dedup();
        spec.optional_workspaces.sort();
        spec.optional_workspaces.dedup();
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
            server: self.server_spec(),
            namespaces: group_namespaces(&operations),
            operations,
            workspaces: self.workspaces.values().cloned().collect(),
            types: self.types.values().cloned().collect(),
            capabilities: self.capabilities.values().cloned().collect(),
            resources: self.resource_specs(),
            guidance: self.guidance.clone(),
            identity,
        }
    }

    /// Every declared resource with its derived lifecycle edges: who grants
    /// it, who releases it, who enumerates it, who requires it.
    pub fn resource_specs(&self) -> Vec<crate::ResourceSpec> {
        self.resources
            .values()
            .map(|decl| crate::ResourceSpec {
                name: decl.name.clone(),
                summary: decl.summary.clone(),
                uri: decl.uri.clone(),
                carrier: decl.carrier_name(),
                within: decl.within.clone(),
                lifetime: decl.lifetime.clone(),
                expiry: decl.expiry.clone(),
                granted_by: self.resource_granters(&decl.name),
                released_by: self.resource_releasers(&decl.name),
                enumerated_by: self.resource_enumerators(&decl.name),
                required_by: self.resource_requirers(&decl.name),
            })
            .collect()
    }

    fn server_spec(&self) -> ServerSpec {
        let mut server = ServerSpec::new(&self.server_name, &self.server_description);
        server.preamble = self.preamble.clone();
        server
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
            "server": self.server_spec(),
            "namespaces": group_namespaces(operations),
            "operations": operations,
            "workspaces": self.workspaces.values().collect::<Vec<_>>(),
            "types": self.types.values().collect::<Vec<_>>(),
            "capabilities": self.capabilities.values().collect::<Vec<_>>(),
            "resources": self.resource_specs(),
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

    /// Validates command-declared workspace requirements: every required or
    /// optional name must match a server declaration, and one command cannot
    /// declare the same workspace in both modes.
    pub fn validate_workspaces(&self) -> Result<()> {
        for command in self.commands.values() {
            for workspace_name in &command.spec.workspaces {
                if !self.workspaces.contains_key(workspace_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` uses workspace `{workspace_name}`, which is not declared on the server",
                        command.spec.name()
                    )));
                }
            }
            for workspace_name in &command.spec.optional_workspaces {
                if !self.workspaces.contains_key(workspace_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` optionally uses workspace `{workspace_name}`, which is not declared on the server",
                        command.spec.name()
                    )));
                }
                if command.spec.workspaces.contains(workspace_name) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` declares workspace `{workspace_name}` as both required and optional",
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
    /// needs both a provider and a consumer. Capabilities derived from
    /// resource declarations skip the provider/consumer rules; the resource
    /// rules (unpaired grant, unenumerable grant) own those semantics.
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
            if self.resource_capabilities.contains(&capability.name) {
                continue;
            }
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

    /// The escape hatches that prefer this command, derived from `fallback`
    /// declarations on other commands. The reverse edge is never written by
    /// hand — exactly as establishing commands are derived from `provides`.
    pub fn derived_fallback_edges(&self, command_name: &str) -> Vec<(String, String)> {
        self.commands
            .values()
            .filter_map(|command| {
                let fallback = command.spec.fallback.as_ref()?;
                fallback
                    .prefer
                    .iter()
                    .any(|preferred| preferred == command_name)
                    .then(|| (command.spec.name(), fallback.when.clone()))
            })
            .collect()
    }

    /// Commands that grant references to a resource, derived from handler
    /// output types.
    pub fn resource_granters(&self, resource: &str) -> Vec<String> {
        self.commands_where(|spec| spec.grants.iter().any(|name| name == resource))
    }

    /// Commands that release a resource, derived from handler signatures.
    pub fn resource_releasers(&self, resource: &str) -> Vec<String> {
        self.commands_where(|spec| spec.releases.iter().any(|name| name == resource))
    }

    /// Commands that enumerate a resource, derived from handler output
    /// types. Enumeration is the recovery path: an agent that lost a
    /// reference re-asks the server instead of remembering.
    pub fn resource_enumerators(&self, resource: &str) -> Vec<String> {
        self.commands_where(|spec| spec.enumerates.iter().any(|name| name == resource))
    }

    /// Commands that require a live reference to a resource, derived from
    /// handler signatures.
    pub fn resource_requirers(&self, resource: &str) -> Vec<String> {
        self.commands_where(|spec| spec.requires_resources.iter().any(|name| name == resource))
    }

    fn commands_where(&self, matches: impl Fn(&CommandSpec) -> bool) -> Vec<String> {
        self.commands
            .values()
            .filter(|command| matches(&command.spec))
            .map(|command| command.spec.name())
            .collect()
    }

    /// Validates resource declarations against every RFC 0012 registration
    /// rule: well-formed unique URI templates, resolvable `within` scoping
    /// with no cycles, no derived-name collisions, signatures referencing
    /// only declared resources, resolvers bound where required, no unpaired
    /// grants, and no unenumerable scoped grants.
    pub fn validate_resources(&self) -> Result<()> {
        if let Some(name) = self.duplicate_resources.first() {
            return Err(FrameworkError::Build(format!(
                "resource `{name}` is declared more than once"
            )));
        }
        for decl in self.resources.values() {
            if decl.summary.trim().is_empty() {
                return Err(FrameworkError::Build(format!(
                    "resource `{}` is missing a summary",
                    decl.name
                )));
            }
            if decl.uri_parts().is_none() {
                return Err(FrameworkError::Build(format!(
                    "resource `{}` URI template `{}` must contain exactly one `{{id}}` slot",
                    decl.name, decl.uri
                )));
            }
            // Minted references travel bare through MCP content and agent
            // context; a template without a scheme mints relative strings
            // that nothing can route back to this server.
            if !crate::model::template_has_scheme(&decl.uri) {
                return Err(FrameworkError::Build(format!(
                    "resource `{}` URI template `{}` must be an absolute URI with a scheme (like `app://{{id}}`)",
                    decl.name, decl.uri
                )));
            }
            for other in self.resources.values() {
                if other.name != decl.name && other.uri == decl.uri {
                    return Err(FrameworkError::Build(format!(
                        "resources `{}` and `{}` declare the same URI template `{}`",
                        decl.name.clone().min(other.name.clone()),
                        decl.name.clone().max(other.name.clone()),
                        decl.uri
                    )));
                }
                // Distinct templates can still mint colliding URIs (like
                // `x://{id}/bar` + `x://foo/{id}`); reads of such a URI
                // would route by map order, so refuse the pair up front.
                if other.name != decl.name
                    && other.uri != decl.uri
                    && crate::model::templates_overlap(decl, other)
                {
                    let (first, second) = if decl.name < other.name {
                        (decl, other)
                    } else {
                        (other, decl)
                    };
                    return Err(FrameworkError::Build(format!(
                        "resources `{}` (`{}`) and `{}` (`{}`) have URI templates that can mint the same URI; templates must not overlap",
                        first.name, first.uri, second.name, second.uri
                    )));
                }
            }
            if let Some(within) = &decl.within {
                if within == &decl.name {
                    return Err(FrameworkError::Build(format!(
                        "resource `{}` is scoped within itself",
                        decl.name
                    )));
                }
                if !self.resources.contains_key(within) {
                    return Err(FrameworkError::Build(format!(
                        "resource `{}` is scoped within `{within}`, which is not a declared resource",
                        decl.name
                    )));
                }
            }
            // Derived names collide with hand declarations: the resource
            // owns `{name}` (capability) and `{name}-ref` (type); the fix
            // is to delete the hand-written declaration.
            if self.types.contains_key(&decl.reference_type_name()) {
                return Err(FrameworkError::Build(format!(
                    "resource `{}` derives reference type `{}`, which is also declared with `declare_type`; the resource owns that name",
                    decl.name,
                    decl.reference_type_name()
                )));
            }
            if self.capabilities.contains_key(&decl.name)
                && !self.resource_capabilities.contains(&decl.name)
            {
                return Err(FrameworkError::Build(format!(
                    "resource `{}` derives capability `{}`, which is also declared with `declare_capability`; the resource owns that name",
                    decl.name, decl.name
                )));
            }
        }
        // `within` cycles.
        for start in self.resources.keys() {
            let mut current = start.as_str();
            let mut hops = 0;
            while let Some(within) = self
                .resources
                .get(current)
                .and_then(|decl| decl.within.as_deref())
            {
                hops += 1;
                if within == start || hops > self.resources.len() {
                    return Err(FrameworkError::Build(format!(
                        "resource scoping cycle through `{start}`; `within` must form a tree"
                    )));
                }
                current = within;
            }
        }
        for command in self.commands.values() {
            let name = command.spec.name();
            let signature_resources = command
                .spec
                .requires_resources
                .iter()
                .chain(&command.spec.grants)
                .chain(&command.spec.releases)
                .chain(&command.spec.enumerates);
            for resource in signature_resources {
                if !self.resources.contains_key(resource) {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` references resource `{resource}` in its handler signature, which is not declared on the server"
                    )));
                }
            }
            for resource in command
                .spec
                .requires_resources
                .iter()
                .chain(&command.spec.releases)
            {
                if !self.resolvers.contains_key(resource) {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` requires resource `{resource}`, which has no bound resolver; bind one with `resolver`"
                    )));
                }
            }
        }
        for decl in self.resources.values() {
            let granters = self.resource_granters(&decl.name);
            if granters.is_empty() {
                continue;
            }
            // The stewardship rule as structure: an unpaired acquisition is
            // undeclarable. A releasing command or a declared expiry names
            // the owner whose drop revokes the grant.
            if self.resource_releasers(&decl.name).is_empty() && decl.expiry.is_none() {
                return Err(FrameworkError::Build(format!(
                    "resource `{}` is granted by {} but no command releases it and no `expiry` retires it; an acquisition must name its release path",
                    decl.name,
                    granters
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            // Enumeration-as-recovery is mandatory for scoped resources.
            // Root resources are exempt: enumerating a lost session would
            // require the very scope that was lost, so their recovery edge
            // is the establishing command.
            if decl.within.is_some() && self.resource_enumerators(&decl.name).is_empty() {
                return Err(FrameworkError::Build(format!(
                    "resource `{}` is granted by {} but no command enumerates it; a scoped resource must be recoverable by re-asking the server",
                    decl.name,
                    granters
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        Ok(())
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

    /// One help line for a required resource: summary and the derived
    /// recovery edge — the enumerator for scoped resources, the granting
    /// commands otherwise.
    fn resource_requirement_line(&self, resource: &str) -> String {
        let Some(decl) = self.resources.get(resource) else {
            return format!("- `{resource}`");
        };
        let enumerators = self.resource_enumerators(resource);
        let recover = if enumerators.is_empty() {
            let granters = self.resource_granters(resource);
            if granters.is_empty() {
                String::new()
            } else {
                format!(" Establish with {}.", backticked_list(&granters))
            }
        } else {
            format!(" Recover with {}.", backticked_list(&enumerators))
        };
        format!(
            "- a live `{}` — {} (carried by `{}`).{recover}",
            decl.name,
            decl.summary,
            decl.carrier_name()
        )
    }

    /// One help line for a granted resource: URI template, validity prose,
    /// enumerator, and releasers — all derived.
    fn resource_grant_line(&self, resource: &str) -> String {
        let Some(decl) = self.resources.get(resource) else {
            return format!("- `{resource}`");
        };
        let mut line = format!("- `{}` ({})", decl.name, decl.uri);
        if let Some(lifetime) = &decl.lifetime {
            line.push_str(&format!(" — {lifetime}"));
        }
        line.push('.');
        let enumerators = self.resource_enumerators(resource);
        if !enumerators.is_empty() {
            line.push_str(&format!(
                " Enumerate with {}.",
                backticked_list(&enumerators)
            ));
        }
        let releasers = self.resource_releasers(resource);
        if !releasers.is_empty() {
            line.push_str(&format!(" Release with {}.", backticked_list(&releasers)));
        }
        if let Some(expiry) = &decl.expiry {
            line.push_str(&format!(" Expiry: {expiry}."));
        }
        line
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
                crate::ArgType::ResourceRef(resource) => {
                    let template = self
                        .resources
                        .get(resource)
                        .map(|decl| decl.uri.as_str())
                        .unwrap_or_default();
                    json!({
                        "type": "string",
                        "description": format!(
                            "{} Accepts a bare id or the full URI ({template}).",
                            arg.summary
                        ),
                    })
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
        if let Some(preamble) = &self.preamble
            && preamble.trim().is_empty()
        {
            return Err(FrameworkError::Build(
                "server preamble is empty; declare text or remove it".to_string(),
            ));
        }
        for type_decl in self.types.values() {
            for variant in &type_decl.variants {
                if let Some(when) = &variant.fallback
                    && when.trim().is_empty()
                {
                    return Err(FrameworkError::Build(format!(
                        "type `{}` variant `{}` declares a fallback with an empty condition",
                        type_decl.name, variant.name
                    )));
                }
            }
            if !type_decl.variants.is_empty()
                && type_decl
                    .variants
                    .iter()
                    .all(|variant| variant.fallback.is_some())
            {
                return Err(FrameworkError::Build(format!(
                    "type `{}` declares a fallback on every variant; a set of alternatives that are all dispreferred prefers nothing",
                    type_decl.name
                )));
            }
        }
        let command_names: BTreeSet<String> =
            self.commands.keys().map(|path| path.join(" ")).collect();
        for command in self.commands.values() {
            let name = command.spec.name();
            if let Some(use_when) = &command.spec.use_when {
                if use_when.trim().is_empty() {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` declares an empty `use_when`"
                    )));
                }
                if command.spec.fallback.is_some() {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` declares both `use_when` and `fallback`; a fallback's condition is its selection criterion"
                    )));
                }
            }
            let mut seen = BTreeSet::new();
            for alternative in &command.spec.alternatives {
                if alternative.when.trim().is_empty() {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` declares alternative `{}` with an empty condition",
                        alternative.command
                    )));
                }
                if alternative.command == name {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` lists itself as an alternative"
                    )));
                }
                if !command_names.contains(&alternative.command) {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` declares alternative `{}`, which is not a catalog command",
                        alternative.command
                    )));
                }
                if !seen.insert(alternative.command.as_str()) {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` declares alternative `{}` more than once",
                        alternative.command
                    )));
                }
            }
            if let Some(fallback) = &command.spec.fallback {
                if fallback.when.trim().is_empty() {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` declares a fallback with an empty condition"
                    )));
                }
                if fallback.prefer.is_empty() {
                    return Err(FrameworkError::Build(format!(
                        "command `{name}` declares a fallback with an empty `prefer` list; an escape hatch must say what it is an escape from"
                    )));
                }
                let mut seen = BTreeSet::new();
                for preferred in &fallback.prefer {
                    if *preferred == name {
                        return Err(FrameworkError::Build(format!(
                            "command `{name}` prefers itself in its fallback declaration"
                        )));
                    }
                    if !command_names.contains(preferred) {
                        return Err(FrameworkError::Build(format!(
                            "command `{name}` prefers `{preferred}`, which is not a catalog command"
                        )));
                    }
                    if !seen.insert(preferred.as_str()) {
                        return Err(FrameworkError::Build(format!(
                            "command `{name}` prefers `{preferred}` more than once"
                        )));
                    }
                }
            }
        }
        let fallback_prefer: BTreeMap<String, Vec<String>> = self
            .commands
            .values()
            .filter_map(|command| {
                command
                    .spec
                    .fallback
                    .as_ref()
                    .map(|fallback| (command.spec.name(), fallback.prefer.clone()))
            })
            .collect();
        for start in fallback_prefer.keys() {
            let mut stack = vec![start.as_str()];
            find_fallback_cycle(start, &fallback_prefer, &mut stack)?;
        }
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
        self.build_plan_with_context(request, &crate::InvocationContext::default())
    }

    pub fn build_plan_with_context(
        &self,
        request: &RunRequest,
        context: &crate::InvocationContext,
    ) -> Result<InvocationPlan> {
        let resolved = self.resolve_context_workspaces(context);
        self.build_plan_prepared(request, &resolved, context)
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
        self.resolve_workspaces(&self.declared_observations())
    }

    /// Resolves the registry's complete workspace requirement set against the
    /// supplied observations and returns roots in canonical workspace-id
    /// order.
    pub fn resolve_workspaces(
        &self,
        observations: &WorkspaceObservationSet,
    ) -> ResolvedWorkspaceSet {
        let mut resolved = resolve_workspaces(&self.workspace_requirements(), observations);
        resolved.roots.sort_by(|left, right| left.id.cmp(&right.id));
        resolved
    }

    fn resolve_context_workspaces(
        &self,
        context: &crate::InvocationContext,
    ) -> ResolvedWorkspaceSet {
        let mut observations = self.declared_observations();
        if let Some(host_roots) = context.host_workspace_roots() {
            observations = observations.with_host_roots(host_roots.clone());
        }
        self.resolve_workspaces(&observations)
    }

    fn validate_pre_resolved_workspaces(
        &self,
        resolved: &ResolvedWorkspaceSet,
    ) -> Result<ResolvedWorkspaceSet> {
        let mut seen = BTreeSet::new();
        for root in &resolved.roots {
            let workspace = root.id.as_str();
            if !self.workspaces.contains_key(workspace) {
                return Err(FrameworkError::InvalidPreResolvedWorkspaceSet {
                    workspace: None,
                    reason: crate::PreResolvedWorkspaceProblem::UnknownWorkspace,
                });
            }
            if !seen.insert(workspace.to_string()) {
                return Err(FrameworkError::InvalidPreResolvedWorkspaceSet {
                    workspace: Some(workspace.to_string()),
                    reason: crate::PreResolvedWorkspaceProblem::DuplicateWorkspace,
                });
            }
        }

        let mut canonical = resolved.clone();
        canonical
            .roots
            .sort_by(|left, right| left.id.cmp(&right.id));
        Ok(canonical)
    }

    /// Builds an invocation plan against pre-resolved workspace roots.
    /// Planning stays synchronous; observation gathering happens in the
    /// adapter and arrives here already resolved.
    pub fn build_plan_with_workspaces(
        &self,
        request: &RunRequest,
        resolved: &ResolvedWorkspaceSet,
    ) -> Result<InvocationPlan> {
        self.build_plan_with_workspaces_and_context(
            request,
            resolved,
            &crate::InvocationContext::default(),
        )
    }

    /// Builds an invocation plan against pre-resolved workspaces and private
    /// host invocation context. The context affects only declared private
    /// fingerprint facts and is never stored on the returned plan.
    pub fn build_plan_with_workspaces_and_context(
        &self,
        request: &RunRequest,
        resolved: &ResolvedWorkspaceSet,
        context: &crate::InvocationContext,
    ) -> Result<InvocationPlan> {
        if context.host_workspace_roots().is_some() {
            return Err(FrameworkError::ConflictingWorkspaceInputs);
        }
        let resolved = self.validate_pre_resolved_workspaces(resolved)?;
        self.build_plan_prepared(request, &resolved, context)
    }

    fn build_plan_prepared(
        &self,
        request: &RunRequest,
        resolved: &ResolvedWorkspaceSet,
        context: &crate::InvocationContext,
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

        // Signature-required resources bind through their carrier argument.
        // A missing carrier is a capability failure with derived recovery
        // edges, not a generic missing argument.
        for resource_name in registered
            .spec
            .requires_resources
            .iter()
            .chain(&registered.spec.releases)
        {
            let Some(resource) = self.resources.get(resource_name) else {
                continue;
            };
            if !request.args.contains_key(&resource.carrier_name()) {
                return Err(FrameworkError::CapabilityMissing {
                    capability: resource.name.clone(),
                    carrier: resource.carrier_name(),
                    providers: self.resource_granters(&resource.name),
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
                        diagnostics: Box::new([]),
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
                            diagnostics: Box::new([
                                mcp_workspace_resolver::WorkspaceDiagnostic::unsupported_scheme(
                                    Some(workspace_id.clone()),
                                    err.to_string(),
                                    value.to_string(),
                                ),
                            ]),
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
                        diagnostics: Box::new([]),
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

        // Optional workspace use contributes a selected execution fact when
        // resolution succeeds and remains absent otherwise.
        for workspace_name in &registered.spec.optional_workspaces {
            let workspace_id = mcp_workspace_resolver::WorkspaceId::from(workspace_name.as_str());
            if resolved.root(&workspace_id).is_some() {
                used_workspaces.insert(workspace_name.clone());
            }
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
        let mut fingerprint_input = json!({
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
        });
        if registered.spec.uses_conversation_identity {
            let identity_digest = context.conversation_identity().map(|identity| {
                stable_hash_value(&json!([
                    "conversation-identity",
                    identity.version(),
                    identity.issuer(),
                    identity.id(),
                ]))
            });
            fingerprint_input
                .as_object_mut()
                .expect("fingerprint input is an object")
                .insert(
                    "conversationIdentity".to_string(),
                    identity_digest.map_or(Value::Null, Value::String),
                );
        }
        let invocation_fingerprint = stable_hash_value(&fingerprint_input);

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
        self.run_with_context(request, crate::InvocationContext::default())
            .await
    }

    pub async fn run_with_context(
        &self,
        request: RunRequest,
        context: crate::InvocationContext,
    ) -> Result<RunResponse> {
        let resolved = self.resolve_context_workspaces(&context);
        self.run_with_lane_prepared(request, None, None, None, &resolved, &context)
            .await
    }

    pub async fn run_in_lane(
        &self,
        request: RunRequest,
        current_tool: impl Into<String>,
        lane: EffectLane,
        primary_tool_name: impl AsRef<str>,
    ) -> Result<RunResponse> {
        self.run_with_lane_prepared(
            request,
            Some(current_tool.into()),
            Some(lane),
            Some(primary_tool_name.as_ref().to_string()),
            &self.resolve_declared_workspaces(),
            &crate::InvocationContext::default(),
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
        self.run_in_lane_with_workspaces_and_context(
            request,
            current_tool,
            lane,
            primary_tool_name,
            resolved,
            &crate::InvocationContext::default(),
        )
        .await
    }

    /// Like [`run_in_lane_with_workspaces`](Self::run_in_lane_with_workspaces),
    /// with private host invocation context shared with planning and handler
    /// dispatch.
    pub async fn run_in_lane_with_workspaces_and_context(
        &self,
        request: RunRequest,
        current_tool: impl Into<String>,
        lane: EffectLane,
        primary_tool_name: impl AsRef<str>,
        resolved: &ResolvedWorkspaceSet,
        context: &crate::InvocationContext,
    ) -> Result<RunResponse> {
        if context.host_workspace_roots().is_some() {
            return Err(FrameworkError::ConflictingWorkspaceInputs);
        }
        let resolved = self.validate_pre_resolved_workspaces(resolved)?;
        self.run_with_lane_prepared(
            request,
            Some(current_tool.into()),
            Some(lane),
            Some(primary_tool_name.as_ref().to_string()),
            &resolved,
            context,
        )
        .await
    }

    async fn run_with_lane_prepared(
        &self,
        request: RunRequest,
        current_tool: Option<String>,
        current_lane: Option<EffectLane>,
        primary_tool_name: Option<String>,
        resolved: &ResolvedWorkspaceSet,
        context: &crate::InvocationContext,
    ) -> Result<RunResponse> {
        let plan = self.build_plan_prepared(&request, resolved, context)?;
        if let Some(current_lane) = current_lane
            && plan.lane != current_lane
        {
            let required_tool =
                self.required_tool_name(primary_tool_name.as_deref().unwrap_or("run"), plan.lane);
            return Err(FrameworkError::WrongEffectLane {
                current_tool: current_tool.unwrap_or_else(|| current_lane.tool_name("run")),
                required_tool,
            });
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

        let resources = self
            .resolve_signature_resources(&registered.spec, &plan)
            .await?;

        let handler_context = if registered.spec.uses_conversation_identity {
            context.clone()
        } else {
            crate::InvocationContext::default()
        };
        let output = registered
            .handler
            .call(CommandContext::with_invocation_context(
                plan.clone(),
                request.stdin,
                resources,
                handler_context,
            ))
            .await
            .map_err(|error| self.enrich_capability_denied(error))?;
        let output = self.mint_output_references(&registered.spec, output)?;
        let output = output.apply_output_spec(&plan.output);

        Ok(RunResponse {
            plan,
            output: Some(output),
            dry_run: false,
        })
    }

    /// Resolves every resource the handler signature requires or releases,
    /// reading the reference from the carrier argument, normalizing URI to
    /// id, and asking the bound resolver. A refusal short-circuits with
    /// derived recovery edges; the handler body never runs.
    async fn resolve_signature_resources(
        &self,
        spec: &CommandSpec,
        plan: &InvocationPlan,
    ) -> Result<crate::ResolvedResources> {
        let mut resolved = crate::ResolvedResources::default();
        for resource_name in spec.requires_resources.iter().chain(&spec.releases) {
            let decl = self.resources.get(resource_name).ok_or_else(|| {
                FrameworkError::Build(format!(
                    "resource `{resource_name}` is not declared on the server"
                ))
            })?;
            let resolver = self.resolvers.get(resource_name).ok_or_else(|| {
                FrameworkError::Build(format!("resource `{resource_name}` has no bound resolver"))
            })?;
            let carrier = decl.carrier_name();
            let reference = plan
                .bound_args
                .get(&carrier)
                .and_then(|arg| arg.value.as_str())
                .ok_or_else(|| FrameworkError::CapabilityMissing {
                    capability: decl.name.clone(),
                    carrier: carrier.clone(),
                    providers: self.resource_granters(&decl.name),
                })?;
            let id = decl.normalize_reference(reference);
            match resolver.resolve_erased(id, plan).await {
                Ok(value) => resolved.insert(decl.name.clone(), value),
                Err(refusal) => {
                    return Err(FrameworkError::ResourceRefused {
                        resource: decl.name.clone(),
                        reference: reference.to_string(),
                        detail: refusal.detail,
                        enumerate: self.resource_enumerators(&decl.name).into(),
                        establish: self.resource_granters(&decl.name).into(),
                    });
                }
            }
        }
        Ok(resolved)
    }

    /// Mints URIs for the references the handler granted or enumerated,
    /// from the declared template. Grant ids that would not round-trip are
    /// refused here — mint time, before the reference escapes. References
    /// outside the command's signature-derived edges are refused too:
    /// `CommandOutput.grants`/`listings` are public, and a hand-populated
    /// vector must not bypass the catalog graph.
    fn mint_output_references(
        &self,
        spec: &CommandSpec,
        mut output: CommandOutput,
    ) -> Result<CommandOutput> {
        let command = spec.name();
        for (reference, edge, declared) in output
            .grants
            .iter_mut()
            .map(|reference| (reference, "grant", &spec.grants))
            .chain(
                output
                    .listings
                    .iter_mut()
                    .map(|reference| (reference, "listing", &spec.enumerates)),
            )
        {
            if !declared.contains(&reference.resource) {
                return Err(FrameworkError::Handler(format!(
                    "command `{command}` emitted a {edge} for resource `{}`, which its signature does not declare; emit resources through `Grant`/`Listing` outputs so the edge is part of the catalog",
                    reference.resource
                )));
            }
            let decl = self.resources.get(&reference.resource).ok_or_else(|| {
                FrameworkError::Handler(format!(
                    "command `{command}` emitted a reference to resource `{}`, which is not declared on the server",
                    reference.resource
                ))
            })?;
            reference.uri = decl.mint_uri(&reference.id)?;
        }
        Ok(output)
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
        ];
        if let Some(preamble) = &self.preamble {
            lines.push(preamble.clone());
        }
        lines.extend([
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
        ]);
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

        if !self.resources.is_empty() {
            lines.push(String::new());
            lines.push("Resources:".to_string());
            for spec in self.resource_specs() {
                let mut line = format!("- `{}` ({}): {}", spec.name, spec.uri, spec.summary);
                if let Some(within) = &spec.within {
                    line.push_str(&format!(" Scoped within `{within}`."));
                }
                if let Some(lifetime) = &spec.lifetime {
                    line.push_str(&format!(" Lifetime: {lifetime}."));
                }
                if let Some(expiry) = &spec.expiry {
                    line.push_str(&format!(" Expiry: {expiry}."));
                }
                lines.push(line);
                if !spec.granted_by.is_empty() {
                    lines.push(format!(
                        "  granted by {}",
                        backticked_list(&spec.granted_by)
                    ));
                }
                if !spec.enumerated_by.is_empty() {
                    lines.push(format!(
                        "  enumerated by {}",
                        backticked_list(&spec.enumerated_by)
                    ));
                }
                if !spec.released_by.is_empty() {
                    lines.push(format!(
                        "  released by {}",
                        backticked_list(&spec.released_by)
                    ));
                }
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
        let mut sections = vec![format!("# `{}`", spec.name()), spec.description.clone()];
        if let Some(use_when) = &spec.use_when {
            sections.push(format!("Use when: {use_when}"));
        }
        if !spec.alternatives.is_empty() {
            let mut lines = vec!["Use instead:".to_string()];
            for alternative in &spec.alternatives {
                lines.push(format!(
                    "- `{}` — {}",
                    alternative.command, alternative.when
                ));
            }
            sections.push(lines.join("\n"));
        }
        if let Some(fallback) = &spec.fallback {
            let preferred = fallback
                .prefer
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            sections.push(format!(
                "Fallback: prefer {preferred}. Use only when {}.",
                fallback.when
            ));
        }
        let reverse_edges = self.derived_fallback_edges(&spec.name());
        if !reverse_edges.is_empty() {
            let mut lines = vec!["Fallbacks:".to_string()];
            for (name, when) in reverse_edges {
                lines.push(format!("- `{name}` — when {when}"));
            }
            sections.push(lines.join("\n"));
        }
        sections.push(self.arguments_text(spec));
        if spec.uses_conversation_identity {
            sections.push(
                "Request context:\n- conversation identity (optional, supplied by host)"
                    .to_string(),
            );
        }
        if !spec.workspaces.is_empty() || !spec.optional_workspaces.is_empty() {
            let mut lines = vec!["Workspaces:".to_string()];
            for name in &spec.workspaces {
                let description = self
                    .workspaces
                    .get(name)
                    .and_then(|decl| decl.description.as_deref())
                    .map(|text| format!("{text} "))
                    .unwrap_or_default();
                lines.push(format!(
                    "- `{name}`: {description}(required, supplied by host)"
                ));
            }
            for name in &spec.optional_workspaces {
                let description = self
                    .workspaces
                    .get(name)
                    .and_then(|decl| decl.description.as_deref())
                    .map(|text| format!("{text} "))
                    .unwrap_or_default();
                lines.push(format!(
                    "- `{name}`: {description}(optional, supplied by host)"
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
        if !spec.requires_resources.is_empty() {
            let mut lines = vec!["Requires resources:".to_string()];
            for name in &spec.requires_resources {
                lines.push(self.resource_requirement_line(name));
            }
            sections.push(lines.join("\n"));
        }
        if !spec.grants.is_empty() {
            let mut lines = vec!["Grants:".to_string()];
            for name in &spec.grants {
                lines.push(self.resource_grant_line(name));
            }
            sections.push(lines.join("\n"));
        }
        if !spec.releases.is_empty() {
            let mut lines = vec!["Releases:".to_string()];
            for name in &spec.releases {
                let summary = self
                    .resources
                    .get(name)
                    .map(|decl| format!(" — {}", decl.summary))
                    .unwrap_or_default();
                lines.push(format!("- `{name}`{summary}"));
            }
            sections.push(lines.join("\n"));
        }
        if !spec.enumerates.is_empty() {
            let mut lines = vec!["Enumerates:".to_string()];
            for name in &spec.enumerates {
                let summary = self
                    .resources
                    .get(name)
                    .map(|decl| format!(" — {}", decl.summary))
                    .unwrap_or_default();
                lines.push(format!(
                    "- `{name}`{summary} (the recovery path for lost references)"
                ));
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
                let fallback = variant
                    .fallback
                    .as_ref()
                    .map(|when| format!(" (fallback — {when})"))
                    .unwrap_or_default();
                lines.push(format!(
                    "  - {}{fallback}: {}",
                    variant.name, variant.summary
                ));
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

fn backticked_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Walks the fallback-preference graph looking for a cycle. Two escape
/// hatches preferring each other describe no ladder; registration rejects
/// the pair.
fn find_fallback_cycle<'a>(
    current: &'a str,
    fallback_prefer: &'a BTreeMap<String, Vec<String>>,
    stack: &mut Vec<&'a str>,
) -> Result<()> {
    let Some(preferred) = fallback_prefer.get(current) else {
        return Ok(());
    };
    for next in preferred {
        if stack.contains(&next.as_str()) {
            return Err(FrameworkError::Build(format!(
                "fallback preference cycle: {} -> `{next}`",
                stack
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(" -> ")
            )));
        }
        stack.push(next);
        find_fallback_cycle(next, fallback_prefer, stack)?;
        stack.pop();
    }
    Ok(())
}

fn lane_description(primary_tool_name: &str, lane: EffectLane) -> String {
    match lane {
        EffectLane::Primary => "Primary execution tool. Start here for all command templates; the framework returns structured retry data when another effect lane is required.".to_string(),
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
