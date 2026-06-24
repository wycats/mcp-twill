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
            FrameworkError::UnknownCommand(_) => Self::UnknownCommand,
            FrameworkError::UnknownArgument(_) => Self::UnknownArgument,
            FrameworkError::MissingArgument(_) => Self::MissingArgument,
            FrameworkError::InvalidArgumentType(_, _) => Self::InvalidArgumentType,
            FrameworkError::WorkspaceMismatch { .. } => Self::WorkspaceMismatch,
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

        Self {
            status,
            command,
            output: None,
            error: Some(ErrorBody {
                code,
                message: message.clone(),
                details,
            }),
            diagnostics: vec![diagnostic],
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

fn status_for_error(error: &FrameworkError) -> ResponseStatus {
    match error {
        FrameworkError::UnknownCommand(_) => ResponseStatus::NotFound,
        FrameworkError::PermissionDenied { .. } => ResponseStatus::PermissionDenied,
        FrameworkError::WrongEffectLane { .. } => ResponseStatus::WrongEffectLane,
        FrameworkError::ApprovalInvalid(_) => ResponseStatus::ApprovalInvalid,
        FrameworkError::Handler(_) => ResponseStatus::Failed,
        FrameworkError::Build(_) => ResponseStatus::Failed,
        _ => ResponseStatus::InvalidInput,
    }
}

fn error_details(error: &FrameworkError) -> Value {
    match error {
        FrameworkError::ShellSyntax(value) => json!({ "syntax": value }),
        FrameworkError::PlaceholderInterpolation(value)
        | FrameworkError::InvalidPlaceholder(value)
        | FrameworkError::UnknownCommand(value)
        | FrameworkError::UnknownArgument(value)
        | FrameworkError::MissingArgument(value) => json!({ "value": value }),
        FrameworkError::InvalidArgumentType(name, expected) => {
            json!({ "argument": name, "expected": expected })
        }
        FrameworkError::WorkspaceMismatch {
            argument,
            workspace,
        } => json!({ "argument": argument, "workspace": workspace }),
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
    PermissionPreview {
        operation_id: plan.operation_id.clone(),
        command: plan.command_path.clone(),
        effect: plan.effect.clone(),
        lane: plan.lane,
        permissions: plan.permissions.clone(),
        workspaces: plan.workspaces.clone(),
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
        FrameworkError::WorkspaceMismatch { workspace, .. } => {
            Some(DiagnosticLocation::Workspace {
                name: workspace.clone(),
            })
        }
        FrameworkError::WrongEffectLane { current_tool, .. } => {
            Some(DiagnosticLocation::ToolName {
                name: current_tool.clone(),
            })
        }
        _ => None,
    };

    let expected = match error {
        FrameworkError::InvalidArgumentType(_, expected) => Some(json!(expected)),
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
        FrameworkError::UnknownCommand(_)
        | FrameworkError::MissingArgument(_)
        | FrameworkError::UnknownArgument(_)
        | FrameworkError::ShellSyntax(_) => vec![SteeringAction {
            kind: SteeringKind::Help,
            label: "Inspect command help".to_string(),
            request: json!({ "tool": "help", "arguments": {} }),
            priority: SteeringPriority::Primary,
        }],
        _ => Vec::new(),
    }
}
