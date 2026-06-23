# RFC 0003: Effect Escalation, Preview, Confirmation, And Replay

- Status: Draft
- Area: permissions, preview, confirmation, replay
- Target milestone: v0.3
- Depends on: RFC 0001, RFC 0002, RFC 0005

## Summary

This RFC defines the framework permission workflow that runs after a command has been parsed and planned. The workflow has two layers. The MCP-facing layer uses effect-lane tools from RFC 0005 so clients can see truthful tool annotations and create their normal approval flows. The framework layer uses typed invocation plans to preview effects, decide whether confirmation is required, issue replay tokens, and verify that an approved invocation is exactly the invocation that later runs.

The design is intentionally ergonomic for agents. Agents should start with the primary `{name}` tool. If a command belongs to a stronger effect lane, the framework returns a structured wrong-lane response that names the exact tool to call and preserves the request. Only after the command reaches the correct lane does framework-level preview or confirmation decide whether the planned effects may dispatch.

## Motivation

Invocation plans are the substrate for permission behavior. The framework parses a command template, binds typed arguments, resolves a catalog operation, checks workspaces, classifies effects, and records permission targets before deciding whether a command may dispatch.

The MCP tool layer also matters. Tool annotations are hints rather than access control, but they are still part of the client experience. Effect-lane tools give clients truthful metadata for reads, writes, deletes, process execution, and network calls while preserving the primary-tool-first flow agents learn from the catalog.

The remaining problem is confirmation. A client or user should be able to see what will happen before dispatch, approve that exact plan, and know that the approval applies only to the reviewed command and arguments. The framework needs a replay record bound to the invocation plan fingerprint.

## Guide-Level Explanation

The normal path starts with the primary execution tool. An agent sends the command request there because the prompt, help, and tool descriptions all say that the primary tool is the starting point. If the command is allowed in that lane, the framework continues. If the command requires an escalated lane, the framework does not dispatch. It returns a wrong-lane tool result that says, for example, `This command requires repo-write`, and includes retry data containing the same request.

After the request reaches a tool whose lane may execute the command, the framework builds or reuses the invocation plan. A preview request explains the user-facing intent and effects without dispatching. A dry run explains the technical plan. A normal execution request either dispatches, returns a confirmation requirement, or returns a denial.

When confirmation is required, the response includes a display message for the user and a replay envelope for the agent or client. The replay token is opaque and hidden from display text. A later request may include the token as approval. Before dispatch, the framework verifies that the token was issued for the same command template, structured arguments, operation id, effect list, workspace resolution, and permission targets. If any meaningful part of the plan changed, the approval is invalid.

This flow makes the primary tool the expected starting point. Escalation is a structured redirect when the catalog requires a different MCP tool annotation profile; confirmation is a separate framework decision about whether the planned effects may run.

### How Agents Should Learn This

Agent-facing docs should teach a simple rule: call `{name}` first and follow structured redirects exactly. If the framework asks for `{name}-write`, retry the same request with `{name}-write`. If the framework returns a preview or confirmation response, present the human-readable permission message and preserve the replay data for the approved retry.

The wording should describe escalated tools as lanes for the same catalog. A command retried through `{name}-write` remains the same invocation request, reaching the lane whose annotations match the operation's effect.

## Reference-Level Explanation

`RunRequest` gains an execution mode and optional approval data. The command shape remains the same across the primary and escalated tools defined by RFC 0005.

```rust
pub enum RunMode {
    Execute,
    Preview,
    DryRun,
}

pub struct ApprovalInput {
    pub token: String,
    pub confirm: bool,
}
```

Preview answers the user-facing question, "What would this command do?" It must parse, bind, plan, resolve workspaces, and evaluate permission metadata. It must not call the handler. Preview text should describe the operation in concrete terms, and structured preview data should include the effect, scope, target, confirmation policy, and operation id.

Dry run answers the implementation-facing question, "What plan did the framework build?" It may include bound arguments, workspace resolution, catalog identity, effect-lane routing, output profile, permission specs, idempotency metadata, and handler routing. It is not the approval surface, although it may be useful for debugging an approval failure.

Normal execution answers, "May this plan dispatch now?" The framework first verifies that the current MCP tool may execute the plan's effect lane. If not, RFC 0005's wrong-lane response is returned and dispatch does not happen. If the lane is correct, the framework asks the authorizer for a decision.

```rust
pub enum PermissionDecision {
    Allow,
    RequireConfirmation,
    Deny { reason: String },
}

pub trait PermissionAuthorizer {
    fn decide(&self, plan: &InvocationPlan) -> PermissionDecision;
}
```

The default authorizer should allow `Pure` and `Read`, require confirmation for `Write`, `Delete`, `Exec`, and `Network`, and deny unknown custom effects unless configured. A server may replace the authorizer, but it must still receive an invocation plan that includes catalog operation identity, effect classification, workspace resolution, and permission targets.

