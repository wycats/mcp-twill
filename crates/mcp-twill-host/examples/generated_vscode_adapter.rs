use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, ArgSpec, CommandContext, CommandRegistry,
    CommandSpec, DynamicCommandFailure, FrameworkHelpProjection, McpProtocolTarget,
    NativeConfirmationRoute, NativeToolDecl, NativeToolSurface, OutputContract,
};
use mcp_twill_host::{
    HostAdapterProfile, HostConfirmationPolicy, HostConfirmationTrigger, HostContextReason,
    HostInvocationLimits, HostProcessLimits, UnsupportedContextPolicy, VsCodeVersion,
    generate_vscode_artifacts,
};
use serde_json::json;

fn registry() -> CommandRegistry {
    let spec = CommandSpec::new(
        ["items", "get"],
        "Get Item",
        "Read one item from the catalog.",
    )
    .with_arg(ArgSpec::string("id", "Item id"))
    .with_output(OutputContract {
        application: Some(ApplicationResultContract::new(json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "value": { "type": "string" }
            },
            "required": ["id", "value"],
            "additionalProperties": false
        }))),
        ..OutputContract::default()
    });
    CommandRegistry::new("generated-host-example", "Generated host example").register_dynamic(
        spec,
        |context: CommandContext| async move {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "id": context.plan.bound_args["id"].value,
                "value": "found"
            })))
        },
    )
}

fn main() -> anyhow::Result<()> {
    let transport = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "in-process".to_string());
    let registry = registry();
    let surface = NativeToolSurface::builder("generated-host-example")
        .framework_help(FrameworkHelpProjection::Omitted)
        .confirmation_route(NativeConfirmationRoute::Unavailable)
        .tool(NativeToolDecl::Direct {
            name: "item_get".to_string(),
            operation_id: "items.get".to_string(),
            title: Some("Get Item".to_string()),
            description: Some("Read one item from the catalog.".to_string()),
        })
        .build(&registry, McpProtocolTarget::V2025_11_25)?;
    let unsupported = [
        HostContextReason::UnknownTokenShape,
        HostContextReason::InvalidSessionResource,
        HostContextReason::InvalidWorkingDirectory,
        HostContextReason::ProviderFailed,
    ]
    .into_iter()
    .fold(UnsupportedContextPolicy::new(), |policy, reason| {
        policy.reason(reason, "This host cannot supply invocation context")
    });
    let profile =
        HostAdapterProfile::vscode("generated-host-example", VsCodeVersion::new(1, 120, 0))
            .confirmation(HostConfirmationPolicy::presentation_only(
                HostConfirmationTrigger::None,
            ))
            .unsupported_context(unsupported)
            .invocation_limits(HostInvocationLimits::new(64 * 1024, 64 * 1024));
    let profile = match transport.as_str() {
        "in-process" => profile.in_process(),
        "process" => profile.process_envelope(
            "bin/generated-host-example",
            ["host", "call"],
            HostProcessLimits::new(64 * 1024, 2_000),
        ),
        _ => anyhow::bail!("expected `in-process` or `process`"),
    }
    .build(surface.snapshot())?;
    print!(
        "{}",
        generate_vscode_artifacts(profile.snapshot())?.adapter_typescript()
    );
    Ok(())
}
