//! RFC 0007 integration acceptance tests: workspace resolution wired into
//! Twill planning, help, resources, dry-run output, and diagnostics.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use mcp_twill::{
    ArgSpec, CliMcpServer, CliMcpServerConfig, CommandOutput, CommandRegistry, CommandSpec,
    ConversationIdentity, ConversationIdentityCompatibility, FrameworkError, HelpRequest,
    HelpTopic, HostWorkspaceRoot, HostWorkspaceRootsObservation, InvocationContext, PermissionSpec,
    PlanWorkspaceRoot, PreResolvedWorkspaceProblem, ResponseEnvelope, ResponseProfile, RunRequest,
    WorkspaceDecl, WorkspaceMetadataCompatibility,
};
use mcp_workspace_resolver::{
    McpRoot, McpRootsObservation, ResolvedWorkspaceRoot, ResolvedWorkspaceSet,
    WorkspaceCapabilities, WorkspaceSelectionReason, WorkspaceSource, resolve_workspaces,
};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientRequest, Meta, Request, ServerResult},
};
use serde_json::{Value, json};

#[derive(Default)]
struct TestClient;

impl ClientHandler for TestClient {}

fn json_object<T: serde::Serialize>(value: T) -> serde_json::Map<String, Value> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .unwrap()
        .clone()
}

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
        .register(
            export_spec(),
            |context: mcp_twill::CommandContext| async move {
                let root = context
                    .workspace_root("repo")
                    .expect("declared workspace resolves at plan time")
                    .clone();
                Ok(CommandOutput::structured(json!({
                    "rootUri": root.root_uri,
                    "source": root.source,
                    "path": root.path().unwrap(),
                })))
            },
        )
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
    let client_plan = registry
        .build_plan_with_workspaces(&run, &resolved)
        .unwrap();

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

// RFC 0009: required and optional declarations are semantic sets even when a
// hand-built CommandSpec bypasses the additive convenience APIs.
#[test]
fn duplicate_workspace_declarations_are_canonicalized_at_registration() {
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

    registry.validate_workspaces().unwrap();
    let registered = registry.command_specs().next().unwrap();
    assert_eq!(registered.workspaces, ["repo"]);
}

// Review regression: PlanWorkspaceRoot::path() preserves non-local path
// shapes instead of flattening everything to a rooted local path.
#[test]
fn plan_workspace_root_path_preserves_path_shapes() {
    let root = |uri: &str| PlanWorkspaceRoot {
        id: "repo".to_string(),
        root_uri: uri.to_string(),
        source: "declared".to_string(),
        source_issuer: None,
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

fn ambient_registry() -> CommandRegistry {
    CommandRegistry::new("ambient-workspace", "RFC 0009 ambient workspace server")
        .declare_workspace(
            WorkspaceDecl::file("project", "file:///declared/project")
                .with_description("Invocation project"),
        )
        .register(
            CommandSpec::new(
                ["artifacts", "export"],
                "Export artifact",
                "Requires the invocation project.",
            )
            .uses_workspace("project"),
            |context: mcp_twill::CommandContext| async move {
                let root = context.workspace_root("project").unwrap();
                Ok(CommandOutput::structured(json!({
                    "rootUri": root.root_uri,
                    "source": root.source,
                    "sourceIssuer": root.source_issuer,
                })))
            },
        )
        .register(
            CommandSpec::new(
                ["tabs", "list"],
                "List tabs",
                "Optionally observes the invocation project.",
            )
            .uses_optional_workspace("project"),
            |context: mcp_twill::CommandContext| async move {
                let root = context.workspace_root("project");
                Ok(CommandOutput::structured(json!({
                    "hasWorkspace": root.is_some(),
                    "rootUri": root.map(|root| root.root_uri.clone()),
                    "source": root.map(|root| root.source.clone()),
                    "sourceIssuer": root.and_then(|root| root.source_issuer.clone()),
                })))
            },
        )
        .register(
            CommandSpec::new(
                ["server", "ping"],
                "Ping",
                "Does not consume workspace context.",
            ),
            |_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) },
        )
}

fn host_context(roots: impl IntoIterator<Item = HostWorkspaceRoot>) -> InvocationContext {
    InvocationContext::new().with_host_workspace_roots(HostWorkspaceRootsObservation::new(roots))
}

