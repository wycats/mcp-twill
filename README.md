# MCP Twill

This repository implements a Rust framework for MCP servers that expose a compact, CLI-shaped command surface without using shell syntax.

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
