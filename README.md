# MCP Twill

This repository implements a Rust framework for MCP servers that expose a compact, CLI-shaped command surface without using shell syntax.

Explore how one declaration becomes schemas, help, native MCP tools, previews,
results, and generated host adapters at
[A Command, Woven](https://command-woven.vercel.app).

## Crates

| Crate | Purpose |
| --- | --- |
| [`mcp-twill`](https://docs.rs/mcp-twill) | Authoritative catalog, builders, planning, execution, native MCP surfaces, resources, results, presentation, and tasks |
| [`mcp-workspace-resolver`](https://docs.rs/mcp-workspace-resolver) | Deterministic workspace-root resolution from MCP, Codex, trusted-host, and declared observations |
| [`mcp-twill-host`](https://docs.rs/mcp-twill-host) | Canonical host profiles, bounded transports, and generated VS Code adapters |

Add the framework to a server:

```sh
cargo add mcp-twill@0.1.0
```

Applications that embed the reusable resolver or generate host integrations can
also add `mcp-workspace-resolver` or `mcp-twill-host` at the same version.
Version 0.1.0 requires Rust 1.88 or newer.

Project values:

- The command string is a template over typed values, not a shell program.
- Placeholders such as `$args.title` bind structured JSON values into argv positions.
- Pipes, redirection, command substitution, and shell expansion do not belong in the command string.
- If similar capabilities are added later, they must be represented as typed framework features.

The MCP server exposes a compact tool surface:

- `help`: consistent discovery for server, command, arguments, examples, and permissions.
- A primary execution tool, `run` by default, that parses a command template, binds typed args, builds an invocation plan, reports progress, and dispatches to native Rust handlers.
- Generated effect-lane execution tools, such as `run-write`, only when the catalog contains commands that need separate MCP annotations.

Agents should start with the primary execution tool. If a command needs another lane, MCP Twill returns a structured tool result naming the required tool and preserving the original typed request for retry.

It also exposes MCP resources and a getting-started prompt so agents can understand the server without loading a large tool list.

## Agent Ergonomics

### Operating Twill Servers

Agents operate a Twill server by reading the generated help, resources, and getting-started prompt, then calling the primary execution tool. Command strings select operations and bind placeholders such as `$args.title`; non-trivial values stay in structured `args`. When an operation requires another effect lane, the framework returns structured retry data naming the required tool and preserving the request.

### Writing Twill Servers

Agents helping write Twill servers should keep the command declaration and handler aligned with the catalog. A command's path, summary, description, typed args, workspace relationships, permissions, examples, output contract, and handler should be added together. The ergonomics API in [RFC 0006](docs/rfcs/stage-4/0006-author-ergonomics.md) makes that the ordinary authoring path with builders, typed handler extraction, permission helpers, workspace helpers, and example validation.

## Design Notes

- [Research and protocol notes](docs/research.md)
- [Draft RFCs](docs/rfcs/README.md)
- [Release history](CHANGELOG.md)
- [Maintainer release process](RELEASING.md)

The 0.1 series is the first public API line. Twill follows semantic versioning,
but pre-1.0 minor releases may intentionally revise public contracts.

## Example

```json
{
  "command": "issues create --title $args.title --body $args.body",
  "args": {
    "title": "Crash on launch",
    "body": "The app exits after the splash screen."
  },
  "output": {
    "format": "structured",
    "fields": ["id", "title"],
    "limit": 10
  }
}
```

Run the example stdio MCP server:

```powershell
cargo run --example issues_server
```

## License

Apache-2.0.
