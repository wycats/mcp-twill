use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::CommandRegistry;

/// Identifies the running server instance and the command contract it is
/// currently serving. Fields a bare `CommandRegistry` cannot know (process
/// id, start time, executable hash) stay `None` until a runtime host fills
/// them in; the framework must not require a host to construct one.
///
/// Replacement status is deliberately absent: nothing can populate it
/// without a runtime host, so the field arrives with the host crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeIdentity {
    pub server_name: String,
    /// The version of the serving crate. When set through
    /// [`CliMcpServer::runtime_identity`](crate::CliMcpServer::runtime_identity)
    /// this is mcp-twill's own version, not the downstream server's; servers
    /// that version their own contract should set this themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    pub catalog_hash: String,
    pub run_schema_hash: String,
    pub help_schema_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_unix_ms: Option<i64>,
}

impl RuntimeIdentity {
    /// Builds the identity a bare registry can report: name and the catalog
    /// and schema hashes. Process facts stay `None` without a runtime host.
    pub fn for_registry(registry: &CommandRegistry) -> Self {
        let identity = registry.catalog_identity();
        Self {
            server_name: registry.server_name().to_string(),
            server_version: None,
            catalog_hash: identity.catalog_hash,
            run_schema_hash: identity.run_schema_hash,
            help_schema_hash: identity.help_schema_hash,
            executable_hash: None,
            process_id: None,
            started_at_unix_ms: None,
        }
    }

    pub fn with_server_version(mut self, version: impl Into<String>) -> Self {
        self.server_version = Some(version.into());
        self
    }
}
