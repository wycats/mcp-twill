# mcp-twill

`mcp-twill` is an MCP server framework built around one authoritative catalog
of typed operations.

A server declares command paths, structured arguments, results, effects,
resources, permissions, guidance, presentation, and task delivery once. Twill
projects that catalog into planning, help, JSON Schema, native MCP tools,
previews, replay fingerprints, diagnostics, and execution.

Command templates are CLI-shaped syntax over typed JSON values. They are not
shell programs: pipes, redirection, command substitution, and shell expansion
are outside the command language.

```toml
[dependencies]
mcp-twill = "0.1.0"
```

See the repository’s
[`issues_server`](https://github.com/wycats/mcp-twill/blob/main/crates/mcp-twill/examples/issues_server.rs)
and
[`native_surface_server`](https://github.com/wycats/mcp-twill/blob/main/crates/mcp-twill/examples/native_surface_server.rs)
examples for the builder and native-surface paths.

The package requires Rust 1.88 or newer. API documentation is available on
[docs.rs](https://docs.rs/mcp-twill), and the design corpus lives in the
[mcp-twill repository](https://github.com/wycats/mcp-twill).

Licensed under Apache-2.0.
