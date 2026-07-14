//! RFC 0010 acceptance tests: declared preconditions (capabilities) wired
//! into registration validation, catalog projection, help, plan-time
//! diagnostics, runtime denial enrichment, and contract checks.

use mcp_twill::{
    ArgSpec, CapabilityDecl, CommandContext, CommandExample, CommandOutput, CommandRegistry,
    CommandSpec, FrameworkError, FrameworkEvent, HelpRequest, ResourceDecl, ResponseEnvelope,
    RunRequest, arg,
};
use serde_json::json;

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

const VALID_RECEIPT: &str = "receipt-current-build";

fn build_capability() -> CapabilityDecl {
    CapabilityDecl::new(
        "validated-build",
        "Proof that the current build passed validation",
    )
    .carried_by("validation_token")
}

fn provider_spec() -> CommandSpec {
    CommandSpec::new(
        ["build", "validate"],
        "Validate build",
        "Validates the current build and returns an opaque receipt.",
    )
    .provides("validated-build")
}

fn consumer_spec() -> CommandSpec {
    let mut example = CommandExample::new(
        "deploy publish --validation-token $args.validation_token",
        "Publish a build after validation",
    );
    example
        .args
        .insert("validation_token".to_string(), json!(VALID_RECEIPT));
    CommandSpec::new(
        ["deploy", "publish"],
        "Publish deployment",
        "Publishes a build after application-owned receipt validation.",
    )
    .with_arg(ArgSpec::string(
        "validation_token",
        "Opaque validation receipt",
    ))
    .with_example(example)
    .requires("validated-build")
}

fn registry() -> CommandRegistry {
    CommandRegistry::new("capability-test", "Capability integration test server")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({ "published": true })))
        })
}

fn builder_registry() -> CommandRegistry {
    CommandRegistry::build(
        "capability-test",
        "Capability integration test server",
        |server| {
            server.capability(build_capability());
            server.command("build validate", |command| {
                command
                    .summary("Validate build")
                    .description("Validates the current build and returns an opaque receipt.")
                    .provides("validated-build")
                    .handle(|_context| async {
                        Ok(CommandOutput::structured(
                            json!({ "receipt": VALID_RECEIPT }),
                        ))
                    });
            });
            server.command("deploy publish", |command| {
                command
                    .summary("Publish deployment")
                    .description("Publishes a build after application-owned receipt validation.")
                    .arg(arg::string("validation_token").summary("Opaque validation receipt"))
                    .requires("validated-build")
                    .example_with_args(
                        "deploy publish --validation-token $args.validation_token",
                        "Publish a build after validation",
                        json!({ "validation_token": VALID_RECEIPT }),
                    )
                    .handle(|_context| async {
                        Ok(CommandOutput::structured(json!({ "published": true })))
                    });
            });
        },
    )
    .expect("builder registry")
}

fn refresh_spec(name: &str) -> CommandSpec {
    let mut example = CommandExample::new(
        format!("build {name} --validation-token $args.validation_token"),
        "Refresh build validation",
    );
    example
        .args
        .insert("validation_token".to_string(), json!(VALID_RECEIPT));
    CommandSpec::new(
        ["build", name],
        "Refresh build validation",
        "Replaces a current validation receipt.",
    )
    .with_arg(ArgSpec::string(
        "validation_token",
        "Opaque validation receipt",
    ))
    .with_example(example)
    .requires("validated-build")
    .provides("validated-build")
}

