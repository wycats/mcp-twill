use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use mcp_twill::{
    ArgSpec, CONVERSATION_IDENTITY_META_KEY, CliMcpServer, CliMcpServerConfig, CommandContext,
    CommandOutput, CommandRegistry, CommandSpec, ConversationIdentity,
    ConversationIdentityCompatibility, ErrorCode, FrameworkError, FrameworkEvent, HelpRequest,
    InvocationContext, PermissionSpec, ReplayRecord, ResolvedResources, ResponseEnvelope, RunMode,
    RunRequest,
};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{
        CallToolRequestParams, ClientRequest, GetPromptRequestParams, Meta,
        ReadResourceRequestParams, Request, ServerResult,
    },
};
use schemars::schema_for;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct TestClient;

impl ClientHandler for TestClient {}

fn request(command: &str, args: Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args).unwrap(),
        stdin: None,
        output: None,
        mode: RunMode::Execute,
        approval: None,
        dry_run: false,
    }
}

fn json_object<T: serde::Serialize>(value: T) -> serde_json::Map<String, Value> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .unwrap()
        .clone()
}

fn meta_with(canonical: Option<Value>, thread_id: Option<Value>) -> Meta {
    let mut values = serde_json::Map::new();
    if let Some(canonical) = canonical {
        values.insert(CONVERSATION_IDENTITY_META_KEY.to_string(), canonical);
    }
    if let Some(thread_id) = thread_id {
        values.insert("threadId".to_string(), thread_id);
    }
    Meta(values)
}

fn canonical(issuer: &str, id: &str) -> Value {
    json!({ "version": 1, "issuer": issuer, "id": id })
}

type Observation = (Option<ConversationIdentity>, String);

fn observed_registry(declaring: bool, seen: Arc<Mutex<Vec<Observation>>>) -> CommandRegistry {
    let mut spec = CommandSpec::new(
        ["session", "current"],
        "Current session",
        "Reports whether host conversation context reached the handler.",
    )
    .with_permission(PermissionSpec::read("session", "Reads session context"));
    if declaring {
        spec = spec.uses_conversation_identity();
    }
    CommandRegistry::new("conversation-test", "Conversation identity test server").register(
        spec,
        move |context: CommandContext| {
            let seen = seen.clone();
            async move {
                let identity = context.conversation_identity().cloned();
                let fingerprint = context.plan.invocation_fingerprint.clone();
                seen.lock().unwrap().push((identity.clone(), fingerprint));
                Ok(CommandOutput::structured(json!({
                    "hasIdentity": identity.is_some(),
                })))
            }
        },
    )
}

fn contract_registry() -> CommandRegistry {
    CommandRegistry::new(
        "conversation-contract",
        "Conversation identity contract server",
    )
    .register(
        CommandSpec::new(
            ["session", "current"],
            "Current session",
            "Uses optional host conversation identity.",
        )
        .uses_conversation_identity(),
        |_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) },
    )
    .register(
        CommandSpec::new(
            ["session", "global"],
            "Global session",
            "Does not consume host conversation identity.",
        ),
        |_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) },
    )
}

mcp_twill::contract_tests!(contract_registry);

async fn call_mcp(
    registry: CommandRegistry,
    config: CliMcpServerConfig,
    meta: Meta,
    request: RunRequest,
) -> anyhow::Result<rmcp::model::CallToolResult> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server = CliMcpServer::with_config(registry, config)?;
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let mut params = CallToolRequestParams::new("run").with_arguments(json_object(request));
    params.meta = Some(meta);
    let result = client.call_tool(params).await?;
    client.cancel().await?;
    server_handle.await??;
    Ok(result)
}

#[test]
fn canonical_identity_round_trips_and_redacts_debug() {
    let identity = ConversationIdentity::new("com.example.host", " opaque id ").unwrap();
    assert_eq!(identity.version(), 1);
    assert_eq!(identity.issuer(), "com.example.host");
    assert_eq!(identity.id(), " opaque id ");

    let encoded = serde_json::to_value(&identity).unwrap();
    assert_eq!(encoded, canonical("com.example.host", " opaque id "));
    let decoded: ConversationIdentity = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, identity);

    let debug = format!("{identity:?}");
    assert!(debug.contains("version: 1"));
    assert!(!debug.contains("com.example.host"));
    assert!(!debug.contains("opaque id"));

    for invalid in [
        json!({ "version": 1, "issuer": "Example.COM", "id": "x" }),
        json!({ "version": 1, "issuer": "com.-example", "id": "x" }),
        json!({ "version": 1, "issuer": "single", "id": "x" }),
        json!({ "version": 1, "issuer": "com.example", "id": "", "extra": true }),
    ] {
        assert!(serde_json::from_value::<ConversationIdentity>(invalid).is_err());
    }
}

