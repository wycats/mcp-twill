use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    CommandContext, CommandOutput, CommandSpec, FrameworkError, HelpRequest, HelpResult, HelpTopic,
    InvocationPlan, InvocationToken, PermissionPolicy, Result, RunRequest, RunResponse,
    TemplateToken, WorkspaceDecl, structured_error, value_matches_type,
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

    pub fn workspaces(&self) -> impl Iterator<Item = &WorkspaceDecl> {
        self.workspaces.values()
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn server_description(&self) -> &str {
        &self.server_description
    }

    pub fn build_plan(&self, request: &RunRequest) -> Result<InvocationPlan> {
        let template = CommandTemplate::parse(&request.command)?;
        let registered = self
            .match_command(&template)
            .ok_or_else(|| FrameworkError::UnknownCommand(request.command.clone()))?;

        let referenced: BTreeSet<_> = template
            .placeholders()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        for arg_name in &referenced {
            if registered.spec.arg(arg_name).is_none() {
                return Err(FrameworkError::UnknownArgument(arg_name.clone()));
            }
            if !request.args.contains_key(arg_name) {
                return Err(FrameworkError::MissingArgument(arg_name.clone()));
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

        let mut bound_args = BTreeMap::new();
        for spec in &registered.spec.args {
            let Some(value) = request.args.get(&spec.name) else {
                if spec.required {
                    return Err(FrameworkError::MissingArgument(spec.name.clone()));
                }
                continue;
            };
            value_matches_type(&spec.name, value, &spec.value_type)?;
            if let Some(workspace_name) = &spec.workspace {
                let workspace = self.workspaces.get(workspace_name).ok_or_else(|| {
                    FrameworkError::WorkspaceMismatch {
                        argument: spec.name.clone(),
                        workspace: workspace_name.clone(),
                    }
                })?;
                let value = value.as_str().ok_or_else(|| {
                    FrameworkError::InvalidArgumentType(
                        spec.name.clone(),
                        spec.value_type.expected_name(),
                    )
                })?;
                if !workspace.contains_path_value(value) {
                    return Err(FrameworkError::WorkspaceMismatch {
                        argument: spec.name.clone(),
                        workspace: workspace_name.clone(),
                    });
                }
            }
            bound_args.insert(
                spec.name.clone(),
                crate::BoundArg {
                    name: spec.name.clone(),
                    value_type: spec.value_type.clone(),
                    value: value.clone(),
                    workspace: spec.workspace.clone(),
                },
            );
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

        Ok(InvocationPlan {
            command_path: registered.spec.path.clone(),
            raw_command: request.command.clone(),
            tokens,
            bound_args,
            permissions: registered.spec.permissions.clone(),
            workspaces: self.workspaces.values().cloned().collect(),
            output: request.output.clone().unwrap_or_default(),
        })
    }

    pub async fn run(&self, request: RunRequest) -> Result<RunResponse> {
        let plan = self.build_plan(&request)?;
        if request.dry_run {
            return Ok(RunResponse {
                plan,
                output: None,
                dry_run: true,
            });
        }

        self.policy.check(&plan.permissions)?;
        let registered = self
            .commands
            .get(&plan.command_path)
            .ok_or_else(|| FrameworkError::UnknownCommand(plan.command_path.join(" ")))?;

        let output = registered
            .handler
            .call(CommandContext {
                plan: plan.clone(),
                stdin: request.stdin,
            })
            .await?
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

    fn server_help(&self) -> HelpResult {
        let mut lines = vec![
            format!("# {}", self.server_name),
            self.server_description.clone(),
            String::new(),
            "Tools: `help`, `run`.".to_string(),
            "Command strings are typed templates, not shell programs.".to_string(),
            String::new(),
            "Commands:".to_string(),
        ];
        for spec in self.command_specs() {
            lines.push(format!("- `{}`: {}", spec.name(), spec.summary));
        }

        HelpResult {
            title: self.server_name.clone(),
            text: lines.join("\n"),
            structured: json!({
                "server": self.server_name,
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
            let error = FrameworkError::UnknownCommand(command.to_string());
            return HelpResult {
                title: "Unknown command".to_string(),
                text: error.to_string(),
                structured: structured_error(&error),
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
        format!(
            "# `{}`\n\n{}\n\n{}\n\n{}",
            spec.name(),
            spec.description,
            self.arguments_text(spec),
            self.examples_text(spec)
        )
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
            lines.push(format!(
                "- `$args.{}`: {:?}, {}, {}{}",
                arg.name, arg.value_type, required, arg.summary, workspace
            ));
        }
        lines.join("\n")
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
