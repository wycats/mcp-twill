//! Deterministic workspace resolution.
//!
//! Observations are processed in authority order: MCP roots, then Codex
//! sandbox metadata, then declared roots. Presence blocks fall-through: a
//! present higher-authority observation (even an empty MCP roots list) means
//! lower-authority sources do not participate.

use crate::codex::{DerivedRoot, DerivedRootKind, RootDerivationPolicy, derive_root};
use crate::diagnostics::WorkspaceDiagnostic;
use crate::observation::{McpRoot, WorkspaceObservationSet};
use crate::path::{NormalizedPath, normalize_file_uri, paths_equal};
use crate::requirement::{
    DeclaredWorkspaceRoot, WorkspaceCapabilities, WorkspaceId, WorkspaceRequirement,
    WorkspaceSelection,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Which observation supplied a resolved root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSource {
    McpRoots,
    CodexSandboxMeta,
    Declared,
}

/// Why a particular root was selected for a requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSelectionReason {
    /// An MCP root's name equals the requirement id.
    MatchedByName,
    /// An MCP root's name equals one of the requirement aliases.
    MatchedByAlias { alias: String },
    /// The client supplied exactly one root and the requirement allows
    /// single-root selection.
    SingleRootPrimary,
    /// A root's URI is path-equivalent to the requirement's configured URI.
    MatchedByUri,
    /// The root was derived from Codex `sandbox_cwd`.
    CodexDerived { kind: DerivedRootKind },
    /// A declared root in the observation set matched the requirement.
    DeclaredObservation,
    /// The requirement's own declared fallback was used.
    DeclaredFallback,
}

/// The root selected for one workspace requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedWorkspaceRoot {
    pub id: WorkspaceId,
    pub root_uri: String,
    pub source: WorkspaceSource,
    pub selection_reason: WorkspaceSelectionReason,
    pub capabilities: WorkspaceCapabilities,
}

/// The outcome of a resolution pass: selected roots plus diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedWorkspaceSet {
    pub roots: Vec<ResolvedWorkspaceRoot>,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

impl ResolvedWorkspaceSet {
    /// The resolved root for `id`, if resolution selected one.
    pub fn root(&self, id: &WorkspaceId) -> Option<&ResolvedWorkspaceRoot> {
        self.roots.iter().find(|root| &root.id == id)
    }
}

/// Resolves each requirement against the observation set using the default
/// Codex root derivation policy.
pub fn resolve_workspaces(
    requirements: &[WorkspaceRequirement],
    observations: &WorkspaceObservationSet,
) -> ResolvedWorkspaceSet {
    resolve_workspaces_with_policy(requirements, observations, &RootDerivationPolicy::default())
}

/// Resolves each requirement against the observation set.
///
/// Sources apply in authority order: MCP roots, Codex sandbox metadata,
/// declared roots. A present observation blocks fall-through to every
/// lower-authority source, even when it resolves nothing.
pub fn resolve_workspaces_with_policy(
    requirements: &[WorkspaceRequirement],
    observations: &WorkspaceObservationSet,
    derivation: &RootDerivationPolicy,
) -> ResolvedWorkspaceSet {
    let mut set = ResolvedWorkspaceSet::default();

    if let Some(mcp) = observations.mcp_roots() {
        resolve_from_mcp_roots(requirements, &mcp.roots, &mut set);
        return set;
    }

    if let Some(codex) = observations.codex_sandbox() {
        let derived = derive_root(&codex.sandbox_cwd, derivation);
        resolve_from_codex(requirements, &derived, &mut set);
        return set;
    }

    resolve_from_declared(requirements, observations.declared(), &mut set);
    set
}

/// A root that passed scheme validation, with its normalized path.
struct FileRoot<'a> {
    root: &'a McpRoot,
    path: NormalizedPath,
}

fn resolve_from_mcp_roots(
    requirements: &[WorkspaceRequirement],
    roots: &[McpRoot],
    set: &mut ResolvedWorkspaceSet,
) {
    let mut file_roots: Vec<FileRoot<'_>> = Vec::new();
    for root in roots {
        match normalize_file_uri(&root.uri) {
            Ok(path) => file_roots.push(FileRoot { root, path }),
            Err(err) => set.diagnostics.push(WorkspaceDiagnostic::unsupported_scheme(
                None,
                err.to_string(),
                root.uri.clone(),
            )),
        }
    }

    for requirement in requirements {
        resolve_requirement_from_roots(requirement, &file_roots, set);
    }
}

