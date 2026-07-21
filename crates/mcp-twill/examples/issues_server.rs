use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

use mcp_twill::{
    ApplicationError, ApplicationErrorDecl, ApplicationErrorUse, ApplicationResult,
    ApplicationSuccess, ArgumentRendering, ArgumentSchemaDecl, CapabilityDecl, CommandContext,
    CommandOutput, CommandRegistry, ConfirmationMessage, ConfirmationPresentation, EventSink,
    Field, FrameworkEvent, Grant, InvocationPlan, JsonInteger, Listing, ReadResource, Release, Res,
    ResolveResource, Resource, ResourceDecl, ResourceRefusal, Result, TypeDecl, Variant,
    WorkspaceDecl, application_error_set, arg,
};
use rmcp::transport::stdio;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct CreateIssueArgs {
    title: String,
    body: String,
}

const VALIDATION_RECEIPT: &str = "receipt-current-build";

#[derive(Debug, Deserialize, JsonSchema)]
struct PublishArgs {
    validation_token: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct PublishedBuild {
    published: bool,
    build: String,
}

#[derive(Debug)]
enum PublishFailure {
    InvalidReceipt,
}

impl fmt::Display for PublishFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("build publication was refused")
    }
}

impl Error for PublishFailure {}

impl ApplicationError for PublishFailure {
    fn declarations() -> Vec<ApplicationErrorDecl> {
        vec![ApplicationErrorDecl::new(
            "invalid_validation_receipt",
            "The validation receipt does not match the current build",
        )]
    }

    fn code(&self) -> &'static str {
        "invalid_validation_receipt"
    }

    fn details(&self) -> Value {
        json!({})
    }
}

application_error_set! {
    struct PublishErrors for PublishFailure {
        ApplicationErrorUse::new("invalid_validation_receipt")
            .for_capability("validated-build"),
    }
}

async fn publish_build(
    _context: CommandContext,
    args: PublishArgs,
) -> ApplicationResult<PublishedBuild, PublishFailure, PublishErrors> {
    if args.validation_token != VALIDATION_RECEIPT {
        return Err(PublishFailure::InvalidReceipt.into());
    }
    Ok(ApplicationSuccess::value(PublishedBuild {
        published: true,
        build: "current".to_string(),
    }))
}

async fn create_issue(_context: CommandContext, args: CreateIssueArgs) -> Result<CommandOutput> {
    Ok(CommandOutput::structured(json!({
        "id": 1,
        "title": args.title,
        "body": args.body,
        "status": "open"
    })))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(inline)]
