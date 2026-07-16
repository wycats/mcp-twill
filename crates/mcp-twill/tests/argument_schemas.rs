//! Acceptance tests for schema-constrained arguments (RFC 0017).

use std::borrow::Cow;
use std::collections::BTreeMap;

use mcp_twill::{
    ApplicationResult, ApplicationSuccess, ArgSchemaMatch, ArgSpec, ArgumentContractReason,
    ArgumentSchemaDecl, ArgumentSchemaKeyword, ArgumentSchemaUse, BoundArg, CommandOutput,
    CommandRegistry, CommandSpec, FrameworkError, FrameworkEvent, HelpRequest, InvocationPlan,
    JsonInteger, Res, ResolveResource, Resource, ResourceDecl, ResourceRefusal, ResponseEnvelope,
    RunMode, RunRequest, SchemaBranchProblem, SchemaBranchSelection, WorkspaceDecl, arg,
};
use rmcp::{ClientHandler, ServiceExt, model::CallToolRequestParams};
use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Number, Value, json};

#[path = "support/vbl.rs"]
mod vbl;

fn vbl_baseline() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vbl/v0.4.9/baseline-tools.json"
    )))
    .expect("parse VBL v0.4.9 baseline")
}

fn canonicalize_required_sets(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
                required.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
            }
            for nested in object.values_mut() {
                canonicalize_required_sets(nested);
            }
        }
        Value::Array(values) => {
            for nested in values {
                canonicalize_required_sets(nested);
            }
        }
        _ => {}
    }
}

fn request(command: &str, args: Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value::<BTreeMap<String, Value>>(args).unwrap(),
        stdin: None,
        output: None,
        mode: RunMode::Execute,
        approval: None,
        dry_run: false,
    }
}

#[derive(Default)]
struct TestClient;

impl ClientHandler for TestClient {}

fn bounded_registry() -> CommandRegistry {
    CommandRegistry::new("schema-test", "Argument schema test server").register(
        CommandSpec::new(["screen", "start"], "Start", "Start capture").with_arg(
            ArgSpec::integer("width", "Capture width").with_inline_schema(json!({
                "type": "integer",
                "minimum": 2,
                "maximum": 10,
                "multipleOf": 2,
            })),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({ "started": true }))) },
    )
}

fn registry_with_schema(schema: Value) -> CommandRegistry {
    CommandRegistry::new("schema-test", "Argument schema test server").register(
        CommandSpec::new(["value", "take"], "Take", "Take one value")
            .with_arg(ArgSpec::inline_schema("value", schema, "Value")),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    )
}

#[test]
fn bounded_integer_plans_and_redacts_mismatches() {
    let registry = bounded_registry();
    registry.validate_argument_schemas().unwrap();
    for value in [
        json!(2),
        json!(10),
        json!(4.0),
        serde_json::from_str("4e0").unwrap(),
    ] {
        let plan = registry
            .build_plan(&request(
                "screen start --width $args.width",
                json!({ "width": value }),
            ))
            .unwrap();
        assert_eq!(
            plan.bound_args["width"].schema_match,
            ArgSchemaMatch::default()
        );
    }

    for (value, keyword, expected) in [
        (json!(1), ArgumentSchemaKeyword::Minimum, "2"),
        (json!(11), ArgumentSchemaKeyword::Maximum, "10"),
        (json!(5), ArgumentSchemaKeyword::MultipleOf, "2"),
        (json!(4.5), ArgumentSchemaKeyword::Type, "\"integer\""),
    ] {
        let error = registry
            .build_plan(&request(
                "screen start --width $args.width",
                json!({ "width": value.clone() }),
            ))
            .unwrap_err();
        assert_eq!(
            error,
            FrameworkError::ArgumentSchemaMismatch {
                argument: "width".to_string(),
                path: String::new(),
                keyword,
                expected: expected.to_string(),
                branches: Vec::new(),
            }
        );
        assert!(!error.to_string().contains(&value.to_string()));
    }
    assert!(serde_json::from_str::<Value>("1e400").is_err());
}

#[tokio::test]
async fn native_tools_call_and_direct_planning_share_the_schema_authority() -> anyhow::Result<()> {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let server = mcp_twill::CliMcpServer::new(bounded_registry())?;
    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;

    for (width, status) in [(4, "ok"), (13, "invalidInput")] {
        let run = request(
            "screen start --width $args.width",
            json!({ "width": width }),
        );
        let arguments = serde_json::to_value(run)?
            .as_object()
            .cloned()
            .expect("run request is an object");
        let response = client
            .call_tool(CallToolRequestParams::new("run").with_arguments(arguments))
            .await?;
        assert_eq!(
            response
                .structured_content
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str),
            Some(status)
        );
    }

    client.cancel().await?;
    Ok(())
}

#[test]
fn presence_edges_compete_with_value_failures_and_bind_fingerprints() {
    let make = |paired: bool| {
        let width = ArgSpec::integer("width", "Width")
            .optional()
            .with_inline_schema(json!({ "type": "integer", "minimum": 1 }));
        let height = ArgSpec::integer("height", "Height")
            .optional()
            .with_inline_schema(json!({ "type": "integer", "minimum": 1 }));
        let (width, height) = if paired {
            (
                width.requires_argument("height"),
                height.requires_argument("width"),
            )
        } else {
            (width, height)
        };
        CommandRegistry::new("schema-test", "Argument schema test server").register(
            CommandSpec::new(["screen", "size"], "Size", "Set capture size")
                .with_arg(width)
                .with_arg(height),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        )
    };
    let paired = make(true);
    paired.validate_argument_schemas().unwrap();
    let built = CommandRegistry::build("schema-test", "Argument schema test server", |server| {
        server.command("screen size", |command| {
            command
                .summary("Size")
                .description("Set capture size")
                .arg(
                    arg::integer("width")
                        .with_inline_schema(json!({ "type": "integer", "minimum": 1 }))
                        .optional()
                        .requires_argument("height")
                        .summary("Width"),
                )
                .arg(
                    arg::integer("height")
                        .with_inline_schema(json!({ "type": "integer", "minimum": 1 }))
                        .optional()
                        .requires_argument("width")
                        .summary("Height"),
                )
                .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
        });
    })
    .unwrap();
    assert_eq!(paired.catalog(), built.catalog());
    assert_eq!(paired.catalog_identity(), built.catalog_identity());
    assert_eq!(
        paired.arg_schema(paired.command_specs().next().unwrap()),
        built.arg_schema(built.command_specs().next().unwrap())
    );
    assert_eq!(
        paired.help(HelpRequest {
            command: Some("screen size".to_string()),
            topic: None,
            detail: None,
        }),
        built.help(HelpRequest {
            command: Some("screen size".to_string()),
            topic: None,
            detail: None,
        })
    );
    paired
        .build_plan(&request("screen size", json!({})))
        .expect("neither optional member is valid");
    for (command, args) in [
        ("screen size --width $args.width", json!({ "width": 2 })),
        ("screen size --height $args.height", json!({ "height": 2 })),
    ] {
        let run = request(command, args);
        let low_error = paired.build_plan(&run).unwrap_err();
        assert_eq!(low_error, built.build_plan(&run).unwrap_err());
        assert!(matches!(
            low_error,
            FrameworkError::ArgumentSchemaMismatch {
                keyword: ArgumentSchemaKeyword::DependentRequired,
                ..
            }
        ));
    }
    let error = paired
        .build_plan(&request(
            "screen size --width $args.width",
            json!({ "width": 0 }),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::Minimum,
            ..
        }
    ));

    let args = json!({ "width": 2, "height": 4 });
    let paired_plan = paired
        .build_plan(&request(
            "screen size --width $args.width --height $args.height",
            args.clone(),
        ))
        .unwrap();
    let independent_plan = make(false)
        .build_plan(&request(
            "screen size --width $args.width --height $args.height",
            args,
        ))
        .unwrap();
    assert_ne!(
        paired_plan.invocation_fingerprint,
        independent_plan.invocation_fingerprint
    );

    let constrained = |minimum| {
        CommandRegistry::new("schema-test", "Argument schema test server").register(
            CommandSpec::new(["screen", "size"], "Size", "Set capture size").with_arg(
                ArgSpec::integer("width", "Width").with_inline_schema(json!({
                    "type": "integer",
                    "minimum": minimum,
                })),
            ),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        )
    };
    let command = "screen size --width $args.width";
    let args = json!({ "width": 3 });
    let first = constrained(1)
        .build_plan(&request(command, args.clone()))
        .unwrap();
    let second = constrained(2).build_plan(&request(command, args)).unwrap();
    assert_ne!(first.invocation_fingerprint, second.invocation_fingerprint);

    let trigger_order = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["presence", "order"], "Order", "Order")
            .with_arg(
                ArgSpec::string("alpha", "Alpha")
                    .optional()
                    .requires_argument("zulu_target"),
            )
            .with_arg(
                ArgSpec::string("beta", "Beta")
                    .optional()
                    .requires_argument("alpha_target"),
            )
            .with_arg(ArgSpec::string("alpha_target", "Alpha target").optional())
            .with_arg(ArgSpec::string("zulu_target", "Zulu target").optional()),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    trigger_order.validate_argument_schemas().unwrap();
    let error = trigger_order
        .build_plan(&request(
            "presence order --alpha $args.alpha --beta $args.beta",
            json!({ "alpha": "present", "beta": "present" }),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        FrameworkError::ArgumentSchemaMismatch {
            argument: "alpha".to_string(),
            path: String::new(),
            keyword: ArgumentSchemaKeyword::DependentRequired,
            expected: "zulu_target".to_string(),
            branches: Vec::new(),
        }
    );
}

