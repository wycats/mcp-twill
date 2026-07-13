//! Acceptance tests from RFC 0007 (resolver-only subset).

use mcp_workspace_resolver::{
    AMBIGUOUS_WORKSPACE_ROOT, CodexSandboxObservation, DeclaredWorkspaceRoot, DerivedRootKind,
    HostWorkspaceRoot, HostWorkspaceRootsObservation, McpRoot, McpRootsObservation,
    RootDerivationPolicy, UNRESOLVED_WORKSPACE_REQUIREMENT, UNSUPPORTED_ROOT_SCHEME,
    WorkspaceObservationSet, WorkspaceRequirement, WorkspaceSelection, WorkspaceSelectionReason,
    WorkspaceSource, resolve_workspaces, resolve_workspaces_with_policy,
};

fn requirement(id: &str) -> WorkspaceRequirement {
    WorkspaceRequirement::new(id)
}

#[test]
fn single_mcp_root_resolves_primary_requirement() {
    let requirements =
        [requirement("repo").with_selection(WorkspaceSelection::PrimaryWhenSingleRoot)];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/project"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(
        resolved.diagnostics.is_empty(),
        "{:?}",
        resolved.diagnostics
    );
    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///workspace/project");
    assert_eq!(root.source, WorkspaceSource::McpRoots);
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::SingleRootPrimary
    );
}

