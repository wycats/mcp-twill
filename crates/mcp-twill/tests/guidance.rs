//! Acceptance tests for guidance decomposition (RFC 0011).

use mcp_twill::{
    Alternative, ArgSpec, CommandExample, CommandOutput, CommandRegistry, CommandSpec, Fallback,
    Field, HelpRequest, PermissionEffect, PermissionSpec, TypeDecl, Variant, contract,
};
use rmcp::ServerHandler;
use serde_json::json;

fn read_permission() -> PermissionSpec {
    PermissionSpec::new(PermissionEffect::Read, "issues", "Reads issue records")
}

fn list_spec() -> CommandSpec {
    CommandSpec::new(["issues", "list"], "List issues", "List issues")
        .with_permission(read_permission())
}

fn export_spec() -> CommandSpec {
    CommandSpec::new(["issues", "export"], "Export issues", "Export raw issue records")
        .with_permission(read_permission())
}

/// A union whose escape-hatch variant declares a fallback condition.
fn issue_ref_type() -> TypeDecl {
    TypeDecl::union("issue-ref", "How to identify an issue")
        .variant(
            Variant::new("number", "Locate by issue number")
                .field(Field::integer("number", "Issue number")),
        )
        .variant(
            Variant::new("search", "Locate by search query")
                .field(Field::string("query", "Search text"))
                .fallback("the issue number is not known"),
        )
}

fn guidance_registry() -> CommandRegistry {
    let mut create_example = CommandExample::new(
        "issues create --title $args.title",
        "Create an issue with a typed title",
    );
    create_example
        .args
        .insert("title".to_string(), json!("Crash"));

    CommandRegistry::new("guidance-test", "Guidance test server")
        .declare_preamble(
            "Issue records are the source of truth; keep them synchronized before acting on stale listings.",
        )
        .declare_type(issue_ref_type())
        .register(
            CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
                .with_arg(ArgSpec::string("title", "Issue title"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "issues",
                    "Creates issues",
                ))
                .with_example(create_example)
                .use_when("reporting a single new problem")
                .alternative("issues sync", "pulling issues that already exist remotely"),
            |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
        )
        .register(
            CommandSpec::new(["issues", "sync"], "Sync issues", "Sync issues")
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "issues",
                    "Syncs issue records",
                ))
                .with_example(CommandExample::new("issues sync", "Pull remote issue records")),
            |_context| async { Ok(CommandOutput::structured(json!({ "synced": 0 }))) },
        )
        .register(list_spec(), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        })
        .register(
            export_spec().fallback(
                ["issues list"],
                "structured listings do not capture the fields you need",
            ),
            |_context| async { Ok(CommandOutput::text("id,title")) },
        )
        .register(
            {
                let mut show_example = CommandExample::new(
                    "issues show --ref $args.ref",
                    "Show an issue located by number",
                );
                show_example
                    .args
                    .insert("ref".to_string(), json!({ "number": 1 }));
                CommandSpec::new(["issues", "show"], "Show issue", "Show one issue")
                    .with_arg(ArgSpec::named("ref", "issue-ref", "Issue to show"))
                    .with_permission(read_permission())
                    .with_example(show_example)
            },
            |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
        )
}

fn help_for(registry: &CommandRegistry, command: &str) -> String {
    registry
        .help(HelpRequest {
            command: Some(command.to_string()),
            topic: None,
            detail: None,
        })
        .text
}

// The guidance-rich registry satisfies every contract rule, including the
// guidance projection check.
mcp_twill::contract_tests!(guidance_registry);

// --- Help rendering ---

#[test]
fn help_renders_use_when() {
    let help = help_for(&guidance_registry(), "issues create");
    assert!(help.contains("Use when: reporting a single new problem"));
}

#[test]
fn help_renders_alternatives() {
    let help = help_for(&guidance_registry(), "issues create");
    assert!(help.contains("Use instead:"));
    assert!(help.contains("- `issues sync` — pulling issues that already exist remotely"));
}

#[test]
fn help_renders_fallback_declaration() {
    let help = help_for(&guidance_registry(), "issues export");
    assert!(help.contains(
        "Fallback: prefer `issues list`. Use only when structured listings do not capture the fields you need."
    ));
}

