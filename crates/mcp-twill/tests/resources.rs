//! RFC 0012 acceptance tests: first-class resources declared once, with
//! lifecycle edges derived from handler signatures — catalog projection,
//! registration validation, URI minting and normalization, structured
//! refusals with recovery edges, and the MCP resource_link/read surface.

use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use mcp_twill::{
    ArgType, CapabilityDecl, CliMcpServer, CommandContext, CommandOutput, CommandRegistry,
    CommandSpec, Field, FrameworkError, Grant, HelpRequest, InvocationPlan, Listing, ReadResource,
    Release, Res, ResolveResource, Resource, ResourceDecl, ResourceRefusal, ResponseEnvelope,
    RunRequest, TypeDecl, Variant, arg,
};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ReadResourceRequestParams},
};
use serde_json::{Value, json};

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

fn json_object<T: serde::Serialize>(value: T) -> anyhow::Result<serde_json::Map<String, Value>> {
    match serde_json::to_value(value)? {
        Value::Object(map) => Ok(map),
        other => anyhow::bail!("expected JSON object, got {other:?}"),
    }
}

/// The handler-side value for the `session` resource.
struct Session {
    id: String,
}

impl Resource for Session {
    const NAME: &'static str = "session";
}

/// The server's lease table; the framework only sees resolved-or-refused.
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
                "session `{reference}` is not live"
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
            Err(ResourceRefusal::new(format!("session `{id}` is not live")))
        }
    }
}

struct RegistryOptions {
    reader: bool,
    enumerator: bool,
    bad_grant_id: bool,
    lifetime: &'static str,
}

impl Default for RegistryOptions {
    fn default() -> Self {
        Self {
            reader: true,
            enumerator: true,
            bad_grant_id: false,
            lifetime: "Valid from `session start` until `session end`",
        }
    }
}

