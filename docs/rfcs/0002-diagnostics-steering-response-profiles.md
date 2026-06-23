# RFC 0002: Diagnostics, Steering, And Response Profiles

- Status: Draft
- Area: errors, output shaping, help steering, MCP tool results
- Target milestone: v0.2
- Depends on: RFC 0001

## Summary

This RFC defines a framework response envelope for command planning and dispatch. The envelope separates display text from structured diagnostics, steering, replay data, and shaped command output. Its purpose is to let agents correct their own requests without scraping prose and to let MCP clients render concise ordinary results without losing the structured information needed for errors, permissions, tasks, or replay.

The envelope is a framework contract that is projected into MCP tool results. Framework-level planning and dispatch failures return structured tool results. Malformed protocol calls remain MCP-layer failures. A well-formed framework request that names an unknown command, supplies the wrong argument type, uses shell syntax in a command template, or calls the wrong effect-lane tool should return a tool execution result with structured diagnostics.

## Motivation

The framework currently preserves structure internally but can collapse planning failures into display strings. A string such as `unknown argument title` is easy to show to a user, but it is a poor recovery contract for an agent. The agent needs to know whether `title` was a token in the command string, a placeholder, a structured argument key, or an output field. It also needs to know whether anything executed before the error and which next call would be useful.

This distinction matters because command templates express operations through typed placeholders and framework fields. When a request carries shell-like intent such as filtering, redirection, or interpolation, diagnostics should explain the template model and steer toward the typed feature that solves the same problem, such as `output.fields`, `output.limit`, a path argument, or a help call for the command.

The response contract also needs proportionate output. Full debug plans make ordinary successful reads noisy, while errors and follow-up workflows need machine-readable data. Response profiles let the framework keep common successes concise and preserve structure where the caller needs it.

## Guide-Level Explanation

A command request passes through parsing, binding, planning, authorization, dispatch, output shaping, and MCP projection. Each phase can produce diagnostics. A diagnostic names the problem in a stable way, locates the relevant part of the request when possible, and may include suggestions. The human-readable message explains the issue, but the code and location are what agents should rely on.

Steering is explicit next-call data. If the command is unknown, steering may point to `help` for the nearest namespace. If an argument is missing, steering may point to command help. If the command uses shell syntax, steering should point to the framework concepts that replace that syntax. If a request went to the wrong effect-lane tool, steering should identify the required tool and preserve the original request for retry.

Response profiles let the caller choose how much structured output to receive. The default profile is concise. A structured profile returns shaped data according to the output request. A debug profile includes the invocation plan and framework metadata. Errors, permission-required results, task results, and replay steps may include structured content even when the caller requested text, because those flows cannot be completed safely from display text alone.

### How Agents Should Learn This

Agents should treat diagnostics as instructions for the next framework call. When structured steering is present, the agent should use the supplied `help`, `retry`, `dryRun`, or permission action and preserve the original structured request unless the diagnostic identifies a specific field to change.

The framework's wording should reinforce that habit. Error text should be direct and specific. When the framework rejects shell syntax, the diagnostic should teach the typed alternative. When an effect-lane redirect is needed, the diagnostic should name the exact tool and supply retry data.

## Reference-Level Explanation

The planner and dispatcher return a `ResponseEnvelope` before MCP projection. The envelope is the internal contract; the adapter decides how to render it into `content`, `structuredContent`, and MCP error flags.

```rust
pub struct ResponseEnvelope {
    pub status: ResponseStatus,
    pub command: Option<Vec<String>>,
    pub output: Option<CommandOutput>,
    pub error: Option<ErrorBody>,
    pub diagnostics: Vec<Diagnostic>,
    pub steering: Vec<SteeringAction>,
    pub display: Option<DisplayHint>,
    pub replay: Option<ReplayEnvelope>,
}

pub enum ResponseStatus {
    Ok,
    InvalidInput,
    PermissionRequired,
    PermissionDenied,
    WrongEffectLane,
    NotFound,
    Failed,
}
```

`command` is the resolved command path, not the raw command string. It is absent when parsing fails before a command can be resolved.

`ErrorBody` carries the stable error code and display message. Codes are public contract; messages are explanatory prose and may evolve.

```rust
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    pub details: serde_json::Value,
}

pub enum ErrorCode {
    EmptyCommand,
    UnterminatedQuote,
    ShellSyntax,
    InvalidPlaceholder,
    PlaceholderInterpolation,
    UnknownCommand,
    UnknownArgument,
    MissingArgument,
    InvalidArgumentType,
    WorkspaceMismatch,
    WrongEffectLane,
    PermissionDenied,
    ApprovalInvalid,
    HandlerFailed,
}
```

A `Diagnostic` points at the request element that caused or explains the problem. Locations are intentionally typed so the agent does not have to reverse-engineer a string span.

```rust
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub message: String,
    pub location: Option<DiagnosticLocation>,
    pub expected: Option<serde_json::Value>,
    pub actual: Option<serde_json::Value>,
    pub suggestions: Vec<Suggestion>,
}

pub enum DiagnosticLocation {
    CommandToken { index: usize, value: String },
    Placeholder { name: String },
    Argument { name: String },
    OutputField { name: String },
    ToolName { name: String },
    Workspace { name: String },
}
```