#[test]
fn help_renders_derived_fallback_edges_on_preferred_command() {
    // `issues list` never declared anything; the edge is derived from
    // `issues export`'s fallback declaration.
    let help = help_for(&guidance_registry(), "issues list");
    assert!(help.contains("Fallbacks:"));
    assert!(
        help.contains("- `issues export` — when structured listings do not capture the fields you need")
    );
}

#[test]
fn server_help_renders_preamble() {
    let help = guidance_registry()
        .help(HelpRequest {
            command: None,
            topic: None,
            detail: None,
        })
        .text;
    assert!(help.contains("Issue records are the source of truth"));
}

#[test]
fn type_help_renders_variant_fallback() {
    let help = help_for(&guidance_registry(), "issues show");
    assert!(help.contains("Type `issue-ref`: How to identify an issue"));
    assert!(help.contains("  - number: Locate by issue number"));
    assert!(
        help.contains("  - search (fallback — the issue number is not known): Locate by search query")
    );
}

// --- Catalog projection ---

#[test]
fn catalog_projects_guidance_declarations() {
    let catalog = guidance_registry().catalog();
    let create = catalog
        .operations
        .iter()
        .find(|operation| operation.id == "issues.create")
        .unwrap();
    assert_eq!(
        create.use_when.as_deref(),
        Some("reporting a single new problem")
    );
    assert_eq!(
        create.alternatives,
        vec![Alternative {
            command: "issues sync".to_string(),
            when: "pulling issues that already exist remotely".to_string(),
        }]
    );
    let export = catalog
        .operations
        .iter()
        .find(|operation| operation.id == "issues.export")
        .unwrap();
    assert_eq!(
        export.fallback,
        Some(Fallback {
            prefer: vec!["issues list".to_string()],
            when: "structured listings do not capture the fields you need".to_string(),
        })
    );
}

#[test]
fn catalog_projects_preamble() {
    let catalog = guidance_registry().catalog();
    assert_eq!(
        catalog.server.preamble.as_deref(),
        Some(
            "Issue records are the source of truth; keep them synchronized before acting on stale listings."
        )
    );
}

fn hash_with(customize: impl FnOnce(CommandSpec) -> CommandSpec) -> String {
    CommandRegistry::new("hash-test", "Hash test server")
        .register(list_spec(), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        })
        .register(customize(export_spec()), |_context| async {
            Ok(CommandOutput::text("id,title"))
        })
        .catalog_identity()
        .catalog_hash
}

#[test]
fn catalog_hash_covers_guidance_declarations() {
    let base = hash_with(|spec| spec);
    assert_ne!(base, hash_with(|spec| spec.use_when("raw records are required")));
    assert_ne!(
        base,
        hash_with(|spec| spec.alternative("issues list", "structured output is enough"))
    );
    assert_ne!(
        base,
        hash_with(|spec| spec.fallback(["issues list"], "structured output falls short"))
    );
}

#[test]
fn catalog_hash_covers_preamble() {
    let without = CommandRegistry::new("hash-test", "Hash test server")
        .register(list_spec(), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        })
        .catalog_identity()
        .catalog_hash;
    let with = CommandRegistry::new("hash-test", "Hash test server")
        .declare_preamble("Records first.")
        .register(list_spec(), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        })
        .catalog_identity()
        .catalog_hash;
    assert_ne!(without, with);
}

// --- Validation failures ---

fn guidance_error(spec: CommandSpec) -> String {
    CommandRegistry::new("guidance-test", "Guidance failure test server")
        .register(list_spec(), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        })
        .register(spec, |_context| async {
            Ok(CommandOutput::text("id,title"))
        })
        .validate_guidance()
        .unwrap_err()
        .to_string()
}

#[test]
fn empty_preamble_is_rejected() {
    let error = CommandRegistry::new("guidance-test", "Guidance test server")
        .declare_preamble("   ")
        .validate_guidance()
        .unwrap_err();
    assert!(error.to_string().contains("server preamble is empty"));
}

