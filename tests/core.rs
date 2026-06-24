use mcp_twill::{
    ArgSpec, CommandExample, CommandOutput, CommandRegistry, CommandSpec, EffectLane, EffectSpec,
    FrameworkError, HelpRequest, OutputSpec, PermissionEffect, PermissionPolicy, PermissionSpec,
    RunRequest, WorkspaceDecl,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

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

fn registry() -> CommandRegistry {
    CommandRegistry::new("test", "test server")
        .declare_workspace(WorkspaceDecl::new("repo", "C:/repo"))
        .register(create_issue_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({
                "id": 1,
                "title": "Created",
                "body": "Body",
                "extra": "hidden"
            })))
        })
        .register(read_file_spec(), |_context| async {
            Ok(CommandOutput::text("file contents"))
        })
}

fn registry_reversed() -> CommandRegistry {
    CommandRegistry::new("test", "test server")
        .declare_workspace(WorkspaceDecl::new("repo", "C:/repo"))
        .register(read_file_spec(), |_context| async {
            Ok(CommandOutput::text("file contents"))
        })
        .register(create_issue_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({
                "id": 1,
                "title": "Created",
                "body": "Body",
                "extra": "hidden"
            })))
        })
}

fn create_issue_spec() -> CommandSpec {
    CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
        .with_arg(ArgSpec::string("title", "Issue title"))
        .with_arg(ArgSpec::string("body", "Issue body"))
        .with_permission(PermissionSpec::new(
            PermissionEffect::Write,
            "issues",
            "Creates an issue",
        ))
}

fn read_file_spec() -> CommandSpec {
    CommandSpec::new(["files", "read"], "Read file", "Read file")
        .with_arg(ArgSpec::path("path", "Path to read", "repo"))
        .with_permission(PermissionSpec::new(
            PermissionEffect::Read,
            "repo",
            "Reads a file",
        ))
}

#[test]
fn parser_binds_typed_placeholders_and_preserves_dangerous_arg_data() {
    let plan = registry()
        .build_plan(&request(
            "issues create --title $args.title --body $args.body",
            json!({
                "title": "A title with spaces",
                "body": "quotes \" and $(not shell) and | are data"
            }),
        ))
        .unwrap();

    assert_eq!(plan.command_path, vec!["issues", "create"]);
    assert_eq!(
        plan.bound_args["body"].value,
        "quotes \" and $(not shell) and | are data"
    );
}

#[test]
fn command_string_rejects_shell_constructs() {
    let error = registry()
        .build_plan(&request(
            "issues create --title $args.title | jq .",
            json!({"title": "x"}),
        ))
        .unwrap_err();
    assert!(matches!(error, FrameworkError::ShellSyntax(_)));
}

#[test]
fn command_string_rejects_substring_interpolation() {
    let error = registry()
        .build_plan(&request(
            "issues create --title=prefix-$args.title",
            json!({"title": "x"}),
        ))
        .unwrap_err();
    assert!(matches!(error, FrameworkError::PlaceholderInterpolation(_)));
}

#[test]
fn missing_unknown_and_wrong_type_args_are_structured_errors() {
    let missing = registry()
        .build_plan(&request(
            "issues create --title $args.title --body $args.body",
            json!({"title": "x"}),
        ))
        .unwrap_err();
    assert_eq!(missing, FrameworkError::MissingArgument("body".to_string()));

    let unknown = registry()
        .build_plan(&request(
            "issues create --title $args.title --body $args.body",
            json!({"title": "x", "body": "y", "extra": "z"}),
        ))
        .unwrap_err();
    assert_eq!(
        unknown,
        FrameworkError::UnknownArgument("extra".to_string())
    );

    let wrong_type = registry()
        .build_plan(&request(
            "issues create --title $args.title --body $args.body",
            json!({"title": true, "body": "y"}),
        ))
        .unwrap_err();
    assert!(matches!(
        wrong_type,
        FrameworkError::InvalidArgumentType(_, _)
    ));
}

