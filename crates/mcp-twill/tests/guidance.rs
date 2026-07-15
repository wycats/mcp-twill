//! Acceptance tests for guidance decomposition (RFC 0011).

use std::sync::{Arc, Mutex};

use mcp_twill::{
    Alternative, ArgSpec, CommandExample, CommandOutput, CommandRegistry, CommandSpec,
    ConversationIdentity, Fallback, Field, FrameworkEvent, HelpRequest, InvocationContext,
    PermissionEffect, PermissionSpec, PlanFacts, ResponseEnvelope, ResponseProfile, RunMode,
    RunRequest, ServerSpec, TypeDecl, Variant, contract,
};
use rmcp::ServerHandler;
use serde_json::json;

fn success(outcome: mcp_twill::CommandExecutionOutcome) -> mcp_twill::RunResponse {
    match outcome {
        mcp_twill::CommandExecutionOutcome::Success(response) => response,
        mcp_twill::CommandExecutionOutcome::ApplicationError { error, .. } => {
            panic!("unexpected application error: {}", error.code)
        }
    }
}

fn read_permission() -> PermissionSpec {
    PermissionSpec::new(PermissionEffect::Read, "issues", "Reads issue records")
}

fn list_spec() -> CommandSpec {
    CommandSpec::new(["issues", "list"], "List issues", "List issues")
        .with_permission(read_permission())
}

