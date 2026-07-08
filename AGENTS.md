# mcp-twill

Rust workspace for Twill, an MCP server framework built around an authoritative
command catalog. Three crates under `crates/`:

- `mcp-twill`: the framework (catalog model, registry, builder DSL, response
  profiles, rmcp adapter). Almost all work happens here.
- `mcp-twill-host`: host-side harness.
- `mcp-workspace-resolver`: workspace resolution (RFC 0007).

## Build and test

```sh
cargo test --workspace          # full suite; must be green
cargo clippy --workspace --all-targets  # must be warning-free
cargo fmt --all                 # CI enforces fmt --check
```

CI runs `cargo fmt --all -- --check` and `cargo test --workspace` on every PR.

Warning policy is green-to-green: if you encounter a clippy warning while
working, fix it in the same PR even if you didn't introduce it. Fix warnings at
the root rather than `#[allow]`ing them (e.g. `FrameworkError` boxes its slice
fields instead of allowing `result_large_err`).

## RFC corpus

Design work is RFC-driven. RFCs live in `docs/rfcs/` under stage directories
(`stage-0` = idea through `stage-4` = stable, shipped). `docs/rfcs/README.md`
is the index; `docs/rfcs/0000-template.md` is the template.

RFCs are managed by the exo CLI (`exo rfc status`, `exo rfc show <id>`,
`exo rfc promote <id> --stage <n>`). Each RFC file carries an
`<!-- exo:N ulid:... -->` anchor on line 1; exo owns that line, and promotion
moves the file between stage directories one stage at a time. Update the README
index when an RFC changes stage.

RFC prose conventions:

- House heading style is `## Rationale And Alternatives` (capital "And").
- Sections make arguments in prose, in the tradition of Rust and Ember RFCs.
  Lists are for genuinely structural content: exact rule inventories
  (Required Invariants), acceptance test enumerations, open questions, tables.
- Acceptance tests named in an RFC map to real tests in `crates/mcp-twill/tests/`.

## Conventions

- PRs squash-merge to `main`.
- New behavior lands with acceptance tests in `crates/mcp-twill/tests/`
  (e.g. `resources.rs` for RFC 0012).
- `.vscode/settings.json` may carry local debug-logging settings
  (`vercel.ai.logging.*`); leave those uncommitted.
- `.logs/` is gitignored scratch space for investigation logs.
- rust-analyzer uses `target/rust-analyzer` as its target dir so it never
  contends with CLI cargo builds.
