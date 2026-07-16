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
    CatalogIdentity, CommandCatalog, CommandContext, CommandExecutionOutcome, CommandGuidance,
    CommandOutput, CommandSpec, EffectLane, FrameworkError, HelpRequest, HelpResult, HelpTopic,
    InvocationPlan, InvocationToken, OperationSpec, PermissionPolicy, Result, RunRequest,
    RunResponse, ServerSpec, TemplateToken, ToolLaneSpec, WorkspaceDecl, group_namespaces,
    stable_hash_value, structured_error, value_matches_type,
};
use crate::{CommandTemplate, PermissionSpec};

const MAX_GUIDANCE_SCALARS: usize = 1_024;

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

#[async_trait]
impl CommandHandler for Arc<dyn CommandHandler> {
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        self.as_ref().call(context).await
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
    argument_schemas: BTreeMap<String, crate::ArgumentSchemaDecl>,
    duplicate_argument_schemas: Vec<String>,
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
    registration_errors: Vec<FrameworkError>,
    policy: PermissionPolicy,
}

#[derive(Clone)]
struct RegisteredCommand {
    spec: CommandSpec,
    handler: Arc<dyn crate::results::ErasedCommandHandler>,
    result_kind: ResultHandlerKind,
    argument_kind: ArgumentHandlerKind,
    result_resource_uses: Vec<crate::resource::ResourceUse>,
}

#[derive(Clone)]
enum ArgumentHandlerKind {
    Dynamic,
    LegacyTyped,
    Constrained(Value),
    ResultAware(Option<Value>),
}

#[derive(Clone)]
enum ResultHandlerKind {
    Legacy,
    Typed {
        pending: crate::results::PendingApplicationContract,
        explicit_application: bool,
    },
    Dynamic,
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
            argument_schemas: BTreeMap::new(),
            duplicate_argument_schemas: Vec::new(),
            capabilities: BTreeMap::new(),
            duplicate_capabilities: Vec::new(),
            resources: BTreeMap::new(),
            duplicate_resources: Vec::new(),
            resource_capabilities: BTreeSet::new(),
            resolvers: BTreeMap::new(),
            readers: BTreeMap::new(),
            guidance: Vec::new(),
            registration_errors: Vec::new(),
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

    pub fn declare_argument_schema(mut self, decl: crate::ArgumentSchemaDecl) -> Self {
        if self.argument_schemas.contains_key(&decl.name) {
            self.duplicate_argument_schemas.push(decl.name.clone());
        }
        self.argument_schemas.insert(decl.name.clone(), decl);
        self
    }

    pub fn argument_schemas(&self) -> impl Iterator<Item = &crate::ArgumentSchemaDecl> {
        self.argument_schemas.values()
    }

    pub fn declare_capability(mut self, decl: crate::CapabilityDecl) -> Self {
        let replaces_derived = self.resource_capabilities.remove(&decl.name);
        if self.capabilities.contains_key(&decl.name) && !replaces_derived {
            self.duplicate_capabilities.push(decl.name.clone());
        }
        self.capabilities.insert(decl.name.clone(), decl);
        self.refresh_typed_result_contracts();
        self
    }