fn role_registry(reverse_registration: bool) -> CommandRegistry {
    let base = CommandRegistry::new("capability-roles", "Capability role test server")
        .declare_capability(build_capability());
    let base = if reverse_registration {
        base.register(refresh_spec("z-refresh"), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(refresh_spec("a-refresh"), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
    } else {
        base.register(refresh_spec("a-refresh"), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(refresh_spec("z-refresh"), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
    };
    base.register(consumer_spec(), |_context| async {
        Err(FrameworkError::capability_denied(
            "validated-build",
            "the validation receipt is stale",
        ))
    })
}

fn public_denial_detail(detail: &str) -> String {
    let envelope = ResponseEnvelope::framework_error(
        FrameworkError::CapabilityDenied {
            capability: "validated-build".to_string(),
            detail: detail.to_string(),
            carrier: Some("validation_token".to_string()),
            providers: vec!["build validate".to_string()],
        },
        None,
        None,
    );
    envelope.error.expect("error body").details["detail"]
        .as_str()
        .expect("detail string")
        .to_string()
}

// Acceptance: a fully declared capability graph passes validation.
#[test]
fn declared_capability_graph_passes_validation() {
    registry().validate_capabilities().expect("valid graph");
}

// Acceptance: the low-level and mutable builder surfaces describe the same
// catalog, help, and identity bytes.
#[test]
fn builder_and_low_level_authoring_are_equivalent() {
    let low_level = registry();
    let builder = builder_registry();
    assert_eq!(
        serde_json::to_value(low_level.catalog()).unwrap(),
        serde_json::to_value(builder.catalog()).unwrap()
    );
    assert_eq!(low_level.catalog_identity(), builder.catalog_identity());
    for command in [None, Some("build validate"), Some("deploy publish")] {
        let request = HelpRequest {
            command: command.map(ToString::to_string),
            topic: None,
            detail: None,
        };
        assert_eq!(
            low_level.help(request.clone()).text,
            builder.help(request).text
        );
    }
}

#[test]
fn builder_and_low_level_validation_failures_are_equivalent() {
    let mut optional = ArgSpec::string("validation_token", "Opaque validation receipt");
    optional.required = false;
    let low_level = CommandRegistry::new("bad", "Bad")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(
            CommandSpec::new(
                ["deploy", "publish"],
                "Publish deployment",
                "Publishes a validated build.",
            )
            .with_arg(optional)
            .requires("validated-build"),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        )
        .validate_capabilities()
        .unwrap_err();

    let builder = match CommandRegistry::build("bad", "Bad", |server| {
        server.capability(build_capability());
        server.command("build validate", |command| {
            command
                .summary("Validate build")
                .description("Validates the current build.")
                .provides("validated-build")
                .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
        });
        server.command("deploy publish", |command| {
            command
                .summary("Publish deployment")
                .description("Publishes a validated build.")
                .arg(
                    arg::string("validation_token")
                        .summary("Opaque validation receipt")
                        .optional(),
                )
                .requires("validated-build")
                .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
        });
    }) {
        Ok(_) => panic!("expected builder validation failure"),
        Err(error) => error,
    };
    assert_eq!(low_level.to_string(), builder.to_string());
}

// Acceptance: old CommandSpec JSON and explicit empty capability lists
// normalize to the same value and catalog hash.
#[test]
fn legacy_and_explicit_empty_capability_lists_are_identical() {
    let mut legacy = serde_json::to_value(consumer_spec()).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.remove("requires");
    object.remove("provides");
    let mut explicit = legacy.clone();
    explicit
        .as_object_mut()
        .unwrap()
        .insert("requires".to_string(), json!([]));
    explicit
        .as_object_mut()
        .unwrap()
        .insert("provides".to_string(), json!([]));

    let legacy: CommandSpec = serde_json::from_value(legacy).unwrap();
    let explicit: CommandSpec = serde_json::from_value(explicit).unwrap();
    assert_eq!(legacy, explicit);
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::to_value(&explicit).unwrap()
    );
    let legacy_hash = CommandRegistry::new("compat", "Compatibility")
        .register(legacy, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .catalog_identity()
        .catalog_hash;
    let explicit_hash = CommandRegistry::new("compat", "Compatibility")
        .register(explicit, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .catalog_identity()
        .catalog_hash;
    assert_eq!(legacy_hash, explicit_hash);
}

// Acceptance: fluent carrier assignment is ordinary visible-field
// replacement and matches direct struct construction.
#[test]
fn later_carried_by_replaces_the_earlier_carrier() {
    let fluent = CapabilityDecl::new(
        "validated-build",
        "Proof that the current build passed validation",
    )
    .carried_by("obsolete_token")
    .carried_by("validation_token");
    let direct = CapabilityDecl {
        name: "validated-build".to_string(),
        summary: "Proof that the current build passed validation".to_string(),
        carrier: "validation_token".to_string(),
    };
    assert_eq!(fluent, direct);
    CommandRegistry::new("replacement", "Replacement")
        .declare_capability(fluent)
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .validate_capabilities()
        .expect("the replacement carrier validates");
}

#[test]
fn repeated_command_capability_edges_are_no_ops() {
    let once = consumer_spec();
    let repeated = consumer_spec()
        .requires("validated-build")
        .requires("validated-build");
    assert_eq!(once.requires, repeated.requires);

    let once = provider_spec();
    let repeated = provider_spec()
        .provides("validated-build")
        .provides("validated-build");
    assert_eq!(once.provides, repeated.provides);
}

// Acceptance: the catalog carries capability declarations and per-operation
// requires/provides.
#[test]
fn catalog_projects_capabilities_and_operation_edges() {
    let catalog = registry().catalog();

    assert_eq!(catalog.capabilities.len(), 1);
    let decl = &catalog.capabilities[0];
    assert_eq!(decl.name, "validated-build");
    assert_eq!(
        decl.summary,
        "Proof that the current build passed validation"
    );
    assert_eq!(decl.carrier, "validation_token");

    let provider = catalog
        .operations
        .iter()
        .find(|op| op.id == "build.validate")
        .expect("provider operation");
    assert_eq!(provider.provides, vec!["validated-build".to_string()]);
    assert!(provider.requires.is_empty());

    let consumer = catalog
        .operations
        .iter()
        .find(|op| op.id == "deploy.publish")
        .expect("consumer operation");
    assert_eq!(consumer.requires, vec!["validated-build".to_string()]);
    assert!(consumer.provides.is_empty());
}

// Acceptance: command help names the required capability and derives the
// establishing command from `provides` declarations.
#[test]
fn command_help_renders_requires_with_derived_establishers() {
    let help = registry().help(HelpRequest {
        command: Some("deploy publish".to_string()),
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("Requires:"), "help: {}", help.text);
    assert!(
        help.text
            .contains("`validated-build`: Proof that the current build passed validation (carried by `validation_token`; establish with `build validate`)"),
        "help: {}",
        help.text
    );

    let provider_help = registry().help(HelpRequest {
        command: Some("build validate".to_string()),
        topic: None,
        detail: None,
    });
    assert!(
        provider_help.text.contains("Provides:"),
        "help: {}",
        provider_help.text
    );
    assert!(
        provider_help.text.contains("`validated-build`")
            && provider_help.text.contains("bootstrap provider"),
        "help: {}",
        provider_help.text
    );
}

// Acceptance: server-level help lists declared capabilities.
#[test]
fn server_help_lists_declared_capabilities() {
    let help = registry().help(HelpRequest {
        command: None,
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("Capabilities:"), "help: {}", help.text);
    assert!(
        help.text.contains("`validated-build`"),
        "help: {}",
        help.text
    );
}

// Acceptance: requiring an undeclared capability fails validation.
#[test]
fn requiring_undeclared_capability_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `deploy publish` requires capability `validated-build`, which is not declared on the server"
        ),
        "error: {error}"
    );
}

// Acceptance: providing an undeclared capability fails validation.
#[test]
fn providing_undeclared_capability_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `build validate` provides capability `validated-build`, which is not declared on the server"
        ),
        "error: {error}"
    );
}

// Acceptance: a declared capability with no providing command fails
// validation.
#[test]
fn capability_without_provider_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(build_capability())
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "capability `validated-build` has no providing command; declare `provides` on the command that establishes it"
        ),
        "error: {error}"
    );
}

