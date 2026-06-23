# RFC 0001: Authoritative Command Catalog

- Status: Draft
- Area: command model, help, resources, prompts, generated projections
- Target milestone: v0.2

## Summary

This RFC proposes `CommandCatalog` as the authoritative public contract for a CLI-shaped MCP server. The catalog describes what the server is for, which commands it supports, which typed arguments those commands accept, what effects they may have, which outputs they produce, and how the framework projects the same contract into help, resources, prompts, schemas, diagnostics, invocation plans, and generated tests.

The proposal makes the catalog the place where public command behavior is declared, explained, planned, and checked. Handler registration connects Rust implementations to catalog operations. Planning proves that each catalog operation can become a concrete invocation. Discovery surfaces, diagnostics, and tests all read from that same contract.

## Motivation

The project exists because agents are good at command-shaped interaction, but raw shell interaction makes correctness depend on quoting, local tools, redirection conventions, and ad hoc help text. A Rust MCP framework can keep the command grammar that agents understand while moving the dangerous and ambiguous parts into typed framework features.

One authoritative command surface lets the agent treat every generated surface as a reliable view of the same command contract. Help, examples, resources, schemas, diagnostics, and dispatch all describe the same operation, so the agent can move from discovery to execution with a single source of truth.

The catalog creates that authority by making every public projection a checked rendering of the same model. Help renders the command contract for humans and agents. Resources provide navigable catalog views. Examples are validated command requests. Generated tests prove that projections, planning, and dispatch stay aligned as the server evolves.

## Guide-Level Explanation

A server author starts by describing the server as a command catalog. The catalog names the server, explains its purpose, declares project values, groups commands into namespaces, and defines each operation in command-shaped terms. The operation path is the phrase an agent writes in the command string, such as `issues create`. The operation id is the stable machine name the framework uses for tests, diagnostics, telemetry, and replay.

An operation declaration says more than "call this Rust function." It describes the arguments that can be bound from `$args`, the stdin contract if the command accepts content, the output contract the handler promises to return, the progress phases a long-running call may report, the effect classification used by permission and effect-lane routing, and the examples that teach the command.

Once the catalog exists, the framework projects it into the MCP server surface. A help call for `issues create` is generated from the same operation that dispatch uses. A command resource for `issues create` contains the same argument descriptions and examples. A getting-started prompt teaches inspection using the same catalog language. A dry-run plan points back to the operation id and catalog hash that produced it.

This proposal keeps the framework honest by requiring deterministic projections. Registration order becomes irrelevant to help output, resource contents, schema hashes, and catalog identity. A server should be able to expose a catalog identity that lets tests and clients say, "these docs, schemas, and handlers describe the same command contract."

### How Agents Should Learn This

Agents should learn a server by starting from the server overview, then using help and command resources to narrow to a command. The prose in those surfaces should describe what the server is for, how command templates work, and which framework values govern examples and diagnostics.

The catalog should make the right path feel obvious. Examples should use `$args.name` placeholders for non-trivial values, structured output controls for selecting fields and limits, and path-typed arguments for filesystem access. If a command resembles a shell command, the catalog should teach the framework form: the command string selects an operation and binds typed placeholders.

## Reference-Level Explanation

`CommandCatalog` is the root model for the public command surface.

```rust
pub struct CommandCatalog {
    pub server: ServerSpec,
    pub namespaces: Vec<NamespaceSpec>,
    pub operations: Vec<OperationSpec>,
}
```

`ServerSpec` describes the server in terms an agent can use before choosing a command.

```rust
pub struct ServerSpec {
    pub name: String,
    pub summary: String,
    pub description: String,
    pub version: Option<String>,
    pub stability: Stability,
    pub values: Vec<ProjectValue>,
}
```

`ProjectValue` is operational guidance. A value such as "command strings are typed templates, not shell programs" belongs in the catalog because it affects how agents form requests and how diagnostics steer them back from invalid syntax.

`NamespaceSpec` gives a stable navigation structure for related operations. A namespace is not dispatchable unless an operation uses that path exactly; it is a documentation and discovery boundary.

```rust
pub struct NamespaceSpec {
    pub path: Vec<String>,
    pub summary: String,
    pub description: Option<String>,
    pub stability: Stability,
}
```

`OperationSpec` describes one dispatchable command. The `id` is stable and machine-oriented. The `path` is the user-facing command path. Display text may change without changing the operation id, but changing the path is a command surface change and should change catalog identity.

```rust
pub struct OperationSpec {
    pub id: String,
    pub path: Vec<String>,
    pub summary: String,
    pub description: String,
    pub effect: EffectSpec,
    pub args: Vec<ArgSpec>,
    pub stdin: Option<StdinContract>,
    pub output: OutputContract,
    pub permissions: Vec<PermissionSpec>,
    pub examples: Vec<CommandExample>,
    pub progress: Vec<ProgressPhaseSpec>,
    pub task_support: TaskSupportSpec,
    pub docs: Vec<DocAnchor>,
    pub stability: Stability,
}
```

`EffectSpec` classifies the externally meaningful behavior of an operation. The effect is used by permission preview, confirmation, response profiles, effect-lane tool routing, retry policy, and generated contract tests. A custom effect may be used, but it must carry enough catalog documentation for permissions and diagnostics to explain it.