    /// Declares a server-held resource (RFC 0012). Lifecycle edges derive
    /// from handler signatures, never from the declaration.
    pub fn declare_resource(mut self, decl: crate::ResourceDecl) -> Self {
        let duplicate = self.resources.contains_key(&decl.name);
        if duplicate {
            self.duplicate_resources.push(decl.name.clone());
        }
        let resource_name = decl.name.clone();
        let derived_capability =
            crate::CapabilityDecl::new(decl.name.clone(), decl.summary.clone())
                .carried_by(decl.carrier_name());
        self.resources.insert(resource_name.clone(), decl);
        if !self.capabilities.contains_key(&resource_name)
            || self.resource_capabilities.contains(&resource_name)
        {
            self.resource_capabilities.insert(resource_name.clone());
            self.capabilities
                .insert(resource_name.clone(), derived_capability);
        }
        if !duplicate {
            let decl = self
                .resources
                .get(&resource_name)
                .expect("inserted resource declaration")
                .clone();
            let mut errors = Vec::new();
            for command in self.commands.values_mut() {
                if command
                    .result_resource_uses
                    .iter()
                    .any(|resource_use| resource_use.resource == resource_name)
                {
                    match inject_result_resource_carrier(&mut command.spec, &decl) {
                        Ok(()) => canonicalize_result_resource_carriers(
                            &mut command.spec,
                            &command.result_resource_uses,
                            &self.resources,
                        ),
                        Err(error) => errors.push(error),
                    }
                }
            }
            self.registration_errors.extend(errors);
        }
        self.refresh_typed_result_contracts();
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

    pub fn register<H>(self, spec: CommandSpec, handler: H) -> Self
    where
        H: CommandHandler,
    {
        self.register_legacy_registration(spec, Arc::new(handler), false)
    }

    pub(crate) fn register_legacy_registration(
        mut self,
        mut spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
        typed_arguments: bool,
    ) -> Self {
        normalize_command_spec(&mut spec);
        self.commands.insert(
            spec.path.clone(),
            RegisteredCommand {
                spec,
                handler: Arc::new(crate::results::LegacyHandlerAdapter(handler)),
                result_kind: ResultHandlerKind::Legacy,
                argument_kind: if typed_arguments {
                    ArgumentHandlerKind::LegacyTyped
                } else {
                    ArgumentHandlerKind::Dynamic
                },
                result_resource_uses: Vec::new(),
            },
        );
        self.refresh_typed_result_contracts();
        self
    }

    pub(crate) fn register_constrained_registration(
        mut self,
        mut spec: CommandSpec,
        registration: crate::ConstrainedHandlerRegistration,
    ) -> Self {
        project_result_resources(
            &mut spec,
            &registration.resource_uses,
            &registration.granted,
            &registration.enumerated,
        );
        if let Err(error) =
            inject_result_resource_carriers(&mut spec, &registration.resource_uses, &self.resources)
        {
            self.registration_errors.push(error);
        }
        normalize_command_spec(&mut spec);
        self.commands.insert(
            spec.path.clone(),
            RegisteredCommand {
                spec,
                handler: Arc::new(crate::results::LegacyHandlerAdapter(registration.handler)),
                result_kind: ResultHandlerKind::Legacy,
                argument_kind: ArgumentHandlerKind::Constrained(registration.argument_schema),
                result_resource_uses: registration.resource_uses,
            },
        );
        self.refresh_typed_result_contracts();
        self
    }

    pub fn register_result<M, H>(self, spec: CommandSpec, handler: H) -> Self
    where
        H: crate::ApplicationResultDialect<M>,
    {
        let registration = handler.into_result_registration();
        self.register_result_registration(spec, registration)
    }

    pub(crate) fn register_result_registration(
        mut self,
        mut spec: CommandSpec,
        registration: crate::results::ResultHandlerRegistration,
    ) -> Self {
        let explicit_application = spec
            .output
            .as_ref()
            .is_some_and(|output| output.application.is_some());
        project_result_resources(
            &mut spec,
            &registration.resource_uses,
            &registration.granted,
            &registration.enumerated,
        );
        if let Err(error) =
            inject_result_resource_carriers(&mut spec, &registration.resource_uses, &self.resources)
        {
            self.registration_errors.push(error);
        }
        normalize_command_spec(&mut spec);
        self.commands.insert(
            spec.path.clone(),
            RegisteredCommand {
                spec,
                handler: registration.handler,
                result_kind: ResultHandlerKind::Typed {
                    pending: registration.pending,
                    explicit_application,
                },
                argument_kind: ArgumentHandlerKind::ResultAware(registration.argument_schema),
                result_resource_uses: registration.resource_uses,
            },
        );
        self.refresh_typed_result_contracts();
        self
    }

    pub fn register_dynamic<M, H>(self, spec: CommandSpec, handler: H) -> Self
    where
        H: crate::DynamicApplicationDialect<M>,
    {
        let registration = handler.into_dynamic_registration();
        self.register_dynamic_registration(spec, registration)
    }

    pub(crate) fn register_dynamic_registration(
        mut self,
        mut spec: CommandSpec,
        registration: crate::results::DynamicHandlerRegistration,
    ) -> Self {
        project_result_resources(
            &mut spec,
            &registration.resource_uses,
            &registration.granted,
            &registration.enumerated,
        );
        if let Err(error) =
            inject_result_resource_carriers(&mut spec, &registration.resource_uses, &self.resources)
        {
            self.registration_errors.push(error);
        }
        if let Some(contract) = spec
            .output
            .as_mut()
            .and_then(|output| output.application.as_mut())
            && let Err(error) = crate::results::compile_contract(contract)
        {
            self.registration_errors.push(error);
        }
        normalize_command_spec(&mut spec);
        self.commands.insert(
            spec.path.clone(),
            RegisteredCommand {
                spec,
                handler: registration.handler,
                result_kind: ResultHandlerKind::Dynamic,
                argument_kind: ArgumentHandlerKind::Dynamic,
                result_resource_uses: registration.resource_uses,
            },
        );
        self.refresh_typed_result_contracts();
        self
    }

    fn refresh_typed_result_contracts(&mut self) {
        let capabilities = &self.capabilities;
        let resource_capabilities = &self.resource_capabilities;
        let providers = self
            .commands
            .values()
            .map(|command| command.spec.clone())
            .collect::<Vec<_>>();
        for command in self.commands.values_mut() {
            let ResultHandlerKind::Typed {
                pending,
                explicit_application,
            } = &command.result_kind
            else {
                continue;
            };
            if *explicit_application {
                continue;
            }
            if let Ok(contract) = compile_pending_result_contract(
                &command.spec,
                pending,
                capabilities,
                resource_capabilities,
                &providers,
            ) {
                command
                    .spec
                    .output
                    .get_or_insert_with(crate::OutputContract::default)
                    .application = Some(contract);
            }
        }
    }

    pub fn validate_results(&self) -> Result<()> {
        if let Some(error) = self.registration_errors.first() {
            return Err(error.clone());
        }
        let providers = self
            .commands
            .values()
            .map(|command| command.spec.clone())
            .collect::<Vec<_>>();
        let mut identities = BTreeMap::<String, crate::ApplicationErrorSpec>::new();
        let mut actions = BTreeMap::<String, String>::new();
        for command in self.commands.values() {
            let contract = match &command.result_kind {
                ResultHandlerKind::Legacy => {
                    if command
                        .spec
                        .output
                        .as_ref()
                        .is_some_and(|output| output.application.is_some())
                    {
                        return Err(FrameworkError::Build(format!(
                            "legacy command `{}` cannot declare an application result contract",
                            command.spec.name()
                        )));
                    }
                    continue;
                }
                ResultHandlerKind::Typed {
                    pending,
                    explicit_application,
                } => {
                    if *explicit_application {
                        return Err(FrameworkError::Build(format!(
                            "typed result command `{}` cannot declare an explicit application result contract",
                            command.spec.name()
                        )));
                    }
                    let contract = compile_pending_result_contract(
                        &command.spec,
                        pending,
                        &self.capabilities,
                        &self.resource_capabilities,
                        &providers,
                    )?;
                    if command
                        .spec
                        .output
                        .as_ref()
                        .and_then(|output| output.application.as_ref())
                        != Some(&contract)
                    {
                        return Err(FrameworkError::Build(format!(
                            "typed result command `{}` has an explicit or stale application contract",
                            command.spec.name()
                        )));
                    }
                    contract
                }
                ResultHandlerKind::Dynamic => {
                    let mut contract = command
                        .spec
                        .output
                        .as_ref()
                        .and_then(|output| output.application.clone())
                        .ok_or_else(|| {
                        FrameworkError::Build(format!(
                            "dynamic result command `{}` is missing an application result contract",
                            command.spec.name()
                        ))
                        })?;
                    crate::results::compile_contract(&mut contract)?;
                    validate_explicit_capability_bindings(
                        &command.spec,
                        &contract,
                        &self.capabilities,
                        &self.resource_capabilities,
                        &providers,
                    )?;
                    contract
                }
            };
            validate_recovery_operations(&contract, &providers)?;
            for recovery in contract
                .errors
                .iter()
                .flat_map(|error| error.recoveries.iter())
            {
                if let crate::ApplicationRecoveryDecl::Action(action) = recovery {
                    if let Some(summary) = actions.get(&action.code) {
                        if summary != &action.summary {
                            return Err(FrameworkError::Build(format!(
                                "application recovery action `{}` has conflicting summaries",
                                action.code
                            )));
                        }
                    } else {
                        actions.insert(action.code.clone(), action.summary.clone());
                    }
                }
            }
            for error in contract.errors {
                if let Some(existing) = identities.get(&error.code) {
                    if existing.summary != error.summary
                        || existing.message != error.message
                        || existing.details_schema != error.details_schema
                    {
                        return Err(FrameworkError::Build(format!(
                            "application error `{}` has conflicting server-wide declarations",
                            error.code
                        )));
                    }
                } else {
                    identities.insert(error.code.clone(), error);
                }
            }
        }
        Ok(())
    }

    pub fn command_specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.values().map(|command| &command.spec)
    }