fn resolve_requirement_from_roots(
    requirement: &WorkspaceRequirement,
    file_roots: &[FileRoot<'_>],
    set: &mut ResolvedWorkspaceSet,
) {
    let matches: Vec<(&FileRoot<'_>, WorkspaceSelectionReason)> = match &requirement.selection {
        WorkspaceSelection::ByNameOrAlias | WorkspaceSelection::PrimaryWhenSingleRoot => {
            match_by_name_or_alias(requirement, file_roots)
        }
        WorkspaceSelection::ExplicitUri { uri } => {
            match match_by_uri(uri, file_roots) {
                Ok(matches) => matches,
                Err(diagnostic) => {
                    set.diagnostics.push(diagnostic);
                    set.diagnostics.push(WorkspaceDiagnostic::unresolved(
                        requirement.id.clone(),
                        format!(
                            "workspace requirement `{}` has a non-file configured URI",
                            requirement.id
                        ),
                    ));
                    return;
                }
            }
        }
    };

    match matches.len() {
        1 => {
            let (file_root, reason) = matches.into_iter().next().expect("one match");
            set.roots.push(ResolvedWorkspaceRoot {
                id: requirement.id.clone(),
                root_uri: file_root.root.uri.clone(),
                source: WorkspaceSource::McpRoots,
                selection_reason: reason,
                capabilities: WorkspaceCapabilities::default(),
            });
        }
        0 => {
            if requirement.selection == WorkspaceSelection::PrimaryWhenSingleRoot
                && file_roots.len() == 1
            {
                let file_root = &file_roots[0];
                set.roots.push(ResolvedWorkspaceRoot {
                    id: requirement.id.clone(),
                    root_uri: file_root.root.uri.clone(),
                    source: WorkspaceSource::McpRoots,
                    selection_reason: WorkspaceSelectionReason::SingleRootPrimary,
                    capabilities: WorkspaceCapabilities::default(),
                });
                return;
            }
            set.diagnostics.push(WorkspaceDiagnostic::unresolved(
                requirement.id.clone(),
                format!(
                    "no MCP root matches workspace requirement `{}`; \
                     lower-authority sources do not apply while MCP roots are present",
                    requirement.id
                ),
            ));
        }
        _ => {
            let uris = matches
                .iter()
                .map(|(file_root, _)| file_root.root.uri.clone())
                .collect();
            set.diagnostics.push(WorkspaceDiagnostic::ambiguous(
                requirement.id.clone(),
                format!(
                    "multiple MCP roots match workspace requirement `{}`",
                    requirement.id
                ),
                uris,
            ));
        }
    }
}

fn match_by_name_or_alias<'a>(
    requirement: &WorkspaceRequirement,
    file_roots: &'a [FileRoot<'a>],
) -> Vec<(&'a FileRoot<'a>, WorkspaceSelectionReason)> {
    let mut matches = Vec::new();
    for file_root in file_roots {
        let Some(name) = file_root.root.name.as_deref() else {
            continue;
        };
        if requirement.id == name {
            matches.push((file_root, WorkspaceSelectionReason::MatchedByName));
        } else if let Some(alias) = requirement.aliases.iter().find(|alias| *alias == name) {
            matches.push((
                file_root,
                WorkspaceSelectionReason::MatchedByAlias {
                    alias: alias.clone(),
                },
            ));
        }
    }
    matches
}

fn match_by_uri<'a>(
    configured_uri: &str,
    file_roots: &'a [FileRoot<'a>],
) -> Result<Vec<(&'a FileRoot<'a>, WorkspaceSelectionReason)>, WorkspaceDiagnostic> {
    let configured = normalize_file_uri(configured_uri).map_err(|err| {
        WorkspaceDiagnostic::unsupported_scheme(None, err.to_string(), configured_uri.to_string())
    })?;

    Ok(file_roots
        .iter()
        .filter(|file_root| paths_equal(&file_root.path, &configured))
        .map(|file_root| (file_root, WorkspaceSelectionReason::MatchedByUri))
        .collect())
}

