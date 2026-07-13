<!-- exo:10 ulid:01kwwy3rzsb7krrrm0pkyaf5s4 -->

# RFC 0010: Declared Preconditions

- Status: Accepted
- Area: command model, catalog, help, diagnostics
- Target milestone: v0.4
- Depends on: RFC 0001 (authoritative command surface), RFC 0002 (diagnostics and steering)

## Summary

This RFC gives commands a way to declare proof-bearing capabilities they require and commands that establish them. A capability is appropriate when the caller presents an opaque proof token whose validity remains application-owned, but the handler does not need Twill to resolve that token into a live server-held value. Declarations project into catalog, help, diagnostics, guidance, and every serving surface; missing carriers fail planning with establishment steering.

Enforcement does not move. The application that validates a build receipt, authentication proof, or other token keeps doing so. The declaration is a promise about what it enforces, exactly as a permission declaration is a promise about what the server will do. What changes is that the promise becomes visible and checkable.

RFC 0012 subsequently gave live server-held values a stronger structural model. Sessions, tabs, artifacts, and similar values are resources, not hand-declared capabilities: handler signatures derive their acquire/use/release/enumerate graph and the framework resolves them before dispatch. RFC 0012 emits a compatibility capability projection so existing help and steering vocabulary continue to work, while RFC 0016 may bind those resource carriers ambiently. A resource name therefore cannot also be hand-declared as a capability. This RFC remains authoritative for explicit non-resource proof carriers and for the compatibility vocabulary resources reuse.

## Motivation

The original motivating case was visible-browser-lab's session and owned-tab handles. At the time, the catalog saw only required strings and could not explain which commands established them or enrich a stale-proof failure with derived recovery. That gap produced the capability model implemented by this RFC.

The corpus learned more after that implementation. RFC 0012 established that VBL's handles are live server-held resources with identity, scope, lifetime, resolution, enumeration, and release. RFC 0016 adds surface-specific ambient binding for the root session resource. Current VBL therefore should not manually declare `session` or `owned-tab` capabilities or require model-visible carriers on every host. Resource signatures derive the same compatibility edges with stronger authority.

A narrower capability use remains distinct and useful. Consider a deployment server that validates a build and returns an opaque `validation_token`. A later publish command must present that proof, but the token does not identify a live object Twill should resolve into the handler; the deployment service validates it against current build state. Without a declaration, the catalog sees another string, help cannot derive the validation step, and every failure site rewrites “run build validate.”

RFC 0001's premise is that the command declaration is the single authoritative source for every projected surface. Explicit proof-token preconditions follow the same pattern as workspaces, types, and resources: name the concept once, reference it from commands, and derive establishment steering. The resource boundary learned since the first draft keeps that vocabulary honest rather than applying it to every stateful handle.

## Guide-Level Explanation

A deployment server declares a proof-bearing capability once, then references it from provider and consumer commands through the shipped builder shape:

```rust
let registry = CommandRegistry::build("deploy", "Deployment Service", |server| {
    server.capability(
        CapabilityDecl::new(
            "validated-build",
            "Proof that the selected build passed current validation",
        )
        .carried_by("validation_token"),
    );

    server.command("build validate", |command| {
        command
            .summary("Validate the selected build")
            .provides("validated-build")
            .handle(validate_build);
    });

    server.command("deploy publish", |command| {
        command
            .summary("Publish a validated build")
            .requires("validated-build")
            .arg(
                arg::string("validation_token")
                    .summary("Proof returned by build validation"),
            )
            .handle(publish_build);
    });
})?;
```

From those declarations the framework renders:

```text
Requires:
  validated-build  Proof that the selected build passed current validation
                   (carried by `validation_token`;
                    establish with `build validate`)
```

Provider roles are derived from `provides` together with `requires`. Adding another bootstrap validator updates help, diagnostics, native guidance, and generated hosts without editing recovery prose; a provider that itself requires the proof is labeled as refresh behavior rather than offered as establishment from absence.

For a live value such as VBL's session or tab, authors use RFC 0012 instead. `Res<Session>`, `Grant<Session>`, and related wrappers derive the resource lifecycle and its compatibility capability edges. RFC 0016 may then make the session carrier ambient or optional on a serving surface. Hand-writing `CapabilityDecl::new("session", ...)` alongside that resource is a registration error.

### What the framework checks, and what it does not

