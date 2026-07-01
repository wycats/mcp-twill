<!-- exo:6 ulid:01kwfys9ar0d6kat4g8drsskj5 -->

# RFC 0006: Author Ergonomics

- Status: Draft
- Area: Rust authoring API, catalog builders, typed handlers, generated examples
- Target milestone: v0.4
- Depends on: RFC 0001, RFC 0002, RFC 0005

## Summary

This RFC proposes an authoring layer that makes a small MCP Twill server easy to write while preserving the catalog as the public command contract. Server authors should be able to describe a command, its typed arguments, its effects, its examples, and its handler in one local piece of Rust code. The framework should turn that authoring form into the same `CommandCatalog`, help, resources, prompts, diagnostics, effect lanes, invocation plans, and tests defined by the earlier RFCs.

The ergonomic layer is a builder API over the existing model. It gives common concepts short, intention-revealing names: string arguments, path arguments, read and write permissions, examples, workspace declarations, output contracts, and handler registration. It also introduces typed argument extraction so a handler can work with Rust values rather than manually reading `context.plan.bound_args`.

The existing `CommandRegistry::register(CommandSpec, handler)` API remains valid. The new API provides a path that feels natural for authoring servers first and catalog records second. Both paths produce the same catalog model and pass through the same planner, diagnostics, lane routing, permission policy, output shaping, progress, and task behavior.

## Motivation

The foundation model is now expressive enough to represent the public contract of a server, but writing that contract directly is more mechanical than the framework's purpose suggests. A server author currently has to build `CommandSpec`, `ArgSpec`, `PermissionSpec`, examples, workspaces, and handlers as separate pieces, then trust that the resulting code still reads like the command surface the agent will see.

MCP Twill should make the correct server shape the easy shape. The author should write code that looks like a catalog entry with a handler attached. The command declaration should put the operation path, argument names, permission effects, examples, and output contract near the code that implements the operation. That locality makes drift easier to see during review and gives the framework enough structure to generate coherent help, resources, prompts, and tests.

This layer also improves the framework's teaching story. The example server should demonstrate the intended authoring habit in a few commands. A new author should be able to copy that pattern and produce a server whose MCP surface is already consistent, annotated, documented, and testable.

## Guide-Level Explanation

A server author starts with a server builder. The builder names the server, describes its purpose, declares workspaces, and adds commands. Each command is described in command-shaped prose and then connected to a native Rust handler.

The ordinary authoring flow keeps a command together. The operation path appears first because it is the phrase agents use in the command string. The summary and description explain the command. Argument declarations name the `$args` values the command may bind. Permission helpers describe the effect lane and permission text. Examples show the command template that agents should use. The handler receives typed values extracted from the request after planning succeeds.

```rust
let registry = CommandRegistry::build(
    "issues-example",
    "Example MCP Twill server for issue tracking commands.",
    |server| {
        server.workspace(WorkspaceDecl::file("repo", "C:/workspace"));

        server.command("issues create", |command| {
            command
                .summary("Create an issue")
                .description("Creates a new issue from typed title and body arguments.")
                .arg(arg::string("title").summary("Issue title"))
                .arg(arg::string("body").summary("Issue body"))
                .write("issues", "Creates a new issue record")
                .example(
                    "issues create --title $args.title --body $args.body",
                    "Create an issue with typed title and body values",
                )
                .handle(create_issue);
        });
    },
)?;
```

Typed handlers make the implementation side of the command feel like ordinary Rust. A command that declares `title` and `body` can receive a `CreateIssueArgs` value. Extraction happens after the framework has parsed the command template, checked placeholders, validated argument types, checked workspace constraints, and built an invocation plan. Extraction failures become the same structured diagnostics as other framework-level input failures.

```rust
#[derive(serde::Deserialize)]
struct CreateIssueArgs {
    title: String,
    body: String,
}

async fn create_issue(
    ctx: CommandContext,
    args: CreateIssueArgs,
) -> Result<CommandOutput> {
    Ok(CommandOutput::structured(json!({
        "title": args.title,
        "status": "open",
        "operation": ctx.plan.operation_id,
    })))
}
```