#[test]
fn nested_union_records_canonical_branch_identity() {
    let schema = json!({
        "type": "array",
        "items": {
            "oneOf": [
                { "type": "string" },
                { "type": "integer" }
            ]
        }
    });
    let registry = CommandRegistry::new("schema-test", "Argument schema test server").register(
        CommandSpec::new(["value", "take"], "Take", "Take values")
            .with_arg(ArgSpec::inline_schema("values", schema, "Values")),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    registry.validate_argument_schemas().unwrap();
    let plan = registry
        .build_plan(&request(
            "value take --values $args.values",
            json!({ "values": ["one", 2] }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["values"].schema_match.selections,
        vec![
            SchemaBranchSelection {
                schema: "@inline:values".to_string(),
                instance_pointer: "/0".to_string(),
                one_of_pointer: "/items/oneOf".to_string(),
                branch_pointer: "/items/oneOf/0".to_string(),
            },
            SchemaBranchSelection {
                schema: "@inline:values".to_string(),
                instance_pointer: "/1".to_string(),
                one_of_pointer: "/items/oneOf".to_string(),
                branch_pointer: "/items/oneOf/1".to_string(),
            },
        ]
    );
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    #[schemars(length(min = 1))]
    query: String,
}

#[test]
fn constrained_handler_compares_and_extracts_the_typed_schema() {
    let registry = CommandRegistry::build("schema-test", "Argument schema test server", |server| {
        server.command("search run", |command| {
            command
                .summary("Search")
                .description("Run a search")
                .arg(
                    arg::string("query")
                        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
                        .summary("Search query"),
                )
                .handle_constrained(|_context, args: SearchArgs| async move {
                    Ok(CommandOutput::structured(json!({ "query": args.query })))
                });
        });
    })
    .unwrap();
    let plan = registry
        .build_plan(&request(
            "search run --query $args.query",
            json!({ "query": "twill" }),
        ))
        .unwrap();
    assert_eq!(plan.bound_args["query"].value, json!("twill"));
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriftedArgs {
    query: bool,
}

#[test]
fn constrained_handler_rejects_derived_schema_drift() {
    let error = CommandRegistry::build("schema-test", "Argument schema test server", |server| {
        server.command("search run", |command| {
            command
                .summary("Search")
                .description("Run a search")
                .arg(arg::string("query").summary("Search query"))
                .handle_constrained(|_context, args: DriftedArgs| async move {
                    Ok(CommandOutput::structured(json!({ "query": args.query })))
                });
        });
    })
    .err()
    .expect("drifted schema must fail registration");
    assert!(matches!(
        error,
        FrameworkError::ArgumentContractViolation {
            reason: ArgumentContractReason::DerivedSchemaDrift,
            ..
        }
    ));

    let legacy = CommandRegistry::build("schema-test", "Argument schema test server", |server| {
        server.command("search legacy", |command| {
            command
                .summary("Search")
                .description("Run a search")
                .arg(
                    arg::string("query")
                        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
                        .summary("Search query"),
                )
                .handle(|_context, args: SearchArgs| async move {
                    Ok(CommandOutput::structured(json!({ "query": args.query })))
                });
        });
    })
    .err()
    .expect("legacy typed constrained handler must fail");
    assert!(
        matches!(legacy, FrameworkError::Build(message) if message.contains("handle_constrained"))
    );
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OptionalTextArgs {
    value: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OptionalPairArgs {
    width: Option<JsonInteger>,
    height: Option<JsonInteger>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationshipOwnedArgs {
    value: Option<String>,
}

impl JsonSchema for RelationshipOwnedArgs {
    fn schema_name() -> Cow<'static, str> {
        "RelationshipOwnedArgs".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "value": { "type": ["string", "null"] }
            },
            "dependentRequired": {
                "value": ["other"]
            },
            "additionalProperties": false
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AmbiguousAnyOfArgs {
    value: Option<String>,
}

impl JsonSchema for AmbiguousAnyOfArgs {
    fn schema_name() -> Cow<'static, str> {
        "AmbiguousAnyOfArgs".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "value": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": "string", "minLength": 1 }
                    ]
                }
            },
            "additionalProperties": false
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PrimitiveIntegerArgs {
    count: u64,
}

#[test]
fn typed_optional_null_and_presence_authority_are_exact() {
    let optional_non_null = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("value optional", |command| {
            command
                .summary("Optional")
                .description("Optional")
                .arg(arg::string("value").optional().summary("Value"))
                .handle_constrained(|_context, args: OptionalTextArgs| async move {
                    Ok(CommandOutput::structured(
                        json!({ "present": args.value.is_some() }),
                    ))
                });
        });
    })
    .unwrap();
    optional_non_null.validate_argument_schemas().unwrap();
    optional_non_null
        .build_plan(&request("value optional", json!({})))
        .unwrap();
    assert!(matches!(
        optional_non_null
            .build_plan(&request(
                "value optional --value $args.value",
                json!({ "value": null }),
            ))
            .unwrap_err(),
        FrameworkError::InvalidArgumentType(_, _)
    ));

    for kind in [json!(["string", "null"]), json!(["null", "string"])] {
        let registry = CommandRegistry::build("schema-test", "schema test", |server| {
            server.command("value nullable", |command| {
                command
                    .summary("Nullable")
                    .description("Nullable")
                    .arg(
                        arg::inline_schema("value", json!({ "type": kind }))
                            .optional()
                            .summary("Value"),
                    )
                    .handle_constrained(|_context, args: OptionalTextArgs| async move {
                        Ok(CommandOutput::structured(
                            json!({ "present": args.value.is_some() }),
                        ))
                    });
            });
        })
        .unwrap();
        registry.validate_argument_schemas().unwrap();
        registry
            .build_plan(&request(
                "value nullable --value $args.value",
                json!({ "value": null }),
            ))
            .unwrap();
    }

    let paired = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("screen size", |command| {
            command
                .summary("Size")
                .description("Size")
                .arg(
                    arg::integer("width")
                        .optional()
                        .requires_argument("height")
                        .summary("Width"),
                )
                .arg(
                    arg::integer("height")
                        .optional()
                        .requires_argument("width")
                        .summary("Height"),
                )
                .handle_constrained(|_context, args: OptionalPairArgs| async move {
                    Ok(CommandOutput::structured(json!({
                        "width": args.width.map(JsonInteger::into_number),
                        "height": args.height.map(JsonInteger::into_number),
                    })))
                });
        });
    })
    .unwrap();
    paired.validate_argument_schemas().unwrap();

    let second_authority = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("value relationship", |command| {
            command
                .summary("Relationship")
                .description("Relationship")
                .arg(arg::string("value").optional().summary("Value"))
                .handle_constrained(|_context, args: RelationshipOwnedArgs| async move {
                    Ok(CommandOutput::structured(json!({
                        "present": args.value.is_some()
                    })))
                });
        });
    })
    .err()
    .expect("derived relationship authority must fail");
    assert!(matches!(
        second_authority,
        FrameworkError::ArgumentContractViolation {
            reason: ArgumentContractReason::DerivedSchemaDrift,
            ..
        }
    ));

    let ambiguous = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("value ambiguous", |command| {
            command
                .summary("Ambiguous")
                .description("Ambiguous")
                .arg(arg::string("value").optional().summary("Value"))
                .handle_constrained(|_context, args: AmbiguousAnyOfArgs| async move {
                    Ok(CommandOutput::structured(json!({
                        "present": args.value.is_some()
                    })))
                });
        });
    })
    .err()
    .expect("ambiguous derived anyOf must fail");
    assert!(matches!(
        ambiguous,
        FrameworkError::ArgumentContractViolation {
            reason: ArgumentContractReason::DerivedSchemaDrift,
            ..
        }
    ));

    let primitive_drift = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("value count", |command| {
            command
                .summary("Count")
                .description("Count")
                .arg(arg::integer("count").summary("Count"))
                .handle_constrained(|_context, args: PrimitiveIntegerArgs| async move {
                    Ok(CommandOutput::structured(json!({ "count": args.count })))
                });
        });
    })
    .err()
    .expect("primitive integer drift must fail");
    assert!(matches!(
        primitive_drift,
        FrameworkError::ArgumentContractViolation {
            reason: ArgumentContractReason::DerivedSchemaDrift,
            ..
        }
    ));
}

#[test]
fn vbl_v0_4_9_property_schemas_project_canonically_with_one_presence_delta() {
    let baseline = vbl_baseline();
    assert!(!vbl::PREAMBLE.is_empty());
    assert_eq!(vbl::ERROR_OWNERS.len(), 22);
    let _guidance_fixture = vbl::registry();
    let registry = vbl::argument_schema_registry(&baseline);
    registry.validate_argument_schemas().unwrap();

    let released = baseline
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| (tool["name"].as_str().unwrap(), tool["inputSchema"].clone()))
        .collect::<BTreeMap<_, _>>();
    let paths = vbl::OPERATION_MAPPING
        .iter()
        .map(|(name, path, _)| (*path, *name))
        .collect::<BTreeMap<_, _>>();
    let mut byte_identical = Vec::new();
    let mut canonical_required_only = Vec::new();
    let mut presence_delta = Vec::new();
    for operation in &registry.catalog().operations {
        let command = operation.path.join(" ");
        let released_name = paths[command.as_str()];
        let spec = registry
            .command_specs()
            .find(|spec| spec.path == operation.path)
            .expect("adopted command declaration");
        let projected = registry.arg_schema(spec);
        let expected = released[released_name].clone();
        if released_name == "screencast_start" {
            let mut expected_with_presence = expected;
            expected_with_presence.as_object_mut().unwrap().insert(
                "dependentRequired".to_string(),
                json!({
                    "max_height": ["max_width"],
                    "max_width": ["max_height"],
                }),
            );
            assert_eq!(projected, expected_with_presence, "{released_name}");
            presence_delta.push(released_name);
        } else if projected == expected {
            byte_identical.push(released_name);
        } else {
            let mut canonical_projected = projected;
            let mut canonical_expected = expected;
            canonicalize_required_sets(&mut canonical_projected);
            canonicalize_required_sets(&mut canonical_expected);
            assert_eq!(
                canonical_projected, canonical_expected,
                "unexpected non-canonical delta for {released_name}"
            );
            canonical_required_only.push(released_name);
        }
    }
    byte_identical.sort_unstable();
    canonical_required_only.sort_unstable();
    presence_delta.sort_unstable();
    assert_eq!(byte_identical.len(), 60);
    assert_eq!(canonical_required_only, ["fill_form", "wait_for"]);
    assert_eq!(presence_delta, ["screencast_start"]);
}