The framework validates the *declaration* and pre-validates the *call shape*. At registration it rejects a `requires` naming an undeclared capability, a required explicit capability whose carrier argument the command does not declare, and a hand-declared capability nothing provides or nothing requires. At planning time, a call missing `validation_token` fails with a diagnostic naming `validated-build` and `build validate` instead of a generic missing-argument error. Resource-derived compatibility capabilities follow RFC 0012's signature and lifecycle validation rather than these hand-declared provider/consumer rules.

What the framework does not do is verify the capability is *valid*. It cannot know whether a validation receipt still names the current build; that state lives in the application. A legacy handler can report a stale proof through the shipped `capability_denied` framework error. An RFC 0014 result-aware handler instead returns a declared application error whose command use is bound to the capability; the result compiler derives the same establishment operations without relabeling that expected refusal as framework failure. A condition needing framework resource resolution belongs in RFC 0012 instead.

`provides` is likewise a checked declaration-level promise, not automatic proof extraction. The framework proves that an in-catalog bootstrap provider exists and projects that operation as establishment steering; it does not infer a proof property from the provider's result, copy a returned value into a later call, or require the consumer's carrier name to be the provider's output-field name. The provider's output contract and adopter acceptance tests must make the proof available to the caller. RFC 0014 validates that output when adopted but does not invent a capability-to-property link. A typed proof-output wrapper needs a real non-resource adopter whose output shape can establish the right derivation rule; the initial contract does not guess one from the hypothetical example.

### How Agents Should Learn This

An agent that reads command help sees the proof requirement, its carrier, and the operations that establish it before making a call. Missing or stale proof produces the same establishment path from one declaration. Resource-backed requirements teach their richer enumeration, release, and ambient-binding behavior through RFCs 0012 and 0016 rather than pretending every precondition is an explicit token.

## Reference-Level Explanation

### Declaration

`CapabilityDecl` is a server-level declaration alongside `WorkspaceDecl` and `TypeDecl`. It is reserved for explicit proof carriers; a live server-held value uses `ResourceDecl`:

```rust
pub struct CapabilityDecl {
    pub name: String,
    pub summary: String,
    /// The argument name that carries proof of this capability on
    /// commands that require it.
    pub carrier: String,
}
```

`CommandSpec` gains two lists:

```rust
pub struct CommandSpec {
    // ...
    /// Capabilities this command requires (names of declared capabilities).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Capabilities this command establishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<String>,
}
```

The low-level and ergonomic surfaces preserve the existing consuming-model and mutable-builder conventions:

```rust
impl CapabilityDecl {
    pub fn new(
        name: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self;
    pub fn carried_by(self, argument: impl Into<String>) -> Self;
}

impl CommandRegistry {
    pub fn declare_capability(self, decl: CapabilityDecl) -> Self;
}

impl CommandSpec {
    pub fn requires(self, capability: impl Into<String>) -> Self;
    pub fn provides(self, capability: impl Into<String>) -> Self;
}

impl ServerBuilder {
    pub fn capability(&mut self, decl: CapabilityDecl) -> &mut Self;
}

impl CommandBuilder {
    pub fn requires(&mut self, capability: impl Into<String>) -> &mut Self;
    pub fn provides(&mut self, capability: impl Into<String>) -> &mut Self;
}
```

Both command surfaces dedupe: declaring the same requirement or provider twice is a no-op, not an error. Missing lists deserialize as empty and empty lists are omitted, so pre-RFC catalogs retain their exact serialized bytes and hash until a capability edge is declared.

The carrier is declared once, on the capability, not per command. A proof that traveled under a different argument name on every command would defeat the point of naming it. Resource carriers are likewise declared once on `ResourceDecl`, but their requiredness may be projected by RFC 0016 because the underlying resource—not this explicit-capability model—owns that behavior.

`CapabilityDecl::new` initializes the public `carrier` field to the empty
string so the fluent path can choose it with `carried_by`. The declaration is a
standalone public value, and `carried_by` is an ordinary visible-field
transformation: a later call replaces the earlier carrier exactly as direct
field construction would. Registration rejects a declaration whose completed
carrier is empty. Once non-empty, the carrier must name the same required
argument on every consumer; that `ArgSpec` supplies the argument grammar,
schema, and summary rather than the capability declaration duplicating them.

### Registration Validation

Registration (and the serving path) rejects hand-declared capabilities when:

- a `requires` or `provides` naming an undeclared capability;
- a completed capability declaration has an empty carrier because
  `carried_by` was never called or direct construction supplied an empty value;