#[test]
fn declaration_projects_through_catalog_help_builder_and_hash() {
    let explicit = contract_registry();
    let operation = explicit
        .operation_specs()
        .into_iter()
        .find(|operation| operation.name() == "session current")
        .unwrap();
    assert!(operation.uses_conversation_identity);

    let catalog = serde_json::to_value(explicit.catalog()).unwrap();
    let catalog_text = catalog.to_string();
    assert!(catalog_text.contains("usesConversationIdentity"));
    let help = explicit.help(HelpRequest {
        command: Some("session current".to_string()),
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("Request context:"));
    assert!(
        help.text
            .contains("conversation identity (optional, supplied by host)")
    );
    assert!(
        !explicit
            .help(HelpRequest {
                command: Some("session global".to_string()),
                topic: None,
                detail: None,
            })
            .text
            .contains("conversation identity (optional, supplied by host)")
    );
    let schema = explicit
        .arg_schema(
            explicit
                .command_specs()
                .find(|spec| spec.name() == "session current")
                .unwrap(),
        )
        .to_string();
    assert!(!schema.contains(CONVERSATION_IDENTITY_META_KEY));
    assert!(!schema.contains("conversationIdentity"));

    let without = CommandRegistry::new(
        "conversation-contract",
        "Conversation identity contract server",
    )
    .register(
        CommandSpec::new(
            ["session", "current"],
            "Current session",
            "Uses optional host conversation identity.",
        ),
        |_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) },
    )
    .register(
        CommandSpec::new(
            ["session", "global"],
            "Global session",
            "Does not consume host conversation identity.",
        ),
        |_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) },
    );
    assert_ne!(
        explicit.catalog_identity().catalog_hash,
        without.catalog_identity().catalog_hash
    );

    let built = CommandRegistry::build("built", "Builder test", |server| {
        server.command("session current", |command| {
            command
                .summary("Current session")
                .description("Uses optional host conversation identity.")
                .uses_conversation_identity()
                .handle(|_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) });
        });
    })
    .unwrap();
    assert!(
        built
            .command_specs()
            .next()
            .unwrap()
            .uses_conversation_identity
    );
}

#[tokio::test]
async fn direct_registry_injection_is_optional_and_declared() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let registry = observed_registry(true, seen.clone());
    let run = request("session current", json!({}));

    registry.run(run.clone()).await.unwrap();
    let identity = ConversationIdentity::new("com.example.host", "thread-a").unwrap();
    registry
        .run_with_context(
            run.clone(),
            InvocationContext::new().with_conversation_identity(identity.clone()),
        )
        .await
        .unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0, None);
    assert_eq!(seen[1].0.as_ref(), Some(&identity));
    assert_ne!(seen[0].1, seen[1].1);

    let plan = registry.build_plan(&run).unwrap();
    let context = CommandContext::new(plan, None, ResolvedResources::default());
    assert_eq!(context.conversation_identity(), None);
}

#[tokio::test]
async fn non_declaring_command_ignores_injected_identity() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let registry = observed_registry(false, seen.clone());
    let run = request("session current", json!({}));
    let identity = ConversationIdentity::new("com.example.host", "thread-a").unwrap();
    let context = InvocationContext::new().with_conversation_identity(identity);

    let absent = registry.build_plan(&run).unwrap();
    let present = registry.build_plan_with_context(&run, &context).unwrap();
    assert_eq!(
        absent.invocation_fingerprint,
        present.invocation_fingerprint
    );
    registry.run_with_context(run, context).await.unwrap();
    assert_eq!(seen.lock().unwrap()[0].0, None);
}