    pub(crate) fn prepare_effect_lane_presentation(
        &self,
        plan: &InvocationPlan,
        tool_name: &str,
        confirmation: crate::presentation::ConfirmationPresentationRequest,
    ) -> Result<crate::PreparedInvocationPresentation> {
        let command = self.commands.get(&plan.command_path).ok_or_else(|| {
            FrameworkError::Build(format!(
                "planned command `{}` is missing from the registry",
                plan.command_path.join(" ")
            ))
        })?;
        let defaults = effect_lane_presentation_defaults(tool_name)?;
        let arguments = plan
            .bound_args
            .iter()
            .map(|(name, argument)| (name.clone(), argument.value.clone()))
            .collect();
        Ok(command.spec.prepare_validated_presentation(
            &defaults,
            &plan.operation_id,
            &arguments,
            confirmation,
        ))
    }

    pub(crate) fn prepare_effect_lane_confirmation(
        &self,
        plan: &InvocationPlan,
        tool_name: &str,
    ) -> Result<crate::PreparedConfirmation> {
        self.prepare_effect_lane_presentation(
            plan,
            tool_name,
            crate::presentation::ConfirmationPresentationRequest::DeclaredOrSurfaceDefault,
        )?
        .confirmation
        .ok_or_else(|| {
            FrameworkError::Build(
                "effect-lane presentation did not prepare confirmation".to_string(),
            )
        })
    }

    pub(crate) fn bind_effect_lane_presentation_fingerprint(
        &self,
        plan: &mut InvocationPlan,
        tool_name: &str,
    ) -> Result<()> {
        let command = self.commands.get(&plan.command_path).ok_or_else(|| {
            FrameworkError::Build(format!(
                "planned command `{}` is missing from the registry",
                plan.command_path.join(" ")
            ))
        })?;
        if command.spec.invocation_message.is_some() && command.spec.confirmation.is_some() {
            return Ok(());
        }
        let defaults = effect_lane_presentation_defaults(tool_name)?;
        let mut fallback = serde_json::Map::new();
        if command.spec.invocation_message.is_none() {
            fallback.insert(
                "invocationMessage".to_string(),
                Value::String(defaults.invocation_message().to_string()),
            );
        }
        if command.spec.confirmation.is_none() {
            fallback.insert(
                "confirmationTitle".to_string(),
                Value::String(defaults.confirmation_title().to_string()),
            );
            fallback.insert(
                "confirmationMessage".to_string(),
                Value::String(defaults.confirmation_message().to_string()),
            );
        }
        plan.invocation_fingerprint = stable_hash_value(&json!({
            "invocationFingerprint": &plan.invocation_fingerprint,
            "surfacePresentationDefaults": fallback,
        }));
        Ok(())
    }

