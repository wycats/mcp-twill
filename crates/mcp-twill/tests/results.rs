//! RFC 0014 acceptance tests: application-owned result contracts.

use std::{borrow::Cow, collections::BTreeMap, error::Error, fmt};

use mcp_twill::{
    ApplicationError, ApplicationErrorBody, ApplicationErrorDecl, ApplicationErrorUse,
    ApplicationMessageDecl, ApplicationOutput, ApplicationOutputResult, ApplicationRecovery,
    ApplicationRecoveryKey, ApplicationRecoverySelection, ApplicationResult,
    ApplicationResultContract, ApplicationSuccess, CommandContext, CommandExecutionOutcome,
    CommandOutput, CommandRegistry, CommandSpec, DynamicApplicationError, DynamicApplicationResult,
    FrameworkError, FrameworkEvent, Grant, HelpRequest, Listing, OutputContract, PlanFacts,
    Resource, ResourceDecl, ResponseEnvelope, ResponseProfile, ResultContractBoundary,
    ResultContractReason, RunMode, RunRequest, application_error_set, contract,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BrowserStatus {
    ready: bool,
    active_tab: Option<String>,
}

struct Tab;

impl Resource for Tab {
    const NAME: &'static str = "tab";
}

#[derive(Debug, Serialize, JsonSchema)]
struct NewTabResult {
    title: String,
}

#[derive(JsonSchema)]
struct SerializationFailure;

impl Serialize for SerializationFailure {
    fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("adversarial serializer secret"))
    }
}

#[derive(JsonSchema)]
struct SchemaMismatch {
    #[schemars(rename = "ok")]
    _ok: bool,
}

impl Serialize for SchemaMismatch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        json!({ "secretPayload": true }).serialize(serializer)
    }
}

async fn serialization_failure(_: CommandContext) -> ApplicationResult<SerializationFailure> {
    Ok(ApplicationSuccess::value(SerializationFailure))
}

async fn schema_mismatch(_: CommandContext) -> ApplicationResult<SchemaMismatch> {
    Ok(ApplicationSuccess::value(SchemaMismatch { _ok: true }))
}

async fn new_tab_with_references(
    _: CommandContext,
) -> ApplicationOutputResult<
    mcp_twill::Listed<Tab, mcp_twill::Granted<Tab, ApplicationSuccess<NewTabResult>>>,
> {
    Ok(ApplicationSuccess::value(NewTabResult {
        title: "New tab".to_string(),
    })
    .grant(Grant::<Tab>::new("tab-1"))
    .listing(Listing::<Tab>::new(["tab-1", "tab-2"])))
}

#[derive(Debug)]
enum BrowserFailure {
    SessionRequired,
    TargetMissing(String),
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
            ApplicationErrorDecl::new("target_missing", "The page target is unavailable")
                .runtime_message(24)
                .details_schema(json!({
                    "type": "object",
                    "properties": { "target": { "type": "string" } },
                    "required": ["target"],
                    "additionalProperties": false,
                })),
        ]
    }

    fn code(&self) -> &'static str {
        match self {
            Self::SessionRequired => "session_required",
            Self::TargetMissing(_) => "target_missing",
        }
    }

    fn details(&self) -> Value {
        match self {
            Self::SessionRequired => json!({}),
            Self::TargetMissing(target) => json!({ "target": target }),
        }
    }

    fn runtime_message(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::SessionRequired => None,
            Self::TargetMissing(target) => Some(Cow::Owned(format!("missing {target}\n\u{202E}"))),
        }
    }

    fn recovery(&self) -> ApplicationRecoverySelection {
        match self {
            Self::SessionRequired => ApplicationRecoverySelection::Declared,
            Self::TargetMissing(_) => {
                ApplicationRecoverySelection::Only(vec![ApplicationRecoveryKey::Action(
                    "refresh_page".to_string(),
                )])
            }
        }
    }
}