fn resolve_from_codex(
    requirements: &[WorkspaceRequirement],
    derived: &DerivedRoot,
    set: &mut ResolvedWorkspaceSet,
) {
    let root_uri = file_uri_for_path(&derived.path);
    let derived_path = normalize_file_uri(&root_uri).expect("file URI normalizes");

    for requirement in requirements {
        if let WorkspaceSelection::ExplicitUri { uri } = &requirement.selection {
            match normalize_file_uri(uri) {
                Ok(configured) if paths_equal(&configured, &derived_path) => {}
                Ok(_) => {
                    set.diagnostics.push(
                        WorkspaceDiagnostic::unresolved(
                            requirement.id.clone(),
                            format!(
                                "Codex-derived root `{root_uri}` does not match the configured URI \
                                 for workspace requirement `{}`",
                                requirement.id
                            ),
                        )
                        .with_roots(vec![root_uri.clone()]),
                    );
                    continue;
                }
                Err(err) => {
                    set.diagnostics.push(WorkspaceDiagnostic::unsupported_scheme(
                        Some(requirement.id.clone()),
                        err.to_string(),
                        uri.clone(),
                    ));
                    continue;
                }
            }
        }

        set.roots.push(ResolvedWorkspaceRoot {
            id: requirement.id.clone(),
            root_uri: root_uri.clone(),
            source: WorkspaceSource::CodexSandboxMeta,
            selection_reason: WorkspaceSelectionReason::CodexDerived {
                kind: derived.kind.clone(),
            },
            capabilities: WorkspaceCapabilities::default(),
        });
    }
}

fn resolve_from_declared(
    requirements: &[WorkspaceRequirement],
    declared: &[DeclaredWorkspaceRoot],
    set: &mut ResolvedWorkspaceSet,
) {
    for requirement in requirements {
        let matches: Vec<&DeclaredWorkspaceRoot> = declared
            .iter()
            .filter(|root| {
                root.id == requirement.id
                    || requirement
                        .aliases
                        .iter()
                        .any(|alias| root.id == alias.as_str())
            })
            .collect();

        match matches.len() {
            1 => {
                let root = matches[0];
                if let Err(err) = normalize_file_uri(&root.uri) {
                    set.diagnostics.push(WorkspaceDiagnostic::unsupported_scheme(
                        Some(requirement.id.clone()),
                        err.to_string(),
                        root.uri.clone(),
                    ));
                    continue;
                }
                set.roots.push(ResolvedWorkspaceRoot {
                    id: requirement.id.clone(),
                    root_uri: root.uri.clone(),
                    source: WorkspaceSource::Declared,
                    selection_reason: WorkspaceSelectionReason::DeclaredObservation,
                    capabilities: root.capabilities,
                });
            }
            0 => match &requirement.fallback {
                Some(fallback) => {
                    if let Err(err) = normalize_file_uri(&fallback.uri) {
                        set.diagnostics.push(WorkspaceDiagnostic::unsupported_scheme(
                            Some(requirement.id.clone()),
                            err.to_string(),
                            fallback.uri.clone(),
                        ));
                        continue;
                    }
                    set.roots.push(ResolvedWorkspaceRoot {
                        id: requirement.id.clone(),
                        root_uri: fallback.uri.clone(),
                        source: WorkspaceSource::Declared,
                        selection_reason: WorkspaceSelectionReason::DeclaredFallback,
                        capabilities: fallback.capabilities,
                    });
                }
                None => {
                    set.diagnostics.push(WorkspaceDiagnostic::unresolved(
                        requirement.id.clone(),
                        format!(
                            "no declared workspace root matches requirement `{}` \
                             and no runtime observation is present",
                            requirement.id
                        ),
                    ));
                }
            },
            _ => {
                let uris = matches.iter().map(|root| root.uri.clone()).collect();
                set.diagnostics.push(WorkspaceDiagnostic::ambiguous(
                    requirement.id.clone(),
                    format!(
                        "multiple declared workspace roots match requirement `{}`",
                        requirement.id
                    ),
                    uris,
                ));
            }
        }
    }
}

/// Formats a filesystem path as a `file:` URI without percent-encoding.
///
/// Lexical formatting only: separators become `/`, drive-letter paths get the
/// `file:///C:/...` shape.
fn file_uri_for_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    // Windows canonicalization produces verbatim and UNC prefixes that must
    // not leak into file: URIs.
    if let Some(unc) = text.strip_prefix("//?/UNC/") {
        return format!("file://{unc}");
    }
    if let Some(verbatim) = text.strip_prefix("//?/") {
        return format!("file:///{verbatim}");
    }
    if let Some(network) = text.strip_prefix("//") {
        return format!("file://{network}");
    }
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::file_uri_for_path;
    use std::path::Path;

    #[test]
    fn file_uri_formats_windows_network_and_verbatim_paths() {
        assert_eq!(
            file_uri_for_path(Path::new(r"\\server\share\repo")),
            "file://server/share/repo"
        );
        assert_eq!(
            file_uri_for_path(Path::new(r"\\?\UNC\server\share\repo")),
            "file://server/share/repo"
        );
        assert_eq!(
            file_uri_for_path(Path::new(r"\\?\C:\repo")),
            "file:///C:/repo"
        );
        assert_eq!(
            file_uri_for_path(Path::new("/workspace/repo")),
            "file:///workspace/repo"
        );
    }
}