```rust
pub enum EffectSpec {
    Pure,
    Read,
    Write,
    Delete,
    Exec,
    Network,
    Composite(Vec<EffectSpec>),
    Custom(String),
}
```

`ArgSpec` describes one structured argument. The canonical `name` is the key in `args`; aliases are conveniences for the command template parser and must not create additional structured argument names.

```rust
pub struct ArgSpec {
    pub name: String,
    pub value_type: ArgType,
    pub required: bool,
    pub repeated: bool,
    pub default: Option<serde_json::Value>,
    pub summary: String,
    pub description: Option<String>,
    pub workspace: Option<String>,
    pub aliases: Vec<String>,
    pub examples: Vec<serde_json::Value>,
}
```

The framework must project the catalog into server overview resources, command catalog resources, per-command resources, permission resources, getting-started prompts, help responses, dry-run invocation plans, input schemas, and generated contract tests. A projection may omit details that do not fit the target surface, but it must not invent command behavior outside the catalog.

```rust
pub struct CatalogIdentity {
    pub catalog_hash: String,
    pub run_schema_hash: String,
    pub help_schema_hash: String,
}
```

Examples and command-shaped guidance are part of the contract. Guidance that is meant to be executed through the framework must parse and plan. Guidance that intentionally describes an external shell command must be explicitly classified as external so agents do not confuse it with framework syntax.

```rust
pub enum GuidanceKind {
    RunCommand,
    HumanAction,
    ExternalShell,
}

pub struct CommandGuidance {
    pub id: String,
    pub surface: String,
    pub text: String,
    pub kind: GuidanceKind,
}
```

### Required Invariants

- Every dispatchable command has exactly one `OperationSpec`.
- Every `OperationSpec` can produce an invocation plan.
- Catalog projection order is deterministic.
- `RunCommand` examples and guidance parse and plan successfully.
- `ExternalShell` guidance is visibly outside the framework command surface.
- Catalog identity changes when command signatures, effects, schemas, or output contracts change.
- Catalog identity does not change when registration order changes.

### Implementation Phases

1. Add catalog types and compatibility conversions from the current command registration types.
2. Move help, resource, prompt, and schema generation to catalog projection functions.
3. Add deterministic sorting and catalog identity hashing.
4. Validate examples and command-shaped guidance during catalog construction or test generation.
5. Update the example server to use catalog-first registration.

### Acceptance Tests

- A multi-namespace catalog generates stable help and resources.
- A registered handler without a catalog operation fails generated coverage.
- A catalog operation without a handler fails generated coverage.
- Every command example parses, binds typed arguments, and plans.
- Guidance marked `RunCommand` compiles through the planner.
- Guidance marked `ExternalShell` is excluded from framework command validation.
- Catalog identity changes when a command signature changes.
- Catalog identity remains stable when registration order changes.

## Drawbacks

The catalog makes simple servers do more up-front design work. A prototype that only wants to register one handler must still describe the command well enough for help, examples, output, effects, and tests. That is intentional, but it raises the minimum level of ceremony compared with an SDK that exposes arbitrary Rust functions as tools.

The catalog can also create pressure to model details before the server author fully understands them. The framework should answer that pressure with conservative defaults and clear draft stability markers, not by allowing public command behavior to remain undocumented.

## Rationale And Alternatives

One alternative is to keep `CommandRegistry` as the only model and generate documentation from handler registrations. That keeps the API smaller, but it makes discovery depend on a type that exists primarily for dispatch. It also makes it too easy to treat help, prompts, output contracts, and examples as optional embellishments. The catalog is proposed because the public contract is broader than dispatch.

Another alternative is to expose one MCP tool per command and rely on MCP tool schemas as the source of truth. That would give each command a native schema, but it would abandon the compact command-shaped surface that this project is exploring. The catalog lets the framework keep a small tool surface while still giving each command a precise model.

A third alternative is to accept handwritten documentation and validate only the examples. That catches some drift, but it does not prevent resources, prompts, permission explanations, and diagnostics from telling different stories. The catalog approach treats every public projection as a checked view of the same command model.

## Prior Art

Rust RFCs distinguish motivation, guide-level explanation, reference-level explanation, drawbacks, alternatives, and future possibilities. That structure is useful here because the framework needs both a teachable agent-facing model and precise implementation rules.

Ember RFCs place unusual weight on detailed design and teaching. That is directly relevant because this framework's names, prompts, examples, and diagnostics are not peripheral documentation; they are how agents learn to operate the server.

The MCP tools, resources, prompts, roots, progress, and schema specifications provide the protocol surfaces the catalog projects into. The catalog is not a replacement for those surfaces. It is the server-side model that keeps them coherent.

## Unresolved Questions

- Should `Pure` and `Read` both remain catalog effects, or should `Pure` be represented as a `Read` operation with stronger idempotency metadata?
- Should handlers be allowed to refine output contracts at runtime, or must the catalog contain the complete output shape?
- Which catalog identity hashes should be exposed through help, resources, runtime health, and diagnostics?

## Future Possibilities

A stable catalog model opens the door to generated SDKs, documentation sites, command diffing, compatibility checks across server versions, and richer editor support for command templates. It could also let clients cache help and schemas by catalog hash, then refresh only when identity changes.

The catalog may eventually support typed composition features that replace common shell idioms. If that happens, those features should extend the catalog and planner rather than introducing shell syntax into the command string.
