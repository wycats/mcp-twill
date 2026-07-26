use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use mcp_twill::{
    CONVERSATION_IDENTITY_META_KEY, CliMcpServer, CommandRegistry, DefaultPermissionAuthorizer,
    FrameworkHelpProjection, HelpDetail, HelpRequest, HelpTopic, InMemoryEventSink,
    McpProtocolTarget, NativeConfirmationBridge, NativeConfirmationBridgeError,
    NativeConfirmationDecision, NativeConfirmationRequest, NativeConfirmationRoute,
    NativeToolSurface, PermissionAuthorizer, PermissionDecision, check_native_surface_projection,
    verify_catalog_coverage,
};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, Meta},
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod specimen {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/mcp-twill/examples/issues_server/site_specimen.rs"
    ));
}

const FORMAT_VERSION: u32 = 1;
const GENERATOR_NAME: &str = "cargo xtask export-site-evidence";
const GENERATOR_COMMAND: &str = "cargo xtask export-site-evidence";
const EVIDENCE_DIR: &str = "site/public/evidence";
const OPERATION_ID: &str = "issues.create";
const NATIVE_TOOL_NAME: &str = "issues_create";
const SAMPLE_TITLE: &str = "Crash on launch";
const SAMPLE_BODY: &str = "The app exits after the splash screen.";
const PRIVATE_ISSUER: &str = "com.example.command-woven.private";
const PRIVATE_ID: &str = "conversation-secret-never-serialize";
const ALTERNATE_PRIVATE_ISSUER: &str = "com.example.command-woven.alternate";
const ALTERNATE_PRIVATE_ID: &str = "alternate-conversation-secret-never-serialize";

