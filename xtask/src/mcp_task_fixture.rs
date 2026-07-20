use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::BufReader,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

pub const LEGACY_COMMIT: &str = "38c84e9f93ad191d9eb26d92b945d17bd0efcaf3";
pub const CORE_RC_COMMIT: &str = "9d700ed62dcf86cb77475c9b81930611a9182f46";
pub const EXTENSION_COMMIT: &str = "8966bea9c4f4e6d71060cc8284a539086e9e234f";
pub const CORE_REPOSITORY: &str = "https://github.com/modelcontextprotocol/modelcontextprotocol";
pub const EXTENSION_REPOSITORY: &str =
    "https://github.com/modelcontextprotocol/experimental-ext-tasks";
pub const PROTOCOL_REVISION: &str = "2026-07-28";
pub const EXTENSION_ID: &str = "io.modelcontextprotocol/tasks";
pub const FINAL_RELEASE_TAG: &str = "2026-07-28";
pub const IMPORT_COMMAND: &str = "cargo xtask import-mcp-task-fixture --core-repository <local-git-repository> --extension-repository <local-git-repository> [--final-ref 2026-07-28]";

const EXPECTED_MANIFEST_SHA256: &str =
    "58e4d1665946dbf1d8630b06c6c0e9cdfe3be0df2f4b1b5df469ffb3b31e6b4c";
const EXPECTED_FINAL_RELEASE_COMMIT: Option<&str> = None;

const SOURCE_COPIES: [SourceCopy; 8] = [
    SourceCopy {
        destination: "core-schema.json",
        source_id: "core-2026-07-28-rc",
        source_path: "schema/draft/schema.json",
    },
    SourceCopy {
        destination: "core-transports.mdx",
        source_id: "core-2026-07-28-rc",
        source_path: "docs/specification/draft/basic/transports.mdx",
    },
    SourceCopy {
        destination: "extension-schema.json",
        source_id: "tasks-extension",
        source_path: "schema/draft/schema.json",
    },
    SourceCopy {
        destination: "extension-sep-2663.md",
        source_id: "tasks-extension",
        source_path: "seps/2663-tasks-extension.md",
    },
    SourceCopy {
        destination: "extension-tasks.md",
        source_id: "tasks-extension",
        source_path: "specification/draft/tasks.md",
    },
    SourceCopy {
        destination: "legacy-progress.mdx",
        source_id: "legacy-2025-11-25",
        source_path: "docs/specification/2025-11-25/basic/utilities/progress.mdx",
    },
    SourceCopy {
        destination: "legacy-schema.json",
        source_id: "legacy-2025-11-25",
        source_path: "schema/2025-11-25/schema.json",
    },
    SourceCopy {
        destination: "legacy-tasks.mdx",
        source_id: "legacy-2025-11-25",
        source_path: "docs/specification/2025-11-25/basic/utilities/tasks.mdx",
    },
];

const REVIEWED_VECTORS: [ReviewedVector; 3] = [
    ReviewedVector {
        destination: "core-wire-vectors.json",
        source_id: "core-2026-07-28-rc",
        source_paths: &[
            "docs/specification/draft/basic/transports.mdx",
            "schema/draft/schema.json",
        ],
        value: core_vectors,
    },
    ReviewedVector {
        destination: "extension-wire-vectors.json",
        source_id: "tasks-extension",
        source_paths: &[
            "schema/draft/schema.json",
            "seps/2663-tasks-extension.md",
            "specification/draft/tasks.md",
        ],
        value: extension_vectors,
    },
    ReviewedVector {
        destination: "legacy-wire-vectors.json",
        source_id: "legacy-2025-11-25",
        source_paths: &[
            "docs/specification/2025-11-25/basic/utilities/tasks.mdx",
            "schema/2025-11-25/schema.json",
        ],
        value: legacy_vectors,
    },
];

struct SourceCopy {
    destination: &'static str,
    source_id: &'static str,
    source_path: &'static str,
}

