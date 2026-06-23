use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::Utc;
use rmcp::{
    Peer, RoleServer, ServerHandler,
    handler::server::tool::schema_for_type,
    model::{
        CallToolRequestParams, CallToolResult, CancelTaskParams, CancelTaskResult, Content,
        CreateTaskResult, GetPromptRequestParams, GetPromptResult, GetTaskInfoParams,
        GetTaskPayloadResult, GetTaskResult, GetTaskResultParams, Implementation,
        ListPromptsResult, ListResourcesResult, ListTasksResult, ListToolsResult, Meta,
        PaginatedRequestParams, ProgressNotificationParam, RawResource, ReadResourceRequestParams,
        ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, Task, TaskStatus,
        TaskSupport, TasksCapability, Tool, ToolAnnotations, ToolExecution,
    },
    service::RequestContext,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    CommandRegistry, EffectLane, FrameworkError, HelpRequest, HelpResult, ResponseEnvelope,
    ResponseProfile, RunRequest, RunResponse, ToolLaneSpec,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliMcpServerConfig {
    pub execution_tool_name: String,
}

impl Default for CliMcpServerConfig {
    fn default() -> Self {
        Self {
            execution_tool_name: "run".to_string(),
        }
    }
}

impl CliMcpServerConfig {
    pub fn with_execution_tool_name(mut self, name: impl Into<String>) -> Self {
        self.execution_tool_name = name.into();
        self
    }
}

#[derive(Clone)]
pub struct CliMcpServer {
    registry: Arc<CommandRegistry>,
    config: CliMcpServerConfig,
    tasks: Arc<Mutex<BTreeMap<String, TaskRecord>>>,
    task_counter: Arc<AtomicU64>,
}

#[derive(Clone)]
struct TaskRecord {
    task: Task,
    payload: Option<Value>,
}

impl CliMcpServer {
    pub fn new(registry: CommandRegistry) -> Self {
        Self::with_config(registry, CliMcpServerConfig::default())
    }

    pub fn with_config(registry: CommandRegistry, config: CliMcpServerConfig) -> Self {
        Self {
            registry: Arc::new(registry),
            config,
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
            task_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn registry(&self) -> &CommandRegistry {
        &self.registry
    }

    pub fn config(&self) -> &CliMcpServerConfig {
        &self.config
    }

    async fn notify_progress(
        meta: &Meta,
        client: &Peer<RoleServer>,
        progress: f64,
        total: f64,
        message: impl Into<String>,
    ) {
        if let Some(progress_token) = meta.get_progress_token() {
            let _ = client
                .notify_progress(ProgressNotificationParam {
                    progress_token,
                    progress,
                    total: Some(total),
                    message: Some(message.into()),
                })
                .await;
        }
    }

    fn execution_lanes(&self) -> Vec<ToolLaneSpec> {
        self.registry.lane_specs(&self.config.execution_tool_name)
    }

    fn tools(&self) -> Vec<Tool> {
        let mut tools = vec![self.help_tool()];
        tools.extend(
            self.execution_lanes()
                .into_iter()
                .map(|lane| self.execution_tool(lane)),
        );
        tools
    }

    fn help_tool(&self) -> Tool {
        Tool::new(
            "help",
            "Return consistent help for the server or a CLI-shaped command.",
            schema_for_type::<HelpRequest>(),
        )
        .annotate(
            ToolAnnotations::new()
                .read_only(true)
                .destructive(false)
                .idempotent(true)
                .open_world(false),
        )
    }

    fn execution_tool(&self, lane: ToolLaneSpec) -> Tool {
        Tool::new(
            lane.tool_name.clone(),
            lane.description,
            schema_for_type::<RunRequest>(),
        )
        .with_execution(ToolExecution::new().with_task_support(TaskSupport::Optional))
        .annotate(annotations_for_lane(lane.lane, &lane.tool_name))
    }

    fn resources(&self) -> Vec<RawResource> {
        let mut resources = vec![
            RawResource::new("cli://server/overview", "Server overview"),
            RawResource::new("cli://catalog", "Command catalog"),
            RawResource::new("cli://commands", "Command catalog"),
            RawResource::new("cli://permissions", "Permission model"),
            RawResource::new("cli://lanes", "Effect-lane tools"),
        ];
        resources.extend(self.registry.command_specs().map(|spec| {
            RawResource::new(
                format!("cli://commands/{}", spec.path.join("/")),
                format!("Command {}", spec.name()),
            )
        }));
        resources
    }

    async fn execute_run_tool(
        registry: Arc<CommandRegistry>,
        config: CliMcpServerConfig,
        tool_name: String,
        meta: Meta,
        client: Peer<RoleServer>,
        request: RunRequest,
    ) -> CallToolResult {
        let profile = response_profile(&request);
        Self::notify_progress(&meta, &client, 1.0, 4.0, "Parsing command template").await;
        let plan = match registry.build_plan(&request) {
            Ok(plan) => plan,
            Err(error) => {
                return envelope_result(ResponseEnvelope::framework_error(
                    error,
                    Some(request),
                    None,
                ));
            }
        };

        Self::notify_progress(&meta, &client, 2.0, 4.0, "Invocation plan ready").await;
        let Some(lane) = registry.tool_lane(&config.execution_tool_name, &tool_name) else {
            return envelope_result(ResponseEnvelope::framework_error(
                FrameworkError::UnknownCommand(tool_name),
                Some(request),
                Some(plan),
            ));
        };

        if plan.lane != lane {
            let required_tool = registry.required_tool_name(&config.execution_tool_name, plan.lane);
            return envelope_result(ResponseEnvelope::framework_error(
                FrameworkError::WrongEffectLane {
                    current_tool: tool_name,
                    required_tool,
                },
                Some(request),
                Some(plan),
            ));
        }

        if request.dry_run {
            return envelope_result(ResponseEnvelope::success(
                RunResponse {
                    plan,
                    output: None,
                    dry_run: true,
                },
                ResponseProfile::Debug,
            ));
        }

        Self::notify_progress(&meta, &client, 3.0, 4.0, "Dispatching command handler").await;
        let result = registry
            .run_in_lane(
                request.clone(),
                tool_name,
                lane,
                &config.execution_tool_name,
            )
            .await;
        match result {
            Ok(response) => {
                Self::notify_progress(&meta, &client, 4.0, 4.0, "Command complete").await;
                success_result(
                    ResponseEnvelope::success(response, profile.clone()),
                    profile,
                )
            }
            Err(error) => envelope_result(ResponseEnvelope::framework_error(
                error,
                Some(request),
                Some(plan),
            )),
        }
    }

    fn parse_arguments<T: DeserializeOwned>(
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> std::result::Result<T, rmcp::ErrorData> {
        serde_json::from_value(Value::Object(arguments.unwrap_or_default()))
            .map_err(|error| rmcp::ErrorData::invalid_params(error.to_string(), None))
    }
}

impl ServerHandler for CliMcpServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_prompts()
            .enable_tasks_with(TasksCapability::server_default())
            .build();
        let mut implementation =
            Implementation::new(self.registry.server_name(), env!("CARGO_PKG_VERSION"));
        implementation.title = Some("MCP Twill".to_string());
        implementation.description = Some(self.registry.server_description().to_string());

        ServerInfo::new(capabilities)
            .with_server_info(implementation)
            .with_instructions(format!(
                "Use `help` to discover command templates. Start execution with `{}`; the framework returns structured retry data when another effect-lane tool is required. Command strings are typed templates, not shell programs.",
                self.config.execution_tool_name
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        let tool_name = request.name.to_string();
        if tool_name == "help" {
            let help_request = Self::parse_arguments::<HelpRequest>(request.arguments)?;
            return Ok(help_result(self.registry.help(help_request)));
        }

        if self
            .registry
            .tool_lane(&self.config.execution_tool_name, &tool_name)
            .is_none()
        {
            return Err(rmcp::ErrorData::invalid_params(
                format!("Unknown tool {tool_name}"),
                None,
            ));
        }

        let run_request = Self::parse_arguments::<RunRequest>(request.arguments)?;
        Ok(Self::execute_run_tool(
            self.registry.clone(),
            self.config.clone(),
            tool_name,
            context.meta.clone(),
            context.peer.clone(),
            run_request,
        )
        .await)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourcesResult, rmcp::ErrorData> {
        Ok(ListResourcesResult::with_all_items(
            self.resources()
                .into_iter()
                .map(|resource| resource.no_annotation())
                .collect(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ReadResourceResult, rmcp::ErrorData> {
        let text = if request.uri == "cli://lanes" {
            lanes_text(&self.execution_lanes())
        } else {
            self.registry.resource_text(&request.uri).ok_or_else(|| {
                rmcp::ErrorData::invalid_params(format!("Unknown resource {}", request.uri), None)
            })?
        };
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, request.uri).with_mime_type("text/markdown"),
        ]))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListPromptsResult, rmcp::ErrorData> {
        Ok(ListPromptsResult::with_all_items(vec![
            rmcp::model::Prompt::new("getting_started", Some("How to use MCP Twill"), None),
        ]))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<GetPromptResult, rmcp::ErrorData> {
        if request.name != "getting_started" {
            return Err(rmcp::ErrorData::invalid_params(
                format!("Unknown prompt {}", request.name),
                None,
            ));
        }
        Ok(GetPromptResult::new(vec![
            rmcp::model::PromptMessage::new_text(
                rmcp::model::PromptMessageRole::User,
                format!(
                    "First call `help` with no command. Then call `help` for a command. Start execution with `{}` and use escalated lane tools only when a structured response asks for one. Use typed `$args.*` values; do not use shell syntax in the command string.",
                    self.config.execution_tool_name
                ),
            ),
        ]))
    }

    async fn enqueue_task(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CreateTaskResult, rmcp::ErrorData> {
        let tool_name = request.name.to_string();
        if self
            .registry
            .tool_lane(&self.config.execution_tool_name, &tool_name)
            .is_none()
        {
            return Err(rmcp::ErrorData::invalid_params(
                format!("Only execution tools support task-augmented execution: {tool_name}"),
                None,
            ));
        }
        let run_request = Self::parse_arguments::<RunRequest>(request.arguments)?;

        let task_id = format!(
            "{}-{}",
            tool_name.replace('-', "_"),
            self.task_counter.fetch_add(1, Ordering::SeqCst)
        );
        let now = Utc::now().to_rfc3339();
        let task = Task::new(
            task_id.clone(),
            TaskStatus::Working,
            now.clone(),
            now.clone(),
        )
        .with_status_message("Queued run command")
        .with_poll_interval(100);

        self.tasks.lock().await.insert(
            task_id.clone(),
            TaskRecord {
                task: task.clone(),
                payload: None,
            },
        );

        let tasks = self.tasks.clone();
        let registry = self.registry.clone();
        let config = self.config.clone();
        let meta = context.meta.clone();
        let client = context.peer.clone();
        tokio::spawn(async move {
            let result = CliMcpServer::execute_run_tool(
                registry,
                config,
                tool_name,
                meta,
                client,
                run_request,
            )
            .await;
            let is_error = result.is_error.unwrap_or(false);
            let mut tasks = tasks.lock().await;
            if let Some(record) = tasks.get_mut(&task_id) {
                record.task.status = TaskStatus::Completed;
                record.task.status_message = Some(if is_error {
                    "Run command completed with a framework error".to_string()
                } else {
                    "Run command completed".to_string()
                });
                record.task.last_updated_at = Utc::now().to_rfc3339();
                record.payload = Some(serde_json::to_value(result).unwrap_or_else(|error| {
                    json!({
                        "content": [{ "type": "text", "text": error.to_string() }],
                        "isError": true
                    })
                }));
            }
        });

        Ok(CreateTaskResult::new(task))
    }

    async fn list_tasks(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListTasksResult, rmcp::ErrorData> {
        let tasks = self.tasks.lock().await;
        Ok(ListTasksResult::new(
            tasks.values().map(|record| record.task.clone()).collect(),
        ))
    }

    async fn get_task_info(
        &self,
        request: GetTaskInfoParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<GetTaskResult, rmcp::ErrorData> {
        let tasks = self.tasks.lock().await;
        let record = tasks.get(&request.task_id).ok_or_else(|| {
            rmcp::ErrorData::invalid_params(format!("Unknown task {}", request.task_id), None)
        })?;
        Ok(GetTaskResult {
            meta: None,
            task: record.task.clone(),
        })
    }

    async fn get_task_result(
        &self,
        request: GetTaskResultParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<GetTaskPayloadResult, rmcp::ErrorData> {
        let tasks = self.tasks.lock().await;
        let record = tasks.get(&request.task_id).ok_or_else(|| {
            rmcp::ErrorData::invalid_params(format!("Unknown task {}", request.task_id), None)
        })?;
        let Some(payload) = &record.payload else {
            return Err(rmcp::ErrorData::invalid_params(
                format!("Task {} is not complete", request.task_id),
                None,
            ));
        };
        Ok(GetTaskPayloadResult::new(payload.clone()))
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CancelTaskResult, rmcp::ErrorData> {
        let mut tasks = self.tasks.lock().await;
        let record = tasks.get_mut(&request.task_id).ok_or_else(|| {
            rmcp::ErrorData::invalid_params(format!("Unknown task {}", request.task_id), None)
        })?;
        if record.task.status == TaskStatus::Working {
            record.task.status = TaskStatus::Cancelled;
            record.task.status_message = Some("Task cancelled".to_string());
            record.task.last_updated_at = Utc::now().to_rfc3339();
        }
        Ok(CancelTaskResult {
            meta: None,
            task: record.task.clone(),
        })
    }
}

fn response_profile(request: &RunRequest) -> ResponseProfile {
    if request.dry_run {
        return ResponseProfile::Debug;
    }
    let Some(output) = &request.output else {
        return ResponseProfile::CompactStructured;
    };
    if let Some(profile) = &output.profile {
        return profile.clone();
    }
    match output.format {
        crate::OutputFormat::Text => ResponseProfile::Text,
        crate::OutputFormat::Structured => ResponseProfile::CompactStructured,
    }
}

fn envelope_result(envelope: ResponseEnvelope) -> CallToolResult {
    let is_error = envelope.error.is_some();
    let value = serde_json::to_value(&envelope).unwrap_or_else(|error| {
        json!({
            "status": "failed",
            "error": {
                "message": error.to_string()
            }
        })
    });
    if is_error {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}

fn success_result(envelope: ResponseEnvelope, profile: ResponseProfile) -> CallToolResult {
    if matches!(profile, ResponseProfile::Text) {
        let text = envelope
            .output
            .as_ref()
            .and_then(|output| {
                output.text.clone().or_else(|| {
                    output
                        .structured
                        .as_ref()
                        .and_then(|value| serde_json::to_string_pretty(value).ok())
                })
            })
            .unwrap_or_else(|| envelope.display_text());
        return CallToolResult::success(vec![Content::text(text)]);
    }
    envelope_result(envelope)
}

fn help_result(result: HelpResult) -> CallToolResult {
    CallToolResult::structured(serde_json::to_value(&result).unwrap_or_else(
        |error| json!({ "title": "Help error", "text": error.to_string(), "structured": {} }),
    ))
}

fn annotations_for_lane(lane: EffectLane, tool_name: &str) -> ToolAnnotations {
    let title = format!("{} execution", tool_name);
    match lane {
        EffectLane::Primary => ToolAnnotations::with_title(title)
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
        EffectLane::Write => ToolAnnotations::with_title(title)
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(false),
        EffectLane::Delete => ToolAnnotations::with_title(title)
            .read_only(false)
            .destructive(true)
            .idempotent(false)
            .open_world(false),
        EffectLane::Exec => ToolAnnotations::with_title(title)
            .read_only(false)
            .destructive(true)
            .idempotent(false)
            .open_world(true),
        EffectLane::Network => ToolAnnotations::with_title(title)
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(true),
    }
}

fn lanes_text(lanes: &[ToolLaneSpec]) -> String {
    let mut lines = vec![
        "# Effect-Lane Tools".to_string(),
        "Start with the primary execution tool. Follow structured retry data when another lane is required.".to_string(),
        String::new(),
    ];
    for lane in lanes {
        lines.push(format!("## `{}`", lane.tool_name));
        lines.push(lane.description.clone());
        lines.push(String::new());
    }
    lines.join("\n")
}

trait NoAnnotation {
    fn no_annotation(self) -> rmcp::model::Resource;
}

impl NoAnnotation for RawResource {
    fn no_annotation(self) -> rmcp::model::Resource {
        rmcp::model::Annotated {
            raw: self,
            annotations: None,
        }
    }
}