#[test]
fn vbl_v0_4_9_measured_argument_vocabulary_is_fully_represented() {
    fn count(value: &Value, keyword: &str) -> usize {
        match value {
            Value::Object(object) => {
                usize::from(object.contains_key(keyword))
                    + object
                        .values()
                        .map(|value| count(value, keyword))
                        .sum::<usize>()
            }
            Value::Array(values) => values.iter().map(|value| count(value, keyword)).sum(),
            _ => 0,
        }
    }
    let baseline = vbl_baseline();
    let tools = baseline.as_array().unwrap();
    let input_count = |keyword| {
        tools
            .iter()
            .map(|tool| count(&tool["inputSchema"], keyword))
            .sum::<usize>()
    };
    assert_eq!(input_count("oneOf"), 21);
    assert_eq!(input_count("minimum"), 5);
    assert_eq!(input_count("maximum"), 5);
    assert_eq!(input_count("multipleOf"), 2);
    assert_eq!(tools.len(), 63);
}

#[test]
fn unsupported_ambiguous_empty_and_malformed_schemas_fail_registration() {
    let cases = [
        ("boolean root", json!(false)),
        (
            "unsupported anyOf",
            json!({ "anyOf": [{ "type": "string" }] }),
        ),
        (
            "unsupported pattern",
            json!({ "type": "string", "pattern": "x" }),
        ),
        ("open object", json!({ "type": "object", "properties": {} })),
        ("empty enum", json!({ "type": "string", "enum": [] })),
        (
            "duplicate enum",
            json!({ "type": "string", "enum": ["x", "x"] }),
        ),
        (
            "numerically duplicate enum",
            json!({ "type": "number", "enum": [1, 1.0] }),
        ),
        (
            "reversed bounds",
            json!({ "type": "integer", "minimum": 2, "maximum": 1 }),
        ),
        (
            "empty divisible range",
            json!({ "type": "integer", "minimum": 1, "maximum": 2, "multipleOf": 3 }),
        ),
        (
            "zero divisor",
            json!({ "type": "integer", "multipleOf": 0 }),
        ),
        (
            "negative divisor",
            json!({ "type": "integer", "multipleOf": -2 }),
        ),
        (
            "fractional divisor",
            json!({ "type": "integer", "multipleOf": 1.5 }),
        ),
        (
            "fractional bound",
            json!({ "type": "integer", "minimum": 1.5 }),
        ),
        (
            "unsupported numeric keyword",
            json!({ "type": "integer", "exclusiveMinimum": 1 }),
        ),
        (
            "numeric literal outside exact I-JSON",
            json!({ "type": "integer", "const": 9_007_199_254_740_992_u64 }),
        ),
        (
            "numeric assertion outside exact I-JSON",
            json!({ "type": "integer", "minimum": 9_007_199_254_740_992_u64 }),
        ),
        (
            "ambiguous numeric branches",
            json!({ "oneOf": [{ "type": "number" }, { "type": "integer" }] }),
        ),
        (
            "numerically equal constant branches",
            json!({ "oneOf": [{ "const": 1 }, { "const": 1.0 }] }),
        ),
        (
            "dead definition",
            json!({ "type": "string", "$defs": { "dead": { "type": "string" } } }),
        ),
        (
            "dangling definition",
            json!({ "$ref": "#/$defs/missing", "$defs": {} }),
        ),
        (
            "reference sibling type contradiction",
            json!({
                "$ref": "#/$defs/text",
                "const": 1,
                "$defs": { "text": { "type": "string" } }
            }),
        ),
        (
            "reference sibling string contradiction",
            json!({
                "$ref": "#/$defs/text",
                "const": "x",
                "$defs": { "text": { "type": "string", "minLength": 2 } }
            }),
        ),
        (
            "reference sibling divisible range contradiction",
            json!({
                "$ref": "#/$defs/range",
                "multipleOf": 3,
                "$defs": {
                    "range": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 2
                    }
                }
            }),
        ),
        (
            "reference target finite domain contradicted by sibling",
            json!({
                "$ref": "#/$defs/finite",
                "minimum": 3,
                "$defs": {
                    "finite": {
                        "type": "integer",
                        "enum": [1, 2]
                    }
                }
            }),
        ),
        (
            "reference target finite union contradicted by sibling",
            json!({
                "$ref": "#/$defs/finite",
                "minimum": 3,
                "$defs": {
                    "finite": {
                        "oneOf": [
                            { "const": 1 },
                            { "const": 2 }
                        ]
                    }
                }
            }),
        ),
        (
            "cyclic definition",
            json!({
                "$ref": "#/$defs/loop",
                "$defs": { "loop": { "$ref": "#/$defs/loop" } }
            }),
        ),
        (
            "remote reference",
            json!({ "$ref": "https://example.com/schema" }),
        ),
        (
            "alternate dialect",
            json!({ "$schema": "https://json-schema.org/draft/2019-09/schema", "type": "string" }),
        ),
        (
            "nested dialect marker",
            json!({
                "type": "array",
                "items": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "string"
                }
            }),
        ),
        (
            "raw relationship authority",
            json!({ "type": "object", "additionalProperties": false, "dependentRequired": {} }),
        ),
        (
            "closed required unknown property",
            json!({
                "type": "object",
                "properties": {},
                "required": ["missing"],
                "additionalProperties": false
            }),
        ),
        (
            "duplicate required member",
            json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value", "value"],
                "additionalProperties": false
            }),
        ),
        (
            "statically empty branch",
            json!({
                "oneOf": [
                    { "type": "string", "const": 1 },
                    { "type": "integer" }
                ]
            }),
        ),
        (
            "finite const excluded by minimum",
            json!({ "type": "integer", "const": 1, "minimum": 2 }),
        ),
        (
            "finite enum excluded by multipleOf",
            json!({ "type": "integer", "enum": [1, 3], "multipleOf": 2 }),
        ),
        (
            "finite enum split across type and minimum exclusions",
            json!({ "type": "integer", "enum": [1, 2.5], "minimum": 2 }),
        ),
        (
            "finite const array excluded by minItems",
            json!({ "type": "array", "const": [], "minItems": 1 }),
        ),
        (
            "finite enum arrays excluded by minItems",
            json!({ "type": "array", "enum": [[], [1]], "minItems": 2 }),
        ),
    ];
    for (name, schema) in cases {
        assert!(
            registry_with_schema(schema)
                .validate_argument_schemas()
                .is_err(),
            "{name} unexpectedly registered"
        );
    }

    let wrong_coarse = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take").with_arg(
            ArgSpec::string("value", "Value").with_inline_schema(json!({
                "type": "integer"
            })),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert!(wrong_coarse.validate_argument_schemas().is_err());

    let repeated_array = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take").with_arg(
            ArgSpec::inline_schema("value", json!({ "type": "array" }), "Value").repeated(),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert!(repeated_array.validate_argument_schemas().is_err());
    let repeated_referenced_array = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take").with_arg(
            ArgSpec::inline_schema(
                "value",
                json!({
                    "$ref": "#/$defs/list",
                    "$defs": {
                        "list": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                "Value",
            )
            .repeated(),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert!(
        repeated_referenced_array
            .validate_argument_schemas()
            .is_err()
    );

    let named_override = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take").with_arg(
            ArgSpec::named("value", "named-value", "Value")
                .with_inline_schema(json!({ "type": "object", "additionalProperties": true })),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert!(named_override.validate_argument_schemas().is_err());

    let conflicting_description = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take").with_arg(ArgSpec::inline_schema(
            "value",
            json!({ "type": "string", "description": "Different" }),
            "Value",
        )),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert!(conflicting_description.validate_argument_schemas().is_err());

    let invalid_name = CommandRegistry::new("schema-test", "schema test")
        .declare_argument_schema(ArgumentSchemaDecl::new(
            "",
            "Missing name",
            json!({ "type": "string" }),
        ))
        .register(
            CommandSpec::new(["value", "take"], "Take", "Take")
                .with_arg(ArgSpec::named_schema("value", "", "Value")),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    assert!(invalid_name.validate_argument_schemas().is_err());

    let serving_invalid = registry_with_schema(json!({
        "type": "string",
        "pattern": "unsupported"
    }));
    assert!(mcp_twill::CliMcpServer::new(serving_invalid).is_err());
}

#[test]
fn supported_assertions_enforce_boundaries_and_global_failure_order() {
    let registry = registry_with_schema(json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "minLength": 2 },
            "tags": {
                "type": "array",
                "minItems": 1,
                "items": { "type": "string" }
            },
            "labels": {
                "type": "object",
                "properties": {},
                "additionalProperties": { "type": "string" }
            }
        },
        "required": ["name", "tags", "labels"],
        "additionalProperties": false
    }));
    registry.validate_argument_schemas().unwrap();
    registry
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": { "name": "ok", "tags": ["a"], "labels": { "x": "y" } } }),
        ))
        .unwrap();

    let cases = [
        (
            json!({ "name": "x", "tags": ["a"], "labels": {} }),
            ArgumentSchemaKeyword::MinLength,
            "/name",
        ),
        (
            json!({ "name": "ok", "tags": [], "labels": {} }),
            ArgumentSchemaKeyword::MinItems,
            "/tags",
        ),
        (
            json!({ "name": "ok", "tags": [1], "labels": {} }),
            ArgumentSchemaKeyword::Type,
            "/tags/0",
        ),
        (
            json!({ "name": "ok", "tags": ["a"], "labels": { "x": 1 } }),
            ArgumentSchemaKeyword::Type,
            "/labels/x",
        ),
        (
            json!({ "name": "ok", "tags": ["a"], "labels": {}, "extra": true }),
            ArgumentSchemaKeyword::AdditionalProperties,
            "",
        ),
    ];
    for (value, keyword, path) in cases {
        let error = registry
            .build_plan(&request(
                "value take --value $args.value",
                json!({ "value": value }),
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            FrameworkError::ArgumentSchemaMismatch {
                keyword: actual,
                path: ref actual_path,
                ..
            } if actual == keyword && actual_path == path
        ));
    }
}

