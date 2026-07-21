use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{SecondsFormat, Utc};
use mcp_workspace_resolver::{CodexSandboxObservation, McpRootsObservation, ResolvedWorkspaceSet};
use rand::{RngCore, rngs::OsRng};
use rmcp::{
    Peer, RoleServer, ServerHandler,
    handler::server::tool::schema_for_type,
    model::{
        CallToolRequestParams, CallToolResult, CancelTaskParams, CancelTaskResult, Content,
        CreateTaskResult, CustomResult, ErrorCode, Extensions, GetPromptRequestParams,
        GetPromptResult, GetTaskInfoParams, GetTaskPayloadResult, GetTaskResult,
        GetTaskResultParams, Implementation, JsonRpcMessage, ListPromptsResult,
        ListResourcesResult, ListTasksResult, ListToolsResult, Meta, PaginatedRequestParams,
        ProgressNotificationParam, ProtocolVersion, RawResource, ReadResourceRequestParams,
        ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, ServerResult, Task,
        TaskStatus, TaskSupport, Tool, ToolAnnotations, ToolExecution,
    },
    service::{RequestContext, RunningService, ServerInitializeError, TxJsonRpcMessage},
    transport::{IntoTransport, Transport},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};

use crate::{
    ApprovalInput, CommandRegistry, CompiledTaskDelivery, DefaultPermissionAuthorizer, EffectLane,
    EventSink, FrameworkError, FrameworkEvent, HelpRequest, HelpResult, InMemoryTaskStore,
    InvocationContext, InvocationPlan, McpProtocolTarget, McpToolSurface,
    NativeApplicationErrorBody, NativeApplicationErrorDialect, NativeApplicationRecovery,
    NativeConfirmationBridge, NativeConfirmationDecision, NativeConfirmationRequest,
    NativeConfirmationRoute, NativeToolSurface, NoopEventSink, PermissionAuthorizer,
    PermissionDecision, PlanFacts, ReplayRecord, ResponseEnvelope, ResponseProfile, RunMode,
    RunRequest, RunResponse, RuntimeIdentity, SemanticTaskProfile, SemanticTaskRecord,
    SemanticTaskStatus, ServingSurfaceIdentity, StoredTaskRecord, SurfacePresentationDefaults,
    TaskAccessContext, TaskAccessPolicy, TaskAccessScope, TaskRuntime, TaskStore, TaskStoreCreate,
    TaskStoreMount, TaskStoreScope, TaskStoreWrite, TaskSupportSpec, ToolLaneSpec,
    checked_task_expiration, derive_task_storage_key, generate_task_id, scope_hex,
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

fn progress_meta<'a>(request: Option<&'a Meta>, context: &'a Meta) -> Option<&'a Meta> {
    request
        .filter(|meta| meta.get_progress_token().is_some())
        .or_else(|| context.get_progress_token().map(|_| context))
}

fn validate_protocol_observations<'a>(
    compiled: &str,
    request: Option<&'a Value>,
    transport: Option<&'a Value>,
    negotiated: Option<&'a str>,
) -> std::result::Result<(), rmcp::ErrorData> {
    let request = request
        .map(|value| {
            value.as_str().ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "MCP protocol version observation must be a string",
                    None,
                )
            })
        })
        .transpose()?;
    let transport = transport
        .map(|value| {
            value.as_str().ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "MCP protocol version observation must be a string",
                    None,
                )
            })
        })
        .transpose()?;

    let mut observed = None;
    for candidate in [request, transport, negotiated].into_iter().flatten() {
        if observed.is_some_and(|observed| observed != candidate) {
            return Err(rmcp::ErrorData::invalid_params(
                "Conflicting MCP protocol version observations",
                None,
            ));
        }
        observed = Some(candidate);
    }

    let observed = observed.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("Missing MCP protocol version observation", None)
    })?;
    if observed != compiled {
        return Err(rmcp::ErrorData::invalid_params(
            format!(
                "MCP protocol version `{observed}` does not match compiled surface `{compiled}`"
            ),
            None,
        ));
    }
    Ok(())
}

#[derive(Clone)]
pub struct CliMcpServer {
    registry: Arc<CommandRegistry>,
    config: CliMcpServerConfig,
    surface: McpToolSurface,
    native_confirmation_bridge: Option<Arc<dyn NativeConfirmationBridge>>,
    task_runtime: Option<TaskRuntime>,
    live_tasks: Arc<Mutex<BTreeMap<String, LiveTaskControl>>>,
    replay: Arc<Mutex<BTreeMap<String, ReplayRecord>>>,
    authorizer: Arc<dyn PermissionAuthorizer>,
    events: Arc<dyn EventSink>,
    identity: Arc<RuntimeIdentity>,
    legacy_session: Arc<LegacySessionState>,
}

#[derive(Default)]
struct LegacySessionState {
    started: AtomicBool,
    closed: AtomicBool,
}

struct LegacyCompatibilityTransport<T> {
    inner: T,
    cleanup: Option<LegacySessionCleanup>,
}

struct LegacySessionCleanup {
    state: Arc<LegacySessionState>,
    live_tasks: Arc<Mutex<BTreeMap<String, LiveTaskControl>>>,
}

impl LegacySessionCleanup {
    async fn run(self) {
        if self.state.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let tasks = std::mem::take(&mut *self.live_tasks.lock().await);
        for task in tasks.into_values() {
            task.cancel();
        }
    }

    fn run_after_drop(self) {
        if self.state.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut tasks) = self.live_tasks.try_lock() {
            let tasks = std::mem::take(&mut *tasks);
            for task in tasks.into_values() {
                task.cancel();
            }
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let live_tasks = self.live_tasks;
            runtime.spawn(async move {
                let tasks = std::mem::take(&mut *live_tasks.lock().await);
                for task in tasks.into_values() {
                    task.cancel();
                }
            });
        }
    }
}

impl<T> Drop for LegacyCompatibilityTransport<T> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup.run_after_drop();
        }
    }
}

impl<T> Transport<RoleServer> for LegacyCompatibilityTransport<T>
where
    T: Transport<RoleServer>,
{
    type Error = T::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<(), Self::Error>> + Send + 'static
    {
        self.inner.send(with_legacy_related_task_meta(message))
    }

    fn receive(
        &mut self,
    ) -> impl std::future::Future<Output = Option<rmcp::service::RxJsonRpcMessage<RoleServer>>> + Send
    {
        self.inner.receive()
    }

    fn close(
        &mut self,
    ) -> impl std::future::Future<Output = std::result::Result<(), Self::Error>> + Send {
        let cleanup = self.cleanup.take();
        async move {
            let result = self.inner.close().await;
            if let Some(cleanup) = cleanup {
                cleanup.run().await;
            }
            result
        }
    }
}

fn with_legacy_related_task_meta(
    mut message: TxJsonRpcMessage<RoleServer>,
) -> TxJsonRpcMessage<RoleServer> {
    if let JsonRpcMessage::Response(response) = &mut message
        && let ServerResult::CreateTaskResult(created) = &response.result
        && let Ok(Value::Object(mut value)) = serde_json::to_value(created)
    {
        value.insert(
            "_meta".to_string(),
            json!({
                "io.modelcontextprotocol/related-task": {
                    "taskId": created.task.task_id,
                }
            }),
        );
        response.result = ServerResult::CustomResult(CustomResult::new(Value::Object(value)));
    }
    message
}

#[derive(Clone)]
enum LiveTaskControl {
    Legacy(tokio::task::AbortHandle),
    Extension(TaskCancellation),
}

impl LiveTaskControl {
    fn cancel(self) {
        match self {
            Self::Legacy(handle) => handle.abort(),
            Self::Extension(cancellation) => cancellation.cancel(),
        }
    }
}

#[derive(Clone, Default)]
struct TaskCancellation {
    inner: Arc<TaskCancellationInner>,
}

#[derive(Default)]
struct TaskCancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl TaskCancellation {
    fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            self.inner.notify.notify_waiters();
        }
    }

    async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.inner.cancelled.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

pub struct CliMcpServerBuilder {
    registry: CommandRegistry,
    config: CliMcpServerConfig,
    config_authored: bool,
    surface: Option<McpToolSurface>,
    authorizer: Option<Arc<dyn PermissionAuthorizer>>,
    native_confirmation_bridge: Option<Arc<dyn NativeConfirmationBridge>>,
    task_runtime: Option<TaskRuntime>,
    errors: Vec<FrameworkError>,
}

impl CliMcpServerBuilder {
    fn new(registry: CommandRegistry) -> Self {
        Self {
            registry,
            config: CliMcpServerConfig::default(),
            config_authored: false,
            surface: None,
            authorizer: None,
            native_confirmation_bridge: None,
            task_runtime: None,
            errors: Vec::new(),
        }
    }

    pub fn config(mut self, config: CliMcpServerConfig) -> Self {
        if self.config_authored {
            self.errors.push(FrameworkError::Build(
                "MCP server assigns `config` more than once".to_string(),
            ));
        } else {
            self.config_authored = true;
            self.config = config;
        }
        self
    }

    pub fn surface(mut self, surface: impl Into<McpToolSurface>) -> Self {
        if self.surface.is_some() {
            self.errors.push(FrameworkError::Build(
                "MCP server assigns `surface` more than once".to_string(),
            ));
        } else {
            self.surface = Some(surface.into());
        }
        self
    }

    pub fn authorizer(mut self, authorizer: Arc<dyn PermissionAuthorizer>) -> Self {
        if self.authorizer.is_some() {
            self.errors.push(FrameworkError::Build(
                "MCP server assigns `authorizer` more than once".to_string(),
            ));
        } else {
            self.authorizer = Some(authorizer);
        }
        self
    }

    pub fn native_confirmation_bridge(mut self, bridge: Arc<dyn NativeConfirmationBridge>) -> Self {
        if self.native_confirmation_bridge.is_some() {
            self.errors.push(FrameworkError::Build(
                "MCP server assigns `native_confirmation_bridge` more than once".to_string(),
            ));
        } else {
            self.native_confirmation_bridge = Some(bridge);
        }
        self
    }

    pub fn task_runtime(mut self, store: Arc<dyn TaskStore>, access: TaskAccessPolicy) -> Self {
        if self.task_runtime.is_some() {
            self.errors.push(FrameworkError::Build(
                "MCP server assigns `task_runtime` more than once".to_string(),
            ));
        } else {
            self.task_runtime = Some(TaskRuntime {
                store,
                access,
                _mount: None,
            });
        }
        self
    }

    pub fn build(self) -> crate::Result<CliMcpServer> {
        CliMcpServer::finish(self)
    }
}

fn finalize_task_runtime(
    surface: &McpToolSurface,
    authored: Option<TaskRuntime>,
) -> crate::Result<Option<TaskRuntime>> {
    let delivery = match surface {
        McpToolSurface::EffectLanes(_) => None,
        McpToolSurface::Native(surface) => Some(surface.snapshot().task_delivery()),
    };
    match delivery {
        Some(CompiledTaskDelivery::Disabled) => {
            if authored.is_some() {
                Err(FrameworkError::Build(
                    "disabled task delivery rejects a task runtime".to_string(),
                ))
            } else {
                Ok(None)
            }
        }
        None | Some(CompiledTaskDelivery::Legacy2025_11_25(_)) => {
            let runtime = authored.unwrap_or_else(|| TaskRuntime {
                store: InMemoryTaskStore::connection(),
                access: TaskAccessPolicy::CapabilityId,
                _mount: None,
            });
            if runtime.store.scope() != TaskStoreScope::Connection {
                return Err(FrameworkError::Build(
                    "legacy task delivery requires a connection-scoped task store".to_string(),
                ));
            }
            Ok(Some(runtime))
        }
        Some(CompiledTaskDelivery::TasksExtension(_)) => {
            let mut runtime = authored.ok_or_else(|| {
                FrameworkError::Build(
                    "Tasks Extension delivery requires an explicit task runtime".to_string(),
                )
            })?;
            if runtime.store.scope() != TaskStoreScope::ServerInstance {
                return Err(FrameworkError::Build(
                    "Tasks Extension delivery requires a server-instance task store".to_string(),
                ));
            }
            runtime._mount = Some(TaskStoreMount::acquire(&runtime.store).map_err(|()| {
                FrameworkError::Build(
                    "Tasks Extension task store is already mounted by a live server".to_string(),
                )
            })?);
            Ok(Some(runtime))
        }
    }
}

