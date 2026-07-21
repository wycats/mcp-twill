use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use futures::StreamExt;
use mcp_twill::{
    ApplicationOutput, ApplicationOutputResult, ApplicationResult, ApplicationResultContract,
    ApplicationSuccess, ArgSpec, CapabilityDecl, CliMcpServer, CliMcpServerConfig, CommandContext,
    CommandExample, CommandOutput, CommandRegistry, CommandSpec, DynamicCommandFailure,
    FrameworkHelpProjection, Grant, InvocationContext, InvocationOrigin, InvocationPlan, Listing,
    McpProtocolTarget, NativeApplicationErrorDialect, NativeConfirmationBridge,
    NativeConfirmationBridgeError, NativeConfirmationDecision, NativeConfirmationRequest,
    NativeConfirmationRoute, NativeExposurePolicy, NativeToolSurface, OutputContract,
    PermissionAuthorizer, PermissionDecision, PermissionEffect, PermissionSpec, Release,
    ResolveResource, Resource, ResourceDecl, ResourceRefusal, RunMode, RunRequest,
    ServingSurfaceIdentity, TaskDeliveryDecl, TaskSupportSpec,
};
use rmcp::{
    ClientHandler, ServiceExt,
    handler::client::progress::ProgressDispatcher,
    model::{
        CallToolRequestParams, CancelTaskParams, ClientInfo, ClientRequest, GetPromptRequestParams,
        GetTaskInfoParams, GetTaskResultParams, Meta, ProgressNotificationParam, ProtocolVersion,
        ReadResourceRequestParams, Request, ResourceContents,
    },
    service::PeerRequestOptions,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

#[path = "support/vbl.rs"]
mod vbl;
#[path = "support/vbl_native.rs"]
mod vbl_native;

fn vbl_fixture(name: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/vbl/v0.4.9")
        .join(name);
    serde_json::from_slice(&std::fs::read(root).expect("read VBL fixture"))
        .expect("parse VBL fixture")
}

fn object_contract(properties: Value, required: &[&str]) -> ApplicationResultContract {
    ApplicationResultContract::new(json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    }))
}

fn item_registry(task_support: TaskSupportSpec) -> CommandRegistry {
    item_registry_with_calls(task_support, Arc::new(AtomicUsize::new(0)))
}

fn item_registry_with_calls(
    task_support: TaskSupportSpec,
    calls: Arc<AtomicUsize>,
) -> CommandRegistry {
    let list = CommandSpec::new(["items", "list"], "List items", "List stored items")
        .task_support(task_support.clone())
        .with_arg(ArgSpec::string("query", "Optional query").optional())
        .with_output(OutputContract {
            application: Some(object_contract(
                json!({ "items": { "type": "array", "items": { "type": "string" } } }),
                &["items"],
            )),
            ..OutputContract::default()
        });
    let get = CommandSpec::new(["items", "get"], "Get item", "Read one stored item")
        .task_support(task_support)
        .with_arg(ArgSpec::string("id", "Item id"))
        .with_output(OutputContract {
            application: Some(object_contract(
                json!({
                    "id": { "type": "string" },
                    "value": { "type": "string" }
                }),
                &["id", "value"],
            )),
            ..OutputContract::default()
        })
        .with_permission(PermissionSpec::new(
            PermissionEffect::Read,
            "items",
            "Reads one item",
        ));

    CommandRegistry::new("native-test", "Native surface tests")
        .declare_preamble("Use the named item tools.")
        .register_dynamic(list, |_context| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "items": ["one", "two"]
            })))
        })
        .register_dynamic(get, move |context: CommandContext| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let id = context
                    .plan
                    .bound_args
                    .get("id")
                    .map(|argument| &argument.value)
                    .and_then(Value::as_str)
                    .unwrap_or("missing");
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                    "id": id,
                    "value": "found"
                })))
            }
        })
}

struct TestLease {
    id: String,
}

impl Resource for TestLease {
    const NAME: &'static str = "test-lease";
}

struct TestLeaseResolver;

impl ResolveResource<TestLease> for TestLeaseResolver {
    async fn resolve(
        &self,
        reference: &str,
        _plan: &InvocationPlan,
    ) -> std::result::Result<TestLease, ResourceRefusal> {
        Ok(TestLease {
            id: reference.to_string(),
        })
    }
}

#[derive(Serialize, JsonSchema)]
struct TestLeaseResult {
    id: String,
}

async fn grant_test_lease(
    _: CommandContext,
) -> ApplicationOutputResult<mcp_twill::Granted<TestLease, ApplicationSuccess<TestLeaseResult>>> {
    Ok(ApplicationSuccess::value(TestLeaseResult {
        id: "lease-1".to_string(),
    })
    .grant(Grant::<TestLease>::new("lease-1")))
}

async fn release_test_lease(
    lease: Release<TestLease>,
    _: CommandContext,
) -> ApplicationResult<TestLeaseResult> {
    Ok(ApplicationSuccess::value(TestLeaseResult {
        id: lease.id.clone(),
    }))
}

async fn list_test_leases(
    _: CommandContext,
) -> ApplicationOutputResult<mcp_twill::Listed<TestLease, ApplicationSuccess<TestLeaseResult>>> {
    Ok(ApplicationSuccess::value(TestLeaseResult {
        id: "lease-1".to_string(),
    })
    .listing(Listing::<TestLease>::new(["lease-1"])))
}

fn grouped_surface(
    registry: &CommandRegistry,
    route: NativeConfirmationRoute,
) -> mcp_twill::Result<NativeToolSurface> {
    NativeToolSurface::builder("item-tools")
        .framework_help(FrameworkHelpProjection::Tool {
            name: "framework-help".to_string(),
        })
        .application_errors(NativeApplicationErrorDialect::Canonical)
        .confirmation_route(route)
        .group("items", |group| {
            group
                .selector("operation")
                .member("list", "items.list")
                .member("get", "items.get")
                .title("Items")
                .description("List or read stored items.");
        })
        .build(registry, McpProtocolTarget::V2025_11_25)
}

fn shape_registry(outputs: &[(&str, Value)]) -> CommandRegistry {
    outputs.iter().fold(
        CommandRegistry::new("shape-test", "Native schema validation"),
        |registry, (name, schema)| {
            let spec =
                CommandSpec::new(["shape", *name], *name, *name).with_output(OutputContract {
                    application: Some(ApplicationResultContract::new(schema.clone())),
                    ..OutputContract::default()
                });
            registry.register_dynamic(spec, |_context| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
            })
        },
    )
}