#[test]
fn empty_use_when_is_rejected() {
    let error = guidance_error(export_spec().use_when("  "));
    assert!(error.contains("declares an empty `use_when`"));
}

#[test]
fn use_when_and_fallback_are_mutually_exclusive() {
    let error = guidance_error(
        export_spec()
            .use_when("raw records are required")
            .fallback(["issues list"], "structured output falls short"),
    );
    assert!(error.contains("declares both `use_when` and `fallback`"));
}

#[test]
fn alternative_with_empty_condition_is_rejected() {
    let error = guidance_error(export_spec().alternative("issues list", "  "));
    assert!(error.contains("with an empty condition"));
}

#[test]
fn self_alternative_is_rejected() {
    let error = guidance_error(export_spec().alternative("issues export", "always"));
    assert!(error.contains("lists itself as an alternative"));
}

#[test]
fn dangling_alternative_is_rejected() {
    let error = guidance_error(export_spec().alternative("issues nope", "never"));
    assert!(error.contains("not a catalog command"));
}

#[test]
fn duplicate_alternative_is_rejected() {
    let error = guidance_error(
        export_spec()
            .alternative("issues list", "one condition")
            .alternative("issues list", "another condition"),
    );
    assert!(error.contains("more than once"));
}

#[test]
fn fallback_with_empty_condition_is_rejected() {
    let error = guidance_error(export_spec().fallback(["issues list"], "  "));
    assert!(error.contains("declares a fallback with an empty condition"));
}

#[test]
fn fallback_with_empty_prefer_list_is_rejected() {
    let error = guidance_error(export_spec().fallback(Vec::<String>::new(), "always"));
    assert!(error.contains("empty `prefer` list"));
}

#[test]
fn fallback_preferring_itself_is_rejected() {
    let error = guidance_error(export_spec().fallback(["issues export"], "always"));
    assert!(error.contains("prefers itself"));
}

#[test]
fn dangling_fallback_preference_is_rejected() {
    let error = guidance_error(export_spec().fallback(["issues nope"], "always"));
    assert!(error.contains("not a catalog command"));
}

#[test]
fn duplicate_fallback_preference_is_rejected() {
    let error = guidance_error(export_spec().fallback(["issues list", "issues list"], "always"));
    assert!(error.contains("more than once"));
}

#[test]
fn fallback_preference_cycle_is_rejected() {
    let error = CommandRegistry::new("guidance-test", "Guidance test server")
        .register(
            CommandSpec::new(["alpha"], "Alpha", "Alpha command")
                .with_permission(read_permission())
                .fallback(["beta"], "beta does not apply"),
            |_context| async { Ok(CommandOutput::text("alpha")) },
        )
        .register(
            CommandSpec::new(["beta"], "Beta", "Beta command")
                .with_permission(read_permission())
                .fallback(["alpha"], "alpha does not apply"),
            |_context| async { Ok(CommandOutput::text("beta")) },
        )
        .validate_guidance()
        .unwrap_err();
    assert!(error.to_string().contains("fallback preference cycle"));
}

#[test]
fn variant_fallback_with_empty_condition_is_rejected() {
    let error = CommandRegistry::new("guidance-test", "Guidance test server")
        .declare_type(
            TypeDecl::union("issue-ref", "How to identify an issue")
                .variant(
                    Variant::new("number", "Locate by issue number")
                        .field(Field::integer("number", "Issue number")),
                )
                .variant(
                    Variant::new("search", "Locate by search query")
                        .field(Field::string("query", "Search text"))
                        .fallback("  "),
                ),
        )
        .validate_guidance()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("declares a fallback with an empty condition")
    );
}

#[test]
fn union_with_fallback_on_every_variant_is_rejected() {
    let error = CommandRegistry::new("guidance-test", "Guidance test server")
        .declare_type(
            TypeDecl::union("issue-ref", "How to identify an issue")
                .variant(
                    Variant::new("number", "Locate by issue number")
                        .field(Field::integer("number", "Issue number"))
                        .fallback("search is unavailable"),
                )
                .variant(
                    Variant::new("search", "Locate by search query")
                        .field(Field::string("query", "Search text"))
                        .fallback("the issue number is not known"),
                ),
        )
        .validate_guidance()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("declares a fallback on every variant")
    );
}

