<!-- exo:4 ulid:01kvrzxqkycwd2mzeawz1cvv1g -->

# RFC 0004: Runtime Maturity, Workspace Identity, Events, And Contract Tests

- Status: Draft
- Area: runtime identity, workspace model, event sinks, generated tests
- Target milestone: v0.4
- Depends on: RFC 0001, RFC 0002, RFC 0005
- Workspace identity implemented by: RFC 0007

## Summary

This RFC proposes optional maturity features around the core command framework: runtime identity, workspace identity, framework events, and generated contract tests. A small stdio server can use the core framework alone. A long-lived, hot-reloaded, project-scoped, or larger server can opt into shared vocabulary for identity, workspaces, events, and coverage.

The proposal makes these features reusable framework patterns. Repository tools, issue tracker tools, and deployment tools can share the same way to identify the running server, describe roots, record planning events, and prove that help, resources, effect-lane tools, examples, and dispatch agree.

## Motivation

The first milestone proves that a compact command-shaped MCP surface can work. The next failures appear when the surface matures. A client may keep a connection open while the server binary is replaced. A path argument may be technically inside a string prefix but outside the user's intended workspace. A command may appear in help but lack a handler. An example may stop planning after a refactor. A failed invocation may disappear before the user or test suite can inspect why it failed.

These problems share the framework's shape: there is a catalog, a runtime serving that catalog, workspaces declared by MCP roots or server configuration, events produced by planning and dispatch, and tests that should prove the public contract remains coherent. The framework should provide common abstractions so product code can focus on domain behavior.

The features remain opt-in. A simple in-process MCP server can define a catalog, register handlers, and run through the core adapter. A mature server can add runtime identity, workspace identity, event sinks, and generated contract coverage as its operational needs grow.

## Guide-Level Explanation

Runtime identity tells a client or diagnostic log which server instance is answering calls and which command contract it is serving. It can include the server name and version, catalog hash, schema hashes, process id, startup time, and replacement status when the host can detect those facts. Hosts use this shared vocabulary when identity matters.

Workspace identity gives path-typed arguments a meaningful context. RFC 0007's `mcp-workspace-resolver` crate now owns this model: workspace requirements, observations from MCP roots and sandbox metadata, authority-order resolution, and structured diagnostics. The resolver preserves where a workspace came from, what it permits, and why it cannot be used. That lets diagnostics say more than "path denied"; they can say which workspace was expected, which root was active, and which policy failed.

Framework events record what the framework observed while parsing, planning, authorizing, dispatching, and shaping output. The default event sink is no-op, and servers that need inspection can record events in memory, JSONL, SQLite, or another backend. The event contract gives servers shared event structure while leaving storage choices to each server.

Generated contract tests turn the catalog into executable coverage. A server should be able to ask the framework for tests that prove every catalog operation appears in discovery, every example plans, every effect-lane tool has truthful metadata, and every declared command can reach a dry-run plan. These tests are maturity infrastructure: they are how a growing command surface stays coherent.

### How Agents Should Learn This

Agents should encounter runtime and workspace maturity through resources and diagnostics. Ordinary commands keep their command-shaped flow. A server overview can expose the catalog hash and active workspace set. A workspace resource can explain whether roots came from MCP or server config. A diagnostic can point to the workspace identity involved in a failed path argument.

Framework diagnostics should use shared runtime and workspace terms. If the command failed because the connected server is stale, the diagnostic should say that in framework language. If the path is outside a declared root, the diagnostic should name the root and the expected workspace. If generated contract tests fail in development, the failure should name the catalog operation and projection that disagree.

## Reference-Level Explanation

`RuntimeIdentity` describes the running server instance and the command contract it is currently serving. Fields that cannot be known in a particular host are optional; the framework must not require executable hashing, process ids, or replacement detection from all transports.

```rust
pub struct RuntimeIdentity {
    pub server_name: String,
    pub server_version: Option<String>,
    pub catalog_hash: String,
    pub run_schema_hash: String,
    pub help_schema_hash: String,
    pub executable_hash: Option<String>,
    pub process_id: Option<u32>,
    pub started_at: Option<Timestamp>,
    pub replacement: Option<ReplacementStatus>,
}
```

Runtime identity should be available to diagnostics, resources, and generated tests. It may also be used by runtime hosts to decide whether a pure or idempotent call can be retried after replacement. Retry policy must remain effect-aware: writes, deletes, process execution, and network calls are not retried after ambiguous failure unless the handler declares an idempotency key.

Workspace identity is provided by RFC 0007. The `ResolvedWorkspaceSet` produced by `mcp-workspace-resolver` carries provenance (which observation source satisfied each requirement) and structured diagnostics, and Twill's planner, adapter, and permission preview already consume it. This RFC originally proposed a separate `WorkspaceIdentity` struct with `source`, `capabilities`, and `diagnostics` fields; the resolver's requirement/observation/resolution vocabulary superseded that design. The resolver already models workspace capabilities (`WorkspaceCapabilities` with read-only and read-write states, carried on declared and resolved roots); the remaining work is enforcement at dispatch time and exposure in Twill's authoring surface, not a new capability type.

The resolver already enforces the access rule this RFC required: when a client observation source is present, server-declared workspaces must not silently widen filesystem access. Presence blocks fall-through, including when the client reports an empty root set.

`FrameworkEvent` records framework observations. Events are not a substitute for handler logs; they are the framework's account of a call's lifecycle and contract checks.

```rust
pub struct FrameworkEvent {
    pub id: String,
    pub timestamp: Timestamp,
    pub runtime: Option<RuntimeIdentity>,
    pub operation_id: Option<String>,
    pub command: Option<String>,
    pub status: ResponseStatus,
    pub effects: Vec<EffectSpec>,
    pub diagnostics: Vec<Diagnostic>,
}

pub trait EventSink {
    fn record(&self, event: FrameworkEvent);
}
```