The builder API should make common permissions and workspaces concise. A read command should not require a hand-written `PermissionSpec::new(PermissionEffect::Read, ...)` at every call site. A path argument should make the workspace relationship visible. The fluent form remains explicit about the public contract, but it removes ceremony that does not add design information.

The ergonomic layer should also make examples useful by construction. An example builder should accept the command template and the typed example values together. The framework should validate those examples through the planner and should expose them through help and resources. A server author should experience examples as executable documentation for agents.

### How Agents Should Learn This

Agents learn this proposal in two roles: as clients operating a Twill server and as coding assistants helping a user write one. The generated MCP surface should still teach the runtime model: inspect help, start execution with the primary tool, use typed `$args.*` placeholders, and follow structured retry data when effect-lane escalation is required. That runtime teaching should be identical whether the server was authored through explicit specs or ergonomic builders.

When an agent helps write a Twill server, it should use the builder API as the ordinary authoring path. The agent should keep each command declaration local: path, summary, description, typed args, workspace relationships, permission helpers, examples, output contract, and handler should appear together unless the codebase has a stronger local pattern. The agent should treat that declaration as the source of the server's public behavior and should add or update examples and contract tests in the same change that adds or changes a command.

The authoring guidance should steer agents toward catalog-preserving edits. If a handler needs a new input, the agent should add an `arg` declaration, update examples, and use typed extraction. If a command's effect changes, the agent should update the permission helper so effect-lane tools and MCP annotations are regenerated from the catalog. If a command needs filesystem access, the agent should declare or reuse a workspace and make the argument path-typed. The agent should not smuggle public behavior into handler-local parsing, raw JSON inspection, or shell command strings.

The ergonomic API should improve generated surfaces by making good catalog content easier to provide. Builder methods such as `summary`, `description`, `arg`, `example`, and `write` should feed the exact text that appears in help, resources, prompts, diagnostics, and generated tests. When an author omits content that agents need, the builder should produce an actionable construction error or a generated contract-test failure that tells both humans and coding agents where to repair the catalog declaration.

Examples should be the primary teaching artifact for both roles. Runtime agents should see examples that pair the command template with structured argument data. Coding agents should create examples in that same paired form and should preserve them when refactoring handlers. Permission helpers should generate plain language that explains the effect without teaching runtime agents to choose effect-lane tools proactively.

## Reference-Level Explanation

The ergonomic layer is an API for constructing the existing catalog and registry types. It must not introduce a second public command model. The output of the builder is a `CommandRegistry` with registered `CommandSpec` values and handlers. The catalog projected from that registry must be indistinguishable from one produced by explicit specs with the same fields.

```rust
impl CommandRegistry {
    pub fn build<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        build: F,
    ) -> Result<Self>
    where
        F: FnOnce(&mut ServerBuilder) -> Result<()>;
}

pub struct ServerBuilder {
    // private
}

impl ServerBuilder {
    pub fn workspace(&mut self, workspace: WorkspaceDecl) -> &mut Self;
    pub fn command<F>(&mut self, path: impl IntoCommandPath, build: F) -> Result<&mut Self>
    where
        F: FnOnce(&mut CommandBuilder) -> Result<()>;
}
```

`CommandBuilder` accumulates one command declaration. It must require a command path, summary, description, and handler before the server can build. It should supply conservative defaults for stability, output contract, task support, and progress phases while leaving the underlying catalog fields visible for authors who need them.

```rust
pub struct CommandBuilder {
    // private
}

impl CommandBuilder {
    pub fn summary(&mut self, summary: impl Into<String>) -> &mut Self;
    pub fn description(&mut self, description: impl Into<String>) -> &mut Self;
    pub fn arg(&mut self, arg: ArgBuilder) -> &mut Self;
    pub fn read(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self;
    pub fn write(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self;
    pub fn delete(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self;
    pub fn exec(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self;
    pub fn network(&mut self, scope: impl Into<String>, description: impl Into<String>) -> &mut Self;
    pub fn example(
        &mut self,
        command: impl Into<String>,
        summary: impl Into<String>,
    ) -> &mut Self;
    pub fn output(&mut self, output: OutputContract) -> &mut Self;
    pub fn handle<H>(&mut self, handler: H) -> &mut Self
    where
        H: CommandHandler;
}
```