/// How one run-tool call ended: the envelope to return, the plan facts (when
/// planning succeeded) for event enrichment, and whether the envelope
/// renders through the output profile.
struct RunOutcome {
    envelope: ResponseEnvelope,
    plan: Option<PlanFacts>,
    rendered_output: bool,
}

struct NativeRunOutcome {
    result: CallToolResult,
    envelope: ResponseEnvelope,
    plan: Option<PlanFacts>,
}

struct LoadedTask {
    key: crate::TaskStorageKey,
    stored: StoredTaskRecord,
    semantic: SemanticTaskRecord,
}

enum LegacyTaskInvocation {
    EffectLane(RunRequest),
    Native(rmcp::model::JsonObject),
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
    fn protocol_version(&self) -> &str {
        match &self.surface {
            McpToolSurface::EffectLanes(_) => "2025-11-25",
            McpToolSurface::Native(surface) => surface.snapshot().protocol_version(),
        }
    }

    fn validate_protocol(
        &self,
        request_meta: Option<&Meta>,
        context: &RequestContext<RoleServer>,
    ) -> std::result::Result<(), rmcp::ErrorData> {
        const KEY: &str = "io.modelcontextprotocol/protocolVersion";
        let request = request_meta.and_then(|meta| meta.0.get(KEY));
        let transport = context.meta.0.get(KEY);
        let negotiated = context
            .peer
            .peer_info()
            .map(|info| info.protocol_version.as_str());
        validate_protocol_observations(self.protocol_version(), request, transport, negotiated)
    }

