use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;

const VERSION: &str = "0.1.2";
const MAX_CRATE_BYTES: u64 = 10 * 1024 * 1024;

pub fn verify_archives() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must live under the workspace root")?;
    let package_dir = workspace.join("target/package");
    let verification_dir = package_dir.join("release-verification");
    let license = fs::read(workspace.join("LICENSE")).context("read workspace license")?;

    if verification_dir.exists() {
        fs::remove_dir_all(&verification_dir)
            .with_context(|| format!("clear {}", verification_dir.display()))?;
    }
    fs::create_dir_all(&verification_dir)
        .with_context(|| format!("create {}", verification_dir.display()))?;

    let resolver = verify_archive(
        &package_dir,
        &verification_dir,
        "mcp-workspace-resolver",
        &license,
    )?;
    let core = verify_archive(&package_dir, &verification_dir, "mcp-twill", &license)?;
    let host = verify_archive(&package_dir, &verification_dir, "mcp-twill-host", &license)?;

    patch_dependency(
        &core.join("Cargo.toml"),
        "mcp-workspace-resolver",
        "../mcp-workspace-resolver-0.1.2",
    )?;
    patch_dependency(&host.join("Cargo.toml"), "mcp-twill", "../mcp-twill-0.1.2")?;

    cargo_test(&resolver, &["--all-features", "--all-targets"])?;
    cargo_test(&core, &["--all-targets"])?;
    cargo_test(&host, &["--all-targets"])?;

    Ok(())
}

fn verify_archive(
    package_dir: &Path,
    verification_dir: &Path,
    package: &str,
    license: &[u8],
) -> Result<PathBuf> {
    let filename = format!("{package}-{VERSION}.crate");
    let direct_archive = package_dir.join(&filename);
    let registry_archive = package_dir.join("tmp-registry").join(&filename);
    let archive = if registry_archive.is_file() {
        registry_archive
    } else {
        direct_archive
    };
    let metadata = fs::metadata(&archive)
        .with_context(|| format!("missing release archive {}", archive.display()))?;
    ensure!(
        metadata.len() <= MAX_CRATE_BYTES,
        "{} is {} bytes, above crates.io's {}-byte limit",
        archive.display(),
        metadata.len(),
        MAX_CRATE_BYTES
    );

    let bytes = fs::read(&archive)
        .with_context(|| format!("read release archive {}", archive.display()))?;
    let digest = hex_digest(&bytes);
    Archive::new(GzDecoder::new(bytes.as_slice()))
        .unpack(verification_dir)
        .with_context(|| format!("extract release archive {}", archive.display()))?;
    let extracted = extracted_package(verification_dir, package);
    let inventory = inventory(&extracted)?;

    for required in [
        ".cargo_vcs_info.json",
        "Cargo.toml",
        "Cargo.toml.orig",
        "LICENSE",
        "README.md",
    ] {
        ensure!(
            inventory.contains(required),
            "{package} archive is missing `{required}`"
        );
    }
    ensure!(
        fs::read(extracted.join("LICENSE"))
            .with_context(|| format!("read packaged license for {package}"))?
            == license,
        "{package} archive license differs from the workspace license"
    );

    let normalized = fs::read_to_string(extracted.join("Cargo.toml"))
        .with_context(|| format!("read normalized manifest for {package}"))?;
    for required in [
        "rust-version = \"1.88\"",
        "license = \"Apache-2.0\"",
        "repository = \"https://github.com/wycats/mcp-twill\"",
        "homepage = \"https://command-woven.vercel.app\"",
        "readme = \"README.md\"",
    ] {
        ensure!(
            normalized.contains(required),
            "{package} normalized manifest is missing `{required}`"
        );
    }

    if package == "mcp-twill" {
        for excluded in ["tests/host_adapters.rs", "tests/support/vbl_host.rs"] {
            ensure!(
                !inventory.contains(excluded),
                "core archive retains repository-only test `{excluded}`"
            );
        }
    }

    if package == "mcp-twill-host" {
        for required in [
            "tests/typescript/package-lock.json",
            "tests/typescript/package.json",
            "tests/typescript/typecheck.mjs",
        ] {
            ensure!(
                inventory.contains(required),
                "host archive is missing `{required}`"
            );
        }
    }

    println!(
        "{package} {VERSION}: {} files, {} bytes, sha256 {digest}",
        inventory.len(),
        metadata.len()
    );
    Ok(extracted)
}

fn extracted_package(package_dir: &Path, package: &str) -> PathBuf {
    package_dir.join(format!("{package}-{VERSION}"))
}

fn inventory(root: &Path) -> Result<BTreeSet<String>> {
    ensure!(
        root.is_dir(),
        "missing extracted package directory {}",
        root.display()
    );
    let mut files = BTreeSet::new();
    collect_files(root, root, &mut files)?;
    Ok(files)
}

fn collect_files(root: &Path, directory: &Path, files: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(directory).with_context(|| format!("read {}", directory.display()))? {
        let entry = entry.with_context(|| format!("read entry under {}", directory.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("read type for {}", path.display()))?;
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .context("inventory path escaped extracted package")?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative);
        } else {
            bail!(
                "package inventory contains non-file entry {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn patch_dependency(manifest: &Path, dependency: &str, relative_path: &str) -> Result<()> {
    let source =
        fs::read_to_string(manifest).with_context(|| format!("read {}", manifest.display()))?;
    let path_line = format!("path = \"{relative_path}\"");
    if source.contains(&path_line) {
        return Ok(());
    }

    let table = format!("[dependencies.{dependency}]");
    let table_start = source
        .find(&table)
        .with_context(|| format!("missing `{table}` in {}", manifest.display()))?;
    let version_start = source[table_start..]
        .find("version = ")
        .map(|offset| table_start + offset)
        .with_context(|| {
            format!(
                "missing version for `{dependency}` in {}",
                manifest.display()
            )
        })?;
    let version_end = source[version_start..]
        .find('\n')
        .map(|offset| version_start + offset + 1)
        .unwrap_or(source.len());

    let mut patched = source;
    patched.insert_str(version_end, &format!("{path_line}\n"));
    fs::write(manifest, patched).with_context(|| format!("patch {}", manifest.display()))?;
    Ok(())
}

fn cargo_test(package: &Path, arguments: &[&str]) -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let manifest = package.join("Cargo.toml");
    let status = Command::new(cargo)
        .arg("test")
        .arg("--manifest-path")
        .arg(&manifest)
        .args(arguments)
        .status()
        .with_context(|| format!("run package tests for {}", package.display()))?;
    ensure!(
        status.success(),
        "package tests failed for {}",
        package.display()
    );
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").expect("write to String");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_lowercase_and_complete() {
        assert_eq!(
            hex_digest(b"twill"),
            "7b6edbb7cb69290fa3c5f18e11a6415d55ba398c5a99eba8f0effd6d47424e12"
        );
    }
}
