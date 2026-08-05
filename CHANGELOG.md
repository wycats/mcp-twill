# Changelog

All notable changes to the public Twill crates are recorded here.

## 0.1.2 - 2026-08-05

- Make native MCP resource and prompt capabilities, experimental metadata, and
  server identity configurable while keeping initialize instructions derived
  from the hash-covered compiled surface.
- Preserve authored nested `required` ordering in native input schemas without
  changing canonical registry schemas or catalog identity, including repeated
  schemas with hoisted definitions and canonically equal grouped members.
- Add an explicit, serialized, hash-covered compatibility dialect for native
  group descriptions that must remain byte-for-byte authored while retaining
  catalog-derived guidance as the default.

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
