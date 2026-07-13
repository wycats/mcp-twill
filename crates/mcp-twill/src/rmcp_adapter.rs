use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::Utc;
use mcp_workspace_resolver::{CodexSandboxObservation, McpRootsObservation, ResolvedWorkspaceSet};
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
    FrameworkError, FrameworkEvent, HelpRequest, HelpResult, InvocationContext, InvocationPlan,
    NoopEventSink, PermissionAuthorizer, PermissionDecision, PlanFacts, ReplayRecord,
    ResponseEnvelope, ResponseProfile, RunMode, RunRequest, RunResponse, RuntimeIdentity,
    ToolLaneSpec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConversationIdentityCompatibility {
    #[default]
    Disabled,
    TrustedCodexThreadId,
}

/// Whether the rmcp adapter may interpret Codex's legacy sandbox metadata as
/// a trusted workspace observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceMetadataCompatibility {
    #[default]
    Disabled,
    TrustedCodexSandboxState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliMcpServerConfig {
    pub execution_tool_name: String,
    pub replay_ttl_ms: i64,
    pub conversation_identity_compatibility: ConversationIdentityCompatibility,
    pub workspace_metadata_compatibility: WorkspaceMetadataCompatibility,
}

impl Default for CliMcpServerConfig {
    fn default() -> Self {
        Self {
            execution_tool_name: "run".to_string(),
            replay_ttl_ms: 10 * 60 * 1000,
            conversation_identity_compatibility: ConversationIdentityCompatibility::Disabled,
            workspace_metadata_compatibility: WorkspaceMetadataCompatibility::Disabled,
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

    pub fn with_conversation_identity_compatibility(
        mut self,
        compatibility: ConversationIdentityCompatibility,
    ) -> Self {
        self.conversation_identity_compatibility = compatibility;
        self
    }

    pub fn with_workspace_metadata_compatibility(
        mut self,
        compatibility: WorkspaceMetadataCompatibility,
    ) -> Self {
        self.workspace_metadata_compatibility = compatibility;
        self
    }
}

/// Per-call application metadata after request-over-context merging and
/// removal of protocol-owned control keys. This wrapper intentionally has no
/// serialization or schema implementation.
#[derive(Clone, Default)]
struct EffectiveApplicationMeta(Meta);

impl EffectiveApplicationMeta {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0.0.get(key)
    }
}

impl fmt::Debug for EffectiveApplicationMeta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("EffectiveApplicationMeta")
            .field(&"<redacted>")
            .finish()
    }
}