#[test]
fn raw_json_unicode_domain_is_set_before_schema_planning() {
    let paired: Value = serde_json::from_str(r#"{"value":"\uD83D\uDE00"}"#).unwrap();
    assert_eq!(paired["value"].as_str().unwrap().chars().count(), 1);
    let registry = registry_with_schema(json!({ "type": "string", "minLength": 1 }));
    registry
        .build_plan(&request("value take --value $args.value", paired))
        .unwrap();
    assert!(serde_json::from_str::<Value>(r#"{"value":"\uD83D"}"#).is_err());
    assert!(serde_json::from_str::<Value>(r#"{"value":"\uDE00"}"#).is_err());
}

#[test]
fn numeric_const_and_enum_equality_follow_json_schema_value_semantics() {
    for schema in [
        json!({ "type": "number", "const": 1 }),
        json!({ "type": "number", "enum": [1] }),
    ] {
        let registry = registry_with_schema(schema);
        registry.validate_argument_schemas().unwrap();
        registry
            .build_plan(&request(
                "value take --value $args.value",
                json!({ "value": 1.0 }),
            ))
            .unwrap();
    }

    let integer = registry_with_schema(json!({ "const": 1 }));
    let zero_fraction = registry_with_schema(json!({ "const": 1.0 }));
    integer.validate_argument_schemas().unwrap();
    zero_fraction.validate_argument_schemas().unwrap();
    assert_eq!(integer.catalog(), zero_fraction.catalog());

    let bounded = registry_with_schema(json!({
        "type": "integer",
        "minimum": 2.0
    }));
    bounded.validate_argument_schemas().unwrap();
    assert_eq!(
        bounded.arg_schema(bounded.command_specs().next().unwrap())["properties"]["value"]["minimum"],
        json!(2)
    );
    assert!(matches!(
        bounded
            .build_plan(&request(
                "value take --value $args.value",
                json!({ "value": 1 }),
            ))
            .unwrap_err(),
        FrameworkError::ArgumentSchemaMismatch { expected, .. } if expected == "2"
    ));
}

#[test]
fn local_reference_chains_preserve_coarse_types_and_apply_siblings() {
    let registry = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take one value").with_arg(
            ArgSpec::string("value", "Value").with_inline_schema(json!({
                "$ref": "#/$defs/non-empty",
                "$defs": {
                    "non-empty": { "$ref": "#/$defs/string" },
                    "string": { "type": "string", "minLength": 1 }
                }
            })),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    registry.validate_argument_schemas().unwrap();
    assert!(mcp_twill::contract::check_type_projection(&registry).is_empty());
    assert!(mcp_twill::contract::check_argument_schema_projection(&registry).is_empty());
    let error = registry
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": "" }),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::MinLength,
            ..
        }
    ));

    let contradictory = registry_with_schema(json!({
        "$ref": "#/$defs/range",
        "maximum": 4,
        "$defs": {
            "range": { "type": "integer", "minimum": 5 }
        }
    }));
    assert!(contradictory.validate_argument_schemas().is_err());

    let referenced_union = registry_with_schema(json!({
        "oneOf": [
            { "$ref": "#/$defs/text" },
            { "$ref": "#/$defs/count" }
        ],
        "$defs": {
            "text": { "type": "string" },
            "count": { "type": "integer" }
        }
    }));
    referenced_union.validate_argument_schemas().unwrap();
    for value in [json!("text"), json!(1)] {
        referenced_union
            .build_plan(&request(
                "value take --value $args.value",
                json!({ "value": value }),
            ))
            .unwrap();
    }
}

#[test]
fn finite_values_imply_coarse_types_and_exact_size_limits_canonicalize() {
    for schema in [
        json!({ "const": "owned" }),
        json!({ "enum": ["owned", "global_readonly"] }),
    ] {
        let registry = CommandRegistry::new("schema-test", "schema test").register(
            CommandSpec::new(["scope", "set"], "Set", "Set scope")
                .with_arg(ArgSpec::string("scope", "Scope").with_inline_schema(schema)),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
        registry.validate_argument_schemas().unwrap();
    }

    let string_schema = serde_json::from_str(r#"{"type":"string","minLength":1.0}"#).unwrap();
    let string_registry = registry_with_schema(string_schema);
    string_registry.validate_argument_schemas().unwrap();
    let string_spec = string_registry.command_specs().next().unwrap();
    assert_eq!(
        string_registry.arg_schema(string_spec)["properties"]["value"]["minLength"],
        1
    );

    let array_schema = serde_json::from_str(r#"{"type":"array","minItems":1e0}"#).unwrap();
    let array_registry = registry_with_schema(array_schema);
    array_registry.validate_argument_schemas().unwrap();
    let array_spec = array_registry.command_specs().next().unwrap();
    assert_eq!(
        array_registry.arg_schema(array_spec)["properties"]["value"]["minItems"],
        1
    );
}

#[test]
fn resource_reference_refinements_reject_empty_finite_composite_domains() {
    let invalid = CommandRegistry::new("schema-test", "schema test").declare_resource(
        ResourceDecl::new("search-index", "Search index")
            .uri("search://index/{id}")
            .reference_schema(json!({
                "$ref": "#/$defs/reference",
                "const": "",
                "$defs": {
                    "reference": { "type": "string" }
                }
            })),
    );
    assert!(invalid.validate_argument_schemas().is_err());

    let broad = CommandRegistry::new("schema-test", "schema test").declare_resource(
        ResourceDecl::new("search-index", "Search index")
            .uri("search://index/{id}")
            .reference_schema(json!({ "type": "string", "minLength": 1 })),
    );
    broad.validate_argument_schemas().unwrap();
}

#[test]
fn enum_authoring_and_presence_edge_validation_are_canonical() {
    let owned = vec!["owned".to_string(), "global_readonly".to_string()];
    let array = ["owned", "global_readonly"];
    assert_eq!(
        ArgSpec::enumerated("scope", owned, "Scope"),
        ArgSpec::enumerated("scope", array, "Scope")
    );
    assert_eq!(
        ArgSpec::enumerated("scope", array, "Scope"),
        ArgSpec::enumerated("scope", array.as_slice(), "Scope")
    );

    let make = |target: &str, trigger_required: bool, target_required: bool| {
        CommandRegistry::new("schema-test", "schema test").register(
            CommandSpec::new(["screen", "size"], "Size", "Size")
                .with_arg(ArgSpec {
                    required: trigger_required,
                    ..ArgSpec::integer("width", "Width").requires_argument(target)
                })
                .with_arg(ArgSpec {
                    required: target_required,
                    ..ArgSpec::integer("height", "Height").optional()
                }),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        )
    };
    for registry in [
        make("", false, false),
        make("width", false, false),
        make("missing", false, false),
        make("height", true, false),
        make("height", false, true),
    ] {
        assert!(registry.validate_argument_schemas().is_err());
    }

    let canonical = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["screen", "size"], "Size", "Size")
            .with_arg(
                ArgSpec::integer("width", "Width")
                    .optional()
                    .requires_argument("height")
                    .requires_argument("height"),
            )
            .with_arg(ArgSpec::integer("height", "Height").optional()),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    canonical.validate_argument_schemas().unwrap();
    assert_eq!(
        canonical.catalog().operations[0].args[0].requires_arguments,
        vec!["height".to_string()]
    );
}

#[test]
fn refinements_replace_one_schema_slot_and_optionality_is_idempotent() {
    let final_schema = json!({ "type": "string", "minLength": 2 });
    let low_arg = ArgSpec::string("value", "Value")
        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
        .with_inline_schema(final_schema.clone())
        .optional()
        .optional();
    let low = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take").with_arg(low_arg),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let built = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("value take", |command| {
            command
                .summary("Take")
                .description("Take")
                .arg(
                    arg::enumerated("value", ["old"])
                        .with_inline_schema(final_schema)
                        .optional()
                        .optional()
                        .summary("Value"),
                )
                .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
        });
    })
    .unwrap();
    assert_eq!(low.catalog(), built.catalog());
    assert_eq!(low.catalog_identity(), built.catalog_identity());
    assert_eq!(
        low.help(HelpRequest {
            command: Some("value take".to_string()),
            topic: None,
            detail: None,
        }),
        built.help(HelpRequest {
            command: Some("value take".to_string()),
            topic: None,
            detail: None,
        })
    );
}

