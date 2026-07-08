use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    marker::PhantomData,
    sync::Arc,
};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    Alternative, ArgSpec, ArgType, CapabilityDecl, CommandContext, CommandExample, CommandGuidance,
    CommandHandler, CommandOutput, CommandRegistry, CommandSpec, Fallback, FrameworkError,
    OutputContract, PermissionSpec, ProgressPhaseSpec, ResourceDecl, Result, StdinContract,
    TypeDecl, WorkspaceDecl,
    resource::{ReadResource, ResolveResource, Resource, ResourceDialect},
};

pub mod arg {
    use super::ArgBuilder;
    use crate::ArgType;

    pub fn string(name: impl Into<String>) -> ArgBuilder {
        ArgBuilder::new(name, ArgType::String)
    }

    pub fn path(name: impl Into<String>, workspace: impl Into<String>) -> ArgBuilder {
        ArgBuilder::new(name, ArgType::Path).workspace(workspace)
    }

    pub fn boolean(name: impl Into<String>) -> ArgBuilder {
        ArgBuilder::new(name, ArgType::Bool)
    }

    pub fn number(name: impl Into<String>) -> ArgBuilder {
        ArgBuilder::new(name, ArgType::Number)
    }

    pub fn json(name: impl Into<String>) -> ArgBuilder {
        ArgBuilder::new(name, ArgType::Json)
    }

    /// An argument whose values match a declared named type (see
    /// `ServerBuilder::declare_type`).
    pub fn named(name: impl Into<String>, type_name: impl Into<String>) -> ArgBuilder {
        ArgBuilder::new(name, ArgType::Named(type_name.into()))
    }
}

impl CommandRegistry {
    pub fn build(
        name: impl Into<String>,
        description: impl Into<String>,
        build: impl FnOnce(&mut ServerBuilder),
    ) -> Result<Self> {
        let mut builder = ServerBuilder::new(name, description);
        build(&mut builder);
        builder.finish()
    }
}

pub struct ServerBuilder {
    name: String,
    description: String,
    preamble: Option<String>,
    workspaces: Vec<WorkspaceDecl>,
    capabilities: Vec<CapabilityDecl>,
    types: Vec<TypeDecl>,
    resources: Vec<ResourceDecl>,
    resource_bindings: Vec<Box<dyn FnOnce(CommandRegistry) -> CommandRegistry>>,
    guidance: Vec<CommandGuidance>,
    commands: Vec<BuiltCommand>,
    command_paths: BTreeSet<Vec<String>>,
    errors: Vec<FrameworkError>,
}

struct BuiltCommand {
    spec: CommandSpec,
    handler: SharedCommandHandler,
}

