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
    ApprovalInvalid,
    BuildFailed,
    HandlerFailed,
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
            FrameworkError::ApprovalInvalid(_) => Self::ApprovalInvalid,
            FrameworkError::WrongEffectLane { .. } => Self::WrongEffectLane,
            FrameworkError::Build(_) => Self::BuildFailed,
            FrameworkError::Handler(_) => Self::HandlerFailed,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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

    pub fn preview(plan: InvocationPlan, requires_confirmation: bool) -> Self {
        let command = Some(plan.command_path.clone());
        let preview = permission_preview(&plan, requires_confirmation);
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

    pub fn permission_required(
        plan: InvocationPlan,
        record: ReplayRecord,
        request: RunRequest,
        tool_name: impl Into<String>,
    ) -> Self {
        let command = Some(plan.command_path.clone());
        let preview = permission_preview(&plan, true);
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

    pub fn framework_error(
        error: FrameworkError,
        request: Option<RunRequest>,
        plan: Option<InvocationPlan>,
    ) -> Self {
        let error = normalize_public_capability_denial(error);
        let code = ErrorCode::from_framework_error(&error);
        let status = status_for_error(&error);
        let message = error.to_string();
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
        let details = error_details(&error);
        let diagnostic = diagnostic_for_error(&error, &code);
        let steering = steering_for_error(&error, retry.as_ref());
        let mut diagnostics = vec![diagnostic];
        diagnostics.extend(workspace_diagnostics(&error));

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

    let mut chunks = Vec::new();
    let mut width = 0;
    let mut truncated = false;
    for scalar in detail.chars() {
        let chunk = match scalar {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\u{0008}' => "\\b".to_string(),
            '\u{000C}' => "\\f".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            scalar if capability_denial_uses_unicode_escape(scalar) => {
                format!("\\u{:04X}", scalar as u32)
            }
            scalar => scalar.to_string(),
        };
        let chunk_width = chunk.chars().count();
        if width + chunk_width > LIMIT {
            truncated = true;
            break;
        }
        width += chunk_width;
        chunks.push(chunk);
    }

    if truncated {
        while width > LIMIT - 1 {
            if let Some(chunk) = chunks.pop() {
                width -= chunk.chars().count();
            } else {
                break;
            }
        }
        chunks.push("…".to_string());
    }
    chunks.concat()
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
        FrameworkError::Handler(_) => ResponseStatus::Failed,
        FrameworkError::Build(_) => ResponseStatus::Failed,
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
        FrameworkError::ApprovalInvalid(value) => json!({ "reason": value }),
        FrameworkError::WrongEffectLane {
            current_tool,
            required_tool,
        } => json!({
            "currentTool": current_tool,
            "requiredTool": required_tool,
        }),
        FrameworkError::Handler(value) => json!({ "handler": value }),
        FrameworkError::Build(value) => json!({ "build": value }),
        FrameworkError::EmptyCommand | FrameworkError::UnterminatedQuote => json!({}),
    }
}

fn permission_preview(plan: &InvocationPlan, requires_confirmation: bool) -> PermissionPreview {
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
        FrameworkError::InvalidConversationIdentity { expected, .. } => {
            expected.as_ref().map(|expected| json!(expected))
        }
        _ => None,
    };

    Diagnostic {
        code: code.clone(),
        message: error.to_string(),
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
