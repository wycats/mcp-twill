use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::Utc;
use mcp_workspace_resolver::{
    CodexSandboxObservation, McpRootsObservation, ResolvedWorkspaceSet, resolve_workspaces,
};
use rand::{RngCore, rngs::OsRng};
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
    ApprovalInput, CommandRegistry, DefaultPermissionAuthorizer, EffectLane, EventSink,
    FrameworkError, FrameworkEvent, HelpRequest, HelpResult, InvocationPlan, NoopEventSink,
    PermissionAuthorizer, PermissionDecision, PlanFacts, ReplayRecord, ResponseEnvelope,
    ResponseProfile, RunMode, RunRequest, RunResponse, RuntimeIdentity, ToolLaneSpec,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliMcpServerConfig {
    pub execution_tool_name: String,
    pub replay_ttl_ms: i64,
}

impl Default for CliMcpServerConfig {
    fn default() -> Self {
        Self {
            execution_tool_name: "run".to_string(),
            replay_ttl_ms: 10 * 60 * 1000,
        }
    }
}

impl CliMcpServerConfig {
    pub fn with_execution_tool_name(mut self, name: impl Into<String>) -> Self {
        self.execution_tool_name = name.into();
        self
    }

    pub fn with_replay_ttl_seconds(mut self, seconds: i64) -> Self {
        self.replay_ttl_ms = seconds.saturating_mul(1000);
        self
    }
}

#[derive(Clone)]
pub struct CliMcpServer {
    registry: Arc<CommandRegistry>,
    config: CliMcpServerConfig,
    tasks: Arc<Mutex<BTreeMap<String, TaskRecord>>>,
    task_counter: Arc<AtomicU64>,
    replay: Arc<Mutex<BTreeMap<String, ReplayRecord>>>,
    authorizer: Arc<dyn PermissionAuthorizer>,
    events: Arc<dyn EventSink>,
}

#[derive(Clone)]
struct TaskRecord {
    task: Task,
    payload: Option<Value>,
}

/// How one run-tool call ended: the envelope to return, the plan facts (when
/// planning succeeded) for event enrichment, and whether the envelope
/// renders through the output profile.
struct RunOutcome {
    envelope: ResponseEnvelope,
    plan: Option<PlanFacts>,
    rendered_output: bool,
}

impl RunOutcome {
    fn envelope(envelope: ResponseEnvelope, plan: Option<PlanFacts>) -> Self {
        Self {
            envelope,
            plan,
            rendered_output: false,
        }
    }

    fn output(envelope: ResponseEnvelope, plan: Option<PlanFacts>) -> Self {
        Self {
            envelope,
            plan,
            rendered_output: true,
        }
    }
}

impl CliMcpServer {
    pub fn new(registry: CommandRegistry) -> crate::Result<Self> {
        Self::with_config(registry, CliMcpServerConfig::default())
    }

    pub fn with_config(
        registry: CommandRegistry,
        config: CliMcpServerConfig,
    ) -> crate::Result<Self> {
        registry.validate_effects()?;
        registry.validate_guidance()?;
        Ok(Self {
            registry: Arc::new(registry),
            config,
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
            task_counter: Arc::new(AtomicU64::new(1)),
            replay: Arc::new(Mutex::new(BTreeMap::new())),
            authorizer: Arc::new(DefaultPermissionAuthorizer),
            events: Arc::new(NoopEventSink),
        })
    }

    /// Replaces the event sink. The default sink discards events.
    pub fn with_event_sink(mut self, events: Arc<dyn EventSink>) -> Self {
        self.events = events;
        self
    }

    pub fn registry(&self) -> &CommandRegistry {
        &self.registry
    }

    pub fn config(&self) -> &CliMcpServerConfig {
        &self.config
    }

    /// The running server's identity: name, crate version, and catalog and
    /// schema hashes. Process facts (pid, start time, executable hash) are
    /// `None` here; a runtime host fills those in.
    pub fn runtime_identity(&self) -> RuntimeIdentity {
        self.registry
            .runtime_identity()
            .with_server_version(env!("CARGO_PKG_VERSION"))
    }

    /// URIs of every resource the server advertises through MCP list_resources.
    pub fn resource_uris(&self) -> Vec<String> {
        self.resources()
            .into_iter()
            .map(|resource| resource.uri)
            .collect()
    }

