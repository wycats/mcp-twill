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
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("unknown argument `{0}`")]
    UnknownArgument(String),
    #[error("missing argument `{0}`")]
    MissingArgument(String),
    #[error("argument `{0}` must be {1}")]
    InvalidArgumentType(String, &'static str),
    #[error("path argument `{argument}` is outside declared workspace `{workspace}`")]
    WorkspaceMismatch { argument: String, workspace: String },
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
