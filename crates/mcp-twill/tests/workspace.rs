//! RFC 0007 integration acceptance tests: workspace resolution wired into
//! Twill planning, help, resources, dry-run output, and diagnostics.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use mcp_twill::{
    ArgSpec, CommandOutput, CommandRegistry, CommandSpec, FrameworkError, HelpRequest, HelpTopic,
    PermissionSpec, ResponseEnvelope, ResponseProfile, RunRequest, WorkspaceDecl,
};
use mcp_workspace_resolver::{McpRoot, McpRootsObservation, resolve_workspaces};
use serde_json::json;

fn request(command: &str, args: serde_json::Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args).unwrap_or_else(|_| BTreeMap::new()),
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    }
}

fn files_read_spec() -> CommandSpec {
    CommandSpec::new(
        ["files", "read"],
        "Read file",
        "Reads a file inside the repository workspace.",
    )
    .with_arg(ArgSpec::path("path", "File to read", "repo"))
    .with_permission(PermissionSpec::read("repo", "Reads repository files"))
}

fn registry() -> CommandRegistry {
    counted_registry(Arc::new(AtomicUsize::new(0)))
}

fn counted_registry(dispatches: Arc<AtomicUsize>) -> CommandRegistry {
    CommandRegistry::new("workspace-test", "Workspace integration test server")
        .declare_workspace(
            WorkspaceDecl::file("repo", "file:///workspace/repo")
                .with_description("Repository root"),
        )
        .register(files_read_spec(), move |_context| {
            let dispatches = dispatches.clone();
            async move {
                dispatches.fetch_add(1, Ordering::SeqCst);
                Ok(CommandOutput::structured(json!({ "content": "ok" })))
            }
        })
}

// Acceptance: a declared workspace fallback is visible in dry-run output.
#[tokio::test]
async fn declared_workspace_root_appears_in_dry_run_plan() {
    let mut run = request(
        "files read $args.path",
        json!({"path": "file:///workspace/repo/src/lib.rs"}),
    );
    run.dry_run = true;

    let response = registry().run(run).await.unwrap();
    assert!(response.dry_run);
    let roots = &response.plan.workspace_roots;
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, "repo");
    assert_eq!(roots[0].root_uri, "file:///workspace/repo");
    assert_eq!(roots[0].source, "declared");
    assert_eq!(roots[0].selection_reason, json!("declared_observation"));
}

// Acceptance: the declared root is visible in mismatch diagnostics.
#[test]
fn workspace_mismatch_names_selected_root_and_path() {
    let error = registry()
        .build_plan(&request(
            "files read $args.path",
            json!({"path": "/etc/passwd"}),
        ))
        .unwrap_err();

    let FrameworkError::WorkspaceMismatch {
        argument,
        workspace,
        selected_root,
        path,
        ..
    } = &error
    else {
        panic!("expected WorkspaceMismatch, got {error:?}");
    };
    assert_eq!(argument, "path");
    assert_eq!(workspace, "repo");
    assert_eq!(selected_root.as_deref(), Some("file:///workspace/repo"));
    assert_eq!(path.as_deref(), Some("/etc/passwd"));

    let envelope = ResponseEnvelope::framework_error(error.clone(), None, None);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(
        value["error"]["details"]["selectedRoot"],
        json!("file:///workspace/repo")
    );
    assert_eq!(value["error"]["details"]["path"], json!("/etc/passwd"));
}

// Acceptance: path traversal outside the selected root is rejected before
// dispatch.
#[tokio::test]
async fn path_traversal_outside_selected_root_is_rejected_before_dispatch() {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let registry = counted_registry(dispatches.clone());

    let traversal = registry
        .run(request(
            "files read $args.path",
            json!({"path": "file:///workspace/repo/../secrets.txt"}),
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        traversal,
        FrameworkError::WorkspaceMismatch { .. }
    ));

    // POSIX containment is case-sensitive: a differently-cased prefix is a
    // different directory.
    let cased = registry
        .run(request(
            "files read $args.path",
            json!({"path": "file:///Workspace/repo/src/lib.rs"}),
        ))
        .await
        .unwrap_err();
    assert!(matches!(cased, FrameworkError::WorkspaceMismatch { .. }));

    assert_eq!(dispatches.load(Ordering::SeqCst), 0);

    let allowed = registry
        .run(request(
            "files read $args.path",
            json!({"path": "file:///workspace/repo/src/lib.rs"}),
        ))
        .await
        .unwrap();
    assert!(allowed.output.is_some());
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);
}

// Acceptance: help and resources show the workspace requirement for path
// arguments.
#[test]
fn help_and_resources_show_workspace_requirement_for_path_args() {
    let registry = registry();

    let help = registry.help(HelpRequest {
        command: Some("files read".to_string()),
        topic: Some(HelpTopic::Arguments),
        detail: None,
    });
    assert!(
        help.text.contains("workspace `repo`"),
        "arguments help must name the workspace requirement: {}",
        help.text
    );

    let resource = registry
        .resource_text("cli://commands/files/read")
        .expect("command resource renders");
    assert!(
        resource.contains("workspace `repo`"),
        "command resource must name the workspace requirement: {resource}"
    );
}

// Acceptance: dry runs show selected root, source, and selection reason.
#[tokio::test]
async fn dry_run_envelope_shows_selected_root_source_and_reason() {
    let mut run = request(
        "files read $args.path",
        json!({"path": "file:///workspace/repo/src/lib.rs"}),
    );
    run.dry_run = true;

    let response = registry().run(run).await.unwrap();
    let envelope = ResponseEnvelope::success(response, ResponseProfile::Debug);
    let value = serde_json::to_value(&envelope).unwrap();
    let root = &value["plan"]["workspaceRoots"][0];
    assert_eq!(root["id"], json!("repo"));
    assert_eq!(root["rootUri"], json!("file:///workspace/repo"));
    assert_eq!(root["source"], json!("declared"));
    assert_eq!(root["selectionReason"], json!("declared_observation"));
}

// The invocation fingerprint binds approvals to the selected roots: the same
// request planned against a different selected root fingerprints differently.
#[test]
fn fingerprint_changes_when_selected_root_changes() {
    let registry = registry();
    let run = request(
        "files read $args.path",
        json!({"path": "file:///workspace/repo/src/lib.rs"}),
    );

    let declared_plan = registry.build_plan(&run).unwrap();

    // A client root named `repo` at the parent directory still contains the
    // path but selects a different root.
    let observations = registry
        .declared_observations()
        .with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace").with_name("repo"),
        ]));
    let resolved = resolve_workspaces(&registry.workspace_requirements(), &observations);
    let client_plan = registry
        .build_plan_with_workspaces(&run, &resolved)
        .unwrap();

    assert_eq!(client_plan.workspace_roots[0].root_uri, "file:///workspace");
    assert_ne!(
        declared_plan.invocation_fingerprint,
        client_plan.invocation_fingerprint
    );
}