// Acceptance: a declared capability with no requiring command fails
// validation.
#[test]
fn capability_without_consumer_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "capability `validated-build` has no requiring command; remove the declaration or declare `requires` on the commands that need it"
        ),
        "error: {error}"
    );
}

// Acceptance: a requiring command without the carrier argument fails
// validation.
#[test]
fn requiring_command_without_carrier_argument_fails_validation() {
    let missing_carrier = CommandSpec::new(
        ["deploy", "publish"],
        "Publish deployment",
        "Publishes a validated build.",
    )
    .requires("validated-build");
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(missing_carrier, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `deploy publish` requires capability `validated-build` but has no `validation_token` argument to carry it"
        ),
        "error: {error}"
    );
}

// Acceptance: an optional carrier argument fails validation; proof of a
// precondition cannot be optional.
#[test]
fn optional_carrier_argument_fails_validation() {
    let optional_carrier = CommandSpec::new(
        ["deploy", "publish"],
        "Publish deployment",
        "Publishes a validated build.",
    )
    .with_arg({
        let mut arg = ArgSpec::string("validation_token", "Opaque validation receipt");
        arg.required = false;
        arg
    })
    .requires("validated-build");
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(optional_carrier, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "command `deploy publish` requires capability `validated-build` but its carrier argument `validation_token` is optional; a carrier must be required"
        ),
        "error: {error}"
    );
}

