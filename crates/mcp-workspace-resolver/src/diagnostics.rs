//! Structured diagnostics for workspace resolution.

use crate::requirement::WorkspaceId;
use serde::{Deserialize, Serialize};

/// Stable diagnostic code: no root matched the requirement.
pub const UNRESOLVED_WORKSPACE_REQUIREMENT: &str = "unresolved_workspace_requirement";
/// Stable diagnostic code: multiple roots matched the requirement.
pub const AMBIGUOUS_WORKSPACE_ROOT: &str = "ambiguous_workspace_root";
/// Stable diagnostic code: a root URI used a scheme other than `file:`.
pub const UNSUPPORTED_ROOT_SCHEME: &str = "unsupported_root_scheme";

/// A structured diagnostic explaining why resolution failed or degraded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDiagnostic {
    /// Stable machine-readable code, such as `unresolved_workspace_requirement`.
    pub code: String,
    /// The requirement this diagnostic concerns, when one applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<WorkspaceId>,
    /// Human-readable explanation.
    pub message: String,
    /// Root URIs involved in the diagnostic (candidates, ambiguous matches,
    /// or the offending non-file URI).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
}

impl WorkspaceDiagnostic {
    pub fn unresolved(requirement: WorkspaceId, message: impl Into<String>) -> Self {
        Self {
            code: UNRESOLVED_WORKSPACE_REQUIREMENT.to_string(),
            requirement: Some(requirement),
            message: message.into(),
            roots: Vec::new(),
        }
    }

    pub fn with_roots(mut self, roots: Vec<String>) -> Self {
        self.roots = roots;
        self
    }

    pub fn ambiguous(
        requirement: WorkspaceId,
        message: impl Into<String>,
        roots: Vec<String>,
    ) -> Self {
        Self {
            code: AMBIGUOUS_WORKSPACE_ROOT.to_string(),
            requirement: Some(requirement),
            message: message.into(),
            roots,
        }
    }

    pub fn unsupported_scheme(
        requirement: Option<WorkspaceId>,
        message: impl Into<String>,
        uri: impl Into<String>,
    ) -> Self {
        Self {
            code: UNSUPPORTED_ROOT_SCHEME.to_string(),
            requirement,
            message: message.into(),
            roots: vec![uri.into()],
        }
    }
}