application_error_set! {
    struct BrowserErrors for BrowserFailure {
        ApplicationErrorUse::new("session_required")
            .recover_with("session start")
            .at_most_one_recovery(),
        ApplicationErrorUse::new("target_missing")
            .recover_with("tabs list")
            .recover_by("refresh_page", "Refresh the page")
            .at_most_one_recovery(),
    }
}

application_error_set! {
    struct CapabilityErrors for BrowserFailure {
        ApplicationErrorUse::new("session_required")
            .for_capability("validated-build"),
    }
}

async fn status_success(
    _: CommandContext,
) -> ApplicationResult<BrowserStatus, BrowserFailure, BrowserErrors> {
    Ok(ApplicationSuccess::value(BrowserStatus {
        ready: true,
        active_tab: Some("tab-1".to_string()),
    }))
}

async fn session_failure(
    _: CommandContext,
) -> ApplicationResult<BrowserStatus, BrowserFailure, BrowserErrors> {
    Err(BrowserFailure::SessionRequired.into())
}

async fn target_failure(
    _: CommandContext,
) -> ApplicationResult<BrowserStatus, BrowserFailure, BrowserErrors> {
    Err(BrowserFailure::TargetMissing("tab-secret".to_string()).into())
}

async fn capability_failure(
    _: CommandContext,
) -> ApplicationResult<BrowserStatus, BrowserFailure, CapabilityErrors> {
    Err(BrowserFailure::SessionRequired.into())
}

async fn dynamic_application_failure(_: CommandContext) -> DynamicApplicationResult {
    Err(DynamicApplicationError::new("session_required").into())
}

fn request(command: &str) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: BTreeMap::new(),
        stdin: None,
        output: None,
        mode: RunMode::Execute,
        approval: None,
        dry_run: false,
    }
}

fn build_error(result: mcp_twill::Result<CommandRegistry>) -> FrameworkError {
    match result {
        Ok(_) => panic!("expected registry construction to fail"),
        Err(error) => error,
    }
}

fn result_registry() -> CommandRegistry {
    CommandRegistry::build("results", "Result-aware commands", |server| {
        server.command("session start", |command| {
            command
                .summary("Start session")
                .description("Start a browser session")
                .handle(|_| async { Ok(CommandOutput::structured(json!({ "started": true }))) });
        });
        server.command("tabs list", |command| {
            command
                .summary("List tabs")
                .description("List browser tabs")
                .handle(|_| async { Ok(CommandOutput::structured(json!([]))) });
        });
        server.command("browser status", |command| {
            command
                .summary("Browser status")
                .description("Inspect browser status")
                .handle_result(status_success);
        });
        server.command("browser require session", |command| {
            command
                .summary("Require session")
                .description("Return the declared session error")
                .handle_result(session_failure);
        });
        server.command("browser find target", |command| {
            command
                .summary("Find target")
                .description("Return a bounded runtime error")
                .handle_result(target_failure);
        });
    })
    .expect("result registry builds")
}

#[test]
fn public_declaration_spellings_are_stable() {
    assert_eq!(
        serde_json::to_value(ApplicationMessageDecl::RuntimeBounded {
            max_scalar_values: 256,
        })
        .unwrap(),
        json!({ "runtimeBounded": { "maxScalarValues": 256 } })
    );
    assert_eq!(
        serde_json::to_value(ResultContractBoundary::ApplicationError).unwrap(),
        json!("applicationError")
    );
    assert_eq!(
        serde_json::to_value(ResultContractReason::InvalidRecoverySelection).unwrap(),
        json!("invalidRecoverySelection")
    );
    let schema = serde_json::to_value(schemars::schema_for!(ResultContractReason)).unwrap();
    assert!(schema.to_string().contains("invalidRecoverySelection"));
}

#[test]
fn absent_application_contract_preserves_legacy_serialization() {
    let value = serde_json::to_value(OutputContract::default()).unwrap();
    assert!(value.get("application").is_none());
    let event = FrameworkEvent::parse_failure("invalid input");
    assert!(
        serde_json::to_value(event)
            .unwrap()
            .get("applicationErrorCode")
            .is_none()
    );
}