const SOURCE_PATHS: &[&str] = &[
    "crates/mcp-twill/examples/issues_server.rs",
    "crates/mcp-twill/examples/issues_server/site_specimen.rs",
    "crates/mcp-twill/src/builder.rs",
    "crates/mcp-twill/src/catalog.rs",
    "crates/mcp-twill/src/contract.rs",
    "crates/mcp-twill/src/conversation_identity.rs",
    "crates/mcp-twill/src/native_surfaces.rs",
    "crates/mcp-twill/src/presentation.rs",
    "crates/mcp-twill/src/registry.rs",
    "crates/mcp-twill/src/rmcp_adapter.rs",
    "docs/adoption/visible-browser-lab/baseline/catalog-measurement.json",
    "crates/mcp-twill/tests/fixtures/vbl/v0.4.9/manifest.json",
    "xtask/src/site_evidence.rs",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum TitleRuleId {
    Unconstrained,
    NonEmpty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum DestinationId {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ConfirmationId {
    Generic,
    TitleInterpolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum PrivateContextId {
    None,
    ConversationIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Selection {
    title_rule: TitleRuleId,
    destination: DestinationId,
    confirmation: ConfirmationId,
    private_context: PrivateContextId,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Generator {
    name: String,
    version: u32,
    command: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Source {
    repository: String,
    paths: Vec<String>,
    source_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Defaults {
    selection: Selection,
    profile: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Control {
    id: String,
    fact_id: String,
    label: String,
    options: Vec<ControlOption>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ControlOption {
    value: String,
    label: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct EvidenceBundle {
    format_version: u32,
    generated_by: Generator,
    source: Source,
    defaults: Defaults,
    controls: Vec<Control>,
    variants: Vec<ScenarioVariant>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ScenarioVariant {
    id: String,
    selection: Selection,
    declaration: Declaration,
    rust_configuration: RustConfiguration,
    catalog_operation: Value,
    help: Value,
    argument_schema: Value,
    compact: CompactProjection,
    native: NativeProjection,
    mcp_surface_comparison: McpSurfaceComparison,
    confirmation: ConfirmationProjection,
    host_preview: HostPreview,
    plan: Value,
    trace: Vec<TraceStep>,
    result: Value,
    fingerprints: Fingerprints,
    semantic_anchors: Vec<SemanticAnchor>,
    comparison_targets: Vec<ComparisonTarget>,
    privacy: PrivacyEvidence,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Declaration {
    source_path: String,
    text: String,
    facts: Vec<DeclarationFact>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct DeclarationFact {
    id: String,
    label: String,
    value: Value,
    display_value: String,
    target_ids: Vec<String>,
    code_presence: DeclarationCodePresence,
    code_ranges: Vec<DeclarationCodeRange>,
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum DeclarationCodePresence {
    Rendered,
    Omitted,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct DeclarationCodeRange {
    start_line: u32,
    end_line: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RustConfiguration {
    title_min_length: Option<u32>,
    permissions: Vec<String>,
    confirmation_kind: ConfirmationId,
    uses_conversation_identity: bool,
    task_support: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CompactProjection {
    tools: Value,
    selected_tool: Value,
    surface_identity: Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct NativeProjection {
    surface: Value,
    tool: Value,
    surface_identity: Value,
}

#[derive(Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpSurfaceComparison {
    operation_id: String,
    compact: McpSurfaceFacts,
    native: McpSurfaceFacts,
}

#[derive(Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpSurfaceFacts {
    tool_name: String,
    tool_inventory: Vec<String>,
    required_inputs: Vec<String>,
    input_fields: Vec<String>,
    has_argument_map: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ConfirmationProjection {
    operation_id: String,
    branch: Value,
    title: String,
    message: String,
    arguments: Value,
    invocation_fingerprint: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HostPreview {
    label: String,
    title: String,
    description: String,
    argument_fields: Vec<HostArgumentField>,
    effect_badges: Vec<String>,
    confirmation: HostConfirmation,
    task_support: String,
    private_context: HostPrivateContext,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HostArgumentField {
    name: String,
    label: String,
    required: bool,
    constraint: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HostConfirmation {
    title: String,
    message: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HostPrivateContext {
    declared: bool,
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum TraceStage {
    Select,
    Bind,
    Validate,
    Authorize,
    Realize,
    Dispatch,
    ResultTask,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TraceStep {
    id: String,
    stage: TraceStage,
    label: String,
    authority: String,
    summary: String,
    payload: Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Fingerprints {
    catalog: String,
    run_schema: String,
    help_schema: String,
    compact_surface: String,
    native_surface: String,
    invocation: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SemanticAnchor {
    id: String,
    label: String,
    source_fact: String,
    target_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ProjectionId {
    Help,
    Schema,
    Mcp,
    Confirmation,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ProfileId {
    Compact,
    Native,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ComparisonTarget {
    id: String,
    fact_id: String,
    projection: ProjectionId,
    label: String,
    value: Value,
    display_value: String,
    editable: bool,
    profiles: Vec<ProfileId>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PrivacyEvidence {
    declared: bool,
    handler_observed: bool,
    raw_identity_serialized: bool,
    redacted_summary: String,
    checks: Vec<PrivacyCheck>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PrivacyCheck {
    surface: String,
    absent: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceManifest {
    format_version: u32,
    generator: Generator,
    sources: Vec<ManifestSource>,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestSource {
    path: String,
    sha256: String,
    provenance: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    path: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Copy)]
struct EvidenceClient;

impl ClientHandler for EvidenceClient {}

#[derive(Default)]
struct BridgeCapture {
    value: Mutex<Option<Value>>,
}

impl BridgeCapture {
    fn take(&self) -> Result<Value> {
        self.value
            .lock()
            .expect("bridge capture")
            .take()
            .context("native confirmation bridge was not called")
    }
}

#[async_trait]
impl NativeConfirmationBridge for BridgeCapture {
    async fn confirm(
        &self,
        request: NativeConfirmationRequest,
    ) -> std::result::Result<NativeConfirmationDecision, NativeConfirmationBridgeError> {
        *self.value.lock().expect("bridge capture") = Some(json!({
            "preview": request.preview(),
            "arguments": request.arguments(),
            "presentation": request.presentation(),
            "invocationFingerprint": request.invocation_fingerprint(),
        }));
        Ok(NativeConfirmationDecision::Allow)
    }
}

#[derive(Default)]
struct AuthorizerCapture {
    value: Mutex<Option<Value>>,
}

impl AuthorizerCapture {
    fn take(&self) -> Result<Value> {
        self.value
            .lock()
            .expect("authorizer capture")
            .take()
            .context("permission authorizer was not called")
    }
}

impl PermissionAuthorizer for AuthorizerCapture {
    fn decide(&self, plan: &mcp_twill::InvocationPlan) -> PermissionDecision {
        let decision = DefaultPermissionAuthorizer.decide(plan);
        let decision_label = match &decision {
            PermissionDecision::Allow => "allow",
            PermissionDecision::RequireConfirmation => "requireConfirmation",
            PermissionDecision::Deny { .. } => "deny",
        };
        *self.value.lock().expect("authorizer capture") = Some(json!({
            "plan": plan,
            "decision": decision_label,
        }));
        decision
    }
}

pub fn export(check: bool) -> Result<()> {
    let repository = repository_root()?;
    let destination = repository.join(EVIDENCE_DIR);
    let temporary = tempfile::tempdir().context("create site evidence staging directory")?;
    let staged = temporary.path().join("evidence");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build site evidence runtime")?;
    runtime.block_on(generate(&repository, &staged))?;

    if check {
        compare_directories(&staged, &destination)
            .with_context(|| format!("site evidence drifted; run `{GENERATOR_COMMAND}`"))?;
    } else {
        sync_directory(&staged, &destination)?;
    }
    Ok(())
}

async fn generate(repository: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| {
        format!(
            "create generated evidence directory `{}`",
            destination.display()
        )
    })?;

    let source_hashes = source_hashes(repository)?;
    let source = Source {
        repository: "wycats/mcp-twill".to_string(),
        paths: SOURCE_PATHS
            .iter()
            .map(|path| (*path).to_string())
            .collect(),
        source_hashes: source_hashes.clone(),
    };
    let mut variants = Vec::with_capacity(16);
    for title_rule in [
        specimen::TitleRule::Unconstrained,
        specimen::TitleRule::NonEmpty,
    ] {
        for destination_kind in [specimen::Destination::Local, specimen::Destination::Remote] {
            for confirmation in [
                specimen::ConfirmationKind::Generic,
                specimen::ConfirmationKind::TitleInterpolated,
            ] {
                for private_context in [
                    specimen::PrivateContext::None,
                    specimen::PrivateContext::ConversationIdentity,
                ] {
                    variants.push(
                        generate_variant(specimen::SpecimenConfig {
                            title_rule,
                            destination: destination_kind,
                            confirmation,
                            private_context,
                        })
                        .await?,
                    );
                }
            }
        }
    }
    variants.sort_by(|left, right| left.id.cmp(&right.id));
    let controls = controls();
    validate_controls(&controls)?;
    validate_variants(&variants, &controls)?;

    let bundle = EvidenceBundle {
        format_version: FORMAT_VERSION,
        generated_by: generator(),
        source,
        defaults: Defaults {
            selection: selection(specimen::SpecimenConfig::default()),
            profile: "native".to_string(),
        },
        controls,
        variants,
    };
    let serialized_bundle =
        serde_json::to_string(&bundle).context("serialize bundle for privacy audit")?;
    ensure!(
        !contains_private_identity(&serialized_bundle),
        "a private conversation identity appeared in the fully serialized evidence bundle"
    );
    write_canonical_json(&destination.join("bundle.json"), &bundle)?;

    let mut schema = serde_json::to_value(schemars::schema_for!(EvidenceBundle))
        .context("serialize evidence bundle schema")?;
    let format_schema = schema
        .pointer_mut("/properties/formatVersion")
        .context("generated bundle schema is missing formatVersion")?;
    *format_schema = json!({ "type": "integer", "const": FORMAT_VERSION });
    write_canonical_value(&destination.join("schema.json"), schema)?;

    let vbl_dir = destination.join("vbl");
    fs::create_dir_all(&vbl_dir).context("create VBL evidence directory")?;
    copy_exact(
        &repository.join("docs/adoption/visible-browser-lab/baseline/catalog-measurement.json"),
        &vbl_dir.join("catalog-measurement.json"),
    )?;
    copy_exact(
        &repository.join("crates/mcp-twill/tests/fixtures/vbl/v0.4.9/manifest.json"),
        &vbl_dir.join("v0.4.9-manifest.json"),
    )?;

    let mut files = collect_files(destination)?
        .into_iter()
        .filter(|path| path != Path::new("manifest.json"))
        .map(|path| {
            let bytes = fs::read(destination.join(&path))
                .with_context(|| format!("read staged evidence `{}`", path.display()))?;
            Ok(ManifestFile {
                path: slash_path(&path),
                sha256: sha256(&bytes),
                bytes: bytes.len() as u64,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let mut sources = SOURCE_PATHS
        .iter()
        .map(|path| ManifestSource {
            path: (*path).to_string(),
            sha256: source_hashes
                .get(*path)
                .expect("source hashes cover the declared inventory")
                .clone(),
            provenance: source_provenance(path).to_string(),
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.path.cmp(&right.path));
    write_canonical_json(
        &destination.join("manifest.json"),
        &EvidenceManifest {
            format_version: FORMAT_VERSION,
            generator: generator(),
            sources,
            files,
        },
    )?;
    Ok(())
}

async fn generate_variant(config: specimen::SpecimenConfig) -> Result<ScenarioVariant> {
    let observation = Arc::new(specimen::HandlerObservation::default());
    let registry = specimen::registry(config, observation.clone())?;
    validate_contract(&registry, config)?;

    let operation = registry
        .operation_specs()
        .into_iter()
        .find(|operation| operation.id == OPERATION_ID)
        .context("site specimen operation is missing")?;
    let command = registry
        .command_specs()
        .find(|command| command.name() == "issues create")
        .context("site specimen command is missing")?;
    let argument_schema = registry.arg_schema(command);
    let help = registry.help(HelpRequest {
        command: Some("issues create".to_string()),
        topic: Some(HelpTopic::Usage),
        detail: Some(HelpDetail::Full),
    });
    let permissions_help = registry.help(HelpRequest {
        command: Some("issues create".to_string()),
        topic: Some(HelpTopic::Permissions),
        detail: Some(HelpDetail::Full),
    });
    let usage_help_value = serde_json::to_value(&help).context("serialize generated help")?;
    let permissions_help_value =
        serde_json::to_value(&permissions_help).context("serialize generated permission help")?;
    let help_value = json!({
        "usage": usage_help_value,
        "permissions": permissions_help_value,
    });

    let compact_server = CliMcpServer::new(registry.clone())?;
    let compact_violations = mcp_twill::contract::check_server_projection(&compact_server);
    ensure!(
        compact_violations.is_empty(),
        "compact server projection violations for {}: {compact_violations:#?}",
        scenario_id(config)
    );
    let compact_identity = compact_server.runtime_identity();
    let compact_tools = serde_json::to_value(compact_server.generated_tools())
        .context("serialize compact tools")?;
    let compact_tool_name = operation
        .lane()
        .tool_name(&compact_server.config().execution_tool_name);
    let compact_selected_tool = compact_tools
        .as_array()
        .and_then(|tools| {
            tools.iter().find(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name == compact_tool_name)
            })
        })
        .cloned()
        .with_context(|| format!("compact tool `{compact_tool_name}` is missing"))?;

    let native_surface = native_surface(&registry)?;
    let native_violations = check_native_surface_projection(&registry, &native_surface);
    ensure!(
        native_violations.is_empty(),
        "native surface projection violations for {}: {native_violations:#?}",
        scenario_id(config)
    );
    let native_document = native_surface.snapshot().document().clone();
    let native_surface_hash = native_surface.snapshot().surface_hash().to_string();
    let native_surface_identity = json!({
        "name": native_surface.snapshot().name(),
        "hash": native_surface.snapshot().surface_hash(),
    });
    let native_tool = serde_json::to_value(
        native_surface
            .snapshot()
            .tools()
            .iter()
            .find(|tool| tool.name == NATIVE_TOOL_NAME)
            .context("native issues_create tool is missing")?,
    )
    .context("serialize native tool")?;
    let mcp_surface_comparison = mcp_surface_comparison(
        &operation.id,
        &compact_tools,
        &compact_selected_tool,
        &native_document,
        &native_tool,
    )?;

    let bridge = Arc::new(BridgeCapture::default());
    let authorizer = Arc::new(AuthorizerCapture::default());
    let events = Arc::new(InMemoryEventSink::new());
    let native_server = CliMcpServer::builder(registry.clone())
        .surface(native_surface.clone())
        .authorizer(authorizer.clone())
        .native_confirmation_bridge(bridge.clone())
        .build()?
        .with_event_sink(events.clone());
    let primary_identity = matches!(
        config.private_context,
        specimen::PrivateContext::ConversationIdentity
    )
    .then_some((PRIVATE_ISSUER, PRIVATE_ID));
    let call_result = call_native_tool(native_server, primary_identity).await?;
    let bridge_value = bridge.take()?;
    let authorizer_value = authorizer.take()?;
    let event_value =
        serde_json::to_value(events.events()).context("serialize framework events")?;
    let plan = observation
        .plan()
        .context("site specimen handler did not capture its plan")?;
    let invocation_fingerprint = plan
        .get("invocationFingerprint")
        .and_then(Value::as_str)
        .context("captured plan is missing invocationFingerprint")?
        .to_string();
    ensure!(
        bridge_value["invocationFingerprint"] == invocation_fingerprint,
        "bridge and handler fingerprints differ for {}",
        scenario_id(config)
    );
    ensure!(
        authorizer_value["plan"]["invocationFingerprint"] == invocation_fingerprint,
        "authorizer and handler fingerprints differ for {}",
        scenario_id(config)
    );
    if primary_identity.is_some() {
        validate_identity_value_binding(
            &registry,
            &native_surface,
            &observation,
            &invocation_fingerprint,
            config,
        )
        .await?;
    }

    let confirmation = confirmation_projection(&bridge_value)?;
    let result = serde_json::to_value(&call_result).context("serialize native call result")?;
    let catalog_operation =
        serde_json::to_value(&operation).context("serialize catalog operation")?;
    let raw_surfaces = [
        ("arguments", bridge_value["arguments"].clone()),
        ("help", help_value.clone()),
        ("argument schema", argument_schema.clone()),
        ("compact tools", compact_tools.clone()),
        ("native surface", native_document.clone()),
        ("confirmation", serde_json::to_value(&confirmation)?),
        ("authorizer", authorizer_value.clone()),
        ("plan", plan.clone()),
        ("framework events", event_value),
        ("application logs (none emitted)", json!([])),
        ("result", result.clone()),
    ];
    let privacy = privacy_evidence(config, observation.identity_observed(), &raw_surfaces)?;

    let host_preview = host_preview(&operation, &native_tool, &confirmation);
    let comparison_targets = comparison_targets(ComparisonInputs {
        help: &help,
        permissions_help: &permissions_help,
        argument_schema: &argument_schema,
        compact_selected_tool: &compact_selected_tool,
        native_tool: &native_tool,
        confirmation: &confirmation,
        host: &host_preview,
        operation: &operation,
    });
    let semantic_anchors = semantic_anchors();
    let rendered_declaration = specimen::declaration(&operation).map_err(anyhow::Error::msg)?;
    validate_declaration_projection(&rendered_declaration.text, &operation)?;
    let declaration = Declaration {
        source_path: "crates/mcp-twill/examples/issues_server/site_specimen.rs".to_string(),
        text: rendered_declaration.text,
        facts: declaration_facts(
            &operation,
            &argument_schema,
            &confirmation,
            &semantic_anchors,
            &rendered_declaration.fact_ranges,
        )?,
    };
    let trace = trace(
        &operation,
        &argument_schema,
        &bridge_value,
        &authorizer_value,
        &confirmation,
        &plan,
        &result,
    );
    let catalog_identity = registry.catalog_identity();
    let rust_configuration = rust_configuration(&operation)?;
    validate_rust_configuration(&rust_configuration, config)?;

    Ok(ScenarioVariant {
        id: scenario_id(config),
        selection: selection(config),
        declaration,
        rust_configuration,
        catalog_operation,
        help: help_value,
        argument_schema,
        compact: CompactProjection {
            tools: compact_tools,
            selected_tool: compact_selected_tool,
            surface_identity: serde_json::to_value(
                compact_identity
                    .surface
                    .as_ref()
                    .context("compact surface identity is missing")?,
            )?,
        },
        native: NativeProjection {
            surface: native_document,
            tool: native_tool,
            surface_identity: native_surface_identity,
        },
        mcp_surface_comparison,
        confirmation,
        host_preview,
        plan,
        trace,
        result,
        fingerprints: Fingerprints {
            catalog: catalog_identity.catalog_hash,
            run_schema: catalog_identity.run_schema_hash,
            help_schema: catalog_identity.help_schema_hash,
            compact_surface: compact_identity
                .surface
                .context("compact surface identity is missing")?
                .hash,
            native_surface: native_surface_hash,
            invocation: invocation_fingerprint,
        },
        semantic_anchors,
        comparison_targets,
        privacy,
    })
}

fn validate_contract(registry: &CommandRegistry, config: specimen::SpecimenConfig) -> Result<()> {
    let violations = verify_catalog_coverage(registry, "run");
    ensure!(
        violations.is_empty(),
        "catalog contract violations for {}: {violations:#?}",
        scenario_id(config)
    );
    Ok(())
}

fn native_surface(registry: &CommandRegistry) -> mcp_twill::Result<NativeToolSurface> {
    NativeToolSurface::builder("command-woven-native")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Bridge)
        .direct(NATIVE_TOOL_NAME, OPERATION_ID)
        .build(registry, McpProtocolTarget::V2025_11_25)
}

async fn call_native_tool(
    server: CliMcpServer,
    private_identity: Option<(&'static str, &'static str)>,
) -> Result<rmcp::model::CallToolResult> {
    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = EvidenceClient.serve(client_transport).await?;
    let mut params = CallToolRequestParams::new(NATIVE_TOOL_NAME).with_arguments(
        serde_json::from_value(json!({
            "title": SAMPLE_TITLE,
            "body": SAMPLE_BODY,
        }))?,
    );
    if let Some((issuer, id)) = private_identity {
        params.meta = Some(Meta(
            [(
                CONVERSATION_IDENTITY_META_KEY.to_string(),
                json!({
                    "version": 1,
                    "issuer": issuer,
                    "id": id,
                }),
            )]
            .into_iter()
            .collect(),
        ));
    }
    let result = client.call_tool(params).await?;
    client.cancel().await?;
    server_handle.await??;
    ensure!(
        result.is_error != Some(true),
        "native specimen call failed: {result:?}"
    );
    Ok(result)
}

async fn validate_identity_value_binding(
    registry: &CommandRegistry,
    surface: &NativeToolSurface,
    observation: &specimen::HandlerObservation,
    primary_fingerprint: &str,
    config: specimen::SpecimenConfig,
) -> Result<()> {
    ensure!(
        (PRIVATE_ISSUER, PRIVATE_ID) != (ALTERNATE_PRIVATE_ISSUER, ALTERNATE_PRIVATE_ID),
        "identity-binding audit requires distinct private identity values"
    );
    let bridge = Arc::new(BridgeCapture::default());
    let authorizer = Arc::new(AuthorizerCapture::default());
    let server = CliMcpServer::builder(registry.clone())
        .surface(surface.clone())
        .authorizer(authorizer.clone())
        .native_confirmation_bridge(bridge.clone())
        .build()?;
    let call_result = call_native_tool(
        server,
        Some((ALTERNATE_PRIVATE_ISSUER, ALTERNATE_PRIVATE_ID)),
    )
    .await?;
    let bridge_value = bridge.take()?;
    let authorizer_value = authorizer.take()?;
    let plan = observation
        .plan()
        .context("alternate identity call did not capture its plan")?;
    let alternate_fingerprint = plan
        .get("invocationFingerprint")
        .and_then(Value::as_str)
        .context("alternate identity plan is missing invocationFingerprint")?;
    ensure!(
        alternate_fingerprint != primary_fingerprint,
        "distinct conversation identity values did not change the invocation fingerprint for {}",
        scenario_id(config)
    );
    ensure!(
        bridge_value["invocationFingerprint"] == alternate_fingerprint,
        "alternate bridge and handler fingerprints differ for {}",
        scenario_id(config)
    );
    ensure!(
        authorizer_value["plan"]["invocationFingerprint"] == alternate_fingerprint,
        "alternate authorizer and handler fingerprints differ for {}",
        scenario_id(config)
    );
    for (surface_name, value) in [
        ("alternate bridge", bridge_value),
        ("alternate authorizer", authorizer_value),
        ("alternate plan", plan),
        (
            "alternate result",
            serde_json::to_value(call_result).context("serialize alternate result for audit")?,
        ),
    ] {
        ensure!(
            !contains_private_identity(&canonical_inline(&value)),
            "alternate private identity appeared in {surface_name} for {}",
            scenario_id(config)
        );
    }
    Ok(())
}

fn confirmation_projection(value: &Value) -> Result<ConfirmationProjection> {
    let presentation = &value["presentation"];
    Ok(ConfirmationProjection {
        operation_id: required_string(presentation, "operationId")?,
        branch: presentation["branch"].clone(),
        title: required_string(presentation, "title")?,
        message: required_string(presentation, "message")?,
        arguments: value["arguments"].clone(),
        invocation_fingerprint: required_string(value, "invocationFingerprint")?,
    })
}

fn mcp_surface_comparison(
    operation_id: &str,
    compact_tools: &Value,
    compact_selected_tool: &Value,
    native_surface: &Value,
    native_tool: &Value,
) -> Result<McpSurfaceComparison> {
    Ok(McpSurfaceComparison {
        operation_id: operation_id.to_string(),
        compact: mcp_surface_facts(compact_tools, compact_selected_tool, "compact")?,
        native: mcp_surface_facts(
            native_surface
                .get("tools")
                .context("native surface is missing its tool inventory")?,
            native_tool,
            "native",
        )?,
    })
}

fn mcp_surface_facts(
    tools: &Value,
    selected_tool: &Value,
    surface_name: &str,
) -> Result<McpSurfaceFacts> {
    let tool_inventory = tools
        .as_array()
        .with_context(|| format!("{surface_name} tool inventory is not an array"))?
        .iter()
        .map(|tool| required_string(tool, "name"))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        !tool_inventory.is_empty(),
        "{surface_name} tool inventory is empty"
    );
    ensure!(
        tool_inventory.iter().collect::<BTreeSet<_>>().len() == tool_inventory.len(),
        "{surface_name} tool inventory contains duplicate names"
    );

    let tool_name = required_string(selected_tool, "name")?;
    ensure!(
        tool_inventory.contains(&tool_name),
        "{surface_name} selected tool `{tool_name}` is absent from its inventory"
    );

    let required_inputs = selected_tool
        .pointer("/inputSchema/required")
        .and_then(Value::as_array)
        .with_context(|| format!("{surface_name} selected tool is missing inputSchema.required"))?
        .iter()
        .map(|input| {
            input.as_str().map(ToOwned::to_owned).with_context(|| {
                format!("{surface_name} selected tool has a non-string required input")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let properties = selected_tool
        .pointer("/inputSchema/properties")
        .and_then(Value::as_object)
        .with_context(|| {
            format!("{surface_name} selected tool is missing inputSchema.properties")
        })?;
    let input_fields = properties.keys().cloned().collect();
    ensure!(
        required_inputs
            .iter()
            .all(|input| properties.contains_key(input)),
        "{surface_name} selected tool requires an input absent from inputSchema.properties"
    );

    Ok(McpSurfaceFacts {
        tool_name,
        tool_inventory,
        required_inputs,
        input_fields,
        has_argument_map: properties.contains_key("args"),
    })
}

fn host_preview(
    operation: &mcp_twill::OperationSpec,
    native_tool: &Value,
    confirmation: &ConfirmationProjection,
) -> HostPreview {
    let properties = native_tool
        .pointer("/inputSchema/properties")
        .and_then(Value::as_object);
    let required = native_tool
        .pointer("/inputSchema/required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let argument_fields = ["title", "body"]
        .into_iter()
        .map(|name| {
            let schema = properties
                .and_then(|properties| properties.get(name))
                .cloned()
                .unwrap_or(Value::Null);
            HostArgumentField {
                name: name.to_string(),
                label: operation
                    .args
                    .iter()
                    .find(|argument| argument.name == name)
                    .map(|argument| argument.summary.clone())
                    .unwrap_or_else(|| name.to_string()),
                required: required.iter().any(|value| value == name),
                constraint: schema_constraint_display(&schema),
            }
        })
        .collect();
    let effect_badges = operation
        .permissions
        .iter()
        .filter_map(|permission| {
            serde_json::to_value(&permission.effect)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
        })
        .collect();
    HostPreview {
        label: "Illustrative host rendering — layout is site-owned; values are Twill-generated."
            .to_string(),
        title: native_tool
            .pointer("/annotations/title")
            .and_then(Value::as_str)
            .unwrap_or(&operation.summary)
            .to_string(),
        description: native_tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(&operation.description)
            .to_string(),
        argument_fields,
        effect_badges,
        confirmation: HostConfirmation {
            title: confirmation.title.clone(),
            message: confirmation.message.clone(),
        },
        task_support: serde_json::to_value(&operation.task_support)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "optional".to_string()),
        private_context: HostPrivateContext {
            declared: operation.uses_conversation_identity,
            label: if operation.uses_conversation_identity {
                "Conversation identity available privately".to_string()
            } else {
                "No private conversation identity declared".to_string()
            },
        },
    }
}

struct ComparisonInputs<'a> {
    help: &'a mcp_twill::HelpResult,
    permissions_help: &'a mcp_twill::HelpResult,
    argument_schema: &'a Value,
    compact_selected_tool: &'a Value,
    native_tool: &'a Value,
    confirmation: &'a ConfirmationProjection,
    host: &'a HostPreview,
    operation: &'a mcp_twill::OperationSpec,
}

fn comparison_targets(inputs: ComparisonInputs<'_>) -> Vec<ComparisonTarget> {
    let ComparisonInputs {
        help,
        permissions_help,
        argument_schema,
        compact_selected_tool,
        native_tool,
        confirmation,
        host,
        operation,
    } = inputs;
    let title_help = help
        .text
        .lines()
        .find(|line| line.contains("$args.title"))
        .map_or(Value::Null, |line| json!(line));
    let private_help = help
        .text
        .lines()
        .find(|line| line.contains("conversation identity"))
        .map_or(Value::Null, |line| json!(line));
    let title_schema = argument_schema
        .pointer("/properties/title")
        .cloned()
        .unwrap_or(Value::Null);
    let native_title_schema = native_tool
        .pointer("/inputSchema/properties/title")
        .cloned()
        .unwrap_or(Value::Null);
    let host_title = host
        .argument_fields
        .iter()
        .find(|field| field.name == "title")
        .map_or(Value::Null, |field| json!(field.constraint));
    let title_display = schema_constraint_display(&title_schema);
    let native_title_display = schema_constraint_display(&native_title_schema);
    vec![
        comparison(
            "help.titleRule",
            "fact.titleRule",
            ProjectionId::Help,
            "Help title constraint",
            title_help.clone(),
            title_help
                .as_str()
                .unwrap_or("No title constraint rendered")
                .to_string(),
            target_scope(true, both_profiles()),
        ),
        comparison(
            "schema.titleRule",
            "fact.titleRule",
            ProjectionId::Schema,
            "Schema title constraint",
            title_schema,
            title_display.clone(),
            target_scope(true, both_profiles()),
        ),
        comparison(
            "mcp.native.titleRule",
            "fact.titleRule",
            ProjectionId::Mcp,
            "Native MCP title constraint",
            native_title_schema,
            native_title_display,
            target_scope(false, native_profiles()),
        ),
        comparison(
            "host.titleRule",
            "fact.titleRule",
            ProjectionId::Host,
            "Host title constraint",
            host_title,
            title_display,
            target_scope(true, both_profiles()),
        ),
        comparison(
            "help.destination",
            "fact.destination",
            ProjectionId::Help,
            "Help effects",
            serde_json::to_value(permissions_help).unwrap_or(Value::Null),
            permissions_help.text.clone(),
            target_scope(false, both_profiles()),
        ),
        comparison(
            "mcp.compact.destination",
            "fact.destination",
            ProjectionId::Mcp,
            "Compact effect-lane tool",
            compact_selected_tool.clone(),
            compact_selected_tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown compact tool")
                .to_string(),
            target_scope(false, compact_profiles()),
        ),
        comparison(
            "mcp.native.destination",
            "fact.destination",
            ProjectionId::Mcp,
            "Native MCP effect annotation",
            native_tool["annotations"].clone(),
            format!(
                "openWorldHint: {}",
                native_tool
                    .pointer("/annotations/openWorldHint")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ),
            target_scope(false, native_profiles()),
        ),
        comparison(
            "host.destination",
            "fact.destination",
            ProjectionId::Host,
            "Host effect badges",
            json!(host.effect_badges),
            host.effect_badges.join(" + "),
            target_scope(false, both_profiles()),
        ),
        comparison(
            "confirmation.message",
            "fact.confirmation",
            ProjectionId::Confirmation,
            "Prepared confirmation",
            json!(confirmation.message),
            confirmation.message.clone(),
            target_scope(false, both_profiles()),
        ),
        comparison(
            "host.confirmation",
            "fact.confirmation",
            ProjectionId::Host,
            "Host confirmation",
            json!(host.confirmation.message),
            host.confirmation.message.clone(),
            target_scope(false, both_profiles()),
        ),
        comparison(
            "help.privateContext",
            "fact.privateContext",
            ProjectionId::Help,
            "Help request context",
            private_help.clone(),
            private_help
                .as_str()
                .unwrap_or("No private conversation identity declared")
                .to_string(),
            target_scope(false, both_profiles()),
        ),
        comparison(
            "host.privateContext",
            "fact.privateContext",
            ProjectionId::Host,
            "Host private context",
            json!({
                "declared": host.private_context.declared,
                "label": host.private_context.label,
            }),
            host.private_context.label.clone(),
            target_scope(false, both_profiles()),
        ),
        comparison(
            "mcp.privateContext",
            "fact.privateContext",
            ProjectionId::Mcp,
            "MCP arguments remain private-context free",
            json!({
                "usesConversationIdentity": operation.uses_conversation_identity,
                "conversationIdentityArgumentPresent": {
                    "compact":
                        canonical_inline(&compact_selected_tool["inputSchema"])
                            .contains("conversationIdentity"),
                    "native":
                        canonical_inline(&native_tool["inputSchema"])
                            .contains("conversationIdentity"),
                },
            }),
            if operation.uses_conversation_identity {
                "Declared privately; absent from MCP arguments".to_string()
            } else {
                "Not declared; absent from MCP arguments".to_string()
            },
            target_scope(false, both_profiles()),
        ),
    ]
}

fn comparison(
    id: &str,
    fact_id: &str,
    projection: ProjectionId,
    label: &str,
    value: Value,
    display_value: String,
    scope: TargetScope,
) -> ComparisonTarget {
    ComparisonTarget {
        id: id.to_string(),
        fact_id: fact_id.to_string(),
        projection,
        label: label.to_string(),
        value,
        display_value,
        editable: scope.editable,
        profiles: scope.profiles,
    }
}

struct TargetScope {
    editable: bool,
    profiles: Vec<ProfileId>,
}

fn target_scope(editable: bool, profiles: Vec<ProfileId>) -> TargetScope {
    TargetScope { editable, profiles }
}

fn both_profiles() -> Vec<ProfileId> {
    vec![ProfileId::Compact, ProfileId::Native]
}

fn compact_profiles() -> Vec<ProfileId> {
    vec![ProfileId::Compact]
}

fn native_profiles() -> Vec<ProfileId> {
    vec![ProfileId::Native]
}

fn schema_constraint_display(schema: &Value) -> String {
    schema
        .get("minLength")
        .and_then(Value::as_u64)
        .map(|minimum| format!("Minimum {minimum} character"))
        .unwrap_or_else(|| "No declared length constraint".to_string())
}

fn semantic_anchors() -> Vec<SemanticAnchor> {
    [
        (
            "anchor.titleRule",
            "Title rule",
            "fact.titleRule",
            &[
                "help.titleRule",
                "schema.titleRule",
                "mcp.native.titleRule",
                "host.titleRule",
            ][..],
        ),
        (
            "anchor.destination",
            "Destination and effect",
            "fact.destination",
            &[
                "help.destination",
                "mcp.compact.destination",
                "mcp.native.destination",
                "host.destination",
            ][..],
        ),
        (
            "anchor.confirmation",
            "Confirmation presentation",
            "fact.confirmation",
            &["confirmation.message", "host.confirmation"][..],
        ),
        (
            "anchor.privateContext",
            "Private context",
            "fact.privateContext",
            &[
                "help.privateContext",
                "mcp.privateContext",
                "host.privateContext",
            ][..],
        ),
    ]
    .into_iter()
    .map(|(id, label, source_fact, target_ids)| SemanticAnchor {
        id: id.to_string(),
        label: label.to_string(),
        source_fact: source_fact.to_string(),
        target_ids: target_ids.iter().map(|id| (*id).to_string()).collect(),
    })
    .collect()
}

fn declaration_facts(
    operation: &mcp_twill::OperationSpec,
    argument_schema: &Value,
    confirmation: &ConfirmationProjection,
    anchors: &[SemanticAnchor],
    rendered_ranges: &[specimen::DeclarationCodeRange],
) -> Result<Vec<DeclarationFact>> {
    let title_value = argument_schema
        .pointer("/properties/title")
        .cloned()
        .unwrap_or(Value::Null);
    let effect_value = serde_json::to_value(&operation.effect).unwrap_or(Value::Null);
    let facts = [
        (
            "fact.titleRule",
            "Title rule",
            title_value.clone(),
            schema_constraint_display(&title_value),
        ),
        (
            "fact.destination",
            "Destination and effect",
            effect_value.clone(),
            effect_display(&effect_value),
        ),
        (
            "fact.confirmation",
            "Confirmation presentation",
            serde_json::to_value(confirmation).unwrap_or(Value::Null),
            confirmation.message.clone(),
        ),
        (
            "fact.privateContext",
            "Private context",
            json!({ "usesConversationIdentity": operation.uses_conversation_identity }),
            if operation.uses_conversation_identity {
                "Optional host-supplied conversation identity".to_string()
            } else {
                "No private conversation identity".to_string()
            },
        ),
    ];
    let known_fact_ids = facts.iter().map(|(id, _, _, _)| *id).collect::<Vec<_>>();
    validate_rendered_range_fact_ids(rendered_ranges, &known_fact_ids)?;

    facts
        .into_iter()
        .map(|(id, label, value, display_value)| {
            let code_ranges = rendered_ranges
                .iter()
                .filter(|range| range.fact_id == id)
                .map(|range| DeclarationCodeRange {
                    start_line: range.start_line,
                    end_line: range.end_line,
                })
                .collect::<Vec<_>>();
            ensure!(
                id == "fact.privateContext" || !code_ranges.is_empty(),
                "displayed declaration is missing a code range for {id}"
            );
            let code_presence = if code_ranges.is_empty() {
                DeclarationCodePresence::Omitted
            } else {
                DeclarationCodePresence::Rendered
            };
            Ok(DeclarationFact {
                id: id.to_string(),
                label: label.to_string(),
                value,
                display_value,
                target_ids: anchors
                    .iter()
                    .find(|anchor| anchor.source_fact == id)
                    .map(|anchor| anchor.target_ids.clone())
                    .unwrap_or_default(),
                code_presence,
                code_ranges,
            })
        })
        .collect()
}

fn validate_rendered_range_fact_ids(
    rendered_ranges: &[specimen::DeclarationCodeRange],
    known_fact_ids: &[&str],
) -> Result<()> {
    for range in rendered_ranges {
        ensure!(
            known_fact_ids.contains(&range.fact_id),
            "displayed declaration has an unknown code-range fact {}",
            range.fact_id
        );
    }
    Ok(())
}

fn effect_display(effect: &Value) -> String {
    match effect {
        Value::String(effect) => effect.clone(),
        Value::Object(object) => object
            .get("composite")
            .and_then(Value::as_array)
            .map(|effects| {
                effects
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" + ")
            })
            .filter(|display| !display.is_empty())
            .unwrap_or_else(|| canonical_inline(effect)),
        _ => canonical_inline(effect),
    }
}

fn trace(
    operation: &mcp_twill::OperationSpec,
    argument_schema: &Value,
    bridge: &Value,
    authorizer: &Value,
    confirmation: &ConfirmationProjection,
    plan: &Value,
    result: &Value,
) -> Vec<TraceStep> {
    vec![
        trace_step(
            "trace.select",
            TraceStage::Select,
            "Select",
            "The catalog selects one operation.",
            json!({
                "operationId": operation.id,
                "path": operation.path,
                "effect": operation.effect,
                "lane": operation.lane(),
            }),
        ),
        trace_step(
            "trace.bind",
            TraceStage::Bind,
            "Bind",
            "Arguments bind while private context stays out of the argument map.",
            json!({
                "boundArgs": plan["boundArgs"],
                "resourceBindingFacts": plan["resourceBindingFacts"],
            }),
        ),
        trace_step(
            "trace.validate",
            TraceStage::Validate,
            "Validate",
            "The generated operation schema validates the bound values.",
            argument_schema.clone(),
        ),
        trace_step(
            "trace.authorize",
            TraceStage::Authorize,
            "Authorize",
            "The framework authorizer evaluates the generated effect.",
            json!({
                "decision": authorizer["decision"],
                "plan": authorizer["plan"],
                "preview": bridge["preview"],
            }),
        ),
        trace_step(
            "trace.realize",
            TraceStage::Realize,
            "Realize",
            "The native confirmation bridge receives the prepared presentation.",
            serde_json::to_value(confirmation).unwrap_or(Value::Null),
        ),
        trace_step(
            "trace.dispatch",
            TraceStage::Dispatch,
            "Dispatch",
            "The selected native surface dispatches the captured invocation plan.",
            json!({
                "origin": plan["origin"],
                "surface": plan["surface"],
                "invocationFingerprint": plan["invocationFingerprint"],
            }),
        ),
        trace_step(
            "trace.resultTask",
            TraceStage::ResultTask,
            "Result / task",
            "This specimen returns immediately; task support remains optional.",
            json!({
                "result": result,
                "taskSupport": operation.task_support,
                "delivery": "immediate",
            }),
        ),
    ]
}

fn trace_step(
    id: &str,
    stage: TraceStage,
    label: &str,
    summary: &str,
    payload: Value,
) -> TraceStep {
    TraceStep {
        id: id.to_string(),
        stage,
        label: label.to_string(),
        authority: "essayAuthored".to_string(),
        summary: summary.to_string(),
        payload,
    }
}

fn validate_declaration_projection(
    declaration: &str,
    operation: &mcp_twill::OperationSpec,
) -> Result<()> {
    let command_name = operation.path.join(" ");
    let mut required = vec![
        (
            "command path",
            format!("server.command({},", rust_string(&command_name)),
        ),
        (
            "operation summary",
            format!(".summary({})", rust_string(&operation.summary)),
        ),
        (
            "operation description",
            format!(".description({})", rust_string(&operation.description)),
        ),
    ];
    let use_when = operation
        .use_when
        .as_deref()
        .context("site specimen operation is missing use_when guidance")?;
    required.push((
        "selection guidance",
        format!(".use_when({})", rust_string(use_when)),
    ));

    for argument in &operation.args {
        required.push((
            "argument declaration",
            format!("arg::string({})", rust_string(&argument.name)),
        ));
        required.push((
            "argument summary",
            format!(".summary({})", rust_string(&argument.summary)),
        ));
        match &argument.schema {
            None => {}
            Some(mcp_twill::ArgumentSchemaUse::Inline { schema }) => {
                required.push(("argument inline schema", indented_json(schema, 20)?))
            }
            Some(mcp_twill::ArgumentSchemaUse::Named { .. }) => {
                bail!(
                    "displayed declaration cannot represent named schema argument `{}`",
                    argument.name
                );
            }
        }
    }
    ensure!(
        declaration.matches("        .arg(").count() == operation.args.len(),
        "displayed declaration duplicated or omitted arguments for {}",
        operation.id
    );

    for permission in &operation.permissions {
        let method = match permission.effect {
            mcp_twill::PermissionEffect::Write => "write",
            mcp_twill::PermissionEffect::Network => "network",
            _ => bail!(
                "displayed declaration cannot represent permission effect `{}`",
                permission.effect.as_label()
            ),
        };
        required.push((
            "permission effect",
            format!(
                ".{}({}, {})",
                method,
                rust_string(&permission.scope),
                rust_string(&permission.description)
            ),
        ));
    }
    ensure!(
        declaration.matches("        .write(").count()
            + declaration.matches("        .network(").count()
            == operation.permissions.len(),
        "displayed declaration duplicated or omitted effects for {}",
        operation.id
    );

    let presentation = operation
        .presentation
        .as_ref()
        .context("site specimen operation is missing presentation")?;
    let invocation_message = presentation
        .invocation_message
        .as_deref()
        .context("site specimen operation is missing invocation message")?;
    required.push((
        "invocation message",
        format!(".invocation_message({})", rust_string(invocation_message)),
    ));
    let confirmation = presentation
        .confirmation
        .as_ref()
        .context("site specimen operation is missing confirmation")?;
    ensure!(
        confirmation.cases.is_empty(),
        "displayed declaration requires confirmation-case coverage"
    );
    required.push((
        "confirmation title",
        format!(
            "ConfirmationMessage::new({})",
            rust_string(&confirmation.default.title)
        ),
    ));
    for segment in &confirmation.default.body {
        match segment {
            mcp_twill::ConfirmationSegment::Text(text) => {
                required.push(("confirmation text", format!(".text({})", rust_string(text))))
            }
            mcp_twill::ConfirmationSegment::Argument {
                argument,
                rendering,
                fallback,
            } => required.push((
                "confirmation argument",
                format!(
                    ".argument({}, {}, {})",
                    rust_string(argument),
                    argument_rendering_source(*rendering),
                    rust_string(fallback)
                ),
            )),
        }
    }
    ensure!(
        declaration.matches("        .confirmation(").count() == 1,
        "displayed declaration must contain exactly one confirmation for {}",
        operation.id
    );

    ensure!(
        declaration.contains(".uses_conversation_identity()")
            == operation.uses_conversation_identity,
        "displayed declaration conversation context drifted for {}",
        operation.id
    );
    ensure!(
        declaration.contains(".idempotent()") == operation.idempotent,
        "displayed declaration idempotency drifted for {}",
        operation.id
    );
    required.push((
        "task support",
        format!(
            ".task_support({})",
            task_support_source(&operation.task_support)
        ),
    ));

    ensure!(
        declaration.matches("        .example_with_args(").count() == operation.examples.len(),
        "displayed declaration duplicated or omitted examples for {}",
        operation.id
    );
    for example in &operation.examples {
        required.push(("example command", rust_string(&example.command)));
        required.push(("example summary", rust_string(&example.summary)));
        required.push((
            "example arguments",
            indented_json(&serde_json::to_value(&example.args)?, 16)?,
        ));
    }

    let application = operation
        .output
        .application
        .as_ref()
        .context("site specimen operation is missing result contract")?;
    ensure!(
        application.errors.is_empty(),
        "displayed declaration requires explicit application-error coverage"
    );
    required.push((
        "empty application error inventory",
        "// Application errors: none.".to_string(),
    ));
    required.push((
        "application success schema",
        indented_json(&application.success_schema, 12)?,
    ));
    ensure!(
        declaration
            .matches("        .result_contract(ApplicationResultContract::new(")
            .count()
            == 1,
        "displayed declaration must contain exactly one result contract for {}",
        operation.id
    );

    for (fact, fragment) in required {
        ensure!(
            declaration.contains(&fragment),
            "displayed declaration omitted {fact} for {}: {fragment}",
            operation.id
        );
    }
    Ok(())
}

fn rust_configuration(operation: &mcp_twill::OperationSpec) -> Result<RustConfiguration> {
    let title = operation
        .args
        .iter()
        .find(|argument| argument.name == "title")
        .context("site specimen operation is missing title argument")?;
    let title_min_length = match &title.schema {
        None => None,
        Some(mcp_twill::ArgumentSchemaUse::Inline { schema }) => schema
            .get("minLength")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .context("title minLength does not fit the displayed configuration")?,
        Some(mcp_twill::ArgumentSchemaUse::Named { .. }) => {
            bail!("site specimen title unexpectedly uses a named schema")
        }
    };
    let confirmation = operation
        .presentation
        .as_ref()
        .and_then(|presentation| presentation.confirmation.as_ref())
        .context("site specimen operation is missing confirmation")?;
    let confirmation_kind = if confirmation.default.body.iter().any(|segment| {
        matches!(
            segment,
            mcp_twill::ConfirmationSegment::Argument { argument, .. }
                if argument == "title"
        )
    }) {
        ConfirmationId::TitleInterpolated
    } else {
        ConfirmationId::Generic
    };
    Ok(RustConfiguration {
        title_min_length,
        permissions: operation
            .permissions
            .iter()
            .map(|permission| permission.effect.as_label())
            .collect(),
        confirmation_kind,
        uses_conversation_identity: operation.uses_conversation_identity,
        task_support: serde_json::to_value(&operation.task_support)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "optional".to_string()),
    })
}

fn validate_rust_configuration(
    configuration: &RustConfiguration,
    config: specimen::SpecimenConfig,
) -> Result<()> {
    let expected_title_min_length =
        matches!(config.title_rule, specimen::TitleRule::NonEmpty).then_some(1);
    ensure!(
        configuration.title_min_length == expected_title_min_length,
        "displayed title configuration drifted for {}",
        scenario_id(config)
    );
    let expected_permissions = match config.destination {
        specimen::Destination::Local => vec!["write".to_string()],
        specimen::Destination::Remote => vec!["write".to_string(), "network".to_string()],
    };
    ensure!(
        configuration.permissions == expected_permissions,
        "displayed permission configuration drifted for {}",
        scenario_id(config)
    );
    ensure!(
        configuration.confirmation_kind == selection(config).confirmation,
        "displayed confirmation configuration drifted for {}",
        scenario_id(config)
    );
    ensure!(
        configuration.uses_conversation_identity
            == matches!(
                config.private_context,
                specimen::PrivateContext::ConversationIdentity
            ),
        "displayed private-context configuration drifted for {}",
        scenario_id(config)
    );
    ensure!(
        configuration.task_support == "optional",
        "displayed task configuration drifted for {}",
        scenario_id(config)
    );
    Ok(())
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn indented_json(value: &Value, indentation: usize) -> Result<String> {
    specimen::render_json(value, indentation).map_err(anyhow::Error::msg)
}

fn argument_rendering_source(rendering: mcp_twill::ArgumentRendering) -> &'static str {
    match rendering {
        mcp_twill::ArgumentRendering::Plain => "ArgumentRendering::Plain",
        mcp_twill::ArgumentRendering::JsonString => "ArgumentRendering::JsonString",
        mcp_twill::ArgumentRendering::TrimmedJsonString => "ArgumentRendering::TrimmedJsonString",
    }
}

fn task_support_source(task_support: &mcp_twill::TaskSupportSpec) -> &'static str {
    match task_support {
        mcp_twill::TaskSupportSpec::Forbidden => "TaskSupportSpec::Forbidden",
        mcp_twill::TaskSupportSpec::Optional => "TaskSupportSpec::Optional",
        mcp_twill::TaskSupportSpec::Required => "TaskSupportSpec::Required",
    }
}

fn privacy_evidence(
    config: specimen::SpecimenConfig,
    handler_observed: bool,
    surfaces: &[(&str, Value)],
) -> Result<PrivacyEvidence> {
    let declared = matches!(
        config.private_context,
        specimen::PrivateContext::ConversationIdentity
    );
    ensure!(
        handler_observed == declared,
        "handler identity observation disagrees with declaration for {}",
        scenario_id(config)
    );
    let checks = surfaces
        .iter()
        .map(|(surface, value)| {
            let text = canonical_inline(value);
            PrivacyCheck {
                surface: (*surface).to_string(),
                absent: !contains_private_identity(&text),
            }
        })
        .collect::<Vec<_>>();
    ensure!(
        checks.iter().all(|check| check.absent),
        "private identity appeared in serialized evidence for {}",
        scenario_id(config)
    );
    Ok(PrivacyEvidence {
        declared,
        handler_observed,
        raw_identity_serialized: false,
        redacted_summary: if declared {
            "The handler observed host-supplied conversation identity; serialized projections contain only its declaration and fingerprint effect."
        } else {
            "No conversation identity was supplied or observed."
        }
        .to_string(),
        checks,
    })
}

fn contains_private_identity(value: &str) -> bool {
    [
        PRIVATE_ISSUER,
        PRIVATE_ID,
        ALTERNATE_PRIVATE_ISSUER,
        ALTERNATE_PRIVATE_ID,
    ]
    .iter()
    .any(|secret| value.contains(secret))
}

fn selection(config: specimen::SpecimenConfig) -> Selection {
    Selection {
        title_rule: match config.title_rule {
            specimen::TitleRule::Unconstrained => TitleRuleId::Unconstrained,
            specimen::TitleRule::NonEmpty => TitleRuleId::NonEmpty,
        },
        destination: match config.destination {
            specimen::Destination::Local => DestinationId::Local,
            specimen::Destination::Remote => DestinationId::Remote,
        },
        confirmation: match config.confirmation {
            specimen::ConfirmationKind::Generic => ConfirmationId::Generic,
            specimen::ConfirmationKind::TitleInterpolated => ConfirmationId::TitleInterpolated,
        },
        private_context: match config.private_context {
            specimen::PrivateContext::None => PrivateContextId::None,
            specimen::PrivateContext::ConversationIdentity => {
                PrivateContextId::ConversationIdentity
            }
        },
    }
}

fn scenario_id(config: specimen::SpecimenConfig) -> String {
    format!(
        "title-{}__destination-{}__confirmation-{}__context-{}",
        config.title_rule.id(),
        config.destination.id(),
        config.confirmation.id(),
        config.private_context.id()
    )
}

fn controls() -> Vec<Control> {
    vec![
        control(
            "titleRule",
            "fact.titleRule",
            "Title rule",
            &[
                ("unconstrained", "Unconstrained"),
                ("nonEmpty", "Non-empty"),
            ],
        ),
        control(
            "destination",
            "fact.destination",
            "Destination",
            &[
                ("local", "Local write"),
                ("remote", "Remote write + network"),
            ],
        ),
        control(
            "confirmation",
            "fact.confirmation",
            "Confirmation",
            &[
                ("generic", "Generic"),
                ("titleInterpolated", "Title-interpolated"),
            ],
        ),
        control(
            "privateContext",
            "fact.privateContext",
            "Private context",
            &[
                ("none", "None"),
                ("conversationIdentity", "Conversation identity"),
            ],
        ),
    ]
}

fn control(id: &str, fact_id: &str, label: &str, options: &[(&str, &str)]) -> Control {
    Control {
        id: id.to_string(),
        fact_id: fact_id.to_string(),
        label: label.to_string(),
        options: options
            .iter()
            .map(|(value, label)| ControlOption {
                value: (*value).to_string(),
                label: (*label).to_string(),
            })
            .collect(),
    }
}

fn validate_controls(controls: &[Control]) -> Result<()> {
    let expected = [
        ("titleRule", "fact.titleRule"),
        ("destination", "fact.destination"),
        ("confirmation", "fact.confirmation"),
        ("privateContext", "fact.privateContext"),
    ];
    ensure!(
        controls.len() == expected.len(),
        "expected four site evidence controls"
    );
    for (control, (expected_id, expected_fact_id)) in controls.iter().zip(expected) {
        ensure!(
            control.id == expected_id && control.fact_id == expected_fact_id,
            "control `{}` has unexpected causal fact `{}`",
            control.id,
            control.fact_id
        );
        ensure!(
            control.options.len() == 2,
            "control `{}` must have exactly two options",
            control.id
        );
    }
    Ok(())
}

fn validate_variants(variants: &[ScenarioVariant], controls: &[Control]) -> Result<()> {
    ensure!(variants.len() == 16, "expected 16 site evidence variants");
    let ids = variants
        .iter()
        .map(|variant| variant.id.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(ids.len() == 16, "site evidence variant ids are not unique");
    for variant in variants {
        validate_declaration_code_ranges(variant)?;
        validate_mcp_surface_comparison(variant)?;
        let facts = variant
            .declaration
            .facts
            .iter()
            .map(|fact| fact.id.as_str())
            .collect::<BTreeSet<_>>();
        let anchor_facts = variant
            .semantic_anchors
            .iter()
            .map(|anchor| anchor.source_fact.as_str())
            .collect::<BTreeSet<_>>();
        for control in controls {
            ensure!(
                facts.contains(control.fact_id.as_str())
                    && anchor_facts.contains(control.fact_id.as_str()),
                "{} control {} references missing causal fact {}",
                variant.id,
                control.id,
                control.fact_id
            );
        }
        ensure!(
            variant.comparison_targets.len() == 13,
            "{} has an incomplete comparison target inventory",
            variant.id
        );
        let targets = variant
            .comparison_targets
            .iter()
            .map(|target| target.id.as_str())
            .collect::<BTreeSet<_>>();
        for target in &variant.comparison_targets {
            let profiles = target.profiles.iter().copied().collect::<BTreeSet<_>>();
            ensure!(
                !profiles.is_empty() && profiles.len() == target.profiles.len(),
                "{} target {} has an invalid profile scope",
                variant.id,
                target.id
            );
            if target.projection != ProjectionId::Mcp {
                ensure!(
                    profiles
                        == [ProfileId::Compact, ProfileId::Native]
                            .into_iter()
                            .collect(),
                    "{} non-MCP target {} must apply to both profiles",
                    variant.id,
                    target.id
                );
            }
        }
        for anchor in &variant.semantic_anchors {
            for target in &anchor.target_ids {
                ensure!(
                    targets.contains(target.as_str()),
                    "{} anchor {} references missing target {}",
                    variant.id,
                    anchor.id,
                    target
                );
            }
        }
    }
    for variant in variants
        .iter()
        .filter(|variant| variant.selection.private_context == PrivateContextId::None)
    {
        let mut paired = variant.selection;
        paired.private_context = PrivateContextId::ConversationIdentity;
        let identity_variant = variant_for_selection(variants, paired)?;
        ensure!(
            variant.fingerprints.invocation != identity_variant.fingerprints.invocation,
            "conversation identity did not change the invocation fingerprint for {}",
            variant.id
        );
    }

    let causal_controls = [
        (
            "titleRule",
            &[
                "help.titleRule",
                "schema.titleRule",
                "mcp.native.titleRule",
                "host.titleRule",
            ][..],
        ),
        (
            "destination",
            &[
                "help.destination",
                "mcp.compact.destination",
                "mcp.native.destination",
                "host.destination",
            ][..],
        ),
        (
            "confirmation",
            &["confirmation.message", "host.confirmation"][..],
        ),
        (
            "privateContext",
            &[
                "help.privateContext",
                "mcp.privateContext",
                "host.privateContext",
            ][..],
        ),
    ];
    for (control, expected) in causal_controls {
        for variant in variants
            .iter()
            .filter(|variant| is_first_control_value(variant.selection, control))
        {
            let paired =
                variant_for_selection(variants, toggle_control(variant.selection, control))?;
            let changed = changed_comparison_targets(variant, paired);
            let expected = expected.iter().copied().collect::<BTreeSet<_>>();
            ensure!(
                changed == expected,
                "{} control `{control}` changed the wrong comparison targets\nexpected: {expected:#?}\nactual: {changed:#?}",
                variant.id
            );
            let mut changed_code_facts = BTreeSet::new();
            for fact in &variant.declaration.facts {
                let left = declaration_fact_snippet(variant, &fact.id)?;
                let right = declaration_fact_snippet(paired, &fact.id)?;
                if left != right {
                    changed_code_facts.insert(fact.id.as_str());
                }
            }
            let expected_code_fact = format!("fact.{control}");
            ensure!(
                changed_code_facts == [expected_code_fact.as_str()].into_iter().collect(),
                "{} control `{control}` changed the wrong declaration ranges: {changed_code_facts:#?}",
                variant.id
            );
        }
    }
    Ok(())
}

fn validate_mcp_surface_comparison(variant: &ScenarioVariant) -> Result<()> {
    let operation_id = required_string(&variant.catalog_operation, "id")?;
    let expected = mcp_surface_comparison(
        &operation_id,
        &variant.compact.tools,
        &variant.compact.selected_tool,
        &variant.native.surface,
        &variant.native.tool,
    )?;
    ensure!(
        variant.mcp_surface_comparison == expected,
        "{} MCP surface comparison disagrees with its generated projections",
        variant.id
    );
    for (surface_name, facts) in [
        ("compact", &variant.mcp_surface_comparison.compact),
        ("native", &variant.mcp_surface_comparison.native),
    ] {
        ensure!(
            !facts.tool_name.is_empty()
                && !facts.tool_inventory.is_empty()
                && !facts.required_inputs.is_empty()
                && !facts.input_fields.is_empty(),
            "{} {surface_name} MCP surface comparison has empty facts",
            variant.id
        );
    }
    Ok(())
}

fn declaration_fact_snippet(variant: &ScenarioVariant, fact_id: &str) -> Result<String> {
    let fact = variant
        .declaration
        .facts
        .iter()
        .find(|fact| fact.id == fact_id)
        .with_context(|| format!("{} is missing declaration fact {fact_id}", variant.id))?;
    let lines = variant.declaration.text.lines().collect::<Vec<_>>();
    let mut snippet = String::new();
    for range in &fact.code_ranges {
        for line_number in range.start_line..=range.end_line {
            let line = lines
                .get((line_number - 1) as usize)
                .with_context(|| format!("{} has an invalid declaration range", variant.id))?;
            snippet.push_str(line);
            snippet.push('\n');
        }
    }
    Ok(snippet)
}

fn validate_declaration_code_ranges(variant: &ScenarioVariant) -> Result<()> {
    let lines = variant.declaration.text.lines().collect::<Vec<_>>();
    let line_count = lines.len() as u32;
    let mut occupied_lines = BTreeMap::<u32, &str>::new();

    for fact in &variant.declaration.facts {
        let permits_empty = fact.id == "fact.privateContext"
            && variant.selection.private_context == PrivateContextId::None;
        ensure!(
            permits_empty || !fact.code_ranges.is_empty(),
            "{} fact {} is missing declaration code ranges",
            variant.id,
            fact.id
        );
        if fact.id == "fact.privateContext" {
            ensure!(
                fact.code_ranges.is_empty() == permits_empty,
                "{} private-context code range disagrees with its declaration",
                variant.id
            );
        }

        let mut previous_end = 0;
        let mut snippet = String::new();
        for range in &fact.code_ranges {
            ensure!(
                range.start_line >= 1
                    && range.start_line <= range.end_line
                    && range.end_line <= line_count,
                "{} fact {} has invalid declaration lines {}..={}",
                variant.id,
                fact.id,
                range.start_line,
                range.end_line
            );
            ensure!(
                range.start_line > previous_end,
                "{} fact {} has unsorted or overlapping declaration ranges",
                variant.id,
                fact.id
            );
            previous_end = range.end_line;

            for line_number in range.start_line..=range.end_line {
                ensure!(
                    occupied_lines.insert(line_number, &fact.id).is_none(),
                    "{} declaration line {} belongs to more than one fact",
                    variant.id,
                    line_number
                );
                let line = lines
                    .get((line_number - 1) as usize)
                    .context("validated declaration line is missing")?;
                snippet.push_str(line);
                snippet.push('\n');
            }
        }
        ensure!(
            fact.code_ranges.is_empty() || !snippet.trim().is_empty(),
            "{} fact {} points only at blank declaration lines",
            variant.id,
            fact.id
        );

        let required_fragment = match fact.id.as_str() {
            "fact.titleRule" => Some("arg::string(\"title\")"),
            "fact.destination" => Some(".write("),
            "fact.confirmation" => Some(".confirmation("),
            "fact.privateContext" if !fact.code_ranges.is_empty() => {
                Some(".uses_conversation_identity()")
            }
            "fact.privateContext" => None,
            other => bail!(
                "{} declaration has an unknown semantic fact {other}",
                variant.id
            ),
        };
        if let Some(fragment) = required_fragment {
            ensure!(
                snippet.contains(fragment),
                "{} fact {} range omitted its declaration fragment {fragment}",
                variant.id,
                fact.id
            );
        }
        if fact.id == "fact.destination" && variant.selection.destination == DestinationId::Remote {
            ensure!(
                snippet.contains(".network("),
                "{} remote destination range omitted its network declaration",
                variant.id
            );
        }
    }

    Ok(())
}

fn variant_for_selection(
    variants: &[ScenarioVariant],
    selection: Selection,
) -> Result<&ScenarioVariant> {
    variants
        .iter()
        .find(|variant| variant.selection == selection)
        .context("paired site evidence variant is missing")
}

fn is_first_control_value(selection: Selection, control: &str) -> bool {
    match control {
        "titleRule" => selection.title_rule == TitleRuleId::Unconstrained,
        "destination" => selection.destination == DestinationId::Local,
        "confirmation" => selection.confirmation == ConfirmationId::Generic,
        "privateContext" => selection.private_context == PrivateContextId::None,
        _ => false,
    }
}

fn toggle_control(mut selection: Selection, control: &str) -> Selection {
    match control {
        "titleRule" => selection.title_rule = TitleRuleId::NonEmpty,
        "destination" => selection.destination = DestinationId::Remote,
        "confirmation" => selection.confirmation = ConfirmationId::TitleInterpolated,
        "privateContext" => {
            selection.private_context = PrivateContextId::ConversationIdentity;
        }
        _ => unreachable!("validated causal control"),
    }
    selection
}

fn changed_comparison_targets<'a>(
    left: &'a ScenarioVariant,
    right: &'a ScenarioVariant,
) -> BTreeSet<&'a str> {
    left.comparison_targets
        .iter()
        .filter_map(|left_target| {
            let right_target = right
                .comparison_targets
                .iter()
                .find(|target| target.id == left_target.id)
                .expect("all variants carry the same comparison target inventory");
            (left_target.display_value != right_target.display_value)
                .then_some(left_target.id.as_str())
        })
        .collect()
}

fn generator() -> Generator {
    Generator {
        name: GENERATOR_NAME.to_string(),
        version: FORMAT_VERSION,
        command: GENERATOR_COMMAND.to_string(),
    }
}

fn source_hashes(repository: &Path) -> Result<BTreeMap<String, String>> {
    SOURCE_PATHS
        .iter()
        .map(|path| {
            let bytes = fs::read(repository.join(path))
                .with_context(|| format!("read evidence source `{path}`"))?;
            Ok(((*path).to_string(), sha256(&bytes)))
        })
        .collect()
}

fn source_provenance(path: &str) -> &'static str {
    match path {
        "docs/adoption/visible-browser-lab/baseline/catalog-measurement.json" => {
            "Frozen pre-port VBL catalog measurement; copied byte-for-byte."
        }
        "crates/mcp-twill/tests/fixtures/vbl/v0.4.9/manifest.json" => {
            "Frozen VBL v0.4.9 fixture manifest; copied byte-for-byte."
        }
        _ => "Repository source used by the Twill evidence generator.",
    }
}

fn repository_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("xtask manifest has no repository parent")
}

fn write_canonical_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let value = serde_json::to_value(value).context("serialize canonical JSON input")?;
    write_canonical_value(path, value)
}

fn write_canonical_value(path: &Path, value: Value) -> Result<()> {
    let canonical = canonicalize(value);
    let mut bytes = serde_json::to_vec_pretty(&canonical).context("encode canonical JSON")?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("write `{}`", path.display()))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        other => other,
    }
}

fn canonical_inline(value: &Value) -> String {
    serde_json::to_string(&canonicalize(value.clone())).unwrap_or_else(|_| "null".to_string())
}

fn required_string(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .with_context(|| format!("captured value is missing string `{field}`"))
}

fn copy_exact(source: &Path, destination: &Path) -> Result<()> {
    let bytes = fs::read(source).with_context(|| format!("read `{}`", source.display()))?;
    fs::write(destination, &bytes)
        .with_context(|| format!("copy `{}` to `{}`", source.display(), destination.display()))?;
    let copied =
        fs::read(destination).with_context(|| format!("re-read `{}`", destination.display()))?;
    ensure!(
        bytes == copied,
        "byte-for-byte evidence copy failed for `{}`",
        source.display()
    );
    Ok(())
}

fn sync_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("create `{}`", destination.display()))?;
    let source_files = collect_files(source)?;
    let destination_files = collect_files(destination)?;
    for stale in destination_files.difference(&source_files) {
        fs::remove_file(destination.join(stale))
            .with_context(|| format!("remove stale evidence `{}`", stale.display()))?;
    }
    for path in source_files {
        let target = destination.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create `{}`", parent.display()))?;
        }
        fs::copy(source.join(&path), &target)
            .with_context(|| format!("install generated evidence `{}`", path.display()))?;
    }
    compare_directories(source, destination)
}

fn compare_directories(expected: &Path, actual: &Path) -> Result<()> {
    ensure!(
        actual.is_dir(),
        "evidence directory `{}` does not exist",
        actual.display()
    );
    let expected_files = collect_files(expected)?;
    let actual_files = collect_files(actual)?;
    ensure!(
        expected_files == actual_files,
        "evidence inventory differs\nexpected: {expected_files:#?}\nactual: {actual_files:#?}"
    );
    for path in expected_files {
        let expected_bytes = fs::read(expected.join(&path))?;
        let actual_bytes = fs::read(actual.join(&path))?;
        ensure!(
            expected_bytes == actual_bytes,
            "evidence file `{}` differs",
            path.display()
        );
    }
    Ok(())
}

fn collect_files(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_files_under(root, root, &mut files)?;
    Ok(files)
}

fn collect_files_under(root: &Path, directory: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
    for entry in
        fs::read_dir(directory).with_context(|| format!("read `{}`", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_files_under(root, &path, files)?;
        } else if file_type.is_file() {
            files.insert(path.strip_prefix(root)?.to_path_buf());
        } else {
            bail!("unsupported evidence entry `{}`", path.display());
        }
    }
    Ok(())
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::specimen;
    use serde_json::json;

    #[tokio::test]
    async fn default_mcp_surface_comparison_matches_generated_tools() {
        let variant = super::generate_variant(specimen::SpecimenConfig::default())
            .await
            .expect("default site evidence variant generates");
        let comparison = &variant.mcp_surface_comparison;

        assert_eq!(comparison.operation_id, "issues.create");
        assert_eq!(comparison.compact.tool_name, "run-write");
        assert_eq!(
            comparison.compact.tool_inventory,
            ["help", "run", "run-write"]
        );
        assert_eq!(comparison.compact.required_inputs, ["command"]);
        assert_eq!(
            comparison.compact.input_fields,
            [
                "approval", "args", "command", "dryRun", "mode", "output", "stdin"
            ]
        );
        assert!(comparison.compact.has_argument_map);

        assert_eq!(comparison.native.tool_name, "issues_create");
        assert_eq!(comparison.native.tool_inventory, ["issues_create"]);
        assert_eq!(comparison.native.required_inputs, ["title", "body"]);
        assert_eq!(comparison.native.input_fields, ["body", "title"]);
        assert!(!comparison.native.has_argument_map);
    }

    #[tokio::test]
    async fn every_variant_has_consistent_mcp_surface_comparison() {
        let mut generated = 0;
        for title_rule in [
            specimen::TitleRule::Unconstrained,
            specimen::TitleRule::NonEmpty,
        ] {
            for destination in [specimen::Destination::Local, specimen::Destination::Remote] {
                for confirmation in [
                    specimen::ConfirmationKind::Generic,
                    specimen::ConfirmationKind::TitleInterpolated,
                ] {
                    for private_context in [
                        specimen::PrivateContext::None,
                        specimen::PrivateContext::ConversationIdentity,
                    ] {
                        let variant = super::generate_variant(specimen::SpecimenConfig {
                            title_rule,
                            destination,
                            confirmation,
                            private_context,
                        })
                        .await
                        .expect("site evidence variant generates");
                        super::validate_mcp_surface_comparison(&variant)
                            .expect("MCP comparison remains projection-derived");
                        generated += 1;
                    }
                }
            }
        }
        assert_eq!(generated, 16);
    }

    #[test]
    fn declaration_json_is_compact_and_schema_ordered() {
        let rendered = specimen::render_json(
            &json!({
                "additionalProperties": false,
                "properties": {
                    "body": { "type": "string" },
                    "id": { "type": "integer" },
                    "status": { "type": "string" },
                    "title": { "type": "string" }
                },
                "required": ["id", "title", "body", "status"],
                "type": "object"
            }),
            4,
        )
        .expect("schema renders");

        assert_eq!(
            rendered,
            concat!(
                "    {\n",
                "      \"type\": \"object\",\n",
                "      \"required\": [\"id\", \"title\", \"body\", \"status\"],\n",
                "      \"properties\": {\n",
                "        \"id\": { \"type\": \"integer\" },\n",
                "        \"title\": { \"type\": \"string\" },\n",
                "        \"body\": { \"type\": \"string\" },\n",
                "        \"status\": { \"type\": \"string\" }\n",
                "      },\n",
                "      \"additionalProperties\": false\n",
                "    }"
            )
        );
    }

    #[test]
    fn rendered_declaration_rejects_unknown_range_facts() {
        let ranges = [specimen::DeclarationCodeRange {
            fact_id: "fact.typo",
            start_line: 1,
            end_line: 1,
        }];
        let error = super::validate_rendered_range_fact_ids(
            &ranges,
            &[
                "fact.titleRule",
                "fact.destination",
                "fact.confirmation",
                "fact.privateContext",
            ],
        )
        .expect_err("unknown renderer facts must fail closed");

        assert!(
            error
                .to_string()
                .contains("unknown code-range fact fact.typo")
        );
    }

    #[test]
    fn tracked_site_evidence_is_current() {
        super::export(true).expect("tracked site evidence matches the Twill generator");
    }
}
