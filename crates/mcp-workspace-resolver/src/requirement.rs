//! Workspace requirements and declared roots.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable identifier for a workspace requirement, such as `repo`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for WorkspaceId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for WorkspaceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for WorkspaceId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for WorkspaceId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Access capabilities for a workspace root. Defaults to read/write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCapabilities {
    pub read: bool,
    pub write: bool,
}

impl WorkspaceCapabilities {
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
        }
    }

    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
        }
    }
}

impl Default for WorkspaceCapabilities {
    fn default() -> Self {
        Self::read_write()
    }
}

/// How a requirement selects one root from a set of runtime roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSelection {
    /// Select the MCP root whose name equals the requirement id or one of its
    /// aliases. Matching is case-sensitive on every platform.
    ByNameOrAlias,
    /// Like [`ByNameOrAlias`](Self::ByNameOrAlias), but when the client
    /// supplies exactly one root and no name matches, that single root
    /// satisfies the requirement.
    PrimaryWhenSingleRoot,
    /// Select the root whose URI is path-equivalent to the configured URI.
    ExplicitUri { uri: String },
}

/// A root supplied by the server author or deployment configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredWorkspaceRoot {
    pub id: WorkspaceId,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: WorkspaceCapabilities,
}

impl DeclaredWorkspaceRoot {
    pub fn new(id: impl Into<WorkspaceId>, uri: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            uri: uri.into(),
            display_name: None,
            capabilities: WorkspaceCapabilities::default(),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_capabilities(mut self, capabilities: WorkspaceCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
}

/// A workspace the server needs resolved before dispatching path arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRequirement {
    pub id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub selection: WorkspaceSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<DeclaredWorkspaceRoot>,
}

impl WorkspaceRequirement {
    /// Creates a requirement with the default [`WorkspaceSelection::ByNameOrAlias`] policy.
    pub fn new(id: impl Into<WorkspaceId>) -> Self {
        Self {
            id: id.into(),
            display_name: None,
            aliases: Vec::new(),
            selection: WorkspaceSelection::ByNameOrAlias,
            fallback: None,
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn with_selection(mut self, selection: WorkspaceSelection) -> Self {
        self.selection = selection;
        self
    }

    pub fn with_fallback(mut self, fallback: DeclaredWorkspaceRoot) -> Self {
        self.fallback = Some(fallback);
        self
    }
}