#[test]
fn declaring_fingerprint_binds_presence_and_complete_identity() {
    let registry = observed_registry(true, Arc::new(Mutex::new(Vec::new())));
    let run = request("session current", json!({}));
    let absent = registry.build_plan(&run).unwrap();
    let first = registry
        .build_plan_with_context(
            &run,
            &InvocationContext::new().with_conversation_identity(
                ConversationIdentity::new("com.example.host", "same").unwrap(),
            ),
        )
        .unwrap();
    let second = registry
        .build_plan_with_context(
            &run,
            &InvocationContext::new().with_conversation_identity(
                ConversationIdentity::new("org.example.host", "same").unwrap(),
            ),
        )
        .unwrap();
    assert_ne!(absent.invocation_fingerprint, first.invocation_fingerprint);
    assert_ne!(first.invocation_fingerprint, second.invocation_fingerprint);
}

#[tokio::test]
async fn explicit_application_argument_remains_unchanged() {
    let registry = CommandRegistry::new("explicit", "Explicit authority test").register(
        CommandSpec::new(
            ["session", "select"],
            "Select session",
            "Receives explicit and ambient authorities independently.",
        )
        .with_arg(ArgSpec::string("agent_session_id", "Explicit session"))
        .uses_conversation_identity(),
        |context: CommandContext| async move {
            Ok(CommandOutput::structured(json!({
                "agentSessionId": context.plan.bound_args["agent_session_id"].value,
                "ambientIssuer": context.conversation_identity().map(ConversationIdentity::issuer),
            })))
        },
    );
    let response = registry
        .run_with_context(
            request(
                "session select $args.agent_session_id",
                json!({ "agent_session_id": "explicit-session" }),
            ),
            InvocationContext::new().with_conversation_identity(
                ConversationIdentity::new("com.example.host", "ambient-session").unwrap(),
            ),
        )
        .await
        .unwrap();
    let output = response.output.unwrap().structured.unwrap();
    assert_eq!(output["agentSessionId"], "explicit-session");
    assert_eq!(output["ambientIssuer"], "com.example.host");
}

#[tokio::test]
async fn framework_projections_never_serialize_raw_identity_or_private_digest() {
    let secret = "identity-secret-never-serialize";
    let identity = ConversationIdentity::new("com.example.host", secret).unwrap();
    let context = InvocationContext::new().with_conversation_identity(identity.clone());
    let registry = CommandRegistry::new("privacy", "Privacy boundary").register(
        CommandSpec::new(
            ["session", "inspect"],
            "Inspect session",
            "Serializes the handler context to prove private fields are skipped.",
        )
        .uses_conversation_identity(),
        |context: CommandContext| async move {
            Ok(CommandOutput::structured(
                serde_json::to_value(&context).unwrap(),
            ))
        },
    );
    let run = request("session inspect", json!({}));
    let plan = registry.build_plan_with_context(&run, &context).unwrap();
    let response = registry.run_with_context(run, context).await.unwrap();

    let digest = {
        let bytes = serde_json::to_vec(&json!([
            "conversation-identity",
            identity.version(),
            identity.issuer(),
            identity.id(),
        ]))
        .unwrap();
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };

    let preview = ResponseEnvelope::preview(plan.clone(), false);
    let replay = ReplayRecord {
        token: "replay-safe".to_string(),
        invocation_fingerprint: plan.invocation_fingerprint.clone(),
        operation_id: plan.operation_id.clone(),
        command_path: plan.command_path.clone(),
        lane: plan.lane,
        issued_at_unix_ms: 1,
        expires_at_unix_ms: 2,
        single_use: true,
    };
    let event = FrameworkEvent::from_envelope(&preview, Some(&mcp_twill::PlanFacts::from(&plan)));
    let help = registry.help(HelpRequest {
        command: Some("session inspect".to_string()),
        topic: None,
        detail: None,
    });
    let command_context_schema = serde_json::to_string(&schema_for!(CommandContext)).unwrap();
    let surfaces = [
        serde_json::to_string(&plan).unwrap(),
        serde_json::to_string(&response).unwrap(),
        serde_json::to_string(&preview).unwrap(),
        serde_json::to_string(&replay).unwrap(),
        serde_json::to_string(&event).unwrap(),
        serde_json::to_string(&help).unwrap(),
        serde_json::to_string(&registry.catalog()).unwrap(),
        command_context_schema.clone(),
    ];
    for surface in surfaces {
        assert!(!surface.contains(secret), "raw identity leaked: {surface}");
        assert!(
            !surface.contains(&digest),
            "private digest leaked: {surface}"
        );
    }
    assert!(!command_context_schema.contains("invocationContext"));
    assert!(!command_context_schema.contains("conversationIdentity"));
}