// Acceptance: declaring the same capability twice fails validation.
#[test]
fn duplicate_capability_declaration_fails_validation() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(build_capability())
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("capability `validated-build` is declared more than once"),
        "error: {error}"
    );
}

// Acceptance: the serving path rejects a registry whose capability graph is
// invalid, so a server that registers cannot serve undeclared preconditions.
#[test]
fn serving_path_rejects_invalid_capability_graph() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = match mcp_twill::CliMcpServer::new(registry) {
        Ok(_) => panic!("expected serving to reject an invalid capability graph"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("requires capability `validated-build`"),
        "error: {error}"
    );
}

// Acceptance: an unbound carrier fails at plan time with a diagnostic
// located at the carrier argument and steering that names every
// establishing command.
#[tokio::test]
async fn missing_carrier_fails_at_plan_time_with_capability_diagnostic() {
    let response = registry()
        .run(request("deploy publish", json!({})))
        .await
        .unwrap_err();

    let FrameworkError::CapabilityMissing {
        capability,
        carrier,
        providers,
    } = &response
    else {
        panic!("expected CapabilityMissing, got {response:?}");
    };
    assert_eq!(capability, "validated-build");
    assert_eq!(carrier, "validation_token");
    assert_eq!(providers, &vec!["build validate".to_string()]);

    let envelope = ResponseEnvelope::framework_error(response.clone(), None, None);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["error"]["code"], json!("capability_missing"));
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("argument `validation_token` carries the `validated-build` capability"),
        "message: {}",
        value["error"]["message"]
    );
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Establish it with `build validate`."),
        "message: {}",
        value["error"]["message"]
    );
    assert_eq!(
        value["diagnostics"][0]["location"],
        json!({ "type": "argument", "name": "validation_token" })
    );
    assert_eq!(
        value["steering"][0]["label"],
        json!("Establish `validated-build` with `build validate`")
    );
    assert_eq!(
        value["steering"][0]["request"],
        json!({ "tool": "help", "arguments": { "command": "build validate" } })
    );
}

// Acceptance: a handler-raised capability denial is enriched by the
// framework with the carrier and establishing commands from declarations.
#[tokio::test]
async fn handler_capability_denial_is_enriched_from_declarations() {
    let registry = CommandRegistry::new("capability-test", "Capability test server")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(consumer_spec(), |_context| async {
            Err(FrameworkError::capability_denied(
                "validated-build",
                "the validation receipt is stale\nrun validation again",
            ))
        });

    let error = registry
        .run(request(
            "deploy publish --validation-token $args.validation_token",
            json!({ "validation_token": "receipt-stale-build" }),
        ))
        .await
        .unwrap_err();

    let FrameworkError::CapabilityDenied {
        capability,
        detail,
        carrier,
        providers,
    } = &error
    else {
        panic!("expected CapabilityDenied, got {error:?}");
    };
    assert_eq!(capability, "validated-build");
    assert_eq!(
        detail,
        "the validation receipt is stale\nrun validation again"
    );
    assert_eq!(carrier.as_deref(), Some("validation_token"));
    assert_eq!(providers, &vec!["build validate".to_string()]);

    let envelope = ResponseEnvelope::framework_error(error.clone(), None, None);
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["error"]["code"], json!("capability_denied"));
    assert_eq!(
        value["error"]["details"]["detail"],
        json!("the validation receipt is stale\\nrun validation again")
    );
    assert_eq!(
        value["diagnostics"][0]["location"],
        json!({ "type": "argument", "name": "validation_token" })
    );
    assert_eq!(
        value["steering"][0]["label"],
        json!("Establish `validated-build` with `build validate`")
    );
}

