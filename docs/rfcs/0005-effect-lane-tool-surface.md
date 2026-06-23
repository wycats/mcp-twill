# RFC 0005: Effect-Lane Tool Surface

- Status: Draft
- Area: MCP tool surface, tool annotations, effect routing, agent steering
- Target milestone: v0.3
- Depends on: RFC 0001, RFC 0002

## Summary

This RFC defines effect-lane execution tools for one command catalog. A framework server exposes one command catalog and one discovery surface, and execution may be represented by a small family of MCP tools so each tool can carry truthful MCP annotations for the effects it may perform.

The server-chosen base name, written here as `{name}`, is the primary execution tool. Agents are instructed to call it first. Additional tools such as `{name}-write`, `{name}-delete`, `{name}-exec`, or `{name}-network` are generated only when the catalog contains commands that require those lanes. Every execution tool accepts the same run request shape and uses the same command catalog. Every execution tool can parse and plan every command, but it only dispatches commands whose required effect lane is allowed by that tool.

When a command reaches the wrong lane, the framework returns a tool execution error, not an MCP protocol error. The error names the required tool and includes structured retry data containing the original request. MCP annotations remain advisory hints for clients; framework permissions and dispatch checks remain the actual enforcement mechanism.

## Motivation

Effect lanes give one command catalog a small set of execution tools with truthful MCP annotations. MCP tool annotations describe a tool's possible behavior, so commands with different effect profiles need tool surfaces whose metadata matches the commands they may dispatch.

The agent still learns one catalog and starts with one primary tool. The server exposes additional MCP tools only where annotations need to differ. The framework decides whether a planned command belongs in another lane and returns an exact retry instruction when escalation is needed.

## Guide-Level Explanation

A server chooses a base execution tool name such as `repo`, `issues`, or `deploy`. That base tool is the primary entry point for execution. Its description tells agents to start there for all commands in the catalog. The server also exposes `help` or an equivalent discovery surface generated from the catalog.

If the catalog contains only low-risk commands that the primary tool can truthfully execute under one annotation profile, no escalated tools are generated. If the catalog contains writes, destructive deletes, external process execution, or network operations that require different annotations, the framework generates lane tools with predictable suffixes. The exact set of generated tools is catalog-driven.

All execution tools accept the same run request. Calling `repo-write` uses the same command grammar and the same command namespace as `repo`. It is the same catalog reached through a tool whose MCP annotations truthfully describe the write lane.

The normal agent flow is intentionally simple. The agent calls `repo` first. If the command is a low-risk command, it runs or proceeds to framework-level permission handling. If the command requires the write lane, the response says that `repo-write` is required and includes a retry object. The agent then calls `repo-write` with the supplied arguments. If policy requires confirmation, RFC 0003 handles preview, confirmation, and replay after the command has reached the correct lane.

### How Agents Should Learn This

Tool descriptions, prompts, and help resources should teach one rule: start with `{name}` and use escalated tools when the framework explicitly asks for them. The steering language should be direct and non-apologetic. A wrong-lane response is the framework performing effect routing with better catalog knowledge than the agent has before planning.

Escalated tool descriptions should say that the tool is normally reached by structured retry from `{name}`. The retry data should preserve the command request exactly, so escalation carries the same typed request into the named lane.

## Reference-Level Explanation

The catalog assigns each operation a required effect lane. A lane is the MCP-facing execution category used to choose tool annotations and dispatch eligibility. It is derived from the operation's effect classification, permission metadata, and any catalog policy that refines how effects map to lanes.

```rust
pub enum EffectLane {
    Primary,
    Write,
    Delete,
    Exec,
    Network,
    Custom(String),
}

pub struct ToolLaneSpec {
    pub tool_name: String,
    pub lane: EffectLane,
    pub allowed_effects: EffectSet,
    pub annotations: ToolAnnotationsSpec,
    pub description: String,
}
```

The primary tool is not required to be read-only by name. It is annotated as the lowest-risk lane it can truthfully execute. A catalog that has pure and read commands may allow those commands through `{name}` and annotate it accordingly. A catalog with only write commands may still have a primary tool, but its annotations must truthfully describe the lowest-risk lane it can execute, which may be a write lane.

Escalated tools use worst-case truthful annotations for the commands they may dispatch. If every command in a lane is non-destructive, the lane should not claim destructive behavior. If any command in a lane is destructive, the lane must be annotated accordingly. `idempotentHint` should be present only when every dispatchable command in that lane is idempotent under the catalog's definition. `openWorldHint` should describe whether the lane may interact with external systems.

Tool annotations are not permissions. They are advisory MCP metadata used by clients for planning and confirmation UX. The framework must still enforce lane eligibility, workspace policy, permission policy, confirmation, and replay internally.

Every execution tool accepts the same request shape used by `run`.

```rust
pub struct RunRequest {
    pub command: String,
    pub args: serde_json::Map<String, serde_json::Value>,
    pub stdin: Option<StdinInput>,
    pub output: Option<OutputSpec>,
    pub dry_run: bool,
    pub mode: Option<RunMode>,
    pub approval: Option<ApprovalInput>,
}
```

On every execution call, the framework parses the command template, binds typed arguments, resolves the catalog operation, validates workspaces, and builds an invocation plan. This planning step happens even when the current tool is not allowed to dispatch the command, because the framework needs the plan to know which lane is required. Dispatch happens only after the lane check succeeds.