`PermissionSpec` describes the effect in terms a user can approve. The target must be derived from typed arguments, workspace identity, or handler-provided planning metadata. It must not be recovered by parsing a shell-like command string.

```rust
pub struct PermissionSpec {
    pub effect: EffectSpec,
    pub scope: String,
    pub target: Option<String>,
    pub description: String,
    pub confirmation: ConfirmationPolicy,
}

pub enum ConfirmationPolicy {
    Never,
    WhenClientRequires,
    Always,
}
```

When confirmation is required, the framework stores or signs a replay record and returns a replay envelope through the response contract from RFC 0002. The display content should state the required approval in plain language. The replay token must be present only in structured content.

```rust
pub struct ReplayRecord {
    pub token_id: String,
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
    pub invocation_fingerprint: String,
    pub operation_id: String,
    pub effects: Vec<PermissionEffect>,
    pub reusable: bool,
}
```

The invocation fingerprint is computed from the normalized command template, structured arguments, stdin metadata, workspace resolution, operation id, effect list, permission targets, and output-affecting fields that matter for the approved operation. The exact hashing format is an implementation detail, but two meaningfully different invocations must not share a replay fingerprint.

### Required Validation Rules

- Preview and dry run never dispatch handlers.
- Wrong-lane calls never dispatch handlers.
- Confirmation replay dispatches only after the token is valid and the invocation fingerprint matches.
- Replay tokens expire.
- Replay tokens are single-use unless the operation is explicitly reusable.
- A denial result must explain whether policy denied the operation or approval validation failed.
- Replay tokens never appear in display content.

### Implementation Phases

1. Add `RunMode`, `ApprovalInput`, replay records, and invocation fingerprinting.
2. Extend `InvocationPlan` with operation id, effects, permission targets, workspace identity, output-affecting request fields, and idempotency metadata.
3. Add `PermissionAuthorizer` and default policy.
4. Add an in-memory confirmation store, then keep the trait open for durable stores.
5. Project permission-required, permission-denied, and approval-invalid results through RFC 0002.
6. Update generated help and prompts to teach primary-tool-first escalation from RFC 0005.

### Acceptance Tests

- Preview returns invocation messages and effects without dispatch.
- Dry run returns a technical plan without dispatch.
- A read command in the primary lane executes without confirmation under the default authorizer.
- A write command first redirects from the primary tool to the write lane, then returns confirmation when policy requires it.
- Replay succeeds only for the same command, arguments, operation, workspaces, effects, targets, and relevant output fields.
- Replay fails if arguments change after approval.
- Replay fails after token expiration.
- Replay fails if a non-reusable token is reused.
- Display content never includes replay tokens.
- Structured content includes replay tokens only when required for confirmation.

## Drawbacks

The two-layer model introduces more concepts than a single allow-or-deny check. Server authors need to understand effect lanes, preview, dry run, confirmation, and replay. Agents need to learn that a wrong-lane response is an expected redirect rather than a failure to be worked around.

The model also requires careful fingerprinting. If the fingerprint includes too little, an approval might be reused unsafely. If it includes too much, harmless output-shaping changes might invalidate approvals unnecessarily. The framework should start conservative and relax only where the semantics are clear.

## Rationale And Alternatives

One alternative is to rely entirely on MCP client approval for tools with broad annotations. That would make the framework simpler, but it would collapse all effects behind worst-case metadata and would not prove that the approved plan is the plan that later dispatched.

Another alternative is to expose preview as the only permission mechanism. Preview is useful, but it cannot by itself bind approval to a later execution. Replay tokens are proposed because they connect user intent to the exact invocation plan.

A third alternative is to ask agents to choose the correct effect-lane tool before the first call. That makes the model more brittle. Agents can often infer whether a command writes or reads, but the catalog already knows. The primary-tool-first rule lets the framework perform that routing and preserves a smooth path for agents.

## Prior Art

MCP tool annotations give clients advisory metadata about whether a tool is read-only, destructive, idempotent, or open-world. RFC 0005 uses those hints at the tool lane level, while this RFC keeps enforcement inside the framework.

Many CLI tools support dry-run flags, but those flags usually produce human text and do not bind approval to later execution. Package managers and infrastructure tools often preview planned changes before applying them; this RFC brings that shape into the MCP command framework with typed arguments and replay verification.

Capability and transaction systems provide a useful analogy: approval should be attached to a specific planned operation, not to a mutable string that can be changed after review.

## Unresolved Questions

- Should preview be represented as `mode: "preview"` or as a dedicated request field?
- Should confirmation tokens be signed stateless values, server-side handles, or both?
- Which output request fields should participate in the replay fingerprint?
- Should custom effects default to denial or confirmation when a server provides a user-facing permission description?

## Future Possibilities

The replay contract could support durable approval stores for long-running clients, explicit cancellation of pending approvals, or user-visible approval history. A future client might also use preview data to render richer approval dialogs with affected resources, workspace labels, and before/after summaries.

The same invocation fingerprinting machinery may later support idempotency keys for safe retry after ambiguous runtime failures.