- a command that requires a capability but does not declare the carrier argument, or declares it optional — the requirement means the proof must be present, so the carrier must be a required argument;
- a declared capability that no command provides — the catalog would name a capability with no way to establish it;
- a declared capability that no command requires — a dead declaration, following RFC 0008's dead-type rule;
- duplicate capability names;
- a hand-declared capability collides with an RFC 0012 resource-derived capability of the same name.

A command may both provide and require a capability when it refreshes an existing proof. A **bootstrap provider** provides the capability without requiring it; a **refresh provider** both requires and provides it. Registration requires at least one bootstrap provider. Plan-time repair, legacy denial steering, and RFC 0014 capability-bound recovery contain only bootstrap providers, sorted by canonical operation id, because a caller with missing or invalid proof cannot invoke a refresh provider. Help may list refresh providers separately, but never presents one as establishment from absence. A command that provides a capability does not implicitly require its carrier.

RFC 0012 resources project compatibility `requires`/`provides` edges but do not enter the explicit capability model's bootstrap-provider and dead-declaration checks. Resource signatures and the resolver, grant, enumeration, release, and expiry checks derive and validate that stronger graph. The argument-bound core catalog still derives and validates the resource carrier from `ResourceDecl`; RFC 0016 may change only that carrier's visibility and requiredness on a compiled native surface. A hand-declared capability carrier remains a required model-visible argument.

### Planning

Preconditions add one plan-time check: for each required capability, the carrier argument must be bound. Because registration already forces the carrier to be a required argument, this check subsumes the existing missing-required-argument failure for that argument — but replaces its diagnostic. The failure locates at the carrier argument and the steering names the capability and its establishing commands, derived from the declarations:

```
argument `validation_token` carries the `validated-build` capability,
which this command requires. Establish it with `build validate`.
```

The provider list is the canonical sorted bootstrap set. A self-dependent refresh operation remains visible in the catalog's `provides` graph and in help as refresh behavior, but never appears in this missing-proof diagnostic or another recovery path that starts without valid proof.

No resolution happens at plan time and nothing new lands on the plan. A capability is not a workspace: there is no observation to resolve, no root to select, and therefore no per-invocation variance for the fingerprint to cover. The requirement is a static spec fact, and the catalog hash — which covers the command spec serialization — is the identity surface that changes when requirements change.

### Runtime Errors And Result-Aware Migration

The shipped public variant and its convenience constructor remain source-compatible for legacy `CommandHandler` implementations:

```rust
pub enum FrameworkError {
    // ...existing variants...
    CapabilityDenied {
        capability: String,
        detail: String,
        carrier: Option<String>,
        providers: Vec<String>,
    },
}

impl FrameworkError {
    pub fn capability_denied(
        capability: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self;
}
```

A legacy handler that determines a presented capability is invalid—for example, a validation receipt names an older build—uses `FrameworkError::capability_denied` with the capability name and an application-supplied detail. The constructor initializes the compatibility enrichment fields empty. At the selected-command legacy adapter boundary, Twill verifies that the capability is hand-declared and required by that command, then replaces `carrier` and `providers` with the authoritative carrier and sorted bootstrap-provider set. The public fields remain available for source and wire compatibility, but values supplied through direct enum construction cannot override catalog steering. Naming an undeclared, resource-derived, or unrequired capability is an unexpected handler contract failure and produces the ordinary redacted `HandlerFailed` family rather than publishing invented precondition guidance.

An RFC 0014 result-aware handler uses the application-owned path instead. Its command error set binds one declared application error use to the required capability:

```rust
ApplicationErrorUse::new("validation_stale")
    .for_capability("validated-build")
```

RFC 0014 validates that the selected command requires that explicit capability and derives its callable recoveries from this RFC's sorted bootstrap providers. The handler returns its typed or dynamic declared application error, whose identity, message, details, and runtime recovery selection remain RFC 0014-owned. No carrier value enters that error automatically. Returning `FrameworkError::CapabilityDenied` through a result-aware handler is a handler contract defect. At that handler adapter boundary Twill replaces it with `FrameworkError::Handler("result-aware handler returned legacy capability denial".into())`, discarding the capability, detail, carrier, and providers before the error reaches a direct registry caller or response shaper. The model-visible result is therefore the ordinary static `HandlerFailed` family, and Twill never implicitly converts a framework error into an application outcome.

