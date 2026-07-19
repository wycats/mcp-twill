//! RFC 0016 acceptance tests: surface-owned ambient resource binding.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use mcp_twill::{
    AmbientBindingContext, AmbientBindingFailure, AmbientBindingInfrastructureError,
    AmbientResourceBinding, ApplicationError, ApplicationErrorDecl, ApplicationErrorFootprint,
    ApplicationErrorUse, ApplicationOutput, ApplicationOutputResult, ApplicationRecoverySelection,
    ApplicationResult, ApplicationSuccess, BindAmbientResource, CONVERSATION_IDENTITY_META_KEY,
    CliMcpServer, CommandContext, CommandOutput, CommandRegistry, ErrorCode, FrameworkError,
    FrameworkHelpProjection, Grant, InMemoryEventSink, InvocationPlan, McpProtocolTarget,
    NativeApplicationErrorDialect, NativeConfirmationRoute, NativeToolSurface,
    PermissionAuthorizer, PermissionDecision, PlanResourceBindingFact, PlanResourceBindingSource,
    PrivateResourceReference, Release, Res, ResolveResource, ResolveResourceWithErrors, Resource,
    ResourceBindingDecl, ResourceBindingMode, ResourceDecl, ResourceRefusal,
    ResourceResolutionFailure, ResponseEnvelope, ResponseStatus, RunMode, RunRequest,
    WorkspaceDecl,
};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, Meta},
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Default)]
struct TestClient;

impl ClientHandler for TestClient {}

#[derive(Debug)]
struct Session {
    id: String,
}

impl Resource for Session {
    const NAME: &'static str = "session";
}

#[derive(Debug, Clone, Copy)]
enum BrowserFailure {
    BrokerUnavailable,
    SessionExpired,
}

impl fmt::Display for BrowserFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("browser operation failed")
    }
}

impl Error for BrowserFailure {}

impl ApplicationError for BrowserFailure {
    fn declarations() -> Vec<ApplicationErrorDecl> {
        vec![
            ApplicationErrorDecl::new("session_required", "No browser session is available"),
            ApplicationErrorDecl::new("broker_unavailable", "The browser broker is unavailable"),
            ApplicationErrorDecl::new("session_expired", "The browser session expired"),
        ]
    }

    fn code(&self) -> &'static str {
        match self {
            Self::BrokerUnavailable => "broker_unavailable",
            Self::SessionExpired => "session_expired",
        }
    }

    fn details(&self) -> Value {
        json!({})
    }

    fn runtime_message(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn recovery(&self) -> ApplicationRecoverySelection {
        ApplicationRecoverySelection::Declared
    }
}

mcp_twill::application_error_set! {
    struct BrowserErrors for BrowserFailure {
        ApplicationErrorUse::new("session_required")
            .recover_with("session.start")
            .at_most_one_recovery(),
        ApplicationErrorUse::new("broker_unavailable"),
        ApplicationErrorUse::new("session_expired")
            .recover_with("session.start")
            .at_most_one_recovery(),
    }
}

struct BinderErrors;

impl ApplicationErrorFootprint<BrowserFailure> for BinderErrors {
    fn codes() -> Vec<&'static str> {
        vec!["broker_unavailable"]
    }
}

struct ResolverErrors;

impl ApplicationErrorFootprint<BrowserFailure> for ResolverErrors {
    fn codes() -> Vec<&'static str> {
        vec!["session_expired"]
    }
}

mcp_twill::application_error_set! {
    struct ResolverBlindErrors for BrowserFailure {
        ApplicationErrorUse::new("broker_unavailable"),
    }
}

#[derive(Debug, Clone, Copy)]
enum BinderFailureMode {
    None,
    Application,
    OutsideFootprint,
    Infrastructure,
}

type BinderObservation = (String, String, Vec<String>);
type BinderObservations = Arc<Mutex<Vec<BinderObservation>>>;

#[derive(Clone)]
struct SessionBinder {
    calls: Arc<AtomicUsize>,
    observations: BinderObservations,
    mode: BinderFailureMode,
}

impl SessionBinder {
    fn healthy(calls: Arc<AtomicUsize>) -> Self {
        Self {
            calls,
            observations: Arc::new(Mutex::new(Vec::new())),
            mode: BinderFailureMode::None,
        }
    }
}

impl BindAmbientResource<Session> for SessionBinder {
    type Error = BrowserFailure;
    type ErrorFootprint = BinderErrors;

    async fn bind(
        &self,
        context: AmbientBindingContext<'_>,
    ) -> std::result::Result<
        PrivateResourceReference,
        AmbientBindingFailure<Self::Error, Self::ErrorFootprint>,
    > {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.observations.lock().unwrap().push((
            context.operation_id.to_string(),
            context.conversation_identity.id().to_string(),
            context
                .workspaces
                .iter()
                .map(|root| root.id.clone())
                .collect(),
        ));
        match self.mode {
            BinderFailureMode::None => {
                PrivateResourceReference::from_id(format!("ambient-{call}")).map_err(Into::into)
            }
            BinderFailureMode::Application => Err(BrowserFailure::BrokerUnavailable.into()),
            BinderFailureMode::OutsideFootprint => Err(BrowserFailure::SessionExpired.into()),
            BinderFailureMode::Infrastructure => Err(AmbientBindingInfrastructureError::new(
                AdversarialError("private-broker-reference"),
            )
            .into()),
        }
    }
}