fn application_error_registry() -> CommandRegistry {
    let success = object_contract(json!({ "started": { "type": "boolean" } }), &["started"]);
    let failing = object_contract(json!({ "opened": { "type": "boolean" } }), &["opened"])
        .with_error_spec(mcp_twill::ApplicationErrorSpec {
            code: "session_required".to_string(),
            summary: "this host supplied no conversation identity; call start_session and pass its agent_session_id".to_string(),
            message: mcp_twill::ApplicationMessageDecl::DeclarationSummary,
            details_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            capability: None,
            recoveries: vec![mcp_twill::ApplicationRecoveryDecl::Operation {
                operation_id: "session.start".to_string(),
            }],
            recovery_cardinality: mcp_twill::RecoveryCardinality::AtMostOne,
        });
    CommandRegistry::new("errors", "Native application errors")
        .register_dynamic(
            CommandSpec::new(["session", "start"], "Start", "Start a session").with_output(
                OutputContract {
                    application: Some(success),
                    ..OutputContract::default()
                },
            ),
            |_| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                    "started": true
                })))
            },
        )
        .register_dynamic(
            CommandSpec::new(["browser", "open"], "Open", "Open a browser").with_output(
                OutputContract {
                    application: Some(failing),
                    ..OutputContract::default()
                },
            ),
            |_| async {
                Err::<ApplicationSuccess<Value>, _>(
                    mcp_twill::DynamicApplicationError::new("session_required").into(),
                )
            },
        )
}

fn native_capability_registry() -> CommandRegistry {
    let provider = CommandSpec::new(
        ["build", "validate"],
        "Validate build",
        "Validate the current build",
    )
    .provides("validated-build")
    .with_output(OutputContract {
        application: Some(object_contract(
            json!({ "receipt": { "type": "string" } }),
            &["receipt"],
        )),
        ..OutputContract::default()
    });
    let mut example = CommandExample::new(
        "deploy publish --validation-token $args.validation_token",
        "Publish a validated build",
    );
    example
        .args
        .insert("validation_token".to_string(), json!("receipt-1"));
    let consumer = CommandSpec::new(
        ["deploy", "publish"],
        "Publish build",
        "Publish a validated build",
    )
    .with_arg(ArgSpec::string(
        "validation_token",
        "Opaque validation receipt",
    ))
    .with_example(example)
    .requires("validated-build")
    .with_output(OutputContract {
        application: Some(object_contract(
            json!({ "published": { "type": "boolean" } }),
            &["published"],
        )),
        ..OutputContract::default()
    });
    CommandRegistry::new("capability-native", "Native capability errors")
        .declare_capability(
            CapabilityDecl::new("validated-build", "Proof of build validation")
                .carried_by("validation_token"),
        )
        .register_dynamic(provider, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "receipt": "receipt-1"
            })))
        })
        .register_dynamic(consumer, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "published": true
            })))
        })
}

fn application_error_surface(
    registry: &CommandRegistry,
    dialect: NativeApplicationErrorDialect,
) -> mcp_twill::Result<NativeToolSurface> {
    NativeToolSurface::builder("browser-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .application_errors(dialect)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("start_session", "session.start")
        .direct("new_tab", "browser.open")
        .build(registry, McpProtocolTarget::V2025_11_25)
}

fn direct_shape_surface(
    registry: &CommandRegistry,
    names: &[&str],
) -> mcp_twill::Result<NativeToolSurface> {
    let mut builder = NativeToolSurface::builder("shape-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable);
    for name in names {
        builder = builder.direct(*name, format!("shape.{name}"));
    }
    builder.build(registry, McpProtocolTarget::V2025_11_25)
}

#[test]
fn declaration_and_snapshot_use_the_accepted_wire_contract() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
    let declaration = serde_json::to_value(surface.declaration())?;
    assert_eq!(declaration["name"], "item-tools");
    assert!(declaration.get("exposure").is_none());
    assert!(declaration.get("applicationErrors").is_none());
    assert_eq!(declaration["confirmation"], "unavailable");
    assert_eq!(declaration["tools"][0]["group"]["selector"], "operation");

    let snapshot = surface.snapshot();
    assert_eq!(snapshot.version(), 1);
    assert_eq!(snapshot.protocol_version(), "2025-11-25");
    assert_eq!(snapshot.name(), "item-tools");
    assert_eq!(
        snapshot.catalog_hash(),
        registry.catalog_identity().catalog_hash
    );
    assert_eq!(snapshot.surface_hash().len(), 64);
    assert_eq!(snapshot.canonical_json(), snapshot.canonical_json());
    assert_eq!(snapshot.operations().len(), 2);
    assert_eq!(snapshot.operations()[1].call().tool(), "items");
    let items = snapshot
        .tools()
        .iter()
        .find(|tool| tool.name == "items")
        .expect("compiled items tool");
    assert_eq!(items.title.as_deref(), Some("Items"));
    assert_eq!(
        items
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.title.as_deref()),
        Some("Items")
    );
    assert_eq!(
        snapshot.operations()[1].call().arguments(),
        Some(&BTreeMap::from([("operation".to_string(), json!("get"),)]))
    );
    assert_eq!(snapshot.document()["surfaceHash"], Value::Null);
    Ok(())
}

