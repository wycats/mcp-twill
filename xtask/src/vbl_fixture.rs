use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::BufReader,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

pub const VBL_TAG: &str = "v0.4.9";
pub const VBL_COMMIT: &str = "f2bd478fa5506df7530b3fd60d7d0114f0ed3160";
pub const VBL_REPOSITORY: &str = "https://github.com/wycats/visible-browser-lab";
pub const IMPORT_COMMAND: &str =
    "cargo xtask import-vbl-fixture --repository <local-git-repository> --ref v0.4.9";

const PAYLOADS: [&str; 5] = [
    "application-error-vectors.json",
    "baseline-tools.json",
    "presentation-vectors.json",
    "surface-catalog.json",
    "vscode-package.json",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format_version: u32,
    pub source: SourceIdentity,
    pub importer: ImporterIdentity,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub repository: String,
    pub tag: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporterIdentity {
    pub version: u32,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub derivation: Derivation,
    pub sources: Vec<SourceEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Derivation {
    RustExport,
    SourceCopy,
    ReviewedVector,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceEntry {
    pub path: String,
    pub sha256: String,
}

struct GeneratedPayload {
    path: &'static str,
    bytes: Vec<u8>,
    derivation: Derivation,
    sources: &'static [&'static str],
}

pub fn import(repository: &Path, reference: &str, check: bool) -> Result<()> {
    ensure!(
        reference == VBL_TAG,
        "the frozen fixture accepts only --ref {VBL_TAG}"
    );
    ensure!(
        repository.is_dir(),
        "repository does not exist: {}",
        repository.display()
    );

    let resolved = git_output(
        repository,
        &["rev-parse", &format!("{reference}^{{commit}}")],
    )?;
    ensure!(
        resolved.trim() == VBL_COMMIT,
        "{reference} peels to {}, expected {VBL_COMMIT}",
        resolved.trim()
    );

    let source_archive = TempDir::new().context("create pristine source archive")?;
    export_archive(repository, VBL_COMMIT, source_archive.path())?;
    let build_archive = TempDir::new().context("create helper build archive")?;
    export_archive(repository, VBL_COMMIT, build_archive.path())?;
    let generated = TempDir::new().context("create generated fixture workspace")?;
    generate_bundle(
        source_archive.path(),
        build_archive.path(),
        generated.path(),
    )?;

    let destination = fixture_directory();
    if check {
        validate_bundle(&destination)?;
        compare_directories(generated.path(), &destination)?;
        println!(
            "VBL fixture matches {VBL_TAG} ({VBL_COMMIT}) at {}",
            destination.display()
        );
        return Ok(());
    }

    validate_bundle(generated.path())?;
    replace_directory(generated.path(), &destination)?;
    println!(
        "imported VBL fixture {VBL_TAG} ({VBL_COMMIT}) into {}",
        destination.display()
    );
    Ok(())
}

pub fn fixture_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask belongs to the workspace")
        .join("crates/mcp-twill/tests/fixtures/vbl/v0.4.9")
}

fn export_archive(repository: &Path, commit: &str, destination: &Path) -> Result<()> {
    let tar_file = tempfile::NamedTempFile::new().context("create Git archive file")?;
    command(
        Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["archive", "--format=tar", "--output"])
            .arg(tar_file.path())
            .arg(commit),
        "export the pinned VBL archive",
    )?;
    let file = fs::File::open(tar_file.path()).context("open Git archive")?;
    let mut archive = tar::Archive::new(BufReader::new(file));
    archive.set_preserve_permissions(false);
    archive
        .unpack(destination)
        .context("unpack the pinned VBL archive")?;
    Ok(())
}

