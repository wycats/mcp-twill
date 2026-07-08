<!-- exo:13 ulid:01kx1nthacje77hc2x28j2brqd -->

# RFC 0013: Conversation Identity Request Context

- Status: Draft
- Area: request metadata, command planning, handler context, catalog projection
- Target milestone: v0.2
- Depends on: RFC 0002 (diagnostics and response profiles), RFC 0003 (preview and replay), RFC 0009 (handler-visible workspace roots)

## Summary

This RFC gives Twill commands an optional, host-supplied conversation identity. A command declares `uses_conversation_identity()`, Twill normalizes supported request metadata into one `ConversationIdentity`, and the handler reads it from `CommandContext`. The value is request context rather than a tool argument: it never enters model-visible schemas, command templates, plans, responses, previews, events, or framework-owned logs.

The canonical MCP representation is a versioned value under `_meta["io.github.wycats.mcp-twill/conversation-identity"]`. Twill also recognizes Codex's existing top-level `_meta.threadId` and normalizes it to issuer `com.openai.codex`. When both observations appear, they must describe the same identity. Malformed or conflicting observations fail before handler dispatch with diagnostics that name the source and problem without disclosing the raw identifier.

Conversation identity is optional ambient context, not an authentication credential and not an application session. Applications decide how to use it. Visible Browser Lab, the motivating consumer, maps the identity to a browser session while continuing to give an explicit `agent_session_id` precedence at its application boundary. Twill owns the reusable metadata, validation, propagation, projection, and fingerprint contract; it does not own browser leases, TTLs, workspace binding, or any other consumer policy.

## Motivation

Some application state belongs to a conversation even though no model-visible argument should carry the conversation handle. Agent context is an unreliable place for infrastructure identity: compaction can remove it, long workflows can lose it, and copying it through every call turns ordinary application use into bookkeeping. The hosts already possess a better fact. Codex persists a thread identity and attaches it to MCP calls; other MCP hosts can supply the canonical namespaced value directly; native adapters such as Visible Browser Lab's VS Code extension can construct the canonical value from their own host context.

Twill already has the architectural precedent for ambient request facts. RFC 0009 lets a command declare `uses_workspace("project")`; the framework resolves host observations before dispatch and gives the result to the handler through `CommandContext`. Conversation identity has the same cross-cutting shape, with one important semantic difference: a declared workspace is required, while conversation identity remains optional because bare MCP clients and global tool invocations may have no conversation to report.

Leaving normalization to each server would split one contract across adapters and handlers. Each server would choose its own metadata key, version policy, Codex compatibility behavior, conflict handling, privacy boundary, and replay semantics. The differences would become visible precisely on failure: one server might silently prefer a malformed canonical value, another might leak the raw id in a plan, and a third might let an approval issued in one conversation replay in another. Twill is the layer that already owns request adaptation, planning, fingerprints, diagnostics, and catalog projection, so it can make those behaviors one framework guarantee.

The metadata channel also protects the authoritative command model. Conversation identity does not appear in `ArgSpec`, generated JSON Schema, examples, or command strings. Model-visible arguments remain exactly the values the command declares. This preserves the distinction RFC 0009 established: ambient host facts arrive through request context, while arguments remain model-authored inputs.

## Guide-Level Explanation

A server author declares that a command can use conversation identity:

```rust
server.command("artifact list", |command| {
    command
        .summary("List artifacts for the current conversation")
        .uses_conversation_identity()
        .handle(|context: CommandContext| async move {
            match context.conversation_identity() {
                Some(identity) => list_for_conversation(identity).await,
                None => list_without_conversation_scope().await,
            }
        });
});
```

The declaration is deliberately optional. It means the handler can consume a host observation when one exists; it does not require every host to invent one. A bare MCP client that sends no identity reaches the handler with `None`. The application may support that case directly, return an application-specific recovery such as `session_required`, or rely on an explicit argument. Twill does not choose among those application policies.

On MCP, a host that implements the canonical contract sends:

```json
{
  "_meta": {
    "io.github.wycats.mcp-twill/conversation-identity": {
      "version": 1,
      "issuer": "com.example.host",
      "id": "opaque-conversation-id"
    }
  }
}
```

