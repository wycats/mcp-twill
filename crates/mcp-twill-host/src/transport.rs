use std::{borrow::Cow, collections::BTreeMap, error::Error, fmt};

use mcp_twill::{
    ApplicationErrorBody, ApplicationRecovery, CliMcpServer, CommandExecutionOutcome,
    ConversationIdentity, ErrorCode, FrameworkError, HostNativeConfirmationMode,
    HostNativeExecutionOutcome, HostWorkspaceRootsObservation, InvocationContext,
    NativeApplicationErrorDialect, NativeApplicationRecovery, ResponseEnvelope, Result,
};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    HostAdapterProfile, HostApplicationRejection, HostConfirmationAuthority,
    HostConfirmationTrigger, HostContextReason, HostInvocationTransport, HostRecoveryAction,
    VsCodeVersion,
    canonical::{UniqueJsonError, canonical_json, parse_unique_json},
};

const TRANSPORT_VERSION: u32 = 1;
const MAX_HOST_TEXT_SCALARS: usize = 1_024;

struct ConversationIdentityTransportSchemaV1;

impl JsonSchema for ConversationIdentityTransportSchemaV1 {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ConversationIdentityTransportV1".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "properties": {
                "version": { "type": "integer", "const": 1 },
                "issuer": {
                    "type": "string",
                    "pattern": "^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$"
                },
                "id": { "type": "string", "minLength": 1 }
            },
            "required": ["version", "issuer", "id"],
            "additionalProperties": false
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostInvocationContextV1 {
    Ambient {
        #[schemars(with = "ConversationIdentityTransportSchemaV1")]
        conversation_identity: ConversationIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_roots: Option<HostWorkspaceRootsObservation>,
    },
    Absent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_roots: Option<HostWorkspaceRootsObservation>,
    },
    Unsupported {
        reason: HostContextReason,
    },
}

impl fmt::Debug for HostInvocationContextV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ambient { .. } => formatter
                .debug_struct("Ambient")
                .field("conversation_identity", &"<redacted>")
                .field("workspace_roots", &"<redacted>")
                .finish(),
            Self::Absent { .. } => formatter
                .debug_struct("Absent")
                .field("workspace_roots", &"<redacted>")
                .finish(),
            Self::Unsupported { reason } => formatter
                .debug_struct("Unsupported")
                .field("reason", reason)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostRuntimeFactsV1 {
    VsCode {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        engine_version: Option<HostVsCodeVersionV1>,
    },
}

impl fmt::Debug for HostRuntimeFactsV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VsCode { .. } => formatter
                .debug_struct("VsCode")
                .field("engine_version", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostVsCodeVersionV1 {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl From<HostVsCodeVersionV1> for VsCodeVersion {
    fn from(value: HostVsCodeVersionV1) -> Self {
        Self::new(value.major, value.minor, value.patch)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCallEnvelopeV1 {
    pub version: u32,
    pub host_profile: String,
    pub host_adapter_hash: String,
    pub surface_hash: String,
    pub tool: String,
    pub arguments: BTreeMap<String, Value>,
    pub context: HostInvocationContextV1,
    pub runtime: HostRuntimeFactsV1,
}

impl fmt::Debug for HostCallEnvelopeV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostCallEnvelopeV1")
            .field("version", &self.version)
            .field("host_profile", &"<redacted>")
            .field("host_adapter_hash", &"<redacted>")
            .field("surface_hash", &"<redacted>")
            .field("tool", &"<redacted>")
            .field("arguments", &"<redacted>")
            .field("context", &"<redacted>")
            .field("runtime", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCallResultV1 {
    pub version: u32,
    pub host_adapter_hash: String,
    pub surface_hash: String,
    pub outcome: HostCallOutcomeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostCallOutcomeV1 {
    Success { text: String },
    ApplicationError { code: String, text: String },
    FrameworkError { code: String, text: String },
}

pub struct HostInProcessAdapter {
    profile: HostAdapterProfile,
    server: CliMcpServer,
}

impl HostAdapterProfile {
    pub fn bind_in_process(self, server: CliMcpServer) -> Result<HostInProcessAdapter> {
        if !matches!(
            self.declaration.transport,
            HostInvocationTransport::InProcess
        ) {
            return Err(build_error(
                "only an in-process host profile can bind an in-process adapter",
            ));
        }
        validate_server_pair(&self, &server)?;
        Ok(HostInProcessAdapter {
            profile: self,
            server,
        })
    }
}

impl HostInProcessAdapter {
    pub async fn call(
        &self,
        tool: &str,
        arguments: BTreeMap<String, Value>,
        context: HostInvocationContextV1,
        runtime: HostRuntimeFactsV1,
    ) -> HostCallResultV1 {
        execute_host_call(
            &self.profile,
            &self.server,
            tool,
            arguments,
            context,
            runtime,
        )
        .await
    }
}

#[derive(Default)]
pub struct HostProcessRouter {
    hosts: BTreeMap<String, RegisteredHost>,
}

struct RegisteredHost {
    profile: HostAdapterProfile,
    server: CliMcpServer,
}

pub struct HostProcessEntrypointError {
    reason: &'static str,
}

impl fmt::Debug for HostProcessEntrypointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HostProcessEntrypointError")
            .field(&self.reason)
            .finish()
    }
}

impl fmt::Display for HostProcessEntrypointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason)
    }
}

impl Error for HostProcessEntrypointError {}

impl HostProcessRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        host: HostAdapterProfile,
        server: CliMcpServer,
    ) -> Result<&mut Self> {
        if !matches!(
            host.declaration.transport,
            HostInvocationTransport::ProcessEnvelopeV1 { .. }
        ) {
            return Err(build_error(
                "host process router accepts only process-envelope profiles",
            ));
        }
        validate_server_pair(&host, &server)?;
        if self.hosts.contains_key(&host.declaration.id) {
            return Err(build_error(
                "host process router profile ids are single-assignment",
            ));
        }
        self.hosts.insert(
            host.declaration.id.clone(),
            RegisteredHost {
                profile: host,
                server,
            },
        );
        Ok(self)
    }

    pub async fn serve_stdio_v1(
        &self,
        profile_id: &str,
        host_adapter_hash: &str,
    ) -> std::result::Result<(), HostProcessEntrypointError> {
        let registered = self
            .hosts
            .get(profile_id)
            .filter(|registered| {
                registered.profile.snapshot.host_adapter_hash() == host_adapter_hash
            })
            .ok_or_else(|| entrypoint_error("generated host profile is unavailable"))?;
        let HostInvocationTransport::ProcessEnvelopeV1 { .. } =
            &registered.profile.declaration.transport
        else {
            return Err(entrypoint_error("generated host profile is unavailable"));
        };
        attach_process_tree_v1()?;
        let limit = registered
            .profile
            .declaration
            .invocation_limits
            .max_call_bytes as usize;
        let bytes = read_bounded_stdin(limit).await?;
        let result =
            handle_process_call_bytes(registered, profile_id, host_adapter_hash, &bytes).await?;
        let mut stdout = tokio::io::stdout();
        stdout
            .write_all(&result)
            .await
            .map_err(|_| entrypoint_error("generated host result could not be written"))?;
        stdout
            .flush()
            .await
            .map_err(|_| entrypoint_error("generated host result could not be written"))
    }
}