The default sink is no-op. The core crate must also provide an in-memory sink, which tests and development inspection need regardless of storage choices. Persistent sinks (JSONL, SQLite, or another backend) remain server-owned; their storage format is outside the core contract.

Generated contract tests accept a catalog and a test server or fixture harness. They should verify discovery, planning, examples, resources, prompts, effect-lane metadata, and output projection. The tests should fail with catalog operation ids and projection names so authors can repair the source of drift.

The first implementation ships this as per-rule check functions plus a `contract_tests!` declarative macro. Each check returns `ContractViolation` values naming the operation id and drifted projection; the macro expands to one `#[test]` per rule over a registry constructor, so each contract rule reports as an individual test result. The check functions remain public so authors who want custom aggregation can call them directly.

### Required Contract Coverage

- Every catalog operation appears in command resources and command help.
- Every required argument appears in generated help and schema projections.
- Every example parses, binds, and plans.
- Every operation can produce a dry-run plan.
- Every operation has an effect classification and permission metadata.
- Every required effect lane from RFC 0005 appears as an MCP tool.
- No unused effect-lane tool is generated.
- Tool annotations match the worst-case truthful behavior of each lane.
- Response profiles obey structured-content rules from RFC 0002.
- Task support declarations match negotiated MCP capabilities.

### Implementation Phases

1. Add catalog-level generated contract test helpers. *(Shipped: the `contract` module and `contract_tests!` macro.)*
2. Workspace identity with provenance and diagnostics. *(Shipped via RFC 0007: the `mcp-workspace-resolver` crate and its Twill integration.)*
3. Add workspace diagnostics to planning failures and workspace resources. *(Diagnostics shipped via RFC 0007: `WorkspaceMismatch` carries resolver diagnostics, and previews show selected roots. A dedicated workspace resource is not yet implemented; only invocation plans and previews expose selected roots today.)*
4. Add `FrameworkEvent` and `EventSink` with a no-op default and an in-memory sink.
5. Add runtime identity types without requiring a runtime host.
6. Add a runtime host as a sibling crate after the core contracts stabilize. The first slice covers identity construction (process id, start time) and effect-aware retry policy; hot-replacement detection is an explicit follow-up.
7. Extend the example server coverage to include events, runtime identity, and generated effect-lane tests.

### Acceptance Tests

- The example server passes generated catalog coverage.
- A catalog command missing generated help fails coverage.
- A command example with an unknown argument fails coverage.
- A workspace path outside roots produces a structured diagnostic. *(Shipped via RFC 0007.)*
- With MCP roots available, server config cannot silently widen path access. *(Shipped via RFC 0007.)*
- Event sinks record planning failures and successful dispatch.
- Runtime identity includes catalog and schema hashes when available.
- Non-runtime servers compile and run without host configuration.

## Drawbacks

The maturity layer risks making the framework look heavier than it is. Runtime identity, event sinks, and generated contract tests can sound like infrastructure requirements even when they are optional. Documentation and APIs should keep the simple path visible: a small server can define a catalog, register handlers, and run without a host or persistent store.

Generated tests also create another surface that must be maintained. If the framework's generated coverage is too rigid, servers may fight it instead of benefiting from it. The tests should enforce framework invariants and leave product-specific policy to server-owned tests.

## Rationale And Alternatives

One alternative is to leave runtime identity and event recording entirely to server authors. That avoids framework scope, but it guarantees inconsistent diagnostics and makes it harder to write reusable tests or clients. The framework does not need to own storage or supervision to define common identity and event shapes.

Another alternative is to make a runtime host mandatory. That would simplify some lifecycle behavior, but it would violate the framework's goal of supporting simple stdio servers. Optional maturity layers preserve the low-friction path while giving larger servers a consistent upgrade path.

A third alternative is to make contract tests handwritten. Handwritten tests remain important for product behavior, but they are a poor way to enforce generic catalog invariants. Generated tests can cover the repetitive framework promises that every server should satisfy.

## Prior Art

MCP roots provide a protocol-level way for clients to declare workspace context. This RFC uses roots as the preferred source of workspace identity and makes server configuration an explicit fallback rather than a silent expansion of filesystem scope.

Runtime identity and health endpoints are common in service frameworks, while generated contract tests are common in schema-driven systems. This proposal adapts those patterns to MCP command servers without requiring a service host.

Rust and Ember both use testable contracts around public APIs and documentation. The same idea applies here: a command catalog is only useful if the generated public surfaces stay synchronized with it.

## Unresolved Questions

All previously unresolved questions have been settled:

- The runtime host lives in a sibling crate, not behind a crate feature. A feature flag would make the core crate's dependency tree and compile surface vary by configuration; a sibling crate keeps the simple path visibly simple and lets the host evolve on its own release cadence.
- The core framework ships the `EventSink` trait, the no-op default, and an in-memory sink. Persistent sinks are server-owned.
- Workspace identity does not include structured VCS metadata fields. RFC 0007's Codex derivation reads VCS markers (`.git`, `.jj`, `.hg`) to locate workspace roots, and the marker name can appear in a resolved root's human-readable selection reason, but no VCS facts are modeled as identity fields; servers that need richer VCS data own that themselves.

## Future Possibilities

Runtime identity could support hot replacement workflows, stale-client diagnostics, and client-side cache invalidation by catalog hash. Workspace identity could support richer path capabilities, such as read-only mounts, generated artifacts, or named output directories.

Framework events could later feed developer tooling, trace viewers, or conformance reports. Generated contract tests could evolve into compatibility reports between catalog versions, making it easier for server authors to understand whether a command surface change is breaking.
