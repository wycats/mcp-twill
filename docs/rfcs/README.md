# RFCs

This directory contains design proposals for the CLI-shaped MCP framework. An RFC is expected to read like a design proposal, not like a set of meeting notes. It should motivate a change, teach the model in the language an implementor or agent author would use, specify the reference behavior precisely, and then discuss drawbacks, alternatives, unresolved questions, and likely extensions.

The house style follows the Rust RFC distinction between motivation, guide-level explanation, reference-level explanation, rationale, and future work. It also follows Ember's emphasis on detailed design and teaching: names, examples, prompts, and diagnostics are part of the design because agents learn the framework through those surfaces.

## Project Values

All RFCs in this directory inherit the same core model. MCP is the protocol and control plane, while this framework provides a command-shaped contract inside MCP. The command string is a template over typed values, not a shell program. Pipes, redirection, command substitution, shell variable expansion, and shell control operators do not belong in the command string. If the framework later supports composition, filtering, globbing, streaming, or redirection-like behavior, those capabilities must be represented as typed framework features with explicit planning, diagnostics, permissions, and tests.

The public MCP surface should remain small and predictable. The first implementation milestone exposed `help` and `run`; RFC 0005 refines that model into one discovery surface plus a small generated family of execution tools when different effect lanes need truthful MCP annotations. The catalog remains the authority either way. Help, examples, diagnostics, permission prompts, resources, prompts, generated schemas, and generated tests should be derived from or checked against the same command model that dispatch uses.

## RFC Shape

New RFCs should start from [RFC 0000: Template](0000-template.md). Each RFC should use these top-level sections unless a later accepted RFC changes the house style:

- `Summary`
- `Motivation`
- `Guide-Level Explanation`
- `Reference-Level Explanation`
- `Drawbacks`
- `Rationale And Alternatives`
- `Prior Art`
- `Unresolved Questions`
- `Future Possibilities`

Lists are appropriate when they define metadata, Rust type sketches, exact validation rules, implementation phases, acceptance tests, or unresolved decisions. They should not replace the argument. When a section is explaining why the framework behaves a certain way, use prose.

RFC review should read the `Summary`, `Motivation`, `Guide-Level Explanation`, and `How Agents Should Learn This` sections as the primary proposal narrative. Those sections should lead with the model the framework provides, the behavior agents should learn, and the implementation contract authors must build. Contrastive language belongs in project values, exact invariants, validation rules, drawbacks, alternatives, and prior art.

## Status Values

- `Draft`: written for design review.
- `Accepted`: approved for implementation.
- `Implemented`: reflected in code and tests.
- `Superseded`: replaced by a later RFC.

## Index

- [RFC 0000: Template](0000-template.md)
- [RFC 0001: Authoritative Command Catalog](stage-4/0001-authoritative-command-surface.md)
- [RFC 0002: Diagnostics, Steering, And Response Profiles](stage-4/0002-diagnostics-steering-response-profiles.md)
- [RFC 0003: Effect Escalation, Preview, Confirmation, And Replay](stage-4/0003-permission-preview-confirmation.md)
- [RFC 0004: Runtime Maturity, Workspace Identity, Events, And Contract Tests](stage-4/0004-runtime-workspace-contracts.md)
- [RFC 0005: Effect-Lane Tool Surface](stage-4/0005-effect-lane-tool-surface.md)
- [RFC 0006: Author Ergonomics](stage-4/0006-author-ergonomics.md)
- [RFC 0007: Workspace Resolution Crate](stage-4/0007-workspace-resolution-crate.md)
- [RFC 0008: Named Argument Types And Unions](stage-0/0008-named-argument-types-and-unions.md)
- [RFC 0009: Handler-Visible Workspace Roots](stage-0/0009-handler-visible-workspace-roots.md)
- [RFC 0010: Declared Preconditions](stage-0/0010-declared-preconditions.md)
- [RFC 0011: Guidance Decomposition](stage-0/0011-guidance-decomposition.md)
- [RFC 0012: First-Class Resources](stage-4/0012-first-class-resources.md)
- [RFC 0013: Conversation Identity Request Context](stage-2/0013-conversation-identity-request-context.md)

## Suggested Implementation Order

RFC 0001 should land first because the catalog is the authority used by the other proposals. RFC 0002 should follow because diagnostics and response profiles make catalog failures usable by agents. RFC 0005 should be implemented before the permission workflow in RFC 0003 is finalized, because effect-lane routing changes the MCP-facing execution surface. RFC 0006 should land once the foundation API is stable enough to wrap, so new example servers teach the preferred authoring path before the preview and replay workflow adds more concepts. RFC 0003 then adds preview, confirmation, and replay on top of catalog effects and effect-lane routing. RFC 0007 should land before the workspace portions of RFC 0004 mature, because it gives path arguments and workspace identity a shared resolver contract. RFC 0004 can land incrementally because its runtime identity, workspace identity, event sinks, and generated contract tests are optional maturity features.
