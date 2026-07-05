use std::sync::Arc;

use mcp_twill::{
    CommandContext, CommandOutput, CommandRegistry, EventSink, FrameworkEvent, Result,
    WorkspaceDecl, arg,
};
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

/// Writes one JSON line per framework event to stderr. Stdout belongs to the
/// MCP transport, so stderr is where a stdio server can log. A production
/// server would use a persistent sink (JSONL file, database) instead.
struct StderrEventSink;

impl EventSink for StderrEventSink {
    fn record(&self, event: FrameworkEvent) {
        if let Ok(line) = serde_json::to_string(&event) {
            eprintln!("{line}");
        }
    }
}

pub fn registry() -> Result<CommandRegistry> {
    let repo_root = std::env::current_dir()
        .map_err(|error| mcp_twill::FrameworkError::Build(error.to_string()))?
        .to_string_lossy()
        .into_owned();

    CommandRegistry::build(
        "issues-example",
        "Example MCP Twill server for issue tracking commands.",
        |server| {
            server.workspace(
                WorkspaceDecl::file("repo", repo_root).with_description("Example repository root"),
            );

            server.command("issues create", |command| {
                command
                    .summary("Create an issue")
                    .description("Creates a new issue from typed title and body arguments.")
                    .arg(arg::string("title").summary("Issue title"))
                    .arg(arg::string("body").summary("Issue body"))
                    .write("issues", "Creates a new issue record")
                    .idempotent()
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
    let server =
        mcp_twill::CliMcpServer::new(registry()?)?.with_event_sink(Arc::new(StderrEventSink));

    // Runtime identity names the contract this instance serves; a supervisor
    // can compare hashes across restarts to detect catalog changes.
    let identity = server.runtime_identity();
    eprintln!(
        "issues-example starting: catalog {} (run schema {}, help schema {})",
        identity.catalog_hash, identity.run_schema_hash, identity.help_schema_hash
    );

    server
        .serve(stdio())
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?
        .waiting()
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?;
    Ok(())
}

/// Generated contract coverage for the example server: one test per contract
/// rule, produced by the framework. RFC 0004's acceptance test is that an
/// example server gets this coverage without writing bespoke assertions.
#[cfg(test)]
mod contract_coverage {
    fn contract_registry() -> mcp_twill::CommandRegistry {
        super::registry().expect("example registry builds")
    }

    mcp_twill::contract_tests!(contract_registry);
}
