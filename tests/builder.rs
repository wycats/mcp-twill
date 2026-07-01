use std::collections::BTreeMap;

use mcp_twill::{
    ArgSpec, CliMcpServer, CliMcpServerConfig, CommandContext, CommandExample, CommandOutput,
    CommandRegistry, CommandSpec, FrameworkError, OutputContract, OutputFormat, PermissionSpec,
    RunRequest, WorkspaceDecl, arg,
};
use rmcp::ServerHandler;
use serde::Deserialize;
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

fn expect_build_err(result: mcp_twill::Result<CommandRegistry>) -> FrameworkError {
    match result {
        Ok(_) => panic!("expected registry construction to fail"),
        Err(error) => error,
    }
}

fn create_example() -> CommandExample {
    let mut example = CommandExample::new(
        "issues create --title $args.title --body $args.body",
        "Create an issue with typed title and body values",
    );
    example.args.insert("title".to_string(), json!("Crash"));
    example.args.insert("body".to_string(), json!("Body"));
    example
}

fn read_file_example() -> CommandExample {
    let mut example = CommandExample::new("files read $args.path", "Read a file from the repo");
    example
        .args
        .insert("path".to_string(), json!("C:/workspace/src/lib.rs"));
    example
}

fn explicit_registry() -> CommandRegistry {
    CommandRegistry::new(
        "issues-example",
        "Example MCP Twill server for issue tracking commands.",
    )
    .declare_workspace(
        WorkspaceDecl::file("repo", "C:/workspace").with_description("Example repository root"),
    )
    .register(
        CommandSpec::new(
            ["files", "read"],
            "Read file",
            "Reads a file from the declared repository workspace.",
        )
        .with_arg(ArgSpec::path("path", "Path to read", "repo"))
        .with_permission(PermissionSpec::read("repo", "Reads repository files"))
        .with_example(read_file_example()),
        |_context| async { Ok(CommandOutput::text("file contents")) },
    )
    .register(
        CommandSpec::new(
            ["issues", "create"],
            "Create an issue",
            "Creates a new issue from typed title and body arguments.",
        )
        .with_arg(ArgSpec::string("title", "Issue title"))
        .with_arg(ArgSpec::string("body", "Issue body"))
        .with_permission(PermissionSpec::write(
            "issues",
            "Creates a new issue record",
        ))
        .with_output(OutputContract {
            format: OutputFormat::Structured,
            summary: "Created issue record.".to_string(),
        })
        .with_example(create_example()),
        |_context| async {
            Ok(CommandOutput::structured(json!({
                "id": 1,
                "title": "Created"
            })))
        },
    )
    .register(
        CommandSpec::new(
            ["issues", "list"],
            "List issues",
            "Lists open issues with structured output.",
        )
        .with_permission(PermissionSpec::read("issues", "Reads issue records"))
        .with_example(CommandExample::new(
            "issues list",
            "List issues without shell pipelines or jq",
        )),
        |_context| async {
            Ok(CommandOutput::structured(json!([
                { "id": 1, "title": "Crash on launch", "status": "open" },
                { "id": 2, "title": "Improve help text", "status": "open" }
            ])))
        },
    )
}

#[derive(Debug, Deserialize)]
struct CreateIssueArgs {
    title: String,
    body: String,
}

async fn create_issue(
    _context: CommandContext,
    args: CreateIssueArgs,
) -> mcp_twill::Result<CommandOutput> {
    Ok(CommandOutput::structured(json!({
        "id": 1,
        "title": args.title,
        "body": args.body,
        "status": "open"
    })))
}

fn builder_registry() -> CommandRegistry {
    CommandRegistry::build(
        "issues-example",
        "Example MCP Twill server for issue tracking commands.",
        |server| {
            server.workspace(
                WorkspaceDecl::file("repo", "C:/workspace")
                    .with_description("Example repository root"),
            );

            server.command("files read", |command| {
                command
                    .summary("Read file")
                    .description("Reads a file from the declared repository workspace.")
                    .arg(arg::path("path", "repo").summary("Path to read"))
                    .read("repo", "Reads repository files")
                    .example_with_args(
                        "files read $args.path",
                        "Read a file from the repo",
                        json!({ "path": "C:/workspace/src/lib.rs" }),
                    )
                    .handle(|_context| async { Ok(CommandOutput::text("file contents")) });
            });

            server.command("issues create", |command| {
                command
                    .summary("Create an issue")
                    .description("Creates a new issue from typed title and body arguments.")
                    .arg(arg::string("title").summary("Issue title"))
                    .arg(arg::string("body").summary("Issue body"))
                    .write("issues", "Creates a new issue record")
                    .output(OutputContract {
                        format: OutputFormat::Structured,
                        summary: "Created issue record.".to_string(),
                    })
                    .example_with_args(
                        "issues create --title $args.title --body $args.body",
                        "Create an issue with typed title and body values",
                        json!({ "title": "Crash", "body": "Body" }),
                    )
                    .handle_typed(create_issue);
            });

            server.command("issues list", |command| {
                command
                    .summary("List issues")
                    .description("Lists open issues with structured output.")
                    .read("issues", "Reads issue records")
                    .example("issues list", "List issues without shell pipelines or jq")
                    .handle(|_context| async {
                        Ok(CommandOutput::structured(json!([
                            { "id": 1, "title": "Crash on launch", "status": "open" },
                            { "id": 2, "title": "Improve help text", "status": "open" }
                        ])))
                    });
            });
        },
    )
    .unwrap()
}

