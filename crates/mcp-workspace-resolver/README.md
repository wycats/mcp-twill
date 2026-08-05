# mcp-workspace-resolver

`mcp-workspace-resolver` selects named workspace roots from the filesystem
context an MCP server can observe.

It keeps authority explicit:

- MCP roots outrank embedding-specific observations.
- Codex sandbox metadata outranks trusted-host and declared fallbacks.
- A present higher-authority observation blocks lower-authority fallthrough,
  including when the observation is empty.
- Paths are normalized and compared without filesystem access.
- Diagnostics are structured and avoid disclosing private host metadata.

The optional `rmcp` feature adds conversions from rmcp root types.

```toml
[dependencies]
mcp-workspace-resolver = { version = "0.1.2", features = ["rmcp"] }
```

The package requires Rust 1.88 or newer. API documentation is available on
[docs.rs](https://docs.rs/mcp-workspace-resolver), and the source lives in the
[mcp-twill repository](https://github.com/wycats/mcp-twill).

Licensed under Apache-2.0.
