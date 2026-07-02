//! Codex sandbox metadata root derivation.
//!
//! Derivation is the one place the resolver touches the real filesystem: it
//! canonicalizes `sandbox_cwd` and walks upward looking for boundary markers.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How a Codex `sandbox_cwd` becomes a workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootDerivationPolicy {
    /// Walk upward from `sandbox_cwd`: the nearest version-control boundary
    /// wins; otherwise the nearest project marker; otherwise `sandbox_cwd`
    /// itself.
    ProjectBoundary {
        vcs_markers: Vec<String>,
        project_markers: Vec<String>,
    },
    /// Use `sandbox_cwd` as the root without walking.
    ExactDirectory,
}

impl RootDerivationPolicy {
    /// The default policy: `.git`/`.jj`/`.hg` boundaries first, then
    /// `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`.
    pub fn project_boundary() -> Self {
        Self::ProjectBoundary {
            vcs_markers: vec![".git".into(), ".jj".into(), ".hg".into()],
            project_markers: vec![
                "Cargo.toml".into(),
                "package.json".into(),
                "pyproject.toml".into(),
                "go.mod".into(),
            ],
        }
    }
}

impl Default for RootDerivationPolicy {
    fn default() -> Self {
        Self::project_boundary()
    }
}

/// How the derived root was chosen, for diagnostics and dry-run output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedRootKind {
    /// A version-control marker was found at the derived directory.
    VcsBoundary { marker: String },
    /// A project marker file was found at the derived directory.
    ProjectMarker { marker: String },
    /// No marker was visible; the sandbox directory itself is the root.
    SandboxCwd,
}

/// The result of deriving a workspace root from Codex sandbox metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedRoot {
    pub path: PathBuf,
    pub kind: DerivedRootKind,
}

/// Derives a workspace root from `sandbox_cwd` under `policy`.
///
/// Canonicalizes `sandbox_cwd` when possible (falling back to the given path
/// if canonicalization fails), then walks upward. The nearest directory
/// containing a VCS marker wins; if none is visible, the nearest directory
/// containing a project marker; otherwise `sandbox_cwd` itself.
pub fn derive_root(sandbox_cwd: &Path, policy: &RootDerivationPolicy) -> DerivedRoot {
    let cwd = sandbox_cwd
        .canonicalize()
        .unwrap_or_else(|_| sandbox_cwd.to_path_buf());

    let (vcs_markers, project_markers) = match policy {
        RootDerivationPolicy::ExactDirectory => {
            return DerivedRoot {
                path: cwd,
                kind: DerivedRootKind::SandboxCwd,
            };
        }
        RootDerivationPolicy::ProjectBoundary {
            vcs_markers,
            project_markers,
        } => (vcs_markers, project_markers),
    };

    if let Some((dir, marker)) = find_nearest_marker(&cwd, vcs_markers) {
        return DerivedRoot {
            path: dir,
            kind: DerivedRootKind::VcsBoundary { marker },
        };
    }

    if let Some((dir, marker)) = find_nearest_marker(&cwd, project_markers) {
        return DerivedRoot {
            path: dir,
            kind: DerivedRootKind::ProjectMarker { marker },
        };
    }

    DerivedRoot {
        path: cwd,
        kind: DerivedRootKind::SandboxCwd,
    }
}

fn find_nearest_marker(start: &Path, markers: &[String]) -> Option<(PathBuf, String)> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        for marker in markers {
            if current.join(marker).exists() {
                return Some((current.to_path_buf(), marker.clone()));
            }
        }
        dir = current.parent();
    }
    None
}
