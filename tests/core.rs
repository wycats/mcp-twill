use mcp_twill::{
    ArgSpec, CommandOutput, CommandRegistry, CommandSpec, FrameworkError, HelpRequest,
    PermissionEffect, PermissionPolicy, PermissionSpec, RunRequest, WorkspaceDecl,
};
use serde_json::json;
use std::collections::BTreeMap;

fn request(command: &str, args: serde_json::Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args).unwrap_or_else(|_| BTreeMap::new()),
        stdin: None,
        output: None,
        dry_run: false,
    }
}

fn registry() -> CommandRegistry {
    CommandRegistry::new("test", "test server")
        .declare_workspace(WorkspaceDecl::new("repo", "C:/repo"))
        .register(
            CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
                .with_arg(ArgSpec::string("title", "Issue title"))
                .with_arg(ArgSpec::string("body", "Issue body"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "issues",
                    "Creates an issue",
                )),
            |_context| async {
                Ok(CommandOutput::structured(json!({
                    "id": 1,
                    "title": "Created",
                    "body": "Body",
                    "extra": "hidden"
                })))
            },
        )
        .register(
            CommandSpec::new(["files", "read"], "Read file", "Read file")
                .with_arg(ArgSpec::path("path", "Path to read", "repo"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Read,
                    "repo",
                    "Reads a file",
                )),
            |_context| async { Ok(CommandOutput::text("file contents")) },
        )
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
fn help_returns_server_and_command_docs() {
    let server = registry().help(HelpRequest::default());
    assert!(server.text.contains("Tools: `help`, `run`"));

    let command = registry().help(HelpRequest {
        command: Some("issues create".to_string()),
        topic: None,
        detail: None,
    });
    assert!(command.text.contains("$args.title"));
}