// Acceptance: capability requirements participate in catalog identity, so
// adding or removing a requirement changes the hash.
#[test]
fn capability_requirements_change_catalog_hash() {
    fn catalog_hash(required: bool) -> String {
        let consumer = if required {
            consumer_spec()
        } else {
            let mut consumer = consumer_spec();
            consumer.requires.clear();
            consumer
        };
        CommandRegistry::new("capability-test", "Capability integration test server")
            .declare_capability(build_capability())
            .register(provider_spec(), |_context| async {
                Ok(CommandOutput::structured(
                    json!({ "receipt": VALID_RECEIPT }),
                ))
            })
            .register(consumer, |_context| async {
                Ok(CommandOutput::structured(json!({ "published": true })))
            })
            .catalog_identity()
            .catalog_hash
    }

    let before = catalog_hash(false);
    let with_requirement = catalog_hash(true);
    let after_removal = catalog_hash(false);
    assert_ne!(with_requirement, before);
    assert_eq!(after_removal, before);
}

// Acceptance: the contract check passes a registry whose declarations and
// projections agree.
#[test]
fn contract_capability_projection_passes_valid_registry() {
    let violations = mcp_twill::contract::check_capability_projection(&registry());
    assert!(violations.is_empty(), "violations: {violations:?}");
}

// Regression: a capability declared without a carrier argument fails
// validation instead of producing errors with an empty argument name.
#[test]
fn capability_without_carrier_fails_validation() {
    for declaration in [
        CapabilityDecl::new(
            "validated-build",
            "Proof that the current build passed validation",
        ),
        CapabilityDecl {
            name: "validated-build".to_string(),
            summary: "Proof that the current build passed validation".to_string(),
            carrier: String::new(),
        },
    ] {
        let registry = CommandRegistry::new("bad", "Bad server")
            .declare_capability(declaration)
            .register(provider_spec(), |_context| async {
                Ok(CommandOutput::structured(json!({})))
            })
            .register(consumer_spec(), |_context| async {
                Ok(CommandOutput::structured(json!({})))
            });
        let error = registry.validate_capabilities().unwrap_err();
        assert!(
            error.to_string().contains(
                "capability `validated-build` does not declare a carrier argument; use `carried_by` to name the argument that carries it"
            ),
            "error: {error}"
        );
    }
}

