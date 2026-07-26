# mcp-twill-host

`mcp-twill-host` compiles a Twill native-tool snapshot into a canonical,
separately hashed host profile.

The profile drives:

- closed typed call, result, and private-context contracts;
- bounded in-process and process-envelope transports;
- deterministic VS Code contribution manifests and TypeScript adapters;
- confirmation, routing, result-shaping, cancellation, and cleanup policy.

Host generation consumes the immutable surface compiled by `mcp-twill`; it does
not re-author schemas or application behavior.

```toml
[dependencies]
mcp-twill-host = "0.1.0"
```

The package requires Rust 1.88 or newer. API documentation is available on
[docs.rs](https://docs.rs/mcp-twill-host), and the source lives in the
[mcp-twill repository](https://github.com/wycats/mcp-twill).

Licensed under Apache-2.0.