/// A server with the full session lifecycle: grant (`session start`),
/// release (`session end`), enumerate (`session list`), require
/// (`session status`) — all edges derived from handler signatures.
fn build_registry(
    options: RegistryOptions,
    status_runs: Arc<AtomicUsize>,
) -> mcp_twill::Result<CommandRegistry> {
    let store = Arc::new(SessionStore::default());
    CommandRegistry::build(
        "resource-test",
        "RFC 0012 resource acceptance test server.",
        |server| {
            server.resource(
                ResourceDecl::new("session", "A live test session lease")
                    .uri("test://session/{id}")
                    .lifetime(options.lifetime),
            );
            server.resolver::<Session>(SessionResolver {
                store: store.clone(),
            });
            if options.reader {
                server.reader::<Session>(SessionReader {
                    store: store.clone(),
                });
            }

            server.command("session start", |command| {
                command
                    .summary("Start a session")
                    .description("Establishes a session lease and grants its reference.")
                    .handle({
                        let store = store.clone();
                        let bad_grant_id = options.bad_grant_id;
                        move |_context: CommandContext| {
                            let store = store.clone();
                            async move {
                                let id = if bad_grant_id {
                                    "bad id!".to_string()
                                } else {
                                    store.start()
                                };
                                Ok(CommandOutput::structured(json!({ "session_id": id }))
                                    .grant(Grant::<Session>::new(id)))
                            }
                        }
                    });
            });

            server.command("session end", |command| {
                command
                    .summary("End a session")
                    .description("Releases the session lease.")
                    .handle({
                        let store = store.clone();
                        move |session: Release<Session>, _context: CommandContext| {
                            let store = store.clone();
                            async move {
                                store.end(&session.id);
                                Ok(CommandOutput::structured(json!({ "ended": session.id })))
                            }
                        }
                    });
            });

            if options.enumerator {
                server.command("session list", |command| {
                    command
                        .summary("List live sessions")
                        .description("Enumerates live session leases.")
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
            }

            server.command("session status", |command| {
                command
                    .summary("Inspect a session")
                    .description("Reads the session lease state.")
                    .handle({
                        let status_runs = status_runs.clone();
                        move |session: Res<Session>, _context: CommandContext| {
                            let status_runs = status_runs.clone();
                            async move {
                                status_runs.fetch_add(1, Ordering::SeqCst);
                                Ok(CommandOutput::structured(json!({
                                    "session_id": session.id,
                                    "status": "live"
                                })))
                            }
                        }
                    });
            });
        },
    )
}

fn session_registry() -> CommandRegistry {
    build_registry(RegistryOptions::default(), Arc::new(AtomicUsize::new(0)))
        .expect("session registry builds")
}

// ---------------------------------------------------------------------------
// Catalog projection
// ---------------------------------------------------------------------------

// Acceptance: the catalog carries the resources section with lifecycle
// edges derived from handler signatures.
#[test]
fn catalog_projects_resource_with_derived_edges() {
    let catalog = session_registry().catalog();

    assert_eq!(catalog.resources.len(), 1);
    let resource = &catalog.resources[0];
    assert_eq!(resource.name, "session");
    assert_eq!(resource.summary, "A live test session lease");
    assert_eq!(resource.uri, "test://session/{id}");
    assert_eq!(resource.carrier, "session_id");
    assert_eq!(
        resource.lifetime.as_deref(),
        Some("Valid from `session start` until `session end`")
    );
    assert_eq!(resource.granted_by, vec!["session start".to_string()]);
    assert_eq!(resource.released_by, vec!["session end".to_string()]);
    assert_eq!(resource.enumerated_by, vec!["session list".to_string()]);
    assert_eq!(resource.required_by, vec!["session status".to_string()]);
}

// Acceptance: signature-derived resource requirements inject the carrier
// argument with the derived reference type; nothing is hand-authored.
#[test]
fn carrier_argument_is_injected_with_derived_reference_type() {
    let registry = session_registry();
    let status = registry
        .command_specs()
        .find(|spec| spec.path.join(" ") == "session status")
        .expect("status command");
    let carrier = status
        .args
        .iter()
        .find(|arg| arg.name == "session_id")
        .expect("injected carrier argument");
    assert!(
        matches!(&carrier.value_type, ArgType::ResourceRef(name) if name == "session"),
        "carrier type: {:?}",
        carrier.value_type
    );
    assert!(carrier.required, "carrier must be required");
    assert_eq!(status.requires_resources, vec!["session".to_string()]);

    let start = registry
        .command_specs()
        .find(|spec| spec.path.join(" ") == "session start")
        .expect("start command");
    assert_eq!(start.grants, vec!["session".to_string()]);
    assert!(
        start.args.iter().all(|arg| arg.name != "session_id"),
        "granting commands do not take the carrier"
    );

    let end = registry
        .command_specs()
        .find(|spec| spec.path.join(" ") == "session end")
        .expect("end command");
    assert_eq!(end.releases, vec!["session".to_string()]);
}

// Acceptance: resource declarations and derived edges are hash-covered —
// editing prose or removing an edge moves the catalog hash.
#[test]
fn resource_declarations_and_edges_move_the_catalog_hash() {
    let base = session_registry().catalog_identity().catalog_hash;

    let edited_prose = build_registry(
        RegistryOptions {
            lifetime: "Valid until the heat death of the universe",
            ..Default::default()
        },
        Arc::new(AtomicUsize::new(0)),
    )
    .expect("edited registry builds")
    .catalog_identity()
    .catalog_hash;
    assert_ne!(base, edited_prose, "editing validity prose moves the hash");

    let fewer_edges = build_registry(
        RegistryOptions {
            enumerator: false,
            ..Default::default()
        },
        Arc::new(AtomicUsize::new(0)),
    )
    .expect("registry without enumerator builds")
    .catalog_identity()
    .catalog_hash;
    assert_ne!(base, fewer_edges, "removing an edge moves the hash");
}

// ---------------------------------------------------------------------------
// Help projection
// ---------------------------------------------------------------------------

// Acceptance: help renders lifecycle facts derived from declarations and
// signatures — requirements with recovery edges, grants with URI template,
// the server-level resource tree.
#[test]
fn help_renders_derived_resource_lifecycle() {
    let registry = session_registry();

    let server_help = registry.help(HelpRequest {
        command: None,
        topic: None,
        detail: None,
    });
    assert!(
        server_help.text.contains("`session`"),
        "server help renders the resource: {}",
        server_help.text
    );

    let status_help = registry.help(HelpRequest {
        command: Some("session status".to_string()),
        topic: None,
        detail: None,
    });
    assert!(
        status_help.text.contains("`session`"),
        "requirement renders: {}",
        status_help.text
    );
    assert!(
        status_help.text.contains("Recover with `session list`"),
        "recovery names the enumerator: {}",
        status_help.text
    );

    let start_help = registry.help(HelpRequest {
        command: Some("session start".to_string()),
        topic: None,
        detail: None,
    });
    assert!(
        start_help.text.contains("test://session/{id}"),
        "grant renders the URI template: {}",
        start_help.text
    );
}

// ---------------------------------------------------------------------------
// Registration validation
// ---------------------------------------------------------------------------

fn build_error(configure: impl FnOnce(&mut mcp_twill::ServerBuilder)) -> String {
    match CommandRegistry::build("resource-test", "Validation test server.", configure) {
        Ok(_) => panic!("registration must fail"),
        Err(error) => error.to_string(),
    }
}

// Acceptance: a handler signature referencing an undeclared resource fails
// registration with a message naming the resource and command.
#[test]
fn undeclared_signature_resource_fails_registration() {
    let error = build_error(|server| {
        server.command("session status", |command| {
            command
                .summary("Inspect a session")
                .description("Reads the session lease state.")
                .handle(
                    |session: Res<Session>, _context: CommandContext| async move {
                        Ok(CommandOutput::structured(json!({ "id": session.id })))
                    },
                );
        });
    });
    assert!(
        error.contains("command `session status`")
            && error.contains("resource `session`")
            && error.contains("not declared"),
        "{error}"
    );
}

// Acceptance: a required resource with no bound resolver fails registration.
#[test]
fn missing_resolver_fails_registration() {
    let error = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease").uri("test://session/{id}"),
        );
        server.command("session status", |command| {
            command
                .summary("Inspect a session")
                .description("Reads the session lease state.")
                .handle(
                    |session: Res<Session>, _context: CommandContext| async move {
                        Ok(CommandOutput::structured(json!({ "id": session.id })))
                    },
                );
        });
    });
    assert!(error.contains("no bound resolver"), "{error}");
}