// Regression: a capability whose only provider also requires it fails
// validation; steering must name a command that can bootstrap the
// capability without already holding it.
#[test]
fn capability_with_only_self_dependent_providers_fails_validation() {
    let mut example = CommandExample::new(
        "build refresh --validation-token $args.validation_token",
        "Refresh build validation",
    );
    example
        .args
        .insert("validation_token".to_string(), json!(VALID_RECEIPT));
    let refresh = CommandSpec::new(
        ["build", "refresh"],
        "Refresh build validation",
        "Replaces a current validation receipt.",
    )
    .with_arg(ArgSpec::string(
        "validation_token",
        "Opaque validation receipt",
    ))
    .with_example(example)
    .requires("validated-build")
    .provides("validated-build");
    let registry = CommandRegistry::new("bad", "Bad server")
        .declare_capability(build_capability())
        .register(refresh, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let error = registry.validate_capabilities().unwrap_err();
    assert!(
        error.to_string().contains(
            "capability `validated-build` has only providers that also require it; declare `provides` on a command that can establish it without an existing `validated-build`"
        ),
        "error: {error}"
    );
}

// Regression: a self-dependent provider is fine as long as a bootstrap
// provider also exists.
#[test]
fn self_dependent_provider_passes_with_bootstrap_provider() {
    let mut example = CommandExample::new(
        "build refresh --validation-token $args.validation_token",
        "Refresh build validation",
    );
    example
        .args
        .insert("validation_token".to_string(), json!(VALID_RECEIPT));
    let refresh = CommandSpec::new(
        ["build", "refresh"],
        "Refresh build validation",
        "Replaces a current validation receipt.",
    )
    .with_arg(ArgSpec::string(
        "validation_token",
        "Opaque validation receipt",
    ))
    .with_example(example)
    .requires("validated-build")
    .provides("validated-build");
    let registry = CommandRegistry::new("ok", "Good server")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(refresh, |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    registry
        .validate_capabilities()
        .expect("bootstrap provider satisfies the graph");
}

// Acceptance: provider order is canonical, while recovery contains only
// commands callable without an existing proof.
#[tokio::test]
async fn bootstrap_and_refresh_provider_roles_are_canonical() {
    let registry = role_registry(true);
    assert_eq!(
        registry.capability_providers("validated-build"),
        vec![
            "build a-refresh".to_string(),
            "build validate".to_string(),
            "build z-refresh".to_string(),
        ]
    );
    assert_eq!(
        registry.catalog_identity(),
        role_registry(false).catalog_identity(),
        "registration order cannot change catalog identity"
    );

    let missing = registry
        .run(request("deploy publish", json!({})))
        .await
        .expect_err("missing proof");
    let FrameworkError::CapabilityMissing { providers, .. } = missing else {
        panic!("expected missing capability");
    };
    assert_eq!(providers, vec!["build validate".to_string()]);

    let denied = registry
        .run(request(
            "deploy publish --validation-token $args.validation_token",
            json!({ "validation_token": "receipt-stale-build" }),
        ))
        .await
        .expect_err("stale proof");
    let FrameworkError::CapabilityDenied { providers, .. } = denied else {
        panic!("expected denied capability");
    };
    assert_eq!(providers, vec!["build validate".to_string()]);

    let help = registry.help(HelpRequest {
        command: Some("deploy publish".to_string()),
        topic: None,
        detail: None,
    });
    assert!(
        help.text.contains("establish with `build validate`")
            && help
                .text
                .contains("refresh with `build a-refresh`, `build z-refresh`"),
        "help: {}",
        help.text
    );
    let refresh_help = registry.help(HelpRequest {
        command: Some("build a-refresh".to_string()),
        topic: None,
        detail: None,
    });
    assert!(
        refresh_help.text.contains("refresh provider"),
        "help: {}",
        refresh_help.text
    );
}

// Acceptance: handler-supplied compatibility steering is replaced by the
// selected command's authoritative declaration graph.
#[tokio::test]
async fn handler_provided_denial_guidance_is_replaced() {
    let registry = CommandRegistry::new("capability-test", "Capability test server")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(consumer_spec(), |_context| async {
            Err(FrameworkError::CapabilityDenied {
                capability: "validated-build".to_string(),
                detail: "the validation receipt is stale".to_string(),
                carrier: Some("custom_carrier".to_string()),
                providers: vec!["custom recover".to_string()],
            })
        });

    let error = registry
        .run(request(
            "deploy publish --validation-token $args.validation_token",
            json!({ "validation_token": "receipt-stale-build" }),
        ))
        .await
        .unwrap_err();

    let FrameworkError::CapabilityDenied {
        carrier, providers, ..
    } = &error
    else {
        panic!("expected CapabilityDenied, got {error:?}");
    };
    assert_eq!(carrier.as_deref(), Some("validation_token"));
    assert_eq!(providers, &vec!["build validate".to_string()]);
}

#[tokio::test]
async fn invalid_legacy_denials_collapse_to_one_handler_failure() {
    async fn assert_redacted(registry: CommandRegistry, command: &str) {
        let error = registry
            .run(request(
                command,
                json!({ "validation_token": VALID_RECEIPT }),
            ))
            .await
            .expect_err("invalid legacy denial");
        assert_eq!(
            error,
            FrameworkError::Handler(
                "legacy handler returned invalid capability denial".to_string()
            )
        );
        let value =
            serde_json::to_value(ResponseEnvelope::framework_error(error, None, None)).unwrap();
        assert_eq!(value["error"]["code"], json!("handler_failed"));
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("invented-proof"));
        assert!(!serialized.contains("secret denial detail"));
    }

    let undeclared = CommandRegistry::new("invalid", "Invalid denial")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        })
        .register(consumer_spec(), |_context| async {
            Err(FrameworkError::CapabilityDenied {
                capability: "invented-proof".to_string(),
                detail: "secret denial detail".to_string(),
                carrier: Some("spoofed".to_string()),
                providers: vec!["spoofed recover".to_string()],
            })
        });
    assert_redacted(
        undeclared,
        "deploy publish --validation-token $args.validation_token",
    )
    .await;

    let unrequired = registry().register(
        CommandSpec::new(
            ["deploy", "status"],
            "Deployment status",
            "Reads deployment status.",
        )
        .with_arg({
            let mut arg = ArgSpec::string("validation_token", "Unused compatibility input");
            arg.required = false;
            arg
        }),
        |_context| async {
            Err(FrameworkError::CapabilityDenied {
                capability: "validated-build".to_string(),
                detail: "secret denial detail".to_string(),
                carrier: None,
                providers: Vec::new(),
            })
        },
    );
    assert_redacted(
        unrequired,
        "deploy status --validation-token $args.validation_token",
    )
    .await;

    let resource_derived = CommandRegistry::build("resource", "Resource denial", |server| {
        server.resource(ResourceDecl::new("session", "A live session").uri("test://session/{id}"));
        server.command("session inspect", |command| {
            command
                .summary("Inspect session")
                .description("Tests rejection of legacy denial for resource authority.")
                .arg(arg::string("session_id").summary("Session reference"))
                .requires("session")
                .example_with_args(
                    "session inspect --session-id $args.session_id",
                    "Inspect a session",
                    json!({ "session_id": "sess-1" }),
                )
                .handle(|_context| async {
                    Err::<CommandOutput, _>(FrameworkError::CapabilityDenied {
                        capability: "session".to_string(),
                        detail: "secret denial detail".to_string(),
                        carrier: None,
                        providers: Vec::new(),
                    })
                });
        });
    })
    .expect("resource-derived registry");
    let error = resource_derived
        .run(request(
            "session inspect --session-id $args.session_id",
            json!({ "session_id": "sess-1" }),
        ))
        .await
        .expect_err("resource-derived denial is invalid");
    assert_eq!(
        error,
        FrameworkError::Handler("legacy handler returned invalid capability denial".to_string())
    );
}

