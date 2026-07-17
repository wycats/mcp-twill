//! Minimal named-tool server for RFC 0015.
//!
//! The command catalog remains authoritative. The native declaration only
//! chooses the public tool name, selector, and operation membership.

use std::collections::BTreeMap;

use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, ArgSpec, CliMcpServer, CommandContext,
    CommandExample, CommandRegistry, CommandSpec, DynamicCommandFailure, FrameworkHelpProjection,
    McpProtocolTarget, NativeConfirmationRoute, NativeToolSurface, OutputContract, Result,
};
use rmcp::{ServiceExt, transport::stdio};
use serde_json::json;

fn registry() -> CommandRegistry {
    let list = CommandSpec::new(["items", "list"], "List items", "List the stored items")
        .with_output(OutputContract {
            application: Some(ApplicationResultContract::new(json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["items"],
                "additionalProperties": false
            }))),
            ..OutputContract::default()
        });
    let get = CommandSpec::new(["items", "get"], "Get item", "Read one stored item")
        .with_arg(ArgSpec::string("id", "The item id"))
        .with_example(CommandExample {
            command: "items get --id $args.id".to_string(),
            summary: "Read item one".to_string(),
            args: BTreeMap::from([("id".to_string(), json!("one"))]),
        })
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

    CommandRegistry::new("native-surface-example", "Named native MCP tools")
        .declare_preamble("Use the named item tool directly.")
        .register_dynamic(list, |_| async {
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "items": ["one", "two"]
            })))
        })
        .register_dynamic(get, |context: CommandContext| async move {
            let id = context
                .plan
                .bound_args
                .get("id")
                .map(|argument| argument.value.clone())
                .unwrap_or_else(|| json!("missing"));
            Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({
                "id": id,
                "value": "found"
            })))
        })
}

fn native_surface(registry: &CommandRegistry) -> Result<NativeToolSurface> {
    NativeToolSurface::builder("native-items")
        .framework_help(FrameworkHelpProjection::Tool {
            name: "framework-help".to_string(),
        })
        .confirmation_route(NativeConfirmationRoute::Unavailable)
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

#[tokio::main]
async fn main() -> Result<()> {
    let registry = registry();
    let surface = native_surface(&registry)?;
    CliMcpServer::with_surface(registry, surface)?
        .serve(stdio())
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?
        .waiting()
        .await
        .map_err(|error| mcp_twill::FrameworkError::Handler(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod contract_coverage {
    fn contract_registry() -> mcp_twill::CommandRegistry {
        super::registry()
    }

    mcp_twill::contract_tests!(contract_registry);

    #[test]
    fn native_surface_projection() {
        let registry = contract_registry();
        let surface = super::native_surface(&registry).expect("native surface compiles");
        mcp_twill::contract::assert_no_violations(mcp_twill::check_native_surface_projection(
            &registry, &surface,
        ));
    }
}