fn effective_application_meta(request: Option<&Meta>, context: &Meta) -> EffectiveApplicationMeta {
    let mut merged = context.clone();
    if let Some(request) = request {
        merged.0.extend(request.0.clone());
    }
    merged
        .0
        .retain(|key, _| key != "progressToken" && !key.starts_with("io.modelcontextprotocol/"));
    EffectiveApplicationMeta(merged)
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
    identity: Arc<RuntimeIdentity>,
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
        registry.validate_types()?;
        registry.validate_workspaces()?;
        registry.validate_capabilities()?;
        registry.validate_resources()?;
        let identity = registry
            .runtime_identity()
            .with_server_version(env!("CARGO_PKG_VERSION"));
        Ok(Self {
            registry: Arc::new(registry),
            config,
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
            task_counter: Arc::new(AtomicU64::new(1)),
            replay: Arc::new(Mutex::new(BTreeMap::new())),
            authorizer: Arc::new(DefaultPermissionAuthorizer),
            events: Arc::new(NoopEventSink),
            identity: Arc::new(identity),
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
    /// `None` here; a runtime host fills those in. Computed once at
    /// construction; recorded events carry this same identity.
    pub fn runtime_identity(&self) -> RuntimeIdentity {
        (*self.identity).clone()
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
        request_meta: Option<Meta>,
        context_meta: Meta,
        client: Peer<RoleServer>,
        request: RunRequest,
    ) -> CallToolResult {
        let profile = response_profile(&request);
        let outcome = self
            .run_tool_flow(tool_name, request_meta, context_meta, client, request)
            .await;
        if self.events.enabled() {
            self.events.record(
                FrameworkEvent::from_envelope(&outcome.envelope, outcome.plan.as_ref())
                    .with_runtime((*self.identity).clone()),
            );
        }
        let links = self.resource_links(&outcome.envelope);
        if outcome.rendered_output {
            let mut result = success_result(outcome.envelope, profile);
            result.content.extend(links);
            result
        } else {
            let mut result = envelope_result(outcome.envelope);
            result.content.extend(links);
            result
        }
    }

    /// Grants and listings become `resource_link` content parts, but only
    /// for resources with a bound reader: a link the server cannot serve
    /// through `resources/read` is a dead link. Without a reader, the URI in
    /// the structured payload is the whole story.
    fn resource_links(&self, envelope: &ResponseEnvelope) -> Vec<Content> {
        let Some(output) = &envelope.output else {
            return Vec::new();
        };
        output
            .grants
            .iter()
            .chain(&output.listings)
            .filter(|reference| {
                !reference.uri.is_empty() && self.registry.has_reader(&reference.resource)
            })
            .map(|reference| {
                Content::resource_link(RawResource::new(
                    reference.uri.clone(),
                    format!("{} {}", reference.resource, reference.id),
                ))
            })
            .collect()
    }

    async fn run_tool_flow(
        &self,
        tool_name: String,
        request_meta: Option<Meta>,
        context_meta: Meta,
        client: Peer<RoleServer>,
        request: RunRequest,
    ) -> RunOutcome {
        let registry = &self.registry;
        let config = &self.config;
        let replay = &self.replay;
        let authorizer = &self.authorizer;
        let profile = response_profile(&request);
        let mode = request.effective_mode();
        // rmcp's transfer-object serde extracts wire `params._meta` into the
        // request extensions exposed as `RequestContext.meta`. Direct handler
        // calls may retain it on `CallToolRequestParams.meta`, so prefer that
        // representation when present and otherwise use the transport-owned
        // context representation for protocol controls such as progress.
        let protocol_meta = request_meta.as_ref().unwrap_or(&context_meta);
        let effective_meta = effective_application_meta(request_meta.as_ref(), &context_meta);
        let invocation_context = match invocation_context_from_meta(
            &effective_meta,
            config.conversation_identity_compatibility,
        ) {
            Ok(context) => context,
            Err(error) => {
                return RunOutcome::envelope(
                    ResponseEnvelope::framework_error(error, Some(request), None),
                    None,
                );
            }
        };
        let codex = match codex_sandbox_observation(
            &effective_meta,
            config.workspace_metadata_compatibility,
        ) {
            Ok(observation) => observation,
            Err(error) => {
                return RunOutcome::envelope(
                    ResponseEnvelope::framework_error(error, Some(request), None),
                    None,
                );
            }
        };
        let resolved = Self::resolve_workspaces_for_call(registry, codex, &client).await;
        Self::notify_progress(protocol_meta, &client, 1.0, 4.0, "Parsing command template").await;
        let plan = match registry.build_plan_with_workspaces_and_context(
            &request,
            &resolved,
            &invocation_context,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                return RunOutcome::envelope(
                    ResponseEnvelope::framework_error(error, Some(request), None),
                    None,
                );
            }
        };
        let plan_for_event = PlanFacts::from(&plan);

        Self::notify_progress(protocol_meta, &client, 2.0, 4.0, "Invocation plan ready").await;
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
            Self::notify_progress(protocol_meta, &client, 4.0, 4.0, "Preview ready").await;
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
                    Self::notify_progress(
                        protocol_meta,
                        &client,
                        4.0,
                        4.0,
                        "Confirmation required",
                    )
                    .await;
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

        Self::notify_progress(
            protocol_meta,
            &client,
            3.0,
            4.0,
            "Dispatching command handler",
        )
        .await;
        let result = registry
            .run_in_lane_with_workspaces_and_context(
                request.clone(),
                tool_name,
                lane,
                &config.execution_tool_name,
                &resolved,
                &invocation_context,
            )
            .await;
        match result {
            Ok(response) => {
                Self::notify_progress(protocol_meta, &client, 4.0, 4.0, "Command complete").await;
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
        codex: Option<CodexSandboxObservation>,
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

        if let Some(codex) = codex {
            observations = observations.with_codex_sandbox(codex);
        }

        registry.resolve_workspaces(&observations)
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
                self.events.record(
                    FrameworkEvent::parse_failure(error.message.clone())
                        .with_runtime((*self.identity).clone()),
                );
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
            .with_instructions({
                let mut instructions = String::new();
                if let Some(preamble) = self.registry.preamble() {
                    instructions.push_str(preamble);
                    instructions.push_str("\n\n");
                }
                instructions.push_str(&format!(
                    "Use `help` to discover command templates. Start execution with `{}`; the framework returns structured retry data when another effect-lane tool is required. Command strings are typed templates, not shell programs.",
                    self.config.execution_tool_name
                ));
                instructions
            })
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

        let request_meta = request.meta.clone();
        let run_request = self.parse_run_request(request.arguments)?;
        Ok(self
            .clone()
            .execute_run_tool(
                tool_name,
                request_meta,
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
        if let Some((decl, id)) = self.registry.match_resource_uri(&request.uri) {
            let name = decl.name.clone();
            let reader = self.registry.resource_reader(&name).ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    format!("Resource `{name}` does not support resources/read"),
                    None,
                )
            })?;
            let value = reader.read_erased(&id).await.map_err(|refusal| {
                rmcp::ErrorData::invalid_params(
                    format!("Cannot read {}: {}", request.uri, refusal.detail),
                    None,
                )
            })?;
            let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(text, request.uri).with_mime_type("application/json"),
            ]));
        }
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
        let mut text = String::new();
        if let Some(preamble) = self.registry.preamble() {
            text.push_str(preamble);
            text.push_str("\n\n");
        }
        text.push_str(&format!(
            "First call `help` with no command. Then call `help` for a command. Start execution with `{}` and use escalated lane tools only when a structured response asks for one. Use typed `$args.*` values; do not use shell syntax in the command string.",
            self.config.execution_tool_name
        ));
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
        let request_meta = request.meta.clone();
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
        let context_meta = context.meta.clone();
        let client = context.peer.clone();
        tokio::spawn(async move {
            let result = server
                .execute_run_tool(tool_name, request_meta, context_meta, client, run_request)
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
        if let std::collections::btree_map::Entry::Vacant(entry) = replay.entry(token) {
            entry.insert(record.clone());
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

const CODEX_SANDBOX_META_KEY: &str = "codex/sandbox-state-meta";

/// Parses the trusted Codex workspace compatibility payload after effective
/// application-context merging. Disabled compatibility ignores every shape.
fn codex_sandbox_observation(
    meta: &EffectiveApplicationMeta,
    compatibility: WorkspaceMetadataCompatibility,
) -> crate::Result<Option<CodexSandboxObservation>> {
    if matches!(compatibility, WorkspaceMetadataCompatibility::Disabled) {
        return Ok(None);
    }
    let Some(state) = meta.get(CODEX_SANDBOX_META_KEY) else {
        return Ok(None);
    };
    let object = state.as_object().ok_or_else(|| {
        invalid_workspace_metadata(None, crate::WorkspaceMetadataProblem::ExpectedObject)
    })?;

    let cwd = aliased_string(object, "sandboxCwd", "sandbox_cwd", true)?.ok_or_else(|| {
        invalid_workspace_metadata(
            Some("sandboxCwd"),
            crate::WorkspaceMetadataProblem::MissingSandboxCwd,
        )
    })?;
    if cwd.is_empty() {
        return Err(invalid_workspace_metadata(
            Some("sandboxCwd"),
            crate::WorkspaceMetadataProblem::InvalidSandboxCwd,
        ));
    }

    let mut observation = CodexSandboxObservation::new(cwd);
    if let Some(profile) = aliased_string(object, "permissionProfile", "permission_profile", false)?
    {
        observation = observation.with_permission_profile(profile);
    }
    Ok(Some(observation))
}

fn aliased_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    canonical: &'static str,
    alias: &'static str,
    cwd: bool,
) -> crate::Result<Option<&'a str>> {
    let canonical_value = object.get(canonical);
    let alias_value = object.get(alias);
    if let (Some(left), Some(right)) = (canonical_value, alias_value)
        && left != right
    {
        return Err(invalid_workspace_metadata(
            Some(canonical),
            crate::WorkspaceMetadataProblem::ConflictingAliases,
        ));
    }
    let Some(value) = canonical_value.or(alias_value) else {
        return Ok(None);
    };
    value.as_str().map(Some).ok_or_else(|| {
        invalid_workspace_metadata(
            Some(canonical),
            if cwd {
                crate::WorkspaceMetadataProblem::InvalidSandboxCwd
            } else {
                crate::WorkspaceMetadataProblem::InvalidPermissionProfile
            },
        )
    })
}

fn invalid_workspace_metadata(
    field: Option<&str>,
    reason: crate::WorkspaceMetadataProblem,
) -> FrameworkError {
    FrameworkError::InvalidWorkspaceMetadata {
        key: CODEX_SANDBOX_META_KEY.to_string(),
        field: field.map(str::to_string),
        reason,
    }
}

fn invocation_context_from_meta(
    meta: &EffectiveApplicationMeta,
    compatibility: ConversationIdentityCompatibility,
) -> crate::Result<InvocationContext> {
    let canonical = meta
        .get(crate::CONVERSATION_IDENTITY_META_KEY)
        .map(crate::conversation_identity::parse_canonical_identity)
        .transpose()?;

    if matches!(compatibility, ConversationIdentityCompatibility::Disabled) {
        return Ok(canonical.map_or_else(InvocationContext::new, |identity| {
            InvocationContext::new().with_conversation_identity(identity)
        }));
    }

    let codex = meta
        .get("threadId")
        .map(crate::conversation_identity::codex_thread_identity)
        .transpose()?;
    match (canonical, codex) {
        (Some(canonical), Some(codex)) if canonical != codex => {
            Err(FrameworkError::ConflictingConversationIdentity)
        }
        (Some(identity), _) | (None, Some(identity)) => {
            Ok(InvocationContext::new().with_conversation_identity(identity))
        }
        (None, None) => Ok(InvocationContext::new()),
    }
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
        let mut text = envelope
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
        // Minted references survive the text projection: without a reader
        // there is no resource_link content part, so this line is the only
        // place the URI reaches a text-profile caller.
        if let Some(output) = &envelope.output {
            for reference in output.grants.iter().chain(&output.listings) {
                if !reference.uri.is_empty() {
                    text.push_str(&format!(
                        "\n{}: {} ({})",
                        reference.resource, reference.id, reference.uri
                    ));
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Meta {
        Meta(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    #[test]
    fn effective_application_metadata_merges_per_key_and_filters_protocol_controls() {
        let context = meta([
            ("contextOnly", json!("kept")),
            ("shared", json!("context")),
            ("progressToken", json!("context-token")),
            (
                "io.modelcontextprotocol/related-task",
                json!("context-task"),
            ),
        ]);
        let request = meta([
            ("requestOnly", json!("kept")),
            ("shared", json!("request")),
            ("progressToken", json!("request-token")),
            ("io.modelcontextprotocol/version", json!(1)),
        ]);

        let effective = effective_application_meta(Some(&request), &context);
        assert_eq!(effective.get("contextOnly"), Some(&json!("kept")));
        assert_eq!(effective.get("requestOnly"), Some(&json!("kept")));
        assert_eq!(effective.get("shared"), Some(&json!("request")));
        assert_eq!(effective.get("progressToken"), None);
        assert_eq!(effective.get("io.modelcontextprotocol/related-task"), None);
        assert_eq!(effective.get("io.modelcontextprotocol/version"), None);

        let debug = format!("{effective:?}");
        assert_eq!(debug, "EffectiveApplicationMeta(\"<redacted>\")");
        assert!(!debug.contains("contextOnly"));
    }

    #[test]
    fn trusted_codex_workspace_metadata_is_strict_and_default_disabled() {
        let malformed = effective_application_meta(
            Some(&meta([(CODEX_SANDBOX_META_KEY, json!({"sandboxCwd": 7}))])),
            &Meta::default(),
        );
        assert_eq!(
            codex_sandbox_observation(&malformed, WorkspaceMetadataCompatibility::Disabled)
                .unwrap(),
            None
        );
        assert!(matches!(
            codex_sandbox_observation(
                &malformed,
                WorkspaceMetadataCompatibility::TrustedCodexSandboxState
            ),
            Err(FrameworkError::InvalidWorkspaceMetadata {
                reason: crate::WorkspaceMetadataProblem::InvalidSandboxCwd,
                ..
            })
        ));

        let conflicting = effective_application_meta(
            Some(&meta([(
                CODEX_SANDBOX_META_KEY,
                json!({"sandboxCwd": "/a", "sandbox_cwd": "/b"}),
            )])),
            &Meta::default(),
        );
        assert!(matches!(
            codex_sandbox_observation(
                &conflicting,
                WorkspaceMetadataCompatibility::TrustedCodexSandboxState
            ),
            Err(FrameworkError::InvalidWorkspaceMetadata {
                reason: crate::WorkspaceMetadataProblem::ConflictingAliases,
                ..
            })
        ));
    }
}
