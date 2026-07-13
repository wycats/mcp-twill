//! Workspace observations supplied by the host environment.
//!
//! [`WorkspaceObservationSet`] keeps its fields private so later observation
//! sources can be added without breaking downstream construction. Build one
//! with [`Default`] plus the `with_*` methods.

use crate::{normalize_file_uri, requirement::DeclaredWorkspaceRoot};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, path::PathBuf};
use thiserror::Error;

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

/// A validated workspace root supplied by a trusted embedding.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostWorkspaceRoot {
    issuer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    uri: String,
}

impl HostWorkspaceRoot {
    pub fn new(
        issuer: impl Into<String>,
        uri: impl Into<String>,
    ) -> Result<Self, HostWorkspaceRootError> {
        Self::build(issuer.into(), None, uri.into())
    }

    pub fn named(
        issuer: impl Into<String>,
        name: impl Into<String>,
        uri: impl Into<String>,
    ) -> Result<Self, HostWorkspaceRootError> {
        Self::build(issuer.into(), Some(name.into()), uri.into())
    }

    fn build(
        issuer: String,
        name: Option<String>,
        uri: String,
    ) -> Result<Self, HostWorkspaceRootError> {
        if !valid_issuer(&issuer) {
            return Err(HostWorkspaceRootError::InvalidIssuer);
        }
        if name.as_deref().is_some_and(str::is_empty) {
            return Err(HostWorkspaceRootError::InvalidName);
        }
        let normalized =
            normalize_file_uri(&uri).map_err(|_| HostWorkspaceRootError::InvalidUri)?;
        if !uri
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
            || !normalized.is_absolute()
        {
            return Err(HostWorkspaceRootError::InvalidUri);
        }
        Ok(Self { issuer, name, uri })
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }
}

impl fmt::Debug for HostWorkspaceRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("HostWorkspaceRoot")
            .field(&"<redacted>")
            .finish()
    }
}

impl<'de> Deserialize<'de> for HostWorkspaceRoot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            issuer: String,
            #[serde(default)]
            name: Option<String>,
            uri: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::build(wire.issuer, wire.name, wire.uri).map_err(de::Error::custom)
    }
}

/// Validation failures for trusted host workspace roots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HostWorkspaceRootError {
    #[error("issuer must be a lowercase reverse-DNS name")]
    InvalidIssuer,
    #[error("workspace root name must be non-empty")]
    InvalidName,
    #[error("workspace root URI must be an absolute file URI")]
    InvalidUri,
}

/// A presence-preserving trusted-host root observation.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct HostWorkspaceRootsObservation(Vec<HostWorkspaceRoot>);

impl HostWorkspaceRootsObservation {
    pub fn new(roots: impl IntoIterator<Item = HostWorkspaceRoot>) -> Self {
        Self(roots.into_iter().collect())
    }

    pub fn roots(&self) -> &[HostWorkspaceRoot] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for HostWorkspaceRootsObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("HostWorkspaceRootsObservation")
            .field(&"<redacted>")
            .finish()
    }
}

impl<'de> Deserialize<'de> for HostWorkspaceRootsObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<HostWorkspaceRoot>::deserialize(deserializer).map(Self)
    }
}

fn valid_issuer(issuer: &str) -> bool {
    let labels = issuer.split('.').collect::<Vec<_>>();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}

/// The workspace facts the runtime provided for a resolution pass.
///
/// Fields are private by design: construct with [`Default`] and the `with_*`
/// builders so new observation sources are not breaking changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceObservationSet {
    mcp_roots: Option<McpRootsObservation>,
    codex_sandbox: Option<CodexSandboxObservation>,
    host_roots: Option<HostWorkspaceRootsObservation>,
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

    pub fn with_host_roots(mut self, roots: HostWorkspaceRootsObservation) -> Self {
        self.host_roots = Some(roots);
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

    pub fn host_roots(&self) -> Option<&HostWorkspaceRootsObservation> {
        self.host_roots.as_ref()
    }

    pub fn declared(&self) -> &[DeclaredWorkspaceRoot] {
        &self.declared
    }
}