#[test]
fn serving_surface_identity_is_closed_and_validated() -> anyhow::Result<()> {
    let identity = ServingSurfaceIdentity::new("native-items", "a".repeat(64))?;
    assert_eq!(
        serde_json::to_value(&identity)?,
        json!({ "name": "native-items", "hash": "a".repeat(64) })
    );
    assert!(
        serde_json::from_value::<ServingSurfaceIdentity>(json!({
            "name": "Native_Items",
            "hash": "a".repeat(64)
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ServingSurfaceIdentity>(json!({
            "name": "native-items",
            "hash": "a".repeat(64),
            "extra": true
        }))
        .is_err()
    );
    let schema = serde_json::to_value(schemars::schema_for!(ServingSurfaceIdentity))?;
    assert_eq!(schema["additionalProperties"], false);
    Ok(())
}

#[test]
fn native_exec_tools_are_conservatively_open_world() -> anyhow::Result<()> {
    let output = OutputContract {
        application: Some(object_contract(
            json!({ "ok": { "type": "boolean" } }),
            &["ok"],
        )),
        ..OutputContract::default()
    };
    let registry = CommandRegistry::new("effects", "Effect annotations")
        .register_dynamic(
            CommandSpec::new(["process", "run"], "Run process", "Run a process")
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Exec,
                    "process",
                    "Runs an external process",
                ))
                .with_output(output.clone()),
            |_| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({ "ok": true })))
            },
        )
        .register_dynamic(
            CommandSpec::new(["remote", "fetch"], "Fetch remote", "Fetch remote data")
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Network,
                    "remote",
                    "Contacts an external service",
                ))
                .with_output(output),
            |_| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({ "ok": true })))
            },
        );
    let surface = NativeToolSurface::builder("exec-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("run_process", "process.run")
        .direct("fetch_remote", "remote.fetch")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let tools = surface
        .snapshot()
        .tools()
        .iter()
        .map(|tool| {
            Ok((
                tool.name.to_string(),
                serde_json::to_value(&tool.annotations)?,
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    assert_eq!(tools["run_process"]["readOnlyHint"], false);
    assert_eq!(tools["run_process"]["destructiveHint"], true);
    assert_eq!(tools["run_process"]["openWorldHint"], true);
    assert_eq!(tools["fetch_remote"]["readOnlyHint"], true);
    assert_eq!(tools["fetch_remote"]["destructiveHint"], false);
    assert_eq!(tools["fetch_remote"]["openWorldHint"], true);
    Ok(())
}

#[test]
fn builder_slots_and_exposure_are_canonical() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let repeated = NativeToolSurface::builder("item-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("list", "items.list")
        .direct("get", "items.get")
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(repeated.to_string().contains("more than once"));

    let complete = NativeToolSurface::builder("item-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("list", "items.list")
        .direct("get", "items.get")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let empty_subset = NativeToolSurface::builder("item-tools")
        .exposure(NativeExposurePolicy::explicit_subset([""; 0]))
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("list", "items.list")
        .direct("get", "items.get")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    assert_eq!(
        complete.snapshot().surface_hash(),
        empty_subset.snapshot().surface_hash()
    );
    Ok(())
}

#[test]
fn explicit_subsets_preserve_granted_resource_lifecycle_edges() -> anyhow::Result<()> {
    let registry = CommandRegistry::build("leases", "Lease lifecycle", |server| {
        server.resource(
            ResourceDecl::new("test-lease", "A test lease")
                .uri("test://lease/{id}")
                .expiry("Test leases expire"),
        );
        server.resolver::<TestLease>(TestLeaseResolver);
        server.command("lease start", |command| {
            command
                .summary("Start lease")
                .description("Grant a lease")
                .handle_result(grant_test_lease);
        });
        server.command("lease end", |command| {
            command
                .summary("End lease")
                .description("Release a lease")
                .handle_result(release_test_lease);
        });
    })?;
    let error = NativeToolSurface::builder("lease-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("start_lease", "lease.start")
        .exposure(NativeExposurePolicy::explicit_subset(["lease.end"]))
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("lease.end"));
    assert!(error.to_string().contains("reachable from `lease.start`"));
    Ok(())
}

#[test]
fn explicit_subsets_preserve_enumerated_resource_lifecycle_edges() -> anyhow::Result<()> {
    let registry = CommandRegistry::build("leases", "Lease lifecycle", |server| {
        server.resource(
            ResourceDecl::new("test-lease", "A test lease")
                .uri("test://lease/{id}")
                .expiry("Test leases expire"),
        );
        server.resolver::<TestLease>(TestLeaseResolver);
        server.command("lease start", |command| {
            command
                .summary("Start lease")
                .description("Grant a lease")
                .handle_result(grant_test_lease);
        });
        server.command("lease list", |command| {
            command
                .summary("List leases")
                .description("Enumerate leases")
                .handle_result(list_test_leases);
        });
        server.command("lease end", |command| {
            command
                .summary("End lease")
                .description("Release a lease")
                .handle_result(release_test_lease);
        });
    })?;
    let error = NativeToolSurface::builder("lease-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("list_leases", "lease.list")
        .exposure(NativeExposurePolicy::explicit_subset([
            "lease.start",
            "lease.end",
        ]))
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("lease.end"));
    assert!(error.to_string().contains("reachable from `lease.list`"));
    Ok(())
}

#[test]
fn declarations_validate_names_coverage_and_required_slots() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    for name in [
        "",
        "UPPER",
        "has_underscore",
        "-option",
        "a--b",
        &"a".repeat(65),
    ] {
        let error = NativeToolSurface::builder(name)
            .framework_help(FrameworkHelpProjection::Omitted)
            .confirmation_route(NativeConfirmationRoute::Unavailable)
            .direct("list", "items.list")
            .exposure(NativeExposurePolicy::explicit_subset(["items.get"]))
            .build(&registry, McpProtocolTarget::V2025_11_25)
            .unwrap_err();
        assert!(error.to_string().contains("surface"));
    }

    let missing_slots = NativeToolSurface::builder("items")
        .direct("list", "items.list")
        .exposure(NativeExposurePolicy::explicit_subset(["items.get"]))
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(missing_slots.to_string().contains("framework help"));

    let missing_operation = NativeToolSurface::builder("items")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("list", "items.list")
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(missing_operation.to_string().contains("does not account"));

    let duplicate = NativeToolSurface::builder("items")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("first", "items.list")
        .direct("second", "items.list")
        .exposure(NativeExposurePolicy::explicit_subset(["items.get"]))
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(duplicate.to_string().contains("mapped more than once"));

    let unknown = NativeToolSurface::builder("items")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("list", "items.list")
        .exposure(NativeExposurePolicy::explicit_subset([
            "items.get",
            "items.unknown",
        ]))
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(unknown.to_string().contains("unknown operation"));

    assert!(
        serde_json::from_value::<mcp_twill::NativeToolSurfaceDecl>(json!({
            "name": "items",
            "frameworkHelp": "omitted"
        }))
        .is_err()
    );
    Ok(())
}

#[test]
fn output_roots_and_group_selector_collisions_fail_closed() -> anyhow::Result<()> {
    let scalar = shape_registry(&[("scalar", json!({ "type": "string" }))]);
    let error = direct_shape_surface(&scalar, &["scalar"]).unwrap_err();
    assert!(error.to_string().contains("object-only"));

    let implicit_object = shape_registry(&[(
        "implicit",
        json!({ "properties": {}, "additionalProperties": false }),
    )]);
    let error = direct_shape_surface(&implicit_object, &["implicit"]).unwrap_err();
    assert!(error.to_string().contains("object-only"));

    let mixed = shape_registry(&[(
        "mixed",
        json!({
            "oneOf": [
                { "type": "object", "properties": {}, "additionalProperties": false },
                { "type": "null" }
            ]
        }),
    )]);
    let error = direct_shape_surface(&mixed, &["mixed"]).unwrap_err();
    assert!(error.to_string().contains("object-only"));

    let union = shape_registry(&[
        (
            "states",
            json!({
                "oneOf": [
                    {
                        "type": "object",
                        "properties": { "state": { "const": "ready", "type": "string" } },
                        "required": ["state"],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "properties": { "state": { "const": "closed", "type": "string" } },
                        "required": ["state"],
                        "additionalProperties": false
                    }
                ]
            }),
        ),
        (
            "plain",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]);
    let surface = NativeToolSurface::builder("unions")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .group("states", |group| {
            group
                .selector("operation")
                .member("states", "shape.states")
                .member("plain", "shape.plain");
        })
        .build(&union, McpProtocolTarget::V2025_11_25)?;
    let output = surface.snapshot().tools()[0]
        .output_schema
        .as_ref()
        .expect("group output");
    assert_eq!(output["oneOf"][0]["oneOf"].as_array().unwrap().len(), 2);

    let constrained_union = shape_registry(&[
        (
            "states",
            json!({
                "type": "object",
                "properties": { "state": { "type": "string" } },
                "required": ["state"],
                "additionalProperties": false,
                "oneOf": [
                    {
                        "type": "object",
                        "properties": { "state": { "const": "ready", "type": "string" } },
                        "required": ["state"]
                    },
                    {
                        "type": "object",
                        "properties": { "state": { "const": "closed", "type": "string" } },
                        "required": ["state"]
                    }
                ]
            }),
        ),
        (
            "plain",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]);
    let surface = NativeToolSurface::builder("constrained-union")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .group("states", |group| {
            group
                .selector("operation")
                .member("states", "shape.states")
                .member("plain", "shape.plain");
        })
        .build(&constrained_union, McpProtocolTarget::V2025_11_25)?;
    let member_union = &surface.snapshot().tools()[0]
        .output_schema
        .as_ref()
        .unwrap()["oneOf"][0];
    assert_eq!(
        member_union["properties"]["operation"],
        json!({ "type": "string", "const": "states" })
    );
    assert!(
        member_union["required"]
            .as_array()
            .unwrap()
            .contains(&json!("operation"))
    );
    assert_eq!(
        member_union["oneOf"][0]["properties"]["operation"],
        json!({ "type": "string", "const": "states" })
    );

    let conflicting = shape_registry(&[
        (
            "bad",
            json!({
                "type": "object",
                "properties": { "operation": { "const": "wrong", "type": "string" } },
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
        (
            "plain",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]);
    let error = NativeToolSurface::builder("conflict")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .group("conflict", |group| {
            group
                .selector("operation")
                .member("bad", "shape.bad")
                .member("plain", "shape.plain");
        })
        .build(&conflicting, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("selector"));

    let open = shape_registry(&[
        (
            "open",
            json!({
                "type": "object",
                "properties": { "value": { "type": "string" } }
            }),
        ),
        (
            "closed",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]);
    let error = NativeToolSurface::builder("open-output")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .group("open-output", |group| {
            group
                .selector("operation")
                .member("open", "shape.open")
                .member("closed", "shape.closed");
        })
        .build(&open, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must exclude undeclared selector")
    );

    let equivalent = shape_registry(&[
        (
            "enumerated",
            json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string", "enum": ["enumerated"] }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
        (
            "referenced",
            json!({
                "$defs": {
                    "selector": { "type": "string", "enum": ["referenced"] }
                },
                "type": "object",
                "properties": {
                    "operation": { "$ref": "#/$defs/selector" }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
        ),
    ]);
    let surface = NativeToolSurface::builder("equivalent")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .group("equivalent", |group| {
            group
                .selector("operation")
                .member("enumerated", "shape.enumerated")
                .member("referenced", "shape.referenced");
        })
        .build(&equivalent, McpProtocolTarget::V2025_11_25)?;
    assert_eq!(surface.snapshot().tools().len(), 1);
    Ok(())
}

#[test]
fn grouped_inputs_require_compatible_properties_and_presence_edges() -> anyhow::Result<()> {
    fn registry(second_requires_height: bool, incompatible_width: bool) -> CommandRegistry {
        let output = OutputContract {
            application: Some(object_contract(json!({}), &[])),
            ..OutputContract::default()
        };
        let first = CommandSpec::new(["resize", "first"], "First", "First")
            .with_arg(
                ArgSpec::integer("width", "Width")
                    .optional()
                    .requires_argument("height"),
            )
            .with_arg(ArgSpec::integer("height", "Height").optional())
            .with_output(output.clone());
        let width = if incompatible_width {
            ArgSpec::string("width", "Width").optional()
        } else {
            let width = ArgSpec::integer("width", "Width").optional();
            if second_requires_height {
                width.requires_argument("height")
            } else {
                width
            }
        };
        let second = CommandSpec::new(["resize", "second"], "Second", "Second")
            .with_arg(width)
            .with_arg(ArgSpec::integer("height", "Height").optional())
            .with_output(output);
        CommandRegistry::new("resize", "Resize")
            .register_dynamic(first, |_| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
            })
            .register_dynamic(second, |_| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
            })
    }
    fn build(registry: &CommandRegistry) -> mcp_twill::Result<NativeToolSurface> {
        NativeToolSurface::builder("resize")
            .framework_help(FrameworkHelpProjection::Omitted)
            .confirmation_route(NativeConfirmationRoute::Unavailable)
            .group("resize", |group| {
                group
                    .selector("operation")
                    .member("first", "resize.first")
                    .member("second", "resize.second");
            })
            .build(registry, McpProtocolTarget::V2025_11_25)
    }

    let surface = build(&registry(true, false))?;
    assert_eq!(
        surface.snapshot().tools()[0].input_schema["dependencies"],
        json!({ "width": ["height"] })
    );
    let edge_error = build(&registry(false, false)).unwrap_err();
    assert!(edge_error.to_string().contains("presence relationships"));
    let schema_error = build(&registry(true, true)).unwrap_err();
    assert!(schema_error.to_string().contains("incompatible schema"));
    Ok(())
}

#[test]
fn adapter_finalization_rejects_stale_surfaces_and_sidecar_mismatches() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let unavailable = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
    let stale_registry = item_registry(TaskSupportSpec::Forbidden);
    let stale = match CliMcpServer::with_surface(stale_registry, unavailable.clone()) {
        Ok(_) => anyhow::bail!("expected stale surface rejection"),
        Err(error) => error,
    };
    assert!(stale.to_string().contains("different command catalog"));

    let bridge = grouped_surface(&registry, NativeConfirmationRoute::Bridge)?;
    let missing_bridge = match CliMcpServer::with_surface(registry.clone(), bridge) {
        Ok(_) => anyhow::bail!("expected missing bridge rejection"),
        Err(error) => error,
    };
    assert!(missing_bridge.to_string().contains("requires a bridge"));

    let unexpected_bridge = CliMcpServer::builder(registry)
        .surface(unavailable)
        .native_confirmation_bridge(Arc::new(CountingBridge {
            calls: Arc::new(AtomicUsize::new(0)),
            decision: NativeConfirmationDecision::Allow,
        }))
        .build();
    let error = match unexpected_bridge {
        Ok(_) => anyhow::bail!("expected unexpected bridge rejection"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("rejects a bridge"));

    let required_registry = item_registry(TaskSupportSpec::Required);
    let required =
        grouped_surface(&required_registry, NativeConfirmationRoute::Unavailable).unwrap_err();
    assert!(
        required
            .to_string()
            .contains("disabled task delivery cannot expose")
    );
    Ok(())
}

#[test]
fn confirmation_bridge_errors_never_expose_their_source() {
    let error = NativeConfirmationBridgeError::new(std::io::Error::other("private bridge secret"));
    assert!(std::error::Error::source(&error).is_none());
    assert_eq!(error.to_string(), "native confirmation bridge failed");
    assert_eq!(
        format!("{error:?}"),
        "NativeConfirmationBridgeError(<redacted>)"
    );
}

#[tokio::test]
async fn native_application_errors_use_the_selected_surface_dialect() -> anyhow::Result<()> {
    let registry = application_error_registry();
    let surface = application_error_surface(&registry, NativeApplicationErrorDialect::Canonical)?;
    let result = call_native_tool(
        CliMcpServer::with_surface(registry, surface)?,
        "new_tab",
        json!({}),
    )
    .await?;
    assert_eq!(result.is_error, Some(true));
    assert!(result.structured_content.is_none());
    let text = &result.content[0].as_text().unwrap().text;
    let canonical: Value = serde_json::from_str(text)?;
    assert_eq!(canonical["code"], "session_required");
    assert_eq!(canonical["details"], json!({}));
    assert_eq!(
        canonical["recoveries"],
        json!([{ "kind": "tool", "tool": "start_session" }])
    );
    assert_eq!(text, &serde_json::to_string(&canonical)?);

    let registry = application_error_registry();
    let surface =
        application_error_surface(&registry, NativeApplicationErrorDialect::FlatSingleRecovery)?;
    let result = call_native_tool(
        CliMcpServer::with_surface(registry, surface)?,
        "new_tab",
        json!({}),
    )
    .await?;
    let flat: Value = serde_json::from_str(&result.content[0].as_text().unwrap().text)?;
    let vectors = vbl_fixture("application-error-vectors.json");
    let expected = vectors["serializationVectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["constructor"] == "session_required")
        .unwrap()["error"]
        .clone();
    assert_eq!(flat, expected);
    Ok(())
}

#[test]
fn flat_recovery_actions_cannot_collide_with_exposed_tool_names() {
    let success = object_contract(json!({ "started": { "type": "boolean" } }), &["started"]);
    let failing = object_contract(json!({ "opened": { "type": "boolean" } }), &["opened"])
        .with_error_spec(mcp_twill::ApplicationErrorSpec {
            code: "session_required".to_string(),
            summary: "Start a session first".to_string(),
            message: mcp_twill::ApplicationMessageDecl::DeclarationSummary,
            details_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            capability: None,
            recoveries: vec![mcp_twill::ApplicationRecoveryDecl::Action(
                mcp_twill::ApplicationActionDecl {
                    code: "start_session".to_string(),
                    summary: "Start a session manually".to_string(),
                },
            )],
            recovery_cardinality: mcp_twill::RecoveryCardinality::AtMostOne,
        });
    let registry = CommandRegistry::new("errors", "Native application errors")
        .register_dynamic(
            CommandSpec::new(["session", "start"], "Start", "Start a session").with_output(
                OutputContract {
                    application: Some(success),
                    ..OutputContract::default()
                },
            ),
            |_| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                    "started": true
                })))
            },
        )
        .register_dynamic(
            CommandSpec::new(["browser", "open"], "Open", "Open a browser").with_output(
                OutputContract {
                    application: Some(failing),
                    ..OutputContract::default()
                },
            ),
            |_| async {
                Err::<ApplicationSuccess<Value>, _>(
                    mcp_twill::DynamicApplicationError::new("session_required").into(),
                )
            },
        );
    let error =
        application_error_surface(&registry, NativeApplicationErrorDialect::FlatSingleRecovery)
            .unwrap_err();
    assert!(error.to_string().contains("ambiguous with an exposed tool"));
}

#[tokio::test]
async fn duplicate_operation_ids_fail_native_compilation_and_bare_routing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = CommandRegistry::new("duplicates", "Duplicate operation ids")
        .register(CommandSpec::new(["a.b"], "Flat", "Flat path"), {
            let calls = calls.clone();
            move |_| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(CommandOutput::structured(json!({})))
                }
            }
        })
        .register(CommandSpec::new(["a", "b"], "Nested", "Nested path"), {
            let calls = calls.clone();
            move |_| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(CommandOutput::structured(json!({})))
                }
            }
        });
    let error = NativeToolSurface::builder("duplicate-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("ambiguous", "a.b")
        .build(&registry, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("duplicate native operation id"));
    let error = registry
        .run_operation_with_context("a.b", serde_json::Map::new(), InvocationContext::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ambiguous operation id `a.b`"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn operation_id_planning_is_distinct_from_command_templates() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let runtime = tokio::runtime::Runtime::new()?;
    let outcome = runtime.block_on(registry.run_operation_with_context(
        "items.get",
        serde_json::from_value(json!({ "id": "42" }))?,
        InvocationContext::default(),
    ))?;
    let mcp_twill::CommandExecutionOutcome::Success(response) = outcome else {
        anyhow::bail!("expected success");
    };
    assert_eq!(response.plan.origin, InvocationOrigin::OperationId);
    assert_eq!(response.plan.raw_command, None);
    assert!(response.plan.tokens.is_empty());
    assert!(response.plan.surface.is_none());
    let wire = serde_json::to_value(&response.plan)?;
    assert_eq!(wire["origin"], "operationId");
    assert!(wire.get("rawCommand").is_none());
    assert!(wire.get("surface").is_none());
    Ok(())
}

#[tokio::test]
async fn operation_id_execution_preserves_registry_validation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let spec = CommandSpec::new(["invalid"], "Invalid", "Invalid legacy result registration")
        .with_output(OutputContract {
            application: Some(object_contract(json!({}), &[])),
            ..OutputContract::default()
        });
    let registry = CommandRegistry::new("invalid", "Invalid registry").register(spec, {
        let calls = calls.clone();
        move |_| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(CommandOutput::structured(json!({})))
            }
        }
    });
    let error = registry
        .run_operation_with_context(
            "invalid",
            serde_json::Map::new(),
            InvocationContext::default(),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("legacy command `invalid` cannot declare")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn native_group_dispatches_without_a_command_string() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
    let expected_surface_hash = surface.snapshot().surface_hash().to_string();
    let server = CliMcpServer::with_surface(registry, surface)?;
    assert_eq!(
        server.runtime_identity().surface.as_ref().unwrap().name,
        "item-tools"
    );

    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let tools = client.list_tools(Default::default()).await?;
    assert_eq!(
        tools
            .tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        vec!["framework-help", "items"]
    );
    let resources = client.list_resources(Default::default()).await?;
    assert!(resources.resources.iter().all(|resource| !matches!(
        resource.uri.as_str(),
        "cli://lanes" | "cli://server/overview"
    )));
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new("cli://server/overview"))
            .await
            .is_err()
    );
    let prompt = client
        .get_prompt(GetPromptRequestParams::new("getting_started"))
        .await?;
    let prompt = serde_json::to_string(&prompt)?;
    assert!(prompt.contains("named MCP tools"));
    assert!(!prompt.contains("command string"));
    assert!(!prompt.contains("Start execution"));
    let result = client
        .call_tool(
            CallToolRequestParams::new("items").with_arguments(serde_json::from_value(json!({
                "operation": "get",
                "id": "42"
            }))?),
        )
        .await?;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.structured_content.as_ref().unwrap()["operation"],
        "get"
    );
    assert_eq!(result.structured_content.as_ref().unwrap()["id"], "42");
    assert_eq!(
        result.content[0].as_text().unwrap().text,
        r#"{"id":"42","operation":"get","value":"found"}"#
    );
    let catalog = client
        .read_resource(ReadResourceRequestParams::new("cli://catalog"))
        .await?;
    let ResourceContents::TextResourceContents { text, .. } = &catalog.contents[0] else {
        anyhow::bail!("expected text catalog resource");
    };
    let catalog: Value = serde_json::from_str(text)?;
    assert_eq!(catalog["activeSurface"]["name"], "item-tools");
    assert_eq!(catalog["activeSurface"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        catalog["activeSurface"]["surfaceHash"],
        expected_surface_hash
    );
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn native_results_preserve_large_schema_valid_payloads() -> anyhow::Result<()> {
    let payload = Arc::new("x".repeat(40 * 1024));
    let registry = CommandRegistry::new("large", "Large native result").register_dynamic(
        CommandSpec::new(["large"], "Large", "Return a large result").with_output(OutputContract {
            application: Some(object_contract(
                json!({ "payload": { "type": "string" } }),
                &["payload"],
            )),
            ..OutputContract::default()
        }),
        {
            let payload = payload.clone();
            move |_| {
                let payload = payload.clone();
                async move {
                    Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                        "payload": payload.as_str()
                    })))
                }
            }
        },
    );
    let surface = NativeToolSurface::builder("large-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("large", "large")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let result = call_native_tool(
        CliMcpServer::with_surface(registry, surface)?,
        "large",
        json!({}),
    )
    .await?;
    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured native result");
    assert_eq!(structured["payload"].as_str().unwrap().len(), 40 * 1024);
    assert!(structured.get("truncated").is_none());
    Ok(())
}