#[tokio::test]
async fn canonical_metadata_reaches_a_declaring_handler() -> anyhow::Result<()> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let result = call_mcp(
        observed_registry(true, seen.clone()),
        CliMcpServerConfig::default(),
        meta_with(Some(canonical("com.example.host", "canonical-id")), None),
        request("session current", json!({})),
    )
    .await?;
    assert_eq!(result.is_error, Some(false));
    let identity = seen.lock().unwrap()[0].0.clone().unwrap();
    assert_eq!(identity.version(), 1);
    assert_eq!(identity.issuer(), "com.example.host");
    assert_eq!(identity.id(), "canonical-id");
    Ok(())
}

#[tokio::test]
async fn default_compatibility_ignores_every_thread_id_shape() -> anyhow::Result<()> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    for thread_id in [json!("legacy-id"), json!(""), json!(42)] {
        let result = call_mcp(
            observed_registry(true, seen.clone()),
            CliMcpServerConfig::default(),
            meta_with(None, Some(thread_id)),
            request("session current", json!({})),
        )
        .await?;
        assert_eq!(result.is_error, Some(false));
    }
    let result = call_mcp(
        observed_registry(true, seen.clone()),
        CliMcpServerConfig::default(),
        meta_with(
            Some(canonical("com.example.host", "canonical-wins")),
            Some(json!("conflicting-ignored")),
        ),
        request("session current", json!({})),
    )
    .await?;
    assert_eq!(result.is_error, Some(false));
    for thread_id in [None, Some(json!(42))] {
        let result = call_mcp(
            observed_registry(true, seen.clone()),
            CliMcpServerConfig::default(),
            meta_with(
                Some(canonical("com.example.host", "canonical-wins")),
                thread_id,
            ),
            request("session current", json!({})),
        )
        .await?;
        assert_eq!(result.is_error, Some(false));
    }

    let seen = seen.lock().unwrap();
    assert!(seen[..3].iter().all(|(identity, _)| identity.is_none()));
    assert_eq!(seen[3].0.as_ref().unwrap().id(), "canonical-wins");
    assert_eq!(seen[3].1, seen[4].1);
    assert_eq!(seen[3].1, seen[5].1);
    Ok(())
}

#[tokio::test]
async fn trusted_codex_compatibility_normalizes_and_compares() -> anyhow::Result<()> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let config = CliMcpServerConfig::default().with_conversation_identity_compatibility(
        ConversationIdentityCompatibility::TrustedCodexThreadId,
    );
    let legacy = call_mcp(
        observed_registry(true, seen.clone()),
        config.clone(),
        meta_with(None, Some(json!("codex-thread"))),
        request("session current", json!({})),
    )
    .await?;
    assert_eq!(legacy.is_error, Some(false));

    let matching = call_mcp(
        observed_registry(true, seen.clone()),
        config.clone(),
        meta_with(
            Some(canonical("com.openai.codex", "codex-thread")),
            Some(json!("codex-thread")),
        ),
        request("session current", json!({})),
    )
    .await?;
    assert_eq!(matching.is_error, Some(false));
    let canonical_only = call_mcp(
        observed_registry(true, seen.clone()),
        config.clone(),
        meta_with(Some(canonical("com.openai.codex", "codex-thread")), None),
        request("session current", json!({})),
    )
    .await?;
    assert_eq!(canonical_only.is_error, Some(false));
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0.as_ref().unwrap().issuer(), "com.openai.codex");
        assert_eq!(seen[0].0.as_ref().unwrap().id(), "codex-thread");
        assert_eq!(seen[0].1, seen[1].1);
        assert_eq!(seen[0].1, seen[2].1);
    }

    let conflicting = call_mcp(
        observed_registry(true, Arc::new(Mutex::new(Vec::new()))),
        config,
        meta_with(
            Some(canonical("com.openai.codex", "canonical-secret")),
            Some(json!("legacy-secret")),
        ),
        request("session current", json!({})),
    )
    .await?;
    assert_eq!(conflicting.is_error, Some(true));
    let value = conflicting.structured_content.unwrap();
    assert_eq!(value["error"]["code"], "invalid_request_context");
    assert_eq!(
        value["diagnostics"][0]["location"],
        json!({ "type": "requestContext", "key": CONVERSATION_IDENTITY_META_KEY })
    );
    assert_eq!(
        value["error"]["details"],
        json!({
            "reason": "conflicting_observations",
            "sources": [CONVERSATION_IDENTITY_META_KEY, "threadId"],
        })
    );
    let rendered = value.to_string();
    assert!(!rendered.contains("canonical-secret"));
    assert!(!rendered.contains("legacy-secret"));
    Ok(())
}

