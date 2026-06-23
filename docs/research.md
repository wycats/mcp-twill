# Research And Protocol Notes

This project is not a shell wrapper and not a proposal to replace MCP with CLIs. It keeps MCP as the protocol/control plane while making the model-facing invocation shape look like a CLI.

## MCP Baseline

The framework uses the 2025-11-25 MCP specification surface:

- Tools expose `help`, one primary execution tool, and catalog-required effect-lane execution tools with truthful MCP annotations.
- Resources expose server overview, command catalog, command docs, and permissions.
- Prompts expose a reusable getting-started prompt.
- Declared workspaces constrain path-typed arguments. MCP roots can be projected into the same workspace model in a later workspace milestone.
- Progress notifications make a single execution call visible as parse, plan, dispatch, and complete phases.
- Task-augmented execution is supported as optional v1 behavior because MCP tasks are still an experimental protocol feature.

## Prior Art

- `mcp-cli` shows the value of dynamic discovery and reducing upfront tool-token load.
- CLI-Anything and OpenCLI show the strength of agent-operable command trees.
- Agent-friendly CLI guidance consistently emphasizes structured output, good help, stable subcommands, meaningful errors, and examples.

## Position

The framework borrows the ergonomic parts of CLI usage without inheriting shell failure modes:

- Familiar command grammar for agents.
- Typed placeholders for non-trivial values.
- Protocol-level output controls instead of `head`, `jq`, or shell pipelines.
- Declarative permissions before dispatch.
- Consistent help/resources/prompts rather than ad hoc `--help` conventions.