#[test]
fn path_args_use_declared_workspaces() {
    let ok = registry()
        .build_plan(&request(
            "files read $args.path",
            json!({"path": "C:\\repo\\src\\lib.rs"}),
        ))
        .unwrap();
    assert_eq!(ok.command_path, vec!["files", "read"]);

    let denied = registry()
        .build_plan(&request(
            "files read $args.path",
            json!({"path": "C:\\other\\secret.txt"}),
        ))
        .unwrap_err();
    assert!(matches!(denied, FrameworkError::WorkspaceMismatch { .. }));

    let traversed = registry()
        .build_plan(&request(
            "files read $args.path",
            json!({"path": "C:\\repo\\..\\other\\secret.txt"}),
        ))
        .unwrap_err();
    assert!(matches!(
        traversed,
        FrameworkError::WorkspaceMismatch { .. }
    ));
}

#[tokio::test]
async fn dry_run_returns_plan_without_dispatch_or_permission_gate() {
    let mut run = request(
        "issues create --title $args.title --body $args.body",
        json!({"title": "x", "body": "y"}),
    );
    run.dry_run = true;

    let response = registry()
        .with_policy(PermissionPolicy::read_only())
        .run(run)
        .await
        .unwrap();

    assert!(response.dry_run);
    assert!(response.output.is_none());
    assert_eq!(response.plan.permissions[0].effect, PermissionEffect::Write);
}

#[tokio::test]
async fn denied_operations_fail_before_dispatch() {
    let error = registry()
        .with_policy(PermissionPolicy::read_only())
        .run(request(
            "issues create --title $args.title --body $args.body",
            json!({"title": "x", "body": "y"}),
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, FrameworkError::PermissionDenied { .. }));
}

#[tokio::test]
async fn output_selects_fields_and_limits_arrays() {
    let mut run = request(
        "issues create --title $args.title --body $args.body",
        json!({"title": "x", "body": "y"}),
    );
    run.output = Some(mcp_twill::OutputSpec {
        fields: Some(vec!["id".to_string(), "title".to_string()]),
        ..Default::default()
    });
    let response = registry().run(run).await.unwrap();
    let structured = response.output.unwrap().structured.unwrap();
    assert_eq!(structured, json!({"id": 1, "title": "Created"}));
}

#[test]
fn output_limits_structured_content_and_preserves_logs_and_cursors() {
    let output = CommandOutput {
        text: None,
        structured: Some(json!([
            { "id": 1, "title": "A very long title that should be truncated" },
            { "id": 2, "title": "Another very long title that should be truncated" }
        ])),
        stderr: vec!["handler warning".to_string()],
        next_cursor: Some("next-page".to_string()),
    }
    .apply_output_spec(&OutputSpec {
        max_bytes: Some(48),
        ..Default::default()
    });

    let structured = output.structured.unwrap();
    assert_eq!(structured["truncated"], true);
    assert_eq!(structured["maxBytes"], 48);
    assert_eq!(output.stderr, vec!["handler warning"]);
    assert_eq!(output.next_cursor.as_deref(), Some("next-page"));
}

#[test]
fn help_returns_server_and_command_docs() {
    let server = registry().help(HelpRequest::default());
    assert!(server.text.contains("primary execution tool"));

    let command = registry().help(HelpRequest {
        command: Some("issues create".to_string()),
        topic: None,
        detail: None,
    });
    assert!(command.text.contains("$args.title"));
}

#[test]
fn catalog_identity_is_stable_across_registration_order() {
    assert_eq!(
        registry().catalog_identity().catalog_hash,
        registry_reversed().catalog_identity().catalog_hash
    );

    let changed = registry().register(
        CommandSpec::new(["issues", "close"], "Close issue", "Close issue"),
        |_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) },
    );
    assert_ne!(
        registry().catalog_identity().catalog_hash,
        changed.catalog_identity().catalog_hash
    );
}

#[test]
fn invocation_fingerprint_is_stable_for_equivalent_plans() {
    let request = request(
        "issues create --title $args.title --body $args.body",
        json!({"title": "x", "body": "y"}),
    );

    let first = registry().build_plan(&request).unwrap();
    let second = registry().build_plan(&request).unwrap();

    assert_eq!(first.invocation_fingerprint, second.invocation_fingerprint);
}