impl ServerBuilder {
    fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            preamble: None,
            workspaces: Vec::new(),
            capabilities: Vec::new(),
            types: Vec::new(),
            resources: Vec::new(),
            resource_bindings: Vec::new(),
            guidance: Vec::new(),
            commands: Vec::new(),
            command_paths: BTreeSet::new(),
            errors: Vec::new(),
        }
    }

    /// Declares server-level operating guidance: posture and conventions
    /// that apply across commands. Projects into server help, the MCP
    /// `instructions` field, and the `getting_started` prompt. Command
    /// routing belongs on the commands themselves (`use_when`,
    /// `alternative`, `fallback`), not here.
    pub fn preamble(&mut self, text: impl Into<String>) -> &mut Self {
        self.preamble = Some(text.into());
        self
    }

    pub fn workspace(&mut self, workspace: WorkspaceDecl) -> &mut Self {
        self.workspaces.push(workspace);
        self
    }

    pub fn capability(&mut self, capability: CapabilityDecl) -> &mut Self {
        self.capabilities.push(capability);
        self
    }

    pub fn declare_type(&mut self, decl: TypeDecl) -> &mut Self {
        self.types.push(decl);
        self
    }

    /// Declares a server-held resource (RFC 0012). Declaring one derives a
    /// reference argument type (`{name}-ref`) and a capability (`{name}`);
    /// the lifecycle edges derive from handler signatures, never from the
    /// declaration.
    pub fn resource(&mut self, decl: ResourceDecl) -> &mut Self {
        self.resources.push(decl);
        self
    }

    /// Binds the resolver that turns references to `T` into live values.
    /// Required for any resource a command requires or releases.
    pub fn resolver<T: Resource>(&mut self, resolver: impl ResolveResource<T>) -> &mut Self {
        self.resource_bindings
            .push(Box::new(move |registry| registry.with_resolver(resolver)));
        self
    }

    /// Binds a reader for `T`. On MCP, a bound reader turns grants into
    /// `resource_link` content parts and serves `resources/read` for minted
    /// URIs; without one, the URI in the structured payload is the whole
    /// story.
    pub fn reader<T: Resource>(&mut self, reader: impl ReadResource<T>) -> &mut Self {
        self.resource_bindings
            .push(Box::new(move |registry| registry.with_reader(reader)));
        self
    }

    pub fn guidance(&mut self, guidance: CommandGuidance) -> &mut Self {
        self.guidance.push(guidance);
        self
    }

    pub fn command(
        &mut self,
        path: impl IntoCommandPath,
        build: impl FnOnce(&mut CommandBuilder),
    ) -> &mut Self {
        let path = path.into_command_path();
        if path.is_empty() {
            self.errors
                .push(FrameworkError::Build("command path is empty".to_string()));
            return self;
        }

        if !self.command_paths.insert(path.clone()) {
            self.errors.push(FrameworkError::Build(format!(
                "duplicate command `{}`",
                path.join(" ")
            )));
            return self;
        }

        let mut command = CommandBuilder::new(path);
        build(&mut command);
        let workspace_names = self
            .workspaces
            .iter()
            .map(|workspace| workspace.name.as_str())
            .collect::<BTreeSet<_>>();

        match command.finish(&workspace_names) {
            Ok(command) => self.commands.push(command),
            Err(error) => self.errors.push(error),
        }
        self
    }

    fn finish(mut self) -> Result<CommandRegistry> {
        if let Some(error) = self.errors.drain(..).next() {
            return Err(error);
        }

        self.project_resource_signatures()?;

        let mut registry = CommandRegistry::new(self.name, self.description);
        if let Some(preamble) = self.preamble.take() {
            registry = registry.declare_preamble(preamble);
        }
        for workspace in self.workspaces.drain(..) {
            registry = registry.declare_workspace(workspace);
        }
        let hand_declared_capabilities = self
            .capabilities
            .iter()
            .map(|capability| capability.name.clone())
            .collect::<BTreeSet<_>>();
        for capability in self.capabilities.drain(..) {
            registry = registry.declare_capability(capability);
        }
        for decl in self.types.drain(..) {
            registry = registry.declare_type(decl);
        }
        for decl in &self.resources {
            registry = registry.declare_resource(decl.clone());
            // A hand-declared capability with the resource's name is a
            // collision `validate_resources` reports; declaring the derived
            // capability over it would mask that.
            if !hand_declared_capabilities.contains(&decl.name) {
                registry = registry.declare_derived_capability(
                    CapabilityDecl::new(decl.name.clone(), decl.summary.clone())
                        .carried_by(decl.carrier_name()),
                );
            }
        }
        for binding in self.resource_bindings {
            registry = binding(registry);
        }
        for guidance in self.guidance.drain(..) {
            registry = registry.declare_guidance(guidance);
        }
        for command in self.commands {
            registry = registry.register(command.spec, command.handler);
        }
        registry.validate_types()?;
        registry.validate_workspaces()?;
        registry.validate_capabilities()?;
        registry.validate_examples()?;
        registry.validate_guidance()?;
        registry.validate_resources()?;
        Ok(registry)
    }

    /// Projects signature-derived resource facts onto command specs, with
    /// every declaration in view: a required or released resource surfaces
    /// as a required carrier argument of the derived reference type, and a
    /// hand-written capability edge repeating a signature-derived fact
    /// deduplicates to the derived one.
    fn project_resource_signatures(&mut self) -> Result<()> {
        let decls = self
            .resources
            .iter()
            .map(|decl| (decl.name.clone(), decl.clone()))
            .collect::<BTreeMap<_, _>>();
        for command in &mut self.commands {
            let spec = &mut command.spec;
            let mut injected = BTreeSet::new();
            let resolved = spec
                .requires_resources
                .iter()
                .chain(&spec.releases)
                .cloned()
                .collect::<Vec<_>>();
            for resource in resolved {
                if !injected.insert(resource.clone()) {
                    continue;
                }
                // Undeclared resources are `validate_resources` errors; skip
                // injection so that check names the real problem.
                let Some(decl) = decls.get(&resource) else {
                    continue;
                };
                let carrier = decl.carrier_name();
                // A hand-written argument under the carrier name would
                // shadow the injected one with a different type or
                // optionality, so the advertised schema would drift from
                // the signature-derived requirement.
                if spec.args.iter().any(|arg| arg.name == carrier) {
                    return Err(FrameworkError::Build(format!(
                        "command `{}` hand-declares argument `{carrier}`, which is the injected carrier for resource `{}`; remove the argument or rename the carrier with `carrier` on the resource declaration",
                        spec.name(),
                        decl.name
                    )));
                }
                spec.args.push(ArgSpec {
                    name: carrier,
                    value_type: ArgType::ResourceRef(resource.clone()),
                    required: true,
                    summary: format!(
                        "The `{}` to operate on; accepts a bare id or its URI.",
                        decl.name
                    ),
                    workspace: None,
                    repeated: false,
                });
            }
            let covered_requires = spec
                .requires_resources
                .iter()
                .chain(&spec.releases)
                .filter(|resource| decls.contains_key(*resource))
                .cloned()
                .collect::<BTreeSet<_>>();
            spec.requires
                .retain(|capability| !covered_requires.contains(capability));
            let covered_provides = spec
                .grants
                .iter()
                .filter(|resource| decls.contains_key(*resource))
                .cloned()
                .collect::<BTreeSet<_>>();
            spec.provides
                .retain(|capability| !covered_provides.contains(capability));
        }
        Ok(())
    }
}