Steering actions describe explicit next calls. A steering request must use framework request shapes, not shell snippets. If a steering action retries a command, it must preserve the structured arguments and output request unless the diagnostic explicitly changes them.

```rust
pub struct SteeringAction {
    pub kind: SteeringKind,
    pub label: String,
    pub request: serde_json::Value,
    pub priority: SteeringPriority,
}

pub enum SteeringKind {
    Help,
    RetryRun,
    RetryWithTool,
    DryRun,
    RequestPermission,
}
```

`RunRequest.output` gains a response profile. The profile controls ordinary successful responses; it does not suppress structured data required for errors, permission, replay, or tasks.

```rust
pub enum ResponseProfile {
    Text,
    Structured,
    CompactStructured,
    Debug,
}
```

`Text` returns display content and omits `structuredContent` for ordinary successes. `Structured` includes shaped output according to the output contract, including selected fields, limits, cursors, and byte limits. `CompactStructured` preserves stable navigation and replay fields while omitting large bodies, traces, and full plans. `Debug` includes the invocation plan, bound arguments, permissions, workspaces, timing, catalog identity, and other inspection data. A dry run should default to `Debug` unless the caller asks for a smaller profile.

The MCP projection layer owns the mapping from envelope to MCP tool result. Ordinary successful `Text` responses should be terse. Error responses should include structured diagnostics. Hidden replay tokens must never appear in display content. A wrong effect-lane response is a tool execution error with structured retry data, not a protocol error, because the client successfully called a valid MCP tool and the framework successfully planned the command.

### Required Invariants

- Stable error codes are not removed or repurposed after publication.
- Diagnostic locations use typed request concepts rather than unstructured string offsets.
- Steering requests use framework request shapes and preserve typed arguments.
- Ordinary successful `Text` responses may omit structured content.
- Error, permission, task, replay, and wrong-lane responses include structured content.
- Replay tokens are never emitted in display text.
- Handlers return framework output rather than constructing MCP tool results directly.

### Implementation Phases

1. Add the envelope, diagnostic, steering, display, replay, and profile types.
2. Convert existing framework errors into stable error codes and diagnostics.
3. Add nearest-command and nearest-namespace steering for command resolution failures.
4. Add response profile handling to output shaping.
5. Centralize MCP projection in one adapter path.
6. Update handlers to return framework output only.

### Acceptance Tests

- Shell syntax errors include `ShellSyntax`, token location, and help steering.
- Unknown commands include nearest available alternatives when known.
- Missing arguments steer to command help.
- Unknown arguments include accepted argument names.
- Type errors include expected and actual type information.
- Workspace mismatches include workspace identity.
- Wrong effect-lane errors include exact retry tool and preserved arguments.
- Ordinary successful text responses omit structured content.
- Structured responses obey fields, limits, cursors, and byte limits.
- Debug responses include the invocation plan and permission metadata.
- Error responses include diagnostics even when text output is requested.

## Drawbacks

The envelope adds a framework layer between handler output and MCP output. That layer is valuable because it makes behavior consistent, but it means handlers cannot casually return arbitrary MCP content if they want full framework support. Server authors may need to learn the difference between display text, structured output, diagnostics, and steering.

Stable error codes also create maintenance responsibility. Once agents learn an error code, changing its meaning becomes a compatibility break. The framework should accept that cost because the alternative is asking agents to parse mutable prose.

## Rationale And Alternatives

One alternative is to use MCP protocol errors for most failures. That would be appropriate for malformed MCP requests, but it is too coarse for framework-level planning failures. A well-formed tool call that contains an unknown command should produce a tool result the agent can learn from, not a protocol failure that hides command-specific recovery data.

Another alternative is to keep successful and failed responses in separate shapes. That makes each shape smaller, but it spreads projection rules across the framework. A single envelope lets the adapter apply consistent policy about display content, structured content, replay secrecy, and response profiles.

A third alternative is to include full structured content for every call. That is simpler to specify, but it burns context in the most common path. Profiles preserve the ability to request structure without making every ordinary success noisy.

## Prior Art

MCP tool results distinguish user-visible content from structured content, and MCP tools can report execution errors without implying that the protocol call itself was invalid. This RFC uses that distinction to keep framework diagnostics inside the tool-result channel.

Rust diagnostics provide stable categories, locations, and suggestions while still offering human-readable messages. This RFC borrows that separation because agents benefit from the same structure human developers use to recover from compiler errors.

CLI tools with machine-readable output modes show the value of separating display from data, but this framework makes that separation part of every command contract instead of leaving agents to discover per-tool flags.

## Unresolved Questions

- Should `PermissionRequired` be projected as an MCP tool result with `isError: true` or as a successful result that asks for confirmation?
- Should compact structured responses include a redacted plan, a plan id, or no plan data?
- Should suggestions ever include replacement command strings, or only typed `help`, `run`, and effect-lane retry requests?
- Should diagnostic locations eventually include source spans in addition to typed locations?

## Future Possibilities

The response envelope could support localized display messages without changing stable error codes. It could also support richer editor integrations that highlight the exact placeholder, argument, or output field involved in a diagnostic.

As task support matures, the same envelope can describe task start, progress, partial output, completion, and replay without creating a separate response family for long-running calls.