Codex already sends a persisted thread id as top-level `_meta.threadId`. Twill turns that observation into the same type with version `1` and issuer `com.openai.codex`. A future Codex release may send both the compatibility field and the canonical value during migration; Twill accepts the pair only when the normalized tuples are equal.

The identity is available to the handler and to the invocation fingerprint, but it is absent from the invocation plan. The fingerprint changes when a declaring command is called from a different conversation, so a permission approval or replay token cannot cross that boundary. The plan, permission preview, framework event, and response remain safe to serialize because none contains the raw value or its standalone digest.

An application may also expose an explicit resource or session argument. Twill binds that argument normally and exposes ambient identity independently. The application owns the precedence rule because only it knows what the explicit value means. Visible Browser Lab chooses a non-empty `agent_session_id` first, ambient identity second, and its explicit recovery protocol last; another application may choose a different policy without changing Twill's transport contract.

### How Agents Should Learn This

Generated help names conversation identity as optional host context for commands that declare it. The catalog carries the same boolean declaration. Tool schemas and examples contain no identity property, so an agent learns the correct habit by construction: call the desired command with its documented arguments and let the host provide context.

An agent should never fabricate the canonical metadata key inside `$args` or retain its value in conversation text. When identity is absent, the command's application-level result teaches the supported fallback. When metadata is malformed or conflicting, the framework diagnostic says that host request context is invalid and identifies the metadata source; changing command arguments is not presented as a repair.

Help and catalog prose describe the capability, never the observed identity. This distinction lets an agent understand why a command's behavior may be conversation-scoped while preserving the privacy boundary around the host's correlation value.

## Reference-Level Explanation

### Canonical Identity

Twill exports the metadata key and validated value type:

```rust
pub const CONVERSATION_IDENTITY_META_KEY: &str =
    "io.github.wycats.mcp-twill/conversation-identity";

#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ConversationIdentity {
    version: u32,
    issuer: String,
    id: String,
}

impl ConversationIdentity {
    pub const VERSION: u32 = 1;

    pub fn new(
        issuer: impl Into<String>,
        id: impl Into<String>,
    ) -> Result<Self>;

    pub fn version(&self) -> u32;
    pub fn issuer(&self) -> &str;
    pub fn id(&self) -> &str;
}
```

`ConversationIdentity` serializes to and deserializes from the three-field JSON object shown above. Deserialization uses the same validation as `new` and rejects unknown fields. Version `1` is the only accepted version.

An issuer is a lowercase reverse-DNS name with at least two labels. Each label contains ASCII lowercase letters, digits, or internal hyphens; it starts and ends with a letter or digit. Empty labels and leading or trailing dots are invalid. `com.openai.codex` and `com.microsoft.vscode` are valid issuers. The issuer is an authority namespace, not a network lookup.

The id is a non-empty JSON string. Twill preserves it byte-for-byte as UTF-8 and assigns no UUID, URI, case, or whitespace semantics. The complete `(version, issuer, id)` tuple is the identity, so equal ids under different issuers remain distinct.

The type implements `Clone`, equality, and hashing over the complete tuple. Its custom `Deserialize` implementation validates the payload. Its custom `Debug` implementation prints the schema version but renders both issuer and id as `<redacted>`. Framework error types and diagnostics never retain or format the raw tuple.

Conversation identity is correlation data. Possessing or asserting a tuple proves neither authentication nor authorization. A server that needs a security principal or an unforgeable capability must resolve one through its own authenticated application boundary.

### Declaration And Projection

`CommandSpec` and `OperationSpec` gain a boolean `uses_conversation_identity` field, serialized as `usesConversationIdentity` and omitted when false. The low-level declaration is `CommandSpec::uses_conversation_identity()`. `CommandBuilder::uses_conversation_identity()` sets the same field.

The declaration is optional in both senses: commands opt in, and an opted-in command accepts absence. Registration therefore has no provider-existence check analogous to a required workspace or capability.

The operation catalog projects `usesConversationIdentity: true`, and the catalog hash covers it through normal operation serialization. Command help renders a `Request context:` section with `conversation identity (optional, supplied by host)`. Generated input schemas, command examples, usage templates, and server instructions gain no identity field or example value.

The generated contract suite adds `check_conversation_identity_projection`. It verifies that every command's spec and operation projection agree, declaring command help names the optional context, non-declaring command help does not, and no generated input schema contains `CONVERSATION_IDENTITY_META_KEY` or a conversation-identity argument.