#[cfg(not(windows))]
fn attach_process_tree_v1() -> std::result::Result<(), HostProcessEntrypointError> {
    Ok(())
}

#[cfg(windows)]
fn attach_process_tree_v1() -> std::result::Result<(), HostProcessEntrypointError> {
    use std::{ffi::c_void, mem::size_of, ptr};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::GetCurrentProcess,
        },
    };

    // SAFETY: every pointer passed to the Windows job APIs is either null as
    // documented or points to a live, correctly sized information record.
    // The successful handle intentionally remains open for this one-shot
    // wrapper's process lifetime. Its closure on normal exit or forced
    // termination applies KILL_ON_JOB_CLOSE to non-breakaway descendants.
    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return Err(entrypoint_error(
                "generated host process tree could not be initialized",
            ));
        }
        let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const information).cast::<c_void>(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
            || AssignProcessToJobObject(job, GetCurrentProcess()) == 0
        {
            CloseHandle(job);
            return Err(entrypoint_error(
                "generated host process tree could not be initialized",
            ));
        }
    }
    Ok(())
}

async fn handle_process_call_bytes(
    registered: &RegisteredHost,
    profile_id: &str,
    host_adapter_hash: &str,
    bytes: &[u8],
) -> std::result::Result<Vec<u8>, HostProcessEntrypointError> {
    let value = match parse_unique_json(bytes) {
        Ok(value) => value,
        Err(UniqueJsonError::Malformed | UniqueJsonError::DuplicateVersion) => {
            return Err(entrypoint_error("generated host call is malformed"));
        }
        Err(UniqueJsonError::DuplicateContext) => {
            let result = framework_result(
                &registered.profile,
                "invalid_request_context",
                "Generated host call contained invalid request context",
            );
            return Ok(bounded_result_bytes(&registered.profile, result));
        }
        Err(UniqueJsonError::DuplicateContract) => {
            let result = framework_result(
                &registered.profile,
                "host_contract_mismatch",
                contract_message(),
            );
            return Ok(bounded_result_bytes(&registered.profile, result));
        }
    };
    let version = value
        .as_object()
        .and_then(|value| value.get("version"))
        .and_then(Value::as_u64)
        .ok_or_else(|| entrypoint_error("generated host call is malformed"))?;
    if version != u64::from(TRANSPORT_VERSION) {
        return Err(entrypoint_error(
            "generated host call uses an unsupported version",
        ));
    }

    let result = match decode_canonical_envelope(bytes, &value) {
        Ok(envelope)
            if envelope.host_profile == profile_id
                && envelope.host_adapter_hash == host_adapter_hash
                && envelope.surface_hash == registered.profile.snapshot.surface_hash() =>
        {
            execute_host_call(
                &registered.profile,
                &registered.server,
                &envelope.tool,
                envelope.arguments,
                envelope.context,
                envelope.runtime,
            )
            .await
        }
        Ok(_) | Err(EnvelopeDecodeFailure::Contract) => framework_result(
            &registered.profile,
            "host_contract_mismatch",
            contract_message(),
        ),
        Err(EnvelopeDecodeFailure::Context) => framework_result(
            &registered.profile,
            "invalid_request_context",
            "Generated host call contained invalid request context",
        ),
    };
    Ok(bounded_result_bytes(&registered.profile, result))
}

enum EnvelopeDecodeFailure {
    Contract,
    Context,
}