#[test]
fn named_declarations_reject_duplicates_dangling_references_and_dead_entries() {
    let declaration = || {
        ArgumentSchemaDecl::new(
            "non-empty",
            "Non-empty text",
            json!({ "type": "string", "minLength": 1 }),
        )
    };
    let duplicate =
        CommandRegistry::new("schema-test", "schema test")
            .declare_argument_schema(declaration())
            .declare_argument_schema(declaration())
            .register(
                CommandSpec::new(["value", "take"], "Take", "Take")
                    .with_arg(ArgSpec::named_schema("value", "non-empty", "Value")),
                |_context| async { Ok(CommandOutput::structured(json!({}))) },
            );
    let low_error = duplicate.validate_argument_schemas().unwrap_err();
    let builder_error = CommandRegistry::build("schema-test", "schema test", |server| {
        server.argument_schema(declaration());
        server.argument_schema(declaration());
        server.command("value take", |command| {
            command
                .summary("Take")
                .description("Take")
                .arg(arg::named_schema("value", "non-empty").summary("Value"))
                .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
        });
    })
    .err()
    .expect("duplicate builder declaration must fail");
    assert_eq!(low_error, builder_error);

    let dangling = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take")
            .with_arg(ArgSpec::named_schema("value", "missing", "Value")),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert!(dangling.validate_argument_schemas().is_err());

    let dead = CommandRegistry::new("schema-test", "schema test")
        .declare_argument_schema(declaration())
        .register(
            CommandSpec::new(["value", "take"], "Take", "Take")
                .with_arg(ArgSpec::string("value", "Value")),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    assert!(dead.validate_argument_schemas().is_err());
}

#[test]
fn json_integer_is_transparent_exact_and_private_by_construction() {
    for value in [
        json!(-2),
        json!(0),
        json!(u64::MAX),
        json!(1.0),
        json!(-0.0),
    ] {
        let integer: JsonInteger = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(integer).unwrap(), value);
    }
    for value in [json!(0.5), json!(-1.25)] {
        let error = serde_json::from_value::<JsonInteger>(value.clone()).unwrap_err();
        assert!(!error.to_string().contains(&value.to_string()));
    }
    let direct = JsonInteger::try_from_number(Number::from_f64(2.5).unwrap()).unwrap_err();
    assert_eq!(direct.to_string(), "JSON number is not an integer");
    assert!(!format!("{direct:?}").contains("2.5"));

    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let schema = generator.subschema_for::<JsonInteger>();
    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({ "type": "integer" })
    );

    let signed: JsonInteger = serde_json::from_value(json!(i64::MAX)).unwrap();
    assert_eq!(signed.as_i64(), Some(i64::MAX));
    let unsigned: JsonInteger = serde_json::from_value(json!(u64::MAX)).unwrap();
    assert_eq!(unsigned.as_u64(), Some(u64::MAX));
    assert_eq!(unsigned.as_i64(), None);

    let negative_zero = JsonInteger::try_from(Number::from_f64(-0.0).unwrap()).unwrap();
    assert_eq!(negative_zero.as_i64(), Some(0));
    assert_eq!(negative_zero.as_u64(), Some(0));
    let above_i64 = JsonInteger::try_from(
        Number::from_f64(9_223_372_036_854_775_808.0).expect("finite JSON number"),
    )
    .unwrap();
    assert_eq!(above_i64.as_i64(), None);
    let above_u64 = JsonInteger::try_from(
        Number::from_f64(18_446_744_073_709_551_616.0).expect("finite JSON number"),
    )
    .unwrap();
    assert_eq!(above_u64.as_u64(), None);

    let root = serde_json::to_value(schemars::schema_for!(JsonInteger)).unwrap();
    assert_eq!(root["type"], json!("integer"));
    assert!(root.get("$defs").is_none());
    assert!(root.get("format").is_none());
}

#[test]
fn declaration_wire_forms_omission_projection_help_and_hashes_are_canonical() {
    let legacy: ArgSpec = serde_json::from_value(json!({
        "name": "value",
        "valueType": "string",
        "required": true,
        "summary": "Value"
    }))
    .unwrap();
    let explicit = ArgSpec {
        schema: None,
        requires_arguments: Vec::new(),
        ..legacy.clone()
    };
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::to_value(&explicit).unwrap()
    );
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("schema")
            .is_none()
    );
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("requiresArguments")
            .is_none()
    );

    let named = ArgumentSchemaUse::named("non-empty");
    assert_eq!(
        serde_json::to_value(&named).unwrap(),
        json!({ "kind": "named", "name": "non-empty" })
    );
    assert!(serde_json::from_value::<ArgumentSchemaUse>(json!("non-empty")).is_err());
    assert!(
        serde_json::from_value::<ArgumentSchemaUse>(json!({
            "Named": { "name": "non-empty" }
        }))
        .is_err()
    );
    let named_with_addition: ArgumentSchemaUse = serde_json::from_value(json!({
        "kind": "named",
        "name": "non-empty",
        "futureField": true
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(named_with_addition).unwrap(),
        serde_json::to_value(&named).unwrap()
    );
    let inline: ArgumentSchemaUse = serde_json::from_value(json!({
        "kind": "inline",
        "schema": { "type": "string" },
        "futureField": true
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(inline).unwrap(),
        json!({ "kind": "inline", "schema": { "type": "string" } })
    );

    let make = |builder: bool| {
        if builder {
            CommandRegistry::build("schema-test", "schema test", |server| {
                server.argument_schema(ArgumentSchemaDecl::new(
                    "non-empty",
                    "A non-empty string",
                    json!({ "type": "string", "minLength": 1 }),
                ));
                server.command("value take", |command| {
                    command
                        .summary("Take")
                        .description("Take one value")
                        .arg(arg::named_schema("value", "non-empty").summary("Value"))
                        .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
                });
            })
            .unwrap()
        } else {
            CommandRegistry::new("schema-test", "schema test")
                .declare_argument_schema(ArgumentSchemaDecl::new(
                    "non-empty",
                    "A non-empty string",
                    json!({ "type": "string", "minLength": 1 }),
                ))
                .register(
                    CommandSpec::new(["value", "take"], "Take", "Take one value")
                        .with_arg(ArgSpec::named_schema("value", "non-empty", "Value")),
                    |_context| async { Ok(CommandOutput::structured(json!({}))) },
                )
        }
    };
    let low = make(false);
    let built = make(true);
    assert_eq!(low.catalog(), built.catalog());
    assert_eq!(low.catalog_identity(), built.catalog_identity());
    assert_eq!(
        low.help(HelpRequest {
            command: Some("value take".to_string()),
            topic: None,
            detail: None,
        }),
        built.help(HelpRequest {
            command: Some("value take".to_string()),
            topic: None,
            detail: None,
        })
    );
    let help = low.help(HelpRequest {
        command: Some("value take".to_string()),
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("non-empty"));
    assert_eq!(low.catalog().argument_schemas.len(), 1);

    let integer_registry = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "count"], "Count", "Count values")
            .with_arg(ArgSpec::integer("limit", "Maximum values")),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let integer_spec = integer_registry.command_specs().next().unwrap();
    assert_eq!(
        integer_registry.arg_schema(integer_spec)["properties"]["limit"],
        json!({ "type": "integer", "description": "Maximum values" })
    );

    let compatibility = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "compatible"], "Compatible", "Compatible").with_arg(
            ArgSpec::string("value", "Compatibility summary")
                .with_inline_schema(json!({ "type": "string", "minLength": 1 })),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let compatibility_spec = compatibility.command_specs().next().unwrap();
    assert_eq!(
        compatibility.arg_schema(compatibility_spec)["properties"]["value"],
        json!({ "type": "string", "minLength": 1 })
    );
    assert!(
        compatibility
            .help(HelpRequest {
                command: Some("value compatible".to_string()),
                topic: None,
                detail: None,
            })
            .text
            .contains("Compatibility summary")
    );

    let no_schema = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take one value")
            .with_arg(ArgSpec::string("value", "Value")),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    assert_ne!(
        no_schema.catalog_identity().catalog_hash,
        low.catalog_identity().catalog_hash
    );
    assert_eq!(
        no_schema.catalog_identity().catalog_hash,
        CommandRegistry::new("schema-test", "schema test")
            .register(
                CommandSpec::new(["value", "take"], "Take", "Take one value")
                    .with_arg(ArgSpec::string("value", "Value")),
                |_context| async { Ok(CommandOutput::structured(json!({}))) },
            )
            .catalog_identity()
            .catalog_hash
    );

    let default_bound = BoundArg {
        name: "value".to_string(),
        value_type: mcp_twill::ArgType::String,
        value: json!("value"),
        workspace: None,
        variants: None,
        schema_match: ArgSchemaMatch::default(),
    };
    assert!(
        serde_json::to_value(default_bound)
            .unwrap()
            .get("schemaMatch")
            .is_none()
    );
}

#[test]
fn value_and_schemars_constructors_share_the_draft_2020_12_compiler() {
    let value = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "string",
        "minLength": 1
    });
    let schema = schemars::Schema::try_from(value.clone()).unwrap();
    let make = |schema: Value| {
        CommandRegistry::new("schema-test", "schema test").register(
            CommandSpec::new(["value", "take"], "Take", "Take")
                .with_arg(ArgSpec::inline_schema("value", schema, "Value")),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        )
    };
    let from_value = make(value);
    let from_schema = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "take"], "Take", "Take")
            .with_arg(ArgSpec::inline_schema("value", schema, "Value")),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    from_value.validate_argument_schemas().unwrap();
    from_schema.validate_argument_schemas().unwrap();
    assert_eq!(from_value.catalog(), from_schema.catalog());
    let spec = from_value.command_specs().next().unwrap();
    assert!(
        from_value.arg_schema(spec)["properties"]["value"]
            .get("$schema")
            .is_none()
    );
    let catalog = from_value.catalog();
    let ArgumentSchemaUse::Inline { schema } = &catalog.operations[0].args[0]
        .schema
        .as_ref()
        .expect("inline catalog schema")
    else {
        panic!("expected inline catalog schema")
    };
    assert!(schema.get("$schema").is_none());

    for marker in [json!(1), json!(null), json!("draft-2020-12")] {
        let invalid = make(json!({ "$schema": marker, "type": "string" }));
        assert!(invalid.validate_argument_schemas().is_err());
    }
}

