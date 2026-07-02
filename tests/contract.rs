use mcp_twill::{
    ArgSpec, CommandExample, CommandOutput, CommandRegistry, CommandSpec, PermissionEffect,
    PermissionSpec, contract,
};
use serde_json::json;

fn create_issue_spec() -> CommandSpec {
    let mut example = CommandExample::new(
        "issues create --title $args.title",
        "Create an issue with a typed title",
    );
    example.args.insert("title".to_string(), json!("Crash"));

    CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
        .with_arg(ArgSpec::string("title", "Issue title"))
        .with_permission(PermissionSpec::new(
            PermissionEffect::Write,
            "issues",
            "Creates issues",
        ))
        .with_example(example)
}

fn registry() -> CommandRegistry {
    CommandRegistry::new("contract-test", "Contract test server")
        .register(create_issue_spec(), |_context| async {
            Ok(CommandOutput::structured(json!({ "id": 1 })))
        })
        .register(
            CommandSpec::new(["issues", "list"], "List issues", "List issues").with_permission(
                PermissionSpec::new(PermissionEffect::Read, "issues", "Reads issues"),
            ),
            |_context| async { Ok(CommandOutput::structured(json!([]))) },
        )
}

// The macro expands to one #[test] per contract rule.
mcp_twill::contract_tests!(registry);

#[test]
fn aggregate_coverage_is_clean() {
    let violations = mcp_twill::verify_catalog_coverage(&registry(), "run");
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn missing_example_for_required_args_is_a_violation() {
    let reg = CommandRegistry::new("contract-test", "Contract test server").register(
        CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
            .with_arg(ArgSpec::string("title", "Issue title"))
            .with_permission(PermissionSpec::new(
                PermissionEffect::Write,
                "issues",
                "Creates issues",
            )),
        |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
    );
    let violations = contract::check_examples_and_plans(&reg);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].projection, "examples");
    assert_eq!(violations[0].operation.as_deref(), Some("issues.create"));
}

#[test]
fn broken_example_is_a_violation_naming_the_operation() {
    let reg = CommandRegistry::new("contract-test", "Contract test server").register(
        CommandSpec::new(["issues", "create"], "Create issue", "Create issue")
            .with_arg(ArgSpec::string("title", "Issue title"))
            .with_permission(PermissionSpec::new(
                PermissionEffect::Write,
                "issues",
                "Creates issues",
            ))
            .with_example(CommandExample::new(
                "issues create --title $args.missing",
                "Example referencing an undeclared argument",
            )),
        |_context| async { Ok(CommandOutput::structured(json!({ "id": 1 }))) },
    );
    let violations = contract::check_examples_and_plans(&reg);
    assert!(
        violations
            .iter()
            .any(|violation| violation.projection == "examples"
                && violation.operation.as_deref() == Some("issues.create")),
        "{violations:?}"
    );
}

#[test]
fn nonpure_operation_without_permissions_is_a_violation() {
    // CommandSpec::new with no permissions is Pure, which is fine. Simulate
    // drift by declaring a permission with an empty description instead.
    let reg = CommandRegistry::new("contract-test", "Contract test server").register(
        CommandSpec::new(["issues", "purge"], "Purge issues", "Purge issues").with_permission(
            PermissionSpec::new(PermissionEffect::Delete, "issues", ""),
        ),
        |_context| async { Ok(CommandOutput::structured(json!({}))) },
    );
    let violations = contract::check_effect_metadata(&reg);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].projection, "permissions");
}

#[test]
fn violations_render_with_operation_and_projection() {
    let violation = contract::ContractViolation {
        operation: Some("issues.create".to_string()),
        projection: "examples",
        message: "example fails planning".to_string(),
    };
    assert_eq!(
        violation.to_string(),
        "[examples] issues.create: example fails planning"
    );
}