#[test]
fn invocation_fingerprint_changes_with_contract_inputs() {
    let base_request = request(
        "issues create --title $args.title --body $args.body",
        json!({"title": "x", "body": "y"}),
    );
    let base = registry().build_plan(&base_request).unwrap();

    let changed_args = registry()
        .build_plan(&request(
            "issues create --title $args.title --body $args.body",
            json!({"title": "changed", "body": "y"}),
        ))
        .unwrap();
    assert_ne!(
        base.invocation_fingerprint,
        changed_args.invocation_fingerprint
    );

    let mut changed_output_request = base_request.clone();
    changed_output_request.output = Some(OutputSpec {
        fields: Some(vec!["id".to_string()]),
        ..Default::default()
    });
    let changed_output = registry().build_plan(&changed_output_request).unwrap();
    assert_ne!(
        base.invocation_fingerprint,
        changed_output.invocation_fingerprint
    );

    let changed_workspace_registry = CommandRegistry::new("test", "test server")
        .declare_workspace(WorkspaceDecl::new("repo", "C:/other"))
        .register(create_issue_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({ "id": 1 })))
        });
    let changed_workspace = changed_workspace_registry
        .build_plan(&base_request)
        .unwrap();
    assert_ne!(
        base.invocation_fingerprint,
        changed_workspace.invocation_fingerprint
    );

    let changed_permission_registry = CommandRegistry::new("test", "test server")
        .declare_workspace(WorkspaceDecl::new("repo", "C:/repo"))
        .register(
            create_issue_spec().with_permission(PermissionSpec::new(
                PermissionEffect::Network,
                "issues-api",
                "Calls the issue API",
            )),
            |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
        );
    let changed_permissions = changed_permission_registry
        .build_plan(&base_request)
        .unwrap();
    assert_ne!(
        base.invocation_fingerprint,
        changed_permissions.invocation_fingerprint
    );
}

#[test]
fn catalog_examples_are_validated_against_the_planner() {
    let mut example = CommandExample::new(
        "issues create --title $args.title --body $args.body",
        "Create an issue",
    );
    example.args.insert("title".to_string(), json!("Title"));
    example.args.insert("body".to_string(), json!("Body"));

    let valid = CommandRegistry::new("test", "test server").register(
        create_issue_spec().with_example(example),
        |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
    );
    valid.validate_examples().unwrap();

    let invalid = CommandRegistry::new("test", "test server").register(
        create_issue_spec().with_example(CommandExample::new(
            "issues create --title $args.title --body $args.body",
            "Missing args",
        )),
        |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
    );
    assert!(matches!(
        invalid.validate_examples().unwrap_err(),
        FrameworkError::MissingArgument(_)
    ));
}

#[test]
fn catalog_projects_composite_effects_and_required_lane() {
    let reg = CommandRegistry::new("test", "test server").register(
        CommandSpec::new(["deploy", "publish"], "Publish deploy", "Publish deploy")
            .with_permission(PermissionSpec::new(
                PermissionEffect::Write,
                "repo",
                "Writes deploy metadata",
            ))
            .with_permission(PermissionSpec::new(
                PermissionEffect::Network,
                "deploy-api",
                "Calls deployment API",
            )),
        |_context| async { Ok(CommandOutput::structured(json!({ "ok": true }))) },
    );

    let operation = reg.operation_specs().remove(0);
    assert!(matches!(operation.effect, EffectSpec::Composite(_)));
    assert_eq!(operation.lane(), EffectLane::Network);

    let lanes: Vec<_> = reg
        .lane_specs("repo")
        .into_iter()
        .map(|lane| lane.tool_name)
        .collect();
    assert_eq!(lanes, vec!["repo", "repo-network"]);
}

#[tokio::test]
async fn lane_checks_redirect_before_dispatch() {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let seen = dispatches.clone();
    let reg = CommandRegistry::new("test", "test server").register(
        create_issue_spec(),
        move |_context| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(CommandOutput::structured(json!({ "id": 1 })))
            }
        },
    );

    let error = reg
        .run_in_lane(
            request(
                "issues create --title $args.title --body $args.body",
                json!({"title": "x", "body": "y"}),
            ),
            "repo",
            EffectLane::Primary,
            "repo",
        )
        .await
        .unwrap_err();
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    assert_eq!(
        error,
        FrameworkError::WrongEffectLane {
            current_tool: "repo".to_string(),
            required_tool: "repo-write".to_string(),
        }
    );

    reg.run_in_lane(
        request(
            "issues create --title $args.title --body $args.body",
            json!({"title": "x", "body": "y"}),
        ),
        "repo-write",
        EffectLane::Write,
        "repo",
    )
    .await
    .unwrap();
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);
}