    pub fn operation_specs(&self) -> Vec<OperationSpec> {
        let mut operations = self
            .commands
            .values()
            .map(|command| {
                let mut operation = OperationSpec::from_command_spec(&command.spec);
                canonicalize_catalog_argument_schemas(&mut operation.args);
                operation
            })
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
            argument_schemas: self.canonical_argument_schema_decls(),
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
        let mut catalog_value = json!({
            "server": self.server_spec(),
            "namespaces": group_namespaces(operations),
            "operations": operations,
            "workspaces": self.workspaces.values().collect::<Vec<_>>(),
            "types": self.types.values().collect::<Vec<_>>(),
            "capabilities": self.capabilities.values().collect::<Vec<_>>(),
            "resources": self.resource_specs(),
            "guidance": self.guidance,
        });
        let argument_schemas = self.canonical_argument_schema_decls();
        if !argument_schemas.is_empty() {
            catalog_value
                .as_object_mut()
                .expect("catalog identity preimage is an object")
                .insert(
                    "argumentSchemas".to_string(),
                    serde_json::to_value(argument_schemas).expect("serialize argument schemas"),
                );
        }
        let resource_reference_schemas = self.canonical_resource_reference_schemas();
        if !resource_reference_schemas.is_empty() {
            catalog_value
                .as_object_mut()
                .expect("catalog identity preimage is an object")
                .insert(
                    "resourceReferenceSchemas".to_string(),
                    serde_json::to_value(resource_reference_schemas)
                        .expect("serialize resource reference schemas"),
                );
        }
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

    pub fn validate_argument_schemas(&self) -> Result<()> {
        if let Some(name) = self.duplicate_argument_schemas.first() {
            return Err(FrameworkError::Build(format!(
                "argument schema `{name}` is declared more than once"
            )));
        }
        for declaration in self.argument_schemas.values() {
            if !valid_schema_name(&declaration.name) {
                return Err(FrameworkError::Build(format!(
                    "argument schema `{}` must use non-empty lower-kebab-case",
                    declaration.name
                )));
            }
            if declaration.summary.trim().is_empty() {
                return Err(FrameworkError::Build(format!(
                    "argument schema `{}` has an empty summary",
                    declaration.name
                )));
            }
            let mut schema = declaration.schema.clone();
            crate::argument_schemas::canonicalize_schema(&mut schema)?;
        }
        let mut referenced = BTreeSet::new();
        for resource in self.resources.values() {
            let Some(schema) = &resource.reference_schema else {
                continue;
            };
            if let crate::ArgumentSchemaUse::Named { name } = schema {
                referenced.insert(name.clone());
            }
            let carrier = crate::ArgSpec {
                name: resource.carrier_name(),
                value_type: crate::ArgType::ResourceRef(resource.name.clone()),
                required: true,
                summary: format!(
                    "The `{}` to operate on; accepts a bare id or its URI.",
                    resource.name
                ),
                workspace: None,
                repeated: false,
                schema: Some(schema.clone()),
                requires_arguments: Vec::new(),
            };
            let compiled =
                crate::argument_schemas::compile_argument_schema(&carrier, &self.argument_schemas)?
                    .expect("resource reference has a schema");
            crate::argument_schemas::validate_resource_reference_domain(resource, &compiled)?;
        }
        for command in self.commands.values() {
            let mut has_constraints = false;
            let mut command_definitions = BTreeMap::<String, Value>::new();
            let args = command
                .spec
                .args
                .iter()
                .map(|arg| (arg.name.as_str(), arg))
                .collect::<BTreeMap<_, _>>();
            for arg in &command.spec.args {
                if let Some(crate::ArgumentSchemaUse::Named { name }) = &arg.schema {
                    referenced.insert(name.clone());
                }
                let compiled =
                    crate::argument_schemas::compile_argument_schema(arg, &self.argument_schemas)?;
                has_constraints |= compiled.is_some() || !arg.requires_arguments.is_empty();
                if let Some(definitions) = compiled
                    .as_ref()
                    .and_then(|compiled| compiled.schema.get("$defs"))
                    .and_then(Value::as_object)
                {
                    for (name, schema) in definitions {
                        if let Some(existing) = command_definitions.get(name)
                            && existing != schema
                        {
                            return Err(FrameworkError::Build(format!(
                                "command `{}` argument schemas define conflicting `$defs/{name}` schemas",
                                command.spec.name()
                            )));
                        }
                        command_definitions.insert(name.clone(), schema.clone());
                    }
                }
                for target in &arg.requires_arguments {
                    if target.is_empty() {
                        return Err(FrameworkError::Build(format!(
                            "command `{}` argument `{}` declares an empty required argument",
                            command.spec.name(),
                            arg.name
                        )));
                    }
                    if target == &arg.name {
                        return Err(FrameworkError::Build(format!(
                            "command `{}` argument `{}` requires itself",
                            command.spec.name(),
                            arg.name
                        )));
                    }
                    let Some(required) = args.get(target.as_str()) else {
                        return Err(FrameworkError::Build(format!(
                            "command `{}` argument `{}` requires unknown argument `{target}`",
                            command.spec.name(),
                            arg.name
                        )));
                    };
                    if arg.required || required.required {
                        return Err(FrameworkError::Build(format!(
                            "command `{}` presence edge `{}` -> `{target}` must connect two optional arguments",
                            command.spec.name(),
                            arg.name
                        )));
                    }
                }
            }
            let operation_id = crate::OperationSpec::from_command_spec(&command.spec).id;
            let excluded_properties = command
                .spec
                .args
                .iter()
                .filter_map(|arg| {
                    matches!(arg.value_type, crate::ArgType::ResourceRef(_))
                        .then_some(arg.name.clone())
                })
                .collect::<BTreeSet<_>>();
            match &command.argument_kind {
                ArgumentHandlerKind::LegacyTyped if has_constraints => {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` uses constrained arguments with a legacy typed handler; use `handle_constrained`",
                        command.spec.name()
                    )));
                }
                ArgumentHandlerKind::Constrained(derived) => {
                    crate::argument_schemas::validate_derived_argument_schema(
                        &operation_id,
                        self.arg_schema(&command.spec),
                        derived,
                        &excluded_properties,
                    )?;
                }
                ArgumentHandlerKind::ResultAware(Some(derived)) if has_constraints => {
                    crate::argument_schemas::validate_derived_argument_schema(
                        &operation_id,
                        self.arg_schema(&command.spec),
                        derived,
                        &excluded_properties,
                    )?;
                }
                ArgumentHandlerKind::Dynamic
                | ArgumentHandlerKind::LegacyTyped
                | ArgumentHandlerKind::ResultAware(_) => {}
            }
        }
        if let Some(dead) = self
            .argument_schemas
            .keys()
            .find(|name| !referenced.contains(*name))
        {
            return Err(FrameworkError::Build(format!(
                "argument schema `{dead}` is never referenced"
            )));
        }
        Ok(())
    }

    fn canonical_argument_schema_decls(&self) -> Vec<crate::ArgumentSchemaDecl> {
        self.argument_schemas
            .values()
            .cloned()
            .map(|mut declaration| {
                let _ = crate::argument_schemas::canonicalize_schema(&mut declaration.schema);
                declaration
            })
            .collect()
    }

    /// Validates presentation only after the command's authoritative
    /// argument schemas are available.
    pub fn validate_presentations(&self) -> Result<()> {
        for command in self.commands.values() {
            crate::presentation::validate_command_presentation(
                &command.spec,
                &self.argument_schemas,
            )?;
        }
        Ok(())
    }

    fn canonical_resource_reference_schemas(&self) -> BTreeMap<String, crate::ArgumentSchemaUse> {
        self.resources
            .values()
            .filter_map(|resource| {
                let mut schema_use = resource.reference_schema.clone()?;
                if let crate::ArgumentSchemaUse::Inline { schema } = &mut schema_use {
                    let _ = crate::argument_schemas::canonicalize_schema(schema);
                }
                Some((resource.name.clone(), schema_use))
            })
            .collect()
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
            if self
                .capability_bootstrap_providers(&capability.name)
                .is_empty()
            {
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
        self.capability_provider_specs(capability, |_| true)
    }

    /// Commands that can establish a capability from absence. Recovery
    /// paths use only this set: a refresh provider cannot run without the
    /// proof it would replace.
    fn capability_bootstrap_providers(&self, capability: &str) -> Vec<String> {
        self.capability_provider_specs(capability, |spec| {
            !spec.requires.iter().any(|required| required == capability)
        })
    }

    /// Commands that replace an already-held proof. Help renders these as a
    /// distinct role and never offers them as recovery from missing proof.
    fn capability_refresh_providers(&self, capability: &str) -> Vec<String> {
        self.capability_provider_specs(capability, |spec| {
            spec.requires.iter().any(|required| required == capability)
        })
    }

    fn capability_provider_specs(
        &self,
        capability: &str,
        role: impl Fn(&CommandSpec) -> bool,
    ) -> Vec<String> {
        let mut providers = self
            .commands
            .values()
            .filter(|command| {
                command
                    .spec
                    .provides
                    .iter()
                    .any(|provided| provided == capability)
                    && role(&command.spec)
            })
            .map(|command| (command.spec.path.join("."), command.spec.name()))
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| left.0.cmp(&right.0));
        providers.into_iter().map(|(_, name)| name).collect()
    }

    fn capability_recovery_providers(&self, capability: &str) -> Vec<String> {
        if self.resource_capabilities.contains(capability) {
            self.resource_granters(capability)
        } else {
            self.capability_bootstrap_providers(capability)
        }
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
                    "resource `{}` derives capability `{}`, which is also declared explicitly; the resource owns that name",
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
        if self.resource_capabilities.contains(capability) {
            return format!(
                "`{}`: {} (derived from resource `{}`; see Resources for lifecycle and recovery)",
                decl.name, decl.summary, decl.name
            );
        }
        let bootstrap = self.capability_bootstrap_providers(capability);
        let refresh = self.capability_refresh_providers(capability);
        let establish = if bootstrap.is_empty() {
            String::new()
        } else {
            format!(
                "; establish with {}",
                bootstrap
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let refresh = if refresh.is_empty() {
            String::new()
        } else {
            format!(
                "; refresh with {}",
                refresh
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "`{}`: {} (carried by `{}`{establish}{refresh})",
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

    /// Replaces every compatibility field on a legacy capability denial from
    /// the selected command's declaration graph. A handler cannot invent
    /// steering or use the legacy channel for resources or unrelated proof.
    fn normalize_capability_denied(
        &self,
        spec: &CommandSpec,
        error: FrameworkError,
    ) -> FrameworkError {
        let FrameworkError::CapabilityDenied {
            capability, detail, ..
        } = error
        else {
            return error;
        };
        let Some(decl) = self.capabilities.get(&capability) else {
            return FrameworkError::Handler(
                "legacy handler returned invalid capability denial".to_string(),
            );
        };
        if self.resource_capabilities.contains(&capability)
            || !spec.requires.iter().any(|required| required == &capability)
        {
            return FrameworkError::Handler(
                "legacy handler returned invalid capability denial".to_string(),
            );
        }
        FrameworkError::CapabilityDenied {
            capability,
            detail,
            carrier: Some(decl.carrier.clone()),
            providers: self.capability_bootstrap_providers(&decl.name),
        }
    }

    fn normalize_dispatch_error(
        &self,
        registered: &RegisteredCommand,
        error: FrameworkError,
    ) -> FrameworkError {
        let error = self.normalize_capability_denied(&registered.spec, error);
        let FrameworkError::ArgumentContractViolation { reason, .. } = error else {
            return error;
        };
        let checked_extractor = match &registered.argument_kind {
            ArgumentHandlerKind::Constrained(_) => true,
            ArgumentHandlerKind::ResultAware(Some(_)) => {
                command_has_argument_constraints(&registered.spec)
            }
            _ => false,
        };
        if checked_extractor && reason == crate::ArgumentContractReason::TypedDeserializationFailed
        {
            FrameworkError::ArgumentContractViolation {
                operation_id: registered.spec.path.join("."),
                argument: None,
                reason,
            }
        } else {
            FrameworkError::Handler(
                "handler returned invalid argument contract violation".to_string(),
            )
        }
    }

    /// The model-facing JSON schema for one command's arguments. Named types
    /// are fully inlined at the property site as a property-level `oneOf`
    /// (array-wrapped when repeated); no `$ref` indirection and no top-level
    /// `oneOf` appear anywhere in the output.
    pub fn arg_schema(&self, spec: &crate::CommandSpec) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        let mut definitions = serde_json::Map::new();
        for arg in &spec.args {
            let compiled =
                crate::argument_schemas::compile_argument_schema(arg, &self.argument_schemas)
                    .ok()
                    .flatten();
            let has_compiled_schema = compiled.is_some();
            let base = if let Some(compiled) = compiled {
                let mut schema = compiled.schema;
                if let Some(argument_definitions) = schema
                    .as_object_mut()
                    .and_then(|schema| schema.remove("$defs"))
                    .and_then(|definitions| definitions.as_object().cloned())
                {
                    for (name, schema) in argument_definitions {
                        definitions.entry(name).or_insert(schema);
                    }
                }
                schema
            } else {
                match &arg.value_type {
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
                    crate::ArgType::Integer => json!({
                        "type": "integer",
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
                }
            };
            let schema = if arg.repeated && !has_compiled_schema {
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
        let mut schema = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        });
        let dependencies = spec
            .args
            .iter()
            .filter(|arg| !arg.requires_arguments.is_empty())
            .map(|arg| {
                (
                    arg.name.clone(),
                    Value::Array(
                        arg.requires_arguments
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        if !dependencies.is_empty() {
            schema
                .as_object_mut()
                .expect("command schema is an object")
                .insert("dependentRequired".to_string(), Value::Object(dependencies));
        }
        if !definitions.is_empty() {
            schema
                .as_object_mut()
                .expect("command schema is an object")
                .insert("$defs".to_string(), Value::Object(definitions));
        }
        schema
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
        if let Some(preamble) = &self.preamble {
            if preamble.trim().is_empty() {
                return Err(FrameworkError::Build(
                    "server preamble is empty; declare text or remove it".to_string(),
                ));
            }
            validate_guidance_text(preamble, &format!("server `{}` preamble", self.server_name))?;
        }
        for type_decl in self.types.values() {
            for variant in &type_decl.variants {
                if let Some(when) = &variant.fallback {
                    if when.trim().is_empty() {
                        return Err(FrameworkError::Build(format!(
                            "type `{}` variant `{}` declares a fallback with an empty condition",
                            type_decl.name, variant.name
                        )));
                    }
                    validate_guidance_text(
                        when,
                        &format!(
                            "type `{}` variant `{}` fallback condition",
                            type_decl.name, variant.name
                        ),
                    )?;
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
                validate_guidance_text(
                    use_when,
                    &format!("command `{name}` `use_when` condition"),
                )?;
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
                validate_guidance_text(
                    &alternative.when,
                    &format!(
                        "command `{name}` alternative `{}` condition",
                        alternative.command
                    ),
                )?;
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
                validate_guidance_text(
                    &fallback.when,
                    &format!("command `{name}` fallback condition"),
                )?;
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
                    providers: self.capability_recovery_providers(&capability.name),
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

        for spec in &registered.spec.args {
            if spec.required && !request.args.contains_key(&spec.name) {
                return Err(FrameworkError::MissingArgument(spec.name.clone()));
            }
        }

        // Every supplied property schema and applicable presence edge enters
        // one deterministic mismatch election before specialized semantic
        // authorities (paths, named values, and resources) run.
        let mut schema_matches = BTreeMap::new();
        let mut schema_failures = Vec::new();
        for spec in &registered.spec.args {
            let Some(value) = request.args.get(&spec.name) else {
                continue;
            };
            if let Some(compiled) =
                crate::argument_schemas::compile_argument_schema(spec, &self.argument_schemas)?
            {
                match crate::argument_schemas::validate_value(&compiled, value) {
                    Ok(schema_match) => {
                        schema_matches.insert(spec.name.clone(), schema_match);
                    }
                    Err(failure) => schema_failures.push((spec.name.clone(), failure)),
                }
            }
            for target in &spec.requires_arguments {
                if !request.args.contains_key(target) {
                    schema_failures.push((
                        spec.name.clone(),
                        crate::argument_schemas::SchemaFailure {
                            path: String::new(),
                            pointer: String::new(),
                            keyword: crate::ArgumentSchemaKeyword::DependentRequired,
                            expected: target.clone(),
                            branches: Vec::new(),
                        },
                    ));
                }
            }
        }
        schema_failures.sort_by(|(left_argument, left), (right_argument, right)| {
            crate::argument_schemas::compare_argument_failures(
                left_argument,
                left,
                right_argument,
                right,
            )
        });
        if let Some((argument, failure)) = schema_failures.into_iter().next() {
            return Err(FrameworkError::ArgumentSchemaMismatch {
                argument,
                path: failure.path,
                keyword: failure.keyword,
                expected: failure.expected,
                branches: failure.branches,
            });
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
            if !schema_matches.contains_key(&spec.name) {
                value_matches_type(&spec.name, value, &spec.value_type)?;
            }
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
                    schema_match: schema_matches.remove(&spec.name).unwrap_or_default(),
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
        let mut argument_schemas = BTreeMap::new();
        for arg in &registered.spec.args {
            if let Some(compiled) =
                crate::argument_schemas::compile_argument_schema(arg, &self.argument_schemas)?
            {
                argument_schemas.insert(arg.name.clone(), compiled.schema);
            }
        }
        if !argument_schemas.is_empty() {
            fingerprint_input
                .as_object_mut()
                .expect("fingerprint input is an object")
                .insert(
                    "argumentSchemas".to_string(),
                    serde_json::to_value(argument_schemas).expect("argument schemas serialize"),
                );
        }
        let argument_requirements = registered
            .spec
            .args
            .iter()
            .filter(|arg| !arg.requires_arguments.is_empty())
            .map(|arg| (arg.name.clone(), arg.requires_arguments.clone()))
            .collect::<BTreeMap<_, _>>();
        if !argument_requirements.is_empty() {
            fingerprint_input
                .as_object_mut()
                .expect("fingerprint input is an object")
                .insert(
                    "argumentRequirements".to_string(),
                    serde_json::to_value(argument_requirements)
                        .expect("argument requirements serialize"),
                );
        }
        if registered.spec.invocation_message.is_some() || registered.spec.confirmation.is_some() {
            fingerprint_input
                .as_object_mut()
                .expect("fingerprint input is an object")
                .insert(
                    "presentationContract".to_string(),
                    json!({
                        "invocationMessage": &registered.spec.invocation_message,
                        "confirmation": &registered.spec.confirmation,
                    }),
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

    pub async fn run(&self, request: RunRequest) -> Result<CommandExecutionOutcome> {
        self.run_with_context(request, crate::InvocationContext::default())
            .await
    }

    pub async fn run_with_context(
        &self,
        request: RunRequest,
        context: crate::InvocationContext,
    ) -> Result<CommandExecutionOutcome> {
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
    ) -> Result<CommandExecutionOutcome> {
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
    ) -> Result<CommandExecutionOutcome> {
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
    ) -> Result<CommandExecutionOutcome> {
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
    ) -> Result<CommandExecutionOutcome> {
        self.validate_argument_schemas()?;
        self.validate_presentations()?;
        self.validate_results()?;
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
            return Ok(CommandExecutionOutcome::Success(RunResponse {
                plan,
                output: None,
                dry_run: true,
            }));
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
        let mut command_context = CommandContext::with_invocation_context(
            plan.clone(),
            request.stdin,
            resources,
            handler_context,
        );
        if command_has_argument_constraints(&registered.spec) {
            command_context = command_context.with_checked_argument_contract();
        }
        let outcome = registered
            .handler
            .call(command_context)
            .await
            .map_err(|error| self.normalize_dispatch_error(registered, error))?;
        match outcome {
            crate::results::HandlerOutcome::Success(output) => {
                let output = self.mint_output_references(&registered.spec, output)?;
                let output = output.apply_output_spec(&plan.output);
                Ok(CommandExecutionOutcome::Success(RunResponse {
                    plan,
                    output: Some(output),
                    dry_run: false,
                }))
            }
            crate::results::HandlerOutcome::ApplicationSuccess {
                value,
                grants,
                listings,
            } => {
                let contract = registered
                    .spec
                    .output
                    .as_ref()
                    .and_then(|output| output.application.as_ref())
                    .ok_or_else(|| {
                        FrameworkError::Build(format!(
                            "result-aware command `{}` has no compiled application contract",
                            registered.spec.name()
                        ))
                    })?;
                crate::results::validate_application_success(contract, &value)?;
                let output = CommandOutput {
                    text: None,
                    structured: Some(value),
                    stderr: Vec::new(),
                    next_cursor: None,
                    grants,
                    listings,
                };
                let output = self.mint_output_references(&registered.spec, output)?;
                let output = output.apply_output_spec(&plan.output);
                let output = output
                    .compact_text_from_structured(plan.output.max_bytes)
                    .map_err(|_| FrameworkError::ResultContractViolation {
                        boundary: crate::ResultContractBoundary::Success,
                        reason: crate::ResultContractReason::SerializationFailed,
                    })?;
                Ok(CommandExecutionOutcome::Success(RunResponse {
                    plan,
                    output: Some(output),
                    dry_run: false,
                }))
            }
            crate::results::HandlerOutcome::ApplicationError(raw) => {
                let contract = registered
                    .spec
                    .output
                    .as_ref()
                    .and_then(|output| output.application.as_ref())
                    .ok_or_else(|| {
                        FrameworkError::Build(format!(
                            "result-aware command `{}` has no compiled application contract",
                            registered.spec.name()
                        ))
                    })?;
                let error = crate::results::validate_application_error(contract, raw)?;
                Ok(CommandExecutionOutcome::ApplicationError { plan, error })
            }
        }
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

        if !self.argument_schemas.is_empty() {
            lines.push(String::new());
            lines.push("Argument schemas:".to_string());
            for declaration in self.canonical_argument_schema_decls() {
                lines.push(format!(
                    "- `{}`: {} — `{}`",
                    declaration.name,
                    declaration.summary,
                    serde_json::to_string(&declaration.schema).unwrap_or_else(|_| "{}".to_string())
                ));
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
        let spec = CommandTemplate::parse(command)
            .ok()
            .and_then(|template| {
                self.match_command(&template)
                    .map(|registered| &registered.spec)
            })
            .or_else(|| {
                self.commands
                    .values()
                    .find(|registered| registered.spec.path.join(".") == command)
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
        if let Some(invocation_message) = &spec.invocation_message {
            sections.push(format!("Invocation: {invocation_message}"));
        }
        if let Some(confirmation) = &spec.confirmation {
            let mut lines = vec![
                "Host confirmation may use this command's declared copy.".to_string(),
                format!("- Default: {}", confirmation.default.title),
            ];
            for case in &confirmation.cases {
                lines.push(format!(
                    "- When {}: {}",
                    confirmation_predicate_help(&case.when),
                    case.message.title
                ));
            }
            sections.push(lines.join("\n"));
        }
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
            for name in spec
                .requires
                .iter()
                .filter(|name| !self.resource_capabilities.contains(name.as_str()))
            {
                lines.push(format!("- {}", self.capability_help_line(name)));
            }
            if lines.len() > 1 {
                sections.push(lines.join("\n"));
            }
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
            for name in spec
                .provides
                .iter()
                .filter(|name| !self.resource_capabilities.contains(name.as_str()))
            {
                let summary = self
                    .capabilities
                    .get(name)
                    .map(|decl| format!(": {}", decl.summary))
                    .unwrap_or_default();
                let role = if spec.requires.iter().any(|required| required == name) {
                    format!(" (refresh provider; requires existing `{name}`)")
                } else {
                    " (bootstrap provider)".to_string()
                };
                lines.push(format!("- `{name}`{summary}{role}"));
            }
            if lines.len() > 1 {
                sections.push(lines.join("\n"));
            }
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
        if let Some(output) = &spec.output {
            let mut lines = vec!["Output:".to_string(), format!("- {}", output.summary)];
            if let Some(application) = &output.application {
                lines.push(format!(
                    "- Application success schema: `{}`",
                    serde_json::to_string(&application.success_schema)
                        .unwrap_or_else(|_| "{}".to_string())
                ));
                if !application.errors.is_empty() {
                    lines.push("Expected application errors:".to_string());
                    for error in &application.errors {
                        lines.push(format!("- `{}`: {}", error.code, error.summary));
                        for recovery in &error.recoveries {
                            match recovery {
                                crate::ApplicationRecoveryDecl::Operation { operation_id } => {
                                    lines.push(format!(
                                        "  - recover with operation `{operation_id}`"
                                    ));
                                }
                                crate::ApplicationRecoveryDecl::Action(action) => {
                                    lines.push(format!(
                                        "  - recovery action `{}`: {}",
                                        action.code, action.summary
                                    ));
                                }
                            }
                        }
                    }
                }
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
                "- `$args.{}`: {}, {}, {}{}{}",
                arg.name,
                value_type,
                required,
                arg.summary,
                workspace,
                self.argument_constraint_text(arg)
            ));
            for target in &arg.requires_arguments {
                lines.push(format!("  - when supplied, also requires `$args.{target}`"));
            }
        }
        let type_lines = self.referenced_types_text(spec);
        if !type_lines.is_empty() {
            lines.push(String::new());
            lines.extend(type_lines);
        }
        let mut named_schemas = spec
            .args
            .iter()
            .filter_map(|arg| match &arg.schema {
                Some(crate::ArgumentSchemaUse::Named { name }) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        named_schemas.sort();
        named_schemas.dedup();
        if !named_schemas.is_empty() {
            lines.push(String::new());
            for name in named_schemas {
                if let Some(declaration) = self.argument_schemas.get(name) {
                    let mut schema = declaration.schema.clone();
                    let _ = crate::argument_schemas::canonicalize_schema(&mut schema);
                    lines.push(format!(
                        "Schema `{}`: {}\n  `{}`",
                        declaration.name,
                        declaration.summary,
                        serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
                    ));
                }
            }
        }
        lines.join("\n")
    }

    fn argument_constraint_text(&self, arg: &crate::ArgSpec) -> String {
        match &arg.schema {
            Some(crate::ArgumentSchemaUse::Named { name }) => {
                format!("; schema `{name}`")
            }
            Some(crate::ArgumentSchemaUse::Inline { .. }) => {
                let Some(schema) =
                    crate::argument_schemas::compile_argument_schema(arg, &self.argument_schemas)
                        .ok()
                        .flatten()
                else {
                    return String::new();
                };
                format!(
                    "; schema `{}`",
                    serde_json::to_string(&schema.schema).unwrap_or_else(|_| "{}".to_string())
                )
            }
            None => String::new(),
        }
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

fn effect_lane_presentation_defaults(
    tool_name: &str,
) -> Result<crate::SurfacePresentationDefaults> {
    let display_title = format!("{tool_name} execution");
    crate::SurfacePresentationDefaults::new(
        format!("Running {display_title}"),
        "Confirmation required",
        format!("Run {display_title}?"),
    )
}

fn validate_guidance_text(text: &str, subject: &str) -> Result<()> {
    let scalar_count = text.chars().take(MAX_GUIDANCE_SCALARS + 1).count();
    if scalar_count > MAX_GUIDANCE_SCALARS {
        return Err(FrameworkError::Build(format!(
            "{subject} exceeds the 1,024 Unicode scalar limit ({scalar_count} or more)"
        )));
    }
    if let Some(scalar) = text
        .chars()
        .find(|scalar| guidance_scalar_is_unsafe(*scalar))
    {
        return Err(FrameworkError::Build(format!(
            "{subject} contains presentation-unsafe scalar U+{:04X}",
            scalar as u32
        )));
    }
    Ok(())
}

fn guidance_scalar_is_unsafe(scalar: char) -> bool {
    matches!(scalar,
        '\u{0000}'..='\u{001F}'
        | '\u{007F}'..='\u{009F}'
        | '\u{061C}'
        | '\u{200E}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
    )
}

fn confirmation_predicate_help(predicate: &crate::ConfirmationPredicate) -> String {
    match predicate {
        crate::ConfirmationPredicate::ArgumentPresent { argument } => {
            format!("argument `{argument}` is present")
        }
        crate::ConfirmationPredicate::ArgumentEquals { argument, .. } => {
            format!("argument `{argument}` equals its declared case value")
        }
    }
}

fn normalize_command_spec(spec: &mut CommandSpec) {
    spec.workspaces.sort();
    spec.workspaces.dedup();
    spec.optional_workspaces.sort();
    spec.optional_workspaces.dedup();
    for arg in &mut spec.args {
        arg.requires_arguments.sort();
        arg.requires_arguments.dedup();
    }
}

pub(crate) fn canonicalize_catalog_argument_schemas(args: &mut [crate::ArgSpec]) {
    for arg in args {
        if let Some(crate::ArgumentSchemaUse::Inline { schema }) = &mut arg.schema {
            // Catalog arguments retain the authorship distinction and the
            // separate `repeated` bit, so canonicalize the declared base
            // document rather than replacing it with the compiled/wrapped
            // effective property schema.
            let _ = crate::argument_schemas::canonicalize_schema(schema);
        }
    }
}

fn valid_schema_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn command_has_argument_constraints(spec: &CommandSpec) -> bool {
    spec.args.iter().any(|arg| {
        arg.schema.is_some()
            || matches!(arg.value_type, crate::ArgType::Integer)
            || !arg.requires_arguments.is_empty()
    })
}

fn project_result_resources(
    spec: &mut CommandSpec,
    uses: &[crate::resource::ResourceUse],
    granted: &[&'static str],
    enumerated: &[&'static str],
) {
    for resource_use in uses {
        spec.requires.push(resource_use.resource.to_string());
        if resource_use.released {
            spec.releases.push(resource_use.resource.to_string());
        } else {
            spec.requires_resources
                .push(resource_use.resource.to_string());
        }
    }
    spec.grants
        .extend(granted.iter().map(|name| (*name).to_string()));
    spec.provides
        .extend(granted.iter().map(|name| (*name).to_string()));
    spec.enumerates
        .extend(enumerated.iter().map(|name| (*name).to_string()));
    for values in [
        &mut spec.requires_resources,
        &mut spec.releases,
        &mut spec.grants,
        &mut spec.enumerates,
        &mut spec.requires,
        &mut spec.provides,
    ] {
        values.sort();
        values.dedup();
    }
}

fn inject_result_resource_carriers(
    spec: &mut CommandSpec,
    uses: &[crate::resource::ResourceUse],
    resources: &BTreeMap<String, crate::ResourceDecl>,
) -> Result<()> {
    let resource_names = uses
        .iter()
        .map(|resource_use| resource_use.resource)
        .collect::<BTreeSet<_>>();
    for resource_name in resource_names {
        if let Some(decl) = resources.get(resource_name) {
            inject_result_resource_carrier(spec, decl)?;
        }
    }
    canonicalize_result_resource_carriers(spec, uses, resources);
    Ok(())
}

fn canonicalize_result_resource_carriers(
    spec: &mut CommandSpec,
    uses: &[crate::resource::ResourceUse],
    resources: &BTreeMap<String, crate::ResourceDecl>,
) {
    let carrier_resources = uses
        .iter()
        .filter_map(|resource_use| resources.get(resource_use.resource))
        .map(|decl| (decl.carrier_name(), decl.name.clone()))
        .collect::<BTreeMap<_, _>>();
    spec.args.sort_by(|left, right| {
        match (
            carrier_resources.get(&left.name),
            carrier_resources.get(&right.name),
        ) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(left_resource), Some(right_resource)) => left_resource.cmp(right_resource),
        }
    });
}

fn inject_result_resource_carrier(
    spec: &mut CommandSpec,
    decl: &crate::ResourceDecl,
) -> Result<()> {
    let carrier = decl.carrier_name();
    if spec.args.iter().any(|arg| arg.name == carrier) {
        return Err(FrameworkError::Build(format!(
            "command `{}` hand-declares argument `{carrier}`, which is the injected carrier for resource `{}`; remove the argument or rename the carrier with `carrier` on the resource declaration",
            spec.name(),
            decl.name
        )));
    }
    spec.args.push(crate::ArgSpec {
        name: carrier,
        value_type: crate::ArgType::ResourceRef(decl.name.clone()),
        required: true,
        summary: format!(
            "The `{}` to operate on; accepts a bare id or its URI.",
            decl.name
        ),
        workspace: None,
        repeated: false,
        schema: decl.reference_schema.clone(),
        requires_arguments: Vec::new(),
    });
    Ok(())
}

fn compile_pending_result_contract(
    spec: &CommandSpec,
    pending: &crate::results::PendingApplicationContract,
    capabilities: &BTreeMap<String, crate::CapabilityDecl>,
    resource_capabilities: &BTreeSet<String>,
    commands: &[CommandSpec],
) -> Result<crate::ApplicationResultContract> {
    let errors =
        crate::results::compose_error_specs(&pending.declarations, &pending.uses, |capability| {
            valid_capability_binding(spec, capability, capabilities, resource_capabilities)
                .then(|| bootstrap_providers(commands, capability))
        })?;
    let mut contract = crate::ApplicationResultContract {
        success_schema: pending.success_schema.clone(),
        errors,
    };
    crate::results::compile_contract(&mut contract)?;
    validate_recovery_operations(&contract, commands)?;
    Ok(contract)
}

fn valid_capability_binding(
    spec: &CommandSpec,
    capability: &str,
    capabilities: &BTreeMap<String, crate::CapabilityDecl>,
    resource_capabilities: &BTreeSet<String>,
) -> bool {
    capabilities.contains_key(capability)
        && !resource_capabilities.contains(capability)
        && spec.requires.iter().any(|required| required == capability)
}

fn bootstrap_providers(commands: &[CommandSpec], capability: &str) -> Vec<String> {
    let mut providers = commands
        .iter()
        .filter(|candidate| {
            candidate
                .provides
                .iter()
                .any(|provided| provided == capability)
                && !candidate
                    .requires
                    .iter()
                    .any(|required| required == capability)
        })
        .map(|candidate| candidate.path.join("."))
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    providers
}

fn validate_explicit_capability_bindings(
    spec: &CommandSpec,
    contract: &crate::ApplicationResultContract,
    capabilities: &BTreeMap<String, crate::CapabilityDecl>,
    resource_capabilities: &BTreeSet<String>,
    commands: &[CommandSpec],
) -> Result<()> {
    for error in &contract.errors {
        let Some(capability) = &error.capability else {
            continue;
        };
        if !valid_capability_binding(spec, capability, capabilities, resource_capabilities) {
            return Err(FrameworkError::Build(format!(
                "application error `{}` binds capability `{capability}` that command `{}` does not require as a hand-declared capability",
                error.code,
                spec.name()
            )));
        }
        let expected = bootstrap_providers(commands, capability)
            .into_iter()
            .map(|operation_id| crate::ApplicationRecoveryDecl::Operation { operation_id })
            .collect::<Vec<_>>();
        if error.recovery_cardinality != crate::RecoveryCardinality::Any
            || error.recoveries != expected
        {
            return Err(FrameworkError::Build(format!(
                "application error `{}` does not use the canonical bootstrap recoveries for capability `{capability}`",
                error.code
            )));
        }
    }
    Ok(())
}

fn validate_recovery_operations(
    contract: &crate::ApplicationResultContract,
    commands: &[CommandSpec],
) -> Result<()> {
    let mut operation_ids = BTreeMap::<String, usize>::new();
    for command in commands {
        *operation_ids.entry(command.path.join(".")).or_default() += 1;
    }
    for error in &contract.errors {
        for recovery in &error.recoveries {
            if let crate::ApplicationRecoveryDecl::Operation { operation_id } = recovery {
                match operation_ids.get(operation_id) {
                    Some(1) => {}
                    Some(_) => {
                        return Err(FrameworkError::Build(format!(
                            "application error `{}` recovers with ambiguous operation id `{operation_id}`",
                            error.code
                        )));
                    }
                    None => {
                        return Err(FrameworkError::Build(format!(
                            "application error `{}` recovers with unknown operation `{operation_id}`",
                            error.code
                        )));
                    }
                }
            }
        }
    }
    Ok(())
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