fn generate_bundle(source_archive: &Path, build_archive: &Path, destination: &Path) -> Result<()> {
    let target_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask belongs to the workspace")
        .join("target/vbl-fixture-import");
    fs::create_dir_all(&target_dir).context("create fixture importer target directory")?;

    let baseline_helper = build_archive.join("agent-surface-contract/examples/export_baseline.rs");
    fs::create_dir_all(baseline_helper.parent().expect("helper has a parent"))?;
    fs::write(&baseline_helper, BASELINE_HELPER).context("write archived baseline helper")?;

    let error_helper = build_archive.join("examples/export_errors.rs");
    fs::create_dir_all(error_helper.parent().expect("helper has a parent"))?;
    fs::write(&error_helper, ERROR_HELPER).context("write archived error helper")?;

    let baseline = cargo_output(
        build_archive,
        &target_dir,
        &[
            "run",
            "--locked",
            "--quiet",
            "-p",
            "agent-surface-contract",
            "--example",
            "export_baseline",
        ],
    )?;
    let errors = cargo_output(
        build_archive,
        &target_dir,
        &["run", "--locked", "--quiet", "--example", "export_errors"],
    )?;
    let surface = cargo_output(
        build_archive,
        &target_dir,
        &[
            "run",
            "--locked",
            "--quiet",
            "--bin",
            "visible-browser-lab-mcp",
            "--",
            "surface",
            "catalog",
        ],
    )?;

    let payloads = [
        GeneratedPayload {
            path: "application-error-vectors.json",
            bytes: canonical_json(&errors)?,
            derivation: Derivation::RustExport,
            sources: &["src/leases.rs"],
        },
        GeneratedPayload {
            path: "baseline-tools.json",
            bytes: canonical_json(&baseline)?,
            derivation: Derivation::RustExport,
            sources: &["agent-surface-contract/src/lib.rs"],
        },
        GeneratedPayload {
            path: "presentation-vectors.json",
            bytes: canonical_value(presentation_vectors())?,
            derivation: Derivation::ReviewedVector,
            sources: &[
                "vscode-extension/src/confirmation.ts",
                "vscode-extension/src/extension.ts",
            ],
        },
        GeneratedPayload {
            path: "surface-catalog.json",
            bytes: canonical_json(&surface)?,
            derivation: Derivation::RustExport,
            sources: &[
                "agent-surface-contract/src/lib.rs",
                "src/surface.rs",
                "src/surface_cli.rs",
            ],
        },
        GeneratedPayload {
            path: "vscode-package.json",
            bytes: fs::read(source_archive.join("vscode-extension/package.json"))
                .context("read archived VS Code package manifest")?,
            derivation: Derivation::SourceCopy,
            sources: &["vscode-extension/package.json"],
        },
    ];

    let mut files = Vec::new();
    for payload in payloads {
        let output = destination.join(payload.path);
        fs::write(&output, &payload.bytes)
            .with_context(|| format!("write generated payload {}", payload.path))?;
        let mut sources = payload
            .sources
            .iter()
            .map(|path| {
                let bytes = fs::read(source_archive.join(path))
                    .with_context(|| format!("read archived source {path}"))?;
                Ok(SourceEntry {
                    path: (*path).to_string(),
                    sha256: sha256(&bytes),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        sources.sort();
        files.push(FileEntry {
            path: payload.path.to_string(),
            sha256: sha256(&payload.bytes),
            derivation: payload.derivation,
            sources,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let manifest = Manifest {
        format_version: 1,
        source: SourceIdentity {
            repository: VBL_REPOSITORY.to_string(),
            tag: VBL_TAG.to_string(),
            commit: VBL_COMMIT.to_string(),
        },
        importer: ImporterIdentity {
            version: 1,
            command: IMPORT_COMMAND.to_string(),
        },
        files,
    };
    fs::write(
        destination.join("manifest.json"),
        canonical_value(serde_json::to_value(manifest)?)?,
    )
    .context("write fixture manifest last")?;
    Ok(())
}

pub fn validate_bundle(directory: &Path) -> Result<()> {
    ensure!(
        directory.is_dir(),
        "fixture directory is missing: {}",
        directory.display()
    );
    let manifest_bytes =
        fs::read(directory.join("manifest.json")).context("read fixture manifest")?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("parse fixture manifest")?;
    ensure!(
        manifest_bytes == canonical_value(serde_json::to_value(&manifest)?)?,
        "manifest.json is not canonical JSON"
    );
    ensure!(
        manifest.format_version == 1,
        "unsupported fixture format version"
    );
    ensure!(
        manifest.source
            == (SourceIdentity {
                repository: VBL_REPOSITORY.to_string(),
                tag: VBL_TAG.to_string(),
                commit: VBL_COMMIT.to_string(),
            }),
        "fixture source identity is not the pinned VBL release"
    );
    ensure!(
        manifest.importer
            == (ImporterIdentity {
                version: 1,
                command: IMPORT_COMMAND.to_string(),
            }),
        "fixture importer identity is not canonical"
    );

    let paths = manifest
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    ensure!(
        paths == PAYLOADS,
        "manifest payload inventory is incomplete or not sorted"
    );

    let actual = directory_inventory(directory)?;
    let expected = PAYLOADS
        .iter()
        .copied()
        .chain(["manifest.json"])
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    ensure!(
        actual == expected,
        "fixture directory contains extra or missing files"
    );

    for entry in &manifest.files {
        validate_relative_path(&entry.path)?;
        validate_hash(&entry.sha256, &entry.path)?;
        ensure!(
            !entry.sources.is_empty(),
            "{} has no source provenance",
            entry.path
        );
        let sorted_sources = entry
            .sources
            .iter()
            .map(|source| source.path.as_str())
            .collect::<Vec<_>>();
        let mut expected_sources = sorted_sources.clone();
        expected_sources.sort_unstable();
        ensure!(
            sorted_sources == expected_sources,
            "{} source paths are not sorted",
            entry.path
        );
        ensure!(
            sorted_sources.windows(2).all(|pair| pair[0] != pair[1]),
            "{} contains duplicate source paths",
            entry.path
        );
        for source in &entry.sources {
            validate_relative_path(&source.path)?;
            validate_hash(&source.sha256, &source.path)?;
        }

        let bytes = fs::read(directory.join(&entry.path))
            .with_context(|| format!("read fixture payload {}", entry.path))?;
        ensure!(
            sha256(&bytes) == entry.sha256,
            "{} hash does not match its payload",
            entry.path
        );
        if entry.derivation != Derivation::SourceCopy {
            let value: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse canonical payload {}", entry.path))?;
            ensure!(
                bytes == canonical_value(value)?,
                "{} is not canonical JSON",
                entry.path
            );
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    ensure!(!path.is_empty(), "fixture path is empty");
    ensure!(
        !path.contains('\\'),
        "fixture path uses a non-normalized separator: {path}"
    );
    let parsed = Path::new(path);
    ensure!(!parsed.is_absolute(), "fixture path is absolute: {path}");
    ensure!(
        parsed
            .components()
            .all(|component| matches!(component, Component::Normal(_))),
        "fixture path can escape or is not normalized: {path}"
    );
    Ok(())
}

fn validate_hash(hash: &str, path: &str) -> Result<()> {
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{path} has a non-canonical SHA-256 hash"
    );
    Ok(())
}

fn directory_inventory(directory: &Path) -> Result<BTreeSet<String>> {
    fs::read_dir(directory)
        .with_context(|| format!("read fixture directory {}", directory.display()))?
        .map(|entry| {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_file(),
                "fixture contains a non-file entry"
            );
            entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("fixture contains a non-UTF-8 filename"))
        })
        .collect()
}

fn compare_directories(expected: &Path, actual: &Path) -> Result<()> {
    let expected_inventory = directory_inventory(expected)?;
    let actual_inventory = directory_inventory(actual)?;
    ensure!(
        expected_inventory == actual_inventory,
        "fixture inventory differs from a fresh import"
    );
    for path in expected_inventory {
        let expected_bytes = fs::read(expected.join(&path))?;
        let actual_bytes = fs::read(actual.join(&path))?;
        ensure!(
            expected_bytes == actual_bytes,
            "fixture payload {path} differs from a fresh import"
        );
    }
    Ok(())
}

fn replace_directory(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("fixture destination has no parent")?;
    fs::create_dir_all(parent).context("create fixture parent directory")?;
    let replacement = parent.join(".v0.4.9.importing");
    if replacement.exists() {
        fs::remove_dir_all(&replacement).context("remove stale fixture staging directory")?;
    }
    copy_directory(source, &replacement)?;
    if destination.exists() {
        fs::remove_dir_all(destination).context("remove previous fixture after validation")?;
    }
    fs::rename(&replacement, destination).context("install validated fixture directory")?;
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_file(),
            "generated fixture contains a non-file entry"
        );
        fs::copy(entry.path(), destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn canonical_json(bytes: &[u8]) -> Result<Vec<u8>> {
    let value: Value = serde_json::from_slice(bytes).context("parse exported JSON")?;
    canonical_value(value)
}

fn canonical_value(mut value: Value) -> Result<Vec<u8>> {
    sort_value(&mut value);
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn sort_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for value in object.values_mut() {
                sort_value(value);
            }
            let old = std::mem::take(object);
            let sorted = old.into_iter().collect::<BTreeMap<_, _>>();
            object.extend(sorted);
        }
        Value::Array(array) => {
            for value in array {
                sort_value(value);
            }
        }
        _ => {}
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn cargo_output(archive: &Path, target_dir: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("cargo")
        .current_dir(archive)
        .env("CARGO_TARGET_DIR", target_dir)
        .args(arguments)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("run archived cargo {}", arguments.join(" ")))?;
    ensure!(output.status.success(), "archived cargo command failed");
    Ok(output.stdout)
}

fn git_output(repository: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .context("run Git")?;
    ensure!(output.status.success(), "Git command failed");
    String::from_utf8(output.stdout).context("Git produced non-UTF-8 output")
}

fn command(command: &mut Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to {description}"))?;
    ensure!(status.success(), "failed to {description}");
    Ok(())
}

fn presentation_vectors() -> Value {
    json!({
        "confirmation": [
            {
                "input": {"target_id": "target-1"},
                "method": "claim_tab",
                "output": {
                    "message": "Claim target target-1 for this agent session.",
                    "title": "Claim browser tab?"
                }
            },
            {
                "input": {},
                "method": "claim_tab",
                "output": {
                    "message": "Claim target (unknown target) for this agent session.",
                    "title": "Claim browser tab?"
                }
            },
            {
                "input": {"tab_id": "tab-1"},
                "method": "close_tab",
                "output": {
                    "message": "Close owned tab tab-1.",
                    "title": "Close browser tab?"
                }
            },
            {
                "input": {"leave_visible": false, "tab_id": "tab-1"},
                "method": "release_tab",
                "output": {
                    "message": "Release owned tab tab-1; a VBL-created target remains eligible for expiry cleanup.",
                    "title": "Release browser tab?"
                }
            },
            {
                "input": {"leave_visible": true, "tab_id": "tab-1", "user_instruction": " Keep this open "},
                "method": "release_tab",
                "output": {
                    "message": "Release owned tab tab-1 and preserve it after this session expires. User instruction: \"Keep this open\".",
                    "title": "Leave browser tab visible?"
                }
            },
            {
                "input": {"leave_visible": true, "tab_id": "tab-1"},
                "method": "release_tab",
                "output": {
                    "message": "Release owned tab tab-1 and preserve it after this session expires. User instruction: (missing; this request will be rejected).",
                    "title": "Leave browser tab visible?"
                }
            },
            {
                "input": {"tab_id": "tab-1"},
                "method": "focus_tab",
                "output": {
                    "message": "Focus owned tab tab-1 for manual inspection or handoff.",
                    "title": "Bring Chrome forward?"
                }
            },
            {"input": {}, "method": "snapshot", "output": null}
        ],
        "invocation": [
            {"displayName": "Start session", "method": "start_session", "output": "Starting a visible browser session"},
            {"displayName": "Snapshot", "method": "snapshot", "output": "Capturing a browser snapshot"},
            {"displayName": "Screenshot", "method": "screenshot", "output": "Capturing a browser screenshot"},
            {"displayName": "Navigate", "method": "navigate", "output": "Navigating the owned browser tab"},
            {"displayName": "Click", "method": "click", "output": "Clicking a browser element"},
            {"displayName": "Fill", "method": "fill", "output": "Filling browser form controls"},
            {"displayName": "Fill form", "method": "fill_form", "output": "Filling browser form controls"},
            {"displayName": "Wait", "method": "wait_for", "output": "Waiting for browser state"},
            {"displayName": "Console", "method": "console", "output": "Running Console"}
        ]
    })
}

const BASELINE_HELPER: &str = r#"fn main() -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(&agent_surface_contract::baseline_catalog())?);
    Ok(())
}
"#;

const ERROR_HELPER: &str = r#"use std::{path::Path, time::Duration};

use serde_json::json;
use visible_browser_lab::leases::{
    AgentSessionId, BrowserToolError, BrowserToolErrorCode, LeaseState, RecoveryAction, TabId,
};

fn main() -> anyhow::Result<()> {
    let session = AgentSessionId("session-example".to_string());
    let tab = TabId("tab-example".to_string());
    let errors = vec![
        ("invalid_input", json!({"message": "invalid input example"}), BrowserToolError::invalid_input("invalid input example")),
        ("chrome_unavailable", json!({"message": "Chrome unavailable example"}), BrowserToolError::chrome_unavailable("Chrome unavailable example")),
        ("invalid_request_context", json!({"message": "invalid request context example"}), BrowserToolError::invalid_request_context("invalid request context example")),
        ("session_required", json!({}), BrowserToolError::session_required()),
        ("workspace_context_conflict", json!({}), BrowserToolError::workspace_context_conflict()),
        ("unknown_session", json!({"sessionId": "session-example"}), BrowserToolError::unknown_session(&session)),
        ("session_expired", json!({"closedTargets": 1, "idleSeconds": 30, "pendingCloseTargets": 2, "sessionId": "session-example"}), BrowserToolError::session_expired(&session, Duration::from_secs(30), 2, 1)),
        ("unknown_tab", json!({"tabId": "tab-example"}), BrowserToolError::unknown_tab(&tab)),
        ("tab_not_owned", json!({"tabId": "tab-example"}), BrowserToolError::tab_not_owned(&tab)),
        ("tab_not_active", json!({"state": "released", "tabId": "tab-example"}), BrowserToolError::tab_not_active(&tab, &LeaseState::Released)),
        ("target_missing", json!({"tabId": "tab-example"}), BrowserToolError::target_missing(&tab)),
        ("target_missing_for_target", json!({"targetId": "target-example"}), BrowserToolError::target_missing_for_target("target-example")),
        ("target_owned", json!({"targetId": "target-example"}), BrowserToolError::target_owned("target-example")),
        ("element_not_found", json!({"target": "ref=e1"}), BrowserToolError::element_not_found("ref=e1")),
        ("element_ambiguous", json!({"count": 2, "target": "button"}), BrowserToolError::element_ambiguous("button", 2)),
        ("element_stale", json!({"reference": "e1"}), BrowserToolError::element_stale("e1")),
        ("element_not_actionable", json!({"message": "element is covered"}), BrowserToolError::element_not_actionable("element is covered")),
        ("operation_timeout", json!({"message": "operation timed out"}), BrowserToolError::operation_timeout("operation timed out")),
        ("focus_required", json!({"tabId": "tab-example"}), BrowserToolError::focus_required(&tab)),
        ("artifact_not_found", json!({"artifactId": "artifact-example"}), BrowserToolError::artifact_not_found("artifact-example")),
        ("artifact_error", json!({"message": "artifact error example"}), BrowserToolError::artifact_error("artifact error example")),
        ("workspace_unavailable", json!({"message": "workspace unavailable example"}), BrowserToolError::workspace_unavailable("workspace unavailable example")),
        ("path_outside_workspace", json!({"path": "/outside/example"}), BrowserToolError::path_outside_workspace(Path::new("/outside/example"))),
    ];
    let error_codes = [
        BrowserToolErrorCode::ChromeUnavailable,
        BrowserToolErrorCode::InvalidRequestContext,
        BrowserToolErrorCode::SessionRequired,
        BrowserToolErrorCode::UnknownSession,
        BrowserToolErrorCode::SessionExpired,
        BrowserToolErrorCode::UnknownTab,
        BrowserToolErrorCode::TabNotOwned,
        BrowserToolErrorCode::TabNotActive,
        BrowserToolErrorCode::TargetMissing,
        BrowserToolErrorCode::TargetOwned,
        BrowserToolErrorCode::InvalidInput,
        BrowserToolErrorCode::OperationTimeout,
        BrowserToolErrorCode::FocusRequired,
        BrowserToolErrorCode::ElementNotFound,
        BrowserToolErrorCode::ElementAmbiguous,
        BrowserToolErrorCode::ElementStale,
        BrowserToolErrorCode::ElementNotActionable,
        BrowserToolErrorCode::ArtifactNotFound,
        BrowserToolErrorCode::ArtifactError,
        BrowserToolErrorCode::WorkspaceUnavailable,
        BrowserToolErrorCode::WorkspaceContextConflict,
        BrowserToolErrorCode::PathOutsideWorkspace,
    ];
    let recovery_actions = [
        RecoveryAction::StartSession,
        RecoveryAction::ListTabs,
        RecoveryAction::NewTab,
        RecoveryAction::ClaimExistingTab,
        RecoveryAction::ReleaseTab,
        RecoveryAction::FocusTab,
        RecoveryAction::StartChrome,
        RecoveryAction::Snapshot,
        RecoveryAction::WaitFor,
    ];
    let vectors = errors
        .into_iter()
        .map(|(constructor, input, error)| json!({"constructor": constructor, "error": error, "input": input}))
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string(&json!({
        "errorCodes": error_codes,
        "recoveryActions": recovery_actions,
        "serializationVectors": vectors,
    }))?);
    Ok(())
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_copy() -> Result<TempDir> {
        let copy = TempDir::new()?;
        copy_directory(&fixture_directory(), copy.path())?;
        Ok(copy)
    }

    fn read_manifest(directory: &Path) -> Result<Manifest> {
        Ok(serde_json::from_slice(&fs::read(
            directory.join("manifest.json"),
        )?)?)
    }

    fn write_manifest(directory: &Path, manifest: &Manifest) -> Result<()> {
        fs::write(
            directory.join("manifest.json"),
            canonical_value(serde_json::to_value(manifest)?)?,
        )?;
        Ok(())
    }

    #[test]
    fn checked_in_fixture_is_complete_and_valid() -> Result<()> {
        validate_bundle(&fixture_directory())
    }

    #[test]
    fn exact_offline_reproduction_matches() -> Result<()> {
        let copy = fixture_copy()?;
        validate_bundle(copy.path())?;
        compare_directories(&fixture_directory(), copy.path())
    }

    #[test]
    fn tampered_payload_is_rejected() -> Result<()> {
        let copy = fixture_copy()?;
        fs::write(copy.path().join("baseline-tools.json"), b"{}\n")?;
        let error = validate_bundle(copy.path()).unwrap_err();
        assert!(error.to_string().contains("hash does not match"));
        Ok(())
    }

    #[test]
    fn extra_and_missing_payloads_are_rejected() -> Result<()> {
        let extra = fixture_copy()?;
        fs::write(extra.path().join("unexpected.json"), b"{}\n")?;
        assert!(
            validate_bundle(extra.path())
                .unwrap_err()
                .to_string()
                .contains("extra or missing")
        );

        let missing = fixture_copy()?;
        fs::remove_file(missing.path().join("surface-catalog.json"))?;
        assert!(
            validate_bundle(missing.path())
                .unwrap_err()
                .to_string()
                .contains("extra or missing")
        );
        Ok(())
    }

    #[test]
    fn escaping_and_non_normalized_paths_are_rejected() -> Result<()> {
        for invalid in [
            "../baseline-tools.json",
            "/baseline-tools.json",
            "nested/../baseline-tools.json",
            "nested\\baseline-tools.json",
        ] {
            let copy = fixture_copy()?;
            let mut manifest = read_manifest(copy.path())?;
            manifest.files[0].sources[0].path = invalid.to_string();
            write_manifest(copy.path(), &manifest)?;
            let error = validate_bundle(copy.path()).unwrap_err();
            assert!(
                error.to_string().contains("inventory")
                    || error.to_string().contains("escape")
                    || error.to_string().contains("absolute")
                    || error.to_string().contains("separator")
            );
        }
        Ok(())
    }

    #[test]
    fn partial_refresh_is_valid_in_isolation_but_not_reproducible() -> Result<()> {
        let copy = fixture_copy()?;
        let changed = canonical_value(json!({"partial": "refresh"}))?;
        fs::write(copy.path().join("baseline-tools.json"), &changed)?;
        let mut manifest = read_manifest(copy.path())?;
        let baseline = manifest
            .files
            .iter_mut()
            .find(|entry| entry.path == "baseline-tools.json")
            .expect("baseline entry");
        baseline.sha256 = sha256(&changed);
        write_manifest(copy.path(), &manifest)?;
        validate_bundle(copy.path())?;
        let error = compare_directories(&fixture_directory(), copy.path()).unwrap_err();
        assert!(error.to_string().contains("differs from a fresh import"));
        Ok(())
    }

    #[test]
    fn manifest_must_be_written_and_hashed_canonically() -> Result<()> {
        let copy = fixture_copy()?;
        let manifest = read_manifest(copy.path())?;
        fs::write(
            copy.path().join("manifest.json"),
            serde_json::to_vec(&manifest)?,
        )?;
        assert!(
            validate_bundle(copy.path())
                .unwrap_err()
                .to_string()
                .contains("canonical JSON")
        );
        Ok(())
    }
}
