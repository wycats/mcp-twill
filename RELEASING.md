# Releasing Twill

Twill publishes three crates at one version from one reviewed commit:

1. `mcp-workspace-resolver`
2. `mcp-twill`
3. `mcp-twill-host`

Cargo derives that order from the versioned path dependencies. `xtask` is
unpublished.

## Prepare

Start from a clean, current `main` checkout. Confirm that the changelog and
three package versions describe the intended release, then run:

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo +1.88.0 check --workspace --all-targets
cargo publish --workspace --dry-run
cargo xtask verify-release-archives
```

Run `npm ci && npm test` in
`target/package/release-verification/mcp-twill-host-0.1.2/tests/typescript`
after archive verification. The test generates and validates both TypeScript
transport variants from the packaged host crate.

Review `target/package/*.crate`, the inventory and SHA-256 output from the
xtask, and the normalized manifests. Confirm the intended crate names are still
available before a first release.

## Publish

Crates.io publication is permanent and requires a separate explicit approval.
From the exact reviewed commit:

```sh
cargo publish --workspace
```

Cargo verifies the packages through a temporary registry, then uploads them in
dependency order. Wait for all three exact versions to appear in the crates.io
index.

Create a disposable project outside this repository that depends only on the
three registry versions. Compile a minimal resolver use, Twill server, and host
profile without path patches. Record the resolved package sources and versions.

After the registry-only consumer check passes, create the annotated `v0.1.2`
tag at the published commit and a GitHub release containing the three archive
SHA-256 values. Configure crates.io trusted publishing for subsequent releases.