#[tokio::test]
async fn typed_success_is_validated_once_and_projects_compact_json() {
    let registry = result_registry();
    let CommandExecutionOutcome::Success(response) = registry
        .run(request("browser status"))
        .await
        .expect("framework succeeds")
    else {
        panic!("expected success");
    };
    let output = response.output.expect("result output");
    assert_eq!(
        output.structured,
        Some(json!({ "ready": true, "activeTab": "tab-1" }))
    );
    assert_eq!(
        output.text.as_deref(),
        Some(r#"{"activeTab":"tab-1","ready":true}"#)
    );
}

#[tokio::test]
async fn application_error_stays_out_of_framework_error_and_shapes_recovery() {
    let registry = result_registry();
    let CommandExecutionOutcome::ApplicationError { plan, error } = registry
        .run(request("browser require session"))
        .await
        .expect("application failure is not a framework error")
    else {
        panic!("expected declared application error");
    };
    assert_eq!(error.code, "session_required");
    assert_eq!(error.message, "No browser session is available");
    assert_eq!(
        error.recoveries,
        vec![ApplicationRecovery::Operation {
            operation_id: "session start".to_string(),
        }]
    );

    let envelope = ResponseEnvelope::application_error(plan, error, ResponseProfile::Structured);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "application_error");
    assert_eq!(
        value["error"]["details"]["applicationCode"],
        "session_required"
    );
    assert_eq!(
        value["steering"][0]["request"]["arguments"]["command"],
        "session start"
    );
    assert!(value.get("plan").is_none());
}

#[tokio::test]
async fn bounded_messages_escape_and_events_retain_only_declared_code() {
    let registry = result_registry();
    let CommandExecutionOutcome::ApplicationError { plan, error } = registry
        .run(request("browser find target"))
        .await
        .expect("application failure is valid")
    else {
        panic!("expected declared application error");
    };
    assert_eq!(error.message, "missing tab-secret\\n…");
    assert!(!error.message.contains('\n'));
    assert!(!error.message.contains('\u{202E}'));
    assert_eq!(
        error.recoveries,
        vec![ApplicationRecovery::Action {
            code: "refresh_page".to_string(),
            summary: "Refresh the page".to_string(),
        }]
    );
    let envelope = ResponseEnvelope::application_error(plan.clone(), error, ResponseProfile::Debug);
    let plan_facts = PlanFacts::from(&plan);
    let event = FrameworkEvent::from_envelope(&envelope, Some(&plan_facts));
    let value = serde_json::to_value(event).unwrap();
    assert_eq!(value["applicationErrorCode"], "target_missing");
    let serialized = value.to_string();
    assert!(!serialized.contains("tab-secret"));
    assert!(!serialized.contains("refresh_page"));
}

#[tokio::test]
async fn dynamic_contract_is_authoritative_and_mismatch_is_redacted() {
    let contract = ApplicationResultContract::new(json!({
        "type": "object",
        "properties": { "ok": { "type": "boolean" } },
        "required": ["ok"],
        "additionalProperties": false,
    }));
    let registry = CommandRegistry::new("dynamic", "Dynamic results").register_dynamic(
        CommandSpec::new(["dynamic"], "Dynamic", "Dynamic result").with_output(OutputContract {
            application: Some(contract),
            ..OutputContract::default()
        }),
        |_context| async {
            Ok::<_, mcp_twill::DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "secret": "must-not-leak"
            })))
        },
    );
    registry.validate_results().unwrap();
    let error = registry.run(request("dynamic")).await.unwrap_err();
    assert_eq!(
        error,
        FrameworkError::ResultContractViolation {
            boundary: ResultContractBoundary::Success,
            reason: ResultContractReason::SchemaMismatch,
        }
    );
    let plan = registry.build_plan(&request("dynamic")).unwrap();
    let envelope = ResponseEnvelope::framework_error(error, None, Some(plan));
    let serialized = serde_json::to_string(&envelope).unwrap();
    assert!(!serialized.contains("must-not-leak"));
    assert!(serialized.contains("result_contract_violation"));
}