#[test]
fn builder_catalog_matches_equivalent_explicit_specs() {
    assert_eq!(
        builder_registry().catalog_identity().catalog_hash,
        explicit_registry().catalog_identity().catalog_hash
    );

    let operation = builder_registry()
        .operation_specs()
        .into_iter()
        .find(|operation| operation.name() == "issues create")
        .unwrap();
    assert_eq!(operation.output.summary, "Created issue record.");
}

#[test]
fn builder_commands_project_help_lanes_and_annotations() {
    let registry = builder_registry();
    assert!(
        registry
            .help(Default::default())
            .text
            .contains("issues create")
    );

    let lanes: Vec<_> = registry
        .lane_specs("repo")
        .into_iter()
        .map(|lane| lane.tool_name)
        .collect();
    assert_eq!(lanes, vec!["repo", "repo-write"]);

    let server = CliMcpServer::with_config(
        registry,
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    )
    .expect("builder registry has no custom effects");
    let repo = server.get_tool("repo").unwrap();
    let repo_write = server.get_tool("repo-write").unwrap();
    assert_eq!(
        repo.annotations.as_ref().unwrap().read_only_hint,
        Some(true)
    );
    assert_eq!(
        repo_write.annotations.as_ref().unwrap().read_only_hint,
        Some(false)
    );
}

#[tokio::test]
async fn typed_handler_receives_deserialized_args() {
    let response = builder_registry()
        .run(request(
            "issues create --title $args.title --body $args.body",
            json!({ "title": "Typed title", "body": "Typed body" }),
        ))
        .await
        .unwrap();

    let output = response.output.unwrap().structured.unwrap();
    assert_eq!(output["title"], "Typed title");
    assert_eq!(output["body"], "Typed body");
}

#[test]
fn builder_preserves_planner_diagnostics_for_inputs() {
    let missing = builder_registry()
        .build_plan(&request(
            "issues create --title $args.title --body $args.body",
            json!({ "title": "Only title" }),
        ))
        .unwrap_err();
    assert_eq!(missing, FrameworkError::MissingArgument("body".to_string()));

    let wrong_type = builder_registry()
        .build_plan(&request(
            "issues create --title $args.title --body $args.body",
            json!({ "title": true, "body": "Body" }),
        ))
        .unwrap_err();
    assert!(matches!(
        wrong_type,
        FrameworkError::InvalidArgumentType(_, _)
    ));
}

#[test]
fn builder_path_args_enforce_declared_workspaces() {
    builder_registry()
        .build_plan(&request(
            "files read $args.path",
            json!({ "path": "C:/workspace/src/lib.rs" }),
        ))
        .unwrap();

    let denied = builder_registry()
        .build_plan(&request(
            "files read $args.path",
            json!({ "path": "C:/elsewhere/src/lib.rs" }),
        ))
        .unwrap_err();
    assert!(matches!(denied, FrameworkError::WorkspaceMismatch { .. }));
}

#[test]
fn builder_validates_examples() {
    let invalid = expect_build_err(CommandRegistry::build("test", "test server", |server| {
        server.command("issues create", |command| {
            command
                .summary("Create issue")
                .description("Create issue")
                .arg(arg::string("title").summary("Issue title"))
                .example("issues create --title $args.title", "Missing args")
                .handle(|_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) });
        });
    }));

    assert!(matches!(invalid, FrameworkError::MissingArgument(_)));
}

#[test]
fn builder_rejects_duplicate_paths_args_unknown_workspaces_and_missing_handlers() {
    let duplicate_path =
        expect_build_err(CommandRegistry::build("test", "test server", |server| {
            server.command("issues list", |command| {
                command
                    .summary("List")
                    .description("List")
                    .handle(|_context| async { Ok(CommandOutput::structured(json!([]))) });
            });
            server.command("issues list", |command| {
                command
                    .summary("List again")
                    .description("List again")
                    .handle(|_context| async { Ok(CommandOutput::structured(json!([]))) });
            });
        }));
    assert!(duplicate_path.to_string().contains("duplicate command"));

    let duplicate_arg = expect_build_err(CommandRegistry::build("test", "test server", |server| {
        server.command("issues create", |command| {
            command
                .summary("Create")
                .description("Create")
                .arg(arg::string("title").summary("Title"))
                .arg(arg::string("title").summary("Title again"))
                .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
        });
    }));
    assert!(duplicate_arg.to_string().contains("duplicate argument"));

    let unknown_workspace =
        expect_build_err(CommandRegistry::build("test", "test server", |server| {
            server.command("files read", |command| {
                command
                    .summary("Read")
                    .description("Read")
                    .arg(arg::path("path", "missing").summary("Path"))
                    .handle(|_context| async { Ok(CommandOutput::text("contents")) });
            });
        }));
    assert!(unknown_workspace.to_string().contains("unknown workspace"));

    let missing_handler =
        expect_build_err(CommandRegistry::build("test", "test server", |server| {
            server.command("issues list", |command| {
                command.summary("List").description("List");
            });
        }));
    assert!(missing_handler.to_string().contains("missing a handler"));
}
