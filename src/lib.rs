pub mod error;
pub mod help;
pub mod model;
pub mod registry;
pub mod rmcp_adapter;
pub mod template;

pub use error::{FrameworkError, Result};
pub use help::{HelpDetail, HelpRequest, HelpResult, HelpTopic};
pub use model::*;
pub use registry::{CommandHandler, CommandRegistry, HandlerFuture};
pub use rmcp_adapter::CliMcpServer;
pub use template::{CommandTemplate, TemplateToken};