#[tokio::test]
async fn typed_serialization_and_schema_failures_are_distinct_and_redacted() {
    for (name, registry, reason, secret) in [
        (
            "serialize",
            CommandRegistry::new("invalid", "Invalid").register_result(
                CommandSpec::new(["serialize"], "Serialize", "Serialize"),
                serialization_failure,
            ),
            ResultContractReason::SerializationFailed,
            "adversarial serializer secret",
        ),
        (
            "mismatch",
            CommandRegistry::new("invalid", "Invalid").register_result(
                CommandSpec::new(["mismatch"], "Mismatch", "Mismatch"),
                schema_mismatch,
            ),
            ResultContractReason::SchemaMismatch,
            "secretPayload",
        ),
    ] {
        registry.validate_results().unwrap();
        let error = registry.run(request(name)).await.unwrap_err();
        assert_eq!(
            error,
            FrameworkError::ResultContractViolation {
                boundary: ResultContractBoundary::Success,
                reason,
            }
        );
        let envelope = ResponseEnvelope::framework_error(
            error,
            None,
            Some(registry.build_plan(&request(name)).unwrap()),
        );
        assert!(!serde_json::to_string(&envelope).unwrap().contains(secret));
    }
}

#[test]
fn builder_and_low_level_typed_registration_are_projection_equivalent() {
    let builder = CommandRegistry::build("equivalent", "Equivalent", |server| {
        server.command("status", |command| {
            command
                .summary("Status")
                .description("Status")
                .handle_result(status_success);
        });
        server.command("session start", |command| {
            command
                .summary("Start")
                .description("Start")
                .handle(|_| async { Ok(CommandOutput::structured(json!({}))) });
        });
        server.command("tabs list", |command| {
            command
                .summary("List")
                .description("List")
                .handle(|_| async { Ok(CommandOutput::structured(json!([]))) });
        });
    })
    .unwrap();
    let low_level = CommandRegistry::new("equivalent", "Equivalent")
        .register_result(
            CommandSpec::new(["status"], "Status", "Status"),
            status_success,
        )
        .register(
            CommandSpec::new(["session", "start"], "Start", "Start"),
            |_| async { Ok(CommandOutput::structured(json!({}))) },
        )
        .register(
            CommandSpec::new(["tabs", "list"], "List", "List"),
            |_| async { Ok(CommandOutput::structured(json!([]))) },
        );
    low_level.validate_results().unwrap();
    assert_eq!(builder.catalog(), low_level.catalog());
    assert_eq!(builder.catalog_identity(), low_level.catalog_identity());
    assert!(contract::check_result_projection(&builder).is_empty());
}

