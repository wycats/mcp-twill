use std::{
    fs,
    path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type WorkspaceId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRequirement {
    pub id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub selection: WorkspaceSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<DeclaredWorkspaceRoot>,
}

impl WorkspaceRequirement {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: None,
            aliases: Vec::new(),
            selection: WorkspaceSelection::default(),
            fallback: None,
        }
    }

    pub fn primary(id: impl Into<String>) -> Self {
        Self::new(id).with_selection(WorkspaceSelection::PrimaryWhenSingleRoot)
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

    fn matches_name(&self, name: &str) -> bool {
        self.id == name || self.aliases.iter().any(|alias| alias == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceSelection {
    ByNameOrAlias,
    PrimaryWhenSingleRoot,
    ExplicitUri { uri: String },
}

impl Default for WorkspaceSelection {
    fn default() -> Self {
        Self::ByNameOrAlias
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceObservationSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_roots: Option<McpRootsObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_sandbox: Option<CodexSandboxObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared: Vec<DeclaredWorkspaceRoot>,
}

impl WorkspaceObservationSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mcp_roots(mut self, roots: McpRootsObservation) -> Self {
        self.mcp_roots = Some(roots);
        self
    }

    pub fn with_codex_sandbox(mut self, sandbox: CodexSandboxObservation) -> Self {
        self.codex_sandbox = Some(sandbox);
        self
    }

    pub fn with_declared(mut self, root: DeclaredWorkspaceRoot) -> Self {
        self.declared.push(root);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
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

    pub fn named(uri: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            name: Some(name.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpRootsObservation {
    pub roots: Vec<McpRoot>,
}

impl McpRootsObservation {
    pub fn new(roots: Vec<McpRoot>) -> Self {
        Self { roots }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexSandboxObservation {
    pub sandbox_cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile: Option<String>,
    #[serde(default)]
    pub root_derivation: RootDerivationPolicy,
}

impl CodexSandboxObservation {
    pub fn new(sandbox_cwd: impl Into<PathBuf>) -> Self {
        Self {
            sandbox_cwd: sandbox_cwd.into(),
            permission_profile: None,
            root_derivation: RootDerivationPolicy::default(),
        }
    }

    pub fn with_permission_profile(mut self, permission_profile: impl Into<String>) -> Self {
        self.permission_profile = Some(permission_profile.into());
        self
    }

    pub fn with_root_derivation(mut self, root_derivation: RootDerivationPolicy) -> Self {
        self.root_derivation = root_derivation;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RootDerivationPolicy {
    ProjectBoundary {
        vcs_markers: Vec<String>,
        project_markers: Vec<String>,
    },
    ExactDirectory,
}

impl Default for RootDerivationPolicy {
    fn default() -> Self {
        Self::ProjectBoundary {
            vcs_markers: vec![".git".to_string(), ".jj".to_string(), ".hg".to_string()],
            project_markers: vec![
                "Cargo.toml".to_string(),
                "package.json".to_string(),
                "pyproject.toml".to_string(),
                "go.mod".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredWorkspaceRoot {
    pub id: WorkspaceId,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: WorkspaceCapabilities,
}

impl DeclaredWorkspaceRoot {
    pub fn new(id: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            uri: uri.into(),
            display_name: None,
            capabilities: WorkspaceCapabilities::default(),
        }
    }

    pub fn file(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(id, path)
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCapabilities {
    pub readable: bool,
    pub writable: bool,
}

impl Default for WorkspaceCapabilities {
    fn default() -> Self {
        Self {
            readable: true,
            writable: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedWorkspaceSet {
    pub roots: Vec<ResolvedWorkspaceRoot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

impl ResolvedWorkspaceSet {
    pub fn root(&self, id: &str) -> Option<&ResolvedWorkspaceRoot> {
        self.roots.iter().find(|root| root.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedWorkspaceRoot {
    pub id: WorkspaceId,
    pub root_uri: String,
    pub source: WorkspaceSource,
    pub selection_reason: WorkspaceSelectionReason,
    pub capabilities: WorkspaceCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceSource {
    McpRoots,
    CodexSandboxMeta,
    Declared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceSelectionReason {
    RequirementName { name: String },
    RequirementAlias { alias: String },
    SingleRoot,
    ExplicitUri { uri: String },
    CodexProjectMarker { marker: String },
    CodexExactDirectory,
    DeclaredFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiagnostic {
    pub code: WorkspaceDiagnosticCode,
    pub message: String,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceDiagnosticCode {
    UnresolvedWorkspaceRequirement,
    AmbiguousWorkspaceRoot,
}

pub fn resolve_workspaces(
    requirements: &[WorkspaceRequirement],
    observations: &WorkspaceObservationSet,
) -> ResolvedWorkspaceSet {
    let mut roots = Vec::new();
    let mut diagnostics = Vec::new();

    for requirement in requirements {
        match resolve_requirement(requirement, observations) {
            ResolutionOutcome::Resolved(root) => roots.push(root),
            ResolutionOutcome::Diagnostic(diagnostic) => diagnostics.push(diagnostic),
            ResolutionOutcome::Unresolved => diagnostics.push(unresolved_diagnostic(requirement)),
        }
    }

    ResolvedWorkspaceSet { roots, diagnostics }
}

enum ResolutionOutcome {
    Resolved(ResolvedWorkspaceRoot),
    Diagnostic(WorkspaceDiagnostic),
    Unresolved,
}

fn resolve_requirement(
    requirement: &WorkspaceRequirement,
    observations: &WorkspaceObservationSet,
) -> ResolutionOutcome {
    if let WorkspaceSelection::ExplicitUri { uri } = &requirement.selection {
        return resolve_explicit_uri_requirement(requirement, observations, uri);
    }

    if let Some(mcp_roots) = &observations.mcp_roots {
        if !mcp_roots.roots.is_empty() {
            return resolve_mcp_requirement(requirement, mcp_roots);
        }
    }

    if let Some(sandbox) = &observations.codex_sandbox {
        let (root_uri, reason) = derive_codex_root(sandbox);
        return ResolutionOutcome::Resolved(ResolvedWorkspaceRoot {
            id: requirement.id.clone(),
            root_uri,
            source: WorkspaceSource::CodexSandboxMeta,
            selection_reason: reason,
            capabilities: WorkspaceCapabilities::default(),
        });
    }

    if let Some(declared) = declared_root_for_requirement(requirement, observations) {
        return ResolutionOutcome::Resolved(resolved_declared_root(
            requirement,
            declared,
            WorkspaceSelectionReason::DeclaredFallback,
        ));
    }

    ResolutionOutcome::Unresolved
}

fn resolve_explicit_uri_requirement(
    requirement: &WorkspaceRequirement,
    observations: &WorkspaceObservationSet,
    uri: &str,
) -> ResolutionOutcome {
    if let Some(mcp_roots) = &observations.mcp_roots {
        if !mcp_roots.roots.is_empty() {
            return resolve_mcp_requirement(requirement, mcp_roots);
        }
    }

    if let Some(sandbox) = &observations.codex_sandbox {
        let (root_uri, _) = derive_codex_root(sandbox);
        if root_uri == uri {
            return ResolutionOutcome::Resolved(ResolvedWorkspaceRoot {
                id: requirement.id.clone(),
                root_uri,
                source: WorkspaceSource::CodexSandboxMeta,
                selection_reason: WorkspaceSelectionReason::ExplicitUri {
                    uri: uri.to_string(),
                },
                capabilities: WorkspaceCapabilities::default(),
            });
        }
        return ResolutionOutcome::Diagnostic(unresolved_diagnostic_with_candidates(
            requirement,
            vec![root_uri],
        ));
    }

    let matches = explicit_declared_roots(requirement, observations, uri);
    match matches.as_slice() {
        [] => ResolutionOutcome::Diagnostic(unresolved_diagnostic(requirement)),
        [declared] => ResolutionOutcome::Resolved(resolved_declared_root(
            requirement,
            declared,
            WorkspaceSelectionReason::ExplicitUri {
                uri: uri.to_string(),
            },
        )),
        _ => ResolutionOutcome::Diagnostic(WorkspaceDiagnostic {
            code: WorkspaceDiagnosticCode::AmbiguousWorkspaceRoot,
            message: format!(
                "workspace requirement `{}` matched multiple declared workspace roots",
                requirement.id
            ),
            workspace_id: requirement.id.clone(),
            candidates: matches.iter().map(|root| root.uri.clone()).collect(),
        }),
    }
}

fn resolve_mcp_requirement(
    requirement: &WorkspaceRequirement,
    observation: &McpRootsObservation,
) -> ResolutionOutcome {
    if matches!(
        requirement.selection,
        WorkspaceSelection::PrimaryWhenSingleRoot
    ) && observation.roots.len() == 1
    {
        return ResolutionOutcome::Resolved(resolved_mcp_root(
            requirement,
            &observation.roots[0],
            WorkspaceSelectionReason::SingleRoot,
        ));
    }

    let matches = matching_mcp_roots(requirement, observation);
    match matches.as_slice() {
        [] => ResolutionOutcome::Diagnostic(unresolved_diagnostic(requirement)),
        [(root, reason)] => {
            ResolutionOutcome::Resolved(resolved_mcp_root(requirement, root, reason.clone()))
        }
        _ => ResolutionOutcome::Diagnostic(ambiguous_diagnostic(requirement, &matches)),
    }
}

fn matching_mcp_roots<'a>(
    requirement: &WorkspaceRequirement,
    observation: &'a McpRootsObservation,
) -> Vec<(&'a McpRoot, WorkspaceSelectionReason)> {
    match &requirement.selection {
        WorkspaceSelection::ExplicitUri { uri } => observation
            .roots
            .iter()
            .filter(|root| &root.uri == uri)
            .map(|root| {
                (
                    root,
                    WorkspaceSelectionReason::ExplicitUri { uri: uri.clone() },
                )
            })
            .collect(),
        WorkspaceSelection::ByNameOrAlias | WorkspaceSelection::PrimaryWhenSingleRoot => {
            observation
                .roots
                .iter()
                .filter_map(|root| {
                    let name = root.name.as_ref()?;
                    if requirement.id == *name {
                        Some((
                            root,
                            WorkspaceSelectionReason::RequirementName { name: name.clone() },
                        ))
                    } else if requirement.matches_name(name) {
                        Some((
                            root,
                            WorkspaceSelectionReason::RequirementAlias {
                                alias: name.clone(),
                            },
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        }
    }
}

fn resolved_mcp_root(
    requirement: &WorkspaceRequirement,
    root: &McpRoot,
    reason: WorkspaceSelectionReason,
) -> ResolvedWorkspaceRoot {
    ResolvedWorkspaceRoot {
        id: requirement.id.clone(),
        root_uri: root.uri.clone(),
        source: WorkspaceSource::McpRoots,
        selection_reason: reason,
        capabilities: WorkspaceCapabilities::default(),
    }
}

fn declared_root_for_requirement<'a>(
    requirement: &'a WorkspaceRequirement,
    observations: &'a WorkspaceObservationSet,
) -> Option<&'a DeclaredWorkspaceRoot> {
    requirement.fallback.as_ref().or_else(|| {
        observations
            .declared
            .iter()
            .find(|root| root.id == requirement.id)
    })
}

fn explicit_declared_roots<'a>(
    requirement: &'a WorkspaceRequirement,
    observations: &'a WorkspaceObservationSet,
    uri: &str,
) -> Vec<&'a DeclaredWorkspaceRoot> {
    requirement
        .fallback
        .iter()
        .chain(observations.declared.iter())
        .filter(|root| root.uri == uri)
        .collect()
}

fn resolved_declared_root(
    requirement: &WorkspaceRequirement,
    root: &DeclaredWorkspaceRoot,
    reason: WorkspaceSelectionReason,
) -> ResolvedWorkspaceRoot {
    ResolvedWorkspaceRoot {
        id: requirement.id.clone(),
        root_uri: root.uri.clone(),
        source: WorkspaceSource::Declared,
        selection_reason: reason,
        capabilities: root.capabilities.clone(),
    }
}

fn derive_codex_root(observation: &CodexSandboxObservation) -> (String, WorkspaceSelectionReason) {
    let sandbox_cwd = normalized_path(&observation.sandbox_cwd);

    match &observation.root_derivation {
        RootDerivationPolicy::ExactDirectory => (
            file_uri(&sandbox_cwd),
            WorkspaceSelectionReason::CodexExactDirectory,
        ),
        RootDerivationPolicy::ProjectBoundary {
            vcs_markers,
            project_markers,
        } => {
            if let Some((path, marker)) = find_ancestor_with_marker(&sandbox_cwd, vcs_markers) {
                return (
                    file_uri(&path),
                    WorkspaceSelectionReason::CodexProjectMarker { marker },
                );
            }
            if let Some((path, marker)) = find_ancestor_with_marker(&sandbox_cwd, project_markers) {
                return (
                    file_uri(&path),
                    WorkspaceSelectionReason::CodexProjectMarker { marker },
                );
            }
            (
                file_uri(&sandbox_cwd),
                WorkspaceSelectionReason::CodexExactDirectory,
            )
        }
    }
}

fn find_ancestor_with_marker(start: &Path, markers: &[String]) -> Option<(PathBuf, String)> {
    let start = normalized_path(start);
    for ancestor in start.ancestors() {
        for marker in markers {
            if ancestor.join(marker).exists() {
                return Some((ancestor.to_path_buf(), marker.clone()));
            }
        }
    }
    None
}

fn normalized_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn unresolved_diagnostic(requirement: &WorkspaceRequirement) -> WorkspaceDiagnostic {
    unresolved_diagnostic_with_candidates(requirement, Vec::new())
}

fn unresolved_diagnostic_with_candidates(
    requirement: &WorkspaceRequirement,
    candidates: Vec<String>,
) -> WorkspaceDiagnostic {
    WorkspaceDiagnostic {
        code: WorkspaceDiagnosticCode::UnresolvedWorkspaceRequirement,
        message: format!(
            "workspace requirement `{}` did not resolve to a workspace root",
            requirement.id
        ),
        workspace_id: requirement.id.clone(),
        candidates,
    }
}

fn ambiguous_diagnostic(
    requirement: &WorkspaceRequirement,
    matches: &[(&McpRoot, WorkspaceSelectionReason)],
) -> WorkspaceDiagnostic {
    WorkspaceDiagnostic {
        code: WorkspaceDiagnosticCode::AmbiguousWorkspaceRoot,
        message: format!(
            "workspace requirement `{}` matched multiple MCP roots",
            requirement.id
        ),
        workspace_id: requirement.id.clone(),
        candidates: matches.iter().map(|(root, _)| root.uri.clone()).collect(),
    }
}

fn file_uri(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if let Some(unc) = value.strip_prefix("//?/UNC/") {
        return format!("file://{unc}");
    }
    if let Some(local) = value.strip_prefix("//?/") {
        return format!("file:///{local}");
    }
    if let Some(unc) = value.strip_prefix("//") {
        return format!("file://{unc}");
    }
    if value.starts_with('/') {
        format!("file://{value}")
    } else {
        format!("file:///{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_one(
        requirement: WorkspaceRequirement,
        observations: WorkspaceObservationSet,
    ) -> ResolvedWorkspaceSet {
        resolve_workspaces(&[requirement], &observations)
    }

    #[test]
    fn single_mcp_root_resolves_primary_workspace_requirement() {
        let resolved = resolve_one(
            WorkspaceRequirement::primary("repo"),
            WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
                McpRoot::new("file:///workspace/repo"),
            ])),
        );

        assert!(resolved.diagnostics.is_empty());
        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, "file:///workspace/repo");
        assert_eq!(root.source, WorkspaceSource::McpRoots);
        assert_eq!(root.selection_reason, WorkspaceSelectionReason::SingleRoot);
    }

    #[test]
    fn multiple_mcp_roots_resolve_by_requirement_id() {
        let observations =
            WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
                McpRoot::named("file:///workspace/docs", "docs"),
                McpRoot::named("file:///workspace/repo", "repo"),
            ]));

        let resolved = resolve_one(WorkspaceRequirement::new("repo"), observations);

        assert!(resolved.diagnostics.is_empty());
        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, "file:///workspace/repo");
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::RequirementName {
                name: "repo".to_string()
            }
        );
    }

    #[test]
    fn multiple_mcp_roots_resolve_by_requirement_id_or_alias() {
        let observations =
            WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
                McpRoot::named("file:///workspace/docs", "docs"),
                McpRoot::named("file:///workspace/repo", "source"),
            ]));

        let resolved = resolve_one(
            WorkspaceRequirement::new("repo").with_alias("source"),
            observations,
        );

        assert!(resolved.diagnostics.is_empty());
        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, "file:///workspace/repo");
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::RequirementAlias {
                alias: "source".to_string()
            }
        );
    }

    #[test]
    fn multiple_mcp_roots_without_match_produce_unresolved_diagnostic() {
        let resolved = resolve_one(
            WorkspaceRequirement::new("repo"),
            WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
                McpRoot::named("file:///workspace/docs", "docs"),
                McpRoot::named("file:///workspace/tmp", "tmp"),
            ])),
        );

        assert!(resolved.roots.is_empty());
        assert_eq!(
            resolved.diagnostics[0].code,
            WorkspaceDiagnosticCode::UnresolvedWorkspaceRequirement
        );
    }

    #[test]
    fn multiple_matching_mcp_roots_produce_ambiguous_diagnostic() {
        let resolved = resolve_one(
            WorkspaceRequirement::new("repo"),
            WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
                McpRoot::named("file:///workspace/one", "repo"),
                McpRoot::named("file:///workspace/two", "repo"),
            ])),
        );

        assert!(resolved.roots.is_empty());
        assert_eq!(
            resolved.diagnostics[0].code,
            WorkspaceDiagnosticCode::AmbiguousWorkspaceRoot
        );
        assert_eq!(resolved.diagnostics[0].candidates.len(), 2);
    }

    #[test]
    fn explicit_uri_resolves_declared_workspace_by_uri() {
        let uri = "file:///workspace/repo";
        let resolved = resolve_one(
            WorkspaceRequirement::new("repo").with_selection(WorkspaceSelection::ExplicitUri {
                uri: uri.to_string(),
            }),
            WorkspaceObservationSet::new()
                .with_declared(DeclaredWorkspaceRoot::file("source", uri)),
        );

        assert!(resolved.diagnostics.is_empty());
        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, uri);
        assert_eq!(root.source, WorkspaceSource::Declared);
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::ExplicitUri {
                uri: uri.to_string()
            }
        );
    }

    #[test]
    fn explicit_uri_matches_codex_derived_root() {
        let temp = test_dir("explicit_codex_match");
        let repo = temp.join("repo");
        let nested = repo.join("src").join("lib");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        let uri = file_uri(&repo);

        let resolved = resolve_one(
            WorkspaceRequirement::new("repo")
                .with_selection(WorkspaceSelection::ExplicitUri { uri: uri.clone() }),
            WorkspaceObservationSet::new()
                .with_codex_sandbox(CodexSandboxObservation::new(&nested)),
        );

        assert!(resolved.diagnostics.is_empty());
        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, uri);
        assert_eq!(root.source, WorkspaceSource::CodexSandboxMeta);
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::ExplicitUri { uri }
        );
    }

    #[test]
    fn explicit_uri_rejects_mismatched_codex_derived_root() {
        let nested = test_dir("explicit_codex_mismatch");
        std::fs::create_dir_all(&nested).unwrap();

        let resolved = resolve_one(
            WorkspaceRequirement::new("repo").with_selection(WorkspaceSelection::ExplicitUri {
                uri: "file:///workspace/other".to_string(),
            }),
            WorkspaceObservationSet::new()
                .with_codex_sandbox(CodexSandboxObservation::new(&nested)),
        );

        assert!(resolved.roots.is_empty());
        assert_eq!(
            resolved.diagnostics[0].code,
            WorkspaceDiagnosticCode::UnresolvedWorkspaceRequirement
        );
        assert_eq!(resolved.diagnostics[0].candidates, vec![file_uri(&nested)]);
    }

    #[test]
    fn codex_sandbox_derives_vcs_project_boundary() {
        let temp = test_dir("codex_vcs");
        let repo = temp.join("repo");
        let nested = repo.join("src").join("lib");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();

        let resolved = resolve_one(
            WorkspaceRequirement::new("repo"),
            WorkspaceObservationSet::new()
                .with_codex_sandbox(CodexSandboxObservation::new(&nested)),
        );

        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, file_uri(&repo));
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::CodexProjectMarker {
                marker: ".git".to_string()
            }
        );
    }

    #[test]
    fn codex_sandbox_derives_project_marker_boundary() {
        let temp = test_dir("codex_project_marker");
        let repo = temp.join("repo");
        let nested = repo.join("src").join("lib");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();

        let resolved = resolve_one(
            WorkspaceRequirement::new("repo"),
            WorkspaceObservationSet::new()
                .with_codex_sandbox(CodexSandboxObservation::new(&nested)),
        );

        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, file_uri(&repo));
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::CodexProjectMarker {
                marker: "Cargo.toml".to_string()
            }
        );
    }

    #[test]
    fn codex_sandbox_canonicalizes_cwd_before_marker_derivation() {
        let temp = test_dir("codex_canonical_marker");
        let repo = temp.join("repo");
        let nested = repo.join("src").join("lib");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        let non_canonical_nested = repo
            .join("src")
            .join("..")
            .join("src")
            .join("lib")
            .join(".");

        let resolved = resolve_one(
            WorkspaceRequirement::new("repo"),
            WorkspaceObservationSet::new()
                .with_codex_sandbox(CodexSandboxObservation::new(non_canonical_nested)),
        );

        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, file_uri(&repo));
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::CodexProjectMarker {
                marker: ".git".to_string()
            }
        );
    }

    #[test]
    fn codex_sandbox_falls_back_to_sandbox_directory() {
        let nested = test_dir("codex_exact");
        std::fs::create_dir_all(&nested).unwrap();

        let resolved = resolve_one(
            WorkspaceRequirement::new("repo"),
            WorkspaceObservationSet::new()
                .with_codex_sandbox(CodexSandboxObservation::new(&nested)),
        );

        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, file_uri(&nested));
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::CodexExactDirectory
        );
    }

    #[test]
    fn declared_workspace_resolves_without_runtime_observations() {
        let resolved = resolve_one(
            WorkspaceRequirement::new("repo"),
            WorkspaceObservationSet::new().with_declared(DeclaredWorkspaceRoot::file(
                "repo",
                "file:///workspace/repo",
            )),
        );

        assert!(resolved.diagnostics.is_empty());
        let root = resolved.root("repo").unwrap();
        assert_eq!(root.root_uri, "file:///workspace/repo");
        assert_eq!(root.source, WorkspaceSource::Declared);
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::DeclaredFallback
        );
    }

    #[test]
    fn file_uri_formats_windows_network_and_verbatim_paths() {
        assert_eq!(
            file_uri(Path::new(r"\\server\share\repo")),
            "file://server/share/repo"
        );
        assert_eq!(
            file_uri(Path::new(r"\\?\UNC\server\share\repo")),
            "file://server/share/repo"
        );
        assert_eq!(file_uri(Path::new(r"\\?\C:\repo")), "file:///C:/repo");
    }

    fn test_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "mcp_workspace_resolver_{name}_{}",
            std::process::id()
        ));
        if path.exists() {
            std::fs::remove_dir_all(&path).unwrap();
        }
        path
    }
}
