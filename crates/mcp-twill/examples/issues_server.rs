use std::sync::Arc;

use mcp_twill::{
    CapabilityDecl, CommandContext, CommandOutput, CommandRegistry, EventSink, Field,
    FrameworkEvent, Result, TypeDecl, Variant, WorkspaceDecl, arg,
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
            server.preamble(
                "Issue records are the source of truth; keep them synchronized \
                 before acting on stale listings.",
            );

            server.workspace(
                WorkspaceDecl::file("repo", repo_root).with_description("Example repository root"),
            );

            server.capability(
                CapabilityDecl::new("session", "A live issue-tracker session lease")
                    .carried_by("session_id"),
            );

            server.declare_type(
                TypeDecl::union("issue-target", "How to locate the issue to act on")
                    .variant(
                        Variant::new("number", "Locate by issue number")
                            .field(Field::integer("number", "The issue number")),
                    )
                    .variant(
                        Variant::new("search", "Locate by title search")
                            .field(Field::string("query", "Text to match against titles"))
                            .fallback("the issue number is not known"),
                    ),
            );

            server.command("issues create", |command| {
                command
                    .summary("Create an issue")
                    .description("Creates a new issue from typed title and body arguments.")
                    .use_when("reporting a single new problem")
                    .alternative("issues sync", "pulling issues that already exist remotely")
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

            server.command("issues close", |command| {
                command
                    .summary("Close an issue")
                    .description("Closes the issue located by the target union.")
                    .arg(arg::named("target", "issue-target").summary("Which issue to close"))
                    .write("issues", "Marks an issue record closed")
                    .idempotent()
                    .example_with_args(
                        "issues close --target $args.target",
                        "Close an issue located by number",
                        json!({ "target": { "number": 1 } }),
                    )
                    .handle(|context: CommandContext| async move {
                        // Dispatch on the recorded variant instead of
                        // re-inspecting the JSON shape.
                        let target = &context.plan.bound_args["target"];
                        let by = match &target.variants {
                            Some(mcp_twill::ArgVariants::Single(variant)) => variant.clone(),
                            _ => "unknown".to_string(),
                        };
                        Ok(CommandOutput::structured(json!({
                            "closed": target.value,
                            "by": by,
                            "status": "closed"
                        })))
                    });
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

            server.command("issues export", |command| {
                command
                    .summary("Export issues to the repository")
                    .description(
                        "Writes an issue export under the repository root. The root is \
                         resolved by the server from the workspace declaration; it is \
                         never a command argument.",
                    )
                    .fallback(
                        ["issues list"],
                        "structured listings do not capture the fields you need",
                    )
                    .uses_workspace("repo")
                    .write("issues", "Writes an export file under the repository root")
                    .idempotent()
                    .example("issues export", "Export issues under the resolved repo root")
                    .handle(|context: CommandContext| async move {
                        let root = context
                            .workspace_root("repo")
                            .expect("declared workspace is resolved at plan time");
                        let path = root.path()?;
                        Ok(CommandOutput::structured(json!({
                            "exported_to": path.join("issues-export.json"),
                            "root_source": root.source,
                        })))
                    });
            });
            server.command("session start", |command| {
                command
                    .summary("Start an issue-tracker session")
                    .description(
                        "Establishes a session lease. Commands that require the `session` \
                         capability accept the returned id through their `session_id` \
                         argument.",
                    )
                    .provides("session")
                    .write("sessions", "Creates a session lease")
                    .example("session start", "Start a session and capture its id")
                    .handle(|_context| async {
                        Ok(CommandOutput::structured(json!({ "session_id": "sess-1" })))
                    });
            });

            server.command("issues sync", |command| {
                command
                    .summary("Sync issues with the remote tracker")
                    .description(
                        "Pushes and pulls issue records over an established session.",
                    )
                    .arg(arg::string("session_id").summary("Session that owns the sync"))
                    .requires("session")
                    .write("issues", "Updates issue records from the remote tracker")
                    .example_with_args(
                        "issues sync --session-id $args.session_id",
                        "Sync issues over an established session",
                        json!({ "session_id": "sess-1" }),
                    )
                    .handle(|context: CommandContext| async move {
                        let session = &context.plan.bound_args["session_id"].value;
                        Ok(CommandOutput::structured(json!({
                            "session_id": session,
                            "synced": 2
                        })))
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