    /// Every tool the server advertises through MCP list_tools.
    pub fn generated_tools(&self) -> Vec<Tool> {
        self.tools()
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
        self,
        tool_name: String,
        meta: Meta,
        client: Peer<RoleServer>,
        request: RunRequest,
    ) -> CallToolResult {
        let profile = response_profile(&request);
        let outcome = self.run_tool_flow(tool_name, meta, client, request).await;
        if self.events.enabled() {
            self.events.record(FrameworkEvent::from_envelope(
                &outcome.envelope,
                outcome.plan.as_ref(),
            ));
        }
        if outcome.rendered_output {
            success_result(outcome.envelope, profile)
        } else {
            envelope_result(outcome.envelope)
        }
    }

    async fn run_tool_flow(
        &self,
        tool_name: String,
        meta: Meta,
        client: Peer<RoleServer>,
        request: RunRequest,
    ) -> RunOutcome {
        let registry = &self.registry;
        let config = &self.config;
        let replay = &self.replay;
        let authorizer = &self.authorizer;
        let profile = response_profile(&request);
        let mode = request.effective_mode();
        let resolved = Self::resolve_workspaces_for_call(registry, &meta, &client).await;
        Self::notify_progress(&meta, &client, 1.0, 4.0, "Parsing command template").await;
        let plan = match registry.build_plan_with_workspaces(&request, &resolved) {
            Ok(plan) => plan,
            Err(error) => {
                return RunOutcome::envelope(
                    ResponseEnvelope::framework_error(error, Some(request), None),
                    None,
                );
            }
        };
        let plan_for_event = PlanFacts::from(&plan);

        Self::notify_progress(&meta, &client, 2.0, 4.0, "Invocation plan ready").await;
        let Some(lane) = registry.tool_lane(&config.execution_tool_name, &tool_name) else {
            return RunOutcome::envelope(
                ResponseEnvelope::framework_error(
                    FrameworkError::UnknownCommand {
                        command: tool_name,
                        nearest: Vec::new(),
                    },
                    Some(request),
                    Some(plan),
                ),
                Some(plan_for_event),
            );
        };

        if plan.lane != lane {
            let required_tool = registry.required_tool_name(&config.execution_tool_name, plan.lane);
            return RunOutcome::envelope(
                ResponseEnvelope::framework_error(
                    FrameworkError::WrongEffectLane {
                        current_tool: tool_name,
                        required_tool,
                    },
                    Some(request),
                    Some(plan),
                ),
                Some(plan_for_event),
            );
        }

        if matches!(mode, RunMode::Preview) {
            let requires_confirmation = match authorizer.decide(&plan) {
                PermissionDecision::Allow => false,
                PermissionDecision::RequireConfirmation => true,
                PermissionDecision::Deny { reason } => {
                    return RunOutcome::envelope(
                        ResponseEnvelope::framework_error(
                            FrameworkError::PermissionDenied {
                                effect: effect_label(&plan.effect),
                                scope: reason,
                            },
                            Some(request),
                            Some(plan),
                        ),
                        Some(plan_for_event),
                    );
                }
            };
            Self::notify_progress(&meta, &client, 4.0, 4.0, "Preview ready").await;
            return RunOutcome::envelope(
                ResponseEnvelope::preview(plan, requires_confirmation),
                Some(plan_for_event),
            );
        }

        if matches!(mode, RunMode::DryRun) {
            return RunOutcome::envelope(
                ResponseEnvelope::success(
                    RunResponse {
                        plan,
                        output: None,
                        dry_run: true,
                    },
                    ResponseProfile::Debug,
                ),
                Some(plan_for_event),
            );
        }

        match authorizer.decide(&plan) {
            PermissionDecision::Allow => {}
            PermissionDecision::RequireConfirmation => {
                if let Some(approval) = &request.approval {
                    if let Err(message) =
                        validate_replay(replay, approval, &plan, Utc::now().timestamp_millis())
                            .await
                    {
                        return RunOutcome::envelope(
                            ResponseEnvelope::framework_error(
                                FrameworkError::ApprovalInvalid(message),
                                Some(request),
                                Some(plan),
                            ),
                            Some(plan_for_event),
                        );
                    }
                } else {
                    let record =
                        issue_replay_record(config, replay, &plan, Utc::now().timestamp_millis())
                            .await;
                    Self::notify_progress(&meta, &client, 4.0, 4.0, "Confirmation required").await;
                    return RunOutcome::envelope(
                        ResponseEnvelope::permission_required(plan, record, request, tool_name),
                        Some(plan_for_event),
                    );
                }
            }
            PermissionDecision::Deny { reason } => {
                return RunOutcome::envelope(
                    ResponseEnvelope::framework_error(
                        FrameworkError::PermissionDenied {
                            effect: effect_label(&plan.effect),
                            scope: reason,
                        },
                        Some(request),
                        Some(plan),
                    ),
                    Some(plan_for_event),
                );
            }
        }

        Self::notify_progress(&meta, &client, 3.0, 4.0, "Dispatching command handler").await;
        let result = registry
            .run_in_lane_with_workspaces(
                request.clone(),
                tool_name,
                lane,
                &config.execution_tool_name,
                &resolved,
            )
            .await;
        match result {
            Ok(response) => {
                Self::notify_progress(&meta, &client, 4.0, 4.0, "Command complete").await;
                RunOutcome::output(
                    ResponseEnvelope::success(response, profile),
                    Some(plan_for_event),
                )
            }
            Err(error) => RunOutcome::envelope(
                ResponseEnvelope::framework_error(error, Some(request), Some(plan)),
                Some(plan_for_event),
            ),
        }
    }