Before any legacy public response, Twill renders denial detail with a fixed
JSON-style encoder and a final bound of 512 Unicode scalar values. It emits
`\"`, `\\`, `\b`, `\f`, `\n`, `\r`, and `\t` for their ordinary JSON
characters; every other C0 control, DEL, C1 control, and the fixed
presentation-unsafe set U+061C, U+200E–U+200F, U+2028–U+202E,
U+2060–U+206F, and U+FEFF uses an uppercase four-digit `\uXXXX` escape.
Other Unicode scalars remain unchanged. Truncation occurs
only between complete input scalars after their complete escape. When the
rendered detail would exceed the bound, Twill reserves one scalar for `…` and
retains the longest complete escaped prefix whose rendered width plus that
marker is at most 512; the final public detail therefore never contains 513
scalars or a partial escape. Framework events and framework-owned logs record
only the capability code/name and declaration-derived steering, never the
detail. Response shaping applies the normalization even when downstream code
constructs the public enum variant directly rather than using the convenience
constructor. Result-aware application messages and details instead follow RFC
0014's declared schemas and bounds.

Escaping and bounding prevent structural/log injection; they cannot tell whether prose semantically copies a secret validation token. Capability validators are application code and must not include carrier values or other credentials in the detail. Application acceptance tests own that obligation. Resource resolvers use RFC 0012/0016 refusal and application-error paths instead.

### Projection

- **Catalog.** The server catalog carries hand-declared and resource-derived capabilities; each operation carries `requires` and `provides`. Resource origin is derived by matching the capability name to the catalog's `resources` inventory rather than adding a second mutable flag. The catalog hash covers all declarations and edges.
- **Help.** Explicit capabilities render their proof carrier, sorted bootstrap establishing operations, and any self-dependent refresh providers as a distinct role. Resource-derived entries defer to RFC 0012's richer lifecycle rendering and RFC 0016's active binding guidance rather than claiming the carrier is always model-supplied.
- **Preview.** Explicit capabilities add no per-invocation state beyond their bound argument. Resource binding source is the redacted RFC 0016 plan fact.
- **Native surfaces.** RFC 0015 translates establishing operation ids through direct/grouped mappings and rejects subsets that leave an exposed requirement without an exposed bootstrap provider; self-dependent refresh providers do not close that path.
- **Generated hosts.** RFC 0019 consumes the translated structured edge. Host text never spells a carrier or establishing tool when a structured operation/resource segment can render it.
- **Compatibility.** RFC 0012's derived capability projection preserves existing catalog consumers. It does not authorize hand-written `requires`/`provides` to override resource signatures.


### Contract Checks

A `check_capability_projection` rule joins the contract suite. For hand-declared capabilities it checks declarations, bootstrap providers, consumers, required carriers, catalog edges, and help. For resource-derived compatibility capabilities it verifies equality with the RFC 0012 resource graph and delegates lifecycle and carrier projection to `check_resource_projection` and RFC 0016's surface check. Registered in `contract_tests!` like the existing rules.

### Required Invariants

- Registration and serving both reject the invalid declarations listed above; a served surface cannot name a capability the catalog does not define.
- `CapabilityDecl::new` does not silently invent a carrier; completed declarations require one non-empty `carried_by` value, and repeated fluent assignment has the same visible replacement semantics as direct field construction.
- A call to a command that requires a capability, made without the carrier argument, fails at planning time with a diagnostic naming the capability and its establishing commands.
- Establishing commands in diagnostics and recovery are the canonical sorted bootstrap-provider subset derived from `provides`/`requires`; self-dependent refresh providers remain distinct and are never offered to a caller without valid proof.
- Adding, removing, or renaming a capability requirement changes the catalog hash.
- The framework performs no explicit-capability validity checks; application code owns proof validation.
- `provides` establishes a declaration edge only: Twill neither extracts nor stores a proof from the provider result, and no output-field convention is inferred from the consumer carrier.
- Public capability-denial detail is deterministically escaped and bounded, omitted from framework events/logs, and must not semantically copy proof-token secrets; carrier and provider steering are always replaced from the selected command's declaration graph.
- Legacy handlers retain framework `CapabilityDenied`; RFC 0014 result-aware handlers use capability-bound declared application errors, and the result-aware adapter replaces a framework `CapabilityDenied` with the fixed redacted `Handler` diagnostic before it can become anything except static `HandlerFailed`.
- Live server-held values use RFC 0012 resources. Their signatures and lifecycle validation—not explicit-capability bootstrap/dead-declaration rules—own the derived compatibility graph and validity.
- Only resource-derived capability carriers may become ambient or optional under RFC 0016; hand-declared capability carriers remain required.
- Establishing operation ids translate through native and generated-host surfaces without being rewritten as prose.