#[tokio::test]
async fn native_planning_errors_suppress_effect_lane_recovery_names() -> anyhow::Result<()> {
    let registry = native_capability_registry();
    let surface = NativeToolSurface::builder("capability-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .direct("validate_build", "build.validate")
        .direct("publish_build", "deploy.publish")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let result = call_native_tool(
        CliMcpServer::with_surface(registry, surface)?,
        "publish_build",
        json!({}),
    )
    .await?;
    assert_eq!(result.is_error, Some(true));
    let text = &result.content[0].as_text().unwrap().text;
    let error: Value = serde_json::from_str(text)?;
    assert_eq!(error["code"], "capability_missing");
    assert_eq!(error["details"]["providers"], Value::Null);
    assert!(!text.contains("build validate"));
    Ok(())
}

#[tokio::test]
async fn protocol_observations_reject_before_native_tool_routing() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
    let server = CliMcpServer::with_surface(registry, surface)?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = LegacyProtocolClient.serve(client_transport).await?;
    let mut request = CallToolRequestParams::new("not-a-tool");
    request.meta = Some(Meta(serde_json::from_value(json!({
        "io.modelcontextprotocol/protocolVersion": "2025-11-25"
    }))?));
    let error = client.call_tool(request).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Conflicting MCP protocol version")
    );
    assert!(!error.to_string().contains("Unknown tool"));
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn native_calls_preserve_progress_through_confirmation_and_dispatch() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Bridge)?;
    let server = CliMcpServer::builder(registry)
        .surface(surface)
        .authorizer(Arc::new(AlwaysConfirm))
        .native_confirmation_bridge(Arc::new(CountingBridge {
            calls: Arc::new(AtomicUsize::new(0)),
            decision: NativeConfirmationDecision::Allow,
        }))
        .build()?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client_handler = ProgressClient::new();
    let dispatcher = client_handler.progress.clone();
    let client = client_handler.serve(client_transport).await?;
    let params = CallToolRequestParams::new("items").with_arguments(serde_json::from_value(
        json!({ "operation": "get", "id": "42" }),
    )?);
    let handle = client
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(params)),
            PeerRequestOptions::no_options(),
        )
        .await?;
    let mut progress = dispatcher.subscribe(handle.progress_token.clone()).await;
    let result = handle.await_response().await?;
    let mut seen = Vec::new();
    while let Ok(Some(notification)) =
        tokio::time::timeout(std::time::Duration::from_millis(100), progress.next()).await
    {
        seen.push(notification.message.unwrap_or_default());
        if seen.len() >= 5 {
            break;
        }
    }
    assert_eq!(serde_json::to_value(result)?["isError"], false);
    for expected in [
        "Planning native invocation",
        "Invocation plan ready",
        "Confirmation required",
        "Dispatching command handler",
        "Command complete",
    ] {
        assert!(seen.iter().any(|message| message == expected), "{seen:?}");
    }
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn grouped_selector_failures_are_invalid_input() -> anyhow::Result<()> {
    for (arguments, expected_code) in [
        (json!({ "id": "42" }), "missing_argument"),
        (
            json!({ "operation": 1, "id": "42" }),
            "invalid_argument_type",
        ),
        (
            json!({ "operation": "missing", "id": "42" }),
            "invalid_argument_type",
        ),
    ] {
        let registry = item_registry(TaskSupportSpec::Optional);
        let surface = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
        let result = call_native_tool(
            CliMcpServer::with_surface(registry, surface)?,
            "items",
            arguments,
        )
        .await?;
        assert_eq!(result.is_error, Some(true));
        let body: Value = serde_json::from_str(&result.content[0].as_text().unwrap().text)?;
        assert_eq!(body["code"], expected_code);
        assert_ne!(body["code"], "build_failed");
    }

    let registry = item_registry(TaskSupportSpec::Optional);
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
    let result = call_native_tool(
        CliMcpServer::with_surface(registry, surface)?,
        "items",
        json!({ "operation": "get" }),
    )
    .await?;
    let body: Value = serde_json::from_str(&result.content[0].as_text().unwrap().text)?;
    assert_eq!(body["code"], "missing_argument");
    assert_eq!(body["details"]["operation"], "items.get");
    Ok(())
}