struct ReviewedVector {
    destination: &'static str,
    source_id: &'static str,
    source_paths: &'static [&'static str],
    value: fn() -> Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format_version: u32,
    pub protocol_revision: String,
    pub extension_id: String,
    pub sources: Vec<SourceIdentity>,
    pub importer: ImporterIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_release: Option<FinalRelease>,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub id: String,
    pub repository: String,
    pub revision: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporterIdentity {
    pub version: u32,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalRelease {
    pub tag: String,
    pub peeled_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub source_id: String,
    pub derivation: Derivation,
    pub source_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Derivation {
    SourceCopy,
    ReviewedVector,
}

pub fn import(
    core_repository: &Path,
    extension_repository: &Path,
    final_reference: Option<&str>,
    check: bool,
) -> Result<()> {
    ensure_repository(core_repository, "core")?;
    ensure_repository(extension_repository, "extension")?;
    verify_commit(core_repository, LEGACY_COMMIT)?;
    verify_commit(core_repository, CORE_RC_COMMIT)?;
    verify_commit(extension_repository, EXTENSION_COMMIT)?;

    let legacy = archive(core_repository, LEGACY_COMMIT)?;
    let core = archive(core_repository, CORE_RC_COMMIT)?;
    let extension = archive(extension_repository, EXTENSION_COMMIT)?;
    let final_release = if let Some(reference) = final_reference {
        ensure!(
            reference == FINAL_RELEASE_TAG,
            "the final release accepts only --final-ref {FINAL_RELEASE_TAG}"
        );
        let peeled_commit = resolve_final_release_commit(core_repository, reference)?;
        let final_core = archive(core_repository, &peeled_commit)?;
        verify_final_core_inputs(core.path(), final_core.path())?;
        Some(FinalRelease {
            tag: FINAL_RELEASE_TAG.to_string(),
            peeled_commit,
        })
    } else {
        None
    };
    let generated = TempDir::new().context("create generated task fixture")?;
    generate_bundle(
        legacy.path(),
        core.path(),
        extension.path(),
        final_release,
        generated.path(),
    )?;
    validate_bundle(generated.path())?;

    let destination = fixture_directory();
    if check {
        validate_bundle(&destination)?;
        compare_directories(generated.path(), &destination)?;
        println!(
            "MCP task fixture matches legacy {LEGACY_COMMIT}, core {CORE_RC_COMMIT}, and extension {EXTENSION_COMMIT}"
        );
    } else {
        replace_directory(generated.path(), &destination)?;
        println!("imported MCP task fixture into {}", destination.display());
    }
    Ok(())
}

pub fn validate_release() -> Result<()> {
    validate_bundle(&fixture_directory())?;
    let manifest = read_manifest(&fixture_directory())?;
    validate_release_manifest(&manifest)
}

pub fn fixture_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask belongs to the workspace")
        .join("crates/mcp-twill/tests/fixtures/mcp/tasks")
}

fn ensure_repository(repository: &Path, kind: &str) -> Result<()> {
    ensure!(
        repository.is_dir(),
        "{kind} repository does not exist: {}",
        repository.display()
    );
    Ok(())
}

fn verify_commit(repository: &Path, commit: &str) -> Result<()> {
    let resolved = git_output(repository, &["rev-parse", &format!("{commit}^{{commit}}")])?;
    ensure!(
        resolved.trim() == commit,
        "{commit} resolves to {}, expected an exact pinned commit",
        resolved.trim()
    );
    Ok(())
}

fn resolve_final_release_commit(repository: &Path, reference: &str) -> Result<String> {
    let peeled_commit = git_output(
        repository,
        &["rev-parse", &format!("refs/tags/{reference}^{{commit}}")],
    )?
    .trim()
    .to_string();
    validate_commit(&peeled_commit, "final release peeled commit")?;
    Ok(peeled_commit)
}

fn archive(repository: &Path, commit: &str) -> Result<TempDir> {
    let destination = TempDir::new().context("create pristine source archive")?;
    let tar_file = tempfile::NamedTempFile::new().context("create Git archive file")?;
    let status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["archive", "--format=tar", "--output"])
        .arg(tar_file.path())
        .arg(commit)
        .status()
        .context("export pinned Git archive")?;
    ensure!(
        status.success(),
        "failed to export pinned Git archive {commit}"
    );
    let file = fs::File::open(tar_file.path()).context("open Git archive")?;
    let mut archive = tar::Archive::new(BufReader::new(file));
    archive.set_preserve_permissions(false);
    archive
        .unpack(destination.path())
        .context("unpack pinned Git archive")?;
    Ok(destination)
}