### Invocation Context

The raw identity travels in a framework-owned invocation context with private fields and no `Serialize`, `Deserialize`, or `JsonSchema` implementation:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvocationContext {
    conversation_identity: Option<ConversationIdentity>,
}

impl InvocationContext {
    pub fn new() -> Self;
    pub fn with_conversation_identity(
        self,
        identity: ConversationIdentity,
    ) -> Self;
    pub fn conversation_identity(&self) -> Option<&ConversationIdentity>;
}
```

`InvocationContext` is the explicit host/test injection surface. Its fields remain private, and it accepts only an already validated `ConversationIdentity`. The public convenience methods are:

```rust
impl CommandRegistry {
    pub fn build_plan_with_context(
        &self,
        request: &RunRequest,
        context: &InvocationContext,
    ) -> Result<InvocationPlan>;

    pub async fn run_with_context(
        &self,
        request: RunRequest,
        context: InvocationContext,
    ) -> Result<RunResponse>;
}
```

Existing `build_plan` and `run` delegate to these methods with `InvocationContext::default()`. Existing workspace- and lane-aware entry points remain identity-free wrappers. Their context-aware counterparts are `build_plan_with_workspaces_and_context` and `run_in_lane_with_workspaces_and_context`, with one additional final `&InvocationContext` argument; the rmcp adapter uses those counterparts for planning, preview, replay validation, task-augmented execution, and dispatch. This keeps runtime workspace observations and conversation identity on one plan/run path rather than composing independently after planning.

`CommandContext` gains a private invocation-context field excluded structurally from both serialization and schema generation, plus the public accessor:

```rust
pub struct CommandContext {
    // ...existing fields...
    #[serde(skip)]
    #[schemars(skip)]
    invocation_context: InvocationContext,
}