#[tokio::test]
async fn native_surfaces_fail_closed_for_every_task_request() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
    let server = CliMcpServer::with_surface(registry, surface)?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;

    let requests = [
        ClientRequest::ListTasksRequest(Default::default()),
        ClientRequest::GetTaskInfoRequest(Request::new(GetTaskInfoParams {
            meta: None,
            task_id: "private-task".to_string(),
        })),
        ClientRequest::GetTaskResultRequest(Request::new(GetTaskResultParams {
            meta: None,
            task_id: "private-task".to_string(),
        })),
        ClientRequest::CancelTaskRequest(Request::new(CancelTaskParams {
            meta: None,
            task_id: "private-task".to_string(),
        })),
    ];
    for request in requests {
        let error = client.send_request(request).await.unwrap_err();
        assert!(error.to_string().contains("Method not found"));
    }

    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn effect_lanes_enforce_forbidden_and_required_task_delivery() -> anyhow::Result<()> {
    let forbidden_calls = Arc::new(AtomicUsize::new(0));
    let forbidden = CliMcpServer::new(item_registry_with_calls(
        TaskSupportSpec::Forbidden,
        forbidden_calls.clone(),
    ))?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        forbidden.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let request = RunRequest {
        command: "items get --id $args.id".to_string(),
        args: BTreeMap::from([("id".to_string(), json!("42"))]),
        stdin: None,
        output: None,
        mode: RunMode::Execute,
        approval: None,
        dry_run: false,
    };
    let params = CallToolRequestParams::new("run")
        .with_arguments(serde_json::from_value(serde_json::to_value(&request)?)?)
        .with_task(serde_json::Map::new());
    let error = client
        .send_request(ClientRequest::CallToolRequest(Request::new(params)))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support task-based invocation")
            || error
                .to_string()
                .contains("does not support task-augmented execution"),
        "{error:?}"
    );
    assert_eq!(forbidden_calls.load(Ordering::SeqCst), 0);
    client.cancel().await?;
    server_handle.await??;

    let required_calls = Arc::new(AtomicUsize::new(0));
    let required = CliMcpServer::new(item_registry_with_calls(
        TaskSupportSpec::Required,
        required_calls.clone(),
    ))?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        required.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let error = client
        .call_tool(
            CallToolRequestParams::new("run")
                .with_arguments(serde_json::from_value(serde_json::to_value(request)?)?),
        )
        .await
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("ErrorCode(-32601)"),
        "{error:?}"
    );
    assert_eq!(required_calls.load(Ordering::SeqCst), 0);
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn native_required_legacy_tools_reject_ordinary_calls() -> anyhow::Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = item_registry_with_calls(TaskSupportSpec::Required, calls.clone());
    let surface = NativeToolSurface::builder("task-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .exposure(NativeExposurePolicy::explicit_subset(["items.list"]))
        .task_delivery(TaskDeliveryDecl::Legacy2025_11_25)
        .direct("work", "items.get")
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let server = CliMcpServer::with_surface(registry, surface)?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let error = client
        .send_request(ClientRequest::CallToolRequest(Request::new(
            CallToolRequestParams::new("work")
                .with_arguments(serde_json::from_value(json!({ "id": "42" }))?),
        )))
        .await
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("ErrorCode(-32601)"),
        "{error:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[derive(Clone, Copy)]
struct TestClient;

impl ClientHandler for TestClient {}

#[derive(Clone, Copy)]
struct LegacyProtocolClient;

impl ClientHandler for LegacyProtocolClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default().with_protocol_version(ProtocolVersion::V_2025_06_18)
    }
}