// Acceptance: an unpaired grant — no releaser, no expiry — fails
// registration; adding expiry prose makes the same shape register.
#[test]
fn unpaired_grant_requires_release_or_expiry() {
    let error = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease").uri("test://session/{id}"),
        );
        server.command("session start", |command| {
            command
                .summary("Start a session")
                .description("Establishes a session lease.")
                .handle(|_context: CommandContext| async move {
                    Ok(CommandOutput::structured(json!({})).grant(Grant::<Session>::new("sess-1")))
                });
        });
    });
    assert!(
        error.contains("resource `session`") && error.contains("no command releases it"),
        "{error}"
    );

    // Expiry prose names the release path, so the same shape registers.
    CommandRegistry::build("resource-test", "Validation test server.", |server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease")
                .uri("test://session/{id}")
                .expiry("All sessions end when the server process exits"),
        );
        server.command("session start", |command| {
            command
                .summary("Start a session")
                .description("Establishes a session lease.")
                .handle(|_context: CommandContext| async move {
                    Ok(CommandOutput::structured(json!({})).grant(Grant::<Session>::new("sess-1")))
                });
        });
    })
    .expect("expiry retires the grant");
}

struct Tab {
    id: String,
}

impl Resource for Tab {
    const NAME: &'static str = "tab";
}

struct TabResolver;

impl ResolveResource<Tab> for TabResolver {
    async fn resolve(
        &self,
        reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<Tab, ResourceRefusal> {
        Ok(Tab {
            id: reference.to_string(),
        })
    }
}

// Acceptance: a scoped resource (declared `within` another) granted without
// an enumerator fails registration — it must be recoverable by re-asking.
#[test]
fn scoped_grant_without_enumerator_fails_registration() {
    let error = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease").uri("test://session/{id}"),
        );
        server.resource(
            ResourceDecl::new("tab", "An open tab in a session")
                .uri("test://tab/{id}")
                .within("session"),
        );
        server.resolver::<Tab>(TabResolver);
        server.command("tab open", |command| {
            command
                .summary("Open a tab")
                .description("Opens a tab and grants its reference.")
                .handle(|_context: CommandContext| async move {
                    Ok(CommandOutput::structured(json!({})).grant(Grant::<Tab>::new("tab-1")))
                });
        });
        server.command("tab close", |command| {
            command
                .summary("Close a tab")
                .description("Releases the tab lease.")
                .handle(|tab: Release<Tab>, _context: CommandContext| async move {
                    Ok(CommandOutput::structured(json!({ "closed": tab.id })))
                });
        });
    });
    assert!(
        error.contains("resource `tab`") && error.contains("no command enumerates it"),
        "{error}"
    );
}

// Acceptance: a URI template without exactly one `{id}` slot fails.
#[test]
fn malformed_uri_template_fails_registration() {
    let error = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease").uri("test://session/"),
        );
    });
    assert!(error.contains("exactly one `{id}` slot"), "{error}");
}