#[tokio::test]
async fn direct_host_root_reaches_required_and_optional_handlers_with_provenance() {
    let context =
        host_context([
            HostWorkspaceRoot::new("com.example.editor", "file:///host/project").unwrap(),
        ]);

    let required = ambient_registry()
        .run_with_context(request("artifacts export", json!({})), context.clone())
        .await
        .unwrap();
    assert_eq!(required.plan.workspace_roots.len(), 1);
    assert_eq!(required.plan.workspace_roots[0].source, "trusted_host");
    assert_eq!(
        required.plan.workspace_roots[0].source_issuer.as_deref(),
        Some("com.example.editor")
    );
    assert_eq!(
        required.output.unwrap().structured.unwrap()["sourceIssuer"],
        "com.example.editor"
    );

    let optional = ambient_registry()
        .run_with_context(request("tabs list", json!({})), context)
        .await
        .unwrap();
    assert_eq!(optional.plan.workspace_roots.len(), 1);
    assert_eq!(
        optional.output.unwrap().structured.unwrap()["hasWorkspace"],
        true
    );
}

#[tokio::test]
async fn optional_workspace_absence_is_valid_and_required_absence_fails() {
    let empty = host_context([]);
    let optional = ambient_registry()
        .run_with_context(request("tabs list", json!({})), empty.clone())
        .await
        .unwrap();
    assert!(optional.plan.workspace_roots.is_empty());
    assert_eq!(
        optional.output.unwrap().structured.unwrap()["hasWorkspace"],
        false
    );

    let required = ambient_registry()
        .run_with_context(request("artifacts export", json!({})), empty)
        .await
        .unwrap_err();
    assert!(matches!(
        required,
        FrameworkError::WorkspaceUnresolved { .. }
    ));
}

#[test]
fn optional_workspace_unmatched_and_unsupported_observations_remain_absent() {
    let registry = ambient_registry();
    for roots in [
        vec![
            McpRoot::new("file:///other").with_name("other"),
            McpRoot::new("file:///another").with_name("another"),
        ],
        vec![McpRoot::new("https://example.com/project").with_name("project")],
    ] {
        let observations = registry
            .declared_observations()
            .with_mcp_roots(McpRootsObservation::new(roots));
        let resolved = registry.resolve_workspaces(&observations);
        let plan = registry
            .build_plan_with_workspaces(&request("tabs list", json!({})), &resolved)
            .unwrap();
        assert!(plan.workspace_roots.is_empty());
    }
}

