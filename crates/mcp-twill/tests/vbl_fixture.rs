//! Offline acceptance tests for the RFC 0015 VBL evidence bootstrap.

use std::collections::BTreeSet;

use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, ArgType, CommandRegistry, CommandSpec,
    DynamicCommandFailure, OutputContract,
};
use serde_json::{Value, json};

#[path = "support/vbl.rs"]
mod vbl;

mcp_twill::contract_tests!(vbl::registry);

fn surface_observation() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vbl/v0.4.9/",
        "surface-catalog.json"
    )))
    .expect("parse surface catalog")
}

fn baseline_observation() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vbl/v0.4.9/",
        "baseline-tools.json"
    )))
    .expect("parse baseline tools")
}

fn error_observation() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vbl/v0.4.9/",
        "application-error-vectors.json"
    )))
    .expect("parse application error vectors")
}

#[test]
fn released_error_inventory_has_one_authored_twill_owner() {
    let observation = error_observation();
    let released = observation["errorCodes"]
        .as_array()
        .expect("released error codes")
        .iter()
        .map(|code| code.as_str().expect("error code"))
        .collect::<BTreeSet<_>>();
    let authored = vbl::ERROR_OWNERS
        .iter()
        .map(|(code, _)| *code)
        .collect::<BTreeSet<_>>();
    assert_eq!(authored.len(), vbl::ERROR_OWNERS.len(), "duplicate owner");
    assert_eq!(authored, released, "every released code has one owner");
    assert_eq!(
        vbl::ERROR_OWNERS
            .iter()
            .find(|(code, _)| *code == "invalid_request_context")
            .map(|(_, owner)| *owner),
        Some(vbl::ErrorOwner::Framework("invalid_request_context"))
    );
    assert!(vbl::ERROR_OWNERS.iter().all(|(code, owner)| {
        *code == "invalid_request_context" || *owner == vbl::ErrorOwner::Application
    }));
}

#[test]
fn every_released_baseline_output_is_an_rfc_0014_source_contract() {
    let baseline = baseline_observation();
    for tool in baseline.as_array().expect("baseline tools") {
        let name = tool["name"].as_str().expect("tool name");
        let schema = tool["outputSchema"].clone();
        let registry = CommandRegistry::new("vbl-output", "VBL output contract").register_dynamic(
            CommandSpec::new([name], name, name).with_output(OutputContract {
                application: Some(ApplicationResultContract::new(schema)),
                ..OutputContract::default()
            }),
            |_context| async {
                Ok::<_, DynamicCommandFailure>(ApplicationSuccess::value(json!({})))
            },
        );
        registry
            .validate_results()
            .unwrap_or_else(|error| panic!("released output `{name}` must compile: {error}"));
        assert!(
            registry.catalog().operations[0]
                .output
                .application
                .is_some(),
            "released output `{name}` must project"
        );
    }
}

#[test]
fn authored_guidance_accounts_for_every_released_operation() {
    let catalog = baseline_observation();
    let released = catalog
        .as_array()
        .expect("released baseline operations")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<BTreeSet<_>>();
    let authored = vbl::OPERATION_MAPPING
        .iter()
        .map(|(released_name, _, _)| *released_name)
        .collect::<BTreeSet<_>>();
    assert_eq!(authored.len(), vbl::OPERATION_MAPPING.len());
    assert_eq!(authored, released);
}

#[test]
fn authored_preamble_preserves_the_released_instruction_contract() {
    let catalog = surface_observation();
    assert!(
        catalog["server_instructions"]
            .as_str()
            .expect("server instructions")
            .contains("preserve the user's active application")
    );
    assert_eq!(
        vbl::PREAMBLE,
        "Routine actions attach to the owned target and preserve the user's active application."
    );
    assert_eq!(vbl::registry().preamble(), Some(vbl::PREAMBLE));
}

#[test]
fn released_operation_titles_are_accounted_for_without_becoming_declarations() {
    let catalog = baseline_observation();
    let released = catalog
        .as_array()
        .expect("released baseline operations")
        .iter()
        .map(|tool| {
            (
                tool["name"].as_str().expect("tool name"),
                tool["title"].as_str().expect("tool title"),
            )
        })
        .collect::<Vec<_>>();
    let authored = vbl::OPERATION_MAPPING
        .iter()
        .map(|(name, _, title)| (*name, *title))
        .collect::<Vec<_>>();
    assert_eq!(authored, released);
}

#[test]
fn authored_guidance_projects_semantic_and_escape_hatch_steering() {
    let registry = vbl::registry();
    let catalog = registry.catalog();
    let click = catalog
        .operations
        .iter()
        .find(|operation| operation.id == "page.click")
        .expect("page click declaration");
    assert_eq!(
        click.args[0].value_type,
        ArgType::Named("element-target".to_string())
    );
    let fill = catalog
        .operations
        .iter()
        .find(|operation| operation.id == "form.fill")
        .expect("form fill declaration");
    assert_eq!(
        fill.use_when.as_deref(),
        Some("filling a single ordinary field")
    );
    assert_eq!(fill.alternatives[0].command, "form fill many");
    let fill_many = catalog
        .operations
        .iter()
        .find(|operation| operation.id == "form.fill.many")
        .expect("form fill many declaration");
    assert_eq!(
        fill_many.use_when.as_deref(),
        Some("updating two or more controls, including combined select and checkbox changes")
    );

    let help = registry
        .help(mcp_twill::HelpRequest {
            command: Some("page evaluate".to_string()),
            topic: None,
            detail: None,
        })
        .text;
    assert!(help.contains("Fallback: prefer `page snapshot`, `console list`, `network list`."));
}