pub struct CommandBuilder {
    path: Vec<String>,
    summary: Option<String>,
    description: Option<String>,
    args: Vec<ArgSpec>,
    permissions: Vec<PermissionSpec>,
    examples: Vec<CommandExample>,
    output: Option<OutputContract>,
    stdin: Option<StdinContract>,
    progress: Vec<ProgressPhaseSpec>,
    idempotent: bool,
    workspaces: Vec<String>,
    requires: Vec<String>,
    provides: Vec<String>,
    use_when: Option<String>,
    alternatives: Vec<Alternative>,
    fallback: Option<Fallback>,
    requires_resources: Vec<String>,
    grants: Vec<String>,
    releases: Vec<String>,
    enumerates: Vec<String>,
    handler: Option<SharedCommandHandler>,
    errors: Vec<FrameworkError>,
}

impl CommandBuilder {
    fn new(path: Vec<String>) -> Self {
        Self {
            path,
            summary: None,
            description: None,
            args: Vec::new(),
            permissions: Vec::new(),
            examples: Vec::new(),
            output: None,
            stdin: None,
            progress: Vec::new(),
            idempotent: false,
            workspaces: Vec::new(),
            requires: Vec::new(),
            provides: Vec::new(),
            use_when: None,
            alternatives: Vec::new(),
            fallback: None,
            requires_resources: Vec::new(),
            grants: Vec::new(),
            releases: Vec::new(),
            enumerates: Vec::new(),
            handler: None,
            errors: Vec::new(),
        }
    }

    pub fn summary(&mut self, summary: impl Into<String>) -> &mut Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn description(&mut self, description: impl Into<String>) -> &mut Self {
        self.description = Some(description.into());
        self
    }

    pub fn arg(&mut self, arg: ArgBuilder) -> &mut Self {
        self.args.push(arg.finish());
        self
    }