#[tokio::test]
async fn malformed_canonical_payloads_have_stable_redacted_diagnostics() -> anyhow::Result<()> {
    let config = CliMcpServerConfig::default().with_conversation_identity_compatibility(
        ConversationIdentityCompatibility::TrustedCodexThreadId,
    );
    let cases = [
        (
            json!({ "issuer": "com.example.host", "id": "never-leak" }),
            Some("version"),
            "missing_field",
        ),
        (
            json!({ "version": 1, "issuer": "com.example.host", "id": "never-leak", "never-leak-field": true }),
            None,
            "unknown_field",
        ),
        (
            json!({ "version": 2, "issuer": "com.example.host", "id": "never-leak" }),
            Some("version"),
            "unsupported_version",
        ),
        (
            json!({ "version": 1, "issuer": "Example.COM", "id": "never-leak" }),
            Some("issuer"),
            "invalid_issuer",
        ),
        (
            json!({ "version": 1, "issuer": "com.example.host", "id": "" }),
            Some("id"),
            "empty_id",
        ),
    ];
    for (canonical, field, reason) in cases {
        let dispatches = Arc::new(Mutex::new(Vec::new()));
        let result = call_mcp(
            observed_registry(true, dispatches.clone()),
            config.clone(),
            meta_with(Some(canonical), Some(json!("valid-legacy"))),
            request("session current", json!({})),
        )
        .await?;
        assert_eq!(result.is_error, Some(true));
        assert!(dispatches.lock().unwrap().is_empty());
        let value = result.structured_content.unwrap();
        assert_eq!(value["error"]["code"], "invalid_request_context");
        assert_eq!(value["error"]["details"]["source"], "canonical");
        assert_eq!(
            value["error"]["details"]["key"],
            CONVERSATION_IDENTITY_META_KEY
        );
        match field {
            Some(field) => assert_eq!(value["error"]["details"]["field"], field),
            None => assert!(value["error"]["details"].get("field").is_none()),
        }
        assert_eq!(value["error"]["details"]["reason"], reason);
        assert!(!value.to_string().contains("never-leak"));
    }
    Ok(())
}

#[tokio::test]
async fn malformed_trusted_thread_id_fails_even_with_valid_canonical() -> anyhow::Result<()> {
    let config = CliMcpServerConfig::default().with_conversation_identity_compatibility(
        ConversationIdentityCompatibility::TrustedCodexThreadId,
    );
    for thread_id in [json!(""), json!(42)] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let result = call_mcp(
            observed_registry(true, seen.clone()),
            config.clone(),
            meta_with(
                Some(canonical("com.openai.codex", "canonical-never-leak")),
                Some(thread_id),
            ),
            request("session current", json!({})),
        )
        .await?;
        assert_eq!(result.is_error, Some(true));
        assert!(seen.lock().unwrap().is_empty());
        let value = result.structured_content.unwrap();
        assert_eq!(value["error"]["details"]["source"], "codexThreadId");
        assert_eq!(value["error"]["details"]["key"], "threadId");
        assert_eq!(
            value["error"]["details"]["reason"],
            "expected_non_empty_string"
        );
        assert!(!value.to_string().contains("canonical-never-leak"));
    }
    Ok(())
}