#[test]
fn invalid_handler_contract_pairings_fail_before_serving() {
    let explicit = ApplicationResultContract::for_type::<BrowserStatus>();
    let typed = CommandRegistry::new("invalid", "Invalid").register_result(
        CommandSpec::new(["typed"], "Typed", "Typed").with_output(OutputContract {
            application: Some(explicit.clone()),
            ..OutputContract::default()
        }),
        status_success,
    );
    assert!(
        typed
            .validate_results()
            .unwrap_err()
            .to_string()
            .contains("explicit")
    );

    let legacy = CommandRegistry::new("invalid", "Invalid").register(
        CommandSpec::new(["legacy"], "Legacy", "Legacy").with_output(OutputContract {
            application: Some(explicit),
            ..OutputContract::default()
        }),
        |_| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert!(
        legacy
            .validate_results()
            .unwrap_err()
            .to_string()
            .contains("legacy")
    );
}

#[test]
fn result_catalog_help_and_hash_change_reversibly() {
    let registry = result_registry();
    let operation = registry
        .catalog()
        .operations
        .into_iter()
        .find(|operation| operation.name() == "browser status")
        .unwrap();
    assert!(operation.output.application.is_some());
    let help = registry.help(HelpRequest {
        command: Some("browser require session".to_string()),
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("Expected application errors:"));
    assert!(help.text.contains("`session_required`"));
    assert!(contract::check_result_projection(&registry).is_empty());
}

#[tokio::test]
async fn dynamic_application_errors_use_the_explicit_contract() {
    let contract = ApplicationResultContract::new(json!({ "type": "object" })).with_error_spec(
        mcp_twill::ApplicationErrorSpec {
            code: "session_required".to_string(),
            summary: "No browser session is available".to_string(),
            message: ApplicationMessageDecl::DeclarationSummary,
            details_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
            capability: None,
            recoveries: Vec::new(),
            recovery_cardinality: mcp_twill::RecoveryCardinality::Any,
        },
    );
    let registry = CommandRegistry::new("dynamic", "Dynamic").register_dynamic(
        CommandSpec::new(["dynamic", "fail"], "Fail", "Fail dynamically").with_output(
            OutputContract {
                application: Some(contract),
                ..OutputContract::default()
            },
        ),
        dynamic_application_failure,
    );
    registry.validate_results().unwrap();
    let CommandExecutionOutcome::ApplicationError { error, .. } = registry
        .run(request("dynamic fail"))
        .await
        .expect("declared failure")
    else {
        panic!("expected application error");
    };
    assert_eq!(error.code, "session_required");
    assert_eq!(error.message, "No browser session is available");
}

#[test]
fn capability_bound_errors_derive_only_bootstrap_recovery() {
    let registry = CommandRegistry::build("capability", "Capability-bound errors", |server| {
        server.capability(
            mcp_twill::CapabilityDecl::new("validated-build", "Validated build proof")
                .carried_by("validation_token"),
        );
        server.command("build validate", |command| {
            command
                .summary("Validate")
                .description("Bootstrap validation")
                .provides("validated-build")
                .handle(|_| async { Ok(CommandOutput::structured(json!({}))) });
        });
        for name in ["build refresh", "build recheck"] {
            server.command(name, |command| {
                command
                    .summary("Refresh")
                    .description("Refresh validation")
                    .arg(mcp_twill::arg::string("validation_token").summary("Existing proof"))
                    .requires("validated-build")
                    .provides("validated-build")
                    .handle(|_| async { Ok(CommandOutput::structured(json!({}))) });
            });
        }
        server.command("deploy publish", |command| {
            command
                .summary("Publish")
                .description("Publish validated build")
                .arg(mcp_twill::arg::string("validation_token").summary("Validation proof"))
                .requires("validated-build")
                .handle_result(capability_failure);
        });
    })
    .unwrap();
    let publish = registry
        .catalog()
        .operations
        .into_iter()
        .find(|operation| operation.name() == "deploy publish")
        .unwrap();
    let recoveries = &publish.output.application.unwrap().errors[0].recoveries;
    assert_eq!(
        recoveries,
        &[mcp_twill::ApplicationRecoveryDecl::Operation {
            operation_id: "build validate".to_string(),
        }]
    );
}

#[tokio::test]
async fn result_aware_legacy_denial_and_handler_sources_are_redacted_publicly() {
    async fn denied(_: CommandContext) -> ApplicationResult<BrowserStatus> {
        Err(FrameworkError::capability_denied("secret-cap", "secret-detail").into())
    }
    let registry = CommandRegistry::new("redaction", "Redaction")
        .register_result(CommandSpec::new(["denied"], "Denied", "Denied"), denied);
    registry.validate_results().unwrap();
    let direct = registry.run(request("denied")).await.unwrap_err();
    assert_eq!(
        direct,
        FrameworkError::Handler(
            "result-aware handler returned legacy capability denial".to_string()
        )
    );
    let plan = registry.build_plan(&request("denied")).unwrap();
    let envelope = ResponseEnvelope::framework_error(direct, None, Some(plan));
    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["error"]["code"], "handler_failed");
    assert_eq!(value["error"]["message"], "Command handler failed");
    assert_eq!(value["error"]["details"], json!({}));
    let serialized = value.to_string();
    assert!(!serialized.contains("secret-cap"));
    assert!(!serialized.contains("secret-detail"));
}

#[test]
fn application_error_body_schema_is_public_and_stable() {
    let body = ApplicationErrorBody {
        code: "session_required".to_string(),
        message: "No browser session is available".to_string(),
        details: json!({}),
        recoveries: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(body).unwrap()["code"],
        "session_required"
    );
    let schema = serde_json::to_value(schemars::schema_for!(ApplicationErrorBody)).unwrap();
    assert!(schema.to_string().contains("recoveries"));
}

#[tokio::test]
async fn typed_resource_wrappers_preserve_result_and_reference_authority() {
    let registry = CommandRegistry::build("resources", "Result resources", |server| {
        server.resource(
            ResourceDecl::new("tab", "A browser tab")
                .uri("test://tab/{id}")
                .expiry("Tabs expire when the browser closes"),
        );
        server.command("tabs new", |command| {
            command
                .summary("New tab")
                .description("Create and enumerate tabs")
                .handle_result(new_tab_with_references);
        });
    })
    .unwrap();
    let catalog = registry.catalog();
    let operation = &catalog.operations[0];
    assert_eq!(operation.grants, ["tab"]);
    assert_eq!(operation.enumerates, ["tab"]);
    assert!(operation.output.application.is_some());
    let CommandExecutionOutcome::Success(response) = registry
        .run(request("tabs new"))
        .await
        .expect("result succeeds")
    else {
        panic!("expected success");
    };
    let output = response.output.unwrap();
    assert_eq!(output.structured.unwrap()["title"], "New tab");
    assert_eq!(output.grants[0].uri, "test://tab/tab-1");
    assert_eq!(
        output
            .listings
            .iter()
            .map(|reference| reference.uri.as_str())
            .collect::<Vec<_>>(),
        ["test://tab/tab-1", "test://tab/tab-2"]
    );
}

fn explicit_schema_registry(schema: Value) -> CommandRegistry {
    CommandRegistry::new("schema", "Schema validation").register_dynamic(
        CommandSpec::new(["schema"], "Schema", "Schema").with_output(OutputContract {
            application: Some(ApplicationResultContract::new(schema)),
            ..OutputContract::default()
        }),
        |_context| async {
            Ok::<_, mcp_twill::DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
        },
    )
}

#[test]
fn schema_dialect_canonicalizes_supported_forms_and_rejects_drift() {
    let left = explicit_schema_registry(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": ["null", "string"],
    }));
    let right = explicit_schema_registry(json!({ "type": ["string", "null"] }));
    left.validate_results().unwrap();
    right.validate_results().unwrap();
    assert_eq!(left.catalog_identity(), right.catalog_identity());
    let catalog = left.catalog();
    let schema = &catalog.operations[0]
        .output
        .application
        .as_ref()
        .unwrap()
        .success_schema;
    assert_eq!(schema, &json!({ "type": ["string", "null"] }));

    for unsupported in [
        json!(true),
        json!({ "type": "number", "minimum": 0 }),
        json!({ "anyOf": [{ "type": "string" }, { "type": "null" }] }),
        json!({ "$ref": "https://example.com/schema" }),
        json!({
            "type": "object",
            "properties": { "value": { "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string" } }
        }),
        json!({
            "$defs": { "unused": { "type": "string" } },
            "type": "object"
        }),
        json!({ "const": 9_007_199_254_740_992_u64 }),
    ] {
        assert!(
            explicit_schema_registry(unsupported)
                .validate_results()
                .is_err(),
            "unsupported schema must fail"
        );
    }
}