#[test]
fn enumerated_builders_accept_owned_array_and_borrowed_values_and_redact_mismatches() {
    fn build(values: impl IntoIterator<Item = impl AsRef<str>>) -> CommandRegistry {
        CommandRegistry::build("schema-test", "schema test", |server| {
            server.command("scope set", |command| {
                command
                    .summary("Set scope")
                    .description("Set the scope")
                    .arg(arg::enumerated("scope", values).summary("Scope"))
                    .handle(|_context| async { Ok(CommandOutput::structured(json!({}))) });
            });
        })
        .unwrap()
    }

    let owned = vec!["owned".to_string(), "global_readonly".to_string()];
    let array = ["owned", "global_readonly"];
    let owned_registry = build(owned);
    let array_registry = build(array);
    let borrowed_registry = build(array.as_slice());
    assert_eq!(owned_registry.catalog(), array_registry.catalog());
    assert_eq!(array_registry.catalog(), borrowed_registry.catalog());
    assert_eq!(
        owned_registry.catalog_identity(),
        borrowed_registry.catalog_identity()
    );

    let error = owned_registry
        .build_plan(&request(
            "scope set --scope $args.scope",
            json!({ "scope": "caller-secret" }),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        FrameworkError::ArgumentSchemaMismatch {
            argument: "scope".to_string(),
            path: String::new(),
            keyword: ArgumentSchemaKeyword::Enum,
            expected: "\"owned\", \"global_readonly\"".to_string(),
            branches: Vec::new(),
        }
    );
    assert!(!error.to_string().contains("caller-secret"));
}

#[test]
fn arrays_and_nested_unions_preserve_physical_and_escaped_pointers() {
    let schema = json!({
        "type": "array",
        "minItems": 1,
        "items": {
            "type": "object",
            "properties": {
                "kind~/name": {
                    "oneOf": [
                        { "const": "alpha" },
                        { "const": "beta" }
                    ]
                }
            },
            "required": ["kind~/name"],
            "additionalProperties": false
        }
    });
    let registry = registry_with_schema(schema);
    registry.validate_argument_schemas().unwrap();
    let first = registry
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": [{ "kind~/name": "alpha" }, { "kind~/name": "beta" }] }),
        ))
        .unwrap();
    let selections = &first.bound_args["value"].schema_match.selections;
    assert_eq!(selections.len(), 2);
    assert_eq!(selections[0].instance_pointer, "/0/kind~0~1name");
    assert_eq!(
        selections[0].one_of_pointer,
        "/items/properties/kind~0~1name/oneOf"
    );
    assert_eq!(
        selections[0].branch_pointer,
        "/items/properties/kind~0~1name/oneOf/0"
    );
    assert_eq!(selections[1].instance_pointer, "/1/kind~0~1name");
    assert_eq!(
        selections[1].branch_pointer,
        "/items/properties/kind~0~1name/oneOf/1"
    );

    let second = registry
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": [{ "kind~/name": "beta" }, { "kind~/name": "alpha" }] }),
        ))
        .unwrap();
    assert_ne!(first.invocation_fingerprint, second.invocation_fingerprint);

    let mismatch = registry
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": [{ "kind~/name": "caller-secret" }] }),
        ))
        .unwrap_err();
    let FrameworkError::ArgumentSchemaMismatch {
        keyword,
        path,
        branches,
        ..
    } = mismatch
    else {
        panic!("expected an argument schema mismatch")
    };
    assert_eq!(keyword, ArgumentSchemaKeyword::OneOf);
    assert_eq!(path, "/0/kind~0~1name");
    assert_eq!(
        branches,
        vec![
            SchemaBranchProblem {
                pointer: "/items/properties/kind~0~1name/oneOf/0".to_string(),
                path: "/0/kind~0~1name".to_string(),
                keyword: ArgumentSchemaKeyword::Const,
                expected: "\"alpha\"".to_string(),
            },
            SchemaBranchProblem {
                pointer: "/items/properties/kind~0~1name/oneOf/1".to_string(),
                path: "/0/kind~0~1name".to_string(),
                keyword: ArgumentSchemaKeyword::Const,
                expected: "\"beta\"".to_string(),
            },
        ]
    );
    assert!(
        !serde_json::to_string(&branches)
            .unwrap()
            .contains("caller-secret")
    );
}

#[test]
fn discriminated_record_arrays_enforce_items_lengths_and_branch_selection() {
    let registry = registry_with_schema(json!({
        "type": "array",
        "minItems": 1,
        "items": {
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "kind": { "const": "text" },
                        "text": { "type": "string", "minLength": 1 }
                    },
                    "required": ["kind", "text"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {
                        "kind": { "const": "count" },
                        "count": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["kind", "count"],
                    "additionalProperties": false
                }
            ]
        }
    }));
    registry.validate_argument_schemas().unwrap();
    let plan = registry
        .build_plan(&request(
            "value take --value $args.value",
            json!({
                "value": [
                    { "kind": "text", "text": "ready" },
                    { "kind": "count", "count": 2 }
                ]
            }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["value"]
            .schema_match
            .selections
            .iter()
            .map(|selection| selection.branch_pointer.as_str())
            .collect::<Vec<_>>(),
        vec!["/items/oneOf/0", "/items/oneOf/1"]
    );
    assert!(matches!(
        registry
            .build_plan(&request(
                "value take --value $args.value",
                json!({ "value": [] }),
            ))
            .unwrap_err(),
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::MinItems,
            ..
        }
    ));
    assert!(matches!(
        registry
            .build_plan(&request(
                "value take --value $args.value",
                json!({ "value": [{ "kind": "text", "text": "" }] }),
            ))
            .unwrap_err(),
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::OneOf,
            branches,
            ..
        } if branches.len() == 2
            && branches[0].keyword == ArgumentSchemaKeyword::MinLength
            && branches[1].keyword == ArgumentSchemaKeyword::Required
    ));
}

#[test]
fn chained_refs_record_the_final_physical_union_and_scale_linearly() {
    let registry = registry_with_schema(json!({
        "$ref": "#/$defs/list",
        "$defs": {
            "list": {
                "type": "array",
                "items": { "$ref": "#/$defs/choice" }
            },
            "choice": {
                "$ref": "#/$defs/final"
            },
            "final": {
                "oneOf": [
                    { "const": "alpha" },
                    { "const": "beta" }
                ]
            }
        }
    }));
    registry.validate_argument_schemas().unwrap();
    let values = (0..256)
        .map(|index| if index % 2 == 0 { "alpha" } else { "beta" })
        .collect::<Vec<_>>();
    let plan = registry
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": values }),
        ))
        .unwrap();
    let selections = &plan.bound_args["value"].schema_match.selections;
    assert_eq!(selections.len(), 256);
    assert!(selections.iter().all(|selection| {
        selection.one_of_pointer == "/$defs/final/oneOf"
            && selection.branch_pointer.starts_with("/$defs/final/oneOf/")
    }));
    assert_eq!(selections[0].instance_pointer, "/0");
    assert!(
        selections
            .iter()
            .any(|selection| selection.instance_pointer == "/255")
    );
    assert!(selections.windows(2).all(|pair| {
        pair[0].instance_pointer <= pair[1].instance_pointer
            && !pair[1]
                .instance_pointer
                .strip_prefix('/')
                .is_some_and(|index| index.len() > 1 && index.starts_with('0'))
    }));
    let serialized = serde_json::to_string(selections).unwrap();
    assert!(!serialized.contains("alpha"));
    assert!(!serialized.contains("beta"));
}

#[test]
fn wire_spellings_and_empty_schema_match_are_exact() {
    let selection = SchemaBranchSelection {
        schema: "choice".to_string(),
        instance_pointer: "/0".to_string(),
        one_of_pointer: "/oneOf".to_string(),
        branch_pointer: "/oneOf/1".to_string(),
    };
    assert_eq!(
        serde_json::to_value(&selection).unwrap(),
        json!({
            "schema": "choice",
            "instancePointer": "/0",
            "oneOfPointer": "/oneOf",
            "branchPointer": "/oneOf/1"
        })
    );
    assert!(
        serde_json::from_value::<SchemaBranchSelection>(json!({
            "schema": "choice",
            "instance_pointer": "/0",
            "one_of_pointer": "/oneOf",
            "branch_pointer": "/oneOf/1"
        }))
        .is_err()
    );
    for (keyword, spelling) in [
        (ArgumentSchemaKeyword::Minimum, "minimum"),
        (ArgumentSchemaKeyword::Maximum, "maximum"),
        (ArgumentSchemaKeyword::MultipleOf, "multiple_of"),
        (
            ArgumentSchemaKeyword::DependentRequired,
            "dependent_required",
        ),
    ] {
        assert_eq!(serde_json::to_value(keyword).unwrap(), json!(spelling));
    }
    assert!(serde_json::from_value::<ArgumentSchemaKeyword>(json!("MultipleOf")).is_err());
    assert_eq!(
        serde_json::to_value(ArgumentContractReason::TypedDeserializationFailed).unwrap(),
        json!("typed_deserialization_failed")
    );

    let explicit_empty: ArgSchemaMatch =
        serde_json::from_value(json!({ "selections": [] })).unwrap();
    assert_eq!(explicit_empty, ArgSchemaMatch::default());
    let bound = BoundArg {
        name: "value".to_string(),
        value_type: mcp_twill::ArgType::String,
        value: json!("value"),
        workspace: None,
        variants: None,
        schema_match: explicit_empty,
    };
    assert!(
        serde_json::to_value(bound)
            .unwrap()
            .get("schemaMatch")
            .is_none()
    );
}

