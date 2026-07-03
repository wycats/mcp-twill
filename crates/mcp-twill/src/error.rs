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