fn decode_canonical_envelope(
    bytes: &[u8],
    value: &Value,
) -> std::result::Result<HostCallEnvelopeV1, EnvelopeDecodeFailure> {
    let canonical = canonical_json(value).map_err(|_| EnvelopeDecodeFailure::Contract)?;
    if canonical != bytes {
        return Err(EnvelopeDecodeFailure::Contract);
    }
    let context = value
        .as_object()
        .and_then(|object| object.get("context"))
        .cloned()
        .ok_or(EnvelopeDecodeFailure::Contract)?;
    serde_json::from_value::<HostInvocationContextV1>(context)
        .map_err(|_| EnvelopeDecodeFailure::Context)?;
    serde_json::from_value(value.clone()).map_err(|_| EnvelopeDecodeFailure::Contract)
}

async fn read_bounded_stdin(
    limit: usize,
) -> std::result::Result<Vec<u8>, HostProcessEntrypointError> {
    let mut stdin = tokio::io::stdin();
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = stdin
            .read(&mut buffer)
            .await
            .map_err(|_| entrypoint_error("generated host call could not be read"))?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > limit {
            return Err(entrypoint_error("generated host call exceeded its bound"));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

async fn execute_host_call(
    profile: &HostAdapterProfile,
    server: &CliMcpServer,
    tool: &str,
    arguments: BTreeMap<String, Value>,
    context: HostInvocationContextV1,
    runtime: HostRuntimeFactsV1,
) -> HostCallResultV1 {
    let Some(compiled_tool) = profile
        .tools
        .iter()
        .find(|candidate| candidate.host_name == tool)
    else {
        return framework_result(profile, "host_contract_mismatch", contract_message());
    };
    let context = if compiled_tool.operations.is_empty() {
        HostInvocationContextV1::Absent {
            workspace_roots: None,
        }
    } else {
        context
    };
    if validate_map_container_depth(&arguments, 128).is_err() {
        return bounded_result(
            profile,
            framework_result(profile, "host_contract_mismatch", contract_message()),
        );
    }
    let envelope = HostCallEnvelopeV1 {
        version: TRANSPORT_VERSION,
        host_profile: profile.declaration.id.clone(),
        host_adapter_hash: profile.snapshot.host_adapter_hash().to_string(),
        surface_hash: profile.snapshot.surface_hash().to_string(),
        tool: tool.to_string(),
        arguments: arguments.clone(),
        context: context.clone(),
        runtime: runtime.clone(),
    };
    let call_size = serde_json::to_value(&envelope)
        .map_err(|_| ())
        .and_then(|value| {
            validate_container_depth(value.get("context").ok_or(())?, 128)?;
            canonical_json(&value).map_err(|_| ())
        })
        .map(|bytes| bytes.len());
    let call_size = match call_size {
        Ok(size) => size,
        Err(()) => {
            return bounded_result(
                profile,
                framework_result(profile, "host_contract_mismatch", contract_message()),
            );
        }
    };
    if call_size > profile.declaration.invocation_limits.max_call_bytes as usize {
        return bounded_result(
            profile,
            payload_result(
                profile,
                "call",
                profile.declaration.invocation_limits.max_call_bytes,
            ),
        );
    }
    if compiled_tool.operations.is_empty() {
        let execution = server
            .execute_host_native_call(
                &compiled_tool.native_name,
                arguments.into_iter().collect(),
                InvocationContext::new(),
                HostNativeConfirmationMode::ServerOnly,
            )
            .await;
        if execution.operation_id().is_some()
            || execution.native_tool() != Some(compiled_tool.native_name.as_str())
        {
            return framework_result(profile, "host_contract_mismatch", contract_message());
        }
        let result = match execution.into_outcome() {
            HostNativeExecutionOutcome::FrameworkHelp(value) => {
                let text = serde_json::to_string(&value)
                    .ok()
                    .unwrap_or_else(|| "{}".to_string());
                HostCallResultV1 {
                    version: TRANSPORT_VERSION,
                    host_adapter_hash: profile.snapshot.host_adapter_hash().to_string(),
                    surface_hash: profile.snapshot.surface_hash().to_string(),
                    outcome: HostCallOutcomeV1::Success { text },
                }
            }
            HostNativeExecutionOutcome::Command(outcome) => match *outcome {
                Err(error) => render_framework_error(profile, &compiled_tool.native_name, error),
                Ok(_) => framework_result(profile, "host_contract_mismatch", contract_message()),
            },
        };
        return bounded_result(profile, result);
    }
    let Some(operation_id) = resolve_operation(profile, compiled_tool, &arguments) else {
        return framework_result(profile, "host_contract_mismatch", contract_message());
    };
    let Some(operation) = profile.operation(&operation_id) else {
        return framework_result(profile, "host_contract_mismatch", contract_message());
    };
    let confirmation_mode = match confirmation_mode(profile, operation, &runtime) {
        Ok(mode) => mode,
        Err(()) => {
            return framework_result(profile, "host_contract_mismatch", contract_message());
        }
    };
    let absent_context = matches!(&context, HostInvocationContextV1::Absent { .. });
    let invocation = match context {
        HostInvocationContextV1::Ambient {
            conversation_identity,
            workspace_roots,
        } => {
            let mut invocation =
                InvocationContext::new().with_conversation_identity(conversation_identity);
            if let Some(roots) = workspace_roots {
                invocation = invocation.with_host_workspace_roots(roots);
            }
            invocation
        }
        HostInvocationContextV1::Absent { workspace_roots } => {
            if let Some(rejection) = profile
                .declaration
                .absent_context
                .rejections
                .get(&operation_id)
            {
                return bounded_result(
                    profile,
                    application_rejection_result(profile, &operation.native_tool, rejection),
                );
            }
            let mut invocation = InvocationContext::new();
            if let Some(roots) = workspace_roots {
                invocation = invocation.with_host_workspace_roots(roots);
            }
            invocation
        }
        HostInvocationContextV1::Unsupported { reason } => {
            if !profile
                .declaration
                .unsupported_context
                .allowed_operations
                .contains(&operation_id)
            {
                let message = profile
                    .declaration
                    .unsupported_context
                    .reasons
                    .get(&reason)
                    .expect("compiled host profile contains every reason");
                let text = render_error_text(
                    &operation.native_tool,
                    "unsupported_host",
                    message,
                    None,
                    profile.declaration.unsupported_context.recovery.as_ref(),
                );
                return bounded_result(
                    profile,
                    application_family_result(profile, false, "unsupported_host", text),
                );
            }
            InvocationContext::new()
        }
    };

    let execution = server
        .execute_host_native_call(
            &compiled_tool.native_name,
            arguments.into_iter().collect(),
            invocation,
            confirmation_mode,
        )
        .await;
    if execution.operation_id() != Some(operation_id.as_str())
        || execution.native_tool() != Some(operation.native_tool.as_str())
    {
        return framework_result(profile, "host_contract_mismatch", contract_message());
    }
    let result = match execution.into_outcome() {
        HostNativeExecutionOutcome::Command(outcome) => match *outcome {
            Ok(CommandExecutionOutcome::Success(response)) => {
                let mut value = response
                    .output
                    .and_then(|output| output.structured)
                    .unwrap_or_else(|| Value::Object(Map::new()));
                if let Some(fixed) = operation.call.arguments() {
                    let Some(object) = value.as_object_mut() else {
                        return bounded_result(
                            profile,
                            framework_result(
                                profile,
                                "result_contract_violation",
                                "Generated host result violated its declared contract",
                            ),
                        );
                    };
                    for (name, selected) in fixed {
                        object.insert(name.clone(), selected.clone());
                    }
                }
                if let Some(object) = value.as_object_mut() {
                    for property in &operation.result_omissions {
                        object.remove(property);
                    }
                }
                let text = serde_json::to_string(&value)
                    .ok()
                    .unwrap_or_else(|| "{}".to_string());
                HostCallResultV1 {
                    version: TRANSPORT_VERSION,
                    host_adapter_hash: profile.snapshot.host_adapter_hash().to_string(),
                    surface_hash: profile.snapshot.surface_hash().to_string(),
                    outcome: HostCallOutcomeV1::Success { text },
                }
            }
            Ok(CommandExecutionOutcome::ApplicationError { error, .. }) => {
                render_application_error(profile, operation, error, absent_context)
            }
            Err(error) => {
                if absent_context && let Some(rejection) = absent_binding_rejection(profile, &error)
                {
                    application_rejection_result(profile, &operation.native_tool, rejection)
                } else {
                    render_framework_error(profile, &operation.native_tool, error)
                }
            }
        },
        HostNativeExecutionOutcome::FrameworkHelp(_) => {
            framework_result(profile, "host_contract_mismatch", contract_message())
        }
    };
    bounded_result(profile, result)
}

fn absent_binding_rejection<'a>(
    profile: &'a HostAdapterProfile,
    error: &FrameworkError,
) -> Option<&'a HostApplicationRejection> {
    let FrameworkError::ResourceBindingMissing { establish, .. } = error else {
        return None;
    };
    profile
        .declaration
        .absent_context
        .rejections
        .iter()
        .find_map(|(operation_id, rejection)| {
            let operation = profile.operation(operation_id)?;
            establish
                .iter()
                .any(|provider| provider == &operation.command_name)
                .then_some(rejection)
        })
}

