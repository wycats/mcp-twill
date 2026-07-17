use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    CommandOutput, ConfirmationPolicy, FrameworkError, InvocationPlan, OutputSpec, PermissionSpec,
    ReplayRecord, ResponseProfile, RunRequest, RunResponse, WorkspaceDecl,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResponseStatus {
    Ok,
    InvalidInput,
    PermissionRequired,
    PermissionDenied,
    WrongEffectLane,
    ApprovalInvalid,
    NotFound,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    EmptyCommand,
    UnterminatedQuote,
    ShellSyntax,
    InvalidPlaceholder,
    PlaceholderInterpolation,
    UnknownCommand,
    UnknownArgument,
    MissingArgument,
    InvalidArgumentType,
    WorkspaceMismatch,
    /// Resolver diagnostic: no root matched a workspace requirement.
    UnresolvedWorkspaceRequirement,
    /// Resolver diagnostic: multiple roots matched a workspace requirement.
    AmbiguousWorkspaceRoot,
    /// Resolver diagnostic: a root URI used a scheme other than `file:`.
    UnsupportedRootScheme,
    /// A required capability's carrier argument was not bound at plan time.
    CapabilityMissing,
    /// The application judged a presented explicit proof invalid.
    CapabilityDenied,
    /// A resource reference did not resolve to a live value (stale lease,
    /// foreign tab, expired session).
    ResourceRefused,
    /// Host request metadata did not satisfy the conversation-identity
    /// contract or contained conflicting trusted observations.
    InvalidRequestContext,
    StdinMismatch,
    WrongEffectLane,
    PermissionRequired,
    PermissionDenied,
    ConfirmationUnavailable,
    ConfirmationCanceled,
    ConfirmationFailed,
    ApprovalInvalid,
    BuildFailed,
    HandlerFailed,
    ApplicationError,
    ResultContractViolation,
    ArgumentContractViolation,
}