Argument builders construct `ArgSpec` values. The common helpers should cover strings, paths, booleans, numbers, and JSON. Path arguments should require a workspace name at construction time. Optional arguments, repeated arguments, aliases, defaults, and examples should be available through additional builder methods.

```rust
pub mod arg {
    pub fn string(name: impl Into<String>) -> ArgBuilder;
    pub fn path(name: impl Into<String>, workspace: impl Into<String>) -> ArgBuilder;
    pub fn boolean(name: impl Into<String>) -> ArgBuilder;
    pub fn number(name: impl Into<String>) -> ArgBuilder;
    pub fn json(name: impl Into<String>) -> ArgBuilder;
}

pub struct ArgBuilder {
    // private
}

impl ArgBuilder {
    pub fn summary(self, summary: impl Into<String>) -> Self;
    pub fn optional(self) -> Self;
    pub fn repeated(self) -> Self;
    pub fn default(self, value: serde_json::Value) -> Self;
    pub fn example(self, value: serde_json::Value) -> Self;
}
```

Typed extraction should be an adapter over the existing `CommandContext`. The framework can provide a trait for extracting values from the planned invocation. Implementations may use `serde::Deserialize` over the bound argument map. The first implementation should support explicit extraction and a typed handler adapter without requiring a procedural macro.

```rust
pub trait FromCommandArgs: Sized {
    fn from_command_args(ctx: &CommandContext) -> Result<Self>;
}

impl<T> FromCommandArgs for T
where
    T: serde::de::DeserializeOwned,
{
    fn from_command_args(ctx: &CommandContext) -> Result<Self> {
        // Deserialize from ctx.plan.bound_args values.
    }
}

pub trait TypedCommandHandler<A>: Send + Sync + 'static {
    async fn call_typed(&self, ctx: CommandContext, args: A) -> Result<CommandOutput>;
}
```

The ergonomic layer should generate construction diagnostics before the MCP server starts. Missing summaries, duplicate command paths, duplicate argument names, path arguments that reference undeclared workspaces, examples that do not plan, commands without handlers, and handlers whose typed extraction cannot be satisfied by the declared args should produce framework construction errors. These errors are developer-facing; runtime request errors still use the response envelope from RFC 0002.

```rust
pub enum BuildError {
    MissingCommandSummary { path: Vec<String> },
    MissingCommandDescription { path: Vec<String> },
    MissingHandler { path: Vec<String> },
    DuplicateCommand { path: Vec<String> },
    DuplicateArgument { path: Vec<String>, arg: String },
    UnknownWorkspace { path: Vec<String>, arg: String, workspace: String },
    InvalidExample { path: Vec<String>, source: FrameworkError },
}
```

The existing explicit API remains supported. `CommandSpec` and `CommandRegistry::register` are still the escape hatch for generated servers, advanced integrations, or code that already has specs. The ergonomic builder should use the same validation path as explicit specs so behavior stays aligned.

### Required Invariants

- Ergonomic builders produce the same catalog model as explicit specs.
- A built command has a path, summary, description, and handler.
- Duplicate command paths are rejected before server startup.
- Duplicate argument names are rejected before server startup.
- Path arguments reference declared workspaces.
- Examples added through builders parse and plan.
- Typed extraction reads from planned bound arguments, not from the raw command string.
- Typed extraction failures become structured framework errors when they occur at runtime.
- Existing `CommandRegistry::register(CommandSpec, handler)` behavior remains compatible.

### Implementation Phases

1. Add `ServerBuilder`, `CommandBuilder`, and `ArgBuilder` as a builder layer over `CommandRegistry`.
2. Add permission helper methods for read, write, delete, exec, and network effects.
3. Add workspace helper constructors for file workspaces and path arguments.
4. Add builder-time validation and construction errors.
5. Add example builders that carry both command templates and typed example args.
6. Add typed argument extraction from `CommandContext`.
7. Add typed handler adapters while preserving the existing `CommandHandler` trait.
8. Rewrite the example server to use the ergonomic API.
9. Add generated contract tests that compare builder output to equivalent explicit specs.

### Acceptance Tests