    /// Gathers per-call workspace observations and resolves them.
    ///
    /// MCP roots are requested only when the client declared the roots
    /// capability; a failed `roots/list` call degrades to an absent
    /// observation rather than failing the tool call. Codex sandbox metadata
    /// is parsed from `codex/sandbox-state-meta` request meta when present.
    /// Declared workspace roots always participate.
    async fn resolve_workspaces_for_call(
        registry: &CommandRegistry,
        meta: &Meta,
        client: &Peer<RoleServer>,
    ) -> ResolvedWorkspaceSet {
        let mut observations = registry.declared_observations();

        let client_declares_roots = client
            .peer_info()
            .is_some_and(|info| info.capabilities.roots.is_some());
        if client_declares_roots {
            // The client's roots are the access boundary. If listing them
            // fails, treat the observation as present-and-empty: requirements
            // stay unresolved rather than widening to declared roots.
            let roots = match client.list_roots().await {
                Ok(result) => McpRootsObservation::from(result),
                Err(_) => McpRootsObservation::new(Vec::new()),
            };
            observations = observations.with_mcp_roots(roots);
        }

        if let Some(codex) = codex_sandbox_observation(meta) {
            observations = observations.with_codex_sandbox(codex);
        }

        resolve_workspaces(&registry.workspace_requirements(), &observations)
    }

