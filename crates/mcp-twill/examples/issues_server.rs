use mcp_twill::{CommandContext, CommandOutput, CommandRegistry, Result, WorkspaceDecl, arg};
use rmcp::{ServiceExt, transport::stdio};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct CreateIssueArgs {
    title: String,
    body: String,
}

async fn create_issue(_context: CommandContext, args: CreateIssueArgs) -> Result<CommandOutput> {
    Ok(CommandOutput::structured(json!({
        "id": 1,
        "title": args.title,
        "body": args.body,
        "status": "open"
    })))
}

fn registry() -> Result<CommandRegistry> {
    CommandRegistry::build(
        "issues-example",
        "Example MCP Twill server for issue tracking commands.",
        |server| {
            server.workspace(
                WorkspaceDecl::file("repo", "C:/workspace")
                    .with_description("Example repository root"),
            );

            server.command("issues create", |command| {
                command
                    .summary("Create an issue")
                    .description("Creates a new issue from typed title and body arguments.")
                    .arg(arg::string("title").summary("Issue title"))
                    .arg(arg::string("body").summary("Issue body"))
                    .write("issues", "Creates a new issue record")
                    .example_with_args(
                        "issues create --title $args.title --body $args.body",
                        "Create an issue with typed title and body values",
                        json!({
                            "title": "Crash on launch",
                            "body": "The app exits after the splash screen."
                        }),
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = mcp_twill::CliMcpServer::new(registry()?);
    server
        .serve(stdio())
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?
        .waiting()
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?;
    Ok(())
}