fn generate_bundle(
    legacy: &Path,
    core: &Path,
    extension: &Path,
    final_release: Option<FinalRelease>,
    output: &Path,
) -> Result<()> {
    let archives = BTreeMap::from([
        ("legacy-2025-11-25", legacy),
        ("core-2026-07-28-rc", core),
        ("tasks-extension", extension),
    ]);
    let mut files = Vec::new();

    for copy in SOURCE_COPIES {
        let bytes = fs::read(archives[copy.source_id].join(copy.source_path))
            .with_context(|| format!("read archived source {}", copy.source_path))?;
        fs::write(output.join(copy.destination), &bytes)
            .with_context(|| format!("write source copy {}", copy.destination))?;
        files.push(FileEntry {
            path: copy.destination.to_string(),
            sha256: sha256(&bytes),
            source_id: copy.source_id.to_string(),
            derivation: Derivation::SourceCopy,
            source_paths: vec![copy.source_path.to_string()],
        });
    }

    for vector in REVIEWED_VECTORS {
        let bytes = canonical_value((vector.value)())?;
        fs::write(output.join(vector.destination), &bytes)
            .with_context(|| format!("write reviewed vector {}", vector.destination))?;
        files.push(FileEntry {
            path: vector.destination.to_string(),
            sha256: sha256(&bytes),
            source_id: vector.source_id.to_string(),
            derivation: Derivation::ReviewedVector,
            source_paths: vector
                .source_paths
                .iter()
                .map(|path| (*path).to_string())
                .collect(),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let manifest = Manifest {
        format_version: 1,
        protocol_revision: PROTOCOL_REVISION.to_string(),
        extension_id: EXTENSION_ID.to_string(),
        sources: expected_sources(),
        importer: ImporterIdentity {
            version: 1,
            command: IMPORT_COMMAND.to_string(),
        },
        final_release,
        files,
    };
    fs::write(
        output.join("manifest.json"),
        canonical_value(serde_json::to_value(manifest)?)?,
    )
    .context("write task fixture manifest last")?;
    Ok(())
}

fn verify_final_core_inputs(locked_core: &Path, final_core: &Path) -> Result<()> {
    for copy in SOURCE_COPIES
        .iter()
        .filter(|copy| copy.source_id == "core-2026-07-28-rc")
    {
        let locked = fs::read(locked_core.join(copy.source_path))
            .with_context(|| format!("read locked core input {}", copy.source_path))?;
        let final_bytes = fs::read(final_core.join(copy.source_path))
            .with_context(|| format!("read final core input {}", copy.source_path))?;
        ensure!(
            locked == final_bytes,
            "final MCP release changes normative input {}",
            copy.source_path
        );
    }
    Ok(())
}

fn expected_sources() -> Vec<SourceIdentity> {
    vec![
        SourceIdentity {
            id: "core-2026-07-28-rc".to_string(),
            repository: CORE_REPOSITORY.to_string(),
            revision: "2026-07-28-RC".to_string(),
            commit: CORE_RC_COMMIT.to_string(),
        },
        SourceIdentity {
            id: "legacy-2025-11-25".to_string(),
            repository: CORE_REPOSITORY.to_string(),
            revision: "2025-11-25".to_string(),
            commit: LEGACY_COMMIT.to_string(),
        },
        SourceIdentity {
            id: "tasks-extension".to_string(),
            repository: EXTENSION_REPOSITORY.to_string(),
            revision: "SEP-2663-final".to_string(),
            commit: EXTENSION_COMMIT.to_string(),
        },
    ]
}

pub fn validate_bundle(directory: &Path) -> Result<()> {
    let manifest_bytes = fs::read(directory.join("manifest.json"))
        .with_context(|| format!("read fixture manifest in {}", directory.display()))?;
    let manifest_hash = sha256(&manifest_bytes);
    if manifest_hash != EXPECTED_MANIFEST_SHA256 {
        let mut sealed: Manifest =
            serde_json::from_slice(&manifest_bytes).context("parse sealed fixture manifest")?;
        ensure!(
            sealed.final_release.take().is_some(),
            "manifest.json does not match the pinned MCP task bundle: found {manifest_hash}"
        );
        let unsealed = canonical_value(serde_json::to_value(sealed)?)?;
        ensure!(
            sha256(&unsealed) == EXPECTED_MANIFEST_SHA256,
            "sealed manifest does not preserve the pinned MCP task bundle"
        );
    }
    validate_bundle_structure(directory)
}

fn validate_bundle_structure(directory: &Path) -> Result<()> {
    ensure!(
        directory.is_dir(),
        "fixture directory is missing: {}",
        directory.display()
    );
    let manifest_bytes =
        fs::read(directory.join("manifest.json")).context("read MCP task fixture manifest")?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("parse MCP task fixture manifest")?;
    ensure!(
        manifest_bytes == canonical_value(serde_json::to_value(&manifest)?)?,
        "manifest.json is not canonical JSON"
    );
    validate_manifest(&manifest)?;

    let expected = manifest
        .files
        .iter()
        .map(|entry| entry.path.clone())
        .chain(["manifest.json".to_string()])
        .collect::<BTreeSet<_>>();
    ensure!(
        directory_inventory(directory)? == expected,
        "fixture directory contains extra or missing files"
    );

    for entry in &manifest.files {
        let bytes = fs::read(directory.join(&entry.path))
            .with_context(|| format!("read fixture payload {}", entry.path))?;
        ensure!(
            sha256(&bytes) == entry.sha256,
            "{} hash does not match its payload",
            entry.path
        );
        if entry.derivation == Derivation::ReviewedVector {
            let value: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse reviewed vector {}", entry.path))?;
            ensure!(
                bytes == canonical_value(value)?,
                "{} is not canonical JSON",
                entry.path
            );
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    ensure!(
        manifest.format_version == 1,
        "unsupported fixture format version"
    );
    ensure!(
        manifest.protocol_revision == PROTOCOL_REVISION,
        "fixture protocol revision is not pinned"
    );
    ensure!(
        manifest.extension_id == EXTENSION_ID,
        "extension id is not pinned"
    );
    ensure!(
        manifest.sources == expected_sources(),
        "source identities are not pinned"
    );
    ensure!(
        manifest.importer
            == (ImporterIdentity {
                version: 1,
                command: IMPORT_COMMAND.to_string(),
            }),
        "fixture importer identity is not canonical"
    );

    let expected_paths = SOURCE_COPIES
        .iter()
        .map(|copy| (copy.destination, (Derivation::SourceCopy, copy.source_id)))
        .chain(REVIEWED_VECTORS.iter().map(|vector| {
            (
                vector.destination,
                (Derivation::ReviewedVector, vector.source_id),
            )
        }))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        manifest.files.len() == expected_paths.len(),
        "manifest payload inventory is incomplete"
    );
    let paths = manifest
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    let mut sorted_paths = paths.clone();
    sorted_paths.sort_unstable();
    ensure!(
        paths == sorted_paths,
        "manifest payload inventory is not sorted"
    );
    ensure!(
        paths.windows(2).all(|pair| pair[0] != pair[1]),
        "manifest contains duplicate payload paths"
    );

    for entry in &manifest.files {
        validate_relative_path(&entry.path)?;
        validate_hash(&entry.sha256, &entry.path)?;
        let Some((derivation, source_id)) = expected_paths.get(entry.path.as_str()) else {
            bail!("manifest contains unexpected payload {}", entry.path);
        };
        ensure!(
            entry.derivation == *derivation,
            "{} has an unexpected derivation",
            entry.path
        );
        ensure!(
            entry.source_id == *source_id,
            "{} has an unexpected source",
            entry.path
        );
        ensure!(
            !entry.source_paths.is_empty(),
            "{} has no source paths",
            entry.path
        );
        let mut sorted_sources = entry.source_paths.clone();
        sorted_sources.sort();
        ensure!(
            entry.source_paths == sorted_sources,
            "{} source paths are not sorted",
            entry.path
        );
        ensure!(
            entry.source_paths.windows(2).all(|pair| pair[0] != pair[1]),
            "{} contains duplicate source paths",
            entry.path
        );
        for path in &entry.source_paths {
            validate_relative_path(path)?;
        }
    }

    if let Some(release) = &manifest.final_release {
        validate_final_release(release)?;
    }
    Ok(())
}

fn validate_release_manifest(manifest: &Manifest) -> Result<()> {
    validate_release_manifest_against(manifest, EXPECTED_FINAL_RELEASE_COMMIT)
}

fn validate_release_manifest_against(
    manifest: &Manifest,
    expected_final_commit: Option<&str>,
) -> Result<()> {
    let release = manifest
        .final_release
        .as_ref()
        .context("MCP 2026-07-28 final release evidence is not sealed")?;
    validate_final_release(release)?;
    let expected_final_commit =
        expected_final_commit.context("MCP 2026-07-28 final release commit is not pinned")?;
    ensure!(
        release.peeled_commit == expected_final_commit,
        "final release peeled commit does not match the pinned commit"
    );
    Ok(())
}

fn validate_final_release(release: &FinalRelease) -> Result<()> {
    ensure!(
        release.tag == FINAL_RELEASE_TAG,
        "final release tag is not canonical"
    );
    validate_commit(&release.peeled_commit, "final release peeled commit")
}

fn validate_commit(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} is not a lowercase 40-character Git commit"
    );
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

fn read_manifest(directory: &Path) -> Result<Manifest> {
    serde_json::from_slice(&fs::read(directory.join("manifest.json"))?)
        .context("parse MCP task fixture manifest")
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
        ensure!(
            fs::read(expected.join(&path))? == fs::read(actual.join(&path))?,
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
    let replacement = parent.join(".tasks.importing");
    if replacement.exists() {
        fs::remove_dir_all(&replacement).context("remove stale fixture staging directory")?;
    }
    copy_directory(source, &replacement)?;
    if destination.exists() {
        fs::remove_dir_all(destination).context("remove previous fixture")?;
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
            object.extend(old.into_iter().collect::<BTreeMap<_, _>>());
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

fn git_output(repository: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .context("run Git")?;
    ensure!(
        output.status.success(),
        "Git command failed: git -C {} {}",
        repository.display(),
        arguments.join(" ")
    );
    String::from_utf8(output.stdout).context("Git produced non-UTF-8 output")
}

fn legacy_vectors() -> Value {
    json!({
        "protocolRevision": "2025-11-25",
        "cases": [
            {
                "name": "create-working-task",
                "request": {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"report_generate","arguments":{},"task":{"ttl":60000}}},
                "response": {"jsonrpc":"2.0","id":1,"result":{"_meta":{"io.modelcontextprotocol/related-task":{"taskId":"task-example"}},"task":{"taskId":"task-example","status":"working","statusMessage":"Task is running","createdAt":"2025-11-25T10:30:00Z","lastUpdatedAt":"2025-11-25T10:30:00Z","ttl":60000,"pollInterval":1000}}}
            },
            {
                "name": "poll-completed-task",
                "request": {"jsonrpc":"2.0","id":2,"method":"tasks/get","params":{"taskId":"task-example"}},
                "response": {"jsonrpc":"2.0","id":2,"result":{"taskId":"task-example","status":"completed","statusMessage":"Task completed","createdAt":"2025-11-25T10:30:00Z","lastUpdatedAt":"2025-11-25T10:30:01Z","ttl":60000,"pollInterval":1000}}
            },
            {
                "name": "retrieve-tool-result",
                "request": {"jsonrpc":"2.0","id":3,"method":"tasks/result","params":{"taskId":"task-example"}},
                "response": {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"done"}],"isError":false,"_meta":{"io.modelcontextprotocol/related-task":{"taskId":"task-example"}}}}
            },
            {
                "name": "cancel-terminal-task",
                "request": {"jsonrpc":"2.0","id":4,"method":"tasks/cancel","params":{"taskId":"task-example"}},
                "response": {"jsonrpc":"2.0","id":4,"error":{"code":-32602,"message":"Cannot cancel task: already in terminal status 'completed'"}}
            },
            {
                "name": "poll-application-error-task",
                "request": {"jsonrpc":"2.0","id":5,"method":"tasks/get","params":{"taskId":"task-application-refusal"}},
                "response": {"jsonrpc":"2.0","id":5,"result":{"taskId":"task-application-refusal","status":"failed","statusMessage":"Task failed","createdAt":"2025-11-25T10:30:00Z","lastUpdatedAt":"2025-11-25T10:30:01Z","ttl":60000,"pollInterval":1000}}
            },
            {
                "name": "retrieve-application-error-result",
                "request": {"jsonrpc":"2.0","id":6,"method":"tasks/result","params":{"taskId":"task-application-refusal"}},
                "response": {"jsonrpc":"2.0","id":6,"result":{"content":[{"type":"text","text":"application refusal"}],"isError":true,"_meta":{"io.modelcontextprotocol/related-task":{"taskId":"task-application-refusal"}}}}
            }
        ]
    })
}

fn request_metadata(with_tasks_extension: bool) -> Value {
    let extensions = if with_tasks_extension {
        json!({EXTENSION_ID: {}})
    } else {
        json!({})
    };
    json!({
        "io.modelcontextprotocol/clientCapabilities": {"extensions": extensions},
        "io.modelcontextprotocol/clientInfo": {
            "name": "mcp-twill-fixture-client",
            "version": "1.0.0"
        },
        "io.modelcontextprotocol/protocolVersion": PROTOCOL_REVISION
    })
}

fn core_vectors() -> Value {
    json!({
        "protocolRevision": PROTOCOL_REVISION,
        "cases": [
            {
                "name": "matching-tool-routing",
                "httpStatus": 200,
                "headers": {"MCP-Protocol-Version":PROTOCOL_REVISION,"Mcp-Method":"tools/call","Mcp-Name":"report_generate"},
                "request": {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"_meta":request_metadata(false),"name":"report_generate","arguments":{}}}
            },
            {
                "name": "header-mismatch",
                "httpStatus": 400,
                "response": {"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"Header mismatch"}}
            },
            {
                "name": "unsupported-protocol-version",
                "httpStatus": 400,
                "response": {"jsonrpc":"2.0","id":1,"error":{"code":-32004,"message":"Unsupported protocol version","data":{"supported":[PROTOCOL_REVISION],"requested":"2099-01-01"}}}
            },
            {
                "name": "unsupported-method",
                "httpStatus": 404,
                "response": {"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}
            }
        ]
    })
}

fn extension_vectors() -> Value {
    json!({
        "extensionId": EXTENSION_ID,
        "cases": [
            {
                "name": "server-directed-task",
                "request": {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"_meta":request_metadata(true),"name":"report_generate","arguments":{}}},
                "response": {"jsonrpc":"2.0","id":1,"result":{"resultType":"task","taskId":"task-example","status":"working","statusMessage":"Task is running","createdAt":"2025-11-25T10:30:00Z","lastUpdatedAt":"2025-11-25T10:30:00Z","ttlMs":60000,"pollIntervalMs":1000}}
            },
            {
                "name": "completed-tool-error-is-completed",
                "request": {"jsonrpc":"2.0","id":2,"method":"tasks/get","params":{"_meta":request_metadata(true),"taskId":"task-example"}},
                "response": {"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","taskId":"task-example","status":"completed","statusMessage":"Task completed","createdAt":"2025-11-25T10:30:00Z","lastUpdatedAt":"2025-11-25T10:30:01Z","ttlMs":60000,"pollIntervalMs":1000,"result":{"resultType":"complete","content":[{"type":"text","text":"application refusal"}],"isError":true}}}
            },
            {
                "name": "update-acknowledgement",
                "request": {"jsonrpc":"2.0","id":3,"method":"tasks/update","params":{"_meta":request_metadata(true),"taskId":"task-example","inputResponses":{}}},
                "response": {"jsonrpc":"2.0","id":3,"result":{"resultType":"complete"}}
            },
            {
                "name": "cooperative-cancel-acknowledgement",
                "request": {"jsonrpc":"2.0","id":4,"method":"tasks/cancel","params":{"_meta":request_metadata(true),"taskId":"task-example"}},
                "response": {"jsonrpc":"2.0","id":4,"result":{"resultType":"complete"}}
            },
            {
                "name": "missing-required-capability",
                "request": {"jsonrpc":"2.0","id":5,"method":"tasks/get","params":{"_meta":request_metadata(false),"taskId":"task-example"}},
                "response": {"jsonrpc":"2.0","id":5,"error":{"code":-32003,"message":"Missing required client capability","data":{"requiredCapabilities":{"extensions":{EXTENSION_ID:{}}}}}}
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_copy() -> Result<TempDir> {
        let copy = TempDir::new()?;
        copy_directory(&fixture_directory(), copy.path())?;
        Ok(copy)
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
    fn exact_offline_copy_reproduces_the_bundle() -> Result<()> {
        let copy = fixture_copy()?;
        validate_bundle(copy.path())?;
        compare_directories(&fixture_directory(), copy.path())
    }

    #[test]
    fn tampered_extra_and_missing_payloads_are_rejected() -> Result<()> {
        let tampered = fixture_copy()?;
        fs::write(tampered.path().join("core-wire-vectors.json"), b"{}\n")?;
        assert!(
            validate_bundle(tampered.path())
                .unwrap_err()
                .to_string()
                .contains("hash does not match")
        );

        let extra = fixture_copy()?;
        fs::write(extra.path().join("unexpected.json"), b"{}\n")?;
        assert!(
            validate_bundle(extra.path())
                .unwrap_err()
                .to_string()
                .contains("extra or missing")
        );

        let missing = fixture_copy()?;
        fs::remove_file(missing.path().join("legacy-tasks.mdx"))?;
        assert!(
            validate_bundle(missing.path())
                .unwrap_err()
                .to_string()
                .contains("extra or missing")
        );
        Ok(())
    }

    #[test]
    fn escaping_and_non_normalized_paths_are_rejected() {
        for invalid in ["../x", "/x", "nested/../x", "nested\\x"] {
            assert!(validate_relative_path(invalid).is_err());
        }
    }

    #[test]
    fn payload_and_manifest_partial_refresh_is_rejected() -> Result<()> {
        let copy = fixture_copy()?;
        let changed = canonical_value(json!({"changed": true}))?;
        fs::write(copy.path().join("core-wire-vectors.json"), &changed)?;
        let mut manifest = read_manifest(copy.path())?;
        manifest
            .files
            .iter_mut()
            .find(|entry| entry.path == "core-wire-vectors.json")
            .expect("core vector")
            .sha256 = sha256(&changed);
        write_manifest(copy.path(), &manifest)?;
        assert!(
            validate_bundle(copy.path())
                .unwrap_err()
                .to_string()
                .contains("does not match the pinned MCP task bundle")
        );
        Ok(())
    }

    #[test]
    fn final_seal_is_the_only_permitted_change_to_the_pinned_manifest() -> Result<()> {
        let sealed = fixture_copy()?;
        let mut manifest = read_manifest(sealed.path())?;
        manifest.final_release = Some(FinalRelease {
            tag: FINAL_RELEASE_TAG.to_string(),
            peeled_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
        });
        write_manifest(sealed.path(), &manifest)?;
        validate_bundle(sealed.path())?;

        let changed = canonical_value(json!({"changed": true}))?;
        fs::write(sealed.path().join("core-wire-vectors.json"), &changed)?;
        manifest
            .files
            .iter_mut()
            .find(|entry| entry.path == "core-wire-vectors.json")
            .expect("core vector")
            .sha256 = sha256(&changed);
        write_manifest(sealed.path(), &manifest)?;
        assert!(
            validate_bundle(sealed.path())
                .unwrap_err()
                .to_string()
                .contains("does not preserve the pinned MCP task bundle")
        );
        Ok(())
    }

    #[test]
    fn rc_bundle_fails_the_release_seal_gate() -> Result<()> {
        let manifest = read_manifest(&fixture_directory())?;
        assert!(manifest.final_release.is_none());
        assert!(
            validate_release_manifest(&manifest)
                .unwrap_err()
                .to_string()
                .contains("not sealed")
        );
        Ok(())
    }

    #[test]
    fn release_seal_requires_the_exact_tag_and_canonical_peel() -> Result<()> {
        const TEST_FINAL_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
        let mut manifest = read_manifest(&fixture_directory())?;
        manifest.final_release = Some(FinalRelease {
            tag: FINAL_RELEASE_TAG.to_string(),
            peeled_commit: TEST_FINAL_COMMIT.to_string(),
        });
        validate_release_manifest_against(&manifest, Some(TEST_FINAL_COMMIT))?;
        assert!(
            validate_release_manifest(&manifest)
                .unwrap_err()
                .to_string()
                .contains("commit is not pinned")
        );
        assert!(
            validate_release_manifest_against(
                &manifest,
                Some("89abcdef0123456789abcdef0123456789abcdef")
            )
            .unwrap_err()
            .to_string()
            .contains("does not match the pinned commit")
        );
        manifest.final_release.as_mut().unwrap().tag = "draft".to_string();
        assert!(validate_release_manifest_against(&manifest, Some(TEST_FINAL_COMMIT)).is_err());
        manifest.final_release.as_mut().unwrap().tag = FINAL_RELEASE_TAG.to_string();
        manifest.final_release.as_mut().unwrap().peeled_commit = "ABC".to_string();
        assert!(validate_release_manifest_against(&manifest, Some(TEST_FINAL_COMMIT)).is_err());
        Ok(())
    }

    #[test]
    fn final_release_requires_byte_identical_frozen_core_inputs() -> Result<()> {
        let locked = TempDir::new()?;
        let final_core = TempDir::new()?;
        for copy in SOURCE_COPIES
            .iter()
            .filter(|copy| copy.source_id == "core-2026-07-28-rc")
        {
            let locked_path = locked.path().join(copy.source_path);
            let final_path = final_core.path().join(copy.source_path);
            fs::create_dir_all(locked_path.parent().expect("source has parent"))?;
            fs::create_dir_all(final_path.parent().expect("source has parent"))?;
            fs::write(&locked_path, copy.source_path.as_bytes())?;
            fs::write(&final_path, copy.source_path.as_bytes())?;
        }
        verify_final_core_inputs(locked.path(), final_core.path())?;
        fs::write(
            final_core.path().join("schema/draft/schema.json"),
            b"normative delta",
        )?;
        assert!(
            verify_final_core_inputs(locked.path(), final_core.path())
                .unwrap_err()
                .to_string()
                .contains("changes normative input")
        );
        Ok(())
    }

    #[test]
    fn final_release_resolution_requires_the_exact_tag_ref() -> Result<()> {
        let repository = TempDir::new()?;
        let git = |arguments: &[&str]| -> Result<()> {
            let status = Command::new("git")
                .arg("-C")
                .arg(repository.path())
                .args(arguments)
                .status()?;
            ensure!(status.success(), "test Git command failed");
            Ok(())
        };
        git(&["init", "--quiet"])?;
        fs::write(repository.path().join("evidence"), b"locked\n")?;
        git(&["add", "evidence"])?;
        git(&[
            "-c",
            "user.name=MCP Twill Tests",
            "-c",
            "user.email=mcp-twill@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ])?;
        git(&["branch", FINAL_RELEASE_TAG])?;
        assert!(resolve_final_release_commit(repository.path(), FINAL_RELEASE_TAG).is_err());

        git(&[
            "update-ref",
            &format!("refs/tags/{FINAL_RELEASE_TAG}"),
            "HEAD",
        ])?;
        let expected = git_output(repository.path(), &["rev-parse", "HEAD"])?;
        assert_eq!(
            resolve_final_release_commit(repository.path(), FINAL_RELEASE_TAG)?,
            expected.trim()
        );
        Ok(())
    }
}