    fn parse_arguments<T: DeserializeOwned>(
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> std::result::Result<T, rmcp::ErrorData> {
        serde_json::from_value(Value::Object(arguments.unwrap_or_default()))
            .map_err(|error| rmcp::ErrorData::invalid_params(error.to_string(), None))
    }

    /// Parses run-tool arguments, recording an invalid-input event when the
    /// request never deserializes. Parse failures are part of the call
    /// lifecycle the event stream captures.
    fn parse_run_request(
        &self,
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> std::result::Result<RunRequest, rmcp::ErrorData> {
        Self::parse_arguments::<RunRequest>(arguments).inspect_err(|error| {
            if self.events.enabled() {
                self.events
                    .record(FrameworkEvent::parse_failure(error.message.clone()));
            }
        })
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

        let run_request = self.parse_run_request(request.arguments)?;
        Ok(self
            .clone()
            .execute_run_tool(
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
        let mut text = format!(
            "First call `help` with no command. Then call `help` for a command. Start execution with `{}` and use escalated lane tools only when a structured response asks for one. Use typed `$args.*` values; do not use shell syntax in the command string.",
            self.config.execution_tool_name
        );
        let guidance = self.registry.guidance();
        if !guidance.is_empty() {
            text.push_str("\n\nGuidance:");
            for entry in guidance {
                match entry.kind {
                    crate::GuidanceKind::RunCommand => {
                        text.push_str(&format!("\n- `{}` ({})", entry.text, entry.surface));
                    }
                    crate::GuidanceKind::HumanAction => {
                        text.push_str(&format!(
                            "\n- (human action) {} ({})",
                            entry.text, entry.surface
                        ));
                    }
                    crate::GuidanceKind::ExternalShell => {
                        text.push_str(&format!(
                            "\n- (external shell, not a framework command) `{}` ({})",
                            entry.text, entry.surface
                        ));
                    }
                }
            }
        }
        Ok(GetPromptResult::new(vec![
            rmcp::model::PromptMessage::new_text(rmcp::model::PromptMessageRole::User, text),
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
        let run_request = self.parse_run_request(request.arguments)?;

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
        let server = self.clone();
        let meta = context.meta.clone();
        let client = context.peer.clone();
        tokio::spawn(async move {
            let result = server
                .execute_run_tool(tool_name, meta, client, run_request)
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

async fn issue_replay_record(
    config: &CliMcpServerConfig,
    replay: &Arc<Mutex<BTreeMap<String, ReplayRecord>>>,
    plan: &InvocationPlan,
    issued_at_unix_ms: i64,
) -> ReplayRecord {
    loop {
        let token = generate_replay_token();
        let record = ReplayRecord {
            token: token.clone(),
            invocation_fingerprint: plan.invocation_fingerprint.clone(),
            operation_id: plan.operation_id.clone(),
            command_path: plan.command_path.clone(),
            lane: plan.lane,
            issued_at_unix_ms,
            expires_at_unix_ms: issued_at_unix_ms.saturating_add(config.replay_ttl_ms),
            single_use: true,
        };
        let mut replay = replay.lock().await;
        if !replay.contains_key(&token) {
            replay.insert(token, record.clone());
            return record;
        }
    }
}

fn generate_replay_token() -> String {
    let mut bytes = [0_u8; 32];
    let mut rng = OsRng;
    rng.fill_bytes(&mut bytes);
    let encoded = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("replay-{encoded}")
}

async fn validate_replay(
    replay: &Arc<Mutex<BTreeMap<String, ReplayRecord>>>,
    approval: &ApprovalInput,
    plan: &InvocationPlan,
    now_unix_ms: i64,
) -> std::result::Result<(), String> {
    if !approval.confirm {
        return Err("approval confirmation was not set".to_string());
    }

    {
        let replay = replay.lock().await;
        let Some(record) = replay.get(&approval.token) else {
            return Err("approval token is unknown or already used".to_string());
        };

        if record.expires_at_unix_ms <= now_unix_ms {
            return Err("approval token expired".to_string());
        }
        if record.invocation_fingerprint != plan.invocation_fingerprint {
            return Err("approved invocation does not match current invocation".to_string());
        }
        if record.operation_id != plan.operation_id || record.lane != plan.lane {
            return Err("approved operation does not match current operation".to_string());
        }
        if !record.single_use {
            return Ok(());
        }
    }

    let Some(record) = replay.lock().await.remove(&approval.token) else {
        return Err("approval token is unknown or already used".to_string());
    };

    if record.expires_at_unix_ms <= now_unix_ms
        || record.invocation_fingerprint != plan.invocation_fingerprint
        || record.operation_id != plan.operation_id
        || record.lane != plan.lane
    {
        return Err("approval token changed during validation".to_string());
    }

    Ok(())
}

fn effect_label(effect: &crate::EffectSpec) -> String {
    match effect {
        crate::EffectSpec::Pure => "pure".to_string(),
        crate::EffectSpec::Read => "read".to_string(),
        crate::EffectSpec::Write => "write".to_string(),
        crate::EffectSpec::Delete => "delete".to_string(),
        crate::EffectSpec::Exec => "exec".to_string(),
        crate::EffectSpec::Network => "network".to_string(),
        crate::EffectSpec::Custom(value) => value.clone(),
        crate::EffectSpec::Composite(effects) => effects
            .iter()
            .map(effect_label)
            .collect::<Vec<_>>()
            .join("+"),
    }
}

/// Parses `codex/sandbox-state-meta` from request meta into a Codex sandbox
/// observation. Accepts `sandboxCwd` (camelCase) or `sandbox_cwd`.
fn codex_sandbox_observation(meta: &Meta) -> Option<CodexSandboxObservation> {
    let state = meta.0.get("codex/sandbox-state-meta")?;
    let cwd = state
        .get("sandboxCwd")
        .or_else(|| state.get("sandbox_cwd"))?
        .as_str()?;
    let mut observation = CodexSandboxObservation::new(cwd);
    if let Some(profile) = state
        .get("permissionProfile")
        .or_else(|| state.get("permission_profile"))
        .and_then(Value::as_str)
    {
        observation = observation.with_permission_profile(profile);
    }
    Some(observation)
}

fn response_profile(request: &RunRequest) -> ResponseProfile {
    if matches!(request.effective_mode(), RunMode::DryRun) {
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
