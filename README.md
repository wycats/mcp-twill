# MCP Twill

This repository implements a Rust framework for MCP servers that expose a compact, CLI-shaped command surface without using shell syntax.

Project values:

- The command string is a template over typed values, not a shell program.
- Placeholders such as `$args.title` bind structured JSON values into argv positions.
- Pipes, redirection, command substitution, and shell expansion do not belong in the command string.
- If similar capabilities are added later, they must be represented as typed framework features.

The MCP server exposes two tools:

- `help`: consistent discovery for server, command, arguments, examples, and permissions.
- `run`: parse a command template, bind typed args, build an invocation plan, report progress, and dispatch to native Rust handlers.

It also exposes MCP resources and a getting-started prompt so agents can understand the server without loading a large tool list.

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