enum WaitCondition {
    Delay {
        duration_ms: JsonInteger,
    },
    Text {
        #[schemars(length(min = 1))]
        text: String,
        state: WaitState,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
enum WaitState {
    Visible,
    Hidden,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    condition: WaitCondition,
    timeout_ms: Option<JsonInteger>,
}

async fn wait_for_page(_context: CommandContext, args: WaitArgs) -> Result<CommandOutput> {
    let condition = match args.condition {
        WaitCondition::Delay { duration_ms } => {
            json!({ "kind": "delay", "duration_ms": duration_ms })
        }
        WaitCondition::Text { text, state } => json!({
            "kind": "text",
            "text": text,
            "state": match state {
                WaitState::Visible => "visible",
                WaitState::Hidden => "hidden",
            }
        }),
    };
    Ok(CommandOutput::structured(json!({
        "condition": condition,
        "timeout_ms": args.timeout_ms,
        "completed": true,
    })))
}

/// The handler-side value for the `session` resource. The resolver produces
/// one from a reference; handlers receive it through `Res<Session>` or
/// `Release<Session>` parameters.
struct Session {
    id: String,
}

impl Resource for Session {
    const NAME: &'static str = "session";
}

/// The server's lease table. The framework never sees this — it hands
/// references to the resolver and receives resolved-or-refused.
#[derive(Default)]
struct SessionStore {
    live: Mutex<BTreeSet<String>>,
    next: Mutex<u64>,
}

impl SessionStore {
    fn start(&self) -> String {
        let mut next = self.next.lock().expect("session counter");
        *next += 1;
        let id = format!("sess-{next}");
        self.live.lock().expect("session table").insert(id.clone());
        id
    }

    fn end(&self, id: &str) {
        self.live.lock().expect("session table").remove(id);
    }

    fn contains(&self, id: &str) -> bool {
        self.live.lock().expect("session table").contains(id)
    }

    fn live_ids(&self) -> Vec<String> {
        self.live
            .lock()
            .expect("session table")
            .iter()
            .cloned()
            .collect()
    }
}

struct SessionResolver {
    store: Arc<SessionStore>,
}

impl ResolveResource<Session> for SessionResolver {
    async fn resolve(
        &self,
        reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<Session, ResourceRefusal> {
        if self.store.contains(reference) {
            Ok(Session {
                id: reference.to_string(),
            })
        } else {
            Err(ResourceRefusal::new(format!(
                "session `{reference}` is not a live session on this server"
            )))
        }
    }
}

struct SessionReader {
    store: Arc<SessionStore>,
}

impl ReadResource<Session> for SessionReader {
    async fn read(&self, id: &str) -> std::result::Result<Value, ResourceRefusal> {
        if self.store.contains(id) {
            Ok(json!({ "id": id, "status": "live" }))
        } else {
            Err(ResourceRefusal::new(format!(
                "session `{id}` is not a live session on this server"
            )))
        }
    }
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
    let store = Arc::new(SessionStore::default());

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

            // An explicit non-resource proof (RFC 0010): the provider returns
            // an opaque receipt under an application-owned field name. The
            // caller explicitly passes it through the declared carrier; Twill
            // derives discovery and recovery edges but never stores the proof.
            server.capability(
                CapabilityDecl::new(
                    "validated-build",
                    "Proof that the current build passed validation",
                )
                .carried_by("validation_token"),
            );

            // A first-class resource (RFC 0012): declaring it derives the
            // `session-ref` argument type and the `session` capability. The
            // lifecycle edges derive from the handler signatures below.
            server.resource(
                ResourceDecl::new("session", "A live issue-tracker session lease")
                    .uri("issues://session/{id}")
                    .lifetime("Valid from `session start` until `session end`")
                    .expiry("All sessions end when the server process exits"),
            );
            server.resolver::<Session>(SessionResolver {
                store: store.clone(),
            });
            server.reader::<Session>(SessionReader {
                store: store.clone(),
            });

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

            server.argument_schema(ArgumentSchemaDecl::new(
                "wait-condition",
                "A browser condition to wait for",
                json!({
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "kind": { "const": "delay" },
                                "duration_ms": { "type": "integer" }
                            },
                            "required": ["kind", "duration_ms"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "kind": { "const": "text" },
                                "text": { "type": "string", "minLength": 1 },
                                "state": {
                                    "type": "string",
                                    "enum": ["visible", "hidden"]
                                }
                            },
                            "required": ["kind", "text", "state"],
                            "additionalProperties": false
                        }
                    ]
                }),
            ));

            server.command("page wait", |command| {
                command
                    .summary("Wait for a page condition")
                    .description("Waits for a declared delay or text-state condition.")
                    .arg(
                        arg::named_schema("condition", "wait-condition")
                            .summary("Condition that completes the wait"),
                    )
                    .arg(
                        arg::integer("timeout_ms")
                            .summary("Maximum wait time in milliseconds")
                            .optional(),
                    )
                    .read("page", "Observes the requested page condition")
                    .example_with_args(
                        "page wait --condition $args.condition --timeout-ms $args.timeout_ms",
                        "Wait until text is visible",
                        json!({
                            "condition": {
                                "kind": "text",
                                "text": "Ready",
                                "state": "visible"
                            },
                            "timeout_ms": 5000
                        }),
                    )
                    .handle_constrained(wait_for_page);
            });

