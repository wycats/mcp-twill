use thiserror::Error;

pub type Result<T> = std::result::Result<T, FrameworkError>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FrameworkError {
    #[error("empty command")]
    EmptyCommand,
    #[error("unterminated quote in command template")]
    UnterminatedQuote,
    #[error("shell syntax is not allowed in command templates: {0}")]
    ShellSyntax(String),
    #[error("placeholder `{0}` must occupy a complete argv token")]
    PlaceholderInterpolation(String),
    #[error("placeholder `{0}` is not an `$args.<name>` placeholder")]
    InvalidPlaceholder(String),
    #[error("unknown command `{command}`")]
    UnknownCommand {
        command: String,
        nearest: Vec<String>,
    },
    #[error("unknown argument `{0}`")]
    UnknownArgument(String),
    #[error("missing argument `{0}`")]
    MissingArgument(String),
    #[error("argument `{0}` must be {1}")]
    InvalidArgumentType(String, &'static str),
    #[error("{}", union_mismatch_message(argument, type_name, problems))]
    ArgumentUnionMismatch {
        argument: String,
        type_name: String,
        /// Every declared variant in declaration order, paired with its
        /// first blocking problem.
        problems: Vec<(String, String)>,
    },
    #[error(
        "{}",
        workspace_mismatch_message(argument, workspace, selected_root, diagnostics)
    )]
    WorkspaceMismatch {
        argument: String,
        workspace: String,
        /// The root URI selected for the workspace, when resolution succeeded
        /// and the failure was a boundary check.
        selected_root: Option<String>,
        /// The offending input path value, when one was supplied.
        path: Option<String>,
        /// Resolver diagnostics explaining why the workspace failed to
        /// resolve, when resolution (rather than containment) failed.
        diagnostics: Vec<mcp_workspace_resolver::WorkspaceDiagnostic>,
    },
    #[error("{}", workspace_unresolved_message(workspace, diagnostics))]
    WorkspaceUnresolved {
        /// The workspace the command declared with `uses_workspace`.
        workspace: String,
        /// Resolver diagnostics explaining why resolution failed.
        diagnostics: Vec<mcp_workspace_resolver::WorkspaceDiagnostic>,
    },
    #[error("stdin mismatch: {0}")]
    StdinMismatch(String),
    #[error("permission denied for `{effect}` on `{scope}`")]
    PermissionDenied { effect: String, scope: String },
    #[error("approval invalid: {0}")]
    ApprovalInvalid(String),
    #[error("This command requires {required_tool}.")]
    WrongEffectLane {
        current_tool: String,
        required_tool: String,
    },
    #[error("registry build failed: {0}")]
    Build(String),
    #[error("command handler failed: {0}")]
    Handler(String),
}

/// Every declared variant appears with its first blocking problem, in
/// declaration order, so an agent sees exactly which fields would make its
/// value match.
fn union_mismatch_message(
    argument: &str,
    type_name: &str,
    problems: &[(String, String)],
) -> String {
    let mut message = format!("argument `{argument}` does not match `{type_name}`:");
    for (variant, problem) in problems {
        message.push_str(&format!("\n  not `{variant}`: {problem}"));
    }
    message
}

/// A boundary failure and a resolution failure are different causes and get
/// different top-level messages; the structured diagnostics carry the detail.
fn workspace_mismatch_message(
    argument: &str,
    workspace: &str,
    selected_root: &Option<String>,
    diagnostics: &[mcp_workspace_resolver::WorkspaceDiagnostic],
) -> String {
    match selected_root {
        Some(root) => {
            format!("path argument `{argument}` is outside workspace `{workspace}` root `{root}`")
        }
        None if !diagnostics.is_empty() => format!(
            "workspace `{workspace}` for path argument `{argument}` could not be resolved: {}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ),
        None => {
            format!("workspace `{workspace}` for path argument `{argument}` could not be resolved")
        }
    }
}

/// The command declared the workspace itself, so the failure is located at
/// the command, not at any argument.
fn workspace_unresolved_message(
    workspace: &str,
    diagnostics: &[mcp_workspace_resolver::WorkspaceDiagnostic],
) -> String {
    if diagnostics.is_empty() {
        format!("workspace `{workspace}` required by this command could not be resolved")
    } else {
        format!(
            "workspace `{workspace}` required by this command could not be resolved: {}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}
