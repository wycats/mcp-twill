//! RFC 0007 integration acceptance tests: workspace resolution wired into
//! Twill planning, help, resources, dry-run output, and diagnostics.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use mcp_twill::{
    ArgSpec, CommandOutput, CommandRegistry, CommandSpec, FrameworkError, HelpRequest, HelpTopic,
    PermissionSpec, PlanWorkspaceRoot, ResponseEnvelope, ResponseProfile, RunRequest,
    WorkspaceDecl,
};
use mcp_workspace_resolver::{McpRoot, McpRootsObservation, resolve_workspaces};
use serde_json::json;

fn request(command: &str, args: serde_json::Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args).expect("test args must be a JSON object of values"),
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

// A single client root must not satisfy unrelated workspace requirements
// when several workspaces are declared.
#[test]
fn single_client_root_does_not_satisfy_unrelated_workspaces() {
    let registry = CommandRegistry::new("multi-workspace", "Multi-workspace server")
        .declare_workspace(WorkspaceDecl::file("repo", "file:///workspace/repo"))
        .declare_workspace(WorkspaceDecl::file("secrets", "file:///workspace/secrets"))
        .register(
            CommandSpec::new(["secrets", "read"], "Read secret", "Reads a secret file.")
                .with_arg(ArgSpec::path("path", "Secret to read", "secrets"))
                .with_permission(PermissionSpec::read("secrets", "Reads secrets")),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );

    let observations = registry
        .declared_observations()
        .with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///workspace/repo").with_name("repo"),
        ]));
    let resolved = resolve_workspaces(&registry.workspace_requirements(), &observations);

    // The lone `repo` client root must not resolve `secrets`, and the secrets
    // path must not be accepted under the repo root.
    let error = registry
        .build_plan_with_workspaces(
            &request(
                "secrets read $args.path",
                json!({"path": "file:///workspace/repo/leaked"}),
            ),
            &resolved,
        )
        .unwrap_err();
    assert!(
        matches!(error, FrameworkError::WorkspaceMismatch { .. }),
        "{error:?}"
    );
}

// Roots capability + failed roots/list must not widen access to declared roots.
// The adapter maps a failed list to an empty (authoritative) observation;
// planning with an empty roots observation must leave requirements unresolved.
#[test]
fn empty_roots_observation_blocks_declared_fallback() {
    let registry = registry();
    let observations = registry
        .declared_observations()
        .with_mcp_roots(McpRootsObservation::new(Vec::new()));
    let resolved = resolve_workspaces(&registry.workspace_requirements(), &observations);

    let error = registry
        .build_plan_with_workspaces(
            &request(
                "files read $args.path",
                json!({"path": "file:///workspace/repo/src/lib.rs"}),
            ),
            &resolved,
        )
        .unwrap_err();
    let FrameworkError::WorkspaceMismatch {
        selected_root,
        diagnostics,
        ..
    } = &error
    else {
        panic!("expected workspace mismatch, got {error:?}");
    };
    assert!(selected_root.is_none());
    assert!(
        !diagnostics.is_empty(),
        "resolver diagnostics explain the unresolved requirement"
    );
    assert!(
        error.to_string().contains("could not be resolved"),
        "resolution failure has its own message: {error}"
    );
}