#[test]
fn typed_schema_normalizes_rust_storage_constraints() {
    #[derive(Serialize, JsonSchema)]
    struct Numbers {
        signed: i32,
        unsigned: u64,
        ratio: f32,
        optional: Option<i16>,
    }
    let contract = ApplicationResultContract::for_type::<Numbers>();
    let text = contract.success_schema.to_string();
    assert!(!text.contains("format"));
    assert!(!text.contains("minimum"));
    assert!(!text.contains("maximum"));
    assert!(text.contains("null"));
}

#[test]
fn builder_rejects_duplicate_handler_and_contract_authority() {
    let duplicate_handler = build_error(CommandRegistry::build("invalid", "Invalid", |server| {
        server.command("duplicate", |command| {
            command
                .summary("Duplicate")
                .description("Duplicate")
                .handle_result(status_success)
                .handle(|_| async { Ok(CommandOutput::structured(json!({}))) });
        });
    }));
    assert!(
        duplicate_handler
            .to_string()
            .contains("more than one handler")
    );

    let duplicate_contract = build_error(CommandRegistry::build("invalid", "Invalid", |server| {
        server.command("duplicate", |command| {
            command
                .summary("Duplicate")
                .description("Duplicate")
                .result_contract(ApplicationResultContract::new(json!({ "type": "object" })))
                .result_contract(ApplicationResultContract::new(json!({ "type": "object" })))
                .handle_dynamic(|_| async {
                    Ok::<_, mcp_twill::DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
                });
        });
    }));
    assert!(duplicate_contract.to_string().contains("result_contract"));
}