struct ProgressClient {
    progress: ProgressDispatcher,
}

impl ProgressClient {
    fn new() -> Self {
        Self {
            progress: ProgressDispatcher::new(),
        }
    }
}

impl ClientHandler for ProgressClient {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.progress.handle_notification(params).await;
    }
}

async fn call_native_tool(
    server: CliMcpServer,
    tool: &str,
    arguments: Value,
) -> anyhow::Result<rmcp::model::CallToolResult> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let result = client
        .call_tool(
            CallToolRequestParams::new(tool.to_string())
                .with_arguments(serde_json::from_value(arguments)?),
        )
        .await?;
    client.cancel().await?;
    server_handle.await??;
    Ok(result)
}

#[derive(Clone)]
struct AlwaysConfirm;

impl PermissionAuthorizer for AlwaysConfirm {
    fn decide(&self, _plan: &mcp_twill::InvocationPlan) -> PermissionDecision {
        PermissionDecision::RequireConfirmation
    }
}

struct CountingBridge {
    calls: Arc<AtomicUsize>,
    decision: NativeConfirmationDecision,
}

struct FailingBridge;

#[async_trait]
impl NativeConfirmationBridge for FailingBridge {
    async fn confirm(
        &self,
        _request: NativeConfirmationRequest,
    ) -> std::result::Result<NativeConfirmationDecision, NativeConfirmationBridgeError> {
        Err(NativeConfirmationBridgeError::new(std::io::Error::other(
            "private bridge secret",
        )))
    }
}

