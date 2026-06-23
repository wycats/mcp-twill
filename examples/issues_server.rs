use mcp_twill::{
    ArgSpec, CliMcpServer, CommandExample, CommandOutput, CommandRegistry, CommandSpec,
    PermissionEffect, PermissionSpec, Result, WorkspaceDecl,
};
use rmcp::{ServiceExt, transport::stdio};
use serde_json::json;

fn registry() -> CommandRegistry {
    CommandRegistry::new(
        "issues-example",
        "Example MCP Twill server for issue tracking commands.",
    )
    .declare_workspace(
        WorkspaceDecl::new("repo", "C:/workspace").with_description("Example repository root"),
    )
    .register(
        CommandSpec::new(
            ["issues", "create"],
            "Create an issue",
            "Creates a new issue from typed title and body arguments.",
        )
        .with_arg(ArgSpec::string("title", "Issue title"))
        .with_arg(ArgSpec::string("body", "Issue body"))
        .with_permission(PermissionSpec::new(
            PermissionEffect::Write,
            "issues",
            "Creates a new issue record",
        ))
        .with_example(CommandExample::new(
            "issues create --title $args.title --body $args.body",
            "Create an issue with typed title and body values",
        )),
        |_context| async {
            Ok(CommandOutput::structured(json!({
                "id": 1,
                "title": "Created from MCP Twill",
                "status": "open"
            })))
        },
    )
    .register(
        CommandSpec::new(
            ["issues", "list"],
            "List issues",
            "Lists open issues with structured output.",
        )
        .with_permission(PermissionSpec::new(
            PermissionEffect::Read,
            "issues",
            "Reads issue records",
        ))
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

#[tokio::main]
async fn main() -> Result<()> {
    let server = CliMcpServer::new(registry());
    server
        .serve(stdio())
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?
        .waiting()
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?;
    Ok(())
}