### Implementation Phases

1. Preserve the shipped explicit-capability declaration, validation, catalog, help, diagnostic, and contract APIs while documenting their non-resource boundary.
2. Reconcile capability projection with RFC 0012 so resource-derived names reuse the vocabulary without accepting hand-declared ownership or duplicate lifecycle authority.
3. Normalize capability-denial detail for public responses, replace compatibility enrichment fields from the selected command's declaration graph, reject undeclared/unrequired runtime denials as handler contract failures, and exclude application detail from framework events/logs.
4. As RFCs 0015, 0016, and 0019 land, extend their owned projections so establishment edges translate through direct/grouped surfaces and only resource-derived carriers may become ambient or optional. These delegated integrations do not turn the downstream resource model into this RFC's prerequisite.
5. Replace the public example with a non-resource proof-token workflow while retaining historical fixture coverage as compatibility evidence.

### Acceptance Tests

Acceptance lives in `crates/mcp-twill/tests/capabilities.rs`. The owner-local
landing proves explicit-capability declaration, validation, planning,
projection, diagnostics, hashing, and the non-resource boundary. Bullets that
explicitly name RFC 0014, RFC 0015, RFC 0016, or RFC 0019 are delegated to
`results.rs`, `native_surfaces.rs`, `ambient_resources.rs`, or
`host_adapters.rs` respectively. Those downstream suites translate the same
catalog-owned capability graph without adding proof storage or lifecycle
authority.


- A compact explicit-capability fixture (historically session-shaped in `capabilities.rs`, but with no `ResourceDecl`) registers providers and consumers, projects `requires`/`provides`, and renders derived establishment help. The RFC example teaches the same contract as `validated-build`.
- Equivalent low-level `CommandRegistry::declare_capability`/`CommandSpec` construction and mutable `ServerBuilder`/`CommandBuilder` authoring produce byte-identical catalog facts, help, hashes, and validation failures.
- Legacy `CommandSpec` JSON without `requires`/`provides` and explicit empty lists normalize to byte-identical catalog data and hash input; capability projection remains absent until adopted.
- Registration failures: `requires` naming an undeclared capability; `CapabilityDecl::new` left without `carried_by` or a directly constructed empty carrier; a requiring command missing the carrier argument; a requiring command with an optional carrier; a capability with no provider; a capability with no consumer; duplicate capability declarations. Each with a message naming the command and capability. Two `carried_by` calls retain only the second visible field value and validate consumers against that one value, matching direct declaration construction.
- The serving path rejects the same invalid registries.
- A call omitting the carrier argument fails at plan time with a diagnostic located at the carrier argument, naming the capability and every establishing command.
- Legacy `capability_denied` produces the same bootstrap steering; exact vectors cover every short JSON escape, remaining C0 controls, DEL, C1 controls, and every fixed presentation-unsafe scalar with uppercase `\uXXXX`, a complete escape at the 512-scalar boundary, and truncation that reserves the final scalar for `…` without ever producing 513 scalars or splitting an escape. The normalized detail is absent from framework events/logs, while a fixture verifies the application validator never copies the carrier value. Direct construction with spoofed carrier/providers is normalized back to catalog values, and naming an undeclared, resource-derived, or unrequired capability becomes a redacted handler contract failure.
- Adding a requirement to a command changes the catalog hash; removing it changes it back.
- `check_capability_projection` fails a registry whose command help omits a required capability, and passes the example server.
- The example server demonstrates a non-resource proof provider (`build validate`) and consumer (`deploy publish`), covered by `contract_tests!`.
- A provider fixture returns its proof under a field whose name differs from the consumer carrier and proves `provides` creates only the catalog/help edge: Twill does not extract, retain, rename, or inject that value. The adopter-owned follow-up call explicitly supplies the returned proof and exercises application validation.
- RFC 0012 acceptance proves a resource derives compatibility capability edges, rejects a hand-declared name collision, and keeps resource lifecycle validation authoritative.
- RFC 0015/0019 acceptance translates explicit and resource-derived establishment paths through direct, grouped, subset, and generated-host surfaces without raw-name prose.
- RFC 0016 acceptance proves ambient/optional projection is accepted only for resource-derived carriers; an explicit hand-declared capability carrier remains required.
- Missing-proof and legacy-denial fixtures with one bootstrap provider plus two self-dependent refresh providers project only the canonical sorted bootstrap set in diagnostics and callable steering; help labels the refresh providers separately and registration-order changes do not alter bytes or hashes.
- RFC 0014 acceptance binds a declared application error use to an explicit capability, derives the identical bootstrap-provider recovery set without authored `recover_with` entries, and returns `ApplicationError`. The same result-aware handler returning `FrameworkError::CapabilityDenied` is replaced by the exact fixed `FrameworkError::Handler` diagnostic above before direct-registry observation and yields static `HandlerFailed` with no capability, legacy detail, or steering on every serving surface.