// Acceptance: two resources sharing a URI template fail registration.
#[test]
fn colliding_uri_templates_fail_registration() {
    let error = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease").uri("test://thing/{id}"),
        );
        server.resource(ResourceDecl::new("tab", "An open tab").uri("test://thing/{id}"));
    });
    assert!(error.contains("same URI template"), "{error}");
}

// Acceptance: a derived capability name colliding with a hand-declared
// capability fails registration; the resource owns the name.
#[test]
fn derived_capability_collision_fails_registration() {
    let error = build_error(|server| {
        server.capability(
            CapabilityDecl::new("session", "A hand-declared session").carried_by("session_id"),
        );
        server.resource(
            ResourceDecl::new("session", "A live test session lease").uri("test://session/{id}"),
        );
        // A complete hand-declared capability graph (provider + requirer)
        // keeps capability validation happy, so validation reaches the
        // resource collision check.
        server.command("session grant", |command| {
            command
                .summary("Grant a session")
                .description("Establishes a session by hand.")
                .provides("session")
                .handle(|_context: CommandContext| async move {
                    Ok(CommandOutput::structured(json!({})))
                });
        });
        server.command("session use", |command| {
            command
                .summary("Use a session")
                .description("Consumes the hand-declared session.")
                .arg(arg::string("session_id").summary("Session to use"))
                .requires("session")
                .handle(|_context: CommandContext| async move {
                    Ok(CommandOutput::structured(json!({})))
                });
        });
    });
    assert!(
        error.contains("capability `session`") && error.contains("the resource owns that name"),
        "{error}"
    );
}

// Acceptance: a derived reference type name colliding with a hand-declared
// type fails registration; the resource owns the name.
#[test]
fn derived_type_collision_fails_registration() {
    let error = build_error(|server| {
        server.declare_type(
            TypeDecl::union("session-ref", "A hand-declared reference").variant(
                Variant::new("id", "Locate by id").field(Field::string("id", "The session id")),
            ),
        );
        server.resource(
            ResourceDecl::new("session", "A live test session lease").uri("test://session/{id}"),
        );
        server.command("session status", |command| {
            command
                .summary("Inspect a session")
                .description("Reads the session lease state.")
                .arg(arg::named("target", "session-ref").summary("Which session to inspect"))
                .handle(|_context: CommandContext| async move {
                    Ok(CommandOutput::structured(json!({})))
                });
        });
    });
    assert!(
        error.contains("reference type `session-ref`")
            && error.contains("the resource owns that name"),
        "{error}"
    );
}

// Acceptance: `within` must form a tree — self-scoping, unknown parents,
// and cycles all fail registration.
#[test]
fn within_must_form_a_tree() {
    let self_scoped = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease")
                .uri("test://session/{id}")
                .within("session"),
        );
    });
    assert!(
        self_scoped.contains("scoped within itself"),
        "{self_scoped}"
    );

    let unknown_parent = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease")
                .uri("test://session/{id}")
                .within("workspace"),
        );
    });
    assert!(
        unknown_parent.contains("not a declared resource"),
        "{unknown_parent}"
    );

    let cycle = build_error(|server| {
        server.resource(
            ResourceDecl::new("session", "A live test session lease")
                .uri("test://session/{id}")
                .within("tab"),
        );
        server.resource(
            ResourceDecl::new("tab", "An open tab")
                .uri("test://tab/{id}")
                .within("session"),
        );
    });
    assert!(cycle.contains("scoping cycle"), "{cycle}");
}

// ---------------------------------------------------------------------------
// Runtime: minting, normalization, refusal
// ---------------------------------------------------------------------------

// Acceptance: grants appear in structured output as {resource, id, uri}
// with the URI minted from the declared template.
#[tokio::test]
async fn grants_mint_uris_in_structured_output() {
    let registry = session_registry();
    let response = registry
        .run(request("session start", json!({})))
        .await
        .expect("grant succeeds");
    let output = response.output.expect("executed output");
    assert_eq!(output.grants.len(), 1);
    let grant = &output.grants[0];
    assert_eq!(grant.resource, "session");
    assert_eq!(grant.id, "sess-1");
    assert_eq!(grant.uri, "test://session/sess-1");
}