// A non-file path argument surfaces the resolver's unsupported-scheme code.
#[test]
fn non_file_path_argument_gets_unsupported_scheme_diagnostic() {
    let registry = registry();
    let error = registry
        .build_plan(&request(
            "files read $args.path",
            json!({"path": "s3://bucket/object"}),
        ))
        .unwrap_err();
    let FrameworkError::WorkspaceMismatch { diagnostics, .. } = &error else {
        panic!("expected workspace mismatch, got {error:?}");
    };
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unsupported_root_scheme"),
        "{diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// RFC 0009: handler-visible workspace roots (`uses_workspace`)
// ---------------------------------------------------------------------------

fn export_spec() -> CommandSpec {
    CommandSpec::new(
        ["issues", "export"],
        "Export issues",
        "Writes an export under the repository root.",
    )
    .uses_workspace("repo")
    .with_permission(PermissionSpec::write("issues", "Writes an export file"))
}

fn declaring_registry() -> CommandRegistry {
    CommandRegistry::new("uses-workspace-test", "RFC 0009 test server")
        .declare_workspace(
            WorkspaceDecl::file("repo", "file:///workspace/repo")
                .with_description("Repository root"),
        )
        .register(export_spec(), |context: mcp_twill::CommandContext| async move {
            let root = context
                .workspace_root("repo")
                .expect("declared workspace resolves at plan time")
                .clone();
            Ok(CommandOutput::structured(json!({
                "rootUri": root.root_uri,
                "source": root.source,
                "path": root.path().unwrap(),
            })))
        })
}

// Acceptance: a command with `uses_workspace` and no path arguments plans
// against the declared fallback and the handler observes the root.
#[tokio::test]
async fn uses_workspace_resolves_declared_fallback_and_reaches_handler() {
    let registry = declaring_registry();
    let response = registry
        .run(request("issues export", json!({})))
        .await
        .unwrap();

    let roots = &response.plan.workspace_roots;
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, "repo");
    assert_eq!(roots[0].root_uri, "file:///workspace/repo");
    assert_eq!(roots[0].source, "declared");

    let structured = response.output.unwrap().structured.unwrap();
    assert_eq!(structured["rootUri"], json!("file:///workspace/repo"));
    assert_eq!(structured["source"], json!("declared"));
    assert_eq!(structured["path"], json!("/workspace/repo"));
}

// Acceptance: a client root named `repo` outranks the declared fallback and
// the plan records the source.
#[test]
fn uses_workspace_prefers_client_root_over_declared_fallback() {
    let registry = declaring_registry();
    let observations = registry
        .declared_observations()
        .with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///clients/repo").with_name("repo"),
        ]));
    let resolved = resolve_workspaces(&registry.workspace_requirements(), &observations);

    let plan = registry
        .build_plan_with_workspaces(&request("issues export", json!({})), &resolved)
        .unwrap();

    assert_eq!(plan.workspace_roots.len(), 1);
    assert_eq!(plan.workspace_roots[0].root_uri, "file:///clients/repo");
    assert_eq!(plan.workspace_roots[0].source, "mcp_roots");
}

// Acceptance: planning fails with WorkspaceUnresolved when no observation
// satisfies the declared requirement, carrying resolver diagnostics.
#[test]
fn uses_workspace_unresolved_fails_at_plan_time_with_diagnostics() {
    let registry = declaring_registry();
    // An empty authoritative roots observation blocks the declared fallback.
    let observations = registry
        .declared_observations()
        .with_mcp_roots(McpRootsObservation::new(Vec::new()));
    let resolved = resolve_workspaces(&registry.workspace_requirements(), &observations);

    let error = registry
        .build_plan_with_workspaces(&request("issues export", json!({})), &resolved)
        .unwrap_err();
    let FrameworkError::WorkspaceUnresolved {
        workspace,
        diagnostics,
    } = &error
    else {
        panic!("expected WorkspaceUnresolved, got {error:?}");
    };
    assert_eq!(workspace, "repo");
    assert!(
        !diagnostics.is_empty(),
        "resolver diagnostics explain the failure"
    );

    let envelope = ResponseEnvelope::framework_error(error.clone(), None, None);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(
        value["error"]["code"],
        json!("unresolved_workspace_requirement")
    );
}

// Acceptance: registration fails when `uses_workspace` names an undeclared
// workspace.
#[test]
fn uses_workspace_with_undeclared_name_fails_validation() {
    let registry = CommandRegistry::new("bad-workspace", "Undeclared workspace server").register(
        CommandSpec::new(["broken", "cmd"], "Broken", "Uses an undeclared workspace.")
            .uses_workspace("missing")
            .with_permission(PermissionSpec::read("stuff", "Reads stuff")),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );

    let error = registry.validate_workspaces().unwrap_err();
    assert!(
        error.to_string().contains("missing"),
        "names the undeclared workspace: {error}"
    );
    assert!(
        error.to_string().contains("broken cmd"),
        "names the declaring command: {error}"
    );
}

// Acceptance: two plans for the same call against different resolved roots
// produce different invocation fingerprints.
#[test]
fn uses_workspace_fingerprint_changes_with_resolved_root() {
    let registry = declaring_registry();
    let run = request("issues export", json!({}));

    let declared_plan = registry.build_plan(&run).unwrap();

    let observations = registry
        .declared_observations()
        .with_mcp_roots(McpRootsObservation::new(vec![
            McpRoot::new("file:///clients/repo").with_name("repo"),
        ]));
    let resolved = resolve_workspaces(&registry.workspace_requirements(), &observations);
    let client_plan = registry.build_plan_with_workspaces(&run, &resolved).unwrap();

    assert_ne!(
        declared_plan.invocation_fingerprint,
        client_plan.invocation_fingerprint
    );
}

