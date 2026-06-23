use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use async_trait::async_trait;
use schemars::schema_for;
use serde_json::{Value, json};

use crate::{
    CatalogIdentity, CommandCatalog, CommandContext, CommandOutput, CommandSpec, EffectLane,
    FrameworkError, HelpRequest, HelpResult, HelpTopic, InvocationPlan, InvocationToken,
    OperationSpec, PermissionPolicy, Result, RunRequest, RunResponse, ServerSpec, TemplateToken,
    ToolLaneSpec, WorkspaceDecl, group_namespaces, stable_hash_value, structured_error,
    value_matches_type,
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
            identity,
        }
    }

    pub fn catalog_identity(&self) -> CatalogIdentity {
        let operations = self.operation_specs();
        self.catalog_identity_for(&operations)
    }

    fn catalog_identity_for(&self, operations: &[OperationSpec]) -> CatalogIdentity {
        let catalog_value = json!({
            "server": ServerSpec::new(&self.server_name, &self.server_description),
            "namespaces": group_namespaces(operations),
            "operations": operations,
            "workspaces": self.workspaces.values().collect::<Vec<_>>(),
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
                    dry_run: true,
                };
                self.build_plan(&request)?;
            }
        }
        Ok(())
    }

    pub fn build_plan(&self, request: &RunRequest) -> Result<InvocationPlan> {
        let template = CommandTemplate::parse(&request.command)?;
        let registered = self
            .match_command(&template)
            .ok_or_else(|| FrameworkError::UnknownCommand(request.command.clone()))?;
        let operation = OperationSpec::from_command_spec(&registered.spec);
        let identity = self.catalog_identity();

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
            operation_id: operation.id.clone(),
            command_path: registered.spec.path.clone(),
            raw_command: request.command.clone(),
            catalog_hash: identity.catalog_hash,
            effect: operation.effect.clone(),
            lane: operation.lane(),
            tokens,
            bound_args,
            permissions: registered.spec.permissions.clone(),
            workspaces: self.workspaces.values().cloned().collect(),
            output: request.output.clone().unwrap_or_default(),
        })
    }

    pub async fn run(&self, request: RunRequest) -> Result<RunResponse> {
        self.run_with_lane(request, None, None, None).await
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
        )
        .await
    }

    async fn run_with_lane(
        &self,
        request: RunRequest,
        current_tool: Option<String>,
        current_lane: Option<EffectLane>,
        primary_tool_name: Option<String>,
    ) -> Result<RunResponse> {
        let plan = self.build_plan(&request)?;
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

    fn server_help(&self) -> HelpResult {
        let mut lines = vec![
            format!("# {}", self.server_name),
            self.server_description.clone(),
            String::new(),
            "Start with the primary execution tool. Use lane tools only when the framework returns structured retry data.".to_string(),
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