#[test]
fn dynamic_builder_contract_order_is_not_authority() {
    fn build(contract_first: bool) -> CommandRegistry {
        CommandRegistry::build("dynamic", "Dynamic", |server| {
            server.command("dynamic", |command| {
                command.summary("Dynamic").description("Dynamic");
                if contract_first {
                    command.result_contract(ApplicationResultContract::new(json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false,
                    })));
                }
                command.handle_dynamic(|_| async {
                    Ok::<_, mcp_twill::DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
                });
                if !contract_first {
                    command.result_contract(ApplicationResultContract::new(json!({
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false,
                    })));
                }
            });
        })
        .unwrap()
    }
    let before = build(true);
    let after = build(false);
    assert_eq!(before.catalog(), after.catalog());
    assert_eq!(before.catalog_identity(), after.catalog_identity());
}

#[tokio::test]
async fn invalid_dynamic_code_and_details_are_redacted_contract_violations() {
    let spec = mcp_twill::ApplicationErrorSpec {
        code: "known".to_string(),
        summary: "Known failure".to_string(),
        message: ApplicationMessageDecl::DeclarationSummary,
        details_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        }),
        capability: None,
        recoveries: Vec::new(),
        recovery_cardinality: mcp_twill::RecoveryCardinality::Any,
    };
    for (code, details, reason, secret) in [
        (
            "undeclared_secret",
            json!({}),
            ResultContractReason::UndeclaredCode,
            "undeclared_secret",
        ),
        (
            "known",
            json!({ "secret": true }),
            ResultContractReason::InvalidDetails,
            "secret",
        ),
    ] {
        let contract = ApplicationResultContract::new(json!({ "type": "object" }))
            .with_error_spec(spec.clone());
        let registry = CommandRegistry::new("invalid", "Invalid").register_dynamic(
            CommandSpec::new(["invalid"], "Invalid", "Invalid").with_output(OutputContract {
                application: Some(contract),
                ..OutputContract::default()
            }),
            move |_| {
                let details = details.clone();
                async move {
                    Err::<ApplicationSuccess<Value>, _>(
                        DynamicApplicationError::new(code).details(details).into(),
                    )
                }
            },
        );
        registry.validate_results().unwrap();
        let failure = registry.run(request("invalid")).await.unwrap_err();
        assert_eq!(
            failure,
            FrameworkError::ResultContractViolation {
                boundary: ResultContractBoundary::ApplicationError,
                reason,
            }
        );
        let envelope = ResponseEnvelope::framework_error(
            failure,
            None,
            Some(registry.build_plan(&request("invalid")).unwrap()),
        );
        assert!(!serde_json::to_string(&envelope).unwrap().contains(secret));
    }
}
