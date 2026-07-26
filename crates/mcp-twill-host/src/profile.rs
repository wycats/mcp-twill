use std::collections::{BTreeMap, BTreeSet};

use mcp_twill::{
    ApplicationErrorSpec, ApplicationMessageDecl, ApplicationRecoveryDecl, EffectSpec,
    ExplicitCarrierPolicy, FrameworkError, NativeApplicationErrorDialect, NativeSurfaceCall,
    NativeToolDecl, NativeToolSurfaceSnapshot, ResourceBindingMode, Result, TaskSupportSpec,
    application_error_accepts_empty_details,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::canonical::{canonical_json, framed_snapshot_hash};

const SNAPSHOT_VERSION: u32 = 1;
const HOST_HASH_DOMAIN: &str = "io.github.wycats.mcp-twill/host-adapter";
const MINIMUM_VSCODE_VERSION: VsCodeVersion = VsCodeVersion::new(1, 120, 0);
const MAX_PROFILE_TEXT_SCALARS: usize = 1_024;

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}

#[derive(Debug, Clone)]
pub struct HostAdapterProfile {
    pub(crate) declaration: HostAdapterProfileDecl,
    pub(crate) snapshot: HostAdapterSnapshot,
    pub(crate) tools: Vec<CompiledHostTool>,
    pub(crate) operations: BTreeMap<String, CompiledHostOperation>,
    pub(crate) native_application_errors: NativeApplicationErrorDialect,
    pub(crate) application_codes: BTreeSet<String>,
    pub(crate) framework_codes: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledHostTool {
    pub(crate) native_name: String,
    pub(crate) host_name: String,
    pub(crate) user_description: String,
    pub(crate) document: Value,
    pub(crate) operations: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledHostOperation {
    pub(crate) command_name: String,
    pub(crate) native_tool: String,
    pub(crate) call: NativeSurfaceCall,
    pub(crate) trusted_confirmation: bool,
    pub(crate) result_omissions: BTreeSet<String>,
    pub(crate) application_errors: BTreeMap<String, ApplicationErrorSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompiledHostGuidance {
    pub(crate) server_prefix: String,
    pub(crate) tool_suffix: String,
    pub(crate) operation_suffixes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostAdapterProfileDecl {
    pub id: String,
    pub surface: String,
    pub kind: HostAdapterKind,
    #[serde(default, skip_serializing_if = "HostToolNameProjection::is_identity")]
    pub tool_names: HostToolNameProjection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prompt_references: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "HostGuidanceProjection::is_empty")]
    pub guidance: HostGuidanceProjection,
    pub confirmation: HostConfirmationPolicy,
    #[serde(default, skip_serializing_if = "HostResultProjection::is_default")]
    pub results: HostResultProjection,
    pub unsupported_context: UnsupportedContextPolicy,
    #[serde(default, skip_serializing_if = "AbsentContextPolicy::is_empty")]
    pub absent_context: AbsentContextPolicy,
    pub invocation_limits: HostInvocationLimits,
    pub transport: HostInvocationTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostAdapterKind {
    VsCodeLanguageModelTools { engine_floor: VsCodeVersion },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct VsCodeVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl VsCodeVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn caret_range(&self) -> String {
        format!("^{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VsCodeEngineRange {
    pub minimum_inclusive: VsCodeVersion,
    pub maximum_inclusive: VsCodeVersion,
}

impl VsCodeEngineRange {
    pub const fn inclusive(
        minimum_inclusive: VsCodeVersion,
        maximum_inclusive: VsCodeVersion,
    ) -> Self {
        Self {
            minimum_inclusive,
            maximum_inclusive,
        }
    }

    pub(crate) fn contains(&self, version: VsCodeVersion) -> bool {
        self.minimum_inclusive <= version && version <= self.maximum_inclusive
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum HostToolNameProjection {
    #[default]
    Identity,
    Prefix(String),
}

impl HostToolNameProjection {
    pub(crate) fn is_identity(&self) -> bool {
        matches!(self, Self::Identity)
    }

    fn project(&self, name: &str) -> String {
        match self {
            Self::Identity => name.to_string(),
            Self::Prefix(prefix) => format!("{prefix}{name}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum HostConfirmationTrigger {
    None,
    EffectDefault,
    DeclaredPresentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostConfirmationAuthority {
    ServerOnly,
    TrustedVsCodeUi { engine_range: VsCodeEngineRange },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostConfirmationPolicy {
    pub trigger: HostConfirmationTrigger,
    pub authority: HostConfirmationAuthority,
}

impl HostConfirmationPolicy {
    pub const fn presentation_only(trigger: HostConfirmationTrigger) -> Self {
        Self {
            trigger,
            authority: HostConfirmationAuthority::ServerOnly,
        }
    }

    pub const fn trusted_vscode_ui(
        trigger: HostConfirmationTrigger,
        engine_range: VsCodeEngineRange,
    ) -> Self {
        Self {
            trigger,
            authority: HostConfirmationAuthority::TrustedVsCodeUi { engine_range },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct HostGuidanceProjection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_prefix: Vec<HostGuidanceSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_suffix: Vec<HostGuidanceSegment>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operation_suffixes: BTreeMap<String, Vec<HostGuidanceSegment>>,
}

impl HostGuidanceProjection {
    pub(crate) fn is_empty(&self) -> bool {
        self.server_prefix.is_empty()
            && self.tool_suffix.is_empty()
            && self.operation_suffixes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostGuidanceSegment {
    Text(String),
    Operation { operation_id: String },
    ResourceCarrier { resource: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct HostResultProjection {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub omit_top_level_properties: BTreeMap<String, BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "HostSuccessDialect::is_default")]
    pub success: HostSuccessDialect,
    #[serde(
        default,
        skip_serializing_if = "HostApplicationErrorDialect::is_default"
    )]
    pub application_error: HostApplicationErrorDialect,
    #[serde(default, skip_serializing_if = "HostFrameworkErrorDialect::is_default")]
    pub framework_error: HostFrameworkErrorDialect,
}

impl HostResultProjection {
    pub(crate) fn is_default(&self) -> bool {
        self.omit_top_level_properties.is_empty()
            && self.success.is_default()
            && self.application_error.is_default()
            && self.framework_error.is_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum HostSuccessDialect {
    #[default]
    CompactJsonText,
}

impl HostSuccessDialect {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Self::CompactJsonText)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum HostApplicationErrorDialect {
    #[default]
    ThrowBoundedText,
}

impl HostApplicationErrorDialect {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Self::ThrowBoundedText)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum HostFrameworkErrorDialect {
    #[default]
    ThrowBoundedText,
}

impl HostFrameworkErrorDialect {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Self::ThrowBoundedText)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HostContextReason {
    UnknownTokenShape,
    InvalidSessionResource,
    InvalidWorkingDirectory,
    ProviderFailed,
}

impl HostContextReason {
    const ALL: [Self; 4] = [
        Self::UnknownTokenShape,
        Self::InvalidSessionResource,
        Self::InvalidWorkingDirectory,
        Self::ProviderFailed,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedContextPolicy {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_operations: BTreeSet<String>,
    pub reasons: BTreeMap<HostContextReason, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<HostRecoveryAction>,
}

impl UnsupportedContextPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow(mut self, operation_id: impl Into<String>) -> Self {
        let operation_id = operation_id.into();
        if !self.allowed_operations.insert(operation_id) {
            self.allowed_operations.insert(String::new());
        }
        self
    }

    pub fn reason(mut self, reason: HostContextReason, summary: impl Into<String>) -> Self {
        if self.reasons.insert(reason, summary.into()).is_some() {
            self.reasons.insert(reason, String::new());
        }
        self
    }

    pub fn recover_by(
        mut self,
        action_code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        if self.recovery.is_some() {
            self.recovery = Some(HostRecoveryAction {
                code: String::new(),
                summary: String::new(),
            });
        } else {
            self.recovery = Some(HostRecoveryAction {
                code: action_code.into(),
                summary: summary.into(),
            });
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct AbsentContextPolicy {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rejections: BTreeMap<String, HostApplicationRejection>,
}

impl AbsentContextPolicy {
    pub(crate) fn is_empty(&self) -> bool {
        self.rejections.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostRecoveryAction {
    pub code: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostApplicationRejection {
    pub application_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<HostRecoveryAction>,
}

impl HostApplicationRejection {
    pub fn new(application_code: impl Into<String>) -> Self {
        Self {
            application_code: application_code.into(),
            runtime_message: None,
            recovery: None,
        }
    }

    pub fn runtime_message(mut self, message: impl Into<String>) -> Self {
        self.runtime_message = Some(if self.runtime_message.is_some() {
            String::new()
        } else {
            message.into()
        });
        self
    }

    pub fn recover_by(
        mut self,
        action_code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        self.recovery = Some(if self.recovery.is_some() {
            HostRecoveryAction {
                code: String::new(),
                summary: String::new(),
            }
        } else {
            HostRecoveryAction {
                code: action_code.into(),
                summary: summary.into(),
            }
        });
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostInvocationTransport {
    InProcess,
    ProcessEnvelopeV1 {
        logical_binary_name: String,
        subcommand: Vec<String>,
        limits: HostProcessLimits,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostInvocationLimits {
    pub max_call_bytes: u32,
    pub max_result_bytes: u32,
}

impl HostInvocationLimits {
    pub fn new(max_call_bytes: u32, max_result_bytes: u32) -> Self {
        Self {
            max_call_bytes,
            max_result_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostProcessLimits {
    pub max_stderr_bytes: u32,
    pub termination_grace_ms: u32,
}

impl HostProcessLimits {
    pub fn new(max_stderr_bytes: u32, termination_grace_ms: u32) -> Self {
        Self {
            max_stderr_bytes,
            termination_grace_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostAdapterSnapshot {
    version: u32,
    profile_id: String,
    native_protocol_version: String,
    catalog_hash: String,
    surface_hash: String,
    host_adapter_hash: String,
    pub(crate) profile: HostAdapterProfileDecl,
    pub(crate) tools: Vec<CompiledHostTool>,
    pub(crate) guidance: CompiledHostGuidance,
    document: Value,
    canonical_json: Box<[u8]>,
}

impl HostAdapterSnapshot {
    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn native_protocol_version(&self) -> &str {
        &self.native_protocol_version
    }

    pub fn catalog_hash(&self) -> &str {
        &self.catalog_hash
    }

    pub fn surface_hash(&self) -> &str {
        &self.surface_hash
    }

    pub fn host_adapter_hash(&self) -> &str {
        &self.host_adapter_hash
    }

    pub fn document(&self) -> &Value {
        &self.document
    }

    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }
}

#[derive(Debug)]
pub struct HostAdapterProfileBuilder {
    declaration: HostAdapterProfileDecl,
    confirmation_authored: bool,
    unsupported_context_authored: bool,
    invocation_limits_authored: bool,
    transport_authored: bool,
    tool_names_authored: bool,
    icon_authored: bool,
    guidance_authored: bool,
    errors: Vec<&'static str>,
}

impl HostAdapterProfile {
    pub fn declaration(&self) -> &HostAdapterProfileDecl {
        &self.declaration
    }

    pub fn snapshot(&self) -> &HostAdapterSnapshot {
        &self.snapshot
    }

    pub fn vscode(id: impl Into<String>, engine_floor: VsCodeVersion) -> HostAdapterProfileBuilder {
        HostAdapterProfileBuilder {
            declaration: HostAdapterProfileDecl {
                id: id.into(),
                surface: String::new(),
                kind: HostAdapterKind::VsCodeLanguageModelTools { engine_floor },
                tool_names: HostToolNameProjection::Identity,
                icon: None,
                prompt_references: BTreeMap::new(),
                guidance: HostGuidanceProjection::default(),
                confirmation: HostConfirmationPolicy::presentation_only(
                    HostConfirmationTrigger::None,
                ),
                results: HostResultProjection::default(),
                unsupported_context: UnsupportedContextPolicy::default(),
                absent_context: AbsentContextPolicy::default(),
                invocation_limits: HostInvocationLimits::new(0, 0),
                transport: HostInvocationTransport::InProcess,
            },
            confirmation_authored: false,
            unsupported_context_authored: false,
            invocation_limits_authored: false,
            transport_authored: false,
            tool_names_authored: false,
            icon_authored: false,
            guidance_authored: false,
            errors: Vec::new(),
        }
    }

    pub(crate) fn operation(&self, operation_id: &str) -> Option<&CompiledHostOperation> {
        self.operations.get(operation_id)
    }
}

impl HostAdapterProfileBuilder {
    pub fn tool_name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.assign_once(
            self.tool_names_authored,
            "host profile assigns tool names more than once",
        );
        if !self.tool_names_authored {
            self.tool_names_authored = true;
            self.declaration.tool_names = HostToolNameProjection::Prefix(prefix.into());
        }
        self
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.assign_once(
            self.icon_authored,
            "host profile assigns icon more than once",
        );
        if !self.icon_authored {
            self.icon_authored = true;
            self.declaration.icon = Some(icon.into());
        }
        self
    }

    pub fn guidance(mut self, projection: HostGuidanceProjection) -> Self {
        self.assign_once(
            self.guidance_authored,
            "host profile assigns guidance more than once",
        );
        if !self.guidance_authored {
            self.guidance_authored = true;
            self.declaration.guidance = projection;
        }
        self
    }

    pub fn prompt_reference(
        mut self,
        operation_id: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        if self
            .declaration
            .prompt_references
            .insert(operation_id.into(), reference.into())
            .is_some()
        {
            self.errors
                .push("host profile repeats a prompt reference operation");
        }
        self
    }

    pub fn confirmation(mut self, policy: HostConfirmationPolicy) -> Self {
        self.assign_once(
            self.confirmation_authored,
            "host profile assigns confirmation more than once",
        );
        if !self.confirmation_authored {
            self.confirmation_authored = true;
            self.declaration.confirmation = policy;
        }
        self
    }

    pub fn omit_result_property(
        mut self,
        operation_id: impl Into<String>,
        property: impl Into<String>,
    ) -> Self {
        let properties = self
            .declaration
            .results
            .omit_top_level_properties
            .entry(operation_id.into())
            .or_default();
        if !properties.insert(property.into()) {
            self.errors
                .push("host profile repeats a result property omission");
        }
        self
    }

    pub fn unsupported_context(mut self, policy: UnsupportedContextPolicy) -> Self {
        self.assign_once(
            self.unsupported_context_authored,
            "host profile assigns unsupported context more than once",
        );
        if !self.unsupported_context_authored {
            self.unsupported_context_authored = true;
            self.declaration.unsupported_context = policy;
        }
        self
    }

    pub fn absent_context_rejects(
        mut self,
        operation_id: impl Into<String>,
        rejection: HostApplicationRejection,
    ) -> Self {
        if self
            .declaration
            .absent_context
            .rejections
            .insert(operation_id.into(), rejection)
            .is_some()
        {
            self.errors
                .push("host profile repeats an absent-context rejection");
        }
        self
    }

    pub fn invocation_limits(mut self, limits: HostInvocationLimits) -> Self {
        self.assign_once(
            self.invocation_limits_authored,
            "host profile assigns invocation limits more than once",
        );
        if !self.invocation_limits_authored {
            self.invocation_limits_authored = true;
            self.declaration.invocation_limits = limits;
        }
        self
    }

    pub fn in_process(mut self) -> Self {
        self.assign_transport(HostInvocationTransport::InProcess);
        self
    }

    pub fn process_envelope(
        mut self,
        logical_binary_name: impl Into<String>,
        subcommand: impl IntoIterator<Item = impl AsRef<str>>,
        limits: HostProcessLimits,
    ) -> Self {
        self.assign_transport(HostInvocationTransport::ProcessEnvelopeV1 {
            logical_binary_name: logical_binary_name.into(),
            subcommand: subcommand
                .into_iter()
                .map(|part| part.as_ref().to_string())
                .collect(),
            limits,
        });
        self
    }

    pub fn build(mut self, surface: &NativeToolSurfaceSnapshot) -> Result<HostAdapterProfile> {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(build_error(error));
        }
        if !self.confirmation_authored {
            return Err(build_error(
                "host profile must explicitly assign confirmation policy",
            ));
        }
        if !self.unsupported_context_authored {
            return Err(build_error(
                "host profile must explicitly assign unsupported-context policy",
            ));
        }
        if !self.invocation_limits_authored {
            return Err(build_error(
                "host profile must explicitly assign invocation limits",
            ));
        }
        if !self.transport_authored {
            return Err(build_error(
                "host profile must explicitly assign invocation transport",
            ));
        }
        self.declaration.surface = surface.name().to_string();
        self.declaration.compile(surface)
    }

    fn assign_once(&mut self, authored: bool, message: &'static str) {
        if authored {
            self.errors.push(message);
        }
    }

    fn assign_transport(&mut self, transport: HostInvocationTransport) {
        self.assign_once(
            self.transport_authored,
            "host profile assigns invocation transport more than once",
        );
        if !self.transport_authored {
            self.transport_authored = true;
            self.declaration.transport = transport;
        }
    }
}

impl HostAdapterProfileDecl {
    pub fn compile(self, surface: &NativeToolSurfaceSnapshot) -> Result<HostAdapterProfile> {
        validate_profile_id(&self.id)?;
        if self.surface != surface.name() {
            return Err(build_error(
                "host profile surface does not match the compiled native surface",
            ));
        }
        validate_kind_and_confirmation(&self, surface)?;
        validate_limits_and_transport(&self)?;
        validate_context_policies(&self, surface)?;
        validate_guidance(&self, surface)?;
        let guidance = compile_guidance(&self.guidance, surface)?;

        let mut tools = Vec::new();
        let mut host_names = BTreeSet::new();
        let serialized_tools = serde_json::to_value(surface.tools())
            .map_err(|_| build_error("cannot project native tools into a host profile"))?;
        let serialized_tools = serialized_tools
            .as_array()
            .ok_or_else(|| build_error("native tool snapshot is not an array"))?;
        for (tool, document) in surface.tools().iter().zip(serialized_tools) {
            let native_name = tool.name.to_string();
            let host_name = self.tool_names.project(&native_name);
            validate_host_tool_name(&host_name)?;
            if !host_names.insert(host_name.clone()) {
                return Err(build_error("host profile produces duplicate tool names"));
            }
            let operations = surface
                .operations()
                .iter()
                .filter(|operation| operation.call().tool() == native_name)
                .map(|operation| operation.spec().id.clone())
                .collect::<Vec<_>>();
            let mut document = document.clone();
            let compiled_description = document
                .get("description")
                .and_then(Value::as_str)
                .ok_or_else(|| build_error("native tool snapshot is missing its description"))?;
            let user_description =
                surface_tool_user_description(surface, &native_name, compiled_description)?;
            if let [operation_id] = operations.as_slice()
                && let Some(properties) = self.results.omit_top_level_properties.get(operation_id)
            {
                project_result_schema(&mut document, properties)?;
            }
            tools.push(CompiledHostTool {
                native_name,
                host_name,
                user_description,
                document,
                operations,
            });
        }

        validate_prompt_references(&self, surface)?;
        validate_result_projection(&self, surface)?;

        let operations = surface
            .operations()
            .iter()
            .map(|operation| {
                let application_errors = operation
                    .spec()
                    .output
                    .application
                    .as_ref()
                    .map(|contract| {
                        contract
                            .errors
                            .iter()
                            .map(|error| (error.code.clone(), error.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    operation.spec().id.clone(),
                    CompiledHostOperation {
                        command_name: operation.spec().name(),
                        native_tool: operation.call().tool().to_string(),
                        call: operation.call().clone(),
                        trusted_confirmation: trigger_matches_operation(
                            self.confirmation.trigger,
                            operation.spec(),
                        ),
                        result_omissions: self
                            .results
                            .omit_top_level_properties
                            .get(&operation.spec().id)
                            .cloned()
                            .unwrap_or_default(),
                        application_errors,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        let application_codes = operations
            .values()
            .flat_map(|operation| operation.application_errors.keys().cloned())
            .chain(
                self.absent_context
                    .rejections
                    .values()
                    .map(|rejection| rejection.application_code.clone()),
            )
            .collect::<BTreeSet<_>>();
        let framework_codes = framework_code_inventory()
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        validate_fitting_limits(&self, &tools, &operations, surface)?;
        let tool_documents = tools
            .iter()
            .map(|tool| {
                json!({
                    "nativeName": tool.native_name,
                    "hostName": tool.host_name,
                    "userDescription": tool.user_description,
                    "operations": tool.operations,
                    "document": tool.document,
                })
            })
            .collect::<Vec<_>>();
        let document = json!({
            "version": SNAPSHOT_VERSION,
            "profile": self,
            "nativeSurface": {
                "version": surface.version(),
                "protocolVersion": surface.protocol_version(),
                "name": surface.name(),
                "catalogHash": surface.catalog_hash(),
                "surfaceHash": surface.surface_hash(),
                "document": surface.document(),
            },
            "serverInstructions": join_guidance(
                &guidance.server_prefix,
                surface.server_instructions(),
            ),
            "guidance": guidance,
            "tools": tool_documents,
            "applicationCodes": application_codes,
            "frameworkCodes": framework_codes,
        });
        let canonical = canonical_json(&document)?;
        let host_adapter_hash =
            framed_snapshot_hash(HOST_HASH_DOMAIN, SNAPSHOT_VERSION, &canonical);
        let snapshot = HostAdapterSnapshot {
            version: SNAPSHOT_VERSION,
            profile_id: self.id.clone(),
            native_protocol_version: surface.protocol_version().to_string(),
            catalog_hash: surface.catalog_hash().to_string(),
            surface_hash: surface.surface_hash().to_string(),
            host_adapter_hash,
            profile: self.clone(),
            tools: tools.clone(),
            guidance: guidance.clone(),
            document,
            canonical_json: canonical.into_boxed_slice(),
        };

        Ok(HostAdapterProfile {
            declaration: self,
            snapshot,
            tools,
            operations,
            native_application_errors: surface.declaration().application_errors,
            application_codes,
            framework_codes,
        })
    }
}

fn surface_tool_user_description(
    surface: &NativeToolSurfaceSnapshot,
    native_name: &str,
    compiled_description: &str,
) -> Result<String> {
    if matches!(
        &surface.declaration().framework_help,
        mcp_twill::FrameworkHelpProjection::Tool { name } if name == native_name
    ) {
        return Ok(compiled_description.to_string());
    }
    let declaration = surface
        .declaration()
        .tools
        .iter()
        .find(|tool| match tool {
            NativeToolDecl::Direct { name, .. } | NativeToolDecl::Group { name, .. } => {
                name == native_name
            }
        })
        .ok_or_else(|| build_error("native tool declaration is missing from its snapshot"))?;
    match declaration {
        NativeToolDecl::Direct { description, .. } | NativeToolDecl::Group { description, .. } => {
            Ok(description
                .clone()
                .unwrap_or_else(|| compiled_description.to_string()))
        }
    }
}

fn validate_fitting_limits(
    declaration: &HostAdapterProfileDecl,
    tools: &[CompiledHostTool],
    operations: &BTreeMap<String, CompiledHostOperation>,
    surface: &NativeToolSurfaceSnapshot,
) -> Result<()> {
    let placeholder_hash = "0".repeat(64);
    for tool in tools {
        let arguments = if tool.operations.is_empty() {
            vec![BTreeMap::new()]
        } else {
            tool.operations
                .iter()
                .filter_map(|operation_id| {
                    operations
                        .get(operation_id)
                        .map(|operation| operation.call.arguments().cloned().unwrap_or_default())
                })
                .collect()
        };
        let calls = arguments.into_iter().map(|arguments| {
            json!({
                "version": 1,
                "hostProfile": declaration.id.as_str(),
                "hostAdapterHash": placeholder_hash.as_str(),
                "surfaceHash": placeholder_hash.as_str(),
                "tool": tool.host_name.as_str(),
                "arguments": arguments,
                "context": {"kind": "absent"},
                "runtime": {"kind": "vs_code"},
            })
        });
        for call in calls {
            let size = canonical_json(&call)?.len();
            if size > declaration.invocation_limits.max_call_bytes as usize {
                return Err(build_error(
                    "host call bound cannot contain every minimal valid call envelope",
                ));
            }
        }
    }

    let mut fallback_texts = vec![
        (
            false,
            "host_contract_mismatch".to_string(),
            "Generated host adapter received an invalid result envelope".to_string(),
        ),
        (
            false,
            "invalid_request_context".to_string(),
            "Generated host call contained invalid request context".to_string(),
        ),
        (
            false,
            "host_payload_too_large".to_string(),
            format!(
                "Generated host result exceeds the configured {}-byte limit",
                declaration.invocation_limits.max_result_bytes
            ),
        ),
    ];
    let longest_tool = tools
        .iter()
        .max_by_key(|tool| tool.native_name.chars().count())
        .map(|tool| tool.native_name.as_str())
        .unwrap_or("generated host call");
    for summary in declaration.unsupported_context.reasons.values() {
        fallback_texts.push((
            false,
            "unsupported_host".to_string(),
            crate::transport::render_declared_error_text(
                longest_tool,
                "unsupported_host",
                summary,
                declaration.unsupported_context.recovery.as_ref(),
            ),
        ));
    }
    for (operation_id, rejection) in &declaration.absent_context.rejections {
        let identity = operations.values().find_map(|operation| {
            operation
                .application_errors
                .get(&rejection.application_code)
        });
        let message = rejection
            .runtime_message
            .as_deref()
            .or_else(|| identity.map(|identity| identity.summary.as_str()))
            .unwrap_or("Application rejected this host invocation");
        let subject = surface
            .operation(operation_id)
            .map(|operation| operation.call().tool())
            .unwrap_or(longest_tool);
        fallback_texts.push((
            true,
            rejection.application_code.clone(),
            crate::transport::render_declared_error_text(
                subject,
                &rejection.application_code,
                message,
                rejection.recovery.as_ref(),
            ),
        ));
    }
    for (application, code, message) in fallback_texts {
        let text = if message.contains(" failed with ") {
            message
        } else {
            crate::transport::render_declared_error_text(
                "generated host call",
                &code,
                &message,
                None,
            )
        };
        let result = json!({
            "version": 1,
            "hostAdapterHash": placeholder_hash.as_str(),
            "surfaceHash": placeholder_hash.as_str(),
            "outcome": {
                "kind": if application { "application_error" } else { "framework_error" },
                "code": code.as_str(),
                "text": text,
            }
        });
        if canonical_json(&result)?.len() > declaration.invocation_limits.max_result_bytes as usize
        {
            return Err(build_error(
                "host result bound cannot contain every static fallback envelope",
            ));
        }
    }
    for operation in surface.operations() {
        let Some(contract) = &operation.spec().output.application else {
            continue;
        };
        for error in &contract.errors {
            for recovery in &error.recoveries {
                let ApplicationRecoveryDecl::Operation {
                    operation_id: target,
                } = recovery
                else {
                    continue;
                };
                if declaration
                    .absent_context
                    .rejections
                    .get(target)
                    .is_some_and(|rejection| rejection.recovery.is_none())
                {
                    return Err(build_error(
                        "absent-context policy makes a recovery operation unavailable without a host recovery action",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_profile_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || id.starts_with('-')
        || id.ends_with('-')
        || id.split('-').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
    {
        return Err(build_error(
            "host profile id must use 1-64 lowercase kebab-case characters",
        ));
    }
    Ok(())
}

fn validate_kind_and_confirmation(
    declaration: &HostAdapterProfileDecl,
    surface: &NativeToolSurfaceSnapshot,
) -> Result<()> {
    let HostAdapterKind::VsCodeLanguageModelTools { engine_floor } = declaration.kind;
    if engine_floor.major != 1 || engine_floor < MINIMUM_VSCODE_VERSION {
        return Err(build_error(
            "VS Code host profiles require an engine floor in the supported 1.x family",
        ));
    }
    if surface
        .operations()
        .iter()
        .any(|operation| operation.spec().task_support == TaskSupportSpec::Required)
    {
        return Err(build_error(
            "version-1 host profiles cannot expose operations that require task delivery",
        ));
    }
    if let HostToolNameProjection::Prefix(prefix) = &declaration.tool_names
        && (prefix.is_empty()
            || prefix.chars().any(|character| {
                character.is_control()
                    || !character.is_ascii()
                    || !matches!(character, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' | '.')
            }))
    {
        return Err(build_error(
            "host tool-name prefix must be non-empty ASCII identifier text",
        ));
    }
    if let Some(icon) = &declaration.icon {
        validate_public_text(icon, "host icon")?;
    }
    if let HostConfirmationAuthority::TrustedVsCodeUi { engine_range } =
        declaration.confirmation.authority
    {
        if declaration.confirmation.trigger == HostConfirmationTrigger::None {
            return Err(build_error(
                "trusted VS Code confirmation authority requires a confirmation trigger",
            ));
        }
        if engine_range.minimum_inclusive > engine_range.maximum_inclusive
            || engine_range.minimum_inclusive < engine_floor
            || engine_range.maximum_inclusive.major != engine_floor.major
        {
            return Err(build_error(
                "trusted VS Code confirmation range must be non-empty and contained in the generated engine family",
            ));
        }
        let can_trigger = surface.operations().iter().any(|operation| {
            trigger_matches_operation(declaration.confirmation.trigger, operation.spec())
        });
        if !can_trigger {
            return Err(build_error(
                "trusted VS Code confirmation policy cannot match an exposed operation",
            ));
        }
    }
    Ok(())
}

fn trigger_matches_operation(
    trigger: HostConfirmationTrigger,
    operation: &mcp_twill::OperationSpec,
) -> bool {
    match trigger {
        HostConfirmationTrigger::None => false,
        HostConfirmationTrigger::DeclaredPresentation => operation
            .presentation
            .as_ref()
            .and_then(|presentation| presentation.confirmation.as_ref())
            .is_some(),
        HostConfirmationTrigger::EffectDefault => effect_requires_confirmation(&operation.effect),
    }
}

pub(crate) fn effect_requires_confirmation(effect: &EffectSpec) -> bool {
    match effect {
        EffectSpec::Write | EffectSpec::Delete | EffectSpec::Exec | EffectSpec::Network => true,
        EffectSpec::Composite(effects) => effects.iter().any(effect_requires_confirmation),
        EffectSpec::Pure | EffectSpec::Read | EffectSpec::Custom(_) => false,
    }
}

fn validate_limits_and_transport(declaration: &HostAdapterProfileDecl) -> Result<()> {
    if declaration.invocation_limits.max_call_bytes == 0
        || declaration.invocation_limits.max_result_bytes == 0
    {
        return Err(build_error("host invocation limits must be nonzero"));
    }
    if let HostInvocationTransport::ProcessEnvelopeV1 {
        logical_binary_name,
        subcommand,
        limits,
    } = &declaration.transport
    {
        validate_launch_token(logical_binary_name)?;
        if subcommand.is_empty() {
            return Err(build_error(
                "process host transport requires a non-empty subcommand",
            ));
        }
        for token in subcommand {
            validate_launch_token(token)?;
            if token == "--"
                || token.starts_with("--host-profile")
                || token.starts_with("--host-adapter-hash")
            {
                return Err(build_error(
                    "process host subcommand conflicts with generated selector arguments",
                ));
            }
        }
        if limits.max_stderr_bytes == 0 || !(1..=30_000).contains(&limits.termination_grace_ms) {
            return Err(build_error(
                "process host limits require nonzero stderr bytes and a 1-30000ms termination grace",
            ));
        }
    }
    Ok(())
}

fn validate_launch_token(token: &str) -> Result<()> {
    if token.is_empty() || token.contains('\0') {
        return Err(build_error(
            "host process launch tokens must be non-empty and NUL-free",
        ));
    }
    Ok(())
}

fn validate_context_policies(
    declaration: &HostAdapterProfileDecl,
    surface: &NativeToolSurfaceSnapshot,
) -> Result<()> {
    let operation_ids = surface
        .operations()
        .iter()
        .map(|operation| operation.spec().id.as_str())
        .collect::<BTreeSet<_>>();
    if declaration
        .unsupported_context
        .allowed_operations
        .contains("")
    {
        return Err(build_error(
            "unsupported-context policy repeats an allowed operation",
        ));
    }
    for operation in &declaration.unsupported_context.allowed_operations {
        if !operation_ids.contains(operation.as_str()) {
            return Err(build_error(
                "unsupported-context policy references an unknown operation",
            ));
        }
    }
    for reason in HostContextReason::ALL {
        let summary = declaration
            .unsupported_context
            .reasons
            .get(&reason)
            .ok_or_else(|| {
                build_error("unsupported-context policy must declare every stable reason")
            })?;
        validate_public_text(summary, "unsupported-context reason")?;
    }
    if let Some(recovery) = &declaration.unsupported_context.recovery {
        validate_recovery(recovery)?;
    }
    let mut recovery_summaries = BTreeMap::new();
    if let Some(recovery) = &declaration.unsupported_context.recovery {
        record_recovery_summary(&mut recovery_summaries, recovery)?;
    }

    let mut identities = BTreeMap::<String, ApplicationErrorSpec>::new();
    for operation in surface.operations() {
        if let Some(contract) = &operation.spec().output.application {
            for error in &contract.errors {
                if let Some(existing) = identities.insert(error.code.clone(), error.clone())
                    && existing != *error
                {
                    return Err(build_error(
                        "application error identity drifts across exposed operations",
                    ));
                }
            }
        }
    }
    for (operation_id, rejection) in &declaration.absent_context.rejections {
        let operation = surface
            .operation(operation_id)
            .ok_or_else(|| build_error("absent-context policy references an unknown operation"))?;
        if operation.call().arguments().is_some() {
            return Err(build_error(
                "absent-context application rejection requires a direct native tool",
            ));
        }
        let identity = identities.get(&rejection.application_code).ok_or_else(|| {
            build_error("absent-context rejection references an unknown application error identity")
        })?;
        if !application_error_accepts_empty_details(identity) {
            return Err(build_error(
                "absent-context rejection requires application details that accept an empty object",
            ));
        }
        match identity.message {
            ApplicationMessageDecl::DeclarationSummary => {
                if rejection.runtime_message.is_some() {
                    return Err(build_error(
                        "declaration-summary host rejection cannot supply a runtime message",
                    ));
                }
            }
            ApplicationMessageDecl::RuntimeBounded { max_scalar_values } => {
                let message = rejection.runtime_message.as_ref().ok_or_else(|| {
                    build_error("runtime-bounded host rejection requires a runtime message")
                })?;
                validate_public_text(message, "absent-context rejection message")?;
                if message.chars().count() > usize::from(max_scalar_values) {
                    return Err(build_error(
                        "absent-context rejection message exceeds its application bound",
                    ));
                }
            }
        }
        if let Some(recovery) = &rejection.recovery {
            validate_recovery(recovery)?;
            record_recovery_summary(&mut recovery_summaries, recovery)?;
        }
        if operation.spec().grants.is_empty() {
            return Err(build_error(
                "absent-context application rejection requires a resource-granting operation",
            ));
        }
        let omissions = declaration
            .results
            .omit_top_level_properties
            .get(operation_id)
            .ok_or_else(|| {
                build_error(
                    "absent-context application rejection requires a projected result omission",
                )
            })?;
        if omissions.is_empty() {
            return Err(build_error(
                "absent-context application rejection requires a projected result omission",
            ));
        }
        let has_unusable_grant = operation.spec().grants.iter().any(|resource| {
            let ambient_only = surface
                .declaration()
                .resource_bindings
                .iter()
                .any(|binding| {
                    binding.resource == *resource
                        && matches!(
                            binding.mode,
                            ResourceBindingMode::Ambient {
                                explicit: ExplicitCarrierPolicy::Omitted,
                                ..
                            }
                        )
                });
            ambient_only
                && surface
                    .resource_carrier(resource)
                    .is_some_and(|carrier| omissions.contains(carrier))
        });
        if !has_unusable_grant {
            return Err(build_error(
                "absent-context application rejection requires one granted resource whose explicit carrier and result are both omitted",
            ));
        }
    }
    Ok(())
}

fn record_recovery_summary(
    summaries: &mut BTreeMap<String, String>,
    recovery: &HostRecoveryAction,
) -> Result<()> {
    if let Some(existing) = summaries.insert(recovery.code.clone(), recovery.summary.clone())
        && existing != recovery.summary
    {
        return Err(build_error(
            "host recovery action codes must retain one summary within a profile",
        ));
    }
    Ok(())
}

fn validate_recovery(recovery: &HostRecoveryAction) -> Result<()> {
    if recovery.code.is_empty()
        || !recovery
            .code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        || recovery.code.starts_with('_')
        || recovery.code.ends_with('_')
        || recovery.code.contains("__")
    {
        return Err(build_error(
            "host recovery action code must use lower snake case",
        ));
    }
    validate_public_text(&recovery.summary, "host recovery summary")
}

fn validate_guidance(
    declaration: &HostAdapterProfileDecl,
    surface: &NativeToolSurfaceSnapshot,
) -> Result<()> {
    let operation_ids = surface
        .operations()
        .iter()
        .map(|operation| operation.spec().id.as_str())
        .collect::<BTreeSet<_>>();
    let structural_vocabulary = guidance_structural_vocabulary(surface);
    for operation in declaration.guidance.operation_suffixes.keys() {
        if !operation_ids.contains(operation.as_str()) {
            return Err(build_error("host guidance references an unknown operation"));
        }
    }
    for segments in declaration
        .guidance
        .server_prefix
        .iter()
        .map(std::slice::from_ref)
        .chain(
            declaration
                .guidance
                .tool_suffix
                .iter()
                .map(std::slice::from_ref),
        )
        .chain(
            declaration
                .guidance
                .operation_suffixes
                .values()
                .map(Vec::as_slice),
        )
    {
        for segment in segments {
            match segment {
                HostGuidanceSegment::Text(text) => {
                    validate_public_text(text, "host guidance text")?;
                    if structural_vocabulary
                        .iter()
                        .any(|token| contains_structural_token(text, token))
                    {
                        return Err(build_error(
                            "host guidance text embeds structural vocabulary; use a typed guidance segment",
                        ));
                    }
                }
                HostGuidanceSegment::Operation { operation_id } => {
                    if !operation_ids.contains(operation_id.as_str()) {
                        return Err(build_error("host guidance references an unknown operation"));
                    }
                }
                HostGuidanceSegment::ResourceCarrier { resource } => {
                    if !surface
                        .declaration()
                        .resource_bindings
                        .iter()
                        .any(|binding| binding.resource == *resource)
                    {
                        return Err(build_error(
                            "host guidance references an unknown resource binding",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn guidance_structural_vocabulary(surface: &NativeToolSurfaceSnapshot) -> BTreeSet<String> {
    let mut vocabulary = BTreeSet::new();
    for tool in surface.tools() {
        vocabulary.insert(tool.name.to_string());
    }
    for operation in surface.operations() {
        vocabulary.insert(operation.spec().id.clone());
        if let Some(arguments) = operation.call().arguments() {
            for value in arguments.values().filter_map(Value::as_str) {
                vocabulary.insert(format!("{} {value}", operation.call().tool()));
            }
        }
    }
    for binding in &surface.declaration().resource_bindings {
        if let Some(carrier) = surface.resource_carrier(&binding.resource) {
            vocabulary.insert(carrier.to_string());
        }
    }
    vocabulary
}

fn contains_structural_token(text: &str, token: &str) -> bool {
    text.match_indices(token).any(|(start, matched)| {
        let end = start + matched.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        !before.is_some_and(is_ascii_identifier_character)
            && !after.is_some_and(is_ascii_identifier_character)
    })
}

fn is_ascii_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
}

fn compile_guidance(
    guidance: &HostGuidanceProjection,
    surface: &NativeToolSurfaceSnapshot,
) -> Result<CompiledHostGuidance> {
    Ok(CompiledHostGuidance {
        server_prefix: render_guidance_segments(&guidance.server_prefix, surface)?,
        tool_suffix: render_guidance_segments(&guidance.tool_suffix, surface)?,
        operation_suffixes: guidance
            .operation_suffixes
            .iter()
            .map(|(operation, segments)| {
                render_guidance_segments(segments, surface)
                    .map(|rendered| (operation.clone(), rendered))
            })
            .collect::<Result<_>>()?,
    })
}

fn render_guidance_segments(
    segments: &[HostGuidanceSegment],
    surface: &NativeToolSurfaceSnapshot,
) -> Result<String> {
    segments
        .iter()
        .map(|segment| match segment {
            HostGuidanceSegment::Text(text) => Ok(text.clone()),
            HostGuidanceSegment::Operation { operation_id } => {
                let operation = surface
                    .operation(operation_id)
                    .ok_or_else(|| build_error("host guidance operation cannot be rendered"))?;
                let mut rendered = operation.call().tool().to_string();
                if let Some(arguments) = operation.call().arguments() {
                    for value in arguments.values().filter_map(Value::as_str) {
                        rendered.push(' ');
                        rendered.push_str(value);
                    }
                }
                Ok(rendered)
            }
            HostGuidanceSegment::ResourceCarrier { resource } => surface
                .resource_carrier(resource)
                .map(str::to_string)
                .ok_or_else(|| build_error("host guidance resource carrier cannot be rendered")),
        })
        .collect::<Result<String>>()
}

fn join_guidance(prefix: &str, body: &str) -> String {
    [prefix, body]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_prompt_references(
    declaration: &HostAdapterProfileDecl,
    surface: &NativeToolSurfaceSnapshot,
) -> Result<()> {
    let mut aliases = BTreeSet::new();
    for (operation_id, alias) in &declaration.prompt_references {
        let operation = surface
            .operation(operation_id)
            .ok_or_else(|| build_error("host prompt reference names an unknown operation"))?;
        if operation.call().arguments().is_some() {
            return Err(build_error(
                "host prompt references require directly mapped operations",
            ));
        }
        validate_host_tool_name(alias)?;
        if !aliases.insert(alias) {
            return Err(build_error("host prompt references must be unique"));
        }
    }
    Ok(())
}

fn validate_result_projection(
    declaration: &HostAdapterProfileDecl,
    surface: &NativeToolSurfaceSnapshot,
) -> Result<()> {
    for (operation_id, properties) in &declaration.results.omit_top_level_properties {
        let operation = surface
            .operation(operation_id)
            .ok_or_else(|| build_error("host result projection references an unknown operation"))?;
        if operation.call().arguments().is_some() {
            return Err(build_error(
                "host result property omission requires a directly mapped operation",
            ));
        }
        let schema = operation
            .spec()
            .output
            .application
            .as_ref()
            .map(|contract| &contract.success_schema)
            .ok_or_else(|| {
                build_error("host result property omission requires an application result contract")
            })?;
        let schema_properties = resolve_schema_object(schema, schema)
            .and_then(|schema| schema.get("properties"))
            .and_then(Value::as_object)
            .ok_or_else(|| build_error("host result omission requires an object success schema"))?;
        for property in properties {
            if !schema_properties.contains_key(property) {
                return Err(build_error(
                    "host result projection omits an unknown top-level property",
                ));
            }
        }
    }
    Ok(())
}

fn project_result_schema(document: &mut Value, properties: &BTreeSet<String>) -> Result<()> {
    let schema = document
        .get_mut("outputSchema")
        .ok_or_else(|| build_error("host result projection requires a native output schema"))?;
    let target_pointer = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix('#'))
        .map(ToOwned::to_owned);
    {
        let target = if let Some(pointer) = target_pointer {
            schema
                .pointer_mut(&pointer)
                .ok_or_else(|| build_error("host result projection cannot resolve output schema"))?
        } else {
            &mut *schema
        };
        let object = target
            .as_object_mut()
            .ok_or_else(|| build_error("host result projection requires an object schema"))?;
        let schema_properties = object
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| build_error("host result projection requires object properties"))?;
        for property in properties {
            schema_properties.remove(property);
        }
        if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|entry| entry.as_str().is_none_or(|name| !properties.contains(name)));
        }
    }
    prune_unreachable_definitions(schema);
    Ok(())
}

fn prune_unreachable_definitions(schema: &mut Value) {
    let Some(definitions) = schema.get("$defs").and_then(Value::as_object).cloned() else {
        return;
    };
    let references = definitions
        .keys()
        .map(|name| {
            (
                format!("#/$defs/{}", name.replace('~', "~0").replace('/', "~1")),
                name.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut reachable = BTreeSet::new();
    collect_definition_references_without_defs(schema, &references, &mut reachable);
    let mut pending = reachable.iter().cloned().collect::<Vec<_>>();
    while let Some(name) = pending.pop() {
        let Some(definition) = definitions.get(&name) else {
            continue;
        };
        let before = reachable.len();
        collect_definition_references(definition, &references, &mut reachable);
        if reachable.len() != before {
            pending.extend(reachable.iter().cloned());
        }
    }
    if let Some(retained) = schema.get_mut("$defs").and_then(Value::as_object_mut) {
        retained.retain(|name, _| reachable.contains(name));
        if retained.is_empty() {
            schema
                .as_object_mut()
                .expect("schema with definitions is an object")
                .remove("$defs");
        }
    }
}

fn collect_definition_references_without_defs(
    value: &Value,
    references: &BTreeMap<String, String>,
    found: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key != "$defs" {
                    collect_definition_references(value, references, found);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_definition_references(value, references, found);
            }
        }
        _ => {}
    }
}

fn collect_definition_references(
    value: &Value,
    references: &BTreeMap<String, String>,
    found: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str)
                && let Some(name) = references.get(reference)
            {
                found.insert(name.clone());
            }
            for value in object.values() {
                collect_definition_references(value, references, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_definition_references(value, references, found);
            }
        }
        _ => {}
    }
}

fn resolve_schema_object<'a>(
    schema: &'a Value,
    root: &'a Value,
) -> Option<&'a serde_json::Map<String, Value>> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let pointer = reference.strip_prefix('#')?;
        return root.pointer(pointer)?.as_object();
    }
    schema.as_object()
}

fn validate_host_tool_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(build_error(
            "generated host tool names must use 1-128 ASCII identifier characters",
        ));
    }
    Ok(())
}

fn validate_public_text(text: &str, subject: &str) -> Result<()> {
    if text.is_empty()
        || text.chars().count() > MAX_PROFILE_TEXT_SCALARS
        || text.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '\u{061C}'
                        | '\u{200E}'..='\u{200F}'
                        | '\u{2028}'..='\u{202E}'
                        | '\u{2060}'..='\u{206F}'
                        | '\u{FEFF}'
                )
        })
    {
        return Err(build_error(format!(
            "{subject} must be non-empty, bounded, and display-safe"
        )));
    }
    Ok(())
}

fn framework_code_inventory() -> BTreeSet<&'static str> {
    [
        "empty_command",
        "unterminated_quote",
        "shell_syntax",
        "invalid_placeholder",
        "placeholder_interpolation",
        "unknown_command",
        "unknown_argument",
        "missing_argument",
        "invalid_argument_type",
        "workspace_mismatch",
        "unresolved_workspace_requirement",
        "ambiguous_workspace_root",
        "unsupported_root_scheme",
        "capability_missing",
        "capability_denied",
        "resource_refused",
        "resource_binding_missing",
        "invalid_request_context",
        "stdin_mismatch",
        "wrong_effect_lane",
        "permission_required",
        "permission_denied",
        "confirmation_unavailable",
        "confirmation_canceled",
        "confirmation_failed",
        "approval_invalid",
        "build_failed",
        "handler_failed",
        "result_contract_violation",
        "argument_contract_violation",
        "host_contract_mismatch",
        "host_payload_too_large",
        "unsupported_host",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_confirmation_spellings_are_exact() {
        let version = VsCodeVersion::new(1, 120, 0);
        assert_eq!(version.caret_range(), "^1.120.0");
        assert_eq!(
            serde_json::to_value(HostConfirmationPolicy::trusted_vscode_ui(
                HostConfirmationTrigger::DeclaredPresentation,
                VsCodeEngineRange::inclusive(version, VsCodeVersion::new(1, 128, 0)),
            ))
            .unwrap(),
            json!({
                "trigger": "declaredPresentation",
                "authority": {
                    "trustedVsCodeUi": {
                        "engineRange": {
                            "minimumInclusive": {"major": 1, "minor": 120, "patch": 0},
                            "maximumInclusive": {"major": 1, "minor": 128, "patch": 0}
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn nested_builder_repetition_fails_closed() {
        let policy = UnsupportedContextPolicy::new()
            .allow("help")
            .allow("help")
            .reason(HostContextReason::UnknownTokenShape, "first")
            .reason(HostContextReason::UnknownTokenShape, "second");
        assert!(policy.allowed_operations.contains(""));
        assert_eq!(
            policy.reasons.get(&HostContextReason::UnknownTokenShape),
            Some(&String::new())
        );
    }
}