    fn ensure_tasks_supported(&self) -> std::result::Result<(), rmcp::ErrorData> {
        if self.task_profile() == Some(SemanticTaskProfile::Legacy2025_11_25) {
            Ok(())
        } else {
            Err(rmcp::ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                "Method not found",
                None,
            ))
        }
    }

    pub fn builder(registry: CommandRegistry) -> CliMcpServerBuilder {
        CliMcpServerBuilder::new(registry)
    }

    /// Serves the negotiated MCP 2025-11-25 compatibility path.
    ///
    /// Twill keeps this dot-call source-compatible with rmcp's `ServiceExt`
    /// while correcting legacy task creation metadata at the private
    /// transport boundary.
    pub async fn serve<T, E, A>(
        self,
        transport: T,
    ) -> std::result::Result<RunningService<RoleServer, Self>, ServerInitializeError>
    where
        T: IntoTransport<RoleServer, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        if self.stateless_compatible() {
            return Err(ServerInitializeError::InitializeFailed(
                rmcp::ErrorData::internal_error(
                    "stateless MCP surfaces require `into_stateless_service`",
                    None,
                ),
            ));
        }
        let cleanup = if self.task_profile() == Some(SemanticTaskProfile::Legacy2025_11_25) {
            if self
                .legacy_session
                .started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(ServerInitializeError::InitializeFailed(
                    rmcp::ErrorData::internal_error(
                        "legacy task runtime already belongs to an MCP session",
                        None,
                    ),
                ));
            }
            Some(LegacySessionCleanup {
                state: self.legacy_session.clone(),
                live_tasks: self.live_tasks.clone(),
            })
        } else {
            None
        };
        let transport = LegacyCompatibilityTransport {
            inner: transport.into_transport(),
            cleanup,
        };
        rmcp::serve_server(self, transport).await
    }

    pub fn new(registry: CommandRegistry) -> crate::Result<Self> {
        Self::builder(registry).build()
    }

    pub fn with_config(
        registry: CommandRegistry,
        config: CliMcpServerConfig,
    ) -> crate::Result<Self> {
        Self::builder(registry).config(config).build()
    }

    pub fn with_surface(
        registry: CommandRegistry,
        surface: impl Into<McpToolSurface>,
    ) -> crate::Result<Self> {
        Self::builder(registry).surface(surface).build()
    }

    pub fn with_config_and_surface(
        registry: CommandRegistry,
        config: CliMcpServerConfig,
        surface: impl Into<McpToolSurface>,
    ) -> crate::Result<Self> {
        Self::builder(registry)
            .config(config)
            .surface(surface)
            .build()
    }

    fn finish(mut builder: CliMcpServerBuilder) -> crate::Result<Self> {
        if let Some(error) = builder.errors.into_iter().next() {
            return Err(error);
        }
        let registry = builder.registry;
        let config = builder.config;
        registry.validate_effects()?;
        registry.validate_guidance()?;
        registry.validate_types()?;
        registry.validate_argument_schemas()?;
        registry.validate_presentations()?;
        registry.validate_workspaces()?;
        registry.validate_capabilities()?;
        registry.validate_resources()?;
        registry.validate_results()?;

        let surface = match builder.surface.take() {
            Some(surface) => surface,
            None => {
                let tools = effect_lane_tools(&registry, &config)?;
                let defaults = effect_lane_presentation_defaults(&registry, &config)?;
                let instructions = effect_lane_instructions(&registry, &config);
                McpToolSurface::EffectLanes(crate::native_surfaces::compile_effect_lane_surface(
                    &registry,
                    &tools,
                    &instructions,
                    &defaults,
                )?)
            }
        };

        let catalog_hash = registry.catalog_identity().catalog_hash;
        if let McpToolSurface::Native(native) = &surface {
            if native.snapshot().catalog_hash() != catalog_hash {
                return Err(FrameworkError::Build(
                    "native surface was compiled for a different command catalog".to_string(),
                ));
            }
            match (
                native.confirmation_route(),
                builder.native_confirmation_bridge.is_some(),
            ) {
                (NativeConfirmationRoute::Bridge, false) => {
                    return Err(FrameworkError::Build(
                        "native bridge confirmation route requires a bridge".to_string(),
                    ));
                }
                (NativeConfirmationRoute::Unavailable, true) => {
                    return Err(FrameworkError::Build(
                        "native unavailable confirmation route rejects a bridge".to_string(),
                    ));
                }
                _ => {}
            }
        } else if builder.native_confirmation_bridge.is_some() {
            return Err(FrameworkError::Build(
                "effect-lane surfaces reject a native confirmation bridge".to_string(),
            ));
        }

        let task_runtime = finalize_task_runtime(&surface, builder.task_runtime.take())?;

        let surface_identity = surface_identity(&surface)?;
        let identity = registry
            .runtime_identity()
            .with_server_version(env!("CARGO_PKG_VERSION"))
            .with_surface(surface_identity);
        Ok(Self {
            registry: Arc::new(registry),
            config,
            surface,
            native_confirmation_bridge: builder.native_confirmation_bridge,
            task_runtime,
            live_tasks: Arc::new(Mutex::new(BTreeMap::new())),
            replay: Arc::new(Mutex::new(BTreeMap::new())),
            authorizer: builder
                .authorizer
                .unwrap_or_else(|| Arc::new(DefaultPermissionAuthorizer)),
            events: Arc::new(NoopEventSink),
            identity: Arc::new(identity),
            legacy_session: Arc::new(LegacySessionState::default()),
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

    async fn notify_native_progress(
        meta: Option<&Meta>,
        client: Option<&Peer<RoleServer>>,
        stateless: Option<&mpsc::Sender<Value>>,
        progress: f64,
        total: f64,
        message: &'static str,
    ) {
        let Some(meta) = meta else {
            return;
        };
        if let Some(client) = client {
            Self::notify_progress(meta, client, progress, total, message).await;
            return;
        }
        let Some(sender) = stateless else {
            return;
        };
        if let Some(progress_token) = meta.get_progress_token() {
            let _ = sender
                .send(json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": {
                        "progressToken": progress_token,
                        "progress": progress,
                        "total": total,
                        "message": message,
                    }
                }))
                .await;
        }
    }

    fn execution_lanes(&self) -> Vec<ToolLaneSpec> {
        self.registry.lane_specs(&self.config.execution_tool_name)
    }

    fn tools(&self) -> Vec<Tool> {
        match &self.surface {
            McpToolSurface::EffectLanes(surface) => surface.tools().to_vec(),
            McpToolSurface::Native(surface) => surface.snapshot().tools().to_vec(),
        }
    }

    fn resources(&self) -> Vec<RawResource> {
        let effect_lanes = matches!(self.surface, McpToolSurface::EffectLanes(_));
        let mut resources = Vec::new();
        if effect_lanes {
            resources.push(RawResource::new("cli://server/overview", "Server overview"));
        }
        resources.extend([
            RawResource::new("cli://catalog", "Command catalog"),
            RawResource::new("cli://commands", "Command catalog"),
            RawResource::new("cli://permissions", "Permission model"),
        ]);
        if effect_lanes {
            resources.push(RawResource::new("cli://lanes", "Effect-lane tools"));
        }
        resources.extend(self.registry.command_specs().map(|spec| {
            RawResource::new(
                format!("cli://commands/{}", spec.path.join("/")),
                format!("Command {}", spec.name()),
            )
        }));
        resources
    }

    fn catalog_resource_text(&self) -> Option<String> {
        let mut catalog = serde_json::to_value(self.registry.catalog()).ok()?;
        let active_surface = match &self.surface {
            McpToolSurface::EffectLanes(surface) => json!({
                "version": surface.document()["version"],
                "protocolVersion": surface.document()["protocolVersion"],
                "name": surface.identity().name,
                "surfaceHash": surface.identity().hash,
                "routes": surface.tools().iter().map(|tool| json!({
                    "tool": tool.name,
                })).collect::<Vec<_>>(),
            }),
            McpToolSurface::Native(surface) => json!({
                "version": surface.snapshot().version(),
                "protocolVersion": surface.snapshot().protocol_version(),
                "name": surface.snapshot().name(),
                "surfaceHash": surface.snapshot().surface_hash(),
                "exposure": surface.declaration().exposure,
                "confirmation": surface.declaration().confirmation,
                "routes": surface.snapshot().operations().iter().map(|operation| json!({
                    "operationId": operation.spec().id,
                    "tool": operation.call().tool(),
                    "arguments": operation.call().arguments(),
                })).collect::<Vec<_>>(),
            }),
        };
        catalog
            .as_object_mut()?
            .insert("activeSurface".to_string(), active_surface);
        serde_json::to_string_pretty(&catalog).ok()
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

    async fn execute_native_tool(
        self,
        tool_name: String,
        request_meta: Option<Meta>,
        context_meta: Meta,
        client: Peer<RoleServer>,
        arguments: rmcp::model::JsonObject,
    ) -> CallToolResult {
        let outcome = self
            .run_native_tool_flow(
                tool_name,
                request_meta,
                context_meta,
                Some(client),
                None,
                arguments,
            )
            .await;
        if self.events.enabled() {
            self.events.record(
                FrameworkEvent::from_envelope(&outcome.envelope, outcome.plan.as_ref())
                    .with_runtime((*self.identity).clone()),
            );
        }
        outcome.result
    }

    async fn execute_native_tool_stateless(
        self,
        tool_name: String,
        request_meta: Option<Meta>,
        arguments: rmcp::model::JsonObject,
        progress: Option<mpsc::Sender<Value>>,
    ) -> CallToolResult {
        let outcome = self
            .run_native_tool_flow(
                tool_name,
                request_meta,
                Meta::default(),
                None,
                progress,
                arguments,
            )
            .await;
        if self.events.enabled() {
            self.events.record(
                FrameworkEvent::from_envelope(&outcome.envelope, outcome.plan.as_ref())
                    .with_runtime((*self.identity).clone()),
            );
        }
        outcome.result
    }

    async fn run_native_tool_flow(
        &self,
        tool_name: String,
        request_meta: Option<Meta>,
        context_meta: Meta,
        client: Option<Peer<RoleServer>>,
        stateless_progress: Option<mpsc::Sender<Value>>,
        arguments: rmcp::model::JsonObject,
    ) -> NativeRunOutcome {
        let surface = match &self.surface {
            McpToolSurface::Native(surface) => surface,
            McpToolSurface::EffectLanes(_) => {
                return native_framework_outcome(
                    FrameworkError::Build(
                        "effect-lane surface entered the native execution path".to_string(),
                    ),
                    None,
                );
            }
        };
        let progress_meta = progress_meta(request_meta.as_ref(), &context_meta);
        let effective_meta = effective_application_meta(request_meta.as_ref(), &context_meta);
        let invocation_context = match invocation_context_from_meta(
            &effective_meta,
            self.config.conversation_identity_compatibility,
        ) {
            Ok(context) => context,
            Err(error) => return native_framework_outcome(error, None),
        };
        let codex = match codex_sandbox_observation(
            &effective_meta,
            self.config.workspace_metadata_compatibility,
        ) {
            Ok(observation) => observation,
            Err(error) => return native_framework_outcome(error, None),
        };
        let resolved = if let Some(client) = &client {
            Self::resolve_workspaces_for_call(&self.registry, codex, client).await
        } else {
            let mut observations = self.registry.declared_observations();
            if let Some(codex) = codex {
                observations = observations.with_codex_sandbox(codex);
            }
            self.registry.resolve_workspaces(&observations)
        };
        Self::notify_native_progress(
            progress_meta,
            client.as_ref(),
            stateless_progress.as_ref(),
            1.0,
            5.0,
            "Planning native invocation",
        )
        .await;
        let (operation_id, selected_arguments) = match surface.resolve_call(&tool_name, arguments) {
            Ok(call) => call,
            Err(error) => return native_framework_outcome(error, None),
        };
        let identity = match surface.identity() {
            Ok(identity) => identity,
            Err(error) => return native_framework_outcome(error, None),
        };
        let prepared = match self.registry.build_native_operation_plan(
            &operation_id,
            selected_arguments.clone(),
            &resolved,
            &invocation_context,
            identity,
            surface,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(operation) = surface.snapshot().operation(&operation_id) {
                    return native_framework_outcome_for_operation(error, operation.spec());
                }
                return native_framework_outcome(error, None);
            }
        };
        let plan = prepared.plan().clone();
        Self::notify_native_progress(
            progress_meta,
            client.as_ref(),
            stateless_progress.as_ref(),
            2.0,
            5.0,
            "Invocation plan ready",
        )
        .await;
        let plan_for_event = PlanFacts::from(&plan);

        if let Some(availability) = self.registry.binding_availability(&prepared) {
            return match availability {
                Ok(crate::CommandExecutionOutcome::ApplicationError { plan, error }) => {
                    native_application_error_outcome(surface, plan, error, plan_for_event)
                }
                Ok(crate::CommandExecutionOutcome::Success(_)) => {
                    unreachable!("binding_availability never returns a Success outcome")
                }
                Err(error) => native_framework_outcome(error, Some((plan, plan_for_event))),
            };
        }

        if let Err(error) = self.registry.check_plan_policy(&plan) {
            return native_framework_outcome(error, Some((plan, plan_for_event)));
        }

        match self.authorizer.decide(&plan) {
            PermissionDecision::Allow => {}
            PermissionDecision::Deny { reason } => {
                return native_framework_outcome(
                    FrameworkError::PermissionDenied {
                        effect: effect_label(&plan.effect),
                        scope: reason,
                    },
                    Some((plan, plan_for_event)),
                );
            }
            PermissionDecision::RequireConfirmation => {
                if matches!(
                    surface.confirmation_route(),
                    NativeConfirmationRoute::Unavailable
                ) {
                    return native_framework_outcome(
                        FrameworkError::ConfirmationUnavailable {
                            operation_id: plan.operation_id.clone(),
                        },
                        Some((plan, plan_for_event)),
                    );
                }
                let Some(defaults) = surface.presentation_defaults(&operation_id) else {
                    return native_framework_outcome(
                        FrameworkError::Build(format!(
                            "native surface has no presentation defaults for `{operation_id}`"
                        )),
                        Some((plan, plan_for_event)),
                    );
                };
                let confirmation = match self.registry.prepare_native_confirmation(&plan, defaults)
                {
                    Ok(confirmation) => confirmation,
                    Err(error) => {
                        return native_framework_outcome(error, Some((plan, plan_for_event)));
                    }
                };
                let preview = crate::response::permission_preview(&plan, true, Some(confirmation));
                let bridge_request = NativeConfirmationRequest::new(
                    preview,
                    selected_arguments
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                    plan.invocation_fingerprint.clone(),
                );
                let bridge = self
                    .native_confirmation_bridge
                    .as_ref()
                    .expect("bridge route was validated at server construction");
                Self::notify_native_progress(
                    progress_meta,
                    client.as_ref(),
                    stateless_progress.as_ref(),
                    3.0,
                    5.0,
                    "Confirmation required",
                )
                .await;
                match bridge.confirm(bridge_request).await {
                    Ok(NativeConfirmationDecision::Allow) => {}
                    Ok(NativeConfirmationDecision::Deny) => {
                        return native_framework_outcome(
                            FrameworkError::PermissionDenied {
                                effect: effect_label(&plan.effect),
                                scope: "native confirmation denied".to_string(),
                            },
                            Some((plan, plan_for_event)),
                        );
                    }
                    Ok(NativeConfirmationDecision::Canceled) => {
                        return native_framework_outcome(
                            FrameworkError::ConfirmationCanceled {
                                operation_id: plan.operation_id.clone(),
                            },
                            Some((plan, plan_for_event)),
                        );
                    }
                    Err(_) => {
                        return native_framework_outcome(
                            FrameworkError::ConfirmationFailed {
                                operation_id: plan.operation_id.clone(),
                            },
                            Some((plan, plan_for_event)),
                        );
                    }
                }
            }
        }

        Self::notify_native_progress(
            progress_meta,
            client.as_ref(),
            stateless_progress.as_ref(),
            4.0,
            5.0,
            "Dispatching command handler",
        )
        .await;
        let outcome = match self
            .registry
            .dispatch_prepared_operation(selected_arguments, prepared)
            .await
        {
            Ok(crate::CommandExecutionOutcome::Success(response)) => {
                native_success_outcome(self, surface, &operation_id, response, plan_for_event)
            }
            Ok(crate::CommandExecutionOutcome::ApplicationError { plan, error }) => {
                native_application_error_outcome(surface, plan, error, plan_for_event)
            }
            Err(error) => native_framework_outcome(error, Some((plan, plan_for_event))),
        };
        Self::notify_native_progress(
            progress_meta,
            client.as_ref(),
            stateless_progress.as_ref(),
            5.0,
            5.0,
            "Command complete",
        )
        .await;
        outcome
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
        // calls may retain it on `CallToolRequestParams.meta`. Progress uses
        // the params representation when it owns a token, then falls back to
        // the transport-owned context representation.
        let progress_meta = progress_meta(request_meta.as_ref(), &context_meta);
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
        if let Some(meta) = progress_meta {
            Self::notify_progress(meta, &client, 1.0, 4.0, "Parsing command template").await;
        }
        let effect_surface = match &self.surface {
            McpToolSurface::EffectLanes(surface) => surface,
            McpToolSurface::Native(_) => {
                return RunOutcome::envelope(
                    ResponseEnvelope::framework_error(
                        FrameworkError::Build(
                            "native surface entered the effect-lane execution path".to_string(),
                        ),
                        Some(request),
                        None,
                    ),
                    None,
                );
            }
        };
        let mut plan = match registry.build_effect_lane_plan(
            &request,
            &resolved,
            &invocation_context,
            effect_surface.identity(),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                return RunOutcome::envelope(
                    ResponseEnvelope::framework_error(error, Some(request), None),
                    None,
                );
            }
        };

        if let Some(meta) = progress_meta {
            Self::notify_progress(meta, &client, 2.0, 4.0, "Invocation plan ready").await;
        }
        let Some(lane) = registry.tool_lane(&config.execution_tool_name, &tool_name) else {
            let plan_for_event = PlanFacts::from(&plan);
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
            let plan_for_event = PlanFacts::from(&plan);
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

        if let Err(error) =
            registry.bind_effect_lane_presentation_fingerprint(&mut plan, &tool_name)
        {
            let plan_for_event = PlanFacts::from(&plan);
            return RunOutcome::envelope(
                ResponseEnvelope::framework_error(error, Some(request), Some(plan)),
                Some(plan_for_event),
            );
        }
        let plan_for_event = PlanFacts::from(&plan);

        if matches!(mode, RunMode::Preview) {
            let prepared_confirmation = match authorizer.decide(&plan) {
                PermissionDecision::Allow => None,
                PermissionDecision::RequireConfirmation => {
                    let confirmation = match registry
                        .prepare_effect_lane_confirmation(&plan, &tool_name)
                    {
                        Ok(confirmation) => confirmation,
                        Err(error) => {
                            return RunOutcome::envelope(
                                ResponseEnvelope::framework_error(error, Some(request), Some(plan)),
                                Some(plan_for_event),
                            );
                        }
                    };
                    Some(confirmation)
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
            };
            if let Some(meta) = progress_meta {
                Self::notify_progress(meta, &client, 4.0, 4.0, "Preview ready").await;
            }
            let envelope = if let Some(confirmation) = prepared_confirmation {
                ResponseEnvelope::preview_with_confirmation(plan, confirmation)
            } else {
                ResponseEnvelope::preview(plan, false)
            };
            return RunOutcome::envelope(envelope, Some(plan_for_event));
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
                    let confirmation = match registry
                        .prepare_effect_lane_confirmation(&plan, &tool_name)
                    {
                        Ok(confirmation) => confirmation,
                        Err(error) => {
                            return RunOutcome::envelope(
                                ResponseEnvelope::framework_error(error, Some(request), Some(plan)),
                                Some(plan_for_event),
                            );
                        }
                    };
                    let record =
                        issue_replay_record(config, replay, &plan, Utc::now().timestamp_millis())
                            .await;
                    if let Some(meta) = progress_meta {
                        Self::notify_progress(meta, &client, 4.0, 4.0, "Confirmation required")
                            .await;
                    }
                    return RunOutcome::envelope(
                        ResponseEnvelope::permission_required_with_confirmation(
                            plan,
                            record,
                            request,
                            tool_name,
                            confirmation,
                        ),
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

        if let Some(meta) = progress_meta {
            Self::notify_progress(meta, &client, 3.0, 4.0, "Dispatching command handler").await;
        }
        let result = registry
            .dispatch_prepared_plan_with_context(request.clone(), plan.clone(), &invocation_context)
            .await;
        match result {
            Ok(crate::CommandExecutionOutcome::Success(response)) => {
                if let Some(meta) = progress_meta {
                    Self::notify_progress(meta, &client, 4.0, 4.0, "Command complete").await;
                }
                RunOutcome::output(
                    ResponseEnvelope::success(response, profile),
                    Some(plan_for_event),
                )
            }
            Ok(crate::CommandExecutionOutcome::ApplicationError { plan, error }) => {
                if let Some(meta) = progress_meta {
                    Self::notify_progress(meta, &client, 4.0, 4.0, "Command complete").await;
                }
                RunOutcome::output(
                    ResponseEnvelope::application_error(plan, error, profile),
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
    /// Server-declared roots are included as the lowest-authority observation;
    /// a present higher-authority observation can block that fall-through.
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

fn effect_lane_help_tool() -> Tool {
    Tool::new(
        "help",
        "Return consistent help for the server or a CLI-shaped command.",
        schema_for_type::<HelpRequest>(),
    )
    .with_execution(ToolExecution::new().with_task_support(TaskSupport::Forbidden))
    .annotate(
        ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
    )
}

fn effect_lane_tools(
    registry: &CommandRegistry,
    config: &CliMcpServerConfig,
) -> crate::Result<Vec<Tool>> {
    let mut tools = vec![effect_lane_help_tool()];
    for lane in registry.lane_specs(&config.execution_tool_name) {
        let support = registry.lane_task_support(lane.lane, &config.execution_tool_name)?;
        tools.push(
            Tool::new(
                lane.tool_name.clone(),
                lane.description,
                schema_for_type::<RunRequest>(),
            )
            .with_execution(ToolExecution::new().with_task_support(rmcp_task_support(&support)))
            .annotate(annotations_for_lane(lane.lane, &lane.tool_name)),
        );
    }
    Ok(tools)
}

fn effect_lane_presentation_defaults(
    registry: &CommandRegistry,
    config: &CliMcpServerConfig,
) -> crate::Result<BTreeMap<String, SurfacePresentationDefaults>> {
    registry
        .lane_specs(&config.execution_tool_name)
        .into_iter()
        .map(|lane| {
            let display_title = format!("{} execution", lane.tool_name);
            Ok((
                lane.tool_name,
                SurfacePresentationDefaults::new(
                    format!("Running {display_title}"),
                    "Confirmation required",
                    format!("Run {display_title}?"),
                )?,
            ))
        })
        .collect()
}

fn effect_lane_instructions(registry: &CommandRegistry, config: &CliMcpServerConfig) -> String {
    let mut instructions = String::new();
    if let Some(preamble) = registry.preamble() {
        instructions.push_str(preamble);
        instructions.push_str("\n\n");
    }
    instructions.push_str(&format!(
        "Use `help` to discover command templates. Start execution with `{}`; the framework returns structured retry data when another effect-lane tool is required. Command strings are typed templates, not shell programs.",
        config.execution_tool_name
    ));
    instructions
}

fn surface_identity(surface: &McpToolSurface) -> crate::Result<ServingSurfaceIdentity> {
    match surface {
        McpToolSurface::EffectLanes(surface) => Ok(surface.identity().clone()),
        McpToolSurface::Native(surface) => surface.identity(),
    }
}

fn rmcp_task_support(support: &TaskSupportSpec) -> TaskSupport {
    match support {
        TaskSupportSpec::Forbidden => TaskSupport::Forbidden,
        TaskSupportSpec::Optional => TaskSupport::Optional,
        TaskSupportSpec::Required => TaskSupport::Required,
    }
}

fn native_result(value: Value, is_error: bool, mut extra: Vec<Content>) -> CallToolResult {
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
    let mut content = vec![Content::text(text)];
    content.append(&mut extra);
    if is_error {
        CallToolResult::error(content)
    } else {
        let mut result = CallToolResult::structured(value);
        result.content = content;
        result
    }
}

fn native_framework_outcome(
    error: FrameworkError,
    planned: Option<(InvocationPlan, PlanFacts)>,
) -> NativeRunOutcome {
    let (plan, plan_facts) = planned
        .map(|(plan, facts)| (Some(plan), Some(facts)))
        .unwrap_or((None, None));
    let envelope = ResponseEnvelope::framework_error(error, None, plan);
    let value = native_framework_error_value(envelope.error.as_ref());
    NativeRunOutcome {
        result: native_result(value, true, Vec::new()),
        envelope,
        plan: plan_facts,
    }
}

fn native_framework_outcome_for_operation(
    error: FrameworkError,
    operation: &crate::OperationSpec,
) -> NativeRunOutcome {
    let envelope =
        ResponseEnvelope::framework_error_for_operation(error, &operation.id, &operation.path);
    let value = native_framework_error_value(envelope.error.as_ref());
    NativeRunOutcome {
        result: native_result(value, true, Vec::new()),
        envelope,
        plan: Some(PlanFacts {
            operation_id: operation.id.clone(),
            command_path: operation.path.clone(),
            effect: operation.effect.clone(),
            resource_binding_facts: Vec::new(),
        }),
    }
}

fn native_framework_error_value(error: Option<&crate::ErrorBody>) -> Value {
    let Some(error) = error else {
        return json!({ "code": "handler_failed", "message": "framework failure" });
    };
    let mut error = error.clone();
    let recovery_fields: &[&str] = match &error.code {
        crate::ErrorCode::CapabilityMissing => {
            error.message = "Required capability proof is missing".to_string();
            &["providers"]
        }
        crate::ErrorCode::CapabilityDenied => {
            error.message = "Capability proof was denied".to_string();
            &["providers"]
        }
        crate::ErrorCode::ResourceRefused => {
            error.message = "Resource reference was refused".to_string();
            &["recover", "enumerate", "establish"]
        }
        crate::ErrorCode::ResourceBindingMissing => &["establish"],
        _ => &[],
    };
    if let Some(details) = error.details.as_object_mut() {
        for field in recovery_fields {
            details.remove(*field);
        }
    }
    serde_json::to_value(error)
        .unwrap_or_else(|_| json!({ "code": "handler_failed", "message": "framework failure" }))
}

fn native_success_outcome(
    server: &CliMcpServer,
    surface: &NativeToolSurface,
    operation_id: &str,
    response: RunResponse,
    plan: PlanFacts,
) -> NativeRunOutcome {
    let mut value = response
        .output
        .as_ref()
        .and_then(|output| output.structured.clone())
        .unwrap_or_else(|| json!({}));
    if let Some(arguments) = surface
        .snapshot()
        .operation(operation_id)
        .and_then(|operation| operation.call().arguments())
        && let Err(error) = inject_native_call_arguments(&mut value, arguments)
    {
        return native_framework_outcome(error, Some((response.plan.clone(), plan)));
    }
    let envelope = ResponseEnvelope::success(response, ResponseProfile::CompactStructured);
    let links = server.resource_links(&envelope);
    NativeRunOutcome {
        result: native_result(value, false, links),
        envelope,
        plan: Some(plan),
    }
}

fn inject_native_call_arguments(
    value: &mut Value,
    arguments: &BTreeMap<String, Value>,
) -> crate::Result<()> {
    let object = value
        .as_object_mut()
        .ok_or(FrameworkError::ResultContractViolation {
            boundary: crate::ResultContractBoundary::Success,
            reason: crate::ResultContractReason::SchemaMismatch,
        })?;
    for (name, selected) in arguments {
        object.insert(name.clone(), selected.clone());
    }
    Ok(())
}

fn native_application_error_outcome(
    surface: &NativeToolSurface,
    plan: InvocationPlan,
    error: crate::ApplicationErrorBody,
    plan_facts: PlanFacts,
) -> NativeRunOutcome {
    let value = match surface.declaration().application_errors {
        NativeApplicationErrorDialect::Canonical => {
            let recoveries = error
                .recoveries
                .iter()
                .filter_map(|recovery| match recovery {
                    crate::ApplicationRecovery::Operation { operation_id } => surface
                        .snapshot()
                        .operation(operation_id)
                        .map(|operation| NativeApplicationRecovery::Tool {
                            tool: operation.call().tool().to_string(),
                            arguments: operation.call().arguments().cloned().unwrap_or_default(),
                        }),
                    crate::ApplicationRecovery::Action { code, summary } => {
                        Some(NativeApplicationRecovery::Action {
                            code: code.clone(),
                            summary: summary.clone(),
                        })
                    }
                })
                .collect();
            serde_json::to_value(NativeApplicationErrorBody {
                code: error.code.clone(),
                message: error.message.clone(),
                details: error.details.clone(),
                recoveries,
            })
            .unwrap_or_else(|_| json!({}))
        }
        NativeApplicationErrorDialect::FlatSingleRecovery => {
            let recovery = error
                .recoveries
                .first()
                .and_then(|recovery| match recovery {
                    crate::ApplicationRecovery::Operation { operation_id } => surface
                        .snapshot()
                        .operation(operation_id)
                        .map(|operation| operation.call().tool().to_string()),
                    crate::ApplicationRecovery::Action { code, .. } => Some(code.clone()),
                });
            serde_json::to_value(crate::FlatNativeApplicationErrorBody {
                code: error.code.clone(),
                message: error.message.clone(),
                recovery,
            })
            .unwrap_or_else(|_| json!({}))
        }
    };
    let envelope =
        ResponseEnvelope::application_error(plan, error, ResponseProfile::CompactStructured);
    NativeRunOutcome {
        result: native_result(value, true, Vec::new()),
        envelope,
        plan: Some(plan_facts),
    }
}

impl CliMcpServer {
    fn task_profile(&self) -> Option<SemanticTaskProfile> {
        match &self.surface {
            McpToolSurface::EffectLanes(_) => Some(SemanticTaskProfile::Legacy2025_11_25),
            McpToolSurface::Native(surface) => match surface.snapshot().task_delivery() {
                CompiledTaskDelivery::Disabled => None,
                CompiledTaskDelivery::Legacy2025_11_25(_) => {
                    Some(SemanticTaskProfile::Legacy2025_11_25)
                }
                CompiledTaskDelivery::TasksExtension(_) => {
                    Some(SemanticTaskProfile::TasksExtension)
                }
            },
        }
    }

    fn task_surface_hash(&self) -> &str {
        match &self.surface {
            McpToolSurface::EffectLanes(surface) => &surface.identity().hash,
            McpToolSurface::Native(surface) => surface.snapshot().surface_hash(),
        }
    }

    fn task_runtime(&self) -> std::result::Result<&TaskRuntime, rmcp::ErrorData> {
        self.task_runtime.as_ref().ok_or_else(|| {
            rmcp::ErrorData::new(ErrorCode::METHOD_NOT_FOUND, "Method not found", None)
        })
    }

    fn task_access_scope(
        runtime: &TaskRuntime,
        extensions: &Extensions,
    ) -> std::result::Result<Option<TaskAccessScope>, crate::TaskAccessScopeError> {
        match &runtime.access {
            TaskAccessPolicy::CapabilityId => Ok(None),
            TaskAccessPolicy::Scoped(provider) => {
                provider.scope(TaskAccessContext::new(extensions)).map(Some)
            }
        }
    }

    async fn load_task(
        &self,
        task_id: &str,
        extensions: &Extensions,
    ) -> std::result::Result<LoadedTask, rmcp::ErrorData> {
        let runtime = self.task_runtime()?;
        let scope = Self::task_access_scope(runtime, extensions).map_err(|_| unknown_task())?;
        let key = derive_task_storage_key(
            self.task_surface_hash(),
            &runtime.access,
            task_id,
            scope.as_ref(),
        )
        .ok_or_else(unknown_task)?;
        let expected_scope = scope.as_ref().map(scope_hex);
        let expected_access_tag = u8::from(matches!(runtime.access, TaskAccessPolicy::Scoped(_)));
        loop {
            let Some(stored) = runtime
                .store
                .get(key)
                .await
                .map_err(|_| task_storage_failed())?
            else {
                return Err(unknown_task());
            };
            let semantic = stored.decode().map_err(|_| task_storage_failed())?;
            if semantic.task_id() != task_id
                || semantic.surface_hash() != self.task_surface_hash()
                || semantic.access_tag() != expected_access_tag
                || semantic.scope() != expected_scope.as_deref()
                || self.task_profile() != Some(semantic.profile())
            {
                return Err(task_storage_failed());
            }
            if semantic
                .expires_at()
                .is_some_and(|deadline| deadline <= unix_millis())
            {
                let _ = runtime.store.remove(key).await;
                return Err(unknown_task());
            }
            if semantic.status() == SemanticTaskStatus::Working
                && !self.live_tasks.lock().await.contains_key(task_id)
            {
                let orphan = semantic.successor(
                    SemanticTaskStatus::Failed,
                    Some(task_execution_failed_outcome()),
                    unix_millis(),
                );
                let encoded =
                    StoredTaskRecord::encode(&orphan).map_err(|_| task_storage_failed())?;
                match runtime
                    .store
                    .compare_and_set(key, stored.revision(), encoded.clone())
                    .await
                    .map_err(|_| task_storage_failed())?
                {
                    TaskStoreWrite::Written => {
                        return Ok(LoadedTask {
                            key,
                            stored: encoded,
                            semantic: orphan,
                        });
                    }
                    TaskStoreWrite::Conflict => continue,
                    TaskStoreWrite::Missing => return Err(unknown_task()),
                }
            }
            return Ok(LoadedTask {
                key,
                stored,
                semantic,
            });
        }
    }

    pub(crate) fn stateless_compatible(&self) -> bool {
        matches!(
            &self.surface,
            McpToolSurface::Native(surface)
                if surface.snapshot().protocol_version()
                    == McpProtocolTarget::V2026_07_28.as_str()
        )
    }

    #[cfg(test)]
    pub(crate) async fn hold_live_tasks_for_test(
        &self,
        acquired: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    ) {
        let _guard = self.live_tasks.lock().await;
        let _ = acquired.send(());
        let _ = release.await;
    }

    pub(crate) fn stateless_tasks_extension_enabled(&self) -> bool {
        self.task_profile() == Some(SemanticTaskProfile::TasksExtension)
    }

    pub(crate) async fn dispatch_stateless(
        &self,
        method: &str,
        params: serde_json::Map<String, Value>,
        extensions: &Extensions,
        progress: Option<mpsc::Sender<Value>>,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        if matches!(method, "tasks/get" | "tasks/update" | "tasks/cancel")
            && !self.stateless_tasks_extension_enabled()
        {
            return Err(crate::stateless::StatelessDispatchError::method_not_found());
        }
        match method {
            "server/discover" => Ok(self.stateless_discover()),
            "tools/list" => serde_json::to_value(ListToolsResult::with_all_items(self.tools()))
                .map_err(|_| {
                    crate::stateless::StatelessDispatchError::internal("Serialization failed")
                }),
            "resources/list" => serde_json::to_value(ListResourcesResult::with_all_items(
                self.resources()
                    .into_iter()
                    .map(|resource| resource.no_annotation())
                    .collect(),
            ))
            .map_err(|_| {
                crate::stateless::StatelessDispatchError::internal("Serialization failed")
            }),
            "prompts/list" => serde_json::to_value(ListPromptsResult::with_all_items(vec![
                rmcp::model::Prompt::new("getting_started", Some("How to use MCP Twill"), None),
            ]))
            .map_err(|_| {
                crate::stateless::StatelessDispatchError::internal("Serialization failed")
            }),
            "tools/call" => self.stateless_call_tool(params, extensions, progress).await,
            "resources/read" => self.stateless_read_resource(params).await,
            "prompts/get" => self.stateless_get_prompt(params),
            "tasks/get" => self.stateless_get_task(params, extensions).await,
            "tasks/update" => self.stateless_update_task(params, extensions).await,
            "tasks/cancel" => self.stateless_cancel_task(params, extensions).await,
            _ => Err(crate::stateless::StatelessDispatchError::method_not_found()),
        }
    }

    fn stateless_discover(&self) -> Value {
        let mut capabilities = serde_json::Map::from_iter([
            ("tools".to_string(), json!({})),
            ("resources".to_string(), json!({})),
            ("prompts".to_string(), json!({})),
        ]);
        if matches!(
            &self.surface,
            McpToolSurface::Native(surface)
                if matches!(surface.snapshot().task_delivery(), CompiledTaskDelivery::TasksExtension(_))
        ) {
            capabilities.insert(
                "extensions".to_string(),
                json!({ "io.modelcontextprotocol/tasks": {} }),
            );
        }
        let instructions = match &self.surface {
            McpToolSurface::Native(surface) => surface.snapshot().server_instructions(),
            McpToolSurface::EffectLanes(surface) => surface.instructions(),
        };
        json!({
            "resultType": "complete",
            "supportedVersions": [McpProtocolTarget::V2026_07_28.as_str()],
            "capabilities": capabilities,
            "serverInfo": {
                "name": self.registry.server_name(),
                "title": "MCP Twill",
                "version": env!("CARGO_PKG_VERSION"),
                "description": self.registry.server_description(),
            },
            "instructions": instructions,
        })
    }

    async fn stateless_call_tool(
        &self,
        mut params: serde_json::Map<String, Value>,
        extensions: &Extensions,
        progress: Option<mpsc::Sender<Value>>,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        let meta_value = params.remove("_meta").unwrap_or_else(|| json!({}));
        let capability = extension_capability(&meta_value)?;
        let meta = serde_json::from_value::<Meta>(meta_value).map_err(|_| {
            crate::stateless::StatelessDispatchError::invalid_params("Invalid params")
        })?;
        let name = params
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| {
                crate::stateless::StatelessDispatchError::invalid_params("Invalid params")
            })?;
        let arguments = params
            .remove("arguments")
            .unwrap_or_else(|| json!({}))
            .as_object()
            .cloned()
            .ok_or_else(|| {
                crate::stateless::StatelessDispatchError::invalid_params("Invalid params")
            })?;
        let surface = match &self.surface {
            McpToolSurface::Native(surface) => surface,
            McpToolSurface::EffectLanes(_) => {
                return Err(crate::stateless::StatelessDispatchError::method_not_found());
            }
        };
        if matches!(
            &surface.declaration().framework_help,
            crate::FrameworkHelpProjection::Tool { name: help_name } if help_name == &name
        ) {
            let request =
                serde_json::from_value::<HelpRequest>(Value::Object(arguments)).map_err(|_| {
                    crate::stateless::StatelessDispatchError::invalid_params("Invalid params")
                })?;
            return complete_tool_result(help_result(surface.help(request)));
        }
        let support = surface.task_support_for_tool(&name).ok_or_else(|| {
            crate::stateless::StatelessDispatchError::invalid_params("Unknown tool")
        })?;
        let delivery = surface.snapshot().task_delivery();
        let defer = match (delivery, support) {
            (_, TaskSupportSpec::Forbidden) => false,
            (CompiledTaskDelivery::Disabled, TaskSupportSpec::Optional) => false,
            (CompiledTaskDelivery::Disabled, TaskSupportSpec::Required) => {
                return Err(crate::stateless::StatelessDispatchError::missing_capability());
            }
            (CompiledTaskDelivery::TasksExtension(delivery), TaskSupportSpec::Optional) => {
                capability
                    && matches!(
                        delivery.optional_policy(),
                        crate::ExtensionOptionalPolicy::DeferredWhenAvailable
                    )
            }
            (CompiledTaskDelivery::TasksExtension(_), TaskSupportSpec::Required) => {
                if !capability {
                    return Err(crate::stateless::StatelessDispatchError::missing_capability());
                }
                true
            }
            (CompiledTaskDelivery::Legacy2025_11_25(_), _) => {
                return Err(crate::stateless::StatelessDispatchError::method_not_found());
            }
        };
        if !defer {
            let result = self
                .clone()
                .execute_native_tool_stateless(name, Some(meta), arguments, progress)
                .await;
            return complete_tool_result(result);
        }
        let retention_ms = match delivery {
            CompiledTaskDelivery::TasksExtension(delivery) => delivery.retention_ms(),
            _ => unreachable!("deferred extension calls require extension delivery"),
        };
        self.create_extension_task(name, arguments, meta, extensions, retention_ms, progress)
            .await
    }

    async fn create_extension_task(
        &self,
        name: String,
        arguments: rmcp::model::JsonObject,
        meta: Meta,
        extensions: &Extensions,
        retention_ms: u64,
        progress: Option<mpsc::Sender<Value>>,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        let runtime = self.task_runtime.as_ref().ok_or_else(|| {
            crate::stateless::StatelessDispatchError::internal("Task creation failed")
        })?;
        let scope = Self::task_access_scope(runtime, extensions).map_err(|_| {
            crate::stateless::StatelessDispatchError::internal("Task access scope unavailable")
        })?;
        let access_tag = u8::from(matches!(runtime.access, TaskAccessPolicy::Scoped(_)));
        let scope_value = scope.as_ref().map(scope_hex);
        let now = unix_millis();
        let surface_hash = self.task_surface_hash().to_string();
        let mut live_task_guard = self.live_tasks.lock().await;
        let mut created = None;
        for _ in 0..8 {
            let task_id = generate_task_id();
            let key =
                derive_task_storage_key(&surface_hash, &runtime.access, &task_id, scope.as_ref())
                    .ok_or_else(|| {
                    crate::stateless::StatelessDispatchError::internal("Task creation failed")
                })?;
            let semantic = SemanticTaskRecord::working(
                task_id.clone(),
                surface_hash.clone(),
                access_tag,
                scope_value.clone(),
                SemanticTaskProfile::TasksExtension,
                now,
                Some(checked_task_expiration(now, retention_ms).ok_or_else(|| {
                    crate::stateless::StatelessDispatchError::internal("Task creation failed")
                })?),
            );
            let stored = StoredTaskRecord::encode(&semantic).map_err(|_| {
                crate::stateless::StatelessDispatchError::internal("Task creation failed")
            })?;
            match runtime
                .store
                .create(key, stored.clone())
                .await
                .map_err(|_| {
                    crate::stateless::StatelessDispatchError::internal("Task creation failed")
                })? {
                TaskStoreCreate::Created => {
                    created = Some((task_id, key, semantic, stored));
                    break;
                }
                TaskStoreCreate::Occupied => continue,
                TaskStoreCreate::CapacityExceeded => {
                    return Err(crate::stateless::StatelessDispatchError::internal(
                        "Task creation failed",
                    ));
                }
            }
        }
        let (task_id, key, semantic, stored) = created.ok_or_else(|| {
            crate::stateless::StatelessDispatchError::internal("Task creation failed")
        })?;
        let creation = extension_task_handle(&semantic);
        let runtime = runtime.clone();
        let server = self.clone();
        let live_tasks = self.live_tasks.clone();
        let task_id_for_runner = task_id.clone();
        let cancellation = TaskCancellation::default();
        let runner_cancellation = cancellation.clone();
        let (start, ready) = tokio::sync::oneshot::channel();
        let runner_runtime = runtime.clone();
        let runner_stored = stored.clone();
        let runner_semantic = semantic.clone();
        let handle = tokio::spawn(async move {
            let _ = ready.await;
            let (status, value) = tokio::select! {
                result = server.execute_native_tool_stateless(name, Some(meta), arguments, progress) => {
                    match serde_json::to_value(result) {
                        Ok(mut value) => {
                            if let Some(value) = value.as_object_mut() {
                                value.insert(
                                    "resultType".to_string(),
                                    Value::String("complete".to_string()),
                                );
                            }
                            (SemanticTaskStatus::Completed, value)
                        }
                        Err(_) => (SemanticTaskStatus::Failed, task_execution_failed_outcome()),
                    }
                }
                () = runner_cancellation.cancelled() => {
                    (SemanticTaskStatus::Cancelled, task_cancelled_outcome())
                }
            };
            commit_task_outcome(
                &runner_runtime,
                key,
                runner_stored,
                runner_semantic,
                status,
                value,
            )
            .await;
            live_tasks.lock().await.remove(&task_id_for_runner);
        });
        live_task_guard.insert(task_id.clone(), LiveTaskControl::Extension(cancellation));
        drop(live_task_guard);
        drop(handle);
        schedule_task_expiration(
            runtime.clone(),
            self.live_tasks.clone(),
            task_id,
            key,
            semantic.expires_at(),
        );
        let _ = start.send(());
        Ok(creation)
    }

    async fn stateless_get_task(
        &self,
        mut params: serde_json::Map<String, Value>,
        extensions: &Extensions,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        require_extension_capability(params.get("_meta"))?;
        let task_id = take_task_id(&mut params)?;
        let loaded = self
            .load_task(&task_id, extensions)
            .await
            .map_err(map_task_error)?;
        Ok(extension_task_result(&loaded.semantic))
    }

    async fn stateless_update_task(
        &self,
        mut params: serde_json::Map<String, Value>,
        extensions: &Extensions,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        require_extension_capability(params.get("_meta"))?;
        let task_id = take_task_id(&mut params)?;
        self.load_task(&task_id, extensions)
            .await
            .map_err(map_task_error)?;
        Ok(json!({ "resultType": "complete" }))
    }

    async fn stateless_cancel_task(
        &self,
        mut params: serde_json::Map<String, Value>,
        extensions: &Extensions,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        require_extension_capability(params.get("_meta"))?;
        let task_id = take_task_id(&mut params)?;
        self.load_task(&task_id, extensions)
            .await
            .map_err(map_task_error)?;
        if let Some(LiveTaskControl::Extension(cancellation)) =
            self.live_tasks.lock().await.get(&task_id).cloned()
        {
            cancellation.cancel();
        }
        Ok(json!({ "resultType": "complete" }))
    }

    async fn stateless_read_resource(
        &self,
        mut params: serde_json::Map<String, Value>,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        let uri = params
            .remove("uri")
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| {
                crate::stateless::StatelessDispatchError::invalid_params("Invalid params")
            })?;
        if let Some((decl, id)) = self.registry.match_resource_uri(&uri) {
            let reader = self.registry.resource_reader(&decl.name).ok_or_else(|| {
                crate::stateless::StatelessDispatchError::invalid_params("Resource is not readable")
            })?;
            let value = reader.read_erased(&id).await.map_err(|_| {
                crate::stateless::StatelessDispatchError::invalid_params("Resource was refused")
            })?;
            return serde_json::to_value(ReadResourceResult::new(vec![
                ResourceContents::text(value.to_string(), uri).with_mime_type("application/json"),
            ]))
            .map_err(|_| {
                crate::stateless::StatelessDispatchError::internal("Serialization failed")
            });
        }
        let text = if uri == "cli://catalog" {
            self.catalog_resource_text()
        } else {
            self.registry.resource_text(&uri)
        }
        .ok_or_else(|| {
            crate::stateless::StatelessDispatchError::invalid_params("Unknown resource")
        })?;
        serde_json::to_value(ReadResourceResult::new(vec![
            ResourceContents::text(text, uri).with_mime_type("text/markdown"),
        ]))
        .map_err(|_| crate::stateless::StatelessDispatchError::internal("Serialization failed"))
    }

    fn stateless_get_prompt(
        &self,
        mut params: serde_json::Map<String, Value>,
    ) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
        let name = params
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| {
                crate::stateless::StatelessDispatchError::invalid_params("Invalid params")
            })?;
        if name != "getting_started" {
            return Err(crate::stateless::StatelessDispatchError::invalid_params(
                "Unknown prompt",
            ));
        }
        let text = match &self.surface {
            McpToolSurface::Native(surface) => surface.snapshot().server_instructions().to_string(),
            McpToolSurface::EffectLanes(_) => String::new(),
        };
        serde_json::to_value(GetPromptResult::new(vec![
            rmcp::model::PromptMessage::new_text(rmcp::model::PromptMessageRole::User, text),
        ]))
        .map_err(|_| crate::stateless::StatelessDispatchError::internal("Serialization failed"))
    }
}

impl ServerHandler for CliMcpServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = if self.task_profile() == Some(SemanticTaskProfile::Legacy2025_11_25) {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .enable_tasks_with(crate::native_surfaces::legacy_tasks_capability())
                .build()
        } else {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build()
        };
        let mut implementation =
            Implementation::new(self.registry.server_name(), env!("CARGO_PKG_VERSION"));
        implementation.title = Some("MCP Twill".to_string());
        implementation.description = Some(self.registry.server_description().to_string());

        let (instructions, protocol_version) = match &self.surface {
            McpToolSurface::EffectLanes(surface) => (
                surface.instructions().to_string(),
                ProtocolVersion::V_2025_11_25,
            ),
            McpToolSurface::Native(surface) => (
                surface.snapshot().server_instructions().to_string(),
                serde_json::from_value(json!(surface.snapshot().protocol_version()))
                    .expect("compiled protocol target is a protocol version"),
            ),
        };
        ServerInfo::new(capabilities)
            .with_server_info(implementation)
            .with_protocol_version(protocol_version)
            .with_instructions(instructions)
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, rmcp::ErrorData> {
        self.validate_protocol(
            request.as_ref().and_then(|request| request.meta.as_ref()),
            &context,
        )?;
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
        self.validate_protocol(request.meta.as_ref(), &context)?;
        let tool_name = request.name.to_string();
        let is_help = match &self.surface {
            McpToolSurface::EffectLanes(_) => tool_name == "help",
            McpToolSurface::Native(surface) => matches!(
                &surface.declaration().framework_help,
                crate::FrameworkHelpProjection::Tool { name } if name == &tool_name
            ),
        };
        if is_help {
            let help_request = Self::parse_arguments::<HelpRequest>(request.arguments)?;
            let help = match &self.surface {
                McpToolSurface::EffectLanes(_) => self.registry.help(help_request),
                McpToolSurface::Native(surface) => surface.help(help_request),
            };
            return Ok(help_result(help));
        }

        match &self.surface {
            McpToolSurface::EffectLanes(_) => {
                let Some(lane) = self
                    .registry
                    .tool_lane(&self.config.execution_tool_name, &tool_name)
                else {
                    return Err(rmcp::ErrorData::invalid_params(
                        format!("Unknown tool {tool_name}"),
                        None,
                    ));
                };
                let task_support = self
                    .registry
                    .lane_task_support(lane, &self.config.execution_tool_name)
                    .map_err(|_| {
                        rmcp::ErrorData::internal_error("Invalid effect-lane task support", None)
                    })?;
                if matches!(task_support, TaskSupportSpec::Required) {
                    return Err(rmcp::ErrorData::invalid_params(
                        format!("Tool {tool_name} requires task-augmented execution"),
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
            McpToolSurface::Native(surface) => {
                if surface
                    .snapshot()
                    .tools()
                    .iter()
                    .all(|tool| tool.name != tool_name)
                {
                    return Err(rmcp::ErrorData::invalid_params(
                        format!("Unknown tool {tool_name}"),
                        None,
                    ));
                }
                if let Some(task_support) = surface.task_support_for_tool(&tool_name) {
                    ensure_ordinary_task_support(&tool_name, task_support)?;
                }
                Ok(self
                    .clone()
                    .execute_native_tool(
                        tool_name,
                        request.meta.clone(),
                        context.meta.clone(),
                        context.peer.clone(),
                        request.arguments.unwrap_or_default(),
                    )
                    .await)
            }
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListResourcesResult, rmcp::ErrorData> {
        self.validate_protocol(
            request.as_ref().and_then(|request| request.meta.as_ref()),
            &context,
        )?;
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
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ReadResourceResult, rmcp::ErrorData> {
        self.validate_protocol(request.meta.as_ref(), &context)?;
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
        let text = if request.uri == "cli://lanes" || request.uri == "cli://server/overview" {
            if !matches!(self.surface, McpToolSurface::EffectLanes(_)) {
                return Err(rmcp::ErrorData::invalid_params(
                    format!("Unknown resource {}", request.uri),
                    None,
                ));
            }
            if request.uri == "cli://lanes" {
                lanes_text(&self.execution_lanes())
            } else {
                self.registry.resource_text(&request.uri).ok_or_else(|| {
                    rmcp::ErrorData::invalid_params(
                        format!("Unknown resource {}", request.uri),
                        None,
                    )
                })?
            }
        } else if request.uri == "cli://catalog" {
            self.catalog_resource_text().ok_or_else(|| {
                rmcp::ErrorData::internal_error("Cannot serialize active catalog", None)
            })?
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
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListPromptsResult, rmcp::ErrorData> {
        self.validate_protocol(
            request.as_ref().and_then(|request| request.meta.as_ref()),
            &context,
        )?;
        Ok(ListPromptsResult::with_all_items(vec![
            rmcp::model::Prompt::new("getting_started", Some("How to use MCP Twill"), None),
        ]))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<GetPromptResult, rmcp::ErrorData> {
        self.validate_protocol(request.meta.as_ref(), &context)?;
        if request.name != "getting_started" {
            return Err(rmcp::ErrorData::invalid_params(
                format!("Unknown prompt {}", request.name),
                None,
            ));
        }
        let text = match &self.surface {
            McpToolSurface::Native(surface) => {
                let mut text = surface.snapshot().server_instructions().to_string();
                text.push_str("\n\nCall the named MCP tools directly.");
                if let crate::FrameworkHelpProjection::Tool { name } =
                    &surface.declaration().framework_help
                {
                    text.push_str(&format!(" Use `{name}` for surface-filtered catalog help."));
                }
                text
            }
            McpToolSurface::EffectLanes(_) => {
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
                text
            }
        };
        Ok(GetPromptResult::new(vec![
            rmcp::model::PromptMessage::new_text(rmcp::model::PromptMessageRole::User, text),
        ]))
    }

    async fn enqueue_task(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CreateTaskResult, rmcp::ErrorData> {
        self.validate_protocol(request.meta.as_ref(), &context)?;
        self.ensure_tasks_supported()?;
        let tool_name = request.name.to_string();
        let task_support = match &self.surface {
            McpToolSurface::EffectLanes(_) => {
                let Some(lane) = self
                    .registry
                    .tool_lane(&self.config.execution_tool_name, &tool_name)
                else {
                    return Err(rmcp::ErrorData::invalid_params(
                        format!(
                            "Only execution tools support task-augmented execution: {tool_name}"
                        ),
                        None,
                    ));
                };
                self.registry
                    .lane_task_support(lane, &self.config.execution_tool_name)
                    .map_err(|_| {
                        rmcp::ErrorData::internal_error("Invalid effect-lane task support", None)
                    })?
            }
            McpToolSurface::Native(surface) => surface
                .task_support_for_tool(&tool_name)
                .ok_or_else(|| rmcp::ErrorData::invalid_params("Unknown tool", None))?,
        };
        if matches!(task_support, TaskSupportSpec::Forbidden) {
            return Err(rmcp::ErrorData::invalid_params(
                format!("Tool {tool_name} does not support task-augmented execution"),
                None,
            ));
        }
        let ttl = legacy_task_ttl(request.task.as_ref())?;
        let request_meta = request.meta.clone();
        let invocation = match &self.surface {
            McpToolSurface::EffectLanes(_) => {
                LegacyTaskInvocation::EffectLane(self.parse_run_request(request.arguments)?)
            }
            McpToolSurface::Native(_) => {
                LegacyTaskInvocation::Native(request.arguments.unwrap_or_default())
            }
        };
        let runtime = self.task_runtime()?.clone();
        let scope = Self::task_access_scope(&runtime, &context.extensions)
            .map_err(|_| task_access_scope_unavailable())?;
        let access_tag = u8::from(matches!(runtime.access, TaskAccessPolicy::Scoped(_)));
        let scope_value = scope.as_ref().map(scope_hex);
        let now = unix_millis();
        let expires_at = ttl
            .map(|ttl| {
                checked_task_expiration(now, ttl).ok_or_else(|| {
                    rmcp::ErrorData::invalid_params("Task ttl is outside the timestamp range", None)
                })
            })
            .transpose()?;
        let surface_hash = self.task_surface_hash().to_string();

        let (task_id, key, semantic, stored) = {
            let mut created = None;
            for _ in 0..8 {
                let task_id = generate_task_id();
                let key = derive_task_storage_key(
                    &surface_hash,
                    &runtime.access,
                    &task_id,
                    scope.as_ref(),
                )
                .ok_or_else(task_creation_failed)?;
                let semantic = SemanticTaskRecord::working(
                    task_id.clone(),
                    surface_hash.clone(),
                    access_tag,
                    scope_value.clone(),
                    SemanticTaskProfile::Legacy2025_11_25,
                    now,
                    expires_at,
                );
                let stored =
                    StoredTaskRecord::encode(&semantic).map_err(|_| task_creation_failed())?;
                match runtime
                    .store
                    .create(key, stored.clone())
                    .await
                    .map_err(|_| task_creation_failed())?
                {
                    TaskStoreCreate::Created => {
                        created = Some((task_id, key, semantic, stored));
                        break;
                    }
                    TaskStoreCreate::Occupied => continue,
                    TaskStoreCreate::CapacityExceeded => return Err(task_creation_failed()),
                }
            }
            created.ok_or_else(task_creation_failed)?
        };

        let task = legacy_task(&semantic);
        let server = self.clone();
        let live_tasks = self.live_tasks.clone();
        let task_id_for_runner = task_id.clone();
        let context_meta = context.meta.clone();
        let client = context.peer.clone();
        let (start, ready) = tokio::sync::oneshot::channel();
        let runner_runtime = runtime.clone();
        let runner_stored = stored.clone();
        let runner_semantic = semantic.clone();
        let handle = tokio::spawn(async move {
            let _ = ready.await;
            let mut result = match invocation {
                LegacyTaskInvocation::EffectLane(request) => {
                    server
                        .execute_run_tool(tool_name, request_meta, context_meta, client, request)
                        .await
                }
                LegacyTaskInvocation::Native(arguments) => {
                    server
                        .execute_native_tool(
                            tool_name,
                            request_meta,
                            context_meta,
                            client,
                            arguments,
                        )
                        .await
                }
            };
            let meta = result.meta.get_or_insert_with(Meta::default);
            meta.0.insert(
                "io.modelcontextprotocol/related-task".to_string(),
                json!({ "taskId": &task_id_for_runner }),
            );
            let (status, value) = match serde_json::to_value(&result) {
                Ok(value) => {
                    let status = if result.is_error.unwrap_or(false) {
                        SemanticTaskStatus::Failed
                    } else {
                        SemanticTaskStatus::Completed
                    };
                    (status, value)
                }
                Err(_) => (SemanticTaskStatus::Failed, task_execution_failed_outcome()),
            };
            commit_task_outcome(
                &runner_runtime,
                key,
                runner_stored,
                runner_semantic,
                status,
                value,
            )
            .await;
            live_tasks.lock().await.remove(&task_id_for_runner);
        });
        self.live_tasks.lock().await.insert(
            task_id.clone(),
            LiveTaskControl::Legacy(handle.abort_handle()),
        );
        schedule_task_expiration(
            runtime.clone(),
            self.live_tasks.clone(),
            task_id,
            key,
            semantic.expires_at(),
        );
        let _ = start.send(());

        Ok(CreateTaskResult::new(task))
    }

    async fn list_tasks(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListTasksResult, rmcp::ErrorData> {
        self.validate_protocol(
            request.as_ref().and_then(|request| request.meta.as_ref()),
            &context,
        )?;
        Err(rmcp::ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            "Method not found",
            None,
        ))
    }

    async fn get_task_info(
        &self,
        request: GetTaskInfoParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<GetTaskResult, rmcp::ErrorData> {
        self.validate_protocol(request.meta.as_ref(), &context)?;
        self.ensure_tasks_supported()?;
        let record = self
            .load_task(&request.task_id, &context.extensions)
            .await?;
        Ok(GetTaskResult {
            meta: None,
            task: legacy_task(&record.semantic),
        })
    }

    async fn get_task_result(
        &self,
        request: GetTaskResultParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<GetTaskPayloadResult, rmcp::ErrorData> {
        self.validate_protocol(request.meta.as_ref(), &context)?;
        self.ensure_tasks_supported()?;
        loop {
            let record = self
                .load_task(&request.task_id, &context.extensions)
                .await?;
            if let Some(payload) = record.semantic.outcome() {
                return legacy_task_payload(payload);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CancelTaskResult, rmcp::ErrorData> {
        self.validate_protocol(request.meta.as_ref(), &context)?;
        self.ensure_tasks_supported()?;
        let runtime = self.task_runtime()?.clone();
        loop {
            let loaded = self
                .load_task(&request.task_id, &context.extensions)
                .await?;
            if loaded.semantic.status() != SemanticTaskStatus::Working {
                return Err(rmcp::ErrorData::invalid_params(
                    "Task is not cancellable",
                    None,
                ));
            }
            let next = loaded.semantic.successor(
                SemanticTaskStatus::Cancelled,
                Some(task_cancelled_outcome()),
                unix_millis(),
            );
            let stored = StoredTaskRecord::encode(&next).map_err(|_| task_storage_failed())?;
            match runtime
                .store
                .compare_and_set(loaded.key, loaded.stored.revision(), stored)
                .await
                .map_err(|_| task_storage_failed())?
            {
                TaskStoreWrite::Written => {
                    if let Some(control) = self.live_tasks.lock().await.remove(&request.task_id) {
                        control.cancel();
                    }
                    return Ok(CancelTaskResult {
                        meta: None,
                        task: legacy_task(&next),
                    });
                }
                TaskStoreWrite::Conflict => continue,
                TaskStoreWrite::Missing => return Err(unknown_task()),
            }
        }
    }
}

fn legacy_task_ttl(
    task: Option<&rmcp::model::JsonObject>,
) -> std::result::Result<Option<u64>, rmcp::ErrorData> {
    let Some(value) = task.and_then(|task| task.get("ttl")) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let ttl = value.as_number().and_then(|number| {
        crate::JsonInteger::try_from_number(number.clone())
            .ok()?
            .as_u64()
    });
    ttl.map(Some).ok_or_else(|| {
        rmcp::ErrorData::invalid_params("Task ttl must be a non-negative integer or null", None)
    })
}

fn unix_millis() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0)
}

fn timestamp(value: u64) -> String {
    i64::try_from(value)
        .ok()
        .and_then(chrono::DateTime::<Utc>::from_timestamp_millis)
        .unwrap_or(chrono::DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn legacy_task(record: &SemanticTaskRecord) -> Task {
    let status = match record.status() {
        SemanticTaskStatus::Working => TaskStatus::Working,
        SemanticTaskStatus::Completed => TaskStatus::Completed,
        SemanticTaskStatus::Failed => TaskStatus::Failed,
        SemanticTaskStatus::Cancelled => TaskStatus::Cancelled,
    };
    let message = match record.status() {
        SemanticTaskStatus::Working => "Task is running",
        SemanticTaskStatus::Completed => "Task completed",
        SemanticTaskStatus::Failed => "Task failed",
        SemanticTaskStatus::Cancelled => "Task cancelled",
    };
    let mut task = Task::new(
        record.task_id().to_string(),
        status,
        timestamp(record.created_at()),
        timestamp(record.updated_at()),
    )
    .with_status_message(message)
    .with_poll_interval(100);
    if let Some(expiration) = record.expires_at() {
        task.ttl = Some(expiration.saturating_sub(record.created_at()));
    }
    task
}

async fn commit_task_outcome(
    runtime: &TaskRuntime,
    key: crate::TaskStorageKey,
    mut stored: StoredTaskRecord,
    mut current: SemanticTaskRecord,
    status: SemanticTaskStatus,
    outcome: Value,
) {
    loop {
        if current.status() != SemanticTaskStatus::Working {
            return;
        }
        let mut next = current.successor(status, Some(outcome.clone()), unix_millis());
        let mut encoded = StoredTaskRecord::encode(&next);
        if encoded.is_err() {
            next = current.successor(
                SemanticTaskStatus::Failed,
                Some(task_execution_failed_outcome()),
                unix_millis(),
            );
            encoded = StoredTaskRecord::encode(&next);
        }
        let Ok(encoded) = encoded else {
            return;
        };
        match runtime
            .store
            .compare_and_set(key, stored.revision(), encoded)
            .await
        {
            Ok(TaskStoreWrite::Written | TaskStoreWrite::Missing) => return,
            Ok(TaskStoreWrite::Conflict) => match runtime.store.get(key).await {
                Ok(Some(reloaded)) => match reloaded.decode() {
                    Ok(record) => {
                        stored = reloaded;
                        current = record;
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                },
                Ok(None) => return,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            },
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
}

fn schedule_task_expiration(
    runtime: TaskRuntime,
    live_tasks: Arc<Mutex<BTreeMap<String, LiveTaskControl>>>,
    task_id: String,
    key: crate::TaskStorageKey,
    deadline: Option<u64>,
) {
    let Some(deadline) = deadline else {
        return;
    };
    tokio::spawn(async move {
        let delay = deadline.saturating_sub(unix_millis());
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        loop {
            match runtime.store.remove(key).await {
                Ok(_) => {
                    if let Some(control) = live_tasks.lock().await.remove(&task_id) {
                        control.cancel();
                    }
                    return;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            }
        }
    });
}

fn task_execution_failed_outcome() -> Value {
    json!({
        "jsonrpc": "2.0",
        "error": { "code": -32603, "message": "Task execution failed" }
    })
}

fn task_cancelled_outcome() -> Value {
    json!({
        "jsonrpc": "2.0",
        "error": { "code": -32000, "message": "Task cancelled" }
    })
}

fn unknown_task() -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params("Unknown task", None)
}

fn task_creation_failed() -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error("Task creation failed", None)
}

fn task_access_scope_unavailable() -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error("Task access scope unavailable", None)
}

fn task_storage_failed() -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error("Task storage failed", None)
}

fn legacy_task_payload(
    outcome: &Value,
) -> std::result::Result<GetTaskPayloadResult, rmcp::ErrorData> {
    let Some(error) = outcome.get("error").and_then(Value::as_object) else {
        return Ok(GetTaskPayloadResult::new(outcome.clone()));
    };
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok())
        .ok_or_else(task_storage_failed)?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(task_storage_failed)?;
    Err(rmcp::ErrorData::new(
        ErrorCode(code),
        message.to_string(),
        error.get("data").cloned(),
    ))
}

fn complete_tool_result(
    result: CallToolResult,
) -> std::result::Result<Value, crate::stateless::StatelessDispatchError> {
    let mut value = serde_json::to_value(result)
        .map_err(|_| crate::stateless::StatelessDispatchError::internal("Serialization failed"))?;
    let object = value.as_object_mut().ok_or_else(|| {
        crate::stateless::StatelessDispatchError::internal("Serialization failed")
    })?;
    object.insert(
        "resultType".to_string(),
        Value::String("complete".to_string()),
    );
    Ok(value)
}

fn extension_task_handle(record: &SemanticTaskRecord) -> Value {
    extension_task(record, "task")
}

fn extension_task_result(record: &SemanticTaskRecord) -> Value {
    extension_task(record, "complete")
}

fn extension_task(record: &SemanticTaskRecord, result_type: &'static str) -> Value {
    let status = match record.status() {
        SemanticTaskStatus::Working => "working",
        SemanticTaskStatus::Completed => "completed",
        SemanticTaskStatus::Failed => "failed",
        SemanticTaskStatus::Cancelled => "cancelled",
    };
    let message = match record.status() {
        SemanticTaskStatus::Working => "Task is running",
        SemanticTaskStatus::Completed => "Task completed",
        SemanticTaskStatus::Failed => "Task failed",
        SemanticTaskStatus::Cancelled => "Task cancelled",
    };
    let mut value = json!({
        "resultType": result_type,
        "taskId": record.task_id(),
        "status": status,
        "statusMessage": message,
        "createdAt": timestamp(record.created_at()),
        "lastUpdatedAt": timestamp(record.updated_at()),
        "ttlMs": record.expires_at().map(|expiration| expiration.saturating_sub(record.created_at())),
        "pollIntervalMs": 100,
    });
    if let Some(outcome) = record.outcome() {
        let projection = match record.status() {
            SemanticTaskStatus::Completed => Some(("result", outcome.clone())),
            SemanticTaskStatus::Failed => Some((
                "error",
                outcome
                    .get("error")
                    .cloned()
                    .expect("validated extension failure has an error object"),
            )),
            SemanticTaskStatus::Working | SemanticTaskStatus::Cancelled => None,
        };
        if let Some((field, outcome)) = projection {
            value
                .as_object_mut()
                .expect("extension task projection is an object")
                .insert(field.to_string(), outcome);
        }
    }
    value
}

fn extension_capability(
    meta: &Value,
) -> std::result::Result<bool, crate::stateless::StatelessDispatchError> {
    let capability = meta
        .as_object()
        .and_then(|meta| meta.get("io.modelcontextprotocol/clientCapabilities"))
        .and_then(Value::as_object)
        .and_then(|capabilities| capabilities.get("extensions"))
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("io.modelcontextprotocol/tasks"));
    let Some(capability) = capability else {
        return Ok(false);
    };
    match capability.as_object() {
        Some(capability) if capability.is_empty() => Ok(true),
        _ => Err(crate::stateless::StatelessDispatchError::invalid_params(
            "Invalid Tasks Extension capability",
        )),
    }
}

fn require_extension_capability(
    meta: Option<&Value>,
) -> std::result::Result<(), crate::stateless::StatelessDispatchError> {
    if extension_capability(meta.unwrap_or(&Value::Null))? {
        Ok(())
    } else {
        Err(crate::stateless::StatelessDispatchError::missing_capability())
    }
}

fn take_task_id(
    params: &mut serde_json::Map<String, Value>,
) -> std::result::Result<String, crate::stateless::StatelessDispatchError> {
    params
        .remove("taskId")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| crate::stateless::StatelessDispatchError::invalid_params("Invalid params"))
}

fn map_task_error(error: rmcp::ErrorData) -> crate::stateless::StatelessDispatchError {
    match error.code {
        ErrorCode::INTERNAL_ERROR => {
            crate::stateless::StatelessDispatchError::internal("Task storage failed")
        }
        _ => crate::stateless::StatelessDispatchError::invalid_params("Unknown task"),
    }
}

fn ensure_ordinary_task_support(
    tool_name: &str,
    task_support: TaskSupportSpec,
) -> std::result::Result<(), rmcp::ErrorData> {
    if matches!(task_support, TaskSupportSpec::Required) {
        Err(rmcp::ErrorData::invalid_params(
            format!("Tool {tool_name} requires task-augmented execution"),
            None,
        ))
    } else {
        Ok(())
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
    if envelope.error.is_some() {
        return envelope_result(envelope);
    }
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

    #[test]
    fn stateless_task_errors_map_by_json_rpc_code() {
        let storage = map_task_error(rmcp::ErrorData::internal_error("renamed detail", None));
        assert_eq!(storage.code, ErrorCode::INTERNAL_ERROR.0);
        assert_eq!(storage.message, "Task storage failed");

        let unknown = map_task_error(rmcp::ErrorData::invalid_params("Task storage failed", None));
        assert_eq!(unknown.code, ErrorCode::INVALID_PARAMS.0);
        assert_eq!(unknown.message, "Unknown task");

        let unexpected = map_task_error(rmcp::ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            "private detail",
            None,
        ));
        assert_eq!(unexpected.code, ErrorCode::INVALID_PARAMS.0);
        assert_eq!(unexpected.message, "Unknown task");
    }

    #[test]
    fn extension_task_projection_separates_handles_results_and_errors() {
        let working = SemanticTaskRecord::working(
            "11".repeat(32),
            "22".repeat(32),
            0,
            None,
            SemanticTaskProfile::TasksExtension,
            1,
            Some(1_001),
        );
        assert_eq!(extension_task_handle(&working)["resultType"], "task");
        assert_eq!(extension_task_result(&working)["resultType"], "complete");

        let failed = working.successor(
            SemanticTaskStatus::Failed,
            Some(task_execution_failed_outcome()),
            2,
        );
        let failed = extension_task_result(&failed);
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"]["code"], -32603);
        assert_eq!(failed["error"]["message"], "Task execution failed");
        assert!(failed["error"].get("error").is_none());

        let cancelled = working.successor(
            SemanticTaskStatus::Cancelled,
            Some(task_cancelled_outcome()),
            2,
        );
        let cancelled = extension_task_result(&cancelled);
        assert_eq!(cancelled["status"], "cancelled");
        assert!(cancelled.get("error").is_none());
        assert!(cancelled.get("result").is_none());
    }

    #[test]
    fn legacy_task_payload_preserves_the_terminal_json_rpc_error_channel() {
        let error = legacy_task_payload(&task_cancelled_outcome()).unwrap_err();
        assert_eq!(error.code, ErrorCode(-32000));
        assert_eq!(error.message, "Task cancelled");
        assert!(error.data.is_none());

        let result = json!({ "content": [], "isError": false });
        assert_eq!(
            serde_json::to_value(legacy_task_payload(&result).unwrap()).unwrap(),
            result
        );
    }

    #[test]
    fn required_task_support_rejects_ordinary_native_dispatch() {
        let error = ensure_ordinary_task_support("work", TaskSupportSpec::Required).unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert_eq!(error.message, "Tool work requires task-augmented execution");
        assert!(ensure_ordinary_task_support("work", TaskSupportSpec::Optional).is_ok());
        assert!(ensure_ordinary_task_support("work", TaskSupportSpec::Forbidden).is_ok());
    }

    fn meta(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Meta {
        Meta(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    #[test]
    fn legacy_creation_wire_adds_authoritative_related_task_metadata() {
        let task = Task::new(
            "task-example".to_string(),
            TaskStatus::Working,
            "2025-11-25T10:30:00.000Z".to_string(),
            "2025-11-25T10:30:00.000Z".to_string(),
        );
        let message = JsonRpcMessage::response(
            ServerResult::CreateTaskResult(CreateTaskResult::new(task)),
            rmcp::model::NumberOrString::Number(1),
        );

        let wire = serde_json::to_value(with_legacy_related_task_meta(message)).unwrap();
        assert_eq!(wire["result"]["task"]["taskId"], "task-example");
        assert_eq!(
            wire["result"]["_meta"]["io.modelcontextprotocol/related-task"],
            json!({ "taskId": "task-example" })
        );
    }

    #[test]
    fn legacy_task_ttl_accepts_exact_zero_fraction_integers() {
        let exact = serde_json::from_str::<Value>("100.0").unwrap();
        let fractional = serde_json::from_str::<Value>("100.5").unwrap();
        assert_eq!(
            legacy_task_ttl(Some(&serde_json::Map::from_iter([(
                "ttl".to_string(),
                exact,
            )])))
            .unwrap(),
            Some(100)
        );
        assert!(
            legacy_task_ttl(Some(&serde_json::Map::from_iter([(
                "ttl".to_string(),
                fractional,
            )])))
            .is_err()
        );
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
    fn progress_uses_the_first_metadata_representation_that_owns_a_token() {
        let request_without_token = meta([("applicationKey", json!("request"))]);
        let context_with_token = meta([("progressToken", json!("transport-token"))]);
        assert_eq!(
            progress_meta(Some(&request_without_token), &context_with_token),
            Some(&context_with_token)
        );

        let request_with_token = meta([("progressToken", json!("params-token"))]);
        assert_eq!(
            progress_meta(Some(&request_with_token), &context_with_token),
            Some(&request_with_token)
        );
        assert_eq!(progress_meta(None, &Meta::default()), None);
    }

    #[test]
    fn protocol_observations_include_the_negotiated_peer_version() {
        let compiled = "2025-11-25";
        let matching = json!(compiled);
        let mismatched = json!("2026-06-30");
        assert!(
            validate_protocol_observations(
                compiled,
                Some(&matching),
                Some(&matching),
                Some(compiled),
            )
            .is_ok()
        );
        assert!(
            validate_protocol_observations(compiled, Some(&mismatched), None, None)
                .unwrap_err()
                .message
                .contains("does not match compiled surface")
        );
        assert!(
            validate_protocol_observations(compiled, Some(&matching), None, Some("2025-06-18"),)
                .unwrap_err()
                .message
                .contains("Conflicting MCP protocol version")
        );
        assert!(
            validate_protocol_observations(compiled, None, None, None)
                .unwrap_err()
                .message
                .contains("Missing MCP protocol version observation")
        );
    }

    #[test]
    fn native_framework_errors_suppress_effect_lane_recovery_names() {
        let capability = crate::ErrorBody {
            code: crate::ErrorCode::CapabilityMissing,
            message: "missing capability".to_string(),
            details: json!({
                "capability": "validated-build",
                "carrier": "validation_token",
                "providers": ["build validate"]
            }),
        };
        let resource = crate::ErrorBody {
            code: crate::ErrorCode::ResourceRefused,
            message: "missing resource".to_string(),
            details: json!({
                "resource": "tab",
                "recover": {
                    "enumerate": ["tabs list"],
                    "establish": ["tabs new"]
                }
            }),
        };
        let missing_binding = crate::ErrorBody {
            code: crate::ErrorCode::ResourceBindingMissing,
            message: "missing binding".to_string(),
            details: json!({
                "resource": "session",
                "binding": "absent",
                "establish": ["session start"]
            }),
        };
        let ambient_refusal = crate::ErrorBody {
            code: crate::ErrorCode::ResourceRefused,
            message: "ambient resource refused".to_string(),
            details: json!({
                "resource": "session",
                "binding": "ambient",
                "enumerate": ["session list"],
                "establish": ["session start"]
            }),
        };

        let capability = native_framework_error_value(Some(&capability));
        assert_eq!(
            capability["message"],
            "Required capability proof is missing"
        );
        assert_eq!(capability["details"]["providers"], Value::Null);
        assert!(!capability.to_string().contains("build validate"));
        let resource = native_framework_error_value(Some(&resource));
        assert_eq!(resource["message"], "Resource reference was refused");
        assert_eq!(resource["details"]["recover"], Value::Null);
        assert!(!resource.to_string().contains("tabs list"));
        assert!(!resource.to_string().contains("tabs new"));
        let missing_binding = native_framework_error_value(Some(&missing_binding));
        assert_eq!(missing_binding["details"]["establish"], Value::Null);
        assert!(!missing_binding.to_string().contains("session start"));
        let ambient_refusal = native_framework_error_value(Some(&ambient_refusal));
        assert_eq!(ambient_refusal["message"], "Resource reference was refused");
        assert_eq!(ambient_refusal["details"]["enumerate"], Value::Null);
        assert_eq!(ambient_refusal["details"]["establish"], Value::Null);
        assert!(!ambient_refusal.to_string().contains("session list"));
        assert!(!ambient_refusal.to_string().contains("session start"));
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

    #[test]
    fn text_profile_keeps_application_errors_in_the_mcp_error_family() {
        let envelope = ResponseEnvelope {
            status: crate::ResponseStatus::Failed,
            command: Some(vec!["browser".to_string(), "status".to_string()]),
            output: None,
            error: Some(crate::ErrorBody {
                code: crate::ErrorCode::ApplicationError,
                message: "No browser session is available".to_string(),
                details: json!({
                    "applicationCode": "session_required",
                    "details": {},
                    "recoveries": [],
                }),
            }),
            diagnostics: Vec::new(),
            steering: Vec::new(),
            display: None,
            replay: None,
            preview: None,
            plan: None,
            retry: None,
        };
        let result = success_result(envelope, ResponseProfile::Text);
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_some());
        assert!(
            result.content[0]
                .raw
                .as_text()
                .unwrap()
                .text
                .contains("application_error")
        );
    }

    #[test]
    fn grouped_selector_injection_rejects_non_object_results_without_panicking() {
        let mut value = json!("invalid grouped result");
        let error = inject_native_call_arguments(
            &mut value,
            &BTreeMap::from([("operation".to_string(), json!("get"))]),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            FrameworkError::ResultContractViolation {
                boundary: crate::ResultContractBoundary::Success,
                reason: crate::ResultContractReason::SchemaMismatch,
            }
        ));
    }

    #[tokio::test]
    async fn generic_confirmation_copy_binds_the_dispatched_surface_fingerprint() {
        let registry = CommandRegistry::new("presentation", "Presentation test").register(
            crate::CommandSpec::new(["run"], "Run", "Run command"),
            |context: crate::CommandContext| async move {
                Ok(crate::CommandOutput::structured(json!({
                    "fingerprint": context.plan.invocation_fingerprint,
                })))
            },
        );
        let request = RunRequest {
            command: "run".to_string(),
            args: BTreeMap::new(),
            stdin: None,
            output: None,
            mode: RunMode::Execute,
            approval: None,
            dry_run: false,
        };
        let bare = registry.build_plan(&request).unwrap();
        let mut repo = bare.clone();
        registry
            .bind_effect_lane_presentation_fingerprint(&mut repo, "repo-write")
            .unwrap();
        let mut workspace = bare.clone();
        registry
            .bind_effect_lane_presentation_fingerprint(&mut workspace, "workspace-write")
            .unwrap();

        assert_ne!(bare.invocation_fingerprint, repo.invocation_fingerprint);
        assert_ne!(
            repo.invocation_fingerprint,
            workspace.invocation_fingerprint
        );
        assert_eq!(
            registry
                .prepare_effect_lane_confirmation(&repo, "repo-write")
                .unwrap()
                .message,
            "Run repo-write execution?"
        );
        let outcome = registry
            .dispatch_prepared_plan_with_context(
                request,
                repo.clone(),
                &crate::InvocationContext::default(),
            )
            .await
            .unwrap();
        let crate::CommandExecutionOutcome::Success(response) = outcome else {
            panic!("expected successful prepared dispatch");
        };
        assert_eq!(
            response.plan.invocation_fingerprint,
            repo.invocation_fingerprint
        );
        assert_eq!(
            response.output.unwrap().structured.unwrap()["fingerprint"],
            repo.invocation_fingerprint
        );
    }

    #[test]
    fn effect_lane_presentation_failure_stays_in_the_framework_error_channel() {
        let registry = CommandRegistry::new("presentation", "Presentation test").register(
            crate::CommandSpec::new(["run"], "Run", "Run command"),
            |_context| async { Ok(crate::CommandOutput::structured(json!({}))) },
        );
        let request = RunRequest {
            command: "run".to_string(),
            args: BTreeMap::new(),
            stdin: None,
            output: None,
            mode: RunMode::Execute,
            approval: None,
            dry_run: false,
        };
        let mut plan = registry.build_plan(&request).unwrap();
        plan.command_path = vec!["missing".to_string()];
        let error = registry
            .prepare_effect_lane_confirmation(&plan, "repo")
            .unwrap_err();
        let envelope = ResponseEnvelope::framework_error(error, Some(request), Some(plan));
        assert_eq!(envelope.status, crate::ResponseStatus::Failed);
        assert_eq!(envelope.error.unwrap().code, crate::ErrorCode::BuildFailed);
    }
}
