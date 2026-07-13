//! Serde round-trips for the public vocabulary types.

use mcp_workspace_resolver::{
    CodexSandboxObservation, DeclaredWorkspaceRoot, HostWorkspaceRoot, HostWorkspaceRootError,
    HostWorkspaceRootsObservation, McpRoot, McpRootsObservation, ResolvedWorkspaceSet,
    WorkspaceDiagnostic, WorkspaceObservationSet, WorkspaceRequirement, WorkspaceSelection,
    resolve_workspaces,
};
use schemars::schema_for;

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
fn legacy_resolved_root_defaults_source_issuer_to_absent() {
    let root: mcp_workspace_resolver::ResolvedWorkspaceRoot =
        serde_json::from_value(serde_json::json!({
            "id": "repo",
            "root_uri": "file:///repo",
            "source": "declared",
            "selection_reason": "declared_fallback",
            "capabilities": { "read": true, "write": false }
        }))
        .unwrap();
    assert_eq!(root.source_issuer, None);
    assert!(
        serde_json::to_value(root)
            .unwrap()
            .get("source_issuer")
            .is_none()
    );
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

#[test]
fn trusted_host_root_wire_form_round_trips_and_schema_is_closed() {
    let root =
        HostWorkspaceRoot::named("com.example.editor", "repo", "file:///workspace/repo").unwrap();
    let json = serde_json::to_value(&root).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "issuer": "com.example.editor",
            "name": "repo",
            "uri": "file:///workspace/repo"
        })
    );
    assert_eq!(
        serde_json::from_value::<HostWorkspaceRoot>(json).unwrap(),
        root
    );
    assert!(
        serde_json::from_value::<HostWorkspaceRoot>(serde_json::json!({
            "issuer": "com.example.editor",
            "uri": "file:///workspace/repo",
            "extra": true
        }))
        .is_err()
    );

    let schema = serde_json::to_value(schema_for!(HostWorkspaceRoot)).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["issuer"]["type"], "string");
    assert_eq!(schema["properties"]["uri"]["type"], "string");

    let collection_schema =
        serde_json::to_value(schema_for!(HostWorkspaceRootsObservation)).unwrap();
    assert_eq!(collection_schema["type"], "array");
    assert_eq!(
        collection_schema["items"]["$ref"],
        "#/$defs/HostWorkspaceRoot"
    );
}

#[test]
fn trusted_host_observation_preserves_absent_and_present_empty() {
    let absent = WorkspaceObservationSet::new();
    let present =
        WorkspaceObservationSet::new().with_host_roots(HostWorkspaceRootsObservation::default());
    assert_ne!(absent, present);
    assert_eq!(
        serde_json::from_value::<WorkspaceObservationSet>(serde_json::to_value(&present).unwrap())
            .unwrap(),
        present
    );
}

#[test]
fn trusted_host_root_validation_and_debug_are_redacted() {
    assert_eq!(
        HostWorkspaceRoot::new("Example.COM", "file:///workspace/repo").unwrap_err(),
        HostWorkspaceRootError::InvalidIssuer
    );
    assert_eq!(
        HostWorkspaceRoot::named("com.example.editor", "", "file:///workspace/repo").unwrap_err(),
        HostWorkspaceRootError::InvalidName
    );
    assert_eq!(
        HostWorkspaceRoot::new("com.example.editor", "relative/path").unwrap_err(),
        HostWorkspaceRootError::InvalidUri
    );

    let roots = HostWorkspaceRootsObservation::new([HostWorkspaceRoot::new(
        "com.example.editor",
        "file:///private/project",
    )
    .unwrap()]);
    let debug = format!("{roots:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("com.example.editor"));
    assert!(!debug.contains("private"));
    assert!(!debug.contains('1'));
}