#[test]
fn public_denial_detail_uses_exact_safe_encoding() {
    assert_eq!(
        public_denial_detail("safe detail: café ✅"),
        "safe detail: café ✅"
    );
    assert_eq!(
        public_denial_detail("\"\\\u{0008}\u{000C}\n\r\t"),
        "\\\"\\\\\\b\\f\\n\\r\\t"
    );

    for scalar in ('\u{0000}'..='\u{001F}')
        .chain('\u{007F}'..='\u{009F}')
        .filter(|scalar| {
            !matches!(
                scalar,
                '\u{0008}' | '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}'
            )
        })
    {
        assert_eq!(
            public_denial_detail(&scalar.to_string()),
            format!("\\u{:04X}", scalar as u32),
            "scalar U+{:04X}",
            scalar as u32
        );
    }
    for scalar in [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{2028}', '\u{2029}', '\u{202A}', '\u{202B}',
        '\u{202C}', '\u{202D}', '\u{202E}', '\u{2060}', '\u{2061}', '\u{2062}', '\u{2063}',
        '\u{2064}', '\u{2065}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{206A}',
        '\u{206B}', '\u{206C}', '\u{206D}', '\u{206E}', '\u{206F}', '\u{FEFF}',
    ] {
        assert_eq!(
            public_denial_detail(&scalar.to_string()),
            format!("\\u{:04X}", scalar as u32)
        );
    }
}

