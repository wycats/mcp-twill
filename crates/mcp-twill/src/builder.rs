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
    ArgSpec, ArgType, CommandContext, CommandExample, CommandGuidance, CommandHandler,
    CommandOutput, CommandRegistry, CommandSpec, FrameworkError, OutputContract, PermissionSpec,
    ProgressPhaseSpec, Result, StdinContract, WorkspaceDecl,
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
    workspaces: Vec<WorkspaceDecl>,
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
            workspaces: Vec::new(),
            guidance: Vec::new(),
            commands: Vec::new(),
            command_paths: BTreeSet::new(),
            errors: Vec::new(),
        }
    }

    pub fn workspace(&mut self, workspace: WorkspaceDecl) -> &mut Self {
        self.workspaces.push(workspace);
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
        if let Some(error) = self.errors.into_iter().next() {
            return Err(error);
        }

        let mut registry = CommandRegistry::new(self.name, self.description);
        for workspace in self.workspaces.drain(..) {
            registry = registry.declare_workspace(workspace);
        }
        for guidance in self.guidance.drain(..) {
            registry = registry.declare_guidance(guidance);
        }
        for command in self.commands {
            registry = registry.register(command.spec, command.handler);
        }
        registry.validate_examples()?;
        registry.validate_guidance()?;
        Ok(registry)
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

    pub fn handle<H>(&mut self, handler: H) -> &mut Self
    where
        H: CommandHandler,
    {
        self.handler = Some(SharedCommandHandler::new(handler));
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
            if let Some(workspace) = &arg.workspace {
                if !workspace_names.contains(workspace.as_str()) {
                    return Err(FrameworkError::Build(format!(
                        "command `{command_name}` argument `{}` references unknown workspace `{workspace}`",
                        arg.name
                    )));
                }
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
