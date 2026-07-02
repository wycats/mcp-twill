//! Resolve named workspace roots from the workspace information an MCP
//! server can observe.
//!
//! A server describes the workspaces its commands need as
//! [`WorkspaceRequirement`]s. The runtime supplies a
//! [`WorkspaceObservationSet`] built from MCP roots, Codex sandbox metadata,
//! and server-declared roots. [`resolve_workspaces`] applies a deterministic
//! policy and returns a [`ResolvedWorkspaceSet`] with the selected root for
//! each requirement plus structured [`WorkspaceDiagnostic`]s.
//!
//! Observations apply in authority order: MCP roots, then Codex sandbox
//! metadata, then declared roots. Presence blocks fall-through — a present
//! higher-authority observation (even an empty MCP roots list) prevents
//! lower-authority sources from participating, so server configuration can
//! never widen filesystem access beyond the boundaries a client declared.

pub mod codex;
pub mod diagnostics;
pub mod observation;
pub mod path;
pub mod requirement;
pub mod resolve;
#[cfg(feature = "rmcp")]
pub mod rmcp;

pub use codex::{DerivedRoot, DerivedRootKind, RootDerivationPolicy, derive_root};
pub use diagnostics::{
    AMBIGUOUS_WORKSPACE_ROOT, UNRESOLVED_WORKSPACE_REQUIREMENT, UNSUPPORTED_ROOT_SCHEME,
    WorkspaceDiagnostic,
};
pub use observation::{
    CodexSandboxObservation, McpRoot, McpRootsObservation, WorkspaceObservationSet,
};
pub use path::{
    NormalizedPath, UnsupportedRootScheme, normalize_file_uri, normalize_path, path_has_prefix,
    paths_equal,
};
pub use requirement::{
    DeclaredWorkspaceRoot, WorkspaceCapabilities, WorkspaceId, WorkspaceRequirement,
    WorkspaceSelection,
};
pub use resolve::{
    ResolvedWorkspaceRoot, ResolvedWorkspaceSet, WorkspaceSelectionReason, WorkspaceSource,
    resolve_workspaces, resolve_workspaces_with_policy,
};