#[async_trait]
impl NativeConfirmationBridge for CountingBridge {
    async fn confirm(
        &self,
        request: NativeConfirmationRequest,
    ) -> std::result::Result<NativeConfirmationDecision, NativeConfirmationBridgeError> {
        assert_eq!(request.preview().operation_id, "items.get");
        assert_eq!(request.arguments()["id"], "42");
        assert_eq!(request.presentation().operation_id, "items.get");
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.decision)
    }
}

#[tokio::test]
async fn native_confirmation_bridge_is_single_shot_and_private() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Bridge)?;
    let calls = Arc::new(AtomicUsize::new(0));
    let server = CliMcpServer::builder(registry)
        .surface(surface)
        .authorizer(Arc::new(AlwaysConfirm))
        .native_confirmation_bridge(Arc::new(CountingBridge {
            calls: calls.clone(),
            decision: NativeConfirmationDecision::Allow,
        }))
        .build()?;
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });
    let client = TestClient.serve(client_transport).await?;
    let result = client
        .call_tool(
            CallToolRequestParams::new("items").with_arguments(serde_json::from_value(json!({
                "operation": "get",
                "id": "42"
            }))?),
        )
        .await?;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[tokio::test]
async fn confirmation_non_allow_outcomes_fail_closed_without_dispatch() -> anyhow::Result<()> {
    for (decision, expected_code) in [
        (NativeConfirmationDecision::Deny, "permission_denied"),
        (
            NativeConfirmationDecision::Canceled,
            "confirmation_canceled",
        ),
    ] {
        let handler_calls = Arc::new(AtomicUsize::new(0));
        let registry = item_registry_with_calls(TaskSupportSpec::Optional, handler_calls.clone());
        let surface = grouped_surface(&registry, NativeConfirmationRoute::Bridge)?;
        let server = CliMcpServer::builder(registry)
            .surface(surface)
            .authorizer(Arc::new(AlwaysConfirm))
            .native_confirmation_bridge(Arc::new(CountingBridge {
                calls: Arc::new(AtomicUsize::new(0)),
                decision,
            }))
            .build()?;
        let result =
            call_native_tool(server, "items", json!({ "operation": "get", "id": "42" })).await?;
        let body: Value = serde_json::from_str(&result.content[0].as_text().unwrap().text)?;
        assert_eq!(result.is_error, Some(true));
        assert_eq!(body["code"], expected_code);
        assert_eq!(handler_calls.load(Ordering::SeqCst), 0);
    }

    let handler_calls = Arc::new(AtomicUsize::new(0));
    let registry = item_registry_with_calls(TaskSupportSpec::Optional, handler_calls.clone());
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Bridge)?;
    let server = CliMcpServer::builder(registry)
        .surface(surface)
        .authorizer(Arc::new(AlwaysConfirm))
        .native_confirmation_bridge(Arc::new(FailingBridge))
        .build()?;
    let result =
        call_native_tool(server, "items", json!({ "operation": "get", "id": "42" })).await?;
    let text = &result.content[0].as_text().unwrap().text;
    let body: Value = serde_json::from_str(text)?;
    assert_eq!(body["code"], "confirmation_failed");
    assert!(!text.contains("private bridge secret"));
    assert_eq!(handler_calls.load(Ordering::SeqCst), 0);

    let handler_calls = Arc::new(AtomicUsize::new(0));
    let registry = item_registry_with_calls(TaskSupportSpec::Optional, handler_calls.clone());
    let surface = grouped_surface(&registry, NativeConfirmationRoute::Unavailable)?;
    let server = CliMcpServer::builder(registry)
        .surface(surface)
        .authorizer(Arc::new(AlwaysConfirm))
        .build()?;
    let result =
        call_native_tool(server, "items", json!({ "operation": "get", "id": "42" })).await?;
    let body: Value = serde_json::from_str(&result.content[0].as_text().expect("text error").text)?;
    assert_eq!(body["code"], "confirmation_unavailable");
    assert_eq!(handler_calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn mixed_task_support_fails_group_and_effect_lane_compilation() -> anyhow::Result<()> {
    let registry = item_registry(TaskSupportSpec::Optional);
    let mut operations = registry.operation_specs();
    assert!(
        operations
            .iter()
            .all(|operation| operation.task_support == TaskSupportSpec::Optional)
    );
    operations.clear();

    let list = CommandSpec::new(["mixed", "list"], "List", "List")
        .task_support(TaskSupportSpec::Forbidden)
        .with_output(OutputContract {
            application: Some(object_contract(json!({}), &[])),
            ..OutputContract::default()
        });
    let get = CommandSpec::new(["mixed", "get"], "Get", "Get")
        .task_support(TaskSupportSpec::Required)
        .with_output(OutputContract {
            application: Some(object_contract(json!({}), &[])),
            ..OutputContract::default()
        });
    let mixed = CommandRegistry::new("mixed", "Mixed")
        .register_dynamic(list, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
        })
        .register_dynamic(get, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
        });
    let error = NativeToolSurface::builder("mixed")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .group("mixed", |group| {
            group
                .selector("operation")
                .member("list", "mixed.list")
                .member("get", "mixed.get");
        })
        .build(&mixed, McpProtocolTarget::V2025_11_25)
        .unwrap_err();
    assert!(error.to_string().contains("mixed task support"));
    let effect_error = match CliMcpServer::with_config(
        mixed,
        CliMcpServerConfig::default().with_execution_tool_name("repo"),
    ) {
        Ok(_) => anyhow::bail!("expected effect-lane task-support mismatch"),
        Err(error) => error,
    };
    assert!(effect_error.to_string().contains("mixed task support"));
    assert!(effect_error.to_string().contains("effect lane `repo`"));
    Ok(())
}