#[test]
fn typed_open_maps_and_closed_records_enforce_object_boundaries() {
    let open = registry_with_schema(json!({
        "type": "object",
        "properties": {},
        "additionalProperties": { "type": "string" }
    }));
    open.validate_argument_schemas().unwrap();
    open.build_plan(&request(
        "value take --value $args.value",
        json!({ "value": { "label": "ok" } }),
    ))
    .unwrap();
    let error = open
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": { "label": 1 } }),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::Type,
            path,
            ..
        } if path == "/label"
    ));

    let closed = registry_with_schema(json!({
        "type": "object",
        "properties": { "label": { "type": "string" } },
        "required": ["label"],
        "additionalProperties": false
    }));
    closed.validate_argument_schemas().unwrap();
    let error = closed
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": { "label": "ok", "secret": true } }),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::AdditionalProperties,
            ref path,
            ..
        } if path.is_empty()
    ));
    assert!(!error.to_string().contains("secret"));
    let multiple_extras = closed
        .build_plan(&request(
            "value take --value $args.value",
            json!({ "value": { "label": "ok", "secret": true, "token": true } }),
        ))
        .unwrap_err();
    assert_eq!(multiple_extras, error);
    assert!(!multiple_extras.to_string().contains("token"));
}

#[test]
fn named_wait_condition_inlines_selects_and_explains_every_branch() {
    let wait_condition = json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "kind": { "const": "text" },
                    "text": { "type": "string", "minLength": 1 }
                },
                "required": ["kind", "text"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": { "const": "state" },
                    "state": { "type": "string", "enum": ["visible", "hidden"] }
                },
                "required": ["kind", "state"],
                "additionalProperties": false
            }
        ]
    });
    let registry = CommandRegistry::new("schema-test", "schema test")
        .declare_argument_schema(ArgumentSchemaDecl::new(
            "wait-condition",
            "Condition to wait for",
            wait_condition.clone(),
        ))
        .register(
            CommandSpec::new(["page", "wait"], "Wait", "Wait")
                .with_arg(ArgSpec::named_schema(
                    "condition",
                    "wait-condition",
                    "Condition to wait for",
                ))
                .with_arg(ArgSpec::integer("timeout_ms", "Timeout").optional()),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    registry.validate_argument_schemas().unwrap();
    let spec = registry.command_specs().next().unwrap();
    assert_eq!(
        registry.arg_schema(spec)["properties"]["condition"],
        wait_condition
    );
    let plan = registry
        .build_plan(&request(
            "page wait --condition $args.condition",
            json!({ "condition": { "kind": "text", "text": "ready" } }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["condition"].schema_match.selections[0].branch_pointer,
        "/oneOf/0"
    );
    let mismatch = registry
        .build_plan(&request(
            "page wait --condition $args.condition",
            json!({ "condition": { "kind": "unknown" } }),
        ))
        .unwrap_err();
    assert!(matches!(
        mismatch,
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::OneOf,
            branches,
            ..
        } if branches.len() == 2
            && branches[0].pointer == "/oneOf/0"
            && branches[1].pointer == "/oneOf/1"
    ));
}

#[test]
fn declaration_and_operation_order_do_not_change_projection_or_identity() {
    fn make(reverse: bool) -> CommandRegistry {
        let first = ArgumentSchemaDecl::new(
            "alpha-text",
            "Alpha text",
            json!({ "type": "string", "minLength": 1 }),
        );
        let second = ArgumentSchemaDecl::new(
            "beta-text",
            "Beta text",
            json!({ "type": "string", "enum": ["beta"] }),
        );
        let registry = CommandRegistry::new("schema-test", "schema test");
        let registry = if reverse {
            registry
                .declare_argument_schema(second)
                .declare_argument_schema(first)
        } else {
            registry
                .declare_argument_schema(first)
                .declare_argument_schema(second)
        };
        let alpha = CommandSpec::new(["alpha", "take"], "Alpha", "Alpha")
            .with_arg(ArgSpec::named_schema("value", "alpha-text", "Alpha text"));
        let beta = CommandSpec::new(["beta", "take"], "Beta", "Beta")
            .with_arg(ArgSpec::named_schema("value", "beta-text", "Beta text"));
        if reverse {
            registry
                .register(beta, |_context| async {
                    Ok(CommandOutput::structured(json!({})))
                })
                .register(alpha, |_context| async {
                    Ok(CommandOutput::structured(json!({})))
                })
        } else {
            registry
                .register(alpha, |_context| async {
                    Ok(CommandOutput::structured(json!({})))
                })
                .register(beta, |_context| async {
                    Ok(CommandOutput::structured(json!({})))
                })
        }
    }
    let forward = make(false);
    let reverse = make(true);
    forward.validate_argument_schemas().unwrap();
    reverse.validate_argument_schemas().unwrap();
    assert_eq!(forward.catalog(), reverse.catalog());
    assert_eq!(forward.catalog_identity(), reverse.catalog_identity());
    assert_eq!(
        forward.help(HelpRequest::default()),
        reverse.help(HelpRequest::default())
    );
    let run = request(
        "alpha take --value $args.value",
        json!({ "value": "alpha" }),
    );
    assert_eq!(
        forward.build_plan(&run).unwrap().invocation_fingerprint,
        reverse.build_plan(&run).unwrap().invocation_fingerprint
    );
}

#[test]
fn command_schema_assembly_deduplicates_or_rejects_same_name_definitions() {
    let schema = |kind: &str| {
        json!({
            "$ref": "#/$defs/shared",
            "$defs": {
                "shared": { "type": kind }
            }
        })
    };
    let make = |right: Value| {
        CommandRegistry::new("schema-test", "schema test").register(
            CommandSpec::new(["value", "take"], "Take", "Take")
                .with_arg(ArgSpec::inline_schema("left", schema("string"), "Left"))
                .with_arg(ArgSpec::inline_schema("right", right, "Right")),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        )
    };
    let identical = make(schema("string"));
    identical.validate_argument_schemas().unwrap();
    let projected = identical.arg_schema(identical.command_specs().next().unwrap());
    assert_eq!(projected["$defs"].as_object().unwrap().len(), 1);
    assert_eq!(projected["$defs"]["shared"], json!({ "type": "string" }));

    let conflicting = make(schema("integer"));
    assert!(conflicting.validate_argument_schemas().is_err());
}

#[test]
fn legacy_resource_schema_omission_is_stable_and_adoption_changes_identity() {
    let legacy: ResourceDecl = serde_json::from_value(json!({
        "name": "search-index",
        "summary": "Search index",
        "uri": "search://index/{id}"
    }))
    .unwrap();
    let explicit = ResourceDecl {
        reference_schema: None,
        ..legacy.clone()
    };
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::to_value(&explicit).unwrap()
    );
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("referenceSchema")
            .is_none()
    );
    let with_consumer = |resource| {
        CommandRegistry::build("schema-test", "schema test", |server| {
            server.resource(resource);
            server.resolver::<SearchIndex>(SearchIndexResolver);
            server.command("session inspect", |command| {
                command.summary("Inspect").description("Inspect").handle(
                    |_index: Res<SearchIndex>, _context| async {
                        Ok(CommandOutput::structured(json!({})))
                    },
                );
            });
        })
        .unwrap()
    };
    let legacy_registry = with_consumer(legacy);
    let explicit_registry = with_consumer(explicit);
    assert_eq!(
        legacy_registry.catalog_identity(),
        explicit_registry.catalog_identity()
    );
    let adopted = with_consumer(
        ResourceDecl::new("search-index", "Search index")
            .uri("search://index/{id}")
            .reference_schema(json!({ "type": "string", "minLength": 1 })),
    );
    adopted.validate_argument_schemas().unwrap();
    assert_ne!(
        legacy_registry.catalog_identity(),
        adopted.catalog_identity()
    );
}

#[test]
fn path_schema_validation_precedes_and_preserves_workspace_containment() {
    let registry = CommandRegistry::new("schema-test", "schema test")
        .declare_workspace(WorkspaceDecl::file("repo", "file:///workspace/repo"))
        .register(
            CommandSpec::new(["file", "read"], "Read", "Read").with_arg(
                ArgSpec::path("path", "Path", "repo")
                    .with_inline_schema(json!({ "type": "string", "minLength": 1 })),
            ),
            |_context| async { Ok(CommandOutput::structured(json!({}))) },
        );
    registry.validate_argument_schemas().unwrap();
    assert!(matches!(
        registry
            .build_plan(&request(
                "file read --path $args.path",
                json!({ "path": "" }),
            ))
            .unwrap_err(),
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::MinLength,
            ..
        }
    ));
    assert!(matches!(
        registry
            .build_plan(&request(
                "file read --path $args.path",
                json!({ "path": "/etc/passwd" }),
            ))
            .unwrap_err(),
        FrameworkError::WorkspaceMismatch { .. }
    ));
}

#[derive(Debug, Serialize, JsonSchema)]
struct SearchResult {
    query: String,
}

struct SearchIndex;

impl Resource for SearchIndex {
    const NAME: &'static str = "search-index";
}

struct SearchIndexResolver;