#[test]
fn multiple_mcp_roots_resolve_by_name() {
    let requirements = [requirement("docs")];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/code").with_name("code"),
            McpRoot::new("file:///workspace/docs").with_name("docs"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(
        resolved.diagnostics.is_empty(),
        "{:?}",
        resolved.diagnostics
    );
    let root = resolved.root(&"docs".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///workspace/docs");
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::MatchedByName
    );
}

#[test]
fn multiple_mcp_roots_resolve_by_alias() {
    let requirements = [requirement("repo").with_alias("source")];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/docs").with_name("docs"),
            McpRoot::new("file:///workspace/code").with_name("source"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///workspace/code");
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::MatchedByAlias {
            alias: "source".into()
        }
    );
}

#[test]
fn name_matching_is_case_sensitive() {
    let requirements = [requirement("repo")];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/a").with_name("Repo"),
            McpRoot::new("file:///workspace/b").with_name("other"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(resolved.roots.is_empty());
    assert_eq!(resolved.diagnostics.len(), 1);
    assert_eq!(
        resolved.diagnostics[0].code,
        UNRESOLVED_WORKSPACE_REQUIREMENT
    );
}

#[test]
fn multiple_mcp_roots_with_no_match_are_unresolved() {
    let requirements = [requirement("repo")];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/a").with_name("alpha"),
            McpRoot::new("file:///workspace/b").with_name("beta"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(resolved.roots.is_empty());
    let diagnostic = &resolved.diagnostics[0];
    assert_eq!(diagnostic.code, UNRESOLVED_WORKSPACE_REQUIREMENT);
    assert_eq!(diagnostic.requirement, Some("repo".into()));
}

#[test]
fn multiple_matching_roots_are_ambiguous() {
    let requirements = [requirement("repo").with_alias("source")];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/a").with_name("repo"),
            McpRoot::new("file:///workspace/b").with_name("source"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(resolved.roots.is_empty());
    let diagnostic = &resolved.diagnostics[0];
    assert_eq!(diagnostic.code, AMBIGUOUS_WORKSPACE_ROOT);
    assert_eq!(
        diagnostic.roots,
        vec![
            "file:///workspace/a".to_string(),
            "file:///workspace/b".to_string()
        ]
    );
}

#[test]
fn present_but_unmatched_mcp_roots_block_fall_through() {
    let requirements = [requirement("repo")
        .with_fallback(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"))];
    let observations = WorkspaceObservationSet::new()
        .with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/a").with_name("alpha"),
            McpRoot::new("file:///workspace/b").with_name("beta"),
        ]))
        .with_declared(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(
        resolved.roots.is_empty(),
        "declared roots must not participate"
    );
    assert_eq!(
        resolved.diagnostics[0].code,
        UNRESOLVED_WORKSPACE_REQUIREMENT
    );
}

#[test]
fn empty_mcp_roots_list_blocks_fall_through() {
    let requirements = [
        requirement("repo")
            .with_fallback(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo")),
        requirement("docs"),
    ];
    let observations = WorkspaceObservationSet::new()
        .with_mcp_roots(McpRootsObservation::default())
        .with_codex_sandbox(CodexSandboxObservation::new("/sandbox/cwd"))
        .with_declared(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(resolved.roots.is_empty(), "nothing may resolve");
    assert_eq!(resolved.diagnostics.len(), 2);
    for diagnostic in &resolved.diagnostics {
        assert_eq!(diagnostic.code, UNRESOLVED_WORKSPACE_REQUIREMENT);
    }
}

#[test]
fn declared_roots_resolve_when_no_runtime_observation_present() {
    let requirements = [requirement("repo")];
    let observations = WorkspaceObservationSet::new()
        .with_declared(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(
        resolved.diagnostics.is_empty(),
        "{:?}",
        resolved.diagnostics
    );
    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///declared/repo");
    assert_eq!(root.source, WorkspaceSource::Declared);
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::DeclaredObservation
    );
}

#[test]
fn requirement_fallback_resolves_when_nothing_else_matches() {
    let requirements = [requirement("repo")
        .with_fallback(DeclaredWorkspaceRoot::new("repo", "file:///fallback/repo"))];
    let observations = WorkspaceObservationSet::new();

    let resolved = resolve_workspaces(&requirements, &observations);

    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///fallback/repo");
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::DeclaredFallback
    );
}

#[test]
fn non_file_uri_root_produces_unsupported_scheme_diagnostic() {
    let requirements = [requirement("repo")];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("https://example.com/repo").with_name("repo"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(resolved.roots.is_empty());
    let scheme_diagnostic = resolved
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == UNSUPPORTED_ROOT_SCHEME)
        .expect("unsupported_root_scheme diagnostic");
    assert_eq!(scheme_diagnostic.roots, vec!["https://example.com/repo"]);
    assert!(
        resolved
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == UNRESOLVED_WORKSPACE_REQUIREMENT),
        "requirement is also unresolved: {:?}",
        resolved.diagnostics
    );
}

#[test]
fn non_file_declared_root_pairs_scheme_and_unresolved_diagnostics() {
    let requirements = [requirement("repo")];
    let observations = WorkspaceObservationSet::new().with_declared(DeclaredWorkspaceRoot::new(
        "repo",
        "https://example.com/repo",
    ));

    let resolved = resolve_workspaces(&requirements, &observations);

    assert!(resolved.roots.is_empty());
    let scheme = resolved
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == UNSUPPORTED_ROOT_SCHEME)
        .expect("scheme diagnostic");
    assert_eq!(scheme.requirement, Some("repo".into()));
    assert!(
        resolved.diagnostics.iter().any(|diagnostic| diagnostic.code
            == UNRESOLVED_WORKSPACE_REQUIREMENT
            && diagnostic.requirement == Some("repo".into())),
        "requirement is also unresolved: {:?}",
        resolved.diagnostics
    );
}

#[test]
fn explicit_uri_selection_matches_equivalent_path() {
    let requirements = [
        requirement("repo").with_selection(WorkspaceSelection::ExplicitUri {
            uri: "file:///workspace/./project/".into(),
        }),
    ];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/project").with_name("anything"),
            McpRoot::new("file:///workspace/other").with_name("other"),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);

    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///workspace/project");
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::MatchedByUri
    );
}

#[test]
fn trusted_host_roots_resolve_by_name_and_preserve_issuer() {
    let requirements = [requirement("repo")];
    let observations =
        WorkspaceObservationSet::new().with_host_roots(HostWorkspaceRootsObservation::new([
            HostWorkspaceRoot::named("com.example.editor", "docs", "file:///workspace/docs")
                .unwrap(),
            HostWorkspaceRoot::named("com.example.editor", "repo", "file:///workspace/repo")
                .unwrap(),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);
    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///workspace/repo");
    assert_eq!(root.source, WorkspaceSource::TrustedHost);
    assert_eq!(root.source_issuer.as_deref(), Some("com.example.editor"));
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::MatchedByName
    );
}

#[test]
fn single_trusted_host_root_uses_registry_primary_rule() {
    let requirements =
        [requirement("repo").with_selection(WorkspaceSelection::PrimaryWhenSingleRoot)];
    let observations =
        WorkspaceObservationSet::new().with_host_roots(HostWorkspaceRootsObservation::new([
            HostWorkspaceRoot::new("com.example.editor", "file:///workspace/project").unwrap(),
        ]));

    let resolved = resolve_workspaces(&requirements, &observations);
    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///workspace/project");
    assert_eq!(
        root.selection_reason,
        WorkspaceSelectionReason::SingleRootPrimary
    );
}

#[test]
fn present_empty_trusted_host_roots_block_declared_fallback() {
    let requirements = [requirement("repo")
        .with_fallback(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"))];
    let observations = WorkspaceObservationSet::new()
        .with_host_roots(HostWorkspaceRootsObservation::default())
        .with_declared(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"));

    let resolved = resolve_workspaces(&requirements, &observations);
    assert!(resolved.roots.is_empty());
    assert_eq!(
        resolved.diagnostics[0].code,
        UNRESOLVED_WORKSPACE_REQUIREMENT
    );
}

#[test]
fn codex_and_mcp_authority_remain_above_trusted_host() {
    let requirements = [requirement("repo")];
    let host = HostWorkspaceRootsObservation::new([HostWorkspaceRoot::named(
        "com.example.editor",
        "repo",
        "file:///host/repo",
    )
    .unwrap()]);

    let mcp = WorkspaceObservationSet::new()
        .with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///mcp/repo").with_name("repo"),
        ]))
        .with_host_roots(host.clone());
    assert_eq!(
        resolve_workspaces(&requirements, &mcp).roots[0].source,
        WorkspaceSource::McpRoots
    );

    let codex = WorkspaceObservationSet::new()
        .with_codex_sandbox(CodexSandboxObservation::new("/codex/repo"))
        .with_host_roots(host);
    assert_eq!(
        resolve_workspaces(&requirements, &codex).roots[0].source,
        WorkspaceSource::CodexSandboxMeta
    );
}

mod codex {
    use super::*;
    use std::fs;

    #[test]
    fn codex_resolves_to_vcs_boundary_above_sandbox_cwd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let nested = project.join("src").join("deep");
        fs::create_dir_all(&nested).expect("create dirs");
        fs::create_dir(project.join(".git")).expect("create .git");
        // A project marker nearer than the VCS boundary must not win.
        fs::write(nested.join("Cargo.toml"), "").expect("write marker");

        let requirements = [requirement("repo")];
        let observations = WorkspaceObservationSet::new()
            .with_codex_sandbox(CodexSandboxObservation::new(&nested));

        let resolved = resolve_workspaces(&requirements, &observations);

        let root = resolved.root(&"repo".into()).expect("resolved");
        assert_eq!(root.source, WorkspaceSource::CodexSandboxMeta);
        let canonical = project.canonicalize().expect("canonicalize");
        assert_eq!(
            root.root_uri,
            format!("file://{}", canonical.display()),
            "diagnostics: {:?}",
            resolved.diagnostics
        );
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::CodexDerived {
                kind: DerivedRootKind::VcsBoundary {
                    marker: ".git".into()
                }
            }
        );
    }

    #[test]
    fn codex_resolves_to_project_marker_when_no_vcs_marker_visible() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let nested = project.join("src");
        fs::create_dir_all(&nested).expect("create dirs");
        fs::write(project.join("package.json"), "{}").expect("write marker");

        let requirements = [requirement("repo")];
        let observations = WorkspaceObservationSet::new()
            .with_codex_sandbox(CodexSandboxObservation::new(&nested));

        let resolved = resolve_workspaces(&requirements, &observations);

        let root = resolved.root(&"repo".into()).expect("resolved");
        let canonical = project.canonicalize().expect("canonicalize");
        assert_eq!(root.root_uri, format!("file://{}", canonical.display()));
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::CodexDerived {
                kind: DerivedRootKind::ProjectMarker {
                    marker: "package.json".into()
                }
            }
        );
    }

    #[test]
    fn codex_resolves_to_sandbox_cwd_when_no_marker_visible() {
        let temp = tempfile::tempdir().expect("tempdir");
        let nested = temp.path().join("bare").join("dir");
        fs::create_dir_all(&nested).expect("create dirs");

        let requirements = [requirement("repo")];
        // Constrain markers to names that cannot exist above the tempdir, so
        // the walk past the tempdir boundary finds nothing.
        let policy = RootDerivationPolicy::ProjectBoundary {
            vcs_markers: vec!["9d3f-nonexistent-vcs".into()],
            project_markers: vec!["9d3f-nonexistent-marker".into()],
        };
        let observations = WorkspaceObservationSet::new()
            .with_codex_sandbox(CodexSandboxObservation::new(&nested));

        let resolved = resolve_workspaces_with_policy(&requirements, &observations, &policy);

        let root = resolved.root(&"repo".into()).expect("resolved");
        let canonical = nested.canonicalize().expect("canonicalize");
        assert_eq!(root.root_uri, format!("file://{}", canonical.display()));
        assert_eq!(
            root.selection_reason,
            WorkspaceSelectionReason::CodexDerived {
                kind: DerivedRootKind::SandboxCwd
            }
        );
    }

    #[test]
    fn codex_blocks_fall_through_to_declared_roots() {
        let temp = tempfile::tempdir().expect("tempdir");
        let nested = temp.path().join("dir");
        fs::create_dir_all(&nested).expect("create dirs");

        let requirements = [requirement("repo")];
        let observations = WorkspaceObservationSet::new()
            .with_codex_sandbox(CodexSandboxObservation::new(&nested))
            .with_declared(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"));

        let resolved = resolve_workspaces(&requirements, &observations);

        let root = resolved.root(&"repo".into()).expect("resolved");
        assert_eq!(root.source, WorkspaceSource::CodexSandboxMeta);
        assert_ne!(root.root_uri, "file:///declared/repo");
    }

    #[test]
    fn explicit_uri_mismatch_diagnostic_carries_the_derived_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let nested = temp.path().join("project");
        fs::create_dir_all(&nested).expect("create dirs");
        fs::create_dir(nested.join(".git")).expect("create .git");

        let requirements = [
            requirement("repo").with_selection(WorkspaceSelection::ExplicitUri {
                uri: "file:///somewhere/else".into(),
            }),
        ];
        let observations = WorkspaceObservationSet::new()
            .with_codex_sandbox(CodexSandboxObservation::new(&nested));

        let resolved = resolve_workspaces(&requirements, &observations);

        assert!(resolved.root(&"repo".into()).is_none());
        let diagnostic = resolved
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.requirement.as_ref() == Some(&"repo".into()))
            .expect("mismatch diagnostic");
        let canonical = nested.canonicalize().expect("canonicalize");
        assert_eq!(
            diagnostic.roots,
            vec![format!("file://{}", canonical.display())],
            "diagnostic carries the derived candidate for tooling"
        );
    }
}