#[test]
fn task_support_builder_and_low_level_paths_are_equivalent() -> anyhow::Result<()> {
    fn low_level(support: TaskSupportSpec) -> CommandRegistry {
        CommandRegistry::new("tasks", "Tasks").register_dynamic(
            CommandSpec::new(["work"], "Work", "Work")
                .task_support(support)
                .with_output(OutputContract {
                    application: Some(object_contract(json!({}), &[])),
                    ..OutputContract::default()
                }),
            |_| async { Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({}))) },
        )
    }
    fn built(support: TaskSupportSpec) -> mcp_twill::Result<CommandRegistry> {
        CommandRegistry::build("tasks", "Tasks", |server| {
            server.command("work", |command| {
                command
                    .summary("Work")
                    .description("Work")
                    .task_support(support)
                    .output(OutputContract {
                        application: Some(object_contract(json!({}), &[])),
                        ..OutputContract::default()
                    })
                    .handle_dynamic(|_| async {
                        Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
                    });
            });
        })
    }

    for support in [
        TaskSupportSpec::Forbidden,
        TaskSupportSpec::Optional,
        TaskSupportSpec::Required,
    ] {
        let low = low_level(support.clone());
        let high = built(support.clone())?;
        assert_eq!(low.catalog(), high.catalog());
        assert_eq!(low.catalog_identity(), high.catalog_identity());
        let low_surface = direct_shape_surface_for_operation(&low, "work")?;
        let high_surface = direct_shape_surface_for_operation(&high, "work")?;
        assert_eq!(
            low_surface.snapshot().canonical_json(),
            high_surface.snapshot().canonical_json()
        );
        let expected = match support {
            TaskSupportSpec::Forbidden => rmcp::model::TaskSupport::Forbidden,
            TaskSupportSpec::Optional => rmcp::model::TaskSupport::Optional,
            TaskSupportSpec::Required => rmcp::model::TaskSupport::Required,
        };
        assert_eq!(low_surface.snapshot().tools()[0].task_support(), expected);
    }

    let optional = low_level(TaskSupportSpec::Optional);
    let required = low_level(TaskSupportSpec::Required);
    assert_eq!(
        serde_json::to_value(optional.command_specs().next().unwrap())?.get("taskSupport"),
        None
    );
    let optional_operation = optional.operation_specs().pop().unwrap();
    let optional_wire = serde_json::to_value(&optional_operation)?;
    assert_eq!(optional_wire.get("taskSupport"), None);
    assert_eq!(
        serde_json::from_value::<mcp_twill::OperationSpec>(optional_wire)?.task_support,
        TaskSupportSpec::Optional
    );
    assert_eq!(
        serde_json::to_value(required.operation_specs().pop().unwrap())?["taskSupport"],
        "required"
    );
    assert_ne!(optional.catalog_identity(), required.catalog_identity());
    Ok(())
}

fn direct_shape_surface_for_operation(
    registry: &CommandRegistry,
    operation_id: &str,
) -> mcp_twill::Result<NativeToolSurface> {
    NativeToolSurface::builder("task-tools")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .task_delivery(TaskDeliveryDecl::Legacy2025_11_25)
        .direct("work", operation_id)
        .build(registry, McpProtocolTarget::V2025_11_25)
}

#[test]
fn vbl_v049_compiles_the_63_operation_27_tool_mapping() -> anyhow::Result<()> {
    let baseline = vbl_fixture("baseline-tools.json");
    let observed = vbl_fixture("surface-catalog.json");
    assert_eq!(vbl::ERROR_OWNERS.len(), 22);
    let (session_resource, ambient_binding) = vbl::ambient_session_adoption();
    assert_eq!(session_resource.carrier_name(), "agent_session_id");
    assert_eq!(ambient_binding.resource, "session");
    drop(vbl::registry());
    drop(vbl::argument_schema_registry(&baseline));
    let registry = vbl_native::registry(&baseline, &observed, vbl::PREAMBLE);
    let surface = vbl_native::surface(&registry, &observed)?;
    assert!(mcp_twill::check_native_surface_projection(&registry, &surface).is_empty());
    assert_eq!(registry.operation_specs().len(), 63);
    assert_eq!(surface.snapshot().tools().len(), 27);
    assert_eq!(surface.snapshot().operations().len(), 63);
    assert_eq!(surface.snapshot().server_instructions(), vbl::PREAMBLE);
    let tools = serde_json::to_value(surface.snapshot().tools())?;
    assert_eq!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].clone())
            .collect::<Vec<_>>(),
        observed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].clone())
            .collect::<Vec<_>>()
    );
    for (compiled, released) in tools
        .as_array()
        .unwrap()
        .iter()
        .zip(observed["tools"].as_array().unwrap())
    {
        assert_eq!(
            compiled["outputSchema"], released["outputSchema"],
            "output schema drift for {}",
            compiled["name"]
        );
        assert_eq!(
            compiled["annotations"], released["annotations"],
            "annotation drift for {}",
            compiled["name"]
        );
        assert_eq!(
            compiled["title"], released["title"],
            "title drift for {}",
            compiled["name"]
        );
    }
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["inputSchema"]["properties"].is_object())
    );
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["inputSchema"].get("oneOf").is_none())
    );
    assert_eq!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "screencast")
            .unwrap()["inputSchema"]["dependencies"],
        json!({
            "max_height": ["max_width"],
            "max_width": ["max_height"]
        })
    );
    let screencast_output = &tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "screencast")
        .unwrap()["outputSchema"];
    assert_eq!(screencast_output["oneOf"].as_array().unwrap().len(), 3);
    assert_eq!(
        screencast_output["oneOf"][1]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        screencast_output["oneOf"][2]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    Ok(())
}