Wrong-lane results are tool execution errors. They should be projected through the response envelope from RFC 0002 and include a stable error code plus retry data. The retry arguments are the same request after normal framework normalization; they must not rewrite the command into shell syntax or drop fields that were present in the original request.

```json
{
  "code": "wrong_effect_lane",
  "currentTool": "repo",
  "requiredTool": "repo-write",
  "message": "This command requires repo-write.",
  "retry": {
    "tool": "repo-write",
    "arguments": { "...": "same request" }
  }
}
```

The framework exposes only lanes required by the catalog. If no command requires delete, `{name}-delete` is not listed. Generated prompts and resources should describe the available lanes as part of the server overview, but they should still steer agents to start with the primary tool.

### Required Invariants

- All execution tools use the same command catalog.
- All execution tools accept the same run request shape.
- All execution tools can parse and plan every command in the catalog.
- A tool dispatches only commands allowed by its lane.
- Wrong-lane calls never dispatch handlers.
- Wrong-lane responses are tool execution errors, not MCP protocol errors.
- Wrong-lane responses include the exact required tool and structured retry arguments.
- The server exposes only effect-lane tools required by the catalog.
- Tool annotations are truthful worst-case hints for the lane, not enforcement.

### Implementation Phases

1. Add effect-lane classification to catalog operations and invocation plans.
2. Add lane-to-tool generation with primary and suffix naming.
3. Generate MCP tool annotations from lane specs.
4. Route all execution tools through the same parser, binder, planner, and output shaper.
5. Add wrong-lane response projection through RFC 0002.
6. Update prompts, help, and resources to teach primary-tool-first usage.
7. Extend generated contract tests to cover lane exposure and annotations.

### Acceptance Tests

- The primary tool dispatches commands allowed by its lane.
- The primary tool redirects a write command to `{name}-write` when write is outside the primary lane.
- An escalated tool dispatches commands allowed by its lane.
- An escalated tool rejects commands that belong to another lane.
- Wrong-lane calls never invoke handlers.
- Every wrong-lane response includes `code`, `currentTool`, `requiredTool`, `message`, and `retry`.
- Retry data preserves the original command request without shell rewriting.
- Generated MCP tools expose expected annotations.
- Only catalog-required lanes are listed as MCP tools.
- Tool descriptions consistently steer agents to start with `{name}`.
- A representative agent flow starts with `{name}`, escalates only after a wrong-lane response, and preserves the request.

## Drawbacks

This proposal adds more than one execution tool, which weakens the original simplicity of an exact two-tool surface. The framework must keep that complexity contained. If every server casually adds many custom lanes, agents will experience the tool surface as noisy even though the catalog is shared.

The primary-tool-first rule also depends on careful wording. If descriptions imply that agents should choose lanes proactively, the design will feel like a set of papercuts. If descriptions hide the existence of lanes entirely, wrong-lane responses may feel surprising. The generated language needs to explain that lanes are effect routing for MCP metadata, while the primary tool remains the starting point.

## Rationale And Alternatives

Keeping a single execution tool is the simplest model. It is rejected here because MCP annotations apply at the tool level. A single broad tool must use broad worst-case annotations, which removes useful client signals for low-risk commands.

Exposing one tool per command gives the most precise annotation metadata. It is rejected because it moves the framework away from a command catalog and toward a large tool list. Agents are effective with command-shaped interfaces, and this project should preserve that advantage.

Requiring agents to call the correct escalated tool first is also rejected. The catalog knows the required lane only after parsing and planning. Letting the framework redirect from the primary tool is more reliable and keeps the ordinary agent strategy stable.

Using MCP annotations as access control is rejected outright. Annotations are hints supplied by servers and consumed by clients. The framework must enforce dispatch eligibility and permissions through its own planning and authorization path.

## Prior Art

MCP `ToolAnnotations` define advisory fields such as read-only, destructive, idempotent, and open-world hints. This RFC uses those hints at the effect-lane level so a compact command catalog can still expose truthful tool metadata.

Command-line tools often group subcommands by risk or require explicit flags for destructive operations. This proposal borrows the idea of visible escalation but moves it into MCP tool metadata and structured retry instead of shell flags.

Capability-based systems distinguish naming a capability from exercising it. The effect-lane tool name gives the client a coarse capability signal, while the framework's permission and replay layers decide whether a particular planned invocation may run.

## Unresolved Questions

- Should the primary lane be named `Primary` internally, or should it always map to a concrete effect such as `Read`?
- Should `Write` and `Delete` always be separate lanes, or should catalogs be allowed to merge them when annotations are identical?
- Should custom lanes be allowed in v1, or should the first implementation restrict lanes to the standard set?
- How should task-capable commands report lane redirects when the client also negotiated task support?

## Future Possibilities

Effect lanes could support dynamic tool-list changes if a catalog is loaded or replaced at runtime. If MCP clients broadly support tool-list change notifications, a runtime host could expose new lanes only when a server activates commands that require them.

Future versions may refine lane annotations with client-specific policy, richer idempotency declarations, or per-command output schemas. Those extensions should keep the same agent rule: start with the primary tool, then follow structured framework redirects when escalation is required.
