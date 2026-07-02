//! rmcp conversion tests (feature-gated).
#![cfg(feature = "rmcp")]

use mcp_workspace_resolver::{
    McpRoot, McpRootsObservation, WorkspaceObservationSet, WorkspaceRequirement, WorkspaceSource,
    resolve_workspaces,
};

#[test]
fn rmcp_root_converts_to_mcp_root() {
    let root = rmcp::model::Root::new("file:///workspace/project").with_name("repo");

    let converted = McpRoot::from(root);
    assert_eq!(converted.uri, "file:///workspace/project");
    assert_eq!(converted.name.as_deref(), Some("repo"));
}

#[test]
fn rmcp_list_roots_result_converts_and_resolves() {
    let result = rmcp::model::ListRootsResult::new(vec![
        rmcp::model::Root::new("file:///workspace/code").with_name("repo"),
        rmcp::model::Root::new("file:///workspace/docs"),
    ]);

    let observation = McpRootsObservation::from(result);
    assert_eq!(observation.roots.len(), 2);
    assert_eq!(observation.roots[1].name, None);

    let requirements = [WorkspaceRequirement::new("repo")];
    let observations = WorkspaceObservationSet::new().with_mcp_roots(observation);
    let resolved = resolve_workspaces(&requirements, &observations);

    let root = resolved.root(&"repo".into()).expect("resolved");
    assert_eq!(root.root_uri, "file:///workspace/code");
    assert_eq!(root.source, WorkspaceSource::McpRoots);
}