// Acceptance: listings carry the reference array with minted URIs.
#[tokio::test]
async fn listings_mint_uris_for_every_reference() {
    let registry = session_registry();
    registry
        .run(request("session start", json!({})))
        .await
        .expect("first grant");
    registry
        .run(request("session start", json!({})))
        .await
        .expect("second grant");

    let response = registry
        .run(request("session list", json!({})))
        .await
        .expect("listing succeeds");
    let output = response.output.expect("executed output");
    assert_eq!(output.listings.len(), 2);
    let uris: Vec<_> = output
        .listings
        .iter()
        .map(|reference| reference.uri.as_str())
        .collect();
    assert_eq!(uris, vec!["test://session/sess-1", "test://session/sess-2"]);
}

// Acceptance: the derived reference type accepts a bare id and a full URI
// and normalizes — the resolver always sees the bare id.
#[tokio::test]
async fn carrier_accepts_bare_id_and_uri() {
    let status_runs = Arc::new(AtomicUsize::new(0));
    let registry =
        build_registry(RegistryOptions::default(), status_runs.clone()).expect("registry builds");
    registry
        .run(request("session start", json!({})))
        .await
        .expect("grant succeeds");

    registry
        .run(request(
            "session status --session-id $args.session_id",
            json!({ "session_id": "sess-1" }),
        ))
        .await
        .expect("bare id resolves");

    // The resolver checks the store for the bare id, so success here proves
    // the URI was normalized before resolution.
    registry
        .run(request(
            "session status --session-id $args.session_id",
            json!({ "session_id": "test://session/sess-1" }),
        ))
        .await
        .expect("full URI normalizes and resolves");

    assert_eq!(status_runs.load(Ordering::SeqCst), 2);
}

// Acceptance: a released resource stops resolving.
#[tokio::test]
async fn released_resource_stops_resolving() {
    let registry = session_registry();
    registry
        .run(request("session start", json!({})))
        .await
        .expect("grant succeeds");
    registry
        .run(request(
            "session end --session-id $args.session_id",
            json!({ "session_id": "sess-1" }),
        ))
        .await
        .expect("release succeeds");

    let error = registry
        .run(request(
            "session status --session-id $args.session_id",
            json!({ "session_id": "sess-1" }),
        ))
        .await
        .expect_err("released session refuses");
    assert!(
        matches!(&error, FrameworkError::ResourceRefused { .. }),
        "{error:?}"
    );
}

// Acceptance: a refusing resolver produces the structured refusal with
// derived recovery edges, and the handler body never runs.
#[tokio::test]
async fn refusal_carries_derived_recovery_edges_and_skips_handler() {
    let status_runs = Arc::new(AtomicUsize::new(0));
    let registry =
        build_registry(RegistryOptions::default(), status_runs.clone()).expect("registry builds");

    let error = registry
        .run(request(
            "session status --session-id $args.session_id",
            json!({ "session_id": "sess-9" }),
        ))
        .await
        .expect_err("unknown session refuses");

    let FrameworkError::ResourceRefused {
        resource,
        reference,
        detail,
        enumerate,
        establish,
    } = &error
    else {
        panic!("expected ResourceRefused, got {error:?}");
    };
    assert_eq!(resource, "session");
    assert_eq!(reference, "sess-9");
    assert!(detail.contains("not live"), "{detail}");
    assert_eq!(enumerate, &vec!["session list".to_string()]);
    assert_eq!(establish, &vec!["session start".to_string()]);
    assert_eq!(status_runs.load(Ordering::SeqCst), 0, "handler never ran");

    let envelope = ResponseEnvelope::framework_error(error.clone(), None, None);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["error"]["code"], json!("resource_refused"));
    assert_eq!(
        value["error"]["details"]["recover"]["enumerate"],
        json!(["session list"])
    );
    assert_eq!(
        value["error"]["details"]["recover"]["establish"],
        json!(["session start"])
    );
}

// Acceptance: a root resource granting without an enumerator registers;
// its refusal recovery edge names the establishing command.
#[tokio::test]
async fn root_resource_refusal_recovers_through_establisher() {
    let registry = build_registry(
        RegistryOptions {
            enumerator: false,
            ..Default::default()
        },
        Arc::new(AtomicUsize::new(0)),
    )
    .expect("root resource without enumerator registers");

    let error = registry
        .run(request(
            "session status --session-id $args.session_id",
            json!({ "session_id": "sess-9" }),
        ))
        .await
        .expect_err("unknown session refuses");

    let FrameworkError::ResourceRefused {
        enumerate,
        establish,
        ..
    } = &error
    else {
        panic!("expected ResourceRefused, got {error:?}");
    };
    assert!(enumerate.is_empty(), "no enumerator to recover through");
    assert_eq!(establish, &vec!["session start".to_string()]);
}

