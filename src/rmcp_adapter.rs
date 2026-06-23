use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::Utc;
use rmcp::{
    Json, Peer, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolRequestParams, CancelTaskParams, CancelTaskResult, CreateTaskResult,
        GetPromptRequestParams, GetPromptResult, GetTaskInfoParams, GetTaskPayloadResult,
        GetTaskResult, GetTaskResultParams, Implementation, ListPromptsResult, ListResourcesResult,
        ListTasksResult, Meta, PaginatedRequestParams, ProgressNotificationParam, RawResource,
        ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
        ServerInfo, Task, TaskStatus, TasksCapability,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{CommandRegistry, HelpRequest, HelpResult, RunRequest, RunResponse};

#[derive(Clone)]
pub struct CliMcpServer {
    registry: Arc<CommandRegistry>,
    tool_router: ToolRouter<Self>,
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
        Self {
            registry: Arc::new(registry),
            tool_router: Self::tool_router(),
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
            task_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn registry(&self) -> &CommandRegistry {
        &self.registry
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

    async fn run_inner(
        registry: Arc<CommandRegistry>,
        meta: Meta,
        client: Peer<RoleServer>,
        request: RunRequest,
    ) -> std::result::Result<RunResponse, String> {
        Self::notify_progress(&meta, &client, 1.0, 4.0, "Parsing command template").await;
        let plan = registry
            .build_plan(&request)
            .map_err(|error| error.to_string())?;
        Self::notify_progress(&meta, &client, 2.0, 4.0, "Invocation plan ready").await;
        if request.dry_run {
            return Ok(RunResponse {
                plan,
                output: None,
                dry_run: true,
            });
        }

        Self::notify_progress(&meta, &client, 3.0, 4.0, "Dispatching command handler").await;
        let response = registry
            .run(request)
            .await
            .map_err(|error| error.to_string())?;
        Self::notify_progress(&meta, &client, 4.0, 4.0, "Command complete").await;
        Ok(response)
    }

    fn resources(&self) -> Vec<RawResource> {
        let mut resources = vec![
            RawResource::new("cli://server/overview", "Server overview"),
            RawResource::new("cli://commands", "Command catalog"),
            RawResource::new("cli://permissions", "Permission model"),
        ];
        resources.extend(self.registry.command_specs().map(|spec| {
            RawResource::new(
                format!("cli://commands/{}", spec.path.join("/")),
                format!("Command {}", spec.name()),
            )
        }));
        resources
    }
}

#[tool_router(router = tool_router)]
impl CliMcpServer {
    #[tool(description = "Return consistent help for the server or a CLI-shaped command.")]
    pub async fn help(
        &self,
        Parameters(request): Parameters<HelpRequest>,
    ) -> std::result::Result<Json<HelpResult>, String> {
        Ok(Json(self.registry.help(request)))
    }

    #[tool(
        description = "Run a CLI-shaped command template over typed arguments.",
        execution(task_support = "optional")
    )]
    pub async fn run(
        &self,
        meta: Meta,
        client: Peer<RoleServer>,
        Parameters(request): Parameters<RunRequest>,
    ) -> std::result::Result<Json<RunResponse>, String> {
        Self::run_inner(self.registry.clone(), meta, client, request)
            .await
            .map(Json)
    }
}

#[tool_handler(router = self.tool_router)]
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
            .with_instructions(
                "Use `help` to discover command templates and `run` to execute them. Command strings are typed templates, not shell programs.",
            )
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
        let Some(text) = self.registry.resource_text(&request.uri) else {
            return Err(rmcp::ErrorData::invalid_params(
                format!("Unknown resource {}", request.uri),
                None,
            ));
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
            rmcp::model::Prompt::new(
                "getting_started",
                Some("How to use MCP Twill"),
                None,
            ),
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
                "First call `help` with no command. Then call `help` for a command. Use `run` with a command template and typed `$args.*` values; do not use shell syntax in the command string.",
            ),
        ]))
    }

    async fn enqueue_task(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CreateTaskResult, rmcp::ErrorData> {
        if request.name != "run" {
            return Err(rmcp::ErrorData::invalid_params(
                "Only run supports task-augmented execution",
                None,
            ));
        }
        let args = request.arguments.unwrap_or_default();
        let run_request: RunRequest = serde_json::from_value(Value::Object(args))
            .map_err(|error| rmcp::ErrorData::invalid_params(error.to_string(), None))?;

        let task_id = format!("run-{}", self.task_counter.fetch_add(1, Ordering::SeqCst));
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
        let meta = context.meta.clone();
        let client = context.peer.clone();
        tokio::spawn(async move {
            let result = CliMcpServer::run_inner(registry, meta, client, run_request).await;
            let mut tasks = tasks.lock().await;
            if let Some(record) = tasks.get_mut(&task_id) {
                record.task.last_updated_at = Utc::now().to_rfc3339();
                match result {
                    Ok(response) => {
                        record.task.status = TaskStatus::Completed;
                        record.task.status_message = Some("Run command completed".to_string());
                        record.payload = Some(json!({
                            "content": [{
                                "type": "text",
                                "text": serde_json::to_string(&response).unwrap_or_default()
                            }],
                            "structuredContent": response,
                            "isError": false
                        }));
                    }
                    Err(error) => {
                        record.task.status = TaskStatus::Failed;
                        record.task.status_message = Some(error.clone());
                        record.payload = Some(json!({
                            "content": [{ "type": "text", "text": error }],
                            "isError": true
                        }));
                    }
                }
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