// --- Serving path ---

#[test]
fn serving_path_rejects_invalid_guidance() {
    let registry = CommandRegistry::new("invalid", "Invalid guidance").register(
        CommandSpec::new(["demo"], "Demo", "Demo command")
            .with_permission(read_permission())
            .alternative("missing command", "the demo does not apply"),
        |_context| async { Ok(CommandOutput::text("ok")) },
    );
    let error = match mcp_twill::CliMcpServer::new(registry) {
        Ok(_) => panic!("serving invalid guidance must fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("not a catalog command"));
}

// --- MCP surface ---

#[test]
fn mcp_instructions_lead_with_preamble() {
    let server = mcp_twill::CliMcpServer::new(guidance_registry()).unwrap();
    let instructions = server.get_info().instructions.unwrap();
    assert!(instructions.starts_with("Issue records are the source of truth"));
    assert!(instructions.contains("Use `help` to discover command templates"));
}

// --- Builder surface ---

fn built_registry() -> CommandRegistry {
    CommandRegistry::build(
        "guidance-example",
        "Builder guidance example server.",
        |server| {
            server.preamble("Issue records are the source of truth.");
            server.command("issues list", |command| {
                command
                    .summary("List issues")
                    .description("Lists open issues.")
                    .read("issues", "Reads issue records")
                    .handle(|_context| async { Ok(CommandOutput::structured(json!([]))) });
            });
            server.command("issues export", |command| {
                command
                    .summary("Export issues")
                    .description("Exports raw issue records.")
                    .read("issues", "Reads issue records")
                    .fallback(
                        ["issues list"],
                        "structured listings do not capture the fields you need",
                    )
                    .handle(|_context| async { Ok(CommandOutput::text("id,title")) });
            });
            server.command("issues audit", |command| {
                command
                    .summary("Audit issues")
                    .description("Audits issue history.")
                    .read("issues", "Reads issue records")
                    .use_when("verifying the history of a record")
                    .alternative("issues list", "you only need current state")
                    .handle(|_context| async { Ok(CommandOutput::structured(json!([]))) });
            });
        },
    )
    .unwrap()
}

#[test]
fn builder_wires_guidance_through() {
    let registry = built_registry();
    assert_eq!(
        registry.preamble(),
        Some("Issue records are the source of truth.")
    );
    let audit = help_for(&registry, "issues audit");
    assert!(audit.contains("Use when: verifying the history of a record"));
    assert!(audit.contains("- `issues list` — you only need current state"));
    let export = help_for(&registry, "issues export");
    assert!(export.contains(
        "Fallback: prefer `issues list`. Use only when structured listings do not capture the fields you need."
    ));
    let list = help_for(&registry, "issues list");
    assert!(list.contains("Fallbacks:"));
    assert!(
        list.contains("- `issues export` — when structured listings do not capture the fields you need")
    );
}

#[test]
fn builder_surfaces_guidance_errors() {
    let result = CommandRegistry::build("broken", "Broken guidance server", |server| {
        server.command("solo", |command| {
            command
                .summary("Solo")
                .description("A lone command.")
                .read("solo", "Reads solo records")
                .alternative("missing", "the record is remote")
                .handle(|_context| async { Ok(CommandOutput::text("ok")) });
        });
    });
    let error = match result {
        Ok(_) => panic!("expected registry construction to fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("not a catalog command"));
}

// --- Contract surface ---

#[test]
fn contract_accepts_guidance_projection() {
    let violations = contract::check_guidance_projection(&guidance_registry());
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn contract_flags_preamble_that_names_commands() {
    let registry = CommandRegistry::new("guidance-test", "Guidance test server")
        .declare_preamble("Always call `issues list` before mutating.")
        .register(list_spec(), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        });
    let violations = contract::check_guidance_projection(&registry);
    assert!(
        violations
            .iter()
            .any(|violation| violation
                .to_string()
                .contains("per-command steering belongs on the command")),
        "{violations:?}"
    );
}