impl CommandContext {
    pub fn conversation_identity(&self) -> Option<&ConversationIdentity>;
}
```

`InvocationContext` can derive `Debug` safely because `ConversationIdentity::Debug` redacts the id. Its `Clone`, `PartialEq`, and `Eq` implementations preserve `CommandContext`'s existing trait surface after the private field is added.

The registry retains the same invocation context from the first plan through handler dispatch. A declaring command receives the normalized identity when present. A non-declaring command receives `None`, even when the host supplied metadata, so handler access always agrees with the catalog declaration.

Adding a private field closes external `CommandContext` struct-literal construction. The supported identity-free construction path is:

```rust
impl CommandContext {
    pub fn new(
        plan: InvocationPlan,
        stdin: Option<StdinSpec>,
        resources: ResolvedResources,
    ) -> Self;
}
```

`CommandContext::new` installs `InvocationContext::default()`, so `conversation_identity()` returns `None`. Framework dispatch uses a crate-private constructor that accepts the normalized invocation context.

### Transport Normalization And Authority

For each execution-tool call, the rmcp adapter reads two possible observations from `RequestContext.meta`:

1. The value under `CONVERSATION_IDENTITY_META_KEY`.
2. Top-level `threadId`, normalized as `{version: 1, issuer: "com.openai.codex", id: threadId}`.

The canonical value is authoritative, while the compatibility value is a consistency observation when both are present. Normalization follows these rules:

1. If the canonical key is present, parse and validate it. A malformed object, unknown field, unsupported version, invalid issuer, or empty id is an error; Twill does not fall back to `threadId`.
2. If `threadId` is present, require a non-empty string and normalize it to the Codex issuer. A present malformed value is an error.
3. If both values are valid, require equality of the complete normalized tuples. Equality succeeds; any difference is a conflict error.
4. If exactly one valid observation exists, use it.
5. If neither exists, produce an empty invocation context.

Normalization happens once for every execution-tool call, before planning and before permission checks. A malformed or conflicting observation therefore fails an execution call even when the selected command does not declare conversation identity. This treats a present value under Twill's owned namespace as a request-integrity claim rather than silently ignoring invalid metadata. A valid identity still reaches and fingerprints only declaring commands. Task-augmented execution clones the same context into its queued work. Help, resource, and prompt requests do not consume conversation identity because they do not dispatch a declared command.

No model-visible argument is an identity observation. Twill does not reserve an argument name, widen schemas, strip pre-validation fields, inspect `agent_session_id`, or derive identity from a process id, MCP connection, stdio lifetime, workspace root, or server `RuntimeIdentity`.

### Planning And Fingerprints

`InvocationPlan` gains no raw identity field. When the selected command declares `uses_conversation_identity`, the fingerprint input includes one private planning fact:

```json
{
  "conversationIdentity": "sha256-of-canonical-tuple-or-null"
}
```

The digest is SHA-256 over the stable JSON encoding of `["conversation-identity", version, issuer, id]`. Absence is encoded as JSON `null`, so an identity-free call differs from an identified call. The digest is supplied directly to the fingerprint calculation and is not stored on `InvocationPlan`, `PlanFacts`, replay records, response envelopes, or framework events. The only public derivative is the existing complete invocation fingerprint.

Commands that do not declare conversation identity omit this fact entirely. Supplying metadata to such a command therefore changes neither its handler context nor its invocation fingerprint.

The adapter must pass one normalized `InvocationContext` to both its initial planning pass and the dispatch path that rebuilds the plan. This preserves the existing invariant that preview, approval replay, and execution calculate the same fingerprint from the same request facts.

### Diagnostics And Non-Disclosure

Twill adds `ErrorCode::InvalidRequestContext` and two framework failures: invalid conversation identity and conflicting conversation identity. Both produce `ResponseStatus::InvalidInput`. The public diagnostic location is:

```rust
pub enum DiagnosticLocation {
    // ...existing variants...
    RequestContext { key: String },
}
```

It serializes as `{ "type": "requestContext", "key": "..." }`. An invalid observation names `CONVERSATION_IDENTITY_META_KEY` or `threadId`. A conflict uses `CONVERSATION_IDENTITY_META_KEY` as the location and names both source keys in redacted error details.

The stable subreason lives in `ErrorBody.details`, while the diagnostic carries the common `InvalidRequestContext` code and request-context location. An invalid value produces:

```json
{
  "source": "canonical",
  "key": "io.github.wycats.mcp-twill/conversation-identity",
  "field": "issuer",
  "reason": "invalid_issuer"
}
```

`source` is `canonical` or `codexThreadId`; `field` is omitted when the whole value has the wrong shape. Stable reasons include `expected_object`, `unknown_field`, `missing_field`, `unsupported_version`, `invalid_issuer`, `empty_id`, and `expected_non_empty_string`. The diagnostic may report the supported version or issuer grammar as `expected`; neither details nor diagnostics report the actual id or raw object.

A conflict produces redacted details with `reason: "conflicting_observations"` and `sources` containing the canonical key and `threadId`; it includes neither tuple. These failures produce no retry-with-arguments steering because command arguments cannot repair host metadata.

Framework privacy is structural:

- `InvocationContext` has no serialization or schema implementation.
- `CommandContext` skips the context field during serde serialization and schemars generation.
- `InvocationPlan`, `RunRequest`, `RunResponse`, `PermissionPreview`, `PlanFacts`, `FrameworkEvent`, catalog, help, and generated schemas contain no identity value or standalone digest.
- `ConversationIdentity::Debug` redacts the issuer and id, and framework diagnostics do not store the raw tuple.
- The final invocation fingerprint may be public because it already commits to all invocation facts and does not expose an identity-specific digest.

Handlers can deliberately read `identity.id()` and assume responsibility for their own application state and logs. The framework's non-disclosure guarantee covers framework-owned projections, diagnostics, telemetry, and ordinary logging paths; it cannot prevent application code from explicitly disclosing a value it requested.

### Application Authority

Twill provides ambient identity independently of declared arguments. When a command also accepts an explicit application handle, both facts reach the handler unchanged. The application defines their relationship.

Visible Browser Lab's coordinated RFC 00011 defines its application order as explicit non-empty `agent_session_id`, then ambient conversation identity, then `session_required`. This order belongs in VBL because Twill cannot know which argument is a session authority, how it resolves, or whether another application should prefer ambient context. The Twill contract guarantees only that ambient identity never overwrites or removes an argument.

### Required Invariants

- The canonical key and version-1 payload round-trip through `ConversationIdentity` with exact tuple equality.
- Canonical metadata and Codex `threadId` are the only rmcp observations; arguments, processes, connections, workspaces, and runtime identity never become conversation identity.
- A malformed canonical value or malformed Codex compatibility value fails before permission checks and handler dispatch.
- Malformed or conflicting identity metadata fails every execution-tool call, including calls to non-declaring commands; non-execution surfaces ignore the metadata.
- Matching canonical and Codex observations succeed; conflicting observations fail without disclosing either id.
- Absence is valid for every command, including one that declares `uses_conversation_identity`.
- A declaring command receives identity through `CommandContext` and binds its presence or digest into the invocation fingerprint.
- A non-declaring command receives `None` and has the same fingerprint whether metadata is present or absent.
- Raw identity and its standalone digest never appear in framework-owned plans, responses, previews, replay records, events, telemetry, help, logs, schemas, or diagnostics.
- Model-visible arguments are preserved exactly; Twill does not add, remove, or reinterpret an identity argument.
- Application-specific explicit handles remain application-owned and are never overwritten by ambient context.

### Implementation Phases

1. **Canonical model and declaration.** Add the constant, validated/redacted `ConversationIdentity`, `InvocationContext`, `CommandSpec` and builder declarations, catalog projection, help, and catalog-hash coverage.
2. **Normalization and diagnostics.** Parse canonical and Codex observations in the rmcp adapter, enforce equality and validation rules, and add redacted request-context diagnostics.
3. **Planning and dispatch.** Thread one invocation context through preview, dry run, replay, tasks, and execution; expose the handler accessor; bind a private digest into declaring-command fingerprints.
4. **Contracts and acceptance.** Add `conversation_identity.rs`, extend the generated contract macro, and verify serialization boundaries across every framework projection.

### Acceptance Tests

- Canonical metadata produces the exact version, issuer, and id in a declaring handler.
- Codex `threadId` produces version `1`, issuer `com.openai.codex`, and the original opaque id.
- Matching canonical and Codex observations reach the handler once; conflicting observations return `InvalidRequestContext` at request-context key `CONVERSATION_IDENTITY_META_KEY` before the handler, authorizer, or resource resolver runs.
- Canonical objects with missing or unknown fields, unsupported versions, invalid issuers, or empty ids fail with redacted diagnostics, including when `threadId` is valid; a present non-string or empty `threadId` fails even when canonical metadata is valid.
- Invalid-request-context responses assert the stable JSON details (`source`, `key`, optional `field`, and `reason`), request-context location, and absence of the raw identity.
- An opted-in command with no observation runs and sees `None`; direct registry execution and `CommandContext::new` are identity-free by default, while explicit `InvocationContext` injection reaches the handler.
- A command without the declaration sees `None` and has identical fingerprints with and without injected identity.
- A non-declaring execution command still rejects malformed or conflicting transport metadata before dispatch, while valid metadata remains unavailable to its handler.
- Malformed conversation metadata does not affect `help`, resource, or prompt requests because those non-execution surfaces do not normalize it.
- Two otherwise identical calls to a declaring command with different identities have different fingerprints; present and absent identity also differ; the same identity is stable across canonical-only, Codex-only, and matching-dual observations.
- Catalog JSON and help project the optional declaration, the catalog hash changes when it is added, and generated input schemas and examples contain no identity field.
- Serializing plans, run and preview envelopes, replay records, events, help, catalog, and diagnostics never contains the raw identity or the identity-specific digest; `Debug` output redacts the issuer and id.
- Serializing a populated `CommandContext` and generating its `JsonSchema` both exclude the private invocation-context field and the raw id.
- Task-augmented execution and ordinary execution deliver the same normalized identity and calculate the same fingerprint.
- A command with an ordinary explicit `agent_session_id` argument receives that argument unchanged alongside ambient identity, proving the framework leaves application precedence intact.

## Drawbacks

The framework gains a second ambient context concept alongside workspaces, plus context-aware planning and execution variants that every adapter path must carry consistently. The raw value stays out of plans, which protects privacy but means a serialized plan alone cannot reconstruct handler context; replay must always arrive through a fresh host request and prove equivalence through the fingerprint.

Adding a private invocation-context field closes external `CommandContext` struct-literal construction. Twill already constructs handler contexts internally, and the implementation will add a public constructor for tests and integrations that need an identity-free context, but this is still a source-level compatibility change for pre-1.0 callers that used literals.

Codex compatibility gives one host-specific field first-class normalization behavior. That is intentional migration support for an observation already shipped by a primary host, but it creates maintenance work if Codex changes the field. Keeping the canonical value authoritative and testing matching dual observations gives that migration a bounded exit.

Conversation identity is forgeable metadata for clients that control `_meta`. It is suitable for correlation and application tenancy within a host integration, not as authentication. Consumers that mistake it for an unforgeable principal could create a security boundary Twill never promised.

Fingerprint binding also means a permission approval created in one conversation cannot be reused after a host changes or loses identity. That is the desired safety property, but it can surface as a new approval request after host recovery even when command arguments are unchanged.

## Rationale And Alternatives

**Put identity in tool arguments.** This would make the value available everywhere the current argument pipeline already reaches. It would also make infrastructure correlation model-visible, force strict schemas to admit a reserved field, put the value into plans and examples, and invite agents to fabricate or retain it. MCP `_meta` and explicit host injection preserve the authoritative argument contract.

**Attach raw identity to `InvocationPlan`.** This would make dispatch and replay plumbing simpler because the value would ride the existing serialized type. Plans feed previews, responses, events, and debugging surfaces, so the convenience would turn a privacy rule into a convention every projection must remember. A separate non-serializing invocation context makes disclosure unrepresentable in those paths.

**Extend `RuntimeIdentity`.** Runtime identity describes the server process and catalog contract. Conversation identity describes the caller of one request. Combining them would give process-scoped data the wrong lifetime and make simultaneous conversations indistinguishable.

**Use process, connection, or stdio lifetime.** These facts can restart within one conversation or outlive it. Codex already supplies its persisted thread id per call, so deriving a weaker identity discards an authoritative observation.

**Make the declaration a hard requirement.** Some deployments have no conversation fact, and many applications already have an explicit protocol that remains correct. Optional context lets one command serve ambient and explicit clients while keeping absence visible to application policy.

**Let Twill choose explicit-argument precedence.** The framework has no general way to know which argument is an application session, whether an empty value is meaningful, or how the handle resolves. Providing both facts and leaving their policy to the consumer preserves Twill's reusable boundary.

**Silently prefer canonical metadata on conflict.** Authority defines which source would win, but a disagreement between two host observations is evidence of a broken adapter or stale injection. Failing before dispatch keeps one call from entering the wrong conversation while making the integration bug observable.

## Prior Art

Distributed tracing separates transport-specific extraction from canonical request context. W3C Trace Context and OpenTelemetry propagate correlation outside application parameters, normalize at process boundaries, and make conflicts or malformed carrier data an infrastructure concern. Conversation identity uses the same shape while explicitly declining the authentication semantics that tracing identifiers also lack.

MCP `_meta` is the protocol-level carrier intended for implementation metadata outside tool schemas. Codex's `threadId` is deployed prior art for attaching a persisted host conversation fact at that boundary.

Twill RFC 0009 is the closest framework precedent. It turns workspace observations into declared, handler-visible context and binds the resolved fact into invocation fingerprints. This RFC reuses that declaration-to-context path while defining optionality and a non-serializing privacy boundary for a correlation identifier.

Visible Browser Lab RFC 00011 is the first consumer contract. It demonstrates the division between framework identity and application sessions: Twill produces a validated ambient tuple, while VBL owns session minting, explicit-handle precedence, workspace binding, TTL cleanup, and browser lease arbitration.

## Unresolved Questions

No design question blocks Stage 1. Cross-project review must confirm that the canonical key, Codex normalization, optional declaration, redaction boundary, and application-owned explicit precedence match VBL RFC 00011 before either RFC advances. New issuers self-assign stable reverse-DNS names; Twill maintains no issuer registry.

## Future Possibilities

The canonical metadata convention could become a multi-implementation MCP proposal after more hosts and servers exercise it. Standardization could change the key or add protocol-level guidance while retaining the same `ConversationIdentity` application type.

Additional host lifecycle observations could accompany identity in request context and let applications reclaim state earlier than their TTL backstops. Lifetime signals need their own authority and failure contract; identity alone remains correlation rather than proof that a conversation has ended.

The non-serializing `InvocationContext` can carry other privacy-sensitive host facts if concrete consumers emerge. Each addition should repeat this RFC's discipline: explicit command declaration, narrow handler access, fingerprint binding where approvals depend on the fact, and projection of capability without projection of value.
