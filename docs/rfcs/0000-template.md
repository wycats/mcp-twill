# RFC 0000: Title

- Status: Draft
- Area: area, subsystem, or protocol surface
- Target milestone: vX.Y
- Depends on: RFC numbers or `None`

## How To Use This Template

Start from this file when drafting a new RFC. Replace instructional text with proposal text before review. The finished RFC should read like a design proposal, not a structured note. It should motivate the change, teach the proposed model as if it exists, define reference behavior precisely enough to implement, and then discuss drawbacks, alternatives, unresolved questions, and future possibilities.

The style is prose-first. Use paragraphs for motivation, explanation, consequences, and rationale. Use compact lists only when the content is structural: metadata, type sketches, protocol fields, validation rules, algorithms, implementation phases, acceptance tests, and unresolved decisions.

Prefer affirmative proposal language. Say what this RFC proposes, defines, requires, or guarantees. Reserve contrastive language for project values, exact invariants, drawbacks, and rejected alternatives. For example, a summary should usually say "This RFC makes the catalog the source of public command behavior" rather than beginning from what the catalog is not.

Review the early prose by reading the `Summary`, `Motivation`, `Guide-Level Explanation`, and `How Agents Should Learn This` sections as a coherent proposal. Those sections should lead with the model being introduced, the behavior agents and implementors should expect, and the concrete framework surface that carries the design.

When the proposal affects agent behavior, include a `How Agents Should Learn This` subsection under `Guide-Level Explanation`. Naming, steering language, diagnostics, examples, prompts, permission escalation, and tool descriptions are part of the design because agents learn the framework through those surfaces.

## Summary

State the proposed change in a few paragraphs. Name the core model or behavior this RFC introduces, the framework surface it changes, and the result an implementor should expect after the RFC is implemented.

The summary should focus on what the proposal establishes. Avoid leading with discarded designs or implementation accidents. If the RFC changes an earlier invariant, say so directly and point to the new invariant.

## Motivation

Explain the problem this proposal solves and why the framework should solve it at this layer. This section should make the reader feel the design pressure before seeing the detailed design.

Use concrete failure modes, but connect them to the framework's model. For this project, good motivation usually explains how command templates, typed arguments, MCP tools, resources, prompts, diagnostics, workspaces, progress, tasks, permissions, or generated tests become better when the proposal exists.

## Guide-Level Explanation

Teach the feature as an implementor or agent author would experience it. Describe the ordinary flow first. Introduce names in the order a reader would use them. Explain the consequences of the design in prose before showing type sketches or algorithms.

If the RFC affects user or agent ergonomics, include this subsection.

### How Agents Should Learn This

Describe how help, resources, prompts, examples, tool descriptions, diagnostics, and structured steering should teach the behavior. Be specific about the desired agent habit. If the RFC changes naming or escalation behavior, say which entry point the agent should try first and what data it should preserve when retrying.

This subsection should not be marketing copy. It should be implementation guidance for the generated surfaces agents actually read.

## Reference-Level Explanation

Specify the behavior precisely enough to implement. This is the right place for Rust type sketches, request and response fields, state machines, planning algorithms, exact validation rules, projection rules, and interoperability requirements.

Normative prose should say what the framework must do. Code blocks should clarify data shape. Lists are appropriate when they define exact mechanics.

```rust
pub struct ProposedType {
    pub field: String,
}
```

Use structural subsections when they help implementors.

### Required Invariants

- State exact rules the implementation must preserve.
- Keep each invariant testable.
- Prefer framework terms over vague words.

### Implementation Phases

1. Put implementation steps in dependency order.
2. Keep the list focused on engineering sequence, not design argument.
3. Include migration or compatibility work when needed.

### Acceptance Tests

- Describe observable behavior that proves the RFC is implemented.
- Include success paths, failure paths, and agent-ergonomics checks when relevant.
- Include generated-contract or MCP metadata tests when the RFC affects public protocol surfaces.

## Drawbacks

Explain the cost of accepting the proposal. Good drawbacks are concrete: additional concepts, compatibility risk, maintenance burden, implementation complexity, migration work, client behavior risk, or testing cost.

Do not use this section to undo the proposal. The goal is to show that the tradeoffs are understood.

## Rationale And Alternatives

Explain why this design is the proposed design. Compare it to serious alternatives, including simpler options and options that may look attractive to an implementor. This is the right place to say why rejected designs were rejected.

When an alternative is rejected because it violates a project value, name the value precisely. For example, raw shell syntax in command strings is rejected because this project treats command strings as templates over typed values.

## Prior Art

Discuss relevant precedent. Prior art may include MCP protocol features, Rust or Ember RFC practice, CLI design patterns, schema-driven frameworks, permission systems, or examples from related tools.

Prior art should explain what this proposal learns from the precedent. It should not be a link dump.

## Unresolved Questions

- Capture decisions that remain open after this RFC.
- Phrase each item as a decision the project can eventually make.
- Avoid using this section for implementation tasks that already belong in `Implementation Phases`.

## Future Possibilities

Describe plausible extensions that become possible if the RFC lands. Keep this section separate from the proposal itself. Future possibilities should not be required for the RFC to be useful.