// Acceptance: a missing carrier fails with the capability-missing shape,
// naming the derived capability and its establishing commands.
#[tokio::test]
async fn missing_carrier_names_derived_capability_and_establishers() {
    let error = session_registry()
        .run(request("session status", json!({})))
        .await
        .expect_err("missing carrier fails");

    let FrameworkError::CapabilityMissing {
        capability,
        carrier,
        providers,
    } = &error
    else {
        panic!("expected CapabilityMissing, got {error:?}");
    };
    assert_eq!(capability, "session");
    assert_eq!(carrier, "session_id");
    assert_eq!(providers, &vec!["session start".to_string()]);
}

// Acceptance: a grant id that would not round-trip through the URI
// template is refused at mint time.
#[tokio::test]
async fn grant_with_unmintable_id_is_refused_at_mint_time() {
    let registry = build_registry(
        RegistryOptions {
            bad_grant_id: true,
            ..Default::default()
        },
        Arc::new(AtomicUsize::new(0)),
    )
    .expect("registry builds");

    let error = registry
        .run(request("session start", json!({})))
        .await
        .expect_err("unmintable id fails");
    assert!(
        error.to_string().contains("would not round-trip"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// Contract checks
// ---------------------------------------------------------------------------

// Acceptance: the fully declared lifecycle passes the resource projection
// contract check.
#[test]
fn contract_resource_projection_passes() {
    let violations = mcp_twill::contract::check_resource_projection(&session_registry());
    assert!(violations.is_empty(), "{violations:?}");
}

// Acceptance: the contract check reports resource edges that validation
// would reject on a hand-assembled registry.
#[test]
fn contract_reports_undeclared_resource_edges() {
    let mut spec = CommandSpec::new(["tab", "open"], "Open tab", "Opens a tab.");
    spec.requires_resources = vec!["tab".to_string()];
    let registry = CommandRegistry::new("resource-test", "Contract test server.")
        .register(spec, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let violations = mcp_twill::contract::check_resource_projection(&registry);
    assert!(
        !violations.is_empty(),
        "undeclared resource edges must violate the contract"
    );
}

// ---------------------------------------------------------------------------
// MCP surface
// ---------------------------------------------------------------------------

struct NullClient;

impl ClientHandler for NullClient {}

// Acceptance: grants of resources with a bound reader emit resource_link
// content parts, and the adapter serves resources/read for their URIs.
#[tokio::test]
async fn mcp_grants_emit_resource_links_and_serve_reads() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(session_registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = NullClient.serve(client_transport).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(json_object(request("session start", json!({})))?),
        )
        .await?;
    assert_ne!(result.is_error, Some(true));

    let content = serde_json::to_string(&result.content)?;
    assert!(content.contains("resource_link"), "{content}");
    assert!(content.contains("test://session/sess-1"), "{content}");
    let structured = serde_json::to_string(&result.structured_content)?;
    assert!(structured.contains("test://session/sess-1"), "{structured}");

    let read = client
        .read_resource(ReadResourceRequestParams::new("test://session/sess-1"))
        .await?;
    let read_json = serde_json::to_string(&read)?;
    assert!(read_json.contains("sess-1"), "{read_json}");
    assert!(read_json.contains("live"), "{read_json}");

    client.cancel().await?;
    Ok(())
}

// Acceptance: without a bound reader no resource_link emits — a link the
// server cannot serve is a dead link — but the structured payload still
// carries the minted URI.
#[tokio::test]
async fn mcp_grants_without_reader_emit_no_links() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = CliMcpServer::new(build_registry(
        RegistryOptions {
            reader: false,
            ..Default::default()
        },
        Arc::new(AtomicUsize::new(0)),
    )?)?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = NullClient.serve(client_transport).await?;

    let result = client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(json_object(request("session start", json!({})))?),
        )
        .await?;
    assert_ne!(result.is_error, Some(true));

    let content = serde_json::to_string(&result.content)?;
    assert!(!content.contains("resource_link"), "{content}");
    let structured = serde_json::to_string(&result.structured_content)?;
    assert!(structured.contains("test://session/sess-1"), "{structured}");

    let read = client
        .read_resource(ReadResourceRequestParams::new("test://session/sess-1"))
        .await;
    assert!(read.is_err(), "no reader, no resources/read");

    client.cancel().await?;
    Ok(())
}