fn export_spec() -> CommandSpec {
    CommandSpec::new(
        ["issues", "export"],
        "Export issues",
        "Export raw issue records",
    )
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

fn run_request(command: &str, args: serde_json::Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value(args).unwrap(),
        stdin: None,
        output: None,
        mode: RunMode::Execute,
        approval: None,
        dry_run: false,
    }
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
    assert!(help.contains(
        "- `issues export` — when structured listings do not capture the fields you need"
    ));
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
        help.contains(
            "  - search (fallback — the issue number is not known): Locate by search query"
        )
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
    assert_ne!(
        base,
        hash_with(|spec| spec.use_when("raw records are required"))
    );
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

fn hash_with_issue_ref_type(decl: TypeDecl) -> String {
    let type_name = decl.name.clone();
    CommandRegistry::new("hash-test", "Hash test server")
        .declare_type(decl)
        .register(
            CommandSpec::new(["issues", "show"], "Show issue", "Show one issue")
                .with_arg(ArgSpec::named("ref", type_name, "Issue to show"))
                .with_permission(read_permission()),
            |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
        )
        .catalog_identity()
        .catalog_hash
}

#[test]
fn catalog_hash_covers_variant_fallback() {
    let with = issue_ref_type();
    let mut without = with.clone();
    without.variants[1].fallback = None;
    assert_ne!(
        hash_with_issue_ref_type(without),
        hash_with_issue_ref_type(with)
    );
}

#[test]
fn fallback_authoring_forms_normalize_identically() {
    let borrowed_values = ["issues list", "issues sync"];
    let array = export_spec().fallback(
        ["issues list", "issues sync"],
        "structured output falls short",
    );
    let borrowed =
        export_spec().fallback(borrowed_values.as_slice(), "structured output falls short");
    let owned = export_spec().fallback(
        vec!["issues list".to_string(), "issues sync".to_string()],
        "structured output falls short",
    );

    assert_eq!(array.fallback, borrowed.fallback);
    assert_eq!(array.fallback, owned.fallback);

    let hash = |spec: CommandSpec| {
        CommandRegistry::new("fallback-forms", "Fallback authoring forms")
            .register(list_spec(), |_context| async {
                Ok(CommandOutput::structured(json!([])))
            })
            .register(
                CommandSpec::new(["issues", "sync"], "Sync issues", "Sync issues")
                    .with_permission(read_permission()),
                |_context| async { Ok(CommandOutput::structured(json!([]))) },
            )
            .register(spec, |_context| async {
                Ok(CommandOutput::text("id,title"))
            })
            .catalog_identity()
            .catalog_hash
    };
    assert_eq!(hash(array), hash(borrowed));
    assert_eq!(
        hash(owned),
        hash(export_spec().fallback(
            ["issues list", "issues sync"],
            "structured output falls short",
        ))
    );
}

#[test]
fn legacy_and_explicit_empty_guidance_json_are_identical() {
    let mut legacy_command = serde_json::to_value(list_spec()).unwrap();
    let mut explicit_command = legacy_command.clone();
    let explicit = explicit_command.as_object_mut().unwrap();
    explicit.insert("useWhen".to_string(), serde_json::Value::Null);
    explicit.insert("alternatives".to_string(), json!([]));
    explicit.insert("fallback".to_string(), serde_json::Value::Null);
    let legacy: CommandSpec = serde_json::from_value(legacy_command.take()).unwrap();
    let explicit: CommandSpec = serde_json::from_value(explicit_command).unwrap();
    assert_eq!(legacy, explicit);
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::to_value(&explicit).unwrap()
    );

    let mut legacy_variant = serde_json::to_value(
        Variant::new("number", "Locate by issue number")
            .field(Field::integer("number", "Issue number")),
    )
    .unwrap();
    let mut explicit_variant = legacy_variant.clone();
    explicit_variant
        .as_object_mut()
        .unwrap()
        .insert("fallback".to_string(), serde_json::Value::Null);
    let legacy: Variant = serde_json::from_value(legacy_variant.take()).unwrap();
    let explicit: Variant = serde_json::from_value(explicit_variant).unwrap();
    assert_eq!(legacy, explicit);

    let server = ServerSpec::new("legacy", "Legacy server");
    let mut legacy_server = serde_json::to_value(&server).unwrap();
    let mut explicit_server = legacy_server.clone();
    explicit_server
        .as_object_mut()
        .unwrap()
        .insert("preamble".to_string(), serde_json::Value::Null);
    let legacy: ServerSpec = serde_json::from_value(legacy_server.take()).unwrap();
    let explicit: ServerSpec = serde_json::from_value(explicit_server).unwrap();
    assert_eq!(legacy, explicit);
    assert_eq!(
        serde_json::to_value(legacy).unwrap(),
        serde_json::to_value(explicit).unwrap()
    );
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

fn registry_build_error(result: mcp_twill::Result<CommandRegistry>) -> String {
    match result {
        Ok(_) => panic!("expected registry construction to fail"),
        Err(error) => error.to_string(),
    }
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

#[test]
fn every_disallowed_guidance_scalar_is_rejected() {
    let mut scalars = (0_u32..=0x1F)
        .chain(0x7F..=0x9F)
        .filter_map(char::from_u32)
        .collect::<Vec<_>>();
    scalars.extend([
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{2028}', '\u{2029}', '\u{202A}', '\u{202B}',
        '\u{202C}', '\u{202D}', '\u{202E}', '\u{2060}', '\u{2061}', '\u{2062}', '\u{2063}',
        '\u{2064}', '\u{2065}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{206A}',
        '\u{206B}', '\u{206C}', '\u{206D}', '\u{206E}', '\u{206F}', '\u{FEFF}',
    ]);

    for scalar in scalars {
        let text = format!("safe{scalar}text");
        let error = guidance_error(export_spec().use_when(text));
        assert!(
            error.contains(&format!("U+{:04X}", scalar as u32)),
            "scalar U+{:04X}: {error}",
            scalar as u32
        );
    }
}

#[test]
fn every_guidance_text_slot_uses_the_display_safety_rule() {
    let unsafe_text = "safe\u{202E}text";

    let preamble = CommandRegistry::new("guidance-test", "Guidance test server")
        .declare_preamble(unsafe_text)
        .validate_guidance()
        .unwrap_err()
        .to_string();
    assert!(preamble.contains("server `guidance-test` preamble"));

    let use_when = guidance_error(export_spec().use_when(unsafe_text));
    assert!(use_when.contains("command `issues export` `use_when` condition"));

    let alternative = guidance_error(export_spec().alternative("issues list", unsafe_text));
    assert!(alternative.contains("alternative `issues list` condition"));

    let fallback = guidance_error(export_spec().fallback(["issues list"], unsafe_text));
    assert!(fallback.contains("command `issues export` fallback condition"));

    let variant = CommandRegistry::new("guidance-test", "Guidance test server")
        .declare_type(
            TypeDecl::union("issue-ref", "How to identify an issue")
                .variant(
                    Variant::new("number", "Locate by issue number")
                        .field(Field::integer("number", "Issue number")),
                )
                .variant(
                    Variant::new("search", "Locate by search query")
                        .field(Field::string("query", "Search text"))
                        .fallback(unsafe_text),
                ),
        )
        .validate_guidance()
        .unwrap_err()
        .to_string();
    assert!(variant.contains("type `issue-ref` variant `search` fallback condition"));
}

#[test]
fn guidance_text_bound_and_byte_preservation_are_exact() {
    let at_limit = "x".repeat(1_024);
    let registry = CommandRegistry::new("guidance-test", "Guidance test server")
        .register(list_spec().use_when(at_limit.clone()), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        });
    registry.validate_guidance().unwrap();
    let operation = &registry.catalog().operations[0];
    assert_eq!(operation.use_when.as_deref(), Some(at_limit.as_str()));
    assert!(help_for(&registry, "issues list").contains(&format!("Use when: {at_limit}")));

    let over_limit = guidance_error(export_spec().use_when("x".repeat(1_025)));
    assert!(over_limit.contains("1,024 Unicode scalar limit"));
    assert!(over_limit.contains("1025"));

    let preserved = "  café ✅  ";
    let registry = CommandRegistry::new("guidance-test", "Guidance test server")
        .register(list_spec().use_when(preserved), |_context| async {
            Ok(CommandOutput::structured(json!([])))
        });
    registry.validate_guidance().unwrap();
    assert_eq!(
        registry.catalog().operations[0].use_when.as_deref(),
        Some(preserved)
    );
    assert!(help_for(&registry, "issues list").contains(&format!("Use when: {preserved}")));
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

#[test]
fn serving_path_rejects_display_unsafe_guidance() {
    let registry = CommandRegistry::new("invalid", "Invalid guidance").register(
        CommandSpec::new(["demo"], "Demo", "Demo command")
            .with_permission(read_permission())
            .use_when("safe\u{202E}text"),
        |_context| async { Ok(CommandOutput::text("ok")) },
    );
    let error = match mcp_twill::CliMcpServer::new(registry) {
        Ok(_) => panic!("serving display-unsafe guidance must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("presentation-unsafe scalar U+202E")
    );
}

fn execution_guidance_registry(dispatches: Arc<Mutex<Vec<&'static str>>>) -> CommandRegistry {
    let preferred_dispatches = dispatches.clone();
    let alternative_dispatches = dispatches.clone();
    CommandRegistry::new("execution", "Guidance is not runtime state")
        .register(
            CommandSpec::new(["preferred"], "Preferred", "Preferred command")
                .with_permission(read_permission())
                .use_when("the ordinary path applies")
                .alternative("alternative", "the neighboring case applies"),
            move |_context| {
                let dispatches = preferred_dispatches.clone();
                async move {
                    dispatches.lock().unwrap().push("preferred");
                    Ok(CommandOutput::text("preferred"))
                }
            },
        )
        .register(
            CommandSpec::new(["alternative"], "Alternative", "Alternative command")
                .with_permission(read_permission()),
            move |_context| {
                let dispatches = alternative_dispatches.clone();
                async move {
                    dispatches.lock().unwrap().push("alternative");
                    Ok(CommandOutput::text("alternative"))
                }
            },
        )
        .register(
            CommandSpec::new(["escape"], "Escape", "Escape hatch")
                .with_permission(read_permission())
                .fallback(
                    ["preferred"],
                    "the ordinary path cannot represent the request",
                ),
            move |_context| {
                let dispatches = dispatches.clone();
                async move {
                    dispatches.lock().unwrap().push("escape");
                    Ok(CommandOutput::text("escape"))
                }
            },
        )
}

#[tokio::test]
async fn guidance_never_gates_planning_or_execution() {
    for reverse_registration in [false, true] {
        let dispatches = Arc::new(Mutex::new(Vec::new()));
        let registry = if reverse_registration {
            // BTree-backed registration canonicalizes command order; use the
            // same declarations with the escape hatch registered first.
            let dispatches_for_escape = dispatches.clone();
            let dispatches_for_preferred = dispatches.clone();
            let dispatches_for_alternative = dispatches.clone();
            CommandRegistry::new("execution", "Guidance is not runtime state")
                .register(
                    CommandSpec::new(["escape"], "Escape", "Escape hatch")
                        .with_permission(read_permission())
                        .fallback(
                            ["preferred"],
                            "the ordinary path cannot represent the request",
                        ),
                    move |_context| {
                        let dispatches = dispatches_for_escape.clone();
                        async move {
                            dispatches.lock().unwrap().push("escape");
                            Ok(CommandOutput::text("escape"))
                        }
                    },
                )
                .register(
                    CommandSpec::new(["alternative"], "Alternative", "Alternative command")
                        .with_permission(read_permission()),
                    move |_context| {
                        let dispatches = dispatches_for_alternative.clone();
                        async move {
                            dispatches.lock().unwrap().push("alternative");
                            Ok(CommandOutput::text("alternative"))
                        }
                    },
                )
                .register(
                    CommandSpec::new(["preferred"], "Preferred", "Preferred command")
                        .with_permission(read_permission())
                        .use_when("the ordinary path applies")
                        .alternative("alternative", "the neighboring case applies"),
                    move |_context| {
                        let dispatches = dispatches_for_preferred.clone();
                        async move {
                            dispatches.lock().unwrap().push("preferred");
                            Ok(CommandOutput::text("preferred"))
                        }
                    },
                )
        } else {
            execution_guidance_registry(dispatches.clone())
        };
        registry.validate_guidance().unwrap();
        let catalog_before = registry.catalog();
        let help_before = help_for(&registry, "preferred");

        for command in ["escape", "preferred", "alternative", "escape"] {
            let response = success(registry.run(run_request(command, json!({}))).await.unwrap());
            assert!(!response.dry_run);
            assert!(response.output.is_some());
        }

        assert_eq!(
            *dispatches.lock().unwrap(),
            ["escape", "preferred", "alternative", "escape"]
        );
        assert_eq!(registry.catalog(), catalog_before);
        assert_eq!(help_for(&registry, "preferred"), help_before);
    }
}

#[tokio::test]
async fn runtime_values_never_become_guidance_or_guidance_owned_state() {
    let registry = CommandRegistry::new("runtime", "Runtime isolation")
        .declare_preamble("Static preamble text")
        .register(
            CommandSpec::new(["echo"], "Echo", "Echo one runtime value")
                .with_arg(ArgSpec::string("value", "Value to echo"))
                .with_permission(read_permission())
                .use_when("echoing one caller-supplied value"),
            |context: mcp_twill::CommandContext| async move {
                Ok(CommandOutput::structured(json!({
                    "echo": context.plan.bound_args["value"].value.clone()
                })))
            },
        );
    registry.validate_guidance().unwrap();
    let catalog_before = serde_json::to_value(registry.catalog()).unwrap();
    let help_before = help_for(&registry, "echo");
    let first_request = run_request("echo --value $args.value", json!({ "value": "runtime-a" }));
    let second_request = run_request("echo --value $args.value", json!({ "value": "runtime-b" }));
    let context = InvocationContext::new().with_conversation_identity(
        ConversationIdentity::new("com.example.host", "private-thread").unwrap(),
    );

    let first_plan = registry.build_plan(&first_request).unwrap();
    let contextual_plan = registry
        .build_plan_with_context(&first_request, &context)
        .unwrap();
    let second_plan = registry.build_plan(&second_request).unwrap();
    assert_eq!(
        first_plan.invocation_fingerprint,
        contextual_plan.invocation_fingerprint
    );
    assert_ne!(
        first_plan.invocation_fingerprint,
        second_plan.invocation_fingerprint
    );

    let first_response = success(registry.run(first_request).await.unwrap());
    let response = success(
        registry
            .run_with_context(second_request, context)
            .await
            .unwrap(),
    );
    assert_ne!(first_response.output, response.output);
    let envelope = ResponseEnvelope::success(response.clone(), ResponseProfile::Debug);
    let preview = ResponseEnvelope::preview(second_plan, false);
    let event = FrameworkEvent::from_envelope(&envelope, Some(&PlanFacts::from(&response.plan)));
    for value in [
        serde_json::to_value(&response.plan).unwrap(),
        serde_json::to_value(&envelope).unwrap(),
        serde_json::to_value(&preview).unwrap(),
        serde_json::to_value(&event).unwrap(),
    ] {
        let text = value.to_string();
        assert!(
            !text.contains("echoing one caller-supplied value"),
            "{text}"
        );
        assert!(!text.contains("Static preamble text"), "{text}");
        assert!(!text.contains("useWhen"), "{text}");
        assert!(!text.contains("alternatives"), "{text}");
        assert!(!text.contains("fallback"), "{text}");
        assert!(!text.contains("preamble"), "{text}");
        assert!(!text.contains("private-thread"), "{text}");
    }

    assert_eq!(
        serde_json::to_value(registry.catalog()).unwrap(),
        catalog_before
    );
    assert_eq!(help_for(&registry, "echo"), help_before);
    assert!(!catalog_before.to_string().contains("runtime-a"));
    assert!(!catalog_before.to_string().contains("runtime-b"));
    assert!(!help_before.contains("runtime-a"));
    assert!(!help_before.contains("runtime-b"));
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
            server.declare_type(issue_ref_type());
            server.command("issues list", |command| {
                command
                    .summary("List issues")
                    .description("Lists open issues.")
                    .read("issues", "Reads issue records")
                    .handle(|_context| async { Ok(CommandOutput::structured(json!([]))) });
            });
            server.command("issues export", |command| {
                let preferred = ["issues list"];
                command
                    .summary("Export issues")
                    .description("Exports raw issue records.")
                    .read("issues", "Reads issue records")
                    .fallback(
                        preferred.as_slice(),
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
            server.command("issues show", |command| {
                command
                    .summary("Show issue")
                    .description("Shows one issue.")
                    .arg(mcp_twill::arg::named("ref", "issue-ref").summary("Issue to show"))
                    .read("issues", "Reads issue records")
                    .handle(|_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) });
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
    assert!(list.contains(
        "- `issues export` — when structured listings do not capture the fields you need"
    ));
}

fn equivalent_low_level_registry() -> CommandRegistry {
    CommandRegistry::new("guidance-example", "Builder guidance example server.")
        .declare_preamble("Issue records are the source of truth.")
        .declare_type(issue_ref_type())
        .register(
            CommandSpec::new(["issues", "list"], "List issues", "Lists open issues.")
                .with_permission(read_permission()),
            |_context| async { Ok(CommandOutput::structured(json!([]))) },
        )
        .register(
            CommandSpec::new(
                ["issues", "export"],
                "Export issues",
                "Exports raw issue records.",
            )
            .with_permission(read_permission())
            .fallback(
                ["issues list"],
                "structured listings do not capture the fields you need",
            ),
            |_context| async { Ok(CommandOutput::text("id,title")) },
        )
        .register(
            CommandSpec::new(["issues", "audit"], "Audit issues", "Audits issue history.")
                .with_permission(read_permission())
                .use_when("verifying the history of a record")
                .alternative("issues list", "you only need current state"),
            |_context| async { Ok(CommandOutput::structured(json!([]))) },
        )
        .register(
            CommandSpec::new(["issues", "show"], "Show issue", "Shows one issue.")
                .with_arg(ArgSpec::named("ref", "issue-ref", "Issue to show"))
                .with_permission(read_permission()),
            |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
        )
}

#[test]
fn builder_and_low_level_guidance_are_equivalent() {
    let low_level = equivalent_low_level_registry();
    let builder = built_registry();
    low_level.validate_guidance().unwrap();
    builder.validate_guidance().unwrap();
    assert_eq!(low_level.preamble(), builder.preamble());
    assert_eq!(low_level.catalog(), builder.catalog());
    assert_eq!(
        low_level.catalog_identity().catalog_hash,
        builder.catalog_identity().catalog_hash
    );
    assert_eq!(
        low_level.help(HelpRequest::default()).text,
        builder.help(HelpRequest::default()).text
    );
    for command in [
        "issues list",
        "issues export",
        "issues audit",
        "issues show",
    ] {
        assert_eq!(help_for(&low_level, command), help_for(&builder, command));
    }
}

#[test]
fn builder_and_low_level_guidance_validation_failures_are_equivalent() {
    let low_level = CommandRegistry::new("broken", "Broken guidance server")
        .register(
            CommandSpec::new(["solo"], "Solo", "A lone command.")
                .with_permission(PermissionSpec::read("solo", "Reads solo records"))
                .alternative("missing", "the record is remote"),
            |_context| async { Ok(CommandOutput::text("ok")) },
        )
        .validate_guidance()
        .unwrap_err()
        .to_string();
    let builder = registry_build_error(CommandRegistry::build(
        "broken",
        "Broken guidance server",
        |server| {
            server.command("solo", |command| {
                command
                    .summary("Solo")
                    .description("A lone command.")
                    .read("solo", "Reads solo records")
                    .alternative("missing", "the record is remote")
                    .handle(|_context| async { Ok(CommandOutput::text("ok")) });
            });
        },
    ));
    assert_eq!(low_level, builder);
}

#[test]
fn low_level_guidance_setters_replace_visible_values() {
    let spec = export_spec()
        .use_when("first condition")
        .use_when("second condition")
        .fallback(["issues list"], "first fallback")
        .fallback(["issues sync"], "second fallback");
    assert_eq!(spec.use_when.as_deref(), Some("second condition"));
    assert_eq!(
        spec.fallback,
        Some(Fallback {
            prefer: vec!["issues sync".to_string()],
            when: "second fallback".to_string(),
        })
    );

    let variant = Variant::new("search", "Search")
        .fallback("first condition")
        .fallback("second condition");
    assert_eq!(variant.fallback.as_deref(), Some("second condition"));

    let registry = CommandRegistry::new("replace", "Replacement semantics")
        .declare_preamble("first preamble")
        .declare_preamble("second preamble");
    assert_eq!(registry.preamble(), Some("second preamble"));
}

#[test]
fn mutable_builders_reject_repeated_guidance_assignments() {
    let preamble = registry_build_error(CommandRegistry::build(
        "repeated",
        "Repeated preamble",
        |server| {
            server.preamble("same").preamble("same");
        },
    ));
    assert!(preamble.contains("server `repeated` assigns `preamble` more than once"));

    let use_when = registry_build_error(CommandRegistry::build(
        "repeated",
        "Repeated use_when",
        |server| {
            server.command("demo", |command| {
                command
                    .summary("Demo")
                    .description("Demo")
                    .read("demo", "Reads demo state")
                    .use_when("same")
                    .use_when("same")
                    .handle(|_context| async { Ok(CommandOutput::text("ok")) });
            });
        },
    ));
    assert!(use_when.contains("command `demo` assigns `use_when` more than once"));

    let fallback = registry_build_error(CommandRegistry::build(
        "repeated",
        "Repeated fallback",
        |server| {
            server.command("preferred", |command| {
                command
                    .summary("Preferred")
                    .description("Preferred")
                    .read("demo", "Reads demo state")
                    .handle(|_context| async { Ok(CommandOutput::text("ok")) });
            });
            server.command("fallback", |command| {
                command
                    .summary("Fallback")
                    .description("Fallback")
                    .read("demo", "Reads demo state")
                    .fallback(["preferred"], "same")
                    .fallback(["preferred"], "same")
                    .handle(|_context| async { Ok(CommandOutput::text("ok")) });
            });
        },
    ));
    assert!(fallback.contains("command `fallback` assigns `fallback` more than once"));
}

fn reverse_fallback_registry(reverse_registration: bool) -> CommandRegistry {
    let registry = CommandRegistry::new("order", "Reverse fallback order").register(
        CommandSpec::new(["preferred"], "Preferred", "Preferred command")
            .with_permission(read_permission()),
        |_context| async { Ok(CommandOutput::text("preferred")) },
    );
    let alpha = CommandSpec::new(["alpha"], "Alpha", "Alpha fallback")
        .with_permission(read_permission())
        .fallback(["preferred"], "alpha is required");
    let zeta = CommandSpec::new(["zeta"], "Zeta", "Zeta fallback")
        .with_permission(read_permission())
        .fallback(["preferred"], "zeta is required");
    if reverse_registration {
        registry
            .register(zeta, |_context| async { Ok(CommandOutput::text("zeta")) })
            .register(alpha, |_context| async { Ok(CommandOutput::text("alpha")) })
    } else {
        registry
            .register(alpha, |_context| async { Ok(CommandOutput::text("alpha")) })
            .register(zeta, |_context| async { Ok(CommandOutput::text("zeta")) })
    }
}

#[test]
fn reverse_fallback_help_is_canonical_across_registration_order() {
    let forward = help_for(&reverse_fallback_registry(false), "preferred");
    let reverse = help_for(&reverse_fallback_registry(true), "preferred");
    assert_eq!(forward, reverse);
    assert!(forward.find("`alpha`").unwrap() < forward.find("`zeta`").unwrap());
}

#[test]
fn authored_guidance_order_is_visible_and_hash_significant() {
    let hash = |spec: CommandSpec| {
        CommandRegistry::new("order", "Authored guidance order")
            .register(
                CommandSpec::new(["one"], "One", "First target").with_permission(read_permission()),
                |_context| async { Ok(CommandOutput::text("one")) },
            )
            .register(
                CommandSpec::new(["two"], "Two", "Second target")
                    .with_permission(read_permission()),
                |_context| async { Ok(CommandOutput::text("two")) },
            )
            .register(spec, |_context| async { Ok(CommandOutput::text("source")) })
            .catalog_identity()
            .catalog_hash
    };

    let alternatives_forward = CommandSpec::new(["source"], "Source", "Source command")
        .with_permission(read_permission())
        .alternative("one", "first case")
        .alternative("two", "second case");
    let alternatives_reverse = CommandSpec::new(["source"], "Source", "Source command")
        .with_permission(read_permission())
        .alternative("two", "second case")
        .alternative("one", "first case");
    assert_ne!(hash(alternatives_forward), hash(alternatives_reverse));

    let fallback_forward = CommandSpec::new(["source"], "Source", "Source command")
        .with_permission(read_permission())
        .fallback(["one", "two"], "neither preferred path applies");
    let fallback_reverse = CommandSpec::new(["source"], "Source", "Source command")
        .with_permission(read_permission())
        .fallback(["two", "one"], "neither preferred path applies");
    assert_ne!(hash(fallback_forward), hash(fallback_reverse));
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
        violations.iter().any(|violation| violation
            .to_string()
            .contains("per-command steering belongs on the command")),
        "{violations:?}"
    );
}