impl ErrorCode {
    pub fn from_framework_error(error: &FrameworkError) -> Self {
        match error {
            FrameworkError::EmptyCommand => Self::EmptyCommand,
            FrameworkError::UnterminatedQuote => Self::UnterminatedQuote,
            FrameworkError::ShellSyntax(_) => Self::ShellSyntax,
            FrameworkError::PlaceholderInterpolation(_) => Self::PlaceholderInterpolation,
            FrameworkError::InvalidPlaceholder(_) => Self::InvalidPlaceholder,
            FrameworkError::UnknownCommand { .. } => Self::UnknownCommand,
            FrameworkError::UnknownArgument(_) => Self::UnknownArgument,
            FrameworkError::MissingArgument(_) => Self::MissingArgument,
            FrameworkError::InvalidArgumentType(_, _) => Self::InvalidArgumentType,
            FrameworkError::ArgumentUnionMismatch { .. } => Self::InvalidArgumentType,
            FrameworkError::ArgumentSchemaMismatch { .. } => Self::InvalidArgumentType,
            FrameworkError::WorkspaceMismatch { .. } => Self::WorkspaceMismatch,
            FrameworkError::WorkspaceUnresolved { .. } => Self::UnresolvedWorkspaceRequirement,
            FrameworkError::CapabilityMissing { .. } => Self::CapabilityMissing,
            FrameworkError::CapabilityDenied { .. } => Self::CapabilityDenied,
            FrameworkError::ResourceRefused { .. } => Self::ResourceRefused,
            FrameworkError::InvalidConversationIdentity { .. }
            | FrameworkError::ConflictingConversationIdentity
            | FrameworkError::InvalidWorkspaceMetadata { .. }
            | FrameworkError::ConflictingWorkspaceInputs
            | FrameworkError::InvalidPreResolvedWorkspaceSet { .. } => Self::InvalidRequestContext,
            FrameworkError::StdinMismatch(_) => Self::StdinMismatch,
            FrameworkError::PermissionDenied { .. } => Self::PermissionDenied,
            FrameworkError::ConfirmationUnavailable { .. } => Self::ConfirmationUnavailable,
            FrameworkError::ConfirmationCanceled { .. } => Self::ConfirmationCanceled,
            FrameworkError::ConfirmationFailed { .. } => Self::ConfirmationFailed,
            FrameworkError::ApprovalInvalid(_) => Self::ApprovalInvalid,
            FrameworkError::WrongEffectLane { .. } => Self::WrongEffectLane,
            FrameworkError::Build(_) => Self::BuildFailed,
            FrameworkError::Handler(_) => Self::HandlerFailed,
            FrameworkError::ResultContractViolation { .. } => Self::ResultContractViolation,
            FrameworkError::ArgumentContractViolation { .. } => Self::ArgumentContractViolation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<DiagnosticLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DiagnosticLocation {
    CommandToken { index: usize, value: String },
    Placeholder { name: String },
    Argument { name: String },
    OutputField { name: String },
    ToolName { name: String },
    Workspace { name: String },
    RequestContext { key: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SteeringAction {
    pub kind: SteeringKind,
    pub label: String,
    pub request: Value,
    pub priority: SteeringPriority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SteeringKind {
    Help,
    RetryRun,
    RetryWithTool,
    DryRun,
    RequestPermission,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SteeringPriority {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DisplayHint {
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetryAction {
    pub tool: String,
    pub arguments: RunRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplayEnvelope {
    pub token: String,
    #[serde(rename = "expiresAtUnixMs")]
    pub expires_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPreview {
    pub operation_id: String,
    pub command: Vec<String>,
    pub effect: crate::EffectSpec,
    pub lane: crate::EffectLane,
    pub permissions: Vec<PermissionSpec>,
    pub workspaces: Vec<WorkspaceDecl>,
    /// The workspace roots this invocation actually planned against — the
    /// roots the approval token binds to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_roots: Vec<crate::PlanWorkspaceRoot>,
    /// The matched union variant for each argument bound to a named type.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub argument_variants: BTreeMap<String, crate::ArgVariants>,
    pub output: OutputSpec,
    pub confirmation_policy: ConfirmationPolicy,
    pub requires_confirmation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<crate::PreparedConfirmation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionPreviewWire {
    operation_id: String,
    command: Vec<String>,
    effect: crate::EffectSpec,
    lane: crate::EffectLane,
    permissions: Vec<PermissionSpec>,
    workspaces: Vec<WorkspaceDecl>,
    #[serde(default)]
    workspace_roots: Vec<crate::PlanWorkspaceRoot>,
    #[serde(default)]
    argument_variants: BTreeMap<String, crate::ArgVariants>,
    output: OutputSpec,
    confirmation_policy: ConfirmationPolicy,
    requires_confirmation: bool,
    #[serde(default)]
    confirmation: Option<crate::PreparedConfirmation>,
}

impl TryFrom<PermissionPreviewWire> for PermissionPreview {
    type Error = String;

    fn try_from(wire: PermissionPreviewWire) -> Result<Self, Self::Error> {
        if let Some(confirmation) = &wire.confirmation {
            if !wire.requires_confirmation {
                return Err("permission preview confirmation requiresConfirmation=true".to_string());
            }
            if confirmation.operation_id != wire.operation_id {
                return Err(
                    "permission preview confirmation operationId does not match preview operationId"
                        .to_string(),
                );
            }
        }
        Ok(Self {
            operation_id: wire.operation_id,
            command: wire.command,
            effect: wire.effect,
            lane: wire.lane,
            permissions: wire.permissions,
            workspaces: wire.workspaces,
            workspace_roots: wire.workspace_roots,
            argument_variants: wire.argument_variants,
            output: wire.output,
            confirmation_policy: wire.confirmation_policy,
            requires_confirmation: wire.requires_confirmation,
            confirmation: wire.confirmation,
        })
    }
}

impl<'de> Deserialize<'de> for PermissionPreview {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PermissionPreviewWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    pub status: ResponseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steering: Vec<SteeringAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayHint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<ReplayEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<PermissionPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<InvocationPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryAction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseEnvelopeWire {
    status: ResponseStatus,
    #[serde(default)]
    command: Option<Vec<String>>,
    #[serde(default)]
    output: Option<CommandOutput>,
    #[serde(default)]
    error: Option<ErrorBody>,
    #[serde(default)]
    diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    steering: Vec<SteeringAction>,
    #[serde(default)]
    display: Option<DisplayHint>,
    #[serde(default)]
    replay: Option<ReplayEnvelope>,
    #[serde(default)]
    preview: Option<PermissionPreview>,
    #[serde(default)]
    plan: Option<InvocationPlan>,
    #[serde(default)]
    retry: Option<RetryAction>,
}

impl TryFrom<ResponseEnvelopeWire> for ResponseEnvelope {
    type Error = String;

    fn try_from(wire: ResponseEnvelopeWire) -> Result<Self, Self::Error> {
        if let Some(confirmation) = wire
            .preview
            .as_ref()
            .and_then(|preview| preview.confirmation.as_ref())
        {
            let display = wire.display.as_ref().ok_or_else(|| {
                "response envelope with prepared confirmation requires display".to_string()
            })?;
            if display.title != confirmation.title || display.summary != confirmation.message {
                return Err(
                    "response envelope display does not match prepared confirmation".to_string(),
                );
            }
        }
        Ok(Self {
            status: wire.status,
            command: wire.command,
            output: wire.output,
            error: wire.error,
            diagnostics: wire.diagnostics,
            steering: wire.steering,
            display: wire.display,
            replay: wire.replay,
            preview: wire.preview,
            plan: wire.plan,
            retry: wire.retry,
        })
    }
}

impl<'de> Deserialize<'de> for ResponseEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ResponseEnvelopeWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl ResponseEnvelope {
    pub fn success(response: RunResponse, profile: ResponseProfile) -> Self {
        let include_plan = response.dry_run || matches!(profile, ResponseProfile::Debug);
        let command = Some(response.plan.command_path.clone());
        let summary = if response.dry_run {
            format!(
                "Dry run planned `{}`.",
                response.plan.command_path.join(" ")
            )
        } else {
            format!(
                "Command `{}` completed.",
                response.plan.command_path.join(" ")
            )
        };
        Self {
            status: ResponseStatus::Ok,
            command,
            output: response.output,
            error: None,
            diagnostics: Vec::new(),
            steering: Vec::new(),
            display: Some(DisplayHint {
                title: "Command complete".to_string(),
                summary,
            }),
            replay: None,
            preview: None,
            plan: include_plan.then_some(response.plan),
            retry: None,
        }
    }

    pub fn application_error(
        plan: InvocationPlan,
        error: crate::ApplicationErrorBody,
        profile: ResponseProfile,
    ) -> Self {
        let command = Some(plan.command_path.clone());
        let steering = error
            .recoveries
            .iter()
            .filter_map(|recovery| match recovery {
                crate::ApplicationRecovery::Operation { operation_id } => Some(SteeringAction {
                    kind: SteeringKind::Help,
                    label: format!("Recover with `{operation_id}`"),
                    request: json!({
                        "tool": "help",
                        "arguments": { "command": operation_id },
                    }),
                    priority: SteeringPriority::Primary,
                }),
                crate::ApplicationRecovery::Action { .. } => None,
            })
            .collect();
        let details = json!({
            "applicationCode": error.code,
            "details": error.details,
            "recoveries": error.recoveries,
        });
        Self {
            status: ResponseStatus::Failed,
            command,
            output: None,
            error: Some(ErrorBody {
                code: ErrorCode::ApplicationError,
                message: error.message.clone(),
                details,
            }),
            diagnostics: Vec::new(),
            steering,
            display: Some(DisplayHint {
                title: "Application error".to_string(),
                summary: error.message,
            }),
            replay: None,
            preview: None,
            plan: matches!(profile, ResponseProfile::Debug).then_some(plan),
            retry: None,
        }
    }

    pub fn preview(plan: InvocationPlan, requires_confirmation: bool) -> Self {
        let command = Some(plan.command_path.clone());
        let preview = permission_preview(&plan, requires_confirmation, None);
        Self {
            status: ResponseStatus::Ok,
            command,
            output: None,
            error: None,
            diagnostics: Vec::new(),
            steering: Vec::new(),
            display: Some(DisplayHint {
                title: "Command preview".to_string(),
                summary: format!("Preview ready for `{}`.", plan.command_path.join(" ")),
            }),
            replay: None,
            preview: Some(preview),
            plan: None,
            retry: None,
        }
    }

    pub(crate) fn preview_with_confirmation(
        plan: InvocationPlan,
        confirmation: crate::PreparedConfirmation,
    ) -> Self {
        let command = Some(plan.command_path.clone());
        let display = DisplayHint {
            title: confirmation.title.clone(),
            summary: confirmation.message.clone(),
        };
        let preview = permission_preview(&plan, true, Some(confirmation));
        Self {
            status: ResponseStatus::Ok,
            command,
            output: None,
            error: None,
            diagnostics: Vec::new(),
            steering: Vec::new(),
            display: Some(display),
            replay: None,
            preview: Some(preview),
            plan: None,
            retry: None,
        }
    }

    pub fn permission_required(
        plan: InvocationPlan,
        record: ReplayRecord,
        request: RunRequest,
        tool_name: impl Into<String>,
    ) -> Self {
        let command = Some(plan.command_path.clone());
        let preview = permission_preview(&plan, true, None);
        let tool_name = tool_name.into();
        Self {
            status: ResponseStatus::PermissionRequired,
            command,
            output: None,
            error: Some(ErrorBody {
                code: ErrorCode::PermissionRequired,
                message: "This command requires confirmation.".to_string(),
                details: json!({
                    "operationId": plan.operation_id,
                    "lane": plan.lane,
                    "expiresAtUnixMs": record.expires_at_unix_ms,
                }),
            }),
            diagnostics: Vec::new(),
            steering: vec![SteeringAction {
                kind: SteeringKind::RequestPermission,
                label: "Confirm and replay this invocation".to_string(),
                request: json!({
                    "tool": tool_name,
                    "arguments": with_approval(request, &record),
                }),
                priority: SteeringPriority::Primary,
            }],
            display: Some(DisplayHint {
                title: "Confirmation required".to_string(),
                summary: format!(
                    "Confirmation required for `{}`.",
                    plan.command_path.join(" ")
                ),
            }),
            replay: Some(ReplayEnvelope {
                token: record.token,
                expires_at_unix_ms: record.expires_at_unix_ms,
            }),
            preview: Some(preview),
            plan: None,
            retry: None,
        }
    }

    pub(crate) fn permission_required_with_confirmation(
        plan: InvocationPlan,
        record: ReplayRecord,
        request: RunRequest,
        tool_name: impl Into<String>,
        confirmation: crate::PreparedConfirmation,
    ) -> Self {
        let command = Some(plan.command_path.clone());
        let display = DisplayHint {
            title: confirmation.title.clone(),
            summary: confirmation.message.clone(),
        };
        let preview = permission_preview(&plan, true, Some(confirmation));
        let tool_name = tool_name.into();
        Self {
            status: ResponseStatus::PermissionRequired,
            command,
            output: None,
            error: Some(ErrorBody {
                code: ErrorCode::PermissionRequired,
                message: "This command requires confirmation.".to_string(),
                details: json!({
                    "operationId": plan.operation_id,
                    "lane": plan.lane,
                    "expiresAtUnixMs": record.expires_at_unix_ms,
                }),
            }),
            diagnostics: Vec::new(),
            steering: vec![SteeringAction {
                kind: SteeringKind::RequestPermission,
                label: "Confirm and replay this invocation".to_string(),
                request: json!({
                    "tool": tool_name,
                    "arguments": with_approval(request, &record),
                }),
                priority: SteeringPriority::Primary,
            }],
            display: Some(display),
            replay: Some(ReplayEnvelope {
                token: record.token,
                expires_at_unix_ms: record.expires_at_unix_ms,
            }),
            preview: Some(preview),
            plan: None,
            retry: None,
        }
    }

    pub fn framework_error(
        error: FrameworkError,
        request: Option<RunRequest>,
        plan: Option<InvocationPlan>,
    ) -> Self {
        let error = normalize_public_capability_denial(error);
        let code = ErrorCode::from_framework_error(&error);
        let status = status_for_error(&error);
        let message = match &error {
            FrameworkError::Handler(_) => "Command handler failed".to_string(),
            FrameworkError::ResultContractViolation { .. } => {
                "The declared result contract was violated".to_string()
            }
            FrameworkError::ArgumentContractViolation { .. } => {
                "The declared argument contract was violated".to_string()
            }
            _ => error.to_string(),
        };
        let command = plan.as_ref().map(|plan| plan.command_path.clone());
        let retry = match (&error, request) {
            (FrameworkError::WrongEffectLane { required_tool, .. }, Some(arguments)) => {
                Some(RetryAction {
                    tool: required_tool.clone(),
                    arguments,
                })
            }
            _ => None,
        };
        let mut details = error_details(&error);
        if let (FrameworkError::ResultContractViolation { .. }, Some(plan)) = (&error, &plan)
            && let Some(details) = details.as_object_mut()
        {
            details.insert("operation".to_string(), json!(plan.operation_id));
        }
        let diagnostic = diagnostic_for_error(&error, &code);
        let steering = steering_for_error(&error, retry.as_ref());
        let mut diagnostics = vec![diagnostic];
        diagnostics.extend(workspace_diagnostics(&error));
        let plan = if matches!(
            &error,
            FrameworkError::ResultContractViolation { .. }
                | FrameworkError::ArgumentContractViolation { .. }
        ) {
            None
        } else {
            plan
        };

        Self {
            status,
            command,
            output: None,
            error: Some(ErrorBody {
                code,
                message: message.clone(),
                details,
            }),
            diagnostics,
            steering,
            display: Some(DisplayHint {
                title: "Command failed".to_string(),
                summary: message,
            }),
            replay: None,
            preview: None,
            plan,
            retry,
        }
    }

    pub fn display_text(&self) -> String {
        self.display
            .as_ref()
            .map(|display| display.summary.clone())
            .or_else(|| self.error.as_ref().map(|error| error.message.clone()))
            .unwrap_or_else(|| "Command result".to_string())
    }
}

/// Produces the display-safe compatibility detail promised by RFC 0010.
/// The bound counts rendered Unicode scalars, including escape sequences and
/// the truncation marker, and truncation never splits one encoded input scalar.
fn normalize_public_capability_denial(error: FrameworkError) -> FrameworkError {
    let FrameworkError::CapabilityDenied {
        capability,
        detail,
        carrier,
        providers,
    } = error
    else {
        return error;
    };
    FrameworkError::CapabilityDenied {
        capability,
        detail: encode_capability_denial_detail(&detail),
        carrier,
        providers,
    }
}

fn encode_capability_denial_detail(detail: &str) -> String {
    const LIMIT: usize = 512;

    let mut output = String::with_capacity(LIMIT);
    let mut rendered_width = 0;
    let mut scalars = detail.chars().peekable();
    while let Some(scalar) = scalars.next() {
        let mut unicode_escape = [0_u8; 6];
        let mut utf8_scalar = [0_u8; 4];
        let (chunk, chunk_width) = match scalar {
            '"' => ("\\\"", 2),
            '\\' => ("\\\\", 2),
            '\u{0008}' => ("\\b", 2),
            '\u{000C}' => ("\\f", 2),
            '\n' => ("\\n", 2),
            '\r' => ("\\r", 2),
            '\t' => ("\\t", 2),
            scalar if capability_denial_uses_unicode_escape(scalar) => (
                capability_denial_unicode_escape(scalar, &mut unicode_escape),
                6,
            ),
            scalar => {
                let chunk: &str = scalar.encode_utf8(&mut utf8_scalar);
                (chunk, 1)
            }
        };

        // When more input remains, reserve the final rendered scalar for the
        // truncation marker. The last input scalar may use the full bound.
        let available = if scalars.peek().is_some() {
            LIMIT - 1
        } else {
            LIMIT
        };
        if rendered_width + chunk_width > available {
            output.push('…');
            break;
        }
        output.push_str(chunk);
        rendered_width += chunk_width;
    }
    output
}

fn capability_denial_unicode_escape(scalar: char, buffer: &mut [u8; 6]) -> &str {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    buffer[0] = b'\\';
    buffer[1] = b'u';
    let value = scalar as u32;
    for (index, shift) in [12, 8, 4, 0].into_iter().enumerate() {
        buffer[index + 2] = HEX[((value >> shift) & 0xF) as usize];
    }
    std::str::from_utf8(buffer).expect("Unicode escape buffer is ASCII")
}

fn capability_denial_uses_unicode_escape(scalar: char) -> bool {
    matches!(scalar,
        '\u{0000}'..='\u{0007}'
        | '\u{000B}'
        | '\u{000E}'..='\u{001F}'
        | '\u{007F}'..='\u{009F}'
        | '\u{061C}'
        | '\u{200E}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
    )
}

fn status_for_error(error: &FrameworkError) -> ResponseStatus {
    match error {
        FrameworkError::UnknownCommand { .. } => ResponseStatus::NotFound,
        FrameworkError::PermissionDenied { .. } => ResponseStatus::PermissionDenied,
        FrameworkError::WrongEffectLane { .. } => ResponseStatus::WrongEffectLane,
        FrameworkError::ApprovalInvalid(_) => ResponseStatus::ApprovalInvalid,
        FrameworkError::ConfirmationUnavailable { .. }
        | FrameworkError::ConfirmationCanceled { .. }
        | FrameworkError::ConfirmationFailed { .. }
        | FrameworkError::Handler(_)
        | FrameworkError::Build(_)
        | FrameworkError::ResultContractViolation { .. }
        | FrameworkError::ArgumentContractViolation { .. } => ResponseStatus::Failed,
        _ => ResponseStatus::InvalidInput,
    }
}

/// Projects resolver workspace diagnostics carried by a workspace mismatch
/// or an unresolved workspace requirement into envelope diagnostics with the
/// resolver's stable codes.
fn workspace_diagnostics(error: &FrameworkError) -> Vec<Diagnostic> {
    let diagnostics = match error {
        FrameworkError::WorkspaceMismatch { diagnostics, .. }
        | FrameworkError::WorkspaceUnresolved { diagnostics, .. } => diagnostics,
        _ => return Vec::new(),
    };
    diagnostics
        .iter()
        .map(|diagnostic| {
            let code = match diagnostic.code.as_str() {
                mcp_workspace_resolver::AMBIGUOUS_WORKSPACE_ROOT => {
                    ErrorCode::AmbiguousWorkspaceRoot
                }
                mcp_workspace_resolver::UNSUPPORTED_ROOT_SCHEME => ErrorCode::UnsupportedRootScheme,
                _ => ErrorCode::UnresolvedWorkspaceRequirement,
            };
            Diagnostic {
                code,
                message: diagnostic.message.clone(),
                location: diagnostic.requirement.as_ref().map(|requirement| {
                    DiagnosticLocation::Workspace {
                        name: requirement.as_str().to_string(),
                    }
                }),
                expected: None,
                actual: (!diagnostic.roots.is_empty()).then(|| json!(diagnostic.roots)),
                suggestions: Vec::new(),
            }
        })
        .collect()
}

fn error_details(error: &FrameworkError) -> Value {
    match error {
        FrameworkError::ShellSyntax(value) => json!({ "syntax": value }),
        FrameworkError::PlaceholderInterpolation(value)
        | FrameworkError::InvalidPlaceholder(value)
        | FrameworkError::UnknownArgument(value)
        | FrameworkError::MissingArgument(value) => json!({ "value": value }),
        FrameworkError::UnknownCommand { command, nearest } => {
            json!({ "value": command, "nearest": nearest })
        }
        FrameworkError::InvalidArgumentType(name, expected) => {
            json!({ "argument": name, "expected": expected })
        }
        FrameworkError::ArgumentUnionMismatch {
            argument,
            type_name,
            problems,
        } => json!({
            "argument": argument,
            "typeName": type_name,
            "variantProblems": problems
                .iter()
                .map(|(variant, problem)| json!({ "variant": variant, "problem": problem }))
                .collect::<Vec<_>>(),
        }),
        FrameworkError::ArgumentSchemaMismatch {
            argument,
            path,
            keyword,
            expected,
            branches,
        } => json!({
            "argument": argument,
            "path": path,
            "keyword": keyword,
            "expected": expected,
            "branches": branches,
        }),
        FrameworkError::WorkspaceMismatch {
            argument,
            workspace,
            selected_root,
            path,
            diagnostics,
        } => json!({
            "argument": argument,
            "workspace": workspace,
            "selectedRoot": selected_root,
            "path": path,
            "workspaceDiagnostics": diagnostics,
        }),
        FrameworkError::WorkspaceUnresolved {
            workspace,
            diagnostics,
        } => json!({
            "workspace": workspace,
            "workspaceDiagnostics": diagnostics,
        }),
        FrameworkError::CapabilityMissing {
            capability,
            carrier,
            providers,
        } => json!({
            "capability": capability,
            "carrier": carrier,
            "providers": providers,
        }),
        FrameworkError::CapabilityDenied {
            capability,
            detail,
            carrier,
            providers,
        } => json!({
            "capability": capability,
            "detail": detail,
            "carrier": carrier,
            "providers": providers,
        }),
        FrameworkError::ResourceRefused {
            resource,
            reference,
            detail,
            enumerate,
            establish,
        } => json!({
            "resource": resource,
            "reference": reference,
            "detail": detail,
            "recover": {
                "enumerate": enumerate,
                "establish": establish,
            },
        }),
        FrameworkError::InvalidConversationIdentity {
            observation_source,
            key,
            field,
            reason,
            ..
        } => {
            let mut details = serde_json::Map::new();
            details.insert("source".to_string(), json!(observation_source));
            details.insert("key".to_string(), json!(key));
            if let Some(field) = field {
                details.insert("field".to_string(), json!(field));
            }
            details.insert("reason".to_string(), json!(reason));
            Value::Object(details)
        }
        FrameworkError::ConflictingConversationIdentity => json!({
            "reason": "conflicting_observations",
            "sources": [crate::CONVERSATION_IDENTITY_META_KEY, "threadId"],
        }),
        FrameworkError::InvalidWorkspaceMetadata { key, field, reason } => {
            let mut details = serde_json::Map::new();
            details.insert("key".to_string(), json!(key));
            if let Some(field) = field {
                details.insert("field".to_string(), json!(field));
            }
            details.insert("reason".to_string(), json!(reason));
            Value::Object(details)
        }
        FrameworkError::ConflictingWorkspaceInputs => json!({
            "key": "hostWorkspaceRoots",
            "reason": "conflicting_workspace_inputs",
        }),
        FrameworkError::InvalidPreResolvedWorkspaceSet { workspace, reason } => {
            let mut details = serde_json::Map::new();
            details.insert("key".to_string(), json!("preResolvedWorkspaces"));
            details.insert("reason".to_string(), json!(reason));
            if let Some(workspace) = workspace {
                details.insert("workspace".to_string(), json!(workspace));
            }
            Value::Object(details)
        }
        FrameworkError::StdinMismatch(reason) => json!({ "reason": reason }),
        FrameworkError::PermissionDenied { effect, scope } => {
            json!({ "effect": effect, "scope": scope })
        }
        FrameworkError::ConfirmationUnavailable { operation_id }
        | FrameworkError::ConfirmationCanceled { operation_id }
        | FrameworkError::ConfirmationFailed { operation_id } => {
            json!({ "operationId": operation_id })
        }
        FrameworkError::ApprovalInvalid(value) => json!({ "reason": value }),
        FrameworkError::WrongEffectLane {
            current_tool,
            required_tool,
        } => json!({
            "currentTool": current_tool,
            "requiredTool": required_tool,
        }),
        FrameworkError::Handler(_) => json!({}),
        FrameworkError::Build(value) => json!({ "build": value }),
        FrameworkError::ResultContractViolation { boundary, reason } => json!({
            "boundary": boundary,
            "reason": reason,
        }),
        FrameworkError::ArgumentContractViolation {
            operation_id,
            argument,
            reason,
        } => json!({
            "operation": operation_id,
            "argument": argument,
            "reason": reason,
        }),
        FrameworkError::EmptyCommand | FrameworkError::UnterminatedQuote => json!({}),
    }
}

pub(crate) fn permission_preview(
    plan: &InvocationPlan,
    requires_confirmation: bool,
    confirmation: Option<crate::PreparedConfirmation>,
) -> PermissionPreview {
    let argument_variants = plan
        .bound_args
        .iter()
        .filter_map(|(name, arg)| {
            arg.variants
                .as_ref()
                .map(|variants| (name.clone(), variants.clone()))
        })
        .collect();
    PermissionPreview {
        operation_id: plan.operation_id.clone(),
        command: plan.command_path.clone(),
        effect: plan.effect.clone(),
        lane: plan.lane,
        permissions: plan.permissions.clone(),
        workspaces: plan.workspaces.clone(),
        workspace_roots: plan.workspace_roots.clone(),
        argument_variants,
        output: plan.output.clone(),
        confirmation_policy: ConfirmationPolicy::EffectDefault,
        requires_confirmation,
        confirmation,
    }
}

fn with_approval(mut request: RunRequest, record: &ReplayRecord) -> RunRequest {
    request.approval = Some(crate::ApprovalInput {
        token: record.token.clone(),
        confirm: true,
    });
    request
}

fn diagnostic_for_error(error: &FrameworkError, code: &ErrorCode) -> Diagnostic {
    let location = match error {
        FrameworkError::ShellSyntax(value) => Some(DiagnosticLocation::CommandToken {
            index: 0,
            value: value.clone(),
        }),
        FrameworkError::PlaceholderInterpolation(value)
        | FrameworkError::InvalidPlaceholder(value) => Some(DiagnosticLocation::Placeholder {
            name: value.clone(),
        }),
        FrameworkError::UnknownArgument(value)
        | FrameworkError::MissingArgument(value)
        | FrameworkError::InvalidArgumentType(value, _) => Some(DiagnosticLocation::Argument {
            name: value.clone(),
        }),
        FrameworkError::ArgumentUnionMismatch { argument, .. } => {
            // Repeated arguments carry an element index (`fields[2]`);
            // the location names the argument itself.
            let name = argument
                .split_once('[')
                .map_or(argument.as_str(), |(base, _)| base);
            Some(DiagnosticLocation::Argument {
                name: name.to_string(),
            })
        }
        FrameworkError::ArgumentSchemaMismatch { argument, .. } => {
            Some(DiagnosticLocation::Argument {
                name: argument.clone(),
            })
        }
        FrameworkError::ArgumentContractViolation { argument, .. } => {
            argument
                .as_ref()
                .map(|argument| DiagnosticLocation::Argument {
                    name: argument.clone(),
                })
        }
        FrameworkError::WorkspaceMismatch { workspace, .. }
        | FrameworkError::WorkspaceUnresolved { workspace, .. } => {
            Some(DiagnosticLocation::Workspace {
                name: workspace.clone(),
            })
        }
        FrameworkError::CapabilityMissing { carrier, .. } => Some(DiagnosticLocation::Argument {
            name: carrier.clone(),
        }),
        FrameworkError::CapabilityDenied { carrier, .. } => {
            carrier
                .as_ref()
                .map(|carrier| DiagnosticLocation::Argument {
                    name: carrier.clone(),
                })
        }
        FrameworkError::WrongEffectLane { current_tool, .. } => {
            Some(DiagnosticLocation::ToolName {
                name: current_tool.clone(),
            })
        }
        FrameworkError::InvalidConversationIdentity { key, .. } => {
            Some(DiagnosticLocation::RequestContext { key: key.clone() })
        }
        FrameworkError::ConflictingConversationIdentity => {
            Some(DiagnosticLocation::RequestContext {
                key: crate::CONVERSATION_IDENTITY_META_KEY.to_string(),
            })
        }
        FrameworkError::InvalidWorkspaceMetadata { key, .. } => {
            Some(DiagnosticLocation::RequestContext { key: key.clone() })
        }
        FrameworkError::ConflictingWorkspaceInputs => Some(DiagnosticLocation::RequestContext {
            key: "hostWorkspaceRoots".to_string(),
        }),
        FrameworkError::InvalidPreResolvedWorkspaceSet { .. } => {
            Some(DiagnosticLocation::RequestContext {
                key: "preResolvedWorkspaces".to_string(),
            })
        }
        _ => None,
    };

    let expected = match error {
        FrameworkError::InvalidArgumentType(_, expected) => Some(json!(expected)),
        FrameworkError::ArgumentUnionMismatch { type_name, .. } => Some(json!(type_name)),
        FrameworkError::ArgumentSchemaMismatch { expected, .. } => Some(json!(expected)),
        FrameworkError::InvalidConversationIdentity { expected, .. } => {
            expected.as_ref().map(|expected| json!(expected))
        }
        _ => None,
    };

    Diagnostic {
        code: code.clone(),
        message: match error {
            FrameworkError::Handler(_) => "Command handler failed".to_string(),
            FrameworkError::ResultContractViolation { .. } => {
                "The declared result contract was violated".to_string()
            }
            FrameworkError::ArgumentContractViolation { .. } => {
                "The declared argument contract was violated".to_string()
            }
            _ => error.to_string(),
        },
        location,
        expected,
        actual: None,
        suggestions: suggestions_for_error(error),
    }
}

fn suggestions_for_error(error: &FrameworkError) -> Vec<Suggestion> {
    match error {
        FrameworkError::ShellSyntax(_) => vec![Suggestion {
            message:
                "Use typed args and output controls instead of shell syntax in the command string."
                    .to_string(),
            replacement: None,
        }],
        FrameworkError::UnknownCommand { nearest, .. } if !nearest.is_empty() => nearest
            .iter()
            .map(|candidate| Suggestion {
                message: format!("Did you mean `{candidate}`?"),
                replacement: Some(Value::String(candidate.clone())),
            })
            .collect(),
        FrameworkError::MissingArgument(_) | FrameworkError::UnknownArgument(_) => {
            vec![Suggestion {
                message: "Call help for this command to inspect accepted arguments.".to_string(),
                replacement: None,
            }]
        }
        FrameworkError::WrongEffectLane { required_tool, .. } => vec![Suggestion {
            message: format!("Retry the same request with `{required_tool}`."),
            replacement: None,
        }],
        _ => Vec::new(),
    }
}

fn steering_for_error(error: &FrameworkError, retry: Option<&RetryAction>) -> Vec<SteeringAction> {
    if let (FrameworkError::WrongEffectLane { .. }, Some(retry)) = (error, retry) {
        return vec![SteeringAction {
            kind: SteeringKind::RetryWithTool,
            label: format!("Retry with {}", retry.tool),
            request: json!({
                "tool": retry.tool,
                "arguments": retry.arguments,
            }),
            priority: SteeringPriority::Primary,
        }];
    }

    match error {
        FrameworkError::UnknownCommand { .. }
        | FrameworkError::MissingArgument(_)
        | FrameworkError::UnknownArgument(_)
        | FrameworkError::ShellSyntax(_) => vec![SteeringAction {
            kind: SteeringKind::Help,
            label: "Inspect command help".to_string(),
            request: json!({ "tool": "help", "arguments": {} }),
            priority: SteeringPriority::Primary,
        }],
        FrameworkError::CapabilityMissing {
            capability,
            providers,
            ..
        }
        | FrameworkError::CapabilityDenied {
            capability,
            providers,
            ..
        } => capability_steering(capability, providers),
        _ => Vec::new(),
    }
}

/// One steering action per establishing command, derived from `provides`
/// declarations, so establishment guidance can never drift from the
/// declarations.
fn capability_steering(capability: &str, providers: &[String]) -> Vec<SteeringAction> {
    providers
        .iter()
        .map(|provider| SteeringAction {
            kind: SteeringKind::Help,
            label: format!("Establish `{capability}` with `{provider}`"),
            request: json!({
                "tool": "help",
                "arguments": { "command": provider },
            }),
            priority: SteeringPriority::Primary,
        })
        .collect()
}