- A small issue-tracker server can be written through the builder API and exposes the expected tools, resources, prompts, lanes, and help.
- The builder output has the same catalog identity as equivalent explicit `CommandSpec` registration.
- A typed handler receives deserialized argument values after planning succeeds.
- Missing required typed values produce the same structured diagnostics as explicit specs.
- A path argument helper enforces the declared workspace.
- Permission helpers produce the expected effect lane tools and annotations.
- Examples added through the builder are validated by the planner.
- A command missing a handler fails during registry construction.
- Duplicate command paths and duplicate argument names fail during registry construction.
- Existing explicit registration tests continue to pass unchanged.

## Drawbacks

This proposal adds another layer to the public Rust API. The framework will need to document both the explicit model and the ergonomic model, and users may wonder which one is the preferred path. The documentation should answer that directly: builders are the ordinary authoring path, while explicit specs are the lower-level model and compatibility path.

Typed extraction also adds a new failure surface. A command can have a valid catalog declaration while its Rust argument type fails to deserialize at runtime. The builder should catch the cases it can catch from declared argument names and types, but the runtime still needs clear diagnostics for serde-level failures.

Builder APIs can become too magical if they infer public command behavior from Rust signatures alone. This RFC keeps summaries, descriptions, examples, permissions, and workspaces explicit because those details teach agents and cannot be recovered reliably from type signatures.

## Rationale And Alternatives

Keeping only the current explicit API is the simplest implementation path. It is a good foundation, but it asks every server author to work at the catalog data-structure level. The builder layer is proposed because server authoring should read like declaring the command surface the agent will use.

A macro-first API could produce very compact code. That option is attractive for examples, but it makes the first ergonomic milestone depend on procedural macro design, attribute syntax, and generated-code diagnostics. This RFC prefers a regular Rust builder first because it is debuggable, composable, and easy to evolve. A macro can be added later as syntax over the same builders.

Inferring the entire catalog from handler function signatures is also attractive. It is rejected as the core model because command help, permissions, examples, workspace policy, and agent steering require prose and policy decisions that Rust signatures do not contain. Typed signatures should help handlers receive values; they should not replace the catalog contract.

A configuration-file approach could make command catalogs language-neutral. That may become useful for generated servers, but it would make the Rust-native framework less ergonomic for the immediate goal. The Rust API should be the first-class authoring surface.

## Prior Art

`clap` demonstrates how a Rust builder or derive API can make command definitions readable while still producing structured command metadata. MCP Twill should borrow the locality of command declaration, but the result is an MCP catalog and planner rather than a process-level CLI parser.

`axum` extractors show how typed handler arguments can make request handling feel direct while a framework performs validation and extraction around the handler. Twill can use the same idea for command args: handlers should receive typed values after the framework has planned and validated the request.

`serde` and `schemars` provide precedent for deriving structured data behavior from Rust types. Twill should use them for typed extraction and schemas where they fit, while keeping command summaries, permissions, workspaces, and examples explicit.

`rmcp`'s tool macros provide convenient Rust authoring for ordinary MCP tools. Twill's ergonomic layer serves a different shape: one command catalog projected through a small MCP tool surface. The authoring API should feel as convenient as a macro-generated tool server while preserving the catalog-first model.

## Unresolved Questions

- Should the first typed handler adapter require `DeserializeOwned`, or should it use a custom derive that can validate arg names at build time?
- Should builder methods accept documentation attributes from Rust doc comments in a later macro layer?
- Should ergonomic examples require explicit example args, or should examples with no placeholders be accepted without args?
- Should builder-time validation return one accumulated error report or fail on the first construction error?
- Should the framework provide a test macro that snapshots catalog identity and generated help?

## Future Possibilities

A later macro layer could make common command declarations even shorter while expanding to the builder API. That macro could read doc comments for summaries, derive arg specs from typed structs, and generate example validation tests.

The builder model could also support generated documentation sites, editor completions for command templates, and server templates for common domains such as repositories, issue trackers, package managers, and deployment systems.

Once preview and replay land, the ergonomic layer can add author-facing helpers for preview text, confirmation copy, replay policy, and event sink metadata. Those helpers should continue to project into the catalog and invocation plan rather than becoming handler-local side channels.