    pub fn read(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self {
        self.permissions
            .push(PermissionSpec::read(scope, description));
        self
    }

    pub fn write(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self {
        self.permissions
            .push(PermissionSpec::write(scope, description));
        self
    }

    /// Declares that re-issuing this command with identical arguments is
    /// safe. Projects into the catalog and the invocation plan, where a
    /// runtime host's retry policy reads it. The framework cannot verify the
    /// handler actually deduplicates; the declaration is the author's promise.
    pub fn idempotent(&mut self) -> &mut Self {
        self.idempotent = true;
        self
    }

    /// Declares that this command requires the named workspace resolved,
    /// without taking a path argument. The resolved root reaches the handler
    /// through the plan (`CommandContext::workspace_root`); it is never
    /// caller-supplied. Planning fails when the workspace does not resolve.
    pub fn uses_workspace(&mut self, name: impl Into<String>) -> &mut Self {
        self.workspaces.push(name.into());
        self
    }

    /// Declares that this command requires the named capability. The
    /// capability's carrier argument must be declared on this command as a
    /// required argument; registration fails otherwise.
    pub fn requires(&mut self, capability: impl Into<String>) -> &mut Self {
        self.requires.push(capability.into());
        self
    }

    /// Declares that this command establishes the named capability. Steering
    /// and help derive "establish it with ..." guidance from this declaration.
    pub fn provides(&mut self, capability: impl Into<String>) -> &mut Self {
        self.provides.push(capability.into());
        self
    }

    /// One sentence: when this command is the right choice. Positive
    /// polarity; mutually exclusive with `fallback`.
    pub fn use_when(&mut self, text: impl Into<String>) -> &mut Self {
        self.use_when = Some(text.into());
        self
    }

    /// Declares a routing edge to the command serving a neighboring case,
    /// with the condition that routes there.
    pub fn alternative(
        &mut self,
        command: impl Into<String>,
        when: impl Into<String>,
    ) -> &mut Self {
        self.alternatives.push(Alternative {
            command: command.into(),
            when: when.into(),
        });
        self
    }

    /// Marks this command as an escape hatch: the commands to exhaust
    /// first and the condition that justifies bypassing them. Mutually
    /// exclusive with `use_when`.
    pub fn fallback(
        &mut self,
        prefer: impl IntoIterator<Item = impl Into<String>>,
        when: impl Into<String>,
    ) -> &mut Self {
        self.fallback = Some(Fallback {
            prefer: prefer.into_iter().map(Into::into).collect(),
            when: when.into(),
        });
        self
    }

    pub fn delete(
        &mut self,
        scope: impl Into<String>,
        description: impl Into<String>,
    ) -> &mut Self {
        self.permissions
            .push(PermissionSpec::delete(scope, description));
        self
    }

    pub fn exec(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self {
        self.permissions
            .push(PermissionSpec::exec(scope, description));
        self
    }

    pub fn network(
        &mut self,
        scope: impl Into<String>,
        description: impl Into<String>,
    ) -> &mut Self {
        self.permissions
            .push(PermissionSpec::network(scope, description));
        self
    }

    pub fn example(&mut self, command: impl Into<String>, summary: impl Into<String>) -> &mut Self {
        self.examples.push(CommandExample::new(command, summary));
        self
    }

    pub fn output(&mut self, output: OutputContract) -> &mut Self {
        self.output = Some(output);
        self
    }

    pub fn stdin(&mut self, mime_type: impl Into<String>, summary: impl Into<String>) -> &mut Self {
        self.stdin = Some(StdinContract {
            mime_type: mime_type.into(),
            summary: summary.into(),
        });
        self
    }

    pub fn progress_phase(
        &mut self,
        name: impl Into<String>,
        summary: impl Into<String>,
    ) -> &mut Self {
        self.progress.push(ProgressPhaseSpec {
            name: name.into(),
            summary: summary.into(),
        });
        self
    }

    pub fn example_with_args(
        &mut self,
        command: impl Into<String>,
        summary: impl Into<String>,
        args: Value,
    ) -> &mut Self {
        let mut example = CommandExample::new(command, summary);
        match args {
            Value::Object(map) => {
                example.args = map.into_iter().collect::<BTreeMap<_, _>>();
                self.examples.push(example);
            }
            _ => self.errors.push(FrameworkError::Build(format!(
                "example for `{}` must use a JSON object for args",
                self.path.join(" ")
            ))),
        }
        self
    }

    /// Installs the handler and reads the command's resource footprint from
    /// its type (RFC 0012): `Res<T>`/`Release<T>` parameters become require
    /// and release edges, `Granted<T>`/`Listed<T>` outputs become grant and
    /// enumerate edges. Plain `Fn(CommandContext)` handlers carry no
    /// resource facts and register unchanged.
    pub fn handle<M, H>(&mut self, handler: H) -> &mut Self
    where
        H: ResourceDialect<M>,
    {
        for resource_use in H::resource_uses() {
            if resource_use.released {
                self.releases.push(resource_use.resource.to_string());
            } else {
                self.requires_resources
                    .push(resource_use.resource.to_string());
            }
        }
        self.grants
            .extend(H::granted().into_iter().map(ToOwned::to_owned));
        self.enumerates
            .extend(H::enumerated().into_iter().map(ToOwned::to_owned));
        self.handler = Some(SharedCommandHandler::from_arc(
            handler.into_command_handler(),
        ));
        self
    }

    pub fn handle_typed<A, H, Fut>(&mut self, handler: H) -> &mut Self
    where
        A: FromCommandArgs + Send + Sync + 'static,
        H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<CommandOutput>> + Send,
    {
        self.handler = Some(SharedCommandHandler::new(TypedHandler::<A, H> {
            handler,
            _marker: PhantomData,
        }));
        self
    }

    fn finish(self, workspace_names: &BTreeSet<&str>) -> Result<BuiltCommand> {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(error);
        }

        let command_name = self.path.join(" ");
        let summary = required_text(self.summary, "summary", &command_name)?;
        let description = required_text(self.description, "description", &command_name)?;
        let handler = self.handler.ok_or_else(|| {
            FrameworkError::Build(format!("command `{command_name}` is missing a handler"))
        })?;

        let mut arg_names = BTreeSet::new();
        for arg in &self.args {
            if arg.summary.trim().is_empty() {
                return Err(FrameworkError::Build(format!(
                    "command `{command_name}` argument `{}` is missing summary",
                    arg.name
                )));
            }
            if !arg_names.insert(arg.name.clone()) {
                return Err(FrameworkError::Build(format!(
                    "command `{command_name}` declares duplicate argument `{}`",
                    arg.name
                )));
            }
            if let Some(workspace) = &arg.workspace
                && !workspace_names.contains(workspace.as_str())
            {
                return Err(FrameworkError::Build(format!(
                    "command `{command_name}` argument `{}` references unknown workspace `{workspace}`",
                    arg.name
                )));
            }
        }

        let mut declared_workspaces = BTreeSet::new();
        for workspace in &self.workspaces {
            if !workspace_names.contains(workspace.as_str()) {
                return Err(FrameworkError::Build(format!(
                    "command `{command_name}` uses workspace `{workspace}`, which is not declared on the server"
                )));
            }
            if !declared_workspaces.insert(workspace.as_str()) {
                return Err(FrameworkError::Build(format!(
                    "command `{command_name}` declares workspace `{workspace}` more than once"
                )));
            }
        }

        let mut spec = CommandSpec::new(self.path, summary, description);
        if let Some(output) = self.output {
            spec = spec.with_output(output);
        }
        if let Some(stdin) = self.stdin {
            spec = spec.with_stdin(stdin);
        }
        for phase in self.progress {
            spec = spec.with_progress_phase(phase);
        }
        for arg in self.args {
            spec = spec.with_arg(arg);
        }
        for permission in self.permissions {
            spec = spec.with_permission(permission);
        }
        for example in self.examples {
            spec = spec.with_example(example);
        }
        if self.idempotent {
            spec = spec.idempotent();
        }
        for workspace in self.workspaces {
            spec = spec.uses_workspace(workspace);
        }
        for capability in self.requires {
            spec = spec.requires(capability);
        }
        for capability in self.provides {
            spec = spec.provides(capability);
        }
        if let Some(text) = self.use_when {
            spec = spec.use_when(text);
        }
        for alternative in self.alternatives {
            spec = spec.alternative(alternative.command, alternative.when);
        }
        if let Some(fallback) = self.fallback {
            spec = spec.fallback(fallback.prefer, fallback.when);
        }
        spec.requires_resources = self.requires_resources;
        spec.grants = self.grants;
        spec.releases = self.releases;
        spec.enumerates = self.enumerates;

        Ok(BuiltCommand { spec, handler })
    }
}

fn required_text(value: Option<String>, field: &str, command: &str) -> Result<String> {
    let value = value.unwrap_or_default();
    if value.trim().is_empty() {
        Err(FrameworkError::Build(format!(
            "command `{command}` is missing {field}"
        )))
    } else {
        Ok(value)
    }
}

pub struct ArgBuilder {
    spec: ArgSpec,
}

impl ArgBuilder {
    fn new(name: impl Into<String>, value_type: ArgType) -> Self {
        Self {
            spec: ArgSpec {
                name: name.into(),
                value_type,
                required: true,
                summary: String::new(),
                workspace: None,
                repeated: false,
            },
        }
    }

    fn workspace(mut self, workspace: impl Into<String>) -> Self {
        self.spec.workspace = Some(workspace.into());
        self
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.spec.summary = summary.into();
        self
    }

    pub fn optional(mut self) -> Self {
        self.spec.required = false;
        self
    }

    pub fn repeated(mut self) -> Self {
        self.spec.repeated = true;
        self
    }

    fn finish(self) -> ArgSpec {
        self.spec
    }
}

pub trait IntoCommandPath {
    fn into_command_path(self) -> Vec<String>;
}

impl IntoCommandPath for &str {
    fn into_command_path(self) -> Vec<String> {
        self.split_whitespace().map(ToOwned::to_owned).collect()
    }
}

impl IntoCommandPath for String {
    fn into_command_path(self) -> Vec<String> {
        self.as_str().into_command_path()
    }
}

impl<const N: usize> IntoCommandPath for [&str; N] {
    fn into_command_path(self) -> Vec<String> {
        self.into_iter().map(ToOwned::to_owned).collect()
    }
}

pub trait FromCommandArgs: Sized {
    fn from_command_args(context: &CommandContext) -> Result<Self>;
}

impl<T> FromCommandArgs for T
where
    T: DeserializeOwned,
{
    fn from_command_args(context: &CommandContext) -> Result<Self> {
        let mut values = serde_json::Map::new();
        for (name, arg) in &context.plan.bound_args {
            values.insert(name.clone(), arg.value.clone());
        }
        serde_json::from_value(Value::Object(values)).map_err(|error| {
            FrameworkError::Build(format!("typed argument extraction failed: {error}"))
        })
    }
}

#[derive(Clone)]
struct SharedCommandHandler {
    inner: Arc<dyn CommandHandler>,
}

impl SharedCommandHandler {
    fn new(handler: impl CommandHandler) -> Self {
        Self {
            inner: Arc::new(handler),
        }
    }

    fn from_arc(inner: Arc<dyn CommandHandler>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl CommandHandler for SharedCommandHandler {
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        self.inner.call(context).await
    }
}

struct TypedHandler<A, H> {
    handler: H,
    _marker: PhantomData<A>,
}

#[async_trait]
impl<A, H, Fut> CommandHandler for TypedHandler<A, H>
where
    A: FromCommandArgs + Send + Sync + 'static,
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<CommandOutput>> + Send,
{
    async fn call(&self, context: CommandContext) -> Result<CommandOutput> {
        let args = A::from_command_args(&context)?;
        (self.handler)(context, args).await
    }
}