#[tokio::test]
async fn transport_integrity_applies_before_non_declaring_dispatch() -> anyhow::Result<()> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let registry = {
        let dispatches = dispatches.clone();
        CommandRegistry::new("plain", "Non-declaring server").register(
            CommandSpec::new(["session", "plain"], "Plain", "Plain command"),
            move |context: CommandContext| {
                let dispatches = dispatches.clone();
                async move {
                    dispatches.fetch_add(1, Ordering::SeqCst);
                    Ok(CommandOutput::structured(json!({
                        "hasIdentity": context.conversation_identity().is_some(),
                    })))
                }
            },
        )
    };
    let invalid = call_mcp(
        registry.clone(),
        CliMcpServerConfig::default(),
        meta_with(Some(json!({ "version": 1 })), None),
        request("session plain", json!({})),
    )
    .await?;
    assert_eq!(invalid.is_error, Some(true));
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    let trusted = CliMcpServerConfig::default().with_conversation_identity_compatibility(
        ConversationIdentityCompatibility::TrustedCodexThreadId,
    );
    let invalid_legacy = call_mcp(
        registry.clone(),
        trusted.clone(),
        meta_with(None, Some(json!(42))),
        request("session plain", json!({})),
    )
    .await?;
    assert_eq!(invalid_legacy.is_error, Some(true));
    let conflict = call_mcp(
        registry.clone(),
        trusted,
        meta_with(
            Some(canonical("com.openai.codex", "canonical")),
            Some(json!("legacy")),
        ),
        request("session plain", json!({})),
    )
    .await?;
    assert_eq!(conflict.is_error, Some(true));
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    let valid = call_mcp(
        registry,
        CliMcpServerConfig::default(),
        meta_with(Some(canonical("com.example.host", "valid")), None),
        request("session plain", json!({})),
    )
    .await?;
    assert_eq!(valid.is_error, Some(false));
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    assert_eq!(
        valid.structured_content.unwrap()["output"]["structured"]["hasIdentity"],
        false
    );
    Ok(())
}

#[tokio::test]
async fn non_execution_surfaces_ignore_malformed_identity_metadata() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server = CliMcpServer::new(contract_registry())?;
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let malformed = meta_with(Some(json!({ "version": 1, "id": "raw-secret" })), None);

    let mut help =
        CallToolRequestParams::new("help").with_arguments(json_object(HelpRequest::default()));
    help.meta = Some(malformed.clone());
    assert_eq!(client.call_tool(help).await?.is_error, Some(false));

    let resource = client
        .read_resource(ReadResourceRequestParams::new("cli://catalog").with_meta(malformed.clone()))
        .await?;
    assert!(!resource.contents.is_empty());
    let prompt = client
        .get_prompt(GetPromptRequestParams::new("getting_started").with_meta(malformed))
        .await?;
    assert!(!prompt.messages.is_empty());

    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn task_and_ordinary_execution_share_identity_and_fingerprint() -> anyhow::Result<()> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server = CliMcpServer::new(observed_registry(true, seen.clone()))?;
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let meta = meta_with(Some(canonical("com.example.host", "task-identity")), None);

    let mut ordinary = CallToolRequestParams::new("run")
        .with_arguments(json_object(request("session current", json!({}))));
    ordinary.meta = Some(meta.clone());
    assert_eq!(client.call_tool(ordinary).await?.is_error, Some(false));

    let mut task = CallToolRequestParams::new("run")
        .with_arguments(json_object(request("session current", json!({}))))
        .with_task(serde_json::Map::new());
    task.meta = Some(meta);
    let created = client
        .send_request(ClientRequest::CallToolRequest(Request::new(task)))
        .await?;
    assert!(matches!(created, ServerResult::CreateTaskResult(_)));
    for _ in 0..40 {
        if seen.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, seen[1].0);
        assert_eq!(seen[0].1, seen[1].1);
    }

    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[test]
fn request_context_error_code_and_location_are_public_and_redacted() {
    let error = FrameworkError::InvalidConversationIdentity {
        observation_source: "canonical".to_string(),
        key: CONVERSATION_IDENTITY_META_KEY.to_string(),
        field: Some("issuer".to_string()),
        reason: "invalid_issuer".to_string(),
        expected: Some("a lowercase reverse-DNS name".to_string()),
    };
    let envelope = ResponseEnvelope::framework_error(error, None, None);
    assert_eq!(
        envelope.error.as_ref().unwrap().code,
        ErrorCode::InvalidRequestContext
    );
    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["status"], "invalidInput");
    assert_eq!(
        value["diagnostics"][0]["location"],
        json!({ "type": "requestContext", "key": CONVERSATION_IDENTITY_META_KEY })
    );
    assert!(value.get("steering").is_none());
}
