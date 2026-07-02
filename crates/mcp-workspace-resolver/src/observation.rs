//! Workspace observations supplied by the host environment.
//!
//! [`WorkspaceObservationSet`] keeps its fields private so later observation
//! sources can be added without breaking downstream construction. Build one
//! with [`Default`] plus the `with_*` methods.

use crate::requirement::DeclaredWorkspaceRoot;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single root from an MCP client's `roots/list` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRoot {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl McpRoot {
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            name: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// The root set returned by an MCP client's `roots/list` request.
///
/// An empty list is still an authoritative observation: the client declared
/// that no roots exist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRootsObservation {
    pub roots: Vec<McpRoot>,
}

impl McpRootsObservation {
    pub fn new(roots: Vec<McpRoot>) -> Self {
        Self { roots }
    }
}

/// Codex `codex/sandbox-state-meta` request metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexSandboxObservation {
    pub sandbox_cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile: Option<String>,
}

impl CodexSandboxObservation {
    pub fn new(sandbox_cwd: impl Into<PathBuf>) -> Self {
        Self {
            sandbox_cwd: sandbox_cwd.into(),
            permission_profile: None,
        }
    }

    pub fn with_permission_profile(mut self, profile: impl Into<String>) -> Self {
        self.permission_profile = Some(profile.into());
        self
    }
}

/// The workspace facts the runtime provided for a resolution pass.
///
/// Fields are private by design: construct with [`Default`] and the `with_*`
/// builders so new observation sources are not breaking changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceObservationSet {
    mcp_roots: Option<McpRootsObservation>,
    codex_sandbox: Option<CodexSandboxObservation>,
    declared: Vec<DeclaredWorkspaceRoot>,
}

impl WorkspaceObservationSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mcp_roots(mut self, observation: McpRootsObservation) -> Self {
        self.mcp_roots = Some(observation);
        self
    }

    pub fn with_codex_sandbox(mut self, observation: CodexSandboxObservation) -> Self {
        self.codex_sandbox = Some(observation);
        self
    }

    pub fn with_declared(mut self, root: DeclaredWorkspaceRoot) -> Self {
        self.declared.push(root);
        self
    }

    pub fn mcp_roots(&self) -> Option<&McpRootsObservation> {
        self.mcp_roots.as_ref()
    }

    pub fn codex_sandbox(&self) -> Option<&CodexSandboxObservation> {
        self.codex_sandbox.as_ref()
    }

    pub fn declared(&self) -> &[DeclaredWorkspaceRoot] {
        &self.declared
    }
}