#[derive(Debug)]
struct AdversarialError(&'static str);

impl fmt::Display for AdversarialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for AdversarialError {}

#[derive(Clone)]
struct SessionResolver {
    calls: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
    refuse: Arc<AtomicBool>,
}

impl ResolveResource<Session> for SessionResolver {
    async fn resolve(
        &self,
        reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<Session, ResourceRefusal> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(reference.to_string());
        if self.refuse.load(Ordering::SeqCst) {
            Err(ResourceRefusal::new(format!(
                "refused private value {reference} \u{202e}"
            )))
        } else {
            Ok(Session {
                id: reference.to_string(),
            })
        }
    }
}

#[derive(Clone, Copy)]
struct TypedSessionResolver;

impl ResolveResourceWithErrors<Session> for TypedSessionResolver {
    type Error = BrowserFailure;
    type ErrorFootprint = ResolverErrors;

    async fn resolve(
        &self,
        _reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<Session, ResourceResolutionFailure<Self::Error, Self::ErrorFootprint>>
    {
        Err(BrowserFailure::SessionExpired.into())
    }
}

#[derive(Clone, Copy)]
struct OutsideFootprintSessionResolver;

impl ResolveResourceWithErrors<Session> for OutsideFootprintSessionResolver {
    type Error = BrowserFailure;
    type ErrorFootprint = BinderErrors;

    async fn resolve(
        &self,
        _reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<Session, ResourceResolutionFailure<Self::Error, Self::ErrorFootprint>>
    {
        Err(BrowserFailure::SessionExpired.into())
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SessionResult {
    session: Option<String>,
    reused: bool,
}

async fn start_session(
    _: CommandContext,
) -> ApplicationOutputResult<mcp_twill::Granted<Session, ApplicationSuccess<SessionResult>>> {
    Ok(ApplicationSuccess::value(SessionResult {
        session: Some("explicit-session".to_string()),
        reused: false,
    })
    .grant(Grant::<Session>::new("explicit-session")))
}

async fn use_session(
    session: Res<Session>,
    _: CommandContext,
) -> ApplicationResult<SessionResult, BrowserFailure, BrowserErrors> {
    Ok(ApplicationSuccess::value(SessionResult {
        session: Some(session.id.clone()),
        reused: true,
    }))
}

async fn use_session_without_resolver_error(
    session: Res<Session>,
    _: CommandContext,
) -> ApplicationResult<SessionResult, BrowserFailure, ResolverBlindErrors> {
    Ok(ApplicationSuccess::value(SessionResult {
        session: Some(session.id.clone()),
        reused: true,
    }))
}

async fn maybe_session(
    session: Option<Res<Session>>,
    _: CommandContext,
) -> ApplicationResult<SessionResult, BrowserFailure, BrowserErrors> {
    Ok(ApplicationSuccess::value(SessionResult {
        session: session.map(|session| session.id.clone()),
        reused: true,
    }))
}

async fn mixed_session_modes(
    _: (Res<Session>, Option<Res<Session>>),
    _: CommandContext,
) -> ApplicationResult<SessionResult, BrowserFailure, BrowserErrors> {
    Ok(ApplicationSuccess::value(SessionResult {
        session: None,
        reused: false,
    }))
}

async fn mixed_session_modes_legacy(
    _: (Res<Session>, Option<Res<Session>>),
    _: CommandContext,
) -> mcp_twill::Result<CommandOutput> {
    Ok(CommandOutput::structured(json!({ "ok": true })))
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GroupSessionResult {
    session: String,
}

async fn use_optional_group_session(
    session: Option<Res<Session>>,
    _: CommandContext,
) -> ApplicationResult<GroupSessionResult, BrowserFailure, BrowserErrors> {
    Ok(ApplicationSuccess::value(GroupSessionResult {
        session: session
            .map(|session| session.id.clone())
            .unwrap_or_else(|| "absent".to_string()),
    }))
}

async fn identify_group_session(_: CommandContext) -> ApplicationResult<GroupSessionResult> {
    Ok(ApplicationSuccess::value(GroupSessionResult {
        session: "ordinary".to_string(),
    }))
}

async fn release_group_session(
    session: Release<Session>,
    _: CommandContext,
) -> ApplicationResult<GroupSessionResult, BrowserFailure, BrowserErrors> {
    Ok(ApplicationSuccess::value(GroupSessionResult {
        session: session.id.clone(),
    }))
}

#[derive(Clone)]
struct Fixture {
    registry: CommandRegistry,
    resolver_calls: Arc<AtomicUsize>,
    resolver_seen: Arc<Mutex<Vec<String>>>,
    refuse: Arc<AtomicBool>,
}

fn fixture() -> Fixture {
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let resolver_seen = Arc::new(Mutex::new(Vec::new()));
    let refuse = Arc::new(AtomicBool::new(false));
    let resolver = SessionResolver {
        calls: resolver_calls.clone(),
        seen: resolver_seen.clone(),
        refuse: refuse.clone(),
    };
    let registry = CommandRegistry::build("ambient-test", "Ambient resource tests", |server| {
        server.workspace(WorkspaceDecl::file("project", "file:///workspace/project"));
        server.resource(
            ResourceDecl::new("session", "A browser session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .reference_schema(json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "Explicit fallback handle. Omit unless a missing-session recovery returned one."
                }))
                .expiry("sessions expire when their lease becomes idle"),
        );
        server.resolver::<Session>(resolver);
        server.command("session start", |command| {
            command
                .summary("Start session")
                .description("Start an explicit browser session")
                .handle_result(start_session);
        });
        server.command("tabs new", |command| {
            command
                .summary("New tab")
                .description("Open a tab in the selected session")
                .uses_optional_workspace("project")
                .handle_result(use_session);
        });
        server.command("session maybe", |command| {
            command
                .summary("Maybe session")
                .description("Use a session when one is selected")
                .arg(
                    mcp_twill::arg::string("trigger")
                        .summary("Require an explicit session when present")
                        .optional()
                        .requires_argument("agent_session_id"),
                )
                .handle_result(maybe_session);
        });
    })
    .unwrap();
    Fixture {
        registry,
        resolver_calls,
        resolver_seen,
        refuse,
    }
}

fn surface(
    registry: &CommandRegistry,
    binder: SessionBinder,
    policy: mcp_twill::ExplicitCarrierPolicy,
    missing: bool,
) -> mcp_twill::Result<NativeToolSurface> {
    let mut binding = AmbientResourceBinding::from_conversation_identity(binder);
    binding = match policy {
        mcp_twill::ExplicitCarrierPolicy::Omitted => binding.omit_explicit_carrier(),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride => {
            binding.with_optional_explicit_carrier()
        }
    };
    if missing {
        binding = binding.missing_as("session_required");
    }
    NativeToolSurface::builder("ambient-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .bind_resource::<Session>(binding)
        .direct("start_session", "session.start")
        .direct("new_tab", "tabs.new")
        .direct("maybe_session", "session.maybe")
        .build(registry, McpProtocolTarget::V2025_11_25)
}

fn canonical_meta(id: &str) -> Meta {
    Meta(serde_json::Map::from_iter([(
        CONVERSATION_IDENTITY_META_KEY.to_string(),
        json!({ "version": 1, "issuer": "com.example.host", "id": id }),
    )]))
}

async fn call_native(
    server: CliMcpServer,
    tool: &str,
    arguments: Value,
    meta: Option<Meta>,
) -> anyhow::Result<rmcp::model::CallToolResult> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let mut request = CallToolRequestParams::new(tool.to_string())
        .with_arguments(serde_json::from_value(arguments)?);
    request.meta = meta;
    let result = client.call_tool(request).await?;
    client.cancel().await?;
    server_handle.await??;
    Ok(result)
}

fn result_body(result: &rmcp::model::CallToolResult) -> Value {
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

fn tool_schema(surface: &NativeToolSurface, name: &str) -> Value {
    let tool = surface
        .snapshot()
        .tools()
        .iter()
        .find(|tool| tool.name.as_ref() == name)
        .unwrap();
    Value::Object((*tool.input_schema).clone())
}

#[test]
fn public_declaration_wire_forms_and_schemas_are_exact() -> anyhow::Result<()> {
    let argument = ResourceBindingDecl {
        resource: "session".to_string(),
        mode: ResourceBindingMode::Argument,
    };
    assert_eq!(
        serde_json::to_value(&argument)?,
        json!({ "resource": "session", "mode": "argument" })
    );
    let ambient = ResourceBindingDecl {
        resource: "session".to_string(),
        mode: ResourceBindingMode::Ambient {
            context: mcp_twill::AmbientContextSource::ConversationIdentity,
            explicit: mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
            missing_error: Some("session_required".to_string()),
        },
    };
    assert_eq!(
        serde_json::to_value(&ambient)?,
        json!({
            "resource": "session",
            "mode": { "ambient": {
                "context": "conversationIdentity",
                "explicit": "optionalOverride",
                "missingError": "session_required"
            }}
        })
    );
    let schema = serde_json::to_string(&schemars::schema_for!(ResourceBindingDecl))?;
    assert!(schema.contains("conversationIdentity"));
    assert!(schema.contains("optionalOverride"));
    Ok(())
}

#[test]
fn surface_schema_help_hash_and_contract_follow_binding_mode() -> anyhow::Result<()> {
    let fixture = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let argument = NativeToolSurface::builder("ambient-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("start_session", "session.start")
        .direct("new_tab", "tabs.new")
        .direct("maybe_session", "session.maybe")
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)?;
    let explicit_argument = NativeToolSurface::builder_from(argument.declaration().clone())
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)?;
    assert_eq!(
        argument.snapshot().canonical_json(),
        explicit_argument.snapshot().canonical_json()
    );
    let override_surface = surface(
        &fixture.registry,
        SessionBinder::healthy(calls.clone()),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let omitted = surface(
        &fixture.registry,
        SessionBinder::healthy(calls),
        mcp_twill::ExplicitCarrierPolicy::Omitted,
        true,
    )?;

    assert_eq!(
        tool_schema(&argument, "new_tab")["required"],
        json!(["agent_session_id"])
    );
    assert!(
        tool_schema(&override_surface, "new_tab")["required"]
            .as_array()
            .is_none_or(|required| required.is_empty())
    );
    assert!(
        tool_schema(&override_surface, "new_tab")["properties"]["agent_session_id"].is_object()
    );
    assert!(
        tool_schema(&omitted, "new_tab")["properties"]
            .get("agent_session_id")
            .is_none()
    );
    assert_ne!(
        argument.snapshot().surface_hash(),
        override_surface.snapshot().surface_hash()
    );
    assert_ne!(
        override_surface.snapshot().surface_hash(),
        omitted.snapshot().surface_hash()
    );
    assert!(mcp_twill::check_resource_binding_projection(&fixture.registry, &argument).is_empty());
    assert!(
        mcp_twill::check_resource_binding_projection(&fixture.registry, &override_surface)
            .is_empty()
    );
    assert!(mcp_twill::check_resource_binding_projection(&fixture.registry, &omitted).is_empty());

    let loaded: mcp_twill::NativeToolSurfaceDecl =
        serde_json::from_value(serde_json::to_value(override_surface.declaration())?)?;
    let rehydrated = NativeToolSurface::builder_from(loaded)
        .attach_resource_binder::<Session>(SessionBinder::healthy(Arc::new(AtomicUsize::new(0))))
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)?;
    assert_eq!(
        rehydrated.snapshot().canonical_json(),
        override_surface.snapshot().canonical_json()
    );
    assert_eq!(
        rehydrated.snapshot().surface_hash(),
        override_surface.snapshot().surface_hash()
    );
    let help = override_surface
        .snapshot()
        .tools()
        .iter()
        .find(|tool| tool.name.as_ref() == "new_tab")
        .unwrap()
        .description
        .as_deref()
        .unwrap();
    assert!(
        help.contains("`session` supplied by host; explicit override `agent_session_id` accepted")
    );
    Ok(())
}

#[tokio::test]
async fn grouped_ambient_refinement_preserves_other_members_ordinary_arguments()
-> anyhow::Result<()> {
    let reference_schema = json!({
        "type": "string",
        "minLength": 1,
        "description": "Explicit fallback handle. Omit unless a missing-session recovery returned one."
    });
    let registry = CommandRegistry::build("grouped", "Grouped resource tests", |server| {
        server.resource(
            ResourceDecl::new("session", "A browser session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .reference_schema(reference_schema.clone())
                .expiry("sessions expire when their lease becomes idle"),
        );
        server.resolver::<Session>(SessionResolver {
            calls: Arc::new(AtomicUsize::new(0)),
            seen: Arc::new(Mutex::new(Vec::new())),
            refuse: Arc::new(AtomicBool::new(false)),
        });
        server.command("session start", |command| {
            command
                .summary("Start")
                .description("Start an explicit browser session")
                .handle_result(start_session);
        });
        server.command("tabs new", |command| {
            command
                .summary("New tab")
                .description("Open a tab in the selected session")
                .arg(
                    mcp_twill::arg::inline_schema(
                        "payload",
                        json!({
                            "type": "object",
                            "properties": {
                                "agent_session_id": { "type": "string" }
                            },
                            "required": ["agent_session_id"],
                            "additionalProperties": false
                        }),
                    )
                    .summary("Nested payload"),
                )
                .arg(
                    mcp_twill::arg::string("trigger")
                        .summary("Trigger ambient use")
                        .optional()
                        .requires_argument("agent_session_id"),
                )
                .handle_result(use_optional_group_session);
        });
        server.command("session identify", |command| {
            command
                .summary("Identify")
                .description("Use an ordinary argument with the carrier's name")
                .arg(
                    mcp_twill::arg::inline_schema("agent_session_id", reference_schema)
                        .summary("Explicit fallback handle. Omit unless a missing-session recovery returned one."),
                )
                .handle_result(identify_group_session);
        });
    })?;
    let surface = NativeToolSurface::builder("grouped-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .bind_resource::<Session>(
            AmbientResourceBinding::from_conversation_identity(SessionBinder::healthy(Arc::new(
                AtomicUsize::new(0),
            )))
            .omit_explicit_carrier(),
        )
        .direct("start_session", "session.start")
        .group("session", |group| {
            group
                .selector("operation")
                .member("new_tab", "tabs.new")
                .member("identify", "session.identify");
        })
        .build(&registry, McpProtocolTarget::V2025_11_25)?;

    let schema = tool_schema(&surface, "session");
    assert!(schema["properties"]["agent_session_id"].is_object());
    assert!(schema["properties"]["payload"]["properties"]["agent_session_id"].is_object());
    assert_eq!(
        schema["properties"]["payload"]["required"],
        json!(["agent_session_id"])
    );
    assert!(schema.get("dependencies").is_none());
    assert!(mcp_twill::check_resource_binding_projection(&registry, &surface).is_empty());
    let result = call_native(
        CliMcpServer::with_surface(registry, surface)?,
        "session",
        json!({
            "operation": "new_tab",
            "payload": { "agent_session_id": "nested" },
            "trigger": "present"
        }),
        Some(canonical_meta("grouped-chat")),
    )
    .await?;
    assert_eq!(result.is_error, Some(false), "{}", result_body(&result));
    assert_eq!(result_body(&result)["session"], "ambient-1");
    Ok(())
}

#[tokio::test]
async fn ambient_selection_resolves_without_disclosing_private_values() -> anyhow::Result<()> {
    let fixture = fixture();
    let binder_calls = Arc::new(AtomicUsize::new(0));
    let binder = SessionBinder::healthy(binder_calls.clone());
    let observations = binder.observations.clone();
    let ambient_surface = surface(
        &fixture.registry,
        binder,
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let events = Arc::new(InMemoryEventSink::new());
    let server = CliMcpServer::with_surface(fixture.registry, ambient_surface)?
        .with_event_sink(events.clone());
    let result = call_native(
        server,
        "new_tab",
        json!({}),
        Some(canonical_meta("secret-chat")),
    )
    .await?;
    assert_eq!(result.is_error, Some(false), "{}", result_body(&result));
    assert_eq!(binder_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.resolver_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.resolver_seen.lock().unwrap().as_slice(),
        ["ambient-1"]
    );
    assert_eq!(observations.lock().unwrap()[0].0, "tabs.new");
    assert_eq!(observations.lock().unwrap()[0].2, ["project"]);

    let event = &events.events()[0];
    assert_eq!(
        event.resource_binding_facts,
        vec![PlanResourceBindingFact {
            resource: "session".to_string(),
            source: PlanResourceBindingSource::Ambient,
        }]
    );
    let event_json = serde_json::to_string(event)?;
    assert!(!event_json.contains("secret-chat"));
    assert!(!event_json.contains("ambient-1"));
    assert!(!event_json.contains("privateDigest"));
    Ok(())
}

#[tokio::test]
async fn explicit_override_wins_and_refusal_never_falls_back() -> anyhow::Result<()> {
    let fixture = fixture();
    fixture.refuse.store(true, Ordering::SeqCst);
    let binder_calls = Arc::new(AtomicUsize::new(0));
    let refusal_surface = surface(
        &fixture.registry,
        SessionBinder::healthy(binder_calls.clone()),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let malformed = call_native(
        CliMcpServer::with_surface(fixture.registry.clone(), refusal_surface.clone())?,
        "new_tab",
        json!({ "agent_session_id": "" }),
        Some(canonical_meta("ambient-secret")),
    )
    .await?;
    assert_eq!(result_body(&malformed)["code"], "invalid_argument_type");
    assert_eq!(binder_calls.load(Ordering::SeqCst), 0);
    let result = call_native(
        CliMcpServer::with_surface(fixture.registry, refusal_surface)?,
        "new_tab",
        json!({ "agent_session_id": "explicit-secret" }),
        Some(canonical_meta("ambient-secret")),
    )
    .await?;
    assert_eq!(binder_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.resolver_seen.lock().unwrap().as_slice(),
        ["explicit-secret"]
    );
    let body = result_body(&result);
    assert_eq!(body["code"], "resource_refused");
    assert_eq!(body["details"]["reference"], "explicit-secret");
    assert!(
        body["details"]["detail"]
            .as_str()
            .unwrap()
            .contains("explicit-secret")
    );
    Ok(())
}

#[tokio::test]
async fn ambient_refusal_and_infrastructure_errors_are_redacted() -> anyhow::Result<()> {
    let fixture = fixture();
    fixture.refuse.store(true, Ordering::SeqCst);
    let refusal_surface = surface(
        &fixture.registry,
        SessionBinder::healthy(Arc::new(AtomicUsize::new(0))),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let result = call_native(
        CliMcpServer::with_surface(fixture.registry.clone(), refusal_surface.clone())?,
        "new_tab",
        json!({}),
        Some(canonical_meta("ambient-secret")),
    )
    .await?;
    let body = result_body(&result);
    assert_eq!(body["code"], "resource_refused");
    assert_eq!(body["details"]["binding"], "ambient");
    let text = body.to_string();
    assert!(!text.contains("ambient-secret"));
    assert!(!text.contains("ambient-1"));
    assert!(!text.contains("refused private"));

    let optional = call_native(
        CliMcpServer::with_surface(fixture.registry.clone(), refusal_surface)?,
        "maybe_session",
        json!({}),
        Some(canonical_meta("ambient-secret")),
    )
    .await?;
    assert_eq!(result_body(&optional)["code"], "resource_refused");

    fixture.refuse.store(false, Ordering::SeqCst);
    let binder = SessionBinder {
        calls: Arc::new(AtomicUsize::new(0)),
        observations: Arc::new(Mutex::new(Vec::new())),
        mode: BinderFailureMode::Infrastructure,
    };
    let infrastructure_surface = surface(
        &fixture.registry,
        binder,
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let result = call_native(
        CliMcpServer::with_surface(fixture.registry, infrastructure_surface)?,
        "new_tab",
        json!({}),
        Some(canonical_meta("ambient-secret")),
    )
    .await?;
    let body = result_body(&result);
    assert_eq!(body["code"], "handler_failed");
    assert!(!body.to_string().contains("private-broker-reference"));
    Ok(())
}

#[tokio::test]
async fn binder_application_errors_use_the_declared_result_contract() -> anyhow::Result<()> {
    let valid_fixture = fixture();
    let binder = SessionBinder {
        calls: Arc::new(AtomicUsize::new(0)),
        observations: Arc::new(Mutex::new(Vec::new())),
        mode: BinderFailureMode::Application,
    };
    let ambient_surface = surface(
        &valid_fixture.registry,
        binder,
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let result = call_native(
        CliMcpServer::with_surface(valid_fixture.registry, ambient_surface)?,
        "new_tab",
        json!({}),
        Some(canonical_meta("ambient-secret")),
    )
    .await?;
    let body = result_body(&result);
    assert_eq!(body["code"], "broker_unavailable");
    assert!(!body.to_string().contains("ambient-secret"));

    let fixture = fixture();
    let outside_footprint = surface(
        &fixture.registry,
        SessionBinder {
            calls: Arc::new(AtomicUsize::new(0)),
            observations: Arc::new(Mutex::new(Vec::new())),
            mode: BinderFailureMode::OutsideFootprint,
        },
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let result = call_native(
        CliMcpServer::with_surface(fixture.registry, outside_footprint)?,
        "new_tab",
        json!({}),
        Some(canonical_meta("ambient-secret")),
    )
    .await?;
    let body = result_body(&result);
    assert_eq!(body["code"], "result_contract_violation");
    assert_eq!(body["details"]["boundary"], "applicationError");
    assert_eq!(body["details"]["reason"], "undeclaredCode");
    assert!(!body.to_string().contains("session_expired"));
    assert!(!body.to_string().contains("ambient-secret"));
    Ok(())
}

#[tokio::test]
async fn required_and_optional_absence_have_distinct_owner_paths() -> anyhow::Result<()> {
    let fixture = fixture();
    let argument_surface = NativeToolSurface::builder("argument-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("start_session", "session.start")
        .direct("new_tab", "tabs.new")
        .direct("maybe_session", "session.maybe")
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)?;
    assert_eq!(
        tool_schema(&argument_surface, "maybe_session")["dependentRequired"]["trigger"],
        json!(["agent_session_id"])
    );
    let result = call_native(
        CliMcpServer::with_surface(fixture.registry.clone(), argument_surface)?,
        "maybe_session",
        json!({ "trigger": "present" }),
        None,
    )
    .await?;
    assert_eq!(result_body(&result)["code"], "invalid_argument_type");

    let binder_calls = Arc::new(AtomicUsize::new(0));
    let with_missing = surface(
        &fixture.registry,
        SessionBinder::healthy(binder_calls.clone()),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let authorization_observations = Arc::new(Mutex::new(Vec::new()));
    let result = call_native(
        CliMcpServer::builder(fixture.registry.clone())
            .surface(with_missing)
            .authorizer(Arc::new(CaptureAuthorizer {
                decision: PermissionDecision::Allow,
                plans: authorization_observations.clone(),
            }))
            .build()?,
        "new_tab",
        json!({}),
        None,
    )
    .await?;
    let body = result_body(&result);
    assert_eq!(body["code"], "session_required");
    assert_eq!(binder_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.resolver_calls.load(Ordering::SeqCst), 0);
    assert!(authorization_observations.lock().unwrap().is_empty());

    let optional_surface = surface(
        &fixture.registry,
        SessionBinder::healthy(binder_calls.clone()),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let result = call_native(
        CliMcpServer::with_surface(fixture.registry.clone(), optional_surface)?,
        "maybe_session",
        json!({}),
        None,
    )
    .await?;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(binder_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.resolver_calls.load(Ordering::SeqCst), 0);

    let without_missing = surface(
        &fixture.registry,
        SessionBinder::healthy(binder_calls),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        false,
    )?;
    let events = Arc::new(InMemoryEventSink::new());
    let result = call_native(
        CliMcpServer::with_surface(fixture.registry, without_missing)?
            .with_event_sink(events.clone()),
        "new_tab",
        json!({}),
        None,
    )
    .await?;
    let body = result_body(&result);
    assert_eq!(body["code"], "resource_binding_missing");
    assert_eq!(body["details"]["binding"], "absent");
    assert!(body["details"].get("establish").is_none());
    assert!(!body.to_string().contains("session start"));
    let envelope = ResponseEnvelope::framework_error(
        FrameworkError::ResourceBindingMissing {
            resource: "session".to_string(),
            establish: Box::new([]),
        },
        None,
        None,
    );
    assert_eq!(envelope.status, ResponseStatus::InvalidInput);
    assert_eq!(
        events.events()[0].resource_binding_facts[0].source,
        PlanResourceBindingSource::Absent
    );
    Ok(())
}

#[derive(Clone)]
struct CaptureAuthorizer {
    decision: PermissionDecision,
    plans: AuthorizationObservations,
}

type AuthorizationObservation = (String, Vec<PlanResourceBindingFact>);
type AuthorizationObservations = Arc<Mutex<Vec<AuthorizationObservation>>>;

impl PermissionAuthorizer for CaptureAuthorizer {
    fn decide(&self, plan: &InvocationPlan) -> PermissionDecision {
        self.plans.lock().unwrap().push((
            plan.invocation_fingerprint.clone(),
            plan.resource_binding_facts.clone(),
        ));
        self.decision.clone()
    }
}

#[tokio::test]
async fn authorization_observes_redacted_source_and_denial_skips_realization() -> anyhow::Result<()>
{
    let fixture = fixture();
    let binder_calls = Arc::new(AtomicUsize::new(0));
    let surface = surface(
        &fixture.registry,
        SessionBinder::healthy(binder_calls.clone()),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let plans = Arc::new(Mutex::new(Vec::new()));
    let authorizer = Arc::new(CaptureAuthorizer {
        decision: PermissionDecision::Deny {
            reason: "test denial".to_string(),
        },
        plans: plans.clone(),
    });
    let server = CliMcpServer::builder(fixture.registry)
        .surface(surface)
        .authorizer(authorizer)
        .build()?;
    let result = call_native(server, "new_tab", json!({}), Some(canonical_meta("secret"))).await?;
    assert_eq!(result_body(&result)["code"], "permission_denied");
    assert_eq!(binder_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.resolver_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        plans.lock().unwrap()[0].1[0].source,
        PlanResourceBindingSource::Ambient
    );
    Ok(())
}

#[tokio::test]
async fn logical_identity_and_selected_source_bind_invocation_fingerprints() -> anyhow::Result<()> {
    let fixture = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let surface = surface(
        &fixture.registry,
        SessionBinder::healthy(calls),
        mcp_twill::ExplicitCarrierPolicy::OptionalOverride,
        true,
    )?;
    let observed = Arc::new(Mutex::new(Vec::new()));
    for (meta, arguments) in [
        (Some(canonical_meta("chat-a")), json!({})),
        (Some(canonical_meta("chat-a")), json!({})),
        (Some(canonical_meta("chat-b")), json!({})),
        (
            Some(canonical_meta("chat-a")),
            json!({ "agent_session_id": "explicit" }),
        ),
    ] {
        let authorizer = Arc::new(CaptureAuthorizer {
            decision: PermissionDecision::Allow,
            plans: observed.clone(),
        });
        let server = CliMcpServer::builder(fixture.registry.clone())
            .surface(surface.clone())
            .authorizer(authorizer)
            .build()?;
        let result = call_native(server, "new_tab", arguments, meta).await?;
        assert_eq!(result.is_error, Some(false));
    }
    let plans = observed.lock().unwrap();
    assert_eq!(plans[0].0, plans[1].0);
    assert_ne!(plans[0].0, plans[2].0);
    assert_ne!(plans[0].0, plans[3].0);
    assert_eq!(plans[0].1[0].source, PlanResourceBindingSource::Ambient);
    assert_eq!(plans[3].1[0].source, PlanResourceBindingSource::Argument);
    Ok(())
}

#[test]
fn private_reference_and_infrastructure_wrappers_are_static_and_redacted() {
    for accepted in ["a", "A-Z_0.~", "session-123"] {
        assert!(PrivateResourceReference::from_id(accepted).is_ok());
    }
    for rejected in [
        "",
        "test://session/id",
        "with space",
        "slash/value",
        "é",
        "line\n",
    ] {
        let error = PrivateResourceReference::from_id(rejected).unwrap_err();
        if !rejected.is_empty() {
            assert!(!error.to_string().contains(rejected));
        }
    }
    let reference = PrivateResourceReference::from_id("private-value").unwrap();
    assert_eq!(
        format!("{reference:?}"),
        "PrivateResourceReference(<redacted>)"
    );

    let error = AmbientBindingInfrastructureError::new(AdversarialError("secret-source"));
    assert_eq!(error.to_string(), "ambient resource binding failed");
    assert_eq!(
        format!("{error:?}"),
        "AmbientBindingInfrastructureError(<redacted>)"
    );
    assert!(error.source().is_none());
}

#[test]
fn builder_rejects_ambiguous_incomplete_and_mismatched_sidecars() {
    let fixture = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let binder = SessionBinder::healthy(calls.clone());
    let error = NativeToolSurface::builder("bad")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .bind_resource::<Session>(AmbientResourceBinding::from_conversation_identity(binder))
        .direct("new_tab", "tabs.new")
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("explicitly choose carrier"));

    let repeated =
        AmbientResourceBinding::from_conversation_identity(SessionBinder::healthy(calls.clone()))
            .omit_explicit_carrier()
            .with_optional_explicit_carrier();
    let error = NativeToolSurface::builder("bad")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .bind_resource::<Session>(repeated)
        .direct("new_tab", "tabs.new")
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("more than once"));

    let declaration = mcp_twill::NativeToolSurfaceDecl {
        name: "loaded".to_string(),
        exposure: mcp_twill::NativeExposurePolicy::Complete,
        framework_help: FrameworkHelpProjection::Omitted,
        application_errors: NativeApplicationErrorDialect::Canonical,
        confirmation: NativeConfirmationRoute::Unavailable,
        resource_bindings: vec![ResourceBindingDecl {
            resource: "session".to_string(),
            mode: ResourceBindingMode::Ambient {
                context: mcp_twill::AmbientContextSource::ConversationIdentity,
                explicit: mcp_twill::ExplicitCarrierPolicy::Omitted,
                missing_error: Some("session_required".to_string()),
            },
        }],
        tools: vec![mcp_twill::NativeToolDecl::Direct {
            name: "new_tab".to_string(),
            operation_id: "tabs.new".to_string(),
            title: None,
            description: None,
        }],
    };
    let error = NativeToolSurface::builder_from(declaration)
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("no attached binder"));
}

#[test]
fn optional_resource_modes_and_dead_missing_errors_fail_registration() {
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = SessionResolver {
        calls: Arc::new(AtomicUsize::new(0)),
        seen: Arc::new(Mutex::new(Vec::new())),
        refuse: Arc::new(AtomicBool::new(false)),
    };
    let mixed = CommandRegistry::build("mixed", "Mixed resource modes", |server| {
        server.resource(
            ResourceDecl::new("session", "A session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .expiry("session leases expire"),
        );
        server.resolver::<Session>(resolver);
        server.command("session start", |command| {
            command
                .summary("Start")
                .description("Start a session")
                .handle_result(start_session);
        });
        server.command("session mixed", |command| {
            command
                .summary("Mixed")
                .description("Invalid mixed resource modes")
                .handle_result(mixed_session_modes);
        });
    })
    .err()
    .expect("mixed resource modes must fail");
    let mixed = mixed.to_string();
    assert!(
        mixed.contains("resource")
            && (mixed.contains("optional") || mixed.contains("more than once")),
        "{mixed}"
    );

    let mixed_legacy = CommandRegistry::build("mixed", "Mixed resource modes", |server| {
        server.resource(
            ResourceDecl::new("session", "A session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .expiry("session leases expire"),
        );
        server.resolver::<Session>(SessionResolver {
            calls: Arc::new(AtomicUsize::new(0)),
            seen: Arc::new(Mutex::new(Vec::new())),
            refuse: Arc::new(AtomicBool::new(false)),
        });
        server.command("session mixed", |command| {
            command
                .summary("Mixed")
                .description("Invalid mixed legacy resource modes")
                .handle(mixed_session_modes_legacy);
        });
    })
    .err()
    .expect("mixed legacy resource modes must fail")
    .to_string();
    assert!(mixed_legacy.contains("both required and optional"));

    let fixture = fixture();
    let error = NativeToolSurface::builder("optional-closure")
        .exposure(mcp_twill::NativeExposurePolicy::explicit_subset([
            "session.start",
            "tabs.new",
        ]))
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("maybe_session", "session.maybe")
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("session.start"));
    assert!(error.to_string().contains("reachable from `session.maybe`"));

    let error = NativeToolSurface::builder("optional-only")
        .exposure(mcp_twill::NativeExposurePolicy::explicit_subset([
            "session.start",
            "tabs.new",
        ]))
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .bind_resource::<Session>(
            AmbientResourceBinding::from_conversation_identity(SessionBinder::healthy(calls))
                .with_optional_explicit_carrier()
                .missing_as("session_required"),
        )
        .direct("maybe_session", "session.maybe")
        .build(&fixture.registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("dead missing error"));
}

#[tokio::test]
async fn typed_resolver_errors_share_the_application_result_boundary() -> anyhow::Result<()> {
    let registry = CommandRegistry::build("typed", "Typed resolver", |server| {
        server.resource(
            ResourceDecl::new("session", "A session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .expiry("session leases expire"),
        );
        server.resolver_with_errors::<Session>(TypedSessionResolver);
        server.command("session start", |command| {
            command
                .summary("Start")
                .description("Start a session")
                .handle_result(start_session);
        });
        server.command("tabs new", |command| {
            command
                .summary("New tab")
                .description("Open a tab")
                .handle_result(use_session);
        });
    })?;
    let binder_calls = Arc::new(AtomicUsize::new(0));
    let native = NativeToolSurface::builder("typed-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .bind_resource::<Session>(
            AmbientResourceBinding::from_conversation_identity(SessionBinder::healthy(
                binder_calls.clone(),
            ))
            .with_optional_explicit_carrier()
            .missing_as("session_required"),
        )
        .direct("start_session", "session.start")
        .direct("new_tab", "tabs.new")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let result = call_native(
        CliMcpServer::with_surface(registry, native)?,
        "new_tab",
        json!({}),
        Some(canonical_meta("chat")),
    )
    .await?;
    assert_eq!(result_body(&result)["code"], "session_expired");
    assert_eq!(binder_calls.load(Ordering::SeqCst), 1);

    let duplicate = fixture()
        .registry
        .with_resolver_with_errors::<Session>(TypedSessionResolver)
        .validate_results()
        .unwrap_err();
    assert!(duplicate.to_string().contains("more than one resolver"));

    let invalid = CommandRegistry::new("invalid", "Invalid typed resolver")
        .declare_resource(
            ResourceDecl::new("session", "A session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .expiry("session leases expire"),
        )
        .with_resolver_with_errors::<Session>(TypedSessionResolver)
        .register_result(
            mcp_twill::CommandSpec::new(["session", "start"], "Start", "Start an explicit session"),
            start_session,
        )
        .register_result(
            mcp_twill::CommandSpec::new(
                ["tabs", "invalid"],
                "Invalid",
                "Uses a resolver error outside the handler footprint",
            ),
            use_session_without_resolver_error,
        );
    let invalid = invalid
        .run(RunRequest {
            command: "tabs invalid --agent-session-id $args.agent_session_id".to_string(),
            args: BTreeMap::from([("agent_session_id".to_string(), json!("session-1"))]),
            stdin: None,
            output: None,
            mode: RunMode::Execute,
            approval: None,
            dry_run: false,
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(invalid.contains("does not declare"), "{invalid}");

    let outside_footprint = CommandRegistry::build("outside", "Outside footprint", |server| {
        server.resource(
            ResourceDecl::new("session", "A session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .expiry("session leases expire"),
        );
        server.resolver_with_errors::<Session>(OutsideFootprintSessionResolver);
        server.command("session start", |command| {
            command
                .summary("Start")
                .description("Start a session")
                .handle_result(start_session);
        });
        server.command("tabs new", |command| {
            command
                .summary("New tab")
                .description("Open a tab")
                .handle_result(use_session);
        });
    })?;
    let outside_footprint = outside_footprint
        .run(RunRequest {
            command: "tabs new --agent-session-id $args.agent_session_id".to_string(),
            args: BTreeMap::from([("agent_session_id".to_string(), json!("session-1"))]),
            stdin: None,
            output: None,
            mode: RunMode::Execute,
            approval: None,
            dry_run: false,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        outside_footprint,
        FrameworkError::ResultContractViolation {
            boundary: mcp_twill::ResultContractBoundary::ApplicationError,
            reason: mcp_twill::ResultContractReason::UndeclaredCode,
        }
    ));
    Ok(())
}

#[tokio::test]
async fn native_planning_deduplicates_consume_and_release_carriers() -> anyhow::Result<()> {
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let registry = CommandRegistry::build("release", "Release resource", |server| {
        server.resource(
            ResourceDecl::new("session", "A session")
                .uri("test://session/{id}")
                .carrier("agent_session_id")
                .expiry("session leases expire"),
        );
        server.resolver::<Session>(SessionResolver {
            calls: resolver_calls.clone(),
            seen: Arc::new(Mutex::new(Vec::new())),
            refuse: Arc::new(AtomicBool::new(false)),
        });
        server.command("session start", |command| {
            command
                .summary("Start")
                .description("Start a session")
                .handle_result(start_session);
        });
        server.command("session finish", |command| {
            command
                .summary("Finish")
                .description("Finish a session")
                .handle_result(release_group_session);
        });
    })?;
    let binder_calls = Arc::new(AtomicUsize::new(0));
    let surface = NativeToolSurface::builder("release-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .bind_resource::<Session>(
            AmbientResourceBinding::from_conversation_identity(SessionBinder::healthy(
                binder_calls.clone(),
            ))
            .omit_explicit_carrier(),
        )
        .direct("start_session", "session.start")
        .direct("finish_session", "session.finish")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let result = call_native(
        CliMcpServer::with_surface(registry, surface)?,
        "finish_session",
        json!({}),
        Some(canonical_meta("release-chat")),
    )
    .await?;
    assert_eq!(result.is_error, Some(false), "{}", result_body(&result));
    assert_eq!(result_body(&result)["session"], "ambient-1");
    assert_eq!(binder_calls.load(Ordering::SeqCst), 1);
    assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn error_code_and_plan_fact_legacy_omission_are_stable() -> anyhow::Result<()> {
    assert_eq!(
        serde_json::to_value(ErrorCode::ResourceBindingMissing)?,
        json!("resource_binding_missing")
    );
    let plan = fixture().registry.build_plan(&RunRequest {
        command: "tabs new --agent-session-id $args.agent_session_id".to_string(),
        args: BTreeMap::from([("agent_session_id".to_string(), json!("session-1"))]),
        stdin: None,
        output: None,
        mode: RunMode::Execute,
        approval: None,
        dry_run: false,
    })?;
    let mut wire = serde_json::to_value(plan)?;
    wire.as_object_mut().unwrap().remove("resourceBindingFacts");
    let plan: InvocationPlan = serde_json::from_value(wire)?;
    assert!(plan.resource_binding_facts.is_empty());
    assert!(
        serde_json::to_value(plan)?
            .get("resourceBindingFacts")
            .is_none()
    );
    Ok(())
}