#[test]
fn public_denial_detail_bound_never_splits_an_escape() {
    assert_eq!(public_denial_detail(&"a".repeat(512)), "a".repeat(512));
    assert_eq!(
        public_denial_detail(&"a".repeat(513)),
        format!("{}…", "a".repeat(511))
    );
    assert_eq!(
        public_denial_detail(&format!("{}\u{0000}", "a".repeat(506))),
        format!("{}\\u0000", "a".repeat(506))
    );
    let truncated = public_denial_detail(&format!("{}\u{0000}bc", "a".repeat(505)));
    assert_eq!(truncated, format!("{}\\u0000…", "a".repeat(505)));
    assert_eq!(truncated.chars().count(), 512);
    assert!(!truncated.ends_with("\\u000…"));
}

#[test]
fn framework_event_omits_capability_denial_detail() {
    let secret = "secret validation_token=receipt-current-build";
    let envelope = ResponseEnvelope::framework_error(
        FrameworkError::CapabilityDenied {
            capability: "validated-build".to_string(),
            detail: secret.to_string(),
            carrier: Some("validation_token".to_string()),
            providers: vec!["build validate".to_string()],
        },
        None,
        None,
    );
    assert!(serde_json::to_string(&envelope).unwrap().contains(secret));
    let event = FrameworkEvent::from_envelope(&envelope, None);
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(!serialized.contains(secret));
    assert!(serialized.contains("validated-build"));
    assert_eq!(
        event.diagnostics[0].message,
        "capability `validated-build` denied"
    );
}

#[tokio::test]
async fn provides_is_only_an_edge_and_never_injects_proof() {
    let registry = CommandRegistry::new("proof", "Proof flow")
        .declare_capability(build_capability())
        .register(provider_spec(), |_context| async {
            Ok(CommandOutput::structured(
                json!({ "receipt": VALID_RECEIPT }),
            ))
        })
        .register(consumer_spec(), |context: CommandContext| async move {
            let token = context.plan.bound_args["validation_token"]
                .value
                .as_str()
                .expect("string token");
            if token != VALID_RECEIPT {
                return Err(FrameworkError::capability_denied(
                    "validated-build",
                    "the validation receipt does not match the current build",
                ));
            }
            Ok(CommandOutput::structured(json!({ "published": true })))
        });

    let validation = registry
        .run(request("build validate", json!({})))
        .await
        .expect("validation succeeds");
    assert_eq!(
        validation.output.unwrap().structured.unwrap()["receipt"],
        json!(VALID_RECEIPT)
    );
    assert!(matches!(
        registry
            .run(request("deploy publish", json!({})))
            .await
            .unwrap_err(),
        FrameworkError::CapabilityMissing { .. }
    ));
    registry
        .run(request(
            "deploy publish --validation-token $args.validation_token",
            json!({ "validation_token": VALID_RECEIPT }),
        ))
        .await
        .expect("caller explicitly supplies returned proof");
    let denied = registry
        .run(request(
            "deploy publish --validation-token $args.validation_token",
            json!({ "validation_token": "receipt-stale-build" }),
        ))
        .await
        .expect_err("application rejects stale proof");
    let FrameworkError::CapabilityDenied { detail, .. } = denied else {
        panic!("expected application-owned capability denial");
    };
    assert_eq!(
        detail,
        "the validation receipt does not match the current build"
    );
    assert!(!detail.contains("receipt-stale-build"));
}

// Regression: a malformed request shape (unknown argument) reports the
// shape error, not a capability diagnostic; the capability check only
// replaces the generic missing-argument case for valid shapes.
#[tokio::test]
async fn unknown_argument_reported_before_missing_carrier() {
    let error = registry()
        .run(request("deploy publish", json!({ "typo": "oops" })))
        .await
        .unwrap_err();

    assert!(
        matches!(&error, FrameworkError::UnknownArgument(name) if name == "typo"),
        "expected UnknownArgument, got {error:?}"
    );
}

// Acceptance: the contract check reports a registry whose capability graph
// is invalid rather than projecting it.
#[test]
fn contract_capability_projection_reports_invalid_graph() {
    let registry = CommandRegistry::new("bad", "Bad server")
        .register(consumer_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({})))
        });
    let violations = mcp_twill::contract::check_capability_projection(&registry);
    assert_eq!(violations.len(), 1);
    assert!(
        violations[0]
            .message
            .contains("requires capability `validated-build`"),
        "violation: {:?}",
        violations[0]
    );
}

mcp_twill::contract_tests!(registry);