## Drawbacks

**The declaration can lie.** A permission declaration is checked against nothing; an explicit precondition declaration is the same kind of promise. If `deploy publish` declares `requires("validated-build")` but stops validating the receipt, the catalog over-claims and no structural test can notice. Resource signatures reduce this risk for live values, but application proof validation remains a promise.

**The model is deliberately partial.** Explicit capabilities have establishment and requirement but no invalidation edge. A build change may make `validated-build` stale, but application validation—not a catalog state machine—detects it. Values that need acquisition, liveness, enumeration, and release use RFC 0012 resources instead.

**One more vocabulary.** The boundary must stay teachable: permissions describe what the server may do; resources are live values Twill resolves; explicit capabilities are proof tokens the application validates. A declaration that does not fit one sentence likely needs a richer existing layer.

## Rationale And Alternatives

**Status quo — prose and boilerplate.** Proof-token workflows otherwise name their validation command in server instructions, tool descriptions, and error sites. Structure removes repeated names, makes edges checkable, and lets each active surface render its own call shape.

**Argument-level semantic types instead of command-level requirements.** RFC 0008 or RFC 0017 could describe the shape of `validation_token`, but the routing fact is that `deploy publish` requires proof established by `build validate`. Argument shape cannot derive that command edge.

**A full protocol model.** The `requires`/`provides` graph could grow invalidation, expiry, and legal-order declarations. Rejected here because Twill cannot observe whether a new build invalidated a receipt, while RFC 0012 already owns enforceable resource lifecycle edges.

**Framework-side enforcement.** Twill could remember every validation token it observed. That view would be incomplete across processes and deployments and could not know when application state invalidated a token. The application remains authoritative; a live value Twill can resolve belongs in the resource layer.

**Free-text precondition slots.** A sentence such as “requires a receipt from build validate” recreates the drift problem inside structure. The establishing operation must remain a checked edge.

## Prior Art

**HTTP 401 and `WWW-Authenticate`.** The closest analog to `capability_denied`: a failure that carries, in-band, the machine-readable description of what credential was missing and how to establish it. This RFC's steering is the same move applied at plan time as well as runtime.

**OAuth scopes on API operations.** Published per-operation requirement metadata (`requires scope: repo`) that clients read before calling, with enforcement at the server. The same declaration/enforcement split this RFC adopts.

**CLI authentication flows.** `gh pr create` failing with "run `gh auth login`" is exactly the derived steering this RFC generates — except `gh` writes that hint by hand at each error site. The capability declaration is the factored form.

**Typestate and session types.** The `provides`/`requires` graph is a deliberately lightweight instance of encoding protocol structure in declarations. The restraint is the lesson: full session types earn their complexity when the compiler enforces them; a declaration surface that cannot enforce should describe less, honestly.

**Object-capability discipline.** RFC 0005 already distinguishes naming a capability from exercising it. This RFC gives the naming side a declaration; exercising remains behind the server's checks.

## Unresolved Questions

No architectural question blocks the initial explicit-proof boundary. Version 1 deliberately requires an in-catalog bootstrap provider, uses one uniform required carrier on every consumer, treats `provides` as a declaration promise rather than result extraction, and handles stale proof through application rejection plus derived re-establishment steering. External origins, typed proof outputs, invalidation edges, and per-command carrier aliases each change one of those choices and require adopter evidence rather than implementation discretion.

## Future Possibilities

A typed proof-output wrapper could connect `provides("validated-build")` to a declared result field without claiming a live resource. A real external-credential adopter could motivate an explicit out-of-band origin; repeated stale-proof workflows could motivate checked invalidation edges; and a surface that cannot maintain one carrier spelling could motivate explicit aliases. Each extension needs its own projection and migration evidence and must preserve the boundary among permissions, resources, and application-validated proof tokens.