            server.command("issues create", |command| {
                command
                    .summary("Create an issue")
                    .description("Creates a new issue from typed title and body arguments.")
                    .use_when("reporting a single new problem")
                    .alternative("issues sync", "pulling issues that already exist remotely")
                    .arg(arg::string("title").summary("Issue title"))
                    .arg(arg::string("body").summary("Issue body"))
                    .write("issues", "Creates a new issue record")
                    .invocation_message("Creating an issue")
                    .confirmation(ConfirmationPresentation::new(
                        ConfirmationMessage::new("Create issue?")
                            .text("Create issue ")
                            .argument("title", ArgumentRendering::JsonString, "(missing title)")
                            .text("?"),
                    ))
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

            server.command("build validate", |command| {
                command
                    .summary("Validate the current build")
                    .description(
                        "Validates the current build and returns an opaque receipt for later deployment.",
                    )
                    .provides("validated-build")
                    .read("build", "Reads the current build inputs")
                    .example("build validate", "Validate the current build")
                    .handle(|_context: CommandContext| async {
                        Ok(CommandOutput::structured(json!({
                            "receipt": VALIDATION_RECEIPT,
                            "validated": true
                        })))
                    });
            });

            server.command("deploy publish", |command| {
                command
                    .summary("Publish the validated build")
                    .description(
                        "Publishes only when application code accepts the explicitly supplied validation receipt.",
                    )
                    .arg(
                        arg::string("validation_token")
                            .summary("Opaque receipt returned by build validate"),
                    )
                    .requires("validated-build")
                    .write("deployment", "Publishes the current build")
                    .idempotent()
                    .example_with_args(
                        "deploy publish --validation-token $args.validation_token",
                        "Publish using an explicitly supplied validation receipt",
                        json!({ "validation_token": VALIDATION_RECEIPT }),
                    )
                    .handle_result(publish_build);
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
                    .description(
                        "Lists open issues with structured output and observes the host repo when available.",
                    )
                    .uses_optional_workspace("repo")
                    .read("issues", "Reads issue records")
                    .example("issues list", "List issues without shell pipelines or jq")
                    .handle(|context: CommandContext| async move {
                        // Optional ambient context is available for application
                        // policy but never becomes a command argument.
                        let _repo = context.workspace_root("repo");
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
                    .example(
                        "issues export",
                        "Export issues under the resolved repo root",
                    )
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
                        "Establishes a session lease. The granted reference is what \
                         commands that operate on a session accept through their \
                         `session_id` argument.",
                    )
                    .write("sessions", "Creates a session lease")
                    .example("session start", "Start a session and capture its id")
                    .handle({
                        let store = store.clone();
                        move |_context: CommandContext| {
                            let store = store.clone();
                            async move {
                                let id = store.start();
                                Ok(CommandOutput::structured(json!({ "session_id": id }))
                                    .grant(Grant::<Session>::new(id)))
                            }
                        }
                    });
            });

            server.command("session list", |command| {
                command
                    .summary("List live sessions")
                    .description(
                        "Enumerates live sessions — the recovery path when a session \
                         id has fallen out of context.",
                    )
                    .read("sessions", "Reads session leases")
                    .example("session list", "Recover the ids of live sessions")
                    .handle({
                        let store = store.clone();
                        move |_context: CommandContext| {
                            let store = store.clone();
                            async move {
                                let ids = store.live_ids();
                                Ok(CommandOutput::structured(json!({ "count": ids.len() }))
                                    .listing(Listing::<Session>::new(ids)))
                            }
                        }
                    });
            });

            server.command("session end", |command| {
                command
                    .summary("End an issue-tracker session")
                    .description("Releases the session lease; its references stop resolving.")
                    .write("sessions", "Removes a session lease")
                    .idempotent()
                    .example_with_args(
                        "session end --session-id $args.session_id",
                        "End a session by id",
                        json!({ "session_id": "sess-1" }),
                    )
                    .handle({
                        let store = store.clone();
                        move |session: Release<Session>, _context: CommandContext| {
                            let store = store.clone();
                            async move {
                                store.end(&session.id);
                                Ok(CommandOutput::structured(json!({
                                    "ended": session.id
                                })))
                            }
                        }
                    });
            });

            server.command("issues sync", |command| {
                command
                    .summary("Sync issues with the remote tracker")
                    .description("Pushes and pulls issue records over an established session.")
                    .use_when("pulling issues that already exist remotely")
                    .write("issues", "Updates issue records from the remote tracker")
                    .example_with_args(
                        "issues sync --session-id $args.session_id",
                        "Sync issues over an established session",
                        json!({ "session_id": "sess-1" }),
                    )
                    .handle(
                        |session: Res<Session>, _context: CommandContext| async move {
                            Ok(CommandOutput::structured(json!({
                                "session_id": session.id,
                                "synced": 2
                            })))
                        },
                    );
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