// Acceptance: command help renders the Workspaces section and the catalog
// entry carries the workspaces list.
#[test]
fn uses_workspace_projects_into_help_and_catalog() {
    let registry = declaring_registry();

    let help = registry.help(mcp_twill::HelpRequest {
        command: Some("issues export".to_string()),
        topic: None,
        detail: None,
    });
    assert!(
        help.text.contains("Workspaces:"),
        "help renders the Workspaces section: {}",
        help.text
    );
    assert!(help.text.contains("repo"), "{}", help.text);

    let catalog = serde_json::to_value(registry.catalog()).unwrap();
    let operations = catalog["operations"].as_array().unwrap();
    let export = operations
        .iter()
        .find(|operation| operation["id"] == json!("issues.export"))
        .expect("export command in catalog");
    assert_eq!(export["workspaces"], json!(["repo"]));
}

// Acceptance: declaring a workspace and also binding a path argument to it
// resolves once and lists the root once.
#[tokio::test]
async fn uses_workspace_with_path_argument_lists_root_once() {
    let registry = CommandRegistry::new("combined", "Combined declaration server")
        .declare_workspace(
            WorkspaceDecl::file("repo", "file:///workspace/repo")
                .with_description("Repository root"),
        )
        .register(
            CommandSpec::new(
                ["files", "stage"],
                "Stage file",
                "Stages a file within the repository.",
            )
            .uses_workspace("repo")
            .with_arg(ArgSpec::path("path", "File to stage", "repo"))
            .with_permission(PermissionSpec::write("repo", "Stages repository files")),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );

    let plan = registry
        .build_plan(&request(
            "files stage $args.path",
            json!({"path": "file:///workspace/repo/src/lib.rs"}),
        ))
        .unwrap();

    let repo_roots: Vec<_> = plan
        .workspace_roots
        .iter()
        .filter(|root| root.id == "repo")
        .collect();
    assert_eq!(repo_roots.len(), 1, "{:?}", plan.workspace_roots);
}

// Review regression: declaring the same workspace twice on a command is a
// no-op at the API boundary, so projection never repeats the entry.
#[test]
fn uses_workspace_twice_dedupes_at_declaration() {
    let spec = CommandSpec::new(["dup", "cmd"], "Dup", "Declares repo twice.")
        .uses_workspace("repo")
        .uses_workspace("repo");
    assert_eq!(spec.workspaces, vec!["repo".to_string()]);
}

// Review regression: a hand-built CommandSpec with duplicate workspace
// entries (bypassing the deduping builder) fails validation.
#[test]
fn duplicate_workspace_declarations_fail_validation() {
    let mut spec = CommandSpec::new(["dup", "cmd"], "Dup", "Declares repo twice.")
        .with_permission(PermissionSpec::read("stuff", "Reads stuff"));
    spec.workspaces = vec!["repo".to_string(), "repo".to_string()];

    let registry = CommandRegistry::new("dup-workspace", "Duplicate workspace server")
        .declare_workspace(
            WorkspaceDecl::file("repo", "file:///workspace/repo")
                .with_description("Repository root"),
        )
        .register(spec, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });

    let error = registry.validate_workspaces().unwrap_err();
    assert!(
        error.to_string().contains("more than once"),
        "unexpected error: {error}"
    );
}

// Review regression: PlanWorkspaceRoot::path() preserves non-local path
// shapes instead of flattening everything to a rooted local path.
#[test]
fn plan_workspace_root_path_preserves_path_shapes() {
    let root = |uri: &str| PlanWorkspaceRoot {
        id: "repo".to_string(),
        root_uri: uri.to_string(),
        source: "declared".to_string(),
        selection_reason: json!(null),
    };

    assert_eq!(
        root("file:///workspace/repo").path().unwrap(),
        std::path::PathBuf::from("/workspace/repo")
    );
    assert_eq!(
        root("file://server/share").path().unwrap(),
        std::path::PathBuf::from("//server/share")
    );
    assert_eq!(
        root("relative/dir").path().unwrap(),
        std::path::PathBuf::from("relative/dir")
    );
}