fn resolve_operation(
    profile: &HostAdapterProfile,
    tool: &crate::profile::CompiledHostTool,
    arguments: &BTreeMap<String, Value>,
) -> Option<String> {
    let matches = tool
        .operations
        .iter()
        .filter(|operation_id| {
            profile.operation(operation_id).is_some_and(|operation| {
                match operation.call.arguments() {
                    None => true,
                    Some(fixed) => fixed
                        .iter()
                        .all(|(name, value)| arguments.get(name) == Some(value)),
                }
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
}

fn confirmation_mode(
    profile: &HostAdapterProfile,
    operation: &crate::profile::CompiledHostOperation,
    runtime: &HostRuntimeFactsV1,
) -> std::result::Result<HostNativeConfirmationMode, ()> {
    let HostRuntimeFactsV1::VsCode { engine_version } = runtime;
    let HostConfirmationAuthority::TrustedVsCodeUi { engine_range } =
        profile.declaration.confirmation.authority
    else {
        return Ok(HostNativeConfirmationMode::ServerOnly);
    };
    let Some(version) = engine_version.map(VsCodeVersion::from) else {
        return Ok(HostNativeConfirmationMode::ServerOnly);
    };
    if !engine_range.contains(version) {
        return Ok(HostNativeConfirmationMode::ServerOnly);
    }
    if !operation.trusted_confirmation {
        return Ok(HostNativeConfirmationMode::ServerOnly);
    }
    Ok(match profile.declaration.confirmation.trigger {
        HostConfirmationTrigger::None => return Err(()),
        HostConfirmationTrigger::EffectDefault => HostNativeConfirmationMode::TrustedEffectDefault,
        HostConfirmationTrigger::DeclaredPresentation => {
            HostNativeConfirmationMode::TrustedDeclaredPresentation
        }
    })
}

fn application_rejection_result(
    profile: &HostAdapterProfile,
    native_tool: &str,
    rejection: &HostApplicationRejection,
) -> HostCallResultV1 {
    let identity = profile.operations.values().find_map(|operation| {
        operation
            .application_errors
            .get(&rejection.application_code)
    });
    let message = rejection
        .runtime_message
        .as_deref()
        .or_else(|| identity.map(|identity| identity.summary.as_str()))
        .unwrap_or("Application rejected this host invocation");
    let text = render_error_text(
        native_tool,
        &rejection.application_code,
        message,
        None,
        rejection.recovery.as_ref(),
    );
    application_family_result(profile, true, &rejection.application_code, text)
}

fn render_application_error(
    profile: &HostAdapterProfile,
    operation: &crate::profile::CompiledHostOperation,
    error: ApplicationErrorBody,
    absent_context: bool,
) -> HostCallResultV1 {
    if !operation.application_errors.contains_key(&error.code)
        || !profile.application_codes.contains(&error.code)
    {
        return framework_result(profile, "host_contract_mismatch", contract_message());
    }
    if absent_context
        && let Some(rejection) = error.recoveries.iter().find_map(|recovery| {
            let ApplicationRecovery::Operation { operation_id } = recovery else {
                return None;
            };
            profile
                .declaration
                .absent_context
                .rejections
                .get(operation_id)
                .filter(|rejection| rejection.application_code == error.code)
        })
    {
        let identity = profile.operations.values().find_map(|candidate| {
            candidate
                .application_errors
                .get(&rejection.application_code)
        });
        let message = rejection
            .runtime_message
            .as_deref()
            .or_else(|| identity.map(|identity| identity.summary.as_str()))
            .unwrap_or("Application rejected this host invocation");
        let text = render_error_text(
            &operation.native_tool,
            &error.code,
            message,
            None,
            rejection.recovery.as_ref(),
        );
        return application_family_result(profile, true, &error.code, text);
    }
    let details = (error.details != Value::Object(Map::new()))
        .then(|| compact_json(&error.details))
        .flatten();
    let recovery = render_recoveries(profile, &error.recoveries, absent_context);
    let text = render_error_text(
        &operation.native_tool,
        &error.code,
        &error.message,
        details.as_deref(),
        recovery.as_ref(),
    );
    application_family_result(profile, true, &error.code, text)
}

fn render_recoveries(
    profile: &HostAdapterProfile,
    recoveries: &[ApplicationRecovery],
    absent_context: bool,
) -> Option<HostRecoveryAction> {
    if recoveries.is_empty() {
        return None;
    }
    let projected = recoveries
        .iter()
        .map(|recovery| match recovery {
            ApplicationRecovery::Operation { operation_id } => {
                if absent_context
                    && let Some(action) = profile
                        .declaration
                        .absent_context
                        .rejections
                        .get(operation_id)
                        .and_then(|rejection| rejection.recovery.as_ref())
                {
                    return Some(NativeApplicationRecovery::Action {
                        code: action.code.clone(),
                        summary: action.summary.clone(),
                    });
                }
                let operation = profile.operation(operation_id)?;
                Some(NativeApplicationRecovery::Tool {
                    tool: operation.native_tool.clone(),
                    arguments: operation.call.arguments().cloned().unwrap_or_default(),
                })
            }
            ApplicationRecovery::Action { code, summary } => {
                Some(NativeApplicationRecovery::Action {
                    code: code.clone(),
                    summary: summary.clone(),
                })
            }
        })
        .collect::<Option<Vec<_>>>()?;
    let summary = match profile.native_application_errors {
        NativeApplicationErrorDialect::Canonical => {
            compact_json(&serde_json::to_value(projected).ok()?)?
        }
        NativeApplicationErrorDialect::FlatSingleRecovery => {
            if projected.len() != 1 {
                return None;
            }
            match &projected[0] {
                NativeApplicationRecovery::Tool { tool, .. } => tool.clone(),
                NativeApplicationRecovery::Action { code, .. } => code.clone(),
            }
        }
    };
    Some(HostRecoveryAction {
        code: "declared".to_string(),
        summary,
    })
}

fn render_framework_error(
    profile: &HostAdapterProfile,
    native_tool: &str,
    error: FrameworkError,
) -> HostCallResultV1 {
    let envelope = ResponseEnvelope::framework_error(error, None, None);
    let Some(body) = envelope.error else {
        return framework_result(profile, "handler_failed", "Framework request failed");
    };
    let code = wire_error_code(&body.code);
    if !profile.framework_codes.contains(code) {
        return framework_result(profile, "host_contract_mismatch", contract_message());
    }
    let text = render_error_text(native_tool, code, &body.message, None, None);
    application_family_result(profile, false, code, text)
}

fn render_error_text(
    subject: &str,
    code: &str,
    message: &str,
    details: Option<&str>,
    recovery: Option<&HostRecoveryAction>,
) -> String {
    let mut parts = vec![format!("{subject} failed with {code}"), message.to_string()];
    if let Some(details) = details {
        parts.push(format!("Details: {details}"));
    }
    if let Some(recovery) = recovery {
        parts.push(format!("Recovery: {}", recovery.summary));
    }
    encode_and_truncate_host_text(&parts.join(". "), MAX_HOST_TEXT_SCALARS)
}

pub(crate) fn render_declared_error_text(
    subject: &str,
    code: &str,
    message: &str,
    recovery: Option<&HostRecoveryAction>,
) -> String {
    render_error_text(subject, code, message, None, recovery)
}

fn encode_and_truncate_host_text(text: &str, limit: usize) -> String {
    let mut chunks = Vec::new();
    let mut input = text.chars().peekable();
    while let Some(scalar) = input.next() {
        let chunk = if scalar == '\\' {
            let mut escaped = String::from('\\');
            match input.peek().copied() {
                Some('u') => {
                    let mut clone = input.clone();
                    clone.next();
                    let digits = clone.by_ref().take(4).collect::<String>();
                    if digits.len() == 4 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        escaped.push(input.next().expect("peeked Unicode escape marker"));
                        escaped.extend(input.by_ref().take(4));
                    }
                }
                Some('"' | '\\' | 'b' | 'f' | 'n' | 'r' | 't') => {
                    escaped.push(input.next().expect("peeked short escape marker"));
                }
                _ => {}
            }
            escaped
        } else if host_text_scalar_is_unsafe(scalar) {
            format!("\\u{:04X}", scalar as u32)
        } else {
            scalar.to_string()
        };
        chunks.push(chunk);
    }
    let total = chunks
        .iter()
        .map(|chunk| chunk.chars().count())
        .sum::<usize>();
    if total <= limit {
        return chunks.concat();
    }
    let mut output = String::new();
    let mut width = 0;
    for chunk in chunks {
        let chunk_width = chunk.chars().count();
        if width + chunk_width > limit.saturating_sub(1) {
            break;
        }
        output.push_str(&chunk);
        width += chunk_width;
    }
    output.push('…');
    output
}

fn host_text_scalar_is_unsafe(scalar: char) -> bool {
    matches!(
        scalar,
        '\u{0000}'..='\u{001F}'
            | '\u{007F}'..='\u{009F}'
            | '\u{061C}'
            | '\u{200E}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

fn compact_json(value: &Value) -> Option<String> {
    serde_json::to_string(value).ok()
}

fn validate_container_depth(value: &Value, maximum: usize) -> std::result::Result<(), ()> {
    let mut stack = vec![(value, 0_usize)];
    while let Some((value, containers)) = stack.pop() {
        match value {
            Value::Array(values) => {
                let containers = containers + 1;
                if containers > maximum {
                    return Err(());
                }
                stack.extend(values.iter().map(|value| (value, containers)));
            }
            Value::Object(values) => {
                let containers = containers + 1;
                if containers > maximum {
                    return Err(());
                }
                stack.extend(values.values().map(|value| (value, containers)));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_map_container_depth(
    value: &BTreeMap<String, Value>,
    maximum: usize,
) -> std::result::Result<(), ()> {
    if maximum == 0 {
        return Err(());
    }
    let mut stack = value
        .values()
        .map(|value| (value, 1_usize))
        .collect::<Vec<_>>();
    while let Some((value, containers)) = stack.pop() {
        match value {
            Value::Array(values) => {
                let containers = containers + 1;
                if containers > maximum {
                    return Err(());
                }
                stack.extend(values.iter().map(|value| (value, containers)));
            }
            Value::Object(values) => {
                let containers = containers + 1;
                if containers > maximum {
                    return Err(());
                }
                stack.extend(values.values().map(|value| (value, containers)));
            }
            _ => {}
        }
    }
    Ok(())
}

fn application_family_result(
    profile: &HostAdapterProfile,
    application: bool,
    code: &str,
    text: String,
) -> HostCallResultV1 {
    HostCallResultV1 {
        version: TRANSPORT_VERSION,
        host_adapter_hash: profile.snapshot.host_adapter_hash().to_string(),
        surface_hash: profile.snapshot.surface_hash().to_string(),
        outcome: if application {
            HostCallOutcomeV1::ApplicationError {
                code: code.to_string(),
                text,
            }
        } else {
            HostCallOutcomeV1::FrameworkError {
                code: code.to_string(),
                text,
            }
        },
    }
}

fn framework_result(profile: &HostAdapterProfile, code: &str, message: &str) -> HostCallResultV1 {
    application_family_result(
        profile,
        false,
        code,
        render_error_text("generated host call", code, message, None, None),
    )
}

fn payload_result(profile: &HostAdapterProfile, direction: &str, limit: u32) -> HostCallResultV1 {
    framework_result(
        profile,
        "host_payload_too_large",
        &format!("Generated host {direction} exceeds the configured {limit}-byte limit"),
    )
}

fn bounded_result(profile: &HostAdapterProfile, result: HostCallResultV1) -> HostCallResultV1 {
    let within_limit = serde_json::to_value(&result)
        .ok()
        .and_then(|value| canonical_json(&value).ok())
        .is_some_and(|bytes| {
            bytes.len() <= profile.declaration.invocation_limits.max_result_bytes as usize
        });
    if within_limit {
        result
    } else {
        payload_result(
            profile,
            "result",
            profile.declaration.invocation_limits.max_result_bytes,
        )
    }
}

fn bounded_result_bytes(profile: &HostAdapterProfile, result: HostCallResultV1) -> Vec<u8> {
    let result = bounded_result(profile, result);
    serde_json::to_value(result)
        .ok()
        .and_then(|value| canonical_json(&value).ok())
        .unwrap_or_else(|| {
            br#"{"hostAdapterHash":"","outcome":{"code":"host_contract_mismatch","kind":"framework_error","text":"Generated host adapter failed"},"surfaceHash":"","version":1}"#.to_vec()
        })
}

fn validate_server_pair(profile: &HostAdapterProfile, server: &CliMcpServer) -> Result<()> {
    let identity = server.runtime_identity();
    let surface = identity
        .surface
        .ok_or_else(|| build_error("host adapter requires a finalized native server"))?;
    if surface.name != profile.declaration.surface
        || surface.hash != profile.snapshot.surface_hash()
        || identity.catalog_hash != profile.snapshot.catalog_hash()
    {
        return Err(build_error(
            "host adapter profile does not match the finalized native server",
        ));
    }
    Ok(())
}

fn wire_error_code(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::EmptyCommand => "empty_command",
        ErrorCode::UnterminatedQuote => "unterminated_quote",
        ErrorCode::ShellSyntax => "shell_syntax",
        ErrorCode::InvalidPlaceholder => "invalid_placeholder",
        ErrorCode::PlaceholderInterpolation => "placeholder_interpolation",
        ErrorCode::UnknownCommand => "unknown_command",
        ErrorCode::UnknownArgument => "unknown_argument",
        ErrorCode::MissingArgument => "missing_argument",
        ErrorCode::InvalidArgumentType => "invalid_argument_type",
        ErrorCode::WorkspaceMismatch => "workspace_mismatch",
        ErrorCode::UnresolvedWorkspaceRequirement => "unresolved_workspace_requirement",
        ErrorCode::AmbiguousWorkspaceRoot => "ambiguous_workspace_root",
        ErrorCode::UnsupportedRootScheme => "unsupported_root_scheme",
        ErrorCode::CapabilityMissing => "capability_missing",
        ErrorCode::CapabilityDenied => "capability_denied",
        ErrorCode::ResourceRefused => "resource_refused",
        ErrorCode::ResourceBindingMissing => "resource_binding_missing",
        ErrorCode::InvalidRequestContext => "invalid_request_context",
        ErrorCode::StdinMismatch => "stdin_mismatch",
        ErrorCode::WrongEffectLane => "wrong_effect_lane",
        ErrorCode::PermissionRequired => "permission_required",
        ErrorCode::PermissionDenied => "permission_denied",
        ErrorCode::ConfirmationUnavailable => "confirmation_unavailable",
        ErrorCode::ConfirmationCanceled => "confirmation_canceled",
        ErrorCode::ConfirmationFailed => "confirmation_failed",
        ErrorCode::ApprovalInvalid => "approval_invalid",
        ErrorCode::BuildFailed => "build_failed",
        ErrorCode::HandlerFailed => "handler_failed",
        ErrorCode::ApplicationError => "application_error",
        ErrorCode::ResultContractViolation => "result_contract_violation",
        ErrorCode::ArgumentContractViolation => "argument_contract_violation",
        ErrorCode::HostContractMismatch => "host_contract_mismatch",
        ErrorCode::HostPayloadTooLarge => "host_payload_too_large",
        ErrorCode::UnsupportedHost => "unsupported_host",
    }
}

fn contract_message() -> &'static str {
    "Generated host adapter received an invalid result envelope"
}

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}

fn entrypoint_error(reason: &'static str) -> HostProcessEntrypointError {
    HostProcessEntrypointError { reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_twill::{
        ApplicationResultContract, ApplicationSuccess, CommandRegistry, CommandSpec,
        DynamicCommandFailure, FrameworkHelpProjection, McpProtocolTarget, NativeConfirmationRoute,
        NativeToolDecl, NativeToolSurface, OutputContract,
    };

    fn process_host() -> RegisteredHost {
        let spec = CommandSpec::new(["ping"], "Ping", "Return a fixed value.").with_output(
            OutputContract {
                application: Some(ApplicationResultContract::new(serde_json::json!({
                    "type": "object",
                    "properties": {"ok": {"type": "boolean"}},
                    "required": ["ok"],
                    "additionalProperties": false
                }))),
                ..OutputContract::default()
            },
        );
        let registry = CommandRegistry::new("host-transport-test", "Host transport test")
            .register_dynamic(spec, |_context| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(
                    serde_json::json!({"ok": true}),
                ))
            });
        let surface = NativeToolSurface::builder("host-transport")
            .framework_help(FrameworkHelpProjection::Omitted)
            .confirmation_route(NativeConfirmationRoute::Unavailable)
            .tool(NativeToolDecl::Direct {
                name: "ping".to_string(),
                operation_id: "ping".to_string(),
                title: Some("Ping".to_string()),
                description: Some("Return a fixed value.".to_string()),
            })
            .build(&registry, McpProtocolTarget::V2025_11_25)
            .expect("test native surface");
        let unsupported = [
            HostContextReason::UnknownTokenShape,
            HostContextReason::InvalidSessionResource,
            HostContextReason::InvalidWorkingDirectory,
            HostContextReason::ProviderFailed,
        ]
        .into_iter()
        .fold(crate::UnsupportedContextPolicy::new(), |policy, reason| {
            policy.reason(reason, "Host context is unavailable")
        });
        let profile =
            HostAdapterProfile::vscode("host-transport-test", crate::VsCodeVersion::new(1, 120, 0))
                .confirmation(crate::HostConfirmationPolicy::presentation_only(
                    crate::HostConfirmationTrigger::None,
                ))
                .unsupported_context(unsupported)
                .invocation_limits(crate::HostInvocationLimits::new(64 * 1024, 64 * 1024))
                .process_envelope(
                    "bin/host-transport-test",
                    ["host", "call"],
                    crate::HostProcessLimits::new(4 * 1024, 100),
                )
                .build(surface.snapshot())
                .expect("test host profile");
        let server = CliMcpServer::builder(registry)
            .surface(surface)
            .build()
            .expect("test server");
        RegisteredHost { profile, server }
    }

    fn call_value(host: &RegisteredHost) -> Value {
        serde_json::to_value(HostCallEnvelopeV1 {
            version: TRANSPORT_VERSION,
            host_profile: host.profile.declaration.id.clone(),
            host_adapter_hash: host.profile.snapshot.host_adapter_hash().to_string(),
            surface_hash: host.profile.snapshot.surface_hash().to_string(),
            tool: "ping".to_string(),
            arguments: BTreeMap::new(),
            context: HostInvocationContextV1::Absent {
                workspace_roots: None,
            },
            runtime: HostRuntimeFactsV1::VsCode {
                engine_version: None,
            },
        })
        .expect("test envelope")
    }

    fn decode_result(bytes: &[u8]) -> HostCallResultV1 {
        serde_json::from_slice(bytes).expect("test result envelope")
    }

    #[test]
    fn transport_spelling_is_closed_and_exact() {
        let context = HostInvocationContextV1::Absent {
            workspace_roots: None,
        };
        assert_eq!(
            serde_json::to_value(context).unwrap(),
            serde_json::json!({"kind": "absent"})
        );
        assert_eq!(
            serde_json::to_value(HostRuntimeFactsV1::VsCode {
                engine_version: Some(HostVsCodeVersionV1 {
                    major: 1,
                    minor: 128,
                    patch: 0,
                }),
            })
            .unwrap(),
            serde_json::json!({
                "kind": "vs_code",
                "engineVersion": {"major": 1, "minor": 128, "patch": 0}
            })
        );
    }

    #[test]
    fn rendered_text_is_bounded_by_unicode_scalars() {
        let text = "x".repeat(MAX_HOST_TEXT_SCALARS + 20);
        let truncated = encode_and_truncate_host_text(&text, MAX_HOST_TEXT_SCALARS);
        assert_eq!(truncated.chars().count(), MAX_HOST_TEXT_SCALARS);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn rendered_text_preserves_complete_uppercase_escapes() {
        let text = format!(
            "{}\\u2028{}",
            "x".repeat(MAX_HOST_TEXT_SCALARS - 1),
            '\u{0085}'
        );
        let truncated = encode_and_truncate_host_text(&text, MAX_HOST_TEXT_SCALARS);
        assert_eq!(truncated.chars().count(), MAX_HOST_TEXT_SCALARS);
        assert!(truncated.ends_with('…'));
        assert!(!truncated.ends_with("\\u20…"));
        assert!(!truncated.contains('\u{0085}'));
    }

    #[test]
    fn call_payload_fallback_is_bounded_as_a_result() {
        let mut host = process_host();
        host.profile.declaration.invocation_limits.max_call_bytes = u32::MAX;
        let result_fallback = payload_result(
            &host.profile,
            "result",
            host.profile.declaration.invocation_limits.max_result_bytes,
        );
        let result_bound = serde_json::to_value(&result_fallback)
            .ok()
            .and_then(|value| canonical_json(&value).ok())
            .expect("canonical result fallback")
            .len();
        host.profile.declaration.invocation_limits.max_result_bytes =
            u32::try_from(result_bound).expect("test result bound");
        let call_fallback = payload_result(
            &host.profile,
            "call",
            host.profile.declaration.invocation_limits.max_call_bytes,
        );

        assert_eq!(
            bounded_result(&host.profile, call_fallback),
            payload_result(
                &host.profile,
                "result",
                host.profile.declaration.invocation_limits.max_result_bytes,
            )
        );
    }

    #[test]
    fn framework_inventory_uses_stable_wire_spelling() {
        assert_eq!(
            wire_error_code(&ErrorCode::HostContractMismatch),
            "host_contract_mismatch"
        );
    }

    #[tokio::test]
    async fn process_bytes_require_canonical_closed_envelopes() {
        let host = process_host();
        let profile_id = host.profile.declaration.id.clone();
        let hash = host.profile.snapshot.host_adapter_hash().to_string();
        let value = call_value(&host);
        let canonical = canonical_json(&value).expect("canonical call");
        let success = handle_process_call_bytes(&host, &profile_id, &hash, &canonical)
            .await
            .expect("valid call");
        assert_eq!(
            decode_result(&success).outcome,
            HostCallOutcomeV1::Success {
                text: r#"{"ok":true}"#.to_string(),
            }
        );

        let mut noncanonical = vec![b' '];
        noncanonical.extend_from_slice(&canonical);
        assert!(matches!(
            decode_result(
                &handle_process_call_bytes(&host, &profile_id, &hash, &noncanonical)
                    .await
                    .expect("shape-valid call returns a result")
            )
            .outcome,
            HostCallOutcomeV1::FrameworkError { ref code, .. }
                if code == "host_contract_mismatch"
        ));

        let mut extended = value.clone();
        extended["context"]["unexpected"] = Value::Bool(true);
        let extended = canonical_json(&extended).expect("canonical extended call");
        assert!(matches!(
            decode_result(
                &handle_process_call_bytes(&host, &profile_id, &hash, &extended)
                    .await
                    .expect("invalid context returns a result")
            )
            .outcome,
            HostCallOutcomeV1::FrameworkError { ref code, .. }
                if code == "invalid_request_context"
        ));

        let duplicate_version = String::from_utf8(canonical)
            .expect("UTF-8 call")
            .replace(r#""version":1"#, r#""version":1,"version":1"#);
        assert!(
            handle_process_call_bytes(&host, &profile_id, &hash, duplicate_version.as_bytes())
                .await
                .is_err()
        );
    }
}
