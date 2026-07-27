# Changelog

All notable changes to the public Twill crates are recorded here.

## 0.1.1 - 2026-07-26

- Preserve protocol-version observation across rmcp 1.7 and 1.8 ownership
  semantics.
- Verify the synchronized packages against current registry dependency
  resolution before tagging the first public release.

## 0.1.0 - 2026-07-26

The first public release includes:

- typed command catalogs, builders, planning, execution, help, and schemas;
- workspace resolution, resources, ambient binding, and conversation identity;
- application-result, presentation, capability, and task-delivery contracts;
- compiled native MCP surfaces and protocol-versioned stateless serving;
- canonical host profiles, bounded transports, and generated VS Code adapters;
- frozen VBL and MCP protocol evidence with contract and acceptance coverage.

The public packages are `mcp-workspace-resolver`, `mcp-twill`, and
`mcp-twill-host`, all released at version 0.1.0.
