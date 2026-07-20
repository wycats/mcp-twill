//! Offline acceptance for RFC 0020's external protocol evidence bootstrap.

use std::collections::BTreeSet;

use serde_json::Value;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mcp/tasks/");

fn fixture(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(format!("{ROOT}{name}")).expect("read fixture"))
        .expect("parse fixture JSON")
}

#[test]
fn manifest_pins_each_external_authority_without_claiming_final_release() {
    let manifest = fixture("manifest.json");
    assert_eq!(manifest["formatVersion"], 1);
    assert_eq!(manifest["protocolRevision"], "2026-07-28");
    assert_eq!(manifest["extensionId"], "io.modelcontextprotocol/tasks");
    assert!(manifest.get("finalRelease").is_none());

    let sources = manifest["sources"]
        .as_array()
        .expect("source identities")
        .iter()
        .map(|source| {
            (
                source["id"].as_str().expect("source id"),
                source["commit"].as_str().expect("source commit"),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        sources,
        BTreeSet::from([
            (
                "core-2026-07-28-rc",
                "9d700ed62dcf86cb77475c9b81930611a9182f46"
            ),
            (
                "legacy-2025-11-25",
                "38c84e9f93ad191d9eb26d92b945d17bd0efcaf3"
            ),
            (
                "tasks-extension",
                "8966bea9c4f4e6d71060cc8284a539086e9e234f"
            ),
        ])
    );
}

#[test]
fn reviewed_vectors_keep_the_task_dialects_distinct() {
    let legacy = fixture("legacy-wire-vectors.json");
    let extension = fixture("extension-wire-vectors.json");
    let legacy_cases = legacy["cases"].as_array().expect("legacy cases");
    let extension_cases = extension["cases"].as_array().expect("extension cases");

    assert!(legacy_cases.iter().any(|case| {
        case["request"]["method"] == "tasks/result"
            && case["response"]["result"]["_meta"]
                .get("io.modelcontextprotocol/related-task")
                .is_some()
    }));
    assert!(extension_cases.iter().any(|case| {
        case["request"]["method"] == "tasks/get"
            && case["response"]["result"]["status"] == "completed"
            && case["response"]["result"]["result"]["isError"] == true
    }));
    assert!(extension_cases.iter().any(|case| {
        case["request"]["method"] == "tasks/cancel"
            && case["response"]["result"]["resultType"] == "complete"
    }));
}

#[test]
fn core_vectors_freeze_the_locked_rc_transport_failures() {
    let core = fixture("core-wire-vectors.json");
    let cases = core["cases"].as_array().expect("core cases");
    let error = |name: &str| {
        cases
            .iter()
            .find(|case| case["name"] == name)
            .unwrap_or_else(|| panic!("missing core case {name}"))
    };

    assert_eq!(error("header-mismatch")["httpStatus"], 400);
    assert_eq!(
        error("header-mismatch")["response"]["error"]["code"],
        -32001
    );
    assert_eq!(error("unsupported-protocol-version")["httpStatus"], 400);
    assert_eq!(
        error("unsupported-protocol-version")["response"]["error"]["code"],
        -32004
    );
    assert_eq!(error("unsupported-method")["httpStatus"], 404);
    assert_eq!(
        error("unsupported-method")["response"]["error"]["code"],
        -32601
    );
}