impl ResolveResource<SearchIndex> for SearchIndexResolver {
    async fn resolve(
        &self,
        _reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<SearchIndex, ResourceRefusal> {
        Ok(SearchIndex)
    }
}

struct RefusingSearchIndexResolver;

impl ResolveResource<SearchIndex> for RefusingSearchIndexResolver {
    async fn resolve(
        &self,
        _reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<SearchIndex, ResourceRefusal> {
        Err(ResourceRefusal::new("search index is not live"))
    }
}

#[tokio::test]
async fn resource_reference_schema_precedes_and_preserves_specialized_resolution() {
    let registry = CommandRegistry::build("schema-test", "schema test", |server| {
        server.resource(
            ResourceDecl::new("search-index", "Search index")
                .uri("search://index/{id}")
                .carrier("search_index_id")
                .reference_schema(json!({ "type": "string", "minLength": 1 })),
        );
        server.resolver::<SearchIndex>(RefusingSearchIndexResolver);
        server.command("search inspect", |command| {
            command.summary("Inspect").description("Inspect").handle(
                |_index: Res<SearchIndex>, _context| async {
                    Ok(CommandOutput::structured(json!({})))
                },
            );
        });
    })
    .unwrap();
    assert!(matches!(
        registry
            .run(request(
                "search inspect --search-index-id $args.search_index_id",
                json!({ "search_index_id": "" }),
            ))
            .await
            .unwrap_err(),
        FrameworkError::ArgumentSchemaMismatch {
            keyword: ArgumentSchemaKeyword::MinLength,
            ..
        }
    ));
    assert!(matches!(
        registry
            .run(request(
                "search inspect --search-index-id $args.search_index_id",
                json!({ "search_index_id": "wrong://reference" }),
            ))
            .await
            .unwrap_err(),
        FrameworkError::ResourceRefused { .. }
    ));
}

async fn result_search(
    _index: Res<SearchIndex>,
    _context: mcp_twill::CommandContext,
    args: SearchArgs,
) -> ApplicationResult<SearchResult> {
    Ok(ApplicationSuccess::value(SearchResult {
        query: args.query,
    }))
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LegacyCountArgs {
    count: u64,
}

async fn legacy_count_result(
    _context: mcp_twill::CommandContext,
    args: LegacyCountArgs,
) -> ApplicationResult<SearchResult> {
    Ok(ApplicationSuccess::value(SearchResult {
        query: args.count.to_string(),
    }))
}

async fn spoofed_contract_result(
    _context: mcp_twill::CommandContext,
    _args: SearchArgs,
) -> ApplicationResult<SearchResult> {
    Err(FrameworkError::ArgumentContractViolation {
        operation_id: "spoofed.operation".to_string(),
        argument: Some("spoofed-secret".to_string()),
        reason: ArgumentContractReason::TypedDeserializationFailed,
    }
    .into())
}

#[tokio::test]
async fn unconstrained_result_handlers_keep_legacy_typed_extraction() {
    let registry = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("count legacy", |command| {
            command
                .summary("Count")
                .description("Count without an RFC 0017 constraint")
                .arg(arg::number("count").summary("Count"))
                .handle_result(legacy_count_result);
        });
    })
    .unwrap();

    assert!(matches!(
        registry
            .run(request("count legacy --count $args.count", json!({ "count": -1 })))
            .await
            .unwrap_err(),
        FrameworkError::Build(message) if message.starts_with("typed argument extraction failed:")
    ));
    let outcome = registry
        .run(request(
            "count legacy --count $args.count",
            json!({ "count": 2 }),
        ))
        .await
        .unwrap();
    let mcp_twill::CommandExecutionOutcome::Success(response) = outcome else {
        panic!("expected a successful application result")
    };
    assert_eq!(
        response.output.and_then(|output| output.structured),
        Some(json!({ "query": "2" }))
    );
}

#[tokio::test]
async fn result_aware_resource_handlers_reuse_checked_constrained_extraction() {
    let registry = CommandRegistry::build("schema-test", "schema test", |server| {
        server.resource(
            ResourceDecl::new("search-index", "Search index")
                .uri("search://index/{id}")
                .carrier("search_index_id")
                .reference_schema(json!({
                    "$ref": "#/$defs/reference",
                    "$defs": {
                        "reference": { "type": "string", "minLength": 1 }
                    }
                }))
                .expiry("Valid for the server process lifetime"),
        );
        server.resolver::<SearchIndex>(SearchIndexResolver);
        server.command("search run", |command| {
            command
                .summary("Search")
                .description("Search the selected index")
                .arg(
                    arg::string("query")
                        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
                        .summary("Search query"),
                )
                .handle_result(result_search);
        });
    })
    .unwrap();

    let outcome = registry
        .run(request(
            "search run --search-index-id $args.search_index_id --query $args.query",
            json!({ "search_index_id": "main", "query": "twill" }),
        ))
        .await
        .unwrap();
    let mcp_twill::CommandExecutionOutcome::Success(response) = outcome else {
        panic!("expected a successful application result")
    };
    assert_eq!(
        response.output.and_then(|output| output.structured),
        Some(json!({ "query": "twill" }))
    );
}

#[tokio::test]
async fn external_marker_inference_supports_context_and_resource_constrained_handlers() {
    let context_only = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("search direct", |command| {
            command
                .summary("Search")
                .description("Search")
                .arg(
                    arg::string("query")
                        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
                        .summary("Search query"),
                )
                .handle_constrained(|_context, args: SearchArgs| async move {
                    Ok(CommandOutput::structured(json!({ "query": args.query })))
                });
        });
    })
    .unwrap();
    context_only.validate_argument_schemas().unwrap();

    let with_resource = CommandRegistry::build("schema-test", "schema test", |server| {
        server.resource(
            ResourceDecl::new("search-index", "Search index")
                .uri("search://index/{id}")
                .carrier("search_index_id")
                .expiry("Valid for the server process lifetime"),
        );
        server.resolver::<SearchIndex>(SearchIndexResolver);
        server.command("search indexed", |command| {
            command
                .summary("Search")
                .description("Search")
                .arg(
                    arg::string("query")
                        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
                        .summary("Search query"),
                )
                .handle_constrained(
                    |_index: Res<SearchIndex>, _context, args: SearchArgs| async move {
                        Ok(CommandOutput::structured(json!({ "query": args.query })))
                    },
                );
        });
    })
    .unwrap();
    with_resource.validate_argument_schemas().unwrap();
    let outcome = with_resource
        .run(request(
            "search indexed --search-index-id $args.search_index_id --query $args.query",
            json!({ "search_index_id": "main", "query": "twill" }),
        ))
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        mcp_twill::CommandExecutionOutcome::Success(_)
    ));
}

#[derive(Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RejectingArgs {
    query: String,
}

impl<'de> Deserialize<'de> for RejectingArgs {
    fn deserialize<D>(_deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "secret serde failure with rejected caller value",
        ))
    }
}

#[tokio::test]
async fn impossible_typed_extraction_failure_is_static_and_non_disclosing() {
    let registry = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("search run", |command| {
            command
                .summary("Search")
                .description("Search")
                .arg(arg::string("query").summary("Search query"))
                .handle_constrained(|_context, args: RejectingArgs| async move {
                    Ok(CommandOutput::structured(json!({ "query": args.query })))
                });
        });
    })
    .unwrap();
    let error = registry
        .run(request(
            "search run --query $args.query",
            json!({ "query": "caller-secret" }),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        FrameworkError::ArgumentContractViolation {
            operation_id: "search.run".to_string(),
            argument: None,
            reason: ArgumentContractReason::TypedDeserializationFailed,
        }
    );
    let envelope = ResponseEnvelope::framework_error(error, None, None);
    let event = FrameworkEvent::from_envelope(&envelope, None);
    assert_eq!(event.operation_id.as_deref(), Some("search.run"));
    assert_eq!(
        event.argument_contract_reason,
        Some(ArgumentContractReason::TypedDeserializationFailed)
    );
    let public = serde_json::to_string(&(envelope, event)).unwrap();
    for secret in ["caller-secret", "secret serde", "RejectingArgs"] {
        assert!(
            !public.contains(secret),
            "public projection leaked {secret}"
        );
    }
    assert!(public.contains("typed_deserialization_failed"));
}

#[tokio::test]
async fn handlers_cannot_fabricate_argument_contract_failures() {
    let dynamic = CommandRegistry::new("schema-test", "schema test").register(
        CommandSpec::new(["value", "dynamic"], "Dynamic", "Dynamic"),
        |_context| async {
            Err(FrameworkError::ArgumentContractViolation {
                operation_id: "spoofed.operation".to_string(),
                argument: Some("spoofed-secret".to_string()),
                reason: ArgumentContractReason::DerivedSchemaDrift,
            })
        },
    );
    assert_eq!(
        dynamic
            .run(request("value dynamic", json!({})))
            .await
            .unwrap_err(),
        FrameworkError::Handler("handler returned invalid argument contract violation".to_string())
    );

    let constrained = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("search spoof", |command| {
            command
                .summary("Search")
                .description("Search")
                .arg(
                    arg::string("query")
                        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
                        .summary("Search query"),
                )
                .handle_constrained(|_context, _args: SearchArgs| async move {
                    Err::<CommandOutput, _>(FrameworkError::ArgumentContractViolation {
                        operation_id: "spoofed.operation".to_string(),
                        argument: Some("spoofed-secret".to_string()),
                        reason: ArgumentContractReason::TypedDeserializationFailed,
                    })
                });
        });
    })
    .unwrap();
    assert_eq!(
        constrained
            .run(request(
                "search spoof --query $args.query",
                json!({ "query": "safe" }),
            ))
            .await
            .unwrap_err(),
        FrameworkError::Handler("handler returned invalid argument contract violation".to_string())
    );

    let result_aware = CommandRegistry::build("schema-test", "schema test", |server| {
        server.command("search result-spoof", |command| {
            command
                .summary("Search")
                .description("Search")
                .arg(
                    arg::string("query")
                        .with_inline_schema(json!({ "type": "string", "minLength": 1 }))
                        .summary("Search query"),
                )
                .handle_result(spoofed_contract_result);
        });
    })
    .unwrap();
    assert_eq!(
        result_aware
            .run(request(
                "search result-spoof --query $args.query",
                json!({ "query": "safe" }),
            ))
            .await
            .unwrap_err(),
        FrameworkError::Handler("handler returned invalid argument contract violation".to_string())
    );
}
