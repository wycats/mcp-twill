//! Serde round-trips for the public vocabulary types.

use mcp_workspace_resolver::{
    CodexSandboxObservation, DeclaredWorkspaceRoot, McpRoot, McpRootsObservation,
    ResolvedWorkspaceSet, WorkspaceDiagnostic, WorkspaceObservationSet, WorkspaceRequirement,
    WorkspaceSelection, resolve_workspaces,
};

#[test]
fn requirement_round_trips() {
    let requirement = WorkspaceRequirement::new("repo")
        .with_display_name("Repository")
        .with_alias("source")
        .with_selection(WorkspaceSelection::ExplicitUri {
            uri: "file:///workspace/repo".into(),
        })
        .with_fallback(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"));

    let json = serde_json::to_string(&requirement).expect("serialize");
    let back: WorkspaceRequirement = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, requirement);
}

#[test]
fn observation_set_round_trips_with_private_fields() {
    let observations = WorkspaceObservationSet::new()
        .with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/repo").with_name("repo"),
        ]))
        .with_codex_sandbox(CodexSandboxObservation::new("/sandbox/cwd"))
        .with_declared(DeclaredWorkspaceRoot::new("repo", "file:///declared/repo"));

    let json = serde_json::to_string(&observations).expect("serialize");
    let back: WorkspaceObservationSet = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, observations);
}

#[test]
fn resolved_set_serializes_with_stable_diagnostic_codes() {
    let requirements = [WorkspaceRequirement::new("repo")];
    let observations =
        WorkspaceObservationSet::new().with_mcp_roots(McpRootsObservation::default());

    let resolved = resolve_workspaces(&requirements, &observations);
    let json = serde_json::to_value(&resolved).expect("serialize");
    assert_eq!(
        json["diagnostics"][0]["code"],
        "unresolved_workspace_requirement"
    );

    let back: ResolvedWorkspaceSet = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back, resolved);
}

#[test]
fn diagnostic_round_trips() {
    let diagnostic = WorkspaceDiagnostic::ambiguous(
        "repo".into(),
        "multiple roots match",
        vec!["file:///a".into(), "file:///b".into()],
    );

    let json = serde_json::to_string(&diagnostic).expect("serialize");
    let back: WorkspaceDiagnostic = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, diagnostic);
}