#[test]
fn optional_declaration_becomes_required_when_a_path_argument_uses_it() {
    let registry = CommandRegistry::new("path-dominates", "Path workspace test")
        .declare_workspace(WorkspaceDecl::file("project", "file:///declared/project"))
        .register(
            CommandSpec::new(["files", "inspect"], "Inspect", "Inspects a path")
                .uses_optional_workspace("project")
                .with_arg(ArgSpec::path("path", "Path to inspect", "project")),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    let observations = registry
        .declared_observations()
        .with_mcp_roots(McpRootsObservation::new(Vec::new()));
    let resolved = registry.resolve_workspaces(&observations);
    assert!(matches!(
        registry.build_plan_with_workspaces(
            &request(
                "files inspect $args.path",
                json!({"path": "file:///declared/project/file.txt"}),
            ),
            &resolved,
        ),
        Err(FrameworkError::WorkspaceMismatch { .. })
    ));
}

#[test]
fn optional_presence_and_provenance_bind_the_invocation_fingerprint() {
    let registry = ambient_registry();
    let run = request("tabs list", json!({}));
    let absent = registry
        .build_plan_with_context(&run, &host_context([]))
        .unwrap();
    let first = registry
        .build_plan_with_context(
            &run,
            &host_context([
                HostWorkspaceRoot::new("com.example.first", "file:///host/project").unwrap(),
            ]),
        )
        .unwrap();
    let second = registry
        .build_plan_with_context(
            &run,
            &host_context([
                HostWorkspaceRoot::new("com.example.second", "file:///host/project").unwrap(),
            ]),
        )
        .unwrap();

    assert_ne!(absent.invocation_fingerprint, first.invocation_fingerprint);
    assert_ne!(first.invocation_fingerprint, second.invocation_fingerprint);
}

#[test]
fn declaration_modes_are_canonical_and_mutually_exclusive() {
    let registry = CommandRegistry::new("modes", "Workspace mode validation")
        .declare_workspace(WorkspaceDecl::file("zeta", "file:///zeta"))
        .declare_workspace(WorkspaceDecl::file("alpha", "file:///alpha"))
        .register(
            CommandSpec::new(["mode", "ok"], "Modes", "Canonical modes")
                .uses_optional_workspace("zeta")
                .uses_optional_workspace("zeta")
                .uses_workspace("alpha")
                .uses_workspace("alpha"),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    let spec = registry.command_specs().next().unwrap();
    assert_eq!(spec.workspaces, ["alpha"]);
    assert_eq!(spec.optional_workspaces, ["zeta"]);

    let invalid = CommandRegistry::new("modes", "Workspace mode validation")
        .declare_workspace(WorkspaceDecl::file("project", "file:///project"))
        .register(
            CommandSpec::new(["mode", "bad"], "Modes", "Conflicting modes")
                .uses_workspace("project")
                .uses_optional_workspace("project"),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    assert!(invalid.validate_workspaces().is_err());
}

fn resolved_root(id: &str, uri: &str) -> ResolvedWorkspaceRoot {
    ResolvedWorkspaceRoot {
        id: id.into(),
        root_uri: uri.to_string(),
        source: WorkspaceSource::TrustedHost,
        source_issuer: Some("com.example.host".to_string()),
        selection_reason: WorkspaceSelectionReason::MatchedByName,
        capabilities: WorkspaceCapabilities::default(),
    }
}

#[test]
fn pre_resolved_workspace_input_is_validated_before_planning() {
    let registry = ambient_registry();
    let run = request("tabs list", json!({}));
    let duplicate = ResolvedWorkspaceSet {
        roots: vec![
            resolved_root("project", "file:///one"),
            resolved_root("project", "file:///two"),
        ],
        diagnostics: Vec::new(),
    };
    assert!(matches!(
        registry.build_plan_with_workspaces(&run, &duplicate),
        Err(FrameworkError::InvalidPreResolvedWorkspaceSet {
            workspace: Some(workspace),
            reason: PreResolvedWorkspaceProblem::DuplicateWorkspace,
        }) if workspace == "project"
    ));

    let unknown = ResolvedWorkspaceSet {
        roots: vec![resolved_root(
            "caller-secret-name",
            "file:///private/secret",
        )],
        diagnostics: Vec::new(),
    };
    let error = registry
        .build_plan_with_workspaces(&run, &unknown)
        .unwrap_err();
    assert!(matches!(
        error,
        FrameworkError::InvalidPreResolvedWorkspaceSet {
            workspace: None,
            reason: PreResolvedWorkspaceProblem::UnknownWorkspace,
        }
    ));
    let envelope =
        serde_json::to_string(&ResponseEnvelope::framework_error(error, None, None)).unwrap();
    assert!(!envelope.contains("caller-secret-name"));
    assert!(!envelope.contains("private/secret"));

    let conflict = registry
        .build_plan_with_workspaces_and_context(
            &run,
            &ResolvedWorkspaceSet {
                roots: vec![resolved_root("project", "file:///project")],
                diagnostics: Vec::new(),
            },
            &host_context([]),
        )
        .unwrap_err();
    assert_eq!(conflict, FrameworkError::ConflictingWorkspaceInputs);

    let identity_only = InvocationContext::new().with_conversation_identity(
        ConversationIdentity::new("com.example.host", "conversation").unwrap(),
    );
    registry
        .build_plan_with_workspaces_and_context(
            &run,
            &ResolvedWorkspaceSet {
                roots: vec![resolved_root("project", "file:///project")],
                diagnostics: Vec::new(),
            },
            &identity_only,
        )
        .unwrap();
}

#[tokio::test]
async fn pre_resolved_run_rejects_raw_host_context_before_dispatch() {
    let error = ambient_registry()
        .run_in_lane_with_workspaces_and_context(
            request("tabs list", json!({})),
            "run",
            mcp_twill::EffectLane::Primary,
            "run",
            &ResolvedWorkspaceSet {
                roots: vec![resolved_root("project", "file:///project")],
                diagnostics: Vec::new(),
            },
            &host_context([]),
        )
        .await
        .unwrap_err();
    assert_eq!(error, FrameworkError::ConflictingWorkspaceInputs);
}

#[test]
fn unmatched_host_observations_and_invocation_context_debug_are_private() {
    let context = host_context([
        HostWorkspaceRoot::named(
            "com.example.secret",
            "private-name",
            "file:///private/raw-project",
        )
        .unwrap(),
        HostWorkspaceRoot::named(
            "com.example.secret",
            "another-private-name",
            "file:///private/other-project",
        )
        .unwrap(),
    ]);
    let debug = format!("{context:?}");
    assert!(!debug.contains("com.example.secret"));
    assert!(!debug.contains("private-name"));
    assert!(!debug.contains("raw-project"));

    let error = ambient_registry()
        .build_plan_with_context(&request("artifacts export", json!({})), &context)
        .unwrap_err();
    let envelope =
        serde_json::to_string(&ResponseEnvelope::framework_error(error, None, None)).unwrap();
    assert!(!envelope.contains("com.example.secret"));
    assert!(!envelope.contains("private-name"));
    assert!(!envelope.contains("raw-project"));
}

#[test]
fn host_root_constructors_keep_their_own_result_channel() {
    let unnamed: std::result::Result<HostWorkspaceRoot, mcp_twill::HostWorkspaceRootError> =
        HostWorkspaceRoot::new("com.example.host", "file:///workspace");
    let named: std::result::Result<HostWorkspaceRoot, mcp_twill::HostWorkspaceRootError> =
        HostWorkspaceRoot::named("com.example.host", "repo", "file:///workspace");
    assert!(unnamed.is_ok());
    assert!(named.is_ok());
}

#[test]
fn selected_plan_roots_are_sorted_by_workspace_id() {
    let registry = CommandRegistry::new("sorted", "Sorted roots")
        .declare_workspace(WorkspaceDecl::file("zeta", "file:///zeta"))
        .declare_workspace(WorkspaceDecl::file("alpha", "file:///alpha"))
        .register(
            CommandSpec::new(["roots", "show"], "Show roots", "Shows both roots")
                .uses_workspace("zeta")
                .uses_workspace("alpha"),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    let resolved = ResolvedWorkspaceSet {
        roots: vec![
            resolved_root("zeta", "file:///zeta"),
            resolved_root("alpha", "file:///alpha"),
        ],
        diagnostics: Vec::new(),
    };
    let plan = registry
        .build_plan_with_workspaces(&request("roots show", json!({})), &resolved)
        .unwrap();
    assert_eq!(
        plan.workspace_roots
            .iter()
            .map(|root| root.id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
}

#[test]
fn optional_workspace_projects_without_becoming_an_argument() {
    let registry = ambient_registry();
    let spec = registry
        .command_specs()
        .find(|spec| spec.name() == "tabs list")
        .unwrap();
    let operation = registry
        .operation_specs()
        .into_iter()
        .find(|operation| operation.id == "tabs.list")
        .unwrap();
    assert!(spec.workspaces.is_empty());
    assert_eq!(spec.optional_workspaces, ["project"]);
    assert_eq!(operation.optional_workspaces, ["project"]);

    let help = registry.help(HelpRequest {
        command: Some("tabs list".to_string()),
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("(optional, supplied by host)"));
    let schema = registry.arg_schema(spec);
    assert!(schema["properties"].as_object().unwrap().is_empty());
}

#[test]
fn legacy_workspace_json_defaults_additive_fields_to_absent() {
    let root: PlanWorkspaceRoot = serde_json::from_value(json!({
        "id": "project",
        "rootUri": "file:///project",
        "source": "declared",
        "selectionReason": "declared_fallback"
    }))
    .unwrap();
    assert_eq!(root.source_issuer, None);
    assert!(
        serde_json::to_value(root)
            .unwrap()
            .get("sourceIssuer")
            .is_none()
    );

    let spec = CommandSpec::new(["legacy", "command"], "Legacy", "Legacy command");
    let mut value = serde_json::to_value(&spec).unwrap();
    value.as_object_mut().unwrap().remove("optionalWorkspaces");
    let decoded: CommandSpec = serde_json::from_value(value).unwrap();
    assert!(decoded.optional_workspaces.is_empty());
    assert!(
        serde_json::to_value(decoded)
            .unwrap()
            .get("optionalWorkspaces")
            .is_none()
    );
}

#[test]
fn workspace_and_conversation_compatibility_policies_are_independent_setters() {
    let config = CliMcpServerConfig::default()
        .with_workspace_metadata_compatibility(
            WorkspaceMetadataCompatibility::TrustedCodexSandboxState,
        )
        .with_conversation_identity_compatibility(
            ConversationIdentityCompatibility::TrustedCodexThreadId,
        )
        .with_workspace_metadata_compatibility(WorkspaceMetadataCompatibility::Disabled);
    assert_eq!(
        config.workspace_metadata_compatibility,
        WorkspaceMetadataCompatibility::Disabled
    );
    assert_eq!(
        config.conversation_identity_compatibility,
        ConversationIdentityCompatibility::TrustedCodexThreadId
    );
}

async fn call_mcp_workspace(
    registry: CommandRegistry,
    config: CliMcpServerConfig,
    meta: Meta,
    run: RunRequest,
) -> anyhow::Result<rmcp::model::CallToolResult> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server = CliMcpServer::with_config(registry, config)?;
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let mut params = CallToolRequestParams::new("run").with_arguments(json_object(run));
    params.meta = Some(meta);
    let result = client.call_tool(params).await?;
    client.cancel().await?;
    server_handle.await??;
    Ok(result)
}

fn codex_meta(value: Value) -> Meta {
    Meta(
        [("codex/sandbox-state-meta".to_string(), value)]
            .into_iter()
            .collect(),
    )
}

#[tokio::test]
async fn raw_tools_call_codex_metadata_is_default_disabled_and_explicitly_trusted()
-> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let value = json!({
        "sandboxCwd": cwd.to_string_lossy(),
        "permissionProfile": "workspace-write",
    });

    let disabled = call_mcp_workspace(
        ambient_registry(),
        CliMcpServerConfig::default(),
        codex_meta(value.clone()),
        request("artifacts export", json!({})),
    )
    .await?;
    assert_eq!(disabled.is_error, Some(false));
    assert_eq!(
        disabled.structured_content.unwrap()["output"]["structured"]["source"],
        "declared"
    );

    let enabled = call_mcp_workspace(
        ambient_registry(),
        CliMcpServerConfig::default().with_workspace_metadata_compatibility(
            WorkspaceMetadataCompatibility::TrustedCodexSandboxState,
        ),
        codex_meta(value),
        request("artifacts export", json!({})),
    )
    .await?;
    assert_eq!(enabled.is_error, Some(false));
    assert_eq!(
        enabled.structured_content.unwrap()["output"]["structured"]["source"],
        "codex_sandbox_meta"
    );
    Ok(())
}

#[tokio::test]
async fn malformed_enabled_codex_metadata_fails_before_command_planning_and_redacts_values()
-> anyhow::Result<()> {
    let result = call_mcp_workspace(
        ambient_registry(),
        CliMcpServerConfig::default().with_workspace_metadata_compatibility(
            WorkspaceMetadataCompatibility::TrustedCodexSandboxState,
        ),
        codex_meta(json!({ "sandboxCwd": { "secret": "/private/raw-path" } })),
        request("server ping", json!({})),
    )
    .await?;
    assert_eq!(result.is_error, Some(true));
    let value = result.structured_content.unwrap();
    assert_eq!(value["error"]["code"], "invalid_request_context");
    assert_eq!(
        value["error"]["details"],
        json!({
            "key": "codex/sandbox-state-meta",
            "field": "sandboxCwd",
            "reason": "invalid_sandbox_cwd",
        })
    );
    let rendered = serde_json::to_string(&value)?;
    assert!(!rendered.contains("private/raw-path"));
    assert!(!rendered.contains("secret"));
    Ok(())
}

#[tokio::test]
async fn ordinary_and_task_execution_share_workspace_context_and_fingerprint() -> anyhow::Result<()>
{
    let seen = Arc::new(Mutex::new(Vec::new()));
    let registry = CommandRegistry::new("task-workspace", "Task workspace parity")
        .declare_workspace(WorkspaceDecl::file("project", "file:///declared/project"))
        .register(
            CommandSpec::new(["workspace", "show"], "Show workspace", "Shows workspace")
                .uses_workspace("project"),
            {
                let seen = seen.clone();
                move |context: mcp_twill::CommandContext| {
                    let seen = seen.clone();
                    async move {
                        let root = context.workspace_root("project").unwrap();
                        seen.lock().unwrap().push((
                            root.root_uri.clone(),
                            context.plan.invocation_fingerprint.clone(),
                        ));
                        Ok(CommandOutput::structured(json!({ "ok": true })))
                    }
                }
            },
        );
    let config = CliMcpServerConfig::default().with_workspace_metadata_compatibility(
        WorkspaceMetadataCompatibility::TrustedCodexSandboxState,
    );
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server = CliMcpServer::with_config(registry, config)?;
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let meta = codex_meta(json!({
        "sandboxCwd": std::env::current_dir()?.to_string_lossy(),
    }));

    let mut ordinary = CallToolRequestParams::new("run")
        .with_arguments(json_object(request("workspace show", json!({}))));
    ordinary.meta = Some(meta.clone());
    assert_eq!(client.call_tool(ordinary).await?.is_error, Some(false));

    let mut task = CallToolRequestParams::new("run")
        .with_arguments(json_object(request("workspace show", json!({}))))
        .with_task(serde_json::Map::new());
    task.meta = Some(meta);
    assert!(matches!(
        client
            .send_request(ClientRequest::CallToolRequest(Request::new(task)))
            .await?,
        ServerResult::CreateTaskResult(_)
    ));
    for _ in 0..40 {
        if seen.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    {
        let observations = seen.lock().unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0], observations[1]);
    }

    client.cancel().await?;
    server_handle.await??;
    Ok(())
}
