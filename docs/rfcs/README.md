# RFCs

This directory contains design proposals for Twill's catalog-authoritative MCP framework and its derived serving surfaces. An RFC is expected to read like a design proposal, not like a set of meeting notes. It should motivate a change, teach the model in the language an implementor or agent author would use, specify the reference behavior precisely, and then discuss drawbacks, alternatives, unresolved questions, and likely extensions.

The house style follows the Rust RFC distinction between motivation, guide-level explanation, reference-level explanation, rationale, and future work. It also follows Ember's emphasis on detailed design and teaching: names, examples, prompts, and diagnostics are part of the design because agents learn the framework through those surfaces.

## Project Values

All RFCs in this directory inherit the same core model. MCP is the protocol and control plane, while this framework provides a command-shaped contract inside MCP. The command string is a template over typed values, not a shell program. Pipes, redirection, command substitution, shell variable expansion, and shell control operators do not belong in the command string. If the framework later supports composition, filtering, globbing, streaming, or redirection-like behavior, those capabilities must be represented as typed framework features with explicit planning, diagnostics, permissions, and tests.

The default public MCP surface should remain small and predictable. The first implementation milestone exposed `help` and `run`; RFC 0005 refines that model into one discovery surface plus a small generated family of execution tools when different effect lanes need truthful MCP annotations. A compatibility embedding may deliberately select a larger native surface, but its names, schemas, routing, and identity must be compiled from an explicit profile over the same catalog rather than maintained as a parallel implementation. The catalog remains the authority in every profile. Help, examples, diagnostics, permission prompts, resources, prompts, generated schemas, and generated tests should be derived from or checked against the same command model that dispatch uses.

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
- [RFC 0009: Handler-Visible Workspace Roots](stage-1/0009-handler-visible-workspace-roots.md)
- [RFC 0010: Declared Preconditions](stage-1/0010-declared-preconditions.md)
- [RFC 0011: Guidance Decomposition](stage-1/0011-guidance-decomposition.md)
- [RFC 0012: First-Class Resources](stage-4/0012-first-class-resources.md)
- [RFC 0013: Conversation Identity Request Context](stage-2/0013-conversation-identity-request-context.md)
- [RFC 0014: Application Result Contracts](stage-0/0014-application-result-contracts.md)
- [RFC 0015: Catalog-Derived Native Tool Surfaces](stage-0/0015-catalog-derived-native-tool-surfaces.md)
- [RFC 0016: Ambient Resource Binding](stage-0/0016-ambient-resource-binding.md)
- [RFC 0017: Schema-Constrained Arguments](stage-0/0017-schema-constrained-arguments.md)
- [RFC 0018: Declared Invocation And Confirmation Presentation](stage-0/0018-declared-invocation-and-confirmation-presentation.md)
- [RFC 0019: Catalog-Derived Host Adapters](stage-0/0019-catalog-derived-host-adapters.md)
- [RFC 0020: Protocol-Versioned Task Delivery](stage-0/0020-protocol-versioned-task-delivery.md)

## Native Application Adoption Suite

The VBL-driven adoption RFCs divide authority by contract layer:

| RFC | Owns | Leaves To Another Layer |
| --- | --- | --- |
| 0009 | Required and optional workspace use, effective application-context metadata, request-only protocol controls, typed host-root provenance, and default-disabled trusted Codex compatibility | Application resource binding and host packaging |
| 0010 | Explicit non-resource proof preconditions and the compatibility vocabulary resources reuse | Resource lifecycle, ambient binding, and application proof validation |
| 0011 | Structured command selection, alternatives, fallbacks, and surface-neutral guidance edges | Preconditions, authorization, and host-specific packaging |
| 0014 | Successful application values, expected application errors, bounded messages, recovery declarations, typed producer footprints, and the narrow declaration-only command-emitter contract | Tool naming and transport-specific error shape |
| 0017 | Authoritative argument-property schemas, planning validation, and stable mismatch diagnostics | Whole-tool grouping and model-facing names |
| 0018 | Portable invocation status and confirmation presentation | Authorization decisions and proof of approval |
| 0015 | Host-neutral direct/grouped MCP tools, native dispatch, annotations, protocol-neutral task-support projection, results, confirmation routing, and the compiled protocol-bound surface snapshot | Task lifecycle/storage/access and editor manifests, opaque platform context, and process packaging |
| 0016 | Native-surface explicit or ambient resource selection, generated-host inheritance, declaration-only required-absence mapping, and dispatch-time realization | Effect-lane ambient authoring and application lease, TTL, ownership, and authentication policy |
| 0020 | Authored native disabled, legacy MCP 2025-11-25, and official Tasks Extension delivery profiles; fixed effect-lane legacy compatibility; negotiation, bounded storage and admission, access scope, state/result projection, retention, cancellation, and task disclosure | Configurable effect-lane delivery, command semantics, application result ownership, per-principal quotas, and host-specific task UX |
| 0019 | Host names, icons, prompt aliases, typed opaque context and runtime facts, fixed result shaping, profile-scoped pre-planning application uses, host-local confirmation authority, versioned host transport, and the compiled host artifact snapshot | Catalog semantics, command-owned application outcomes, base authorization policy, and application session policy |

### Authoring Path

Applications construct the suite in four layers. `ServerBuilder` and
`CommandBuilder` declare operation semantics, handler signatures, schemas,
results, guidance, workspace and identity uses, task support, and presentation.
`NativeToolSurface::builder` compiles selected operations into direct or grouped
tools and chooses serving-layer resource binding, confirmation routing, and task
delivery. `CliMcpServer::builder` binds that compiled surface to private runtime
sidecars such as confirmation bridges and task stores. A
`HostAdapterProfileBuilder` then consumes the immutable native snapshot to
compile host naming, packaging, context policy, result shaping, and transport;
host generators consume only that compiled host snapshot.

The similarly named methods stay on the layer that owns their decision:

| Authoring Question | Owning Method |
| --- | --- |
| May this command use deferred delivery? | `CommandBuilder::task_support` |
| How does this native surface deliver eligible work? | `NativeToolSurfaceBuilder::task_delivery` |
| What confirmation copy belongs to this operation? | `CommandBuilder::confirmation` |
| Where can this native surface obtain confirmation? | `NativeToolSurfaceBuilder::confirmation_route` |
| May this generated host's UI satisfy the server decision? | `HostAdapterProfileBuilder::confirmation` |
| How is a resource requirement supplied on this surface? | `NativeToolSurfaceBuilder::bind_resource` |
| How is a private binder attached to a deserialized ambient declaration? | `NativeToolSurfaceBuilder::attach_resource_binder` |
| What typed resource does the handler require? | `Res<T>` or `Option<Res<T>>` in the handler signature |
| Which handler path validates constrained arguments? | `handle_constrained`, or `handle_result` when the handler also declares application outcomes |
| Which handler path derives typed application success and errors? | `handle_result`, from its `ApplicationResult` or `ApplicationOutputResult` return type |
| Which handler path uses a runtime-shaped application value? | One explicit `result_contract` plus `handle_dynamic`; constrained properties reuse that same path |
| What produces the exact VS Code manifest projection and adapter source? | `generate_vscode_artifacts(compiled_profile.snapshot())` |
| How is a process host profile bound to its matching native server? | `HostProcessRouter::register` |
| How is an in-process host profile bound to its matching native server? | `HostAdapterProfile::bind_in_process` |

The suite preserves one authoring idiom across those layers. Extensions to the
existing `ServerBuilder` and `CommandBuilder` use their shipped mutable
`&mut self -> &mut Self` convention. Standalone declaration builders consume
and return their values for fluent composition. `Snapshot` names are reserved
for immutable, versioned compiler outputs; private binders, authorizers,
confirmation bridges, and host providers remain runtime sidecars and never
enter snapshot serialization or identity.

Each RFC states whether a fluent method transforms a visible declaration value,
assigns one finalization slot, or adds an ordered/keyed item. A finalizing
builder that can receive competing construction inputs diagnoses repeated
scalar assignments instead of granting the last call hidden authority;
semantic defaults do not count as authored assignments. A consuming method on
a public declaration value may instead replace a visible field or idempotently
select a value when its owning RFC says so, because the returned value exposes
the complete authority and retains no hidden authorship history. Additive
methods preserve order only where it is public and otherwise reject duplicate
keys. RFC 0014's standalone result/error declarations and RFC 0017's `ArgSpec`
schema refinements use the visible-value model before those values enter a
finalizing builder or registration boundary. A typed RFC 0014 result handler
derives its sole application contract; a dynamic result handler occupies one
explicit contract slot. Pairing an explicit application contract with a typed,
legacy, or constrained non-result handler is always a construction error, even
when the authored value would compare equal to the derived contract.

When a public declaration accepts a collection of strings, its ergonomic
constructor accepts `impl IntoIterator<Item = impl AsRef<str>>`, copies into
owned strings, and produces the same owned collection from literal arrays,
borrowed slices, and owned vectors. RFC 0015 explicit-subset exposure, RFC 0017
enumerations, RFC 0011 fallback preferences, and RFC 0019 process subcommands
follow that shared collection boundary, while their owning RFC still decides
whether order is semantic or canonicalized away.

These RFCs preserve existing serialized forms with defaults and omission rules where stated, and existing constructors/builders initialize the new facts. They are still a coordinated pre-1.0 Rust source migration: downstream struct literals for evolving public catalog types such as `CommandSpec`, `ArgSpec`, `OutputContract`, `InvocationPlan`, and `RuntimeIdentity` must add the new fields or move to constructors, and exhaustive matches on growing public enums must handle the new variants. Individual RFCs call out field-type and function-signature changes beyond that shared model migration.

A serde default is a compatibility spelling, not an alternate semantic or
identity. When an omitted legacy field and an explicit default mean the same
thing, compilation expands both into one normalized representation before
catalog, surface, or host snapshot hashing. Authored order remains significant
only where a caller can observe it, such as tool/member order, schema branch
order, guidance segments, or presentation copy. Facts an RFC explicitly
classifies as set-like or complete coverage are deduplicated and canonically
sorted at their owning compiler boundary. Contract tests compare omitted,
explicit-default, low-level, and ergonomic-builder paths whenever an RFC adds
such a field.

Unless an RFC defines a closed private transport spelling explicitly, new
public declarations follow Twill's existing serde convention: Rust
`snake_case` fields serialize as `camelCase`, and enum variant names serialize
as `camelCase`. A serialized enum with named variant fields therefore uses
`rename_all_fields = "camelCase"` in addition to its variant-level
`rename_all`; `rename_all` alone does not transform those fields. An owning RFC
must state any tag field or non-default enum representation rather than leaving
generated artifacts to infer one. JSON
Schema and canonical snapshot fixtures use the serialized spelling; Rust API
snippets continue to use the Rust spelling. Unknown-field policy remains owned
by each boundary: versioned host transports are closed, while additive catalog
declarations retain the repository's existing compatibility behavior unless
their RFC says otherwise.

Compiled `Snapshot` values are not serde wire types. Their owning RFC exposes
one normalized `document()` and one RFC 8785 `canonical_json()` byte sequence;
the Rust snapshot itself implements neither serialization, deserialization, nor
JSON Schema and has no public construction or mutation path. An owner may also
expose non-serializing, read-only typed accessors so same-language consumers do
not parse the canonical document. Those accessors and the document are built
from one completed internal representation and contract fixtures prove their
agreement. A downstream snapshot nests an upstream canonical document as a
JSON value and repeats its computed identity fields explicitly rather than
serializing the upstream Rust fields or reconstructing its document.
When a snapshot embeds exact external-protocol objects or capabilities, its
canonical document and typed accessors also name the exact protocol revision
used to compile them. A legacy connection negotiated to another revision
rejects before publication; a stateless request carrying another revision
rejects at ingress before method routing or result projection. Neither
observation selects or recompiles a different snapshot, so wire-version drift
cannot become an untracked adapter fact.

Snapshot versions identify complete compiler and generated-runtime contracts,
not only JSON document shapes. Every fixed behavior that can change wire
projection, dispatch, task lifecycle, or generated-host execution appears in
the canonical document or forces a snapshot-version increment, which changes
the framed hash even when authored declarations stay constant. Active RFC 0020
delivery members therefore include their task runtime contract version and
fixed stored-record bound. That version also freezes the private codec's exact
writer byte language and accepted decoder-version set, so either compatibility
change receives a new surface identity. RFC 0019 host snapshot version 1
similarly freezes callback order, encoding, cancellation settlement, process
cleanup, launch validation, and diagnostic-sink behavior; a semantic change
regenerates artifacts under a new host hash, and a process-wire change also
receives a new transport variant.

Compatibility evidence also keeps observation separate from adoption. The
versioned VBL bundle defined by RFC 0015 records what the released application
served: tool schemas, extension contributions, error vectors, and presentation
vectors with exact provenance. New Twill declarations live visibly in
`crates/mcp-twill/tests/support/vbl.rs` and are compared against those
observations. A released source file is never labeled a Twill declaration, and
a newly authored migration mapping is never labeled an export from the old
release. Normal workspace tests consume only checked-in bytes; fixture refresh
is an explicit local import and review operation.

External protocol fixtures follow the same observation rule. RFC 0020's MCP
task bundle records the exact core-specification and Tasks Extension repository
commits, protocol revision, SEP identity, extension identifier, derivation kind,
source paths, and payload hashes for every frozen wire case. Normal tests verify
the complete checked-in bundle without network access. A moving specification
URL is never protocol identity: refreshing a pinned source produces a reviewed
fixture/provenance diff, and a semantic change receives the new delivery,
snapshot, or runtime-contract identity owned by RFC 0020 before generated wire
behavior changes.

The same rule applies to error ownership. The released VBL error-vector bundle
is complete observation evidence; the Twill support fixture assigns each
vector exactly once to an RFC 0014 application declaration or to the precise
framework family now owning request-context, workspace, or planner failure.
Compatibility review therefore sees both preserved application behavior and
intentional ownership migration without turning framework errors into
application values. Raw workspace observation and path-containment failures
remain framework-owned; a broker session rejecting an otherwise valid selected
workspace remains the RFC 0016 binder's declared application outcome.

### Representative Adoption: VBL `new_tab`

`new_tab` is the suite's representative end-to-end adoption example. It creates
one browser tab, but its public contract crosses the layers that make a native
application more than a collection of MCP handlers: typed input and output,
resource custody, optional host workspace, ambient conversation binding, a
stable direct tool, ordinary or deferred delivery, and generated VS Code
projection. The example keeps each declaration on the layer that owns it.

The catalog declares the application operation. `Res<Session>` derives the
session requirement and its argument-bound `agent_session_id` carrier.
`Granted<Tab, _>` derives the new tab resource edge. Optional workspace use
allows the VBL binder to apply its application-owned session/workspace policy
when a host supplied a root, while keeping the browser operation callable when
no root was selected:

```rust
server.workspace(
    WorkspaceDecl::new("project", "file:///srv/default-project")
        .with_description("The project associated with this invocation"),
);

server.resource(
    ResourceDecl::new("session", "A leased browser session")
        .uri("vbl://session/{id}")
        .carrier("agent_session_id")
        .expiry("idle sessions expire and release their owned targets"),
);
server.resource(
    ResourceDecl::new("tab", "A browser tab leased to one session")
        .uri("vbl://tab/{id}")
        .within("session")
        .lifetime("live until closed, released, or the owning session ends"),
);

type NewTabApplicationResult = ApplicationOutputResult<
    Granted<Tab, ApplicationSuccess<NewTabResult>>,
    BrowserFailure,
    TabsNewErrors,
>;

async fn handle_new_tab(
    session: Res<Session>,
    ctx: CommandContext,
    args: NewTabArgs,
) -> NewTabApplicationResult {
    create_tab(session, ctx, args).await
}

server.command("tabs new", |command| {
    command
        .summary("Create a background browser tab owned by this session")
        .arg(
            arg::string("url")
                .summary("Initial URL for the new tab")
                .optional(),
        )
        .arg(
            arg::boolean("focus")
                .summary("Bring Chrome forward after creating the tab")
                .optional(),
        )
        .uses_optional_workspace("project")
        .task_support(TaskSupportSpec::Optional)
        .handle_result(handle_new_tab);
});
```

The command deliberately omits `uses_conversation_identity()`. Its handler does
not consume the raw tuple. A native surface instead declares that conversation
identity may satisfy the `Session` resource requirement. A VS Code-specific
surface omits the explicit carrier because that profile intentionally exposes
only the ambient conversation path; an MCP/Codex sibling surface uses
`with_optional_explicit_carrier()` to retain the established explicit fallback.
Both surfaces dispatch the same catalog operation:

```rust
let vscode_surface = NativeToolSurface::builder("vbl-vscode")
    .application_errors(NativeApplicationErrorDialect::FlatSingleRecovery)
    .framework_help(FrameworkHelpProjection::Omitted)
    .confirmation_route(NativeConfirmationRoute::Unavailable)
    .task_delivery(TaskDeliveryDecl::Disabled)
    .bind_resource::<Session>(
        AmbientResourceBinding::from_conversation_identity(
            SessionBinder::new(broker.clone()),
        )
        .omit_explicit_carrier()
        .missing_as("session_required"),
    )
    .direct("new_tab", "tabs new")
    .build(&registry, McpProtocolTarget::V2025_11_25)?;
```

RFC 0019 compiles the VS Code host profile from this immutable native snapshot.
Its handwritten context provider has one narrow job: convert the opaque
invocation token into RFC 0013 conversation identity and RFC 0009 workspace
observations. Generated code keeps the snapshot's `new_tab` schema unchanged,
so `agent_session_id` is absent by construction rather than removed by a
TypeScript transform. At dispatch, RFC 0009 selects any valid optional root,
RFC 0016 selects and later realizes the ambient session, RFC 0012 resolves it,
and the handler receives `Res<Session>` and returns the validated RFC 0014 value
plus its `Granted<Tab, _>` edge. Neither identity nor an internal session handle
enters the plan, response, host result, help, event stream, or log.

This surface chooses `TaskDeliveryDecl::Disabled`, so its default-optional task
support executes through ordinary delivery. Another compiled surface may select
one RFC 0020 delivery profile without changing the command, binder, handler, or
application result. Deferred delivery wraps the same invocation lifecycle and
stores the same validated terminal `CallToolResult`; it does not create another
`new_tab` contract.

The walkthrough is also the suite-level composition fixture. Owner-local tests
prove each declaration independently; the downstream RFC 0019 matrix must call
`new_tab {}` through the generated host with identity-only, identity-plus-root,
absent, and unsupported context observations. It proves the selected source and
workspace reach the binder, the handler sees only resolved values, ordinary and
explicit-fallback surfaces preserve their distinct schemas, and every public
projection remains free of raw identity and internal session handles.

## Shared Hash And Fingerprint Framing

Existing catalog hashes, invocation fingerprints, and RFC 0013 private
identity digests retain their shipped `stable_hash_value` JSON encoding. The
adoption suite does not silently reinterpret those existing bytes. New
versioned compiled snapshots use one cross-language framing rule. Let
`payload` be the RFC 8785 JSON Canonicalization Scheme bytes of the snapshot's
document, excluding its own hash field. The lowercase hexadecimal SHA-256 is
computed over:

```text
UTF8(domain) || 0x00 || U32_BE(version) || U64_BE(payload_byte_length) || payload
```

The fixed domains are
`io.github.wycats.mcp-twill/effect-lane-surface`,
`io.github.wycats.mcp-twill/native-tool-surface`, and
`io.github.wycats.mcp-twill/host-adapter`. Domain strings contain no NUL.
`canonical_json()` returns `payload`, not the framing prefix. Hash-vector tests
cover the empty/minimal document, non-ASCII strings, object-key reordering,
array-order preservation, version changes, domain changes, and payload-length
boundaries. Generated code embeds the computed hash and never reimplements
the framing algorithm. Snapshot compilation rejects a number that RFC 8785
cannot represent under its I-JSON/IEEE-754 constraints; it never rounds a
schema or declaration value merely to obtain canonical bytes.

Invocation fingerprints remain SHA-256 over Twill's existing stable JSON
fingerprint object. RFC 0015 adds one mandatory `invocation` member identifying
origin and serving surface. RFC 0017's canonical `schemaMatch` selections feed
the existing bound-argument input, while RFC 0018 and RFC 0016 add
`presentationContract` and `resourceBindings` only when applicable. Each owning
RFC defines the exact JSON shape and sorting rule. This changes fingerprint
values intentionally when the suite lands while preserving plan, event, and
response serialization; stored approval or replay records from an earlier
framework version do not survive that contract migration.

The identity layers have non-overlapping authority:

| Identity Layer | Owns | Includes | Deliberately Excludes |
| --- | --- | --- | --- |
| Catalog hash | Normalized server and operation semantics | Workspace/capability/resource/type declarations, guidance, result and argument contracts, command presentation, effects, and `TaskSupportSpec` | Serving tool names and grouping, protocol revision, runtime context, adapter sidecars, and invocation values |
| Native or effect-lane surface hash | One complete versioned MCP serving contract | Catalog identity, protocol revision, exposure and routing, projected schemas/help/instructions, application-error dialect, presentation defaults, RFC 0016 binding declarations, and RFC 0020 delivery/runtime-contract facts | Authorizer, confirmation-bridge, binder, resolver, task-store, and access-provider objects; request values and runtime task selection |
| Host adapter hash | One complete generated-artifact and private host-transport contract | The unchanged nested native snapshot plus host naming, guidance, confirmation policy, result projection, context gates, limits, transport, and accepted result-code inventories | Observed runtime version, invocation context, resolved launch paths/environment, cancellation tokens, application values, and reusable approval scope |
| Invocation fingerprint | One prepared operation attempt under one serving identity | Origin and surface, validated bound arguments and named/schema branch selections, effects and output request, selected workspaces, a declaring command's RFC 0013 identity digest, command presentation contract, and selected RFC 0016 binding facts/digests | Host adapter hash, rendered presentation, task id/state, immediate-versus-deferred choice, private sidecar objects, and raw metadata |
| Task runtime contract version | Task semantics that must remain compatible under one active delivery member | State transitions, per-record and per-runtime count bounds, atomic admission, oversized-outcome replacement, store/runner obligations, writer bytes, and the complete accepted decoder language; the value is embedded in and changes the surface hash | A standalone negotiation or lookup authority, store instances, access providers, task ids, and request-specific state |
| Task storage key | Opaque private lookup isolation for one retained task | Compiled surface hash, access-mode tag, task id, and any verified private scope digest under RFC 0020's fixed framing | A separately supplied catalog hash or document, public discovery, invocation fingerprints, model-visible authorization, and cross-surface migration authority |

A fact appears in the lowest layer that owns its semantics. Downstream layers
may embed an upstream identity or unchanged canonical document, but they do
not recalculate, reinterpret, or selectively copy its declarations. Runtime
objects and observations remain outside a hash unless an RFC explicitly turns
their normalized semantic result into a per-invocation fingerprint fact.

Every compiled surface embeds the complete catalog identity. Consequently,
any catalog change produces a new surface hash and changes fingerprints for
calls prepared through that surface, even when the selected operation's own
declaration is unchanged. Bare-registry execution has no surface hash and
changes only when one of its direct fingerprint inputs changes. Command-local
inputs such as RFC 0018's `presentationContract` therefore remain necessary
for bare execution and make the approved contract explicit on every origin;
they do not weaken the compiled adapter's conservative whole-surface boundary.

Cross-layer contract vectors mutate one fact at a time and prove this
propagation. “Changes” assumes the downstream surface, host, invocation, or
task key is rebuilt for the mutated input; “stable” means the layer has no
authority to absorb that fact.

| Single Mutation | Catalog Hash | Surface Hash | Host Hash | Served Fingerprint | Bare Fingerprint | Task Storage Key |
| --- | --- | --- | --- | --- | --- | --- |
| Unrelated catalog declaration | Changes | Changes | Changes | Changes | Stable | Changes through the surface hash |
| Selected command presentation declaration | Changes | Changes | Changes | Changes | Changes through `presentationContract` | Changes through the surface hash |
| Native surface declaration or snapshot version | Stable | Changes | Changes | Changes | Stable | Changes through the surface hash |
| Host profile declaration or host snapshot version | Stable | Stable | Changes | Stable | Stable | Stable |
| Equivalent private authorizer, bridge, binder, resolver, store, access-provider, or launch object | Stable | Stable | Stable | Stable unless its normalized selected invocation fact changes | Stable under the same condition | Stable for the same surface, task id, mode, and verified scope |
| Selected argument or normalized context fact owned by the invocation contract | Stable | Stable | Stable | Changes | Changes | Stable |
| Task runtime contract version or stored-record bound | Stable | Changes | Changes | Changes | Stable | Changes through the surface hash |
| Task id, access-mode tag, or verified private scope | Stable | Stable | Stable | Stable | Stable | Changes |

These vectors supplement each RFC's local hash tests. They prevent a future
refactor from “fixing” one layer by copying the fact into every downstream
identity, or from dropping the deliberate propagation that occurs when a
downstream hash embeds an upstream one.

RFC 0019's host adapter hash remains artifact and private-transport identity,
not invocation identity. Host context rejection happens before native
preparation, authorization, or replay consumption; fixed host result shaping
happens after the registry has produced its validated outcome. Calls that two
host profiles both dispatch over the same native surface therefore share a
fingerprint when their operation, arguments, and normalized typed context are
the same. A hash-covered host confirmation policy may satisfy one matching
base `RequireConfirmation` decision on its private entrypoint, but creates no
replay record and never widens a hard denial. Deployments that need execution
or approval isolation compile
distinct native surface identities rather than treating host packaging as an
authorization scope.

Process and in-process generated hosts return the same closed
`HostCallResultV1` carrying the compiled host and surface identities. This
detects stale or accidentally cross-wired runtimes before result projection.
It does not authenticate arbitrary embedding code: the configured executable
and the in-process hook are explicit trusted dispatch boundaries, and the
latter conforms only when it delegates to the bound Rust adapter. Generated
launch resolution similarly snapshots one exact plain-data deployment record
after call preflight. Replacing a captured hook member cannot redirect a later
call, while state intentionally read inside that receiver-bound method remains
application deployment behavior. Inherited fields and accessors on the returned
launch record never become a second invocation or deployment authority.

Generated VS Code registration is a separate activation boundary. Each
artifact exports one transport-specific, synchronous, one-shot registration
function. It captures provider/runtime methods with their original receivers,
registers the complete compiled contribution set, and transfers one composite
disposable to the extension context only after every registration succeeds.
Failure disposes the partial set in reverse order, continues past disposal
exceptions while preserving the activation failure, and permits a clean retry;
a call after success fails before observing replacement hooks. Registration
itself invokes no provider, runtime, sink, or application behavior.

Generated in-process cancellation also crosses that trusted hook boundary.
The bridge registers the host token before starting the bound Rust call, drops
the Rust future when cancellation finds it pending, and settles only after the
future has been dropped or completed. Generated code waits for settlement,
discards a racing result, and propagates host cancellation; it reserves
`HostContractMismatch` for hook
failure while the token remains live. Process transport instead owns
termination, escalation, and reaping directly. Neither path reports
cancellation complete while framework-owned dispatch work remains pending.

Generated process diagnostics use a separate bounded sink state machine. The
adapter offers at most the profile's child-origin stderr cap, keeps one copied
chunk in flight with no queue, and may add only the fixed 30-byte truncation
notice. Sink delay, mutation, throw, rejection, or non-settlement never blocks
pipe draining, cancellation, process reaping, or the tool result.

Generated host preflight also owns callback-local logical snapshot boundaries.
VS Code's `prepareInvocation` copies representable model arguments into one
private deeply immutable tree, renders RFC 0018 presentation, and retains no
approval state. Its later `invoke` callback independently snapshots its input,
then snapshots the typed provider result before runtime work. Construction uses
the profile's common call bound, version 1's fixed 128-container depth limit,
and the corpus RFC 8785 writer for canonical byte counting and finite binary64
number normalization. Registration also parses `vscode.version` once into a
closed private runtime fact. Process transport writes that captured value
directly. A conforming in-process hook forwards the received fact unchanged,
while the hook remains an explicitly trusted embedding boundary that could
substitute another value; the closed type and host hash detect drift but do
not prove provenance.
`ServerOnly` preserves server authorization. An explicit `TrustedVsCodeUi`
policy may satisfy only a base `RequireConfirmation` for which the same
compiled trigger matches and the runtime version lies in the hash-covered,
installed-tested range. Within each callback, no caller-owned object is reread
after snapshot construction, and generated code carries no approval flag.

The VBL host glue keeps workspace and deployment observations separate. A
supported opaque token owns its identity and optional `workingDirectory`; an
identity-only token never borrows the active editor's workspace. Only an absent
token may carry the active editor's workspace folder, then the first workspace
folder, while an unsupported token carries no partial facts. Independently,
the process launch resolver snapshots an absolute executable, `process.cwd()`,
and string-valued `process.env` entries, then applies the four reviewed VBL
setting overrides. These are typed host/application inputs, not model
arguments or implicit child-process inheritance.

## Shared Invocation Lifecycle

The adoption RFCs compose into one invocation lifecycle. The earliest failing
stage owns the public error, and no later stage runs. This ordering is part of
the cross-RFC contract rather than an adapter implementation detail:

RFC 0020 deferred delivery wraps this lifecycle after the outer MCP call
envelope identifies a known public tool route, validates the selected profile's
task controls, and decides that the call will materialize a task. Successful
atomic store creation is the task's linearization point before step 1; the
creation response always carries the retained revision-zero `working` seed,
while polling observes whichever later revision is authoritative. The receiver
runs the remaining stages in a private cancellable runner. An
undecodable outer call, malformed task control, unknown public tool name,
support mismatch, missing required extension capability, or task-access/store
creation failure creates no task. By contrast, malformed recognized invocation
context and an invalid grouped-operation selector are runner outcomes after
task creation and terminate that observable task through the selected
profile's ordinary result rules. Task existence conveys no dispatch authority:
only step 9 may move a prepared invocation into the authority-bearing execution
capsule.

1. The adapter validates its transport, performs any host-local route preflight,
   and normalizes recognized request context. A generated RFC 0019 `invoke`
   callback first constructs one private
   immutable logical argument snapshot and performs local direct/grouped route
   preflight. Only after that succeeds does it call its captured context provider
   once and snapshot the closed typed result before transport or runtime work.
   Cancellation or an unrepresentable, over-depth, or input-only over-bound
   host value stops before the provider; provider failure retains no partial
   context. RFC 0013 canonical identity and explicitly enabled compatibility
   observations, RFC 0009 trusted workspace metadata, and RFC 0019 host
   envelope/context integrity fail here before command planning. Raw context
   does not survive normalization. For RFC 0020 delivery, the already-created
   private runner owns one non-serializing deferred-input carrier, performs
   this normalization exactly once, and drops the raw metadata portion. Typed
   workspace observations and raw arguments, or their generated-host logical
   snapshots, survive only until the later
   routing/planning steps consume them; the task record, store, polling,
   cancellation, and retained-record paths never receive or reconstruct any of
   those inputs.
2. The active Rust surface authoritatively resolves the already-known public
   route and, for a group, its selector to one catalog operation. RFC 0019
   unsupported/absent-context
   gates then apply to that selected operation before its argument planner;
   their declarations are argument-independent by construction. A
   framework-owned rejection retains
   its host error family. The one structurally proven application-owned gate
   uses an explicit hash-covered profile declaration that references and obeys
   an RFC 0014 server-wide identity. It constructs the host result directly
   and never becomes a command `ApplicationErrorUse` or
   `CommandExecutionOutcome`.
3. The command planner validates the stdin contract, unknown arguments, and,
   for command-template origins, placeholder structure before checking missing
   arguments. Missing RFC 0010
   proof carriers and argument-bound RFC 0012 resource carriers use their
   declaration-derived diagnostics before the generic missing-argument
   diagnostic. It then binds supplied values: RFC 0017 schema checks precede
   named-type, path, workspace-containment, and resource-reference checks for
   each argument.
4. Planning selects required and optional RFC 0009 workspaces. A compiled
   native surface then selects RFC 0016 resource-binding sources: a supplied
   explicit resource carrier wins; otherwise an eligible ambient source or
   absence is recorded. Effect-lane and bare-registry entrypoints remain
   argument-bound and record no ambient binding facts. The completed public
   plan, private prepared invocation, and fingerprint are produced once from
   all selected public and private facts.
5. Wrong-lane routing consumes that prepared plan first and redirects without
   invoking application binders, resolvers, handlers, or availability gates.
6. On a correct-lane adapter call, an RFC 0016 required binding whose
   selected source is `Absent` produces its compiled static application or
   framework outcome from the completed plan. This availability gate runs
   for ordinary, deferred, permission-preview, and dry-run modes before
   policy, replay consumption, capsule creation, or application code; optional
   absence continues. RFC 0020 stores the same `isError: true` tool result with
   no execution capsule and applies only the selected profile's terminal
   status/result envelope; neither path realizes the missing binding.
   The application form is an RFC 0014 declaration-only command emitter: the
   selected command already owns the code, message/details policy, and
   recovery, while the hash-covered surface mapping contributes only the
   structural absence condition. It is distinct from RFC 0019's host-only
   profile use, which constructs no command outcome.
7. Dry run returns the completed plan before authorization, preserving the
   existing preview-only behavior.
8. The registry hard permission policy runs before the adapter authorizer.
   Denial stops there. A require-confirmation decision uses RFC 0018 prepared
   copy and RFC 0003/RFC 0015 replay binding for the exact fingerprint; approval
   never causes replanning of the accepted invocation. Permission-preview mode
   reports the decision without invoking a confirmation bridge or dispatching;
   `RequireConfirmation` includes the exact prepared confirmation in structured
   preview and validates the existing display hint as an exact title/message
   projection of that value. An RFC 0015 bridge request
   owns that preview and its `presentation()` accessor borrows the same nested
   value; no second prepared copy can disagree. Allow and deny include no
   confirmation presentation. A generated host's earlier
   `prepareInvocation` callback snapshots and routes the complete public input,
   then evaluates RFC 0018 over the direct arguments or the grouped member view
   with its surface selector excluded. It returns exact plain-string VS Code
   presentation fields and retains no approval state. The later invocation
   independently snapshots and routes its complete input before capturing host
   context. A `TrustedVsCodeUi` host profile may assert byte equivalence of the
   complete RFC 8785 snapshots—including any grouped selector—only when the
   captured runtime version lies in its tested range and the invocation-side
   compiled Twill confirmation trigger matches. The base
   authorizer still runs; hard or authorizer denial remains denial, and an
   unmatched requirement follows the compiled RFC 0015 route. Host context is
   captured from invocation options after the platform UI boundary.
   A deferred pre-dispatch denial, missing approval, confirmation
   cancellation, or bridge failure stores the same tool result through RFC
   0020 and retains no prepared authority.
9. Approved dispatch realizes RFC 0016 ambient bindings, resolves RFC 0012
   resources, and invokes the handler. Only here may RFC 0020 deferred delivery move the
   prepared invocation into a private authority-bearing execution capsule.
   Explicit or ambient
   refusal stops before handler execution, and no later source is tried.
10. RFC 0014 validates the materialized success or declared application failure
   before response shaping. A result-aware proof refusal may bind its declared
   application error to an RFC 0010 explicit capability, deriving only that
   capability's callable bootstrap providers; it never reuses the legacy
   framework-denial channel. Native, CLI-shaped, deferred, and generated-host
   projections consume that one validated outcome without relabeling its owner.

Approval authorizes an attempted dispatch, not a transactional outcome.
RFC 0003 single-use replay is consumed after the complete prepared fingerprint
matches and before application work; later binder, resolver, handler, result,
or transport failure does not restore it. Native bridge allow has the same
one-attempt meaning without a replay token. Cancellation is effect-free only
while the private prepared invocation has not entered application code. Once a
binder, broker, resolver, or handler accepts work, cancellation is best-effort
and cleanup/resource/idempotency declarations govern surviving effects.

RFC 0020 owns the public task state machine around this shared lifecycle. Legacy 2025-11-25 delivery is client-selected and maps `isError: true` tool outcomes to `failed` with retrieval through `tasks/result`. The official Tasks Extension is server-selected after per-request capability advertisement and maps every `CallToolResult`, including `isError: true`, to `completed` with its result inline in `tasks/get`. Every later extension task operation advertises that capability again; Streamable HTTP additionally routes it with `Mcp-Name` equal to the task id before Twill applies its separate access policy. JSON-RPC execution errors remain `failed` under the extension.

Before authorization, a task owns only a private cancellable runner. A pre-dispatch tool outcome stores the ordinary validated `CallToolResult` and creates no capsule. After authorization, the exact `PreparedInvocation` may move into one private execution capsule. Task records and stores never contain replay authority, invocation context, ambient references, raw arguments, handlers, or capsules, and Twill never reconstructs execution authority from a retained record.

RFC 0020 also owns profile-specific cancellation. Legacy cancellation races through one atomic terminal transition. Extension cancellation acknowledges intent and permits the eventual task to become cancelled, completed, or failed according to the actual winner. In either profile, cancellation before realization performs no application work; cancellation after a binder, broker, resolver, or handler accepts work is best-effort and makes no rollback claim. A server-instance task store has one exclusive live finalized-server owner; clones and request routers share that owner, and a persistent backend may be remounted only after the earlier owner exits. This makes a retained working record without a local runner evidence of an orphan rather than evidence of another conforming live worker. Active/active task execution needs a later lease protocol.

Task metadata uses only bounded framework-owned status messages. Application/framework error text, invocation presentation, arguments, private context, and result bytes never enter status text or task telemetry. Terminal application and framework outcomes remain available only through the selected profile's declared result surface.

RFC 0020 also fixes the complete private task-record cap at 1,048,576 bytes and
the mounted-runtime capacity at 256 retained records. Atomic creation checks
both key occupancy and capacity, and removal frees one slot. Every framework-
owned live runner first owns a working record, so the same count also bounds
those runners; it does not bound application-owned allocations or external
work accepted after dispatch. A fitting terminal outcome is stored unchanged. An outcome whose successor
record would cross that cap is discarded before store mutation and replaced
by the static `Task execution failed` JSON-RPC outcome; both task dialects
therefore expose a failed task without a partial or truncated application
result. The fallback itself is construction-proven to fit.

Task access remains outside ambient application context. RFC 0020 uses OS-random capability ids and no listing, with an explicit capability-id or embedding-derived transport scope policy. Request-local capability declaration and `Mcp-Name` are negotiation and routing checks rather than task authority. Conversation identity, workspaces, and arguments never become authorization scope. The opaque storage key also binds the exact compiled serving-surface hash, so another surface, catalog revision, protocol target, or delivery profile cannot probe a retained record through a deliberately shared store. A finite immutable expiration is the only lifecycle metadata exposed to an external store, allowing backend TTL cleanup without exposing semantic task state; after the deadline, delayed deletion remains privately retryable while every public operation returns the same unknown-task result. Its server-instance store, retention, expiry, and process-lifetime claims are explicit runtime configuration rather than implications of the command catalog.

The lifecycle order is per invocation, not a global serialization guarantee.
Different calls—including calls with the same operation, conversation
identity, workspace, resource, or fingerprint—may overlap in binders,
resolvers, handlers, confirmation bridges, and host transports. `Send + Sync`
on application sidecars is therefore a behavioral contract. Twill consumes a
single-use replay token atomically, but does not coalesce ordinary calls or
turn an idempotency declaration into a lock. Applications own atomic same-key
creation, broker deduplication, and cleanup of work accepted before
cancellation.

Acceptance suites may split these stages across RFC-specific files, but at
least one composition test must exercise the complete sequence and prove that
each earlier failure suppresses every later hook. This keeps new adapters from
acquiring a subtly different precedence merely by composing the same parts in
a different order.

## Shared Construction And Panic Boundary

Declaration callbacks such as error inventories, recovery sets, producer
footprints, and extractor metadata are pure deterministic construction input.
They perform no I/O, start no task, acquire no lease, and return the same value
for the same program. Framework construction may evaluate them more than once
across validation, contract tests, and equivalent builders. Concrete binders,
resolvers, authorizers, bridges, and handlers may be allocated before final
validation, but Twill never invokes them while compiling a registry, surface,
or host profile. A failed final build publishes no adapter and drops its
private sidecars. Ordinary Rust destructors still run when those owned values
are dropped; “never invokes” means Twill calls no binder, resolver, authorizer,
bridge, or handler method. Sidecar constructors and `Drop` implementations
must tolerate construction that never reaches publication.

Sealing a public dialect trait controls who may implement its extraction and
erasure hooks; it does not make the inferred marker parameter an internal
type. Rust must name that selected parameter while type-checking a downstream
builder call, even when the application never writes it. New dialects therefore
reuse RFC 0012's public inferred marker family wherever the handler shapes
agree. External-crate compile fixtures cover every supported shape and prevent
a crate-private marker from silently narrowing the public builder API. The
private seal is generic over that marker (`private::Sealed<M>`), which keeps
shape-specific blanket implementations disjoint while preventing downstream
implementations from forging framework extraction or erasure hooks.

A Rust panic is not an application error, framework diagnostic, or recovery
selection. Ordinary in-process library entrypoints follow the embedding's
panic/unwind policy; Twill does not catch a payload and turn its text into a
model-visible result. Boundaries that already isolate execution—a task runner
or generated child process—may report only their existing static redacted
infrastructure/contract failure when the isolated execution disappears. They
never expose panic payload, backtrace, or child stderr through a tool result.

## Shared Failure Ownership

The RFC that introduces a failure owns its framework variant, stable public
code, response status, safe details, and redaction rule. Later surfaces carry
that outcome without relabeling it. RFC 0020 preserves the underlying tool
outcome while applying its selected profile's task status/result envelope:
legacy `isError: true` becomes task `failed`, while the official extension
makes every returned `CallToolResult` task `completed` and reserves task
`failed` for JSON-RPC errors. Task status never changes the owning result
family. RFC 0019 generated hosts retain the owning application or framework
family and may apply only their declared bounded text projection.

| Owner | Condition | Stable Public Mapping |
| --- | --- | --- |
| RFC 0009 | Malformed enabled workspace metadata, dual raw/pre-resolved authority, or duplicate/unknown pre-resolved workspace ids | `InvalidRequestContext` / `InvalidInput`; only the recognized key, optional catalog-verified workspace, optional schema field, and stable reason may project |
| RFC 0013 | Malformed canonical conversation identity or malformed/conflicting compatibility observations under an explicitly enabled trusted-host policy | `InvalidRequestContext` / `InvalidInput`; only recognized source keys, optional schema field, expected grammar/version, and stable reason may project; the observed tuple remains private |
| RFC 0010 | A declared capability carrier is absent | Existing `CapabilityMissing` / `InvalidInput`; the catalog-derived carrier and canonical bootstrap providers remain authoritative |
| RFC 0010 | A legacy handler refuses a presented proof | Existing `CapabilityDenied` / `InvalidInput`; the catalog-derived carrier and canonical bootstrap providers replace handler-supplied steering, while refresh providers remain excluded |
| RFC 0014 | A declared application failure, including a result-aware handler's refusal of a presented explicit proof | `ApplicationError` / `Failed`; validated application code, details, and runtime recovery selection remain application-owned, while a capability binding derives only RFC 0010 bootstrap-provider operations |
| RFC 0014 | Unexpected handler failure or result-contract defect | `HandlerFailed` or `ResultContractViolation` / `Failed`; source text and rejected values remain private |
| RFC 0015 | Confirmation route unavailable, interaction canceled, or bridge failed | Corresponding `ConfirmationUnavailable`, `ConfirmationCanceled`, or `ConfirmationFailed` / `Failed`; no bridge or host reason projects |
| RFC 0016 | Required resource has no selected source and its binding selects `missing_as` | Command-owned `ApplicationError` / `Failed`; the hash-covered static mapping emits only the selected command's declaration summary, validated empty details, and declared recovery, with no application callback or new error use |
| RFC 0016 | Required resource has no selected source and no application mapping | `ResourceBindingMissing` / `InvalidInput`; resource, `absent`, and catalog-derived establishment operations only |
| RFC 0016 | Ordinary resolver refuses a private ambient reference | Existing `ResourceRefused` / `InvalidInput`; resource, `ambient`, and catalog-derived enumeration/establishment operations only |
| RFC 0016 | Ambient binder infrastructure fails | Existing `HandlerFailed` / `Failed`; the retained application source has static redacted `Display` and `Debug` and no error-chain source |
| RFC 0017 | Supplied argument violates its declared schema | Existing `InvalidArgumentType` / `InvalidInput`; schema-owned path, keyword, expected constraint, and branch problems only |
| RFC 0017 | Typed extraction disagrees with the registered schema | `ArgumentContractViolation` / `Failed`; operation, optional argument, and stable reason only |
| RFC 0019 | Host/profile/hash/envelope contract drifts | `HostContractMismatch` / `Failed`; no supplied hashes, envelope, or decoder text projects |
| RFC 0019 | A call or result exceeds a generated transport bound | `HostPayloadTooLarge`; caller-supplied call is `InvalidInput`, generated result is `Failed`, and only direction plus configured limit project |
| RFC 0019 | The trusted host context cannot support the operation | `UnsupportedHost` / `Failed`; profile-declared reason summary and optional host recovery only |
| RFC 0019 | A structurally proven absent-context gate makes an application's resource-establishment success unusable on that host | Host-only `ApplicationError` / `Failed`; one hash-covered profile use must reference an exposed RFC 0014 identity, obey its message/details policy, and provide only non-callable host recovery. It never becomes a command outcome or native-MCP error use. |
| RFC 0020 | An extension task operation omits or malforms its request-local capability | JSON-RPC `-32003` with exact required-capability data for omission, or invalid parameters for malformed capability; no task access or storage runs |
| RFC 0020 | A Streamable HTTP task operation has missing, repeated, malformed, or unequal `Mcp-Name` routing | JSON-RPC `-32602` / `Invalid task routing`; no access provider, task lookup, or store operation runs |
| RFC 0020 | Task access or record creation, including fixed-capacity admission, fails before a record exists | JSON-RPC `-32603` with static `Task access scope unavailable` or `Task creation failed`; no task id, record, capacity, occupancy, scope, or source projects |
| RFC 0020 | Task lookup authority is absent, invalid, expired, unavailable, or belongs to another compiled surface | JSON-RPC `-32602` / `Unknown task`; malformed ids, unknown ids, surface/profile mismatch, scope mismatch, and lookup-provider failure remain indistinguishable |
| RFC 0020 | A task-store operation is unavailable | JSON-RPC `-32603` / `Task storage failed`; the last committed record remains authoritative, backend text is private, and a live terminal candidate is reconciled before later projection |
| RFC 0020 | A validated terminal outcome cannot fit the fixed private record cap | The oversized candidate is discarded before store mutation and the fitting JSON-RPC `-32603` / `Task execution failed` outcome is committed at the same successor revision; no partial application result or resource reference projects |
| RFC 0020 | Scheduler, worker-join, capsule, or orphaned-runner infrastructure fails after task creation | Stored JSON-RPC `-32603` / `Task execution failed`; the selected task dialect supplies only its declared failed envelope, with no panic, source, capsule, or private invocation fact |

No application declaration may acquire one of these framework-owned
conditions merely to preserve an old spelling. Conversely, an adapter may not
collapse a declared application failure into `HandlerFailed`. RFC 0019's
profile-scoped use is an explicit application declaration on that host result
surface, not relabeling of a framework condition or invention of a command
outcome. RFC 0016's declaration-only emitter instead exercises an error use
already owned by the selected command and can contribute only its structural
absence condition; it cannot author application content. RFC 0014's capability binding likewise owns the application's refusal
of a present proof; RFC 0010 still owns missing-carrier planning and the legacy
framework-denial compatibility path. Events and framework-owned logs retain only the metadata explicitly
allowed by the owning row and the disclosure table below.

## Shared Disclosure Ownership

The same composition rule applies to data retention. A fact may cross only the
framework surfaces owned by its declaration; reaching trusted application code
does not silently authorize framework telemetry to retain it.

Static catalog/display prose and runtime application prose have different
boundaries. Authored guidance and summaries are validated once and rejected
when empty, over their owning RFC's bound, control-bearing, or
presentation-unsafe; accepted text then projects byte-for-byte. Runtime text
that an owning RFC deliberately permits is encoded with its fixed
complete-escape algorithm. Whenever such an RFC appends `…`, the marker counts
inside the declared final scalar bound. A downstream renderer may impose a
smaller complete-text bound, but it cannot recover a raw pre-encoding value or
split an escape merely to fit.

| Fact | Allowed Framework Destinations | Excluded Framework Destinations |
| --- | --- | --- |
| Catalog declarations, schemas, summaries, and static recovery/presentation declarations | Catalog, help, schemas, compiled snapshots, hashes, contract fixtures | Private runtime sidecars |
| Model-visible arguments and explicit resource references | Bound plans, previews, replay/fingerprint inputs, handlers, and declared responses where applicable | Framework events and framework-owned logs as raw values |
| Selected workspace roots | Plans, workspace previews, fingerprints, and handlers that declared the workspace | Raw observation maps, unrelated handlers, and implicit framework logging |
| Redacted surface/binding facts | Plans, previews, authorizers, events, and fingerprints where the owning RFC declares them | Raw identity, workspace observation, resource reference, or digest |
| Rendered invocation and confirmation presentation, source, and branch identity | Permission previews, trusted confirmation bridges, and generated host UI at their declared destination | Framework events and framework-owned logs; the initial contract records no presentation fact because a server cannot attest which pre-invocation host UI branch was shown |
| Validated application success and error bodies | Immediate tool responses, RFC 0020's private task store or live terminal-reconciliation candidate, and the declared terminal task result surface; RFC 0019 may carry only their declared final host text and family/code through its private result transport. A pre-planning RFC 0019 profile use may carry only its compiled static message, empty validated details, and non-callable host recovery on that host surface. | Framework events and framework-owned logs; an event may retain only status and a catalog-declared identity code plus the active profile when the use is profile-scoped. Raw bodies and error details do not enter the RFC 0019 result envelope. |
| Raw RFC 0020 task ids and `Mcp-Name` routing values | Selected task protocol requests/responses, private transport routing, storage-key derivation, and the private task-store codec | Plans, previews, fingerprints, snapshots, help, diagnostics, status text, framework events, and framework-owned logs; routing never substitutes for task access authority |
| Raw effective application-context metadata, request-owned progress tokens, and deferred tool arguments | One live RFC 0020 deferred-input carrier until normalize-once planning consumes the metadata and arguments; the progress token remains a separate runner-only protocol fact until that execution ends | Task records/stores/codecs, retained-record recovery, plans except validated bound arguments, fingerprints, events, `Debug`, public/model-visible schemas, generated artifact payloads, application context, and framework-owned logs; context fallback never creates progress authority |
| Concrete conversation-identity values, host-context values, observed host-runtime versions, ambient references/digests, logical host-input/context snapshots, prepared invocation carriers, host-envelope instances, and resolved host-launch values | Non-serializing invocation/prepared state, the explicitly versioned private host transport that must carry invocation facts, and the generated process-launch boundary for deployment facts | Plans except declared redacted facts, fingerprints, responses, previews, events, `Debug`, public/model-visible schemas, generated artifact payloads, and framework-owned logs. A generated private transport or launch interface may declare the fields it must carry but embeds no invocation or resolved deployment value. |
| Redacted framework diagnostics | Responses, events, and bounded framework logs | Raw source errors, validator/serde text, rejected values, provider exceptions, and private context |

Application handlers, binders, resolvers, brokers, confirmation bridges, and
embedding-owned stderr sinks are trusted application boundaries. They may apply
a deployment's own logging policy to values they legitimately receive. Twill
does not copy those values into framework events or logs on their behalf, and
generated adapters never turn raw process or decoder output into model-visible
errors.

## Native Adoption Migration Ledger

The suite is wire-additive where stated, but it is not entirely Rust-source
compatible. Twill is pre-1.0, so the RFCs choose one truthful API over parallel
legacy execution families. The implementation sequence uses this ledger as the
authoritative migration checklist:

| RFC | Rust Source Migration | Serialized And Runtime Compatibility | Adopter Action |
| --- | --- | --- | --- |
| 0009 | Public `CommandSpec`, server configuration, workspace-root structs, and workspace/error enums grow fields or variants; struct literals and exhaustive matches must update. Existing pre-resolved-workspace-plus-context calls may not also place host roots in `InvocationContext`. | Missing optional-workspace/provenance fields remain omitted and preserve existing workspace-specific fingerprint inputs. Codex sandbox compatibility changes behavior intentionally: it is disabled unless the embedding enables the trusted policy. Workspace and conversation-identity compatibility remain independent config fields; neither enables or validates the other. Dual raw/pre-resolved workspace authority now fails closed. A selected optional root, changed root, or changed source/issuer changes the complete invocation fingerprint. Effective metadata merging applies only to application context; progress and RFC 0020 controls remain sourced from the protocol request, and a context-only progress token is ignored. | Known Codex embeddings enable `TrustedCodexSandboxState` and, when they also need legacy identity, independently enable RFC 0013's `TrustedCodexThreadId`; direct trusted hosts inject validated roots through context-only execution paths; callers that supply a pre-resolved set keep host roots out of `InvocationContext`; ordinary servers do nothing. Preserve any request progress token separately when adapting the call, never inject one through context fallback, and discard approval/replay records whose selected workspace fact changes. |
| 0010 | Capability declarations and command fields are additive; public enum matches account for capability failures. Existing legacy handlers retain `FrameworkError::CapabilityDenied`; result-aware handlers use RFC 0014 instead. `CapabilityDecl::new` still requires a completed `carried_by` value before registration. | Missing/empty capability lists normalize identically. Existing explicit proof calls keep their carrier; provider roles now distinguish callable bootstrap from self-dependent refresh, and resource-derived compatibility edges remain RFC 0012-owned. Legacy denial detail now uses one display-safe complete-escape encoder and a final 512-scalar bound including `…`; ordinary safe detail remains byte-identical. Changing capability declarations or edges changes the catalog hash but adds no new per-invocation fingerprint fact beyond the already bound carrier argument. | Keep only non-resource proofs as hand-declared capabilities; give every declaration one non-empty carrier and a provider that does not itself require the proof; remove resource-name collisions and let resource signatures derive live-value edges. Keep legacy denial detail free of carrier secrets and rely on the bounded encoder only as a public-text safety boundary. Regenerate catalog-derived surfaces after changing the capability graph. |
| 0011 | Guidance and preamble fields are additive; public struct literals add their default values or move to builders. Mutable command/server builders now reject repeated scalar guidance assignments instead of retaining the last call. The fallback iterator item bound changes from `Into<String>` to `AsRef<str>` so borrowed slices remain a supported authoring form; custom item types implementing only `Into<String>` must add `AsRef<str>` or convert explicitly before calling `fallback`. | Missing guidance fields preserve existing catalog bytes and behavior. Safe guidance up to 1,024 scalars projects unchanged; empty, over-bound, control-bearing, or presentation-unsafe text fails registration. Adopting or editing guidance changes the catalog hash but adds no per-invocation fingerprint fact; guidance never changes dispatch legality. | Move routing prose into `use_when`, alternatives, fallback, variant fallback, and the server preamble; retain application prose that is not a structural route. Consolidate each mutable-builder scalar slot into one call, keep authored edge order intentional, and regenerate every catalog-derived serving or host artifact after changing guidance. |
| 0014 | Every public `CommandRegistry::run*` method changes from `Result<RunResponse>` to `Result<CommandExecutionOutcome>`; direct callers must match success versus application failure. `OutputContract`, `FrameworkEvent`, public error enums, `ApplicationErrorUse`, and `ApplicationErrorSpec` grow additive fields/variants. Existing `CommandHandler` signatures and defaulted RFC 0012 output wrappers remain source-compatible. Command builders that invoked more than one handler installer, repeated output presentation, or paired an explicit application contract with a non-dynamic handler now fail construction instead of using incidental last-write or equality behavior. | Legacy output/event fields and absent capability bindings remain omitted. Commands using one legacy handler and one output declaration preserve success behavior; result-aware commands add declared application outcomes without relabeling framework failures. Safe bounded-runtime messages remain byte-identical; controls and presentation-unsafe scalars use the exact complete-escape encoder, and the declared final bound includes any `…` marker. Model-visible `HandlerFailed` becomes a static empty-details infrastructure response, while direct Rust callers retain the original `FrameworkError`. Editing a result schema, error identity/use, message policy, or recovery edge changes the catalog hash and requires downstream surface and host snapshots to regenerate. | Migrate direct registry callers once; give every command exactly one handler installer and output presentation; use `handle_result` as the sole typed contract authority or pair `handle_dynamic` with exactly one explicit contract; move portable caller-visible text/cursor facts and intentional handler refusals into the declared application value/error contract. For an RFC 0010 proof refusal, bind the use with `.for_capability(...)` instead of repeating recovery operations or returning legacy `CapabilityDenied`. |
| 0017 | `ArgType::Integer` and diagnostic enums grow variants; `ArgSpec`, `ResourceDecl`, and `BoundArg` grow defaulted fields. Legacy typed handlers stay on `handle`; constrained typed handlers move to `handle_constrained`, while RFC 0014 handlers select checked extraction automatically. | Missing schema fields preserve coarse schemas, catalog bytes, and fingerprint input. Semantic constraint changes alter the catalog hash; a changed selected `oneOf` branch alters the canonical `schemaMatch` facts and invocation fingerprint without copying another argument value. Branches reached through local references retain their physical canonical declaration pointers rather than acquiring use-site or validator-generated expansion identities; mismatch-side branch pointers use that same address. `JsonInteger` contributes the exact inline property subschema without root-only Schemars annotations. | Preserve compatibility schemas byte-for-byte where required; use RFC 0008 for named record unions, RFC 0017 `named_schema` for reusable JSON Schema declarations, `inline_schema` for one-site values, and `JsonInteger` when typed extraction must accept the unconstrained integer subset of Twill's JSON number domain. Regenerate compiled surfaces after schema changes and discard approval/replay records whose selected branch identity changes. |
| 0018 | `CommandSpec` gains defaulted presentation fields and builders; presentation types are new. | Missing declarations preserve command catalog bytes and use the active surface's deterministic `Running <display-title>` / `Confirmation required` / `Run <display-title>?` fallbacks. A declaration changes catalog identity and the exact `presentationContract` invocation-fingerprint member; changing a surface-owned fallback changes the surface hash and complete served fingerprint. Reordering disjoint cases preserves selected copy but intentionally changes serialized/help order and identity. Presentation never changes authorization. Each host callback renders from one immutable logical argument snapshot; a separated invocation callback requires an explicit host value-equivalence contract before UI can stand in for approval. Rust and generated TypeScript share the frozen ECMAScript `TrimString` table and the exact quote-inclusive 256-scalar truncation budget. | Move portable invocation/confirmation switches into declarations; keep host trigger policy outside the command. Keep authored case order intentional, and discard approval/replay records after adopting, reordering, or changing command presentation or the active surface fallback contract. |
| 0015 | `CommandSpec` gains a defaulted task-support field and low-level/builder setters; `InvocationPlan::raw_command` becomes `Option<String>` and `InvocationOrigin` requires exhaustive handling; plan/runtime identity structs and framework error enums grow fields/variants. Existing `CliMcpServer` constructors keep the effect-lane default; native builders, base-authorizer configuration, and operation-id execution are additive, and native compilation requires an explicit `McpProtocolTarget`. Mixed `TaskSupportSpec` values inside one generated tool and success schemas that can accept a non-object now fail surface construction. | Omitted and explicit `Optional` command declarations preserve existing operation-catalog bytes and behavior; `Forbidden`/`Required` are explicit adoption. Existing command-template/effect-lane plan wire shapes remain unchanged. Omitting the new finalizing-builder authorizer slot retains `DefaultPermissionAuthorizer`; a supplied object remains private and unhashed. Native success retains matching object `structuredContent` and compact text; application/framework tool errors omit `structuredContent` and carry compact text so the success-only `outputSchema` remains truthful. Every invocation fingerprint gains the exact origin/serving-surface member, intentionally invalidating earlier approval/replay records; native surfaces carry a separate protocol-bound compiled identity including RFC 0020 delivery projection. Convenience server constructors remain limited to synthesized defaults and reject a surface requiring a bridge or extension task runtime. | Update direct plan consumers; declare non-default task support where required; make every generated tool's reachable operations task-support-homogeneous; keep scalar/array result contracts on CLI or generated-host surfaces unless a later native projection declares a truthful wrapper; discard pre-suite approval/replay records; compile through the single `build(&registry, target)` boundary; choose one native surface per adapter; build omission sets with `explicit_subset`; seed serialized declarations through `builder_from`; use the finalizing server builder whenever confirmation, task delivery, or authorization needs a private adapter sidecar; configure at most one base authorizer and configure a bridge only for a `Bridge` route. |
| 0016 | RFC 0012 extractor traits gain default methods, so existing implementors remain source-compatible. RFC 0015 native surface declarations and plans gain defaulted binding fields; new binders/resolvers/failure variants are additive. | Omitted and explicit argument-binding defaults compile to one normalized native surface/hash. Existing effect-lane, bare-registry, and argument-bound native resource execution remains the default. An ambient override or `missing_as` mapping changes native surface identity; the selected argument/ambient/absent source and any ambient private digest change the complete invocation fingerprint without adding the raw identity or reference to a plan. | Add ambient binding only to a native surface and let generated hosts consume that compiled snapshot. Fresh fluent authoring uses `bind_resource` to declare the mode and attach its binder atomically; a declaration loaded through `builder_from` uses `attach_resource_binder` so runtime setup never restates serialized policy. Keep effect-lane and bare-registry calls argument-bound. Use `missing_as` only for a code already declared by every required consumer whose static summary, empty details, and full recovery are valid for structural absence; use a binder or resolver for contextual application failures. Discard approval/replay records when adopting ambient binding or changing the selected logical slot. |
| 0020 | RFC 0015 native surface declarations gain defaulted `TaskDeliveryDecl`; native builders gain `task_delivery`, snapshots gain the compiled `task_delivery` accessor, and finalized task-enabled adapters gain one atomic task-runtime pair containing store plus access policy, along with task protocol methods. Public matches account for disabled, legacy, and extension compiled delivery views. | Omitted and explicit native `Disabled` normalize identically. Existing effect-lane constructors retain exact fixed legacy 2025-11-25 behavior on that negotiated revision and expose no delivery authoring path. Legacy delivery uses a negotiated-connection store; extension adoption is explicit, server-directed, protocol-bound, and uses an exclusively mounted server-instance store plus different result/cancellation semantics. The two scopes are not interchangeable. Runtime task choice does not change invocation fingerprints; an authored native delivery declaration changes surface identity. Retained task ids remain addressable only under the exact compiled surface hash, access mode, and owning scope that created them. Version-1 private records have a fixed one-MiB cap and each mounted runtime admits at most 256 retained records; the latter also bounds framework-owned live runners because creation precedes runner start. An over-cap record candidate becomes the static task-execution failure rather than a truncated result, while full-runtime admission becomes the static task-creation failure without disclosing occupancy. | Keep effect-lane legacy compatibility version-gated and let each separately negotiated legacy session own its connection-scoped default runtime; choose a native extension optional policy and bounded retention through `task_delivery`; inspect only the compiled snapshot accessor; for `TasksExtension`, install one exclusively mounted server-instance store plus capability-id or verified transport-scope access through one `task_runtime` assignment; share it only across clones and request routers of that finalized server, remount persistent storage only after the prior owner exits, and use disjoint namespaces until a future active/active lease protocol exists; require request-local extension capability on every task operation and `Mcp-Name` routing on Streamable HTTP; account atomically for the fixed 256-record namespace capacity and free a slot only on successful removal; drain or accept expiry of retained tasks before a catalog/surface/access migration; never use routing, conversation identity, workspaces, or arguments as task authorization. |
| 0019 | Host profile, confirmation-policy, snapshot, envelope, runtime-binding, exact in-memory artifact-generator, and generated runtime-hook APIs are new; host-specific error-code matches account for the new stable families. Every profile explicitly assigns common call/result limits, while process profiles additionally assign stderr and termination limits. | Core catalog/native behavior is unchanged. The version-1 host transport is intentionally closed and changes only through a new version. Generated preparation and invocation each snapshot model input locally; invocation additionally snapshots typed provider context and carries one registration-captured runtime-version fact before process or in-process delivery. Both transports use the same bounded RFC 8785 outer-envelope bytes and return the same identity-bearing result. In-process cancellation waits for the trusted bridge to drop or observe completion of the Rust future; process cancellation waits for termination and reaping. `ServerOnly` retains ordinary authorization. `TrustedVsCodeUi` is a hash-covered, range-bounded entrypoint policy that can satisfy only a trigger-matching base `RequireConfirmation`; it carries no approval flag or replay authority. Host-only changes invalidate the host adapter hash without changing invocation fingerprints; a changed nested native surface invalidates both hashes and the served fingerprint. | Declare the VS Code engine floor, confirmation posture, any exact installed-tested trusted range, and reviewed common/process limits; replace VBL's hand-written schema transform, routing, result filter, process wrapper, envelope/error formatter, and confirmation switches with generated artifacts; retain only opaque host extraction, application settings, atomic deployment launch resolution, an optional application diagnostic sink, in-process cancellation-to-future-drop glue when that transport is selected, and host activation glue. A `ProcessEnvelopeV1` deployment grants every caller able to invoke the configured subcommand host-context and runtime-fact authority and, under `TrustedVsCodeUi`, its narrow confirmation authority; choose the exact generated `InProcess` bridge or an authenticated launcher/channel when that caller set is broader than the trusted host. Regenerate generated artifacts after any host or nested-surface identity change. |

Every implementation PR must prove the row it changes at three levels:

1. Compile fixtures cover APIs claimed source-compatible and demonstrate the
   documented mechanical rewrite for intentional breaks.
2. Checked-in pre-change JSON fixtures deserialize, normalize, and reserialize
   according to the RFC's omission/default rule; explicit defaults yield the
   same canonical bytes and hashes.
3. Existing effect-lane and example-server tests remain green before new
   feature acceptance runs. The final RFC 0019/VBL gate regenerates, packages,
   installs, and exercises exact downstream artifacts after the hand-written
   compatibility layer is removed.

Public constructors and builders are the forward-compatible authoring path.
Downstream code that continues to use public struct literals or exhaustive
matches accepts the pre-1.0 maintenance cost recorded above; the framework does
not add duplicate legacy methods solely to preserve those forms.

## Suggested Implementation Order

RFC 0001 should land first because the catalog is the authority used by the other proposals. RFC 0002 should follow because diagnostics and response profiles make catalog failures usable by agents. RFC 0005 should be implemented before the permission workflow in RFC 0003 is finalized, because effect-lane routing changes the MCP-facing execution surface. RFC 0006 should land once the foundation API is stable enough to wrap, so new example servers teach the preferred authoring path before the preview and replay workflow adds more concepts. RFC 0003 then adds preview, confirmation, and replay on top of catalog effects and effect-lane routing. RFC 0007 should land before the workspace portions of RFC 0004 mature, because it gives path arguments and workspace identity a shared resolver contract. RFC 0004 can land incrementally because its runtime identity, workspace identity, event sinks, and generated contract tests are optional maturity features.

The current repository has two pre-existing lifecycle facts that this suite
consumes rather than reorders. RFC 0008's named-union implementation is already
in `main` while its Stage-0 reconciliation and promotion remain a separate
lifecycle pass; these RFCs use that shipped API without reopening RFC 0008.
RFC 0013 is implemented at Stage 2 and records RFC 0009 as its conceptual
workspace-context precedent. Because RFC 0013 shipped first, the current RFC
0009 reconciliation extends RFC 0013's existing private `InvocationContext`
container while retaining RFC 0009's sole ownership of workspace semantics.
That chronology requires neither an RFC 0013 edit nor a reverse transfer of
workspace authority, and RFC 0009 may advance through its own later lifecycle
gates without changing RFC 0013's accepted contract.

For native application adoption, RFC 0009 completes typed required and optional workspace context, RFC 0010 separates explicit proof capabilities from RFC 0012 resources, RFC 0011 supplies structured selection guidance, RFC 0014 makes application success and failure contracts authoritative, RFC 0017 makes rich argument schemas authoritative, RFC 0018 declares portable invocation and confirmation presentation, RFC 0015 projects those command contracts as direct or grouped MCP tools, RFC 0016 adds native-surface ambient resource binding inherited by generated hosts, RFC 0020 adds protocol-versioned deferred delivery around the completed invocation lifecycle, and RFC 0019 generates host adapters from the completed native snapshot. RFCs 0010, 0011, 0014, 0017, and 0018 precede RFC 0015 because a native tool needs exact precondition, guidance, input, result, and presentation contracts; RFC 0016 precedes RFC 0020's complete composition matrix, and RFC 0019 packages the resulting surface without re-deriving it.

Cross-references to later RFCs describe additive integration, not reverse
dependencies. RFC 0015 defines the native surface compiler and the extension
points RFC 0016 later fills with ambient binding; RFC 0018's command-owned
presentation is independently implementable before RFC 0015 carries it into a
surface snapshot. RFC 0014 owns the shared result-schema compiler, and RFC 0017
then applies the same machinery to model-facing argument properties under its
stricter ambiguity rules. RFC 0020 consumes RFC 0015's protocol-neutral
`TaskSupportSpec` and extends its existing surface builder, declaration, and
snapshot types with the final delivery slot while owning every runtime task
record and wire lifecycle; RFC 0015 never grows a fallback task state machine
or provisional delivery type.

Within this coordinated suite, `Depends on` is an acyclic prerequisite graph.
Reconciliation against a later RFC may narrow ownership, record migration
facts, or delegate integration acceptance, but it does not add that later RFC
to the earlier proposal's prerequisite header. The later proposal retains the
forward edge when it consumes or extends the earlier public contract. This is
why RFC 0012 depends on RFCs 0010 and 0011 while their reconciled bodies may
describe the stronger resource model that subsequently adopted their
vocabulary.

The dependency-safe implementation slices are concrete. RFCs 0009 through
0011 first land their owner-local reconciliations. An evidence-only RFC 0015
fixture bootstrap then lands the pinned VBL observation bundle, manifest
validator, and local-checkout importer without adding a surface compiler,
adapter, runtime API, or lifecycle advancement. RFC 0014 can therefore land
result contracts, RFC 0017 argument schemas, and RFC 0018 its portable renderer
against one provenance-checked corpus without acquiring a reverse dependency
on RFC 0015's public implementation. The RFC 0015 owner-local implementation
then consumes those APIs, including RFC 0018 surface integration, but exposes
ordinary native delivery only and introduces no RFC 0020 placeholder. RFC 0016 then lands ordinary
ambient binding over that prepared-invocation boundary. RFC 0020 is the single
downstream slice that adds delivery declarations and compiled accessors to RFC
0015, moves RFC 0016's already-prepared private authority into task capsules,
and owns every store, wire, and lifecycle API. RFC 0019 lands last over the
completed snapshot. Intermediate slices remain unreleased and their RFCs do
not advance beyond Stage 1; generated artifacts and frozen hashes are
regenerated at the RFC 0020 and RFC 0019 integration gates rather than
protected by temporary compatibility APIs.

### Landing And Lifecycle Gates

Implementation may land in the dependency order above without one monolithic
suite PR. An upstream implementation supplies its owned catalog/runtime
contract, canonical source projections, and owner-local tests. A later RFC
supplies the adapter, grouping, ambient-binding, or generated-artifact tests it
owns. Acceptance bullets that explicitly name that later RFC or its downstream
surface are delegated suite-integration obligations; they keep the final
projection truthful without turning the later RFC into a reverse `Depends on`
edge.

The acceptance owner is therefore fixed before implementation starts. The
owner-local suite proves the RFC's own public contract; the delegated suite
may only prove composition through a later surface. A delegated test cannot
invent an earlier RFC API, change its authority order, or relabel its failure
family.

Every `Required Invariants` bullet must be traceable to at least one explicit
acceptance bullet. When proof is delegated, that acceptance bullet names the
downstream RFC, suite, or application gate that owns it; a generally green
integration run is not a substitute for the named assertion. Conversely, a
downstream composition test cannot stand in for the upstream owner-local
contract or advance its lifecycle before that local evidence exists.

| RFC | Owner-Local Twill Suite | Delegated Integration Owner |
| --- | --- | --- |
| 0009 | `workspace.rs` | RFC 0020 `tasks.rs`; RFC 0019 `host_adapters.rs` |
| 0010 | `capabilities.rs` | RFC 0014 `results.rs`; RFC 0015 `native_surfaces.rs`; RFC 0016 `ambient_resources.rs`; RFC 0019 `host_adapters.rs` |
| 0011 | `guidance.rs` | RFC 0015 `native_surfaces.rs`; RFC 0019 `host_adapters.rs`; VBL selection evaluation |
| 0014 | `results.rs` | RFC 0015 `native_surfaces.rs`; RFC 0020 `tasks.rs`; RFC 0019 `host_adapters.rs` |
| 0017 | `argument_schemas.rs` | RFC 0015 `native_surfaces.rs`; RFC 0019 `host_adapters.rs` |
| 0018 | `presentation.rs` plus portable renderer vectors | RFC 0015 `native_surfaces.rs`; RFC 0019 `host_adapters.rs` and generated TypeScript vectors |
| 0015 | Evidence-only VBL fixture bootstrap, then `native_surfaces.rs` | RFC 0016 `ambient_resources.rs`; RFC 0020 `tasks.rs`; RFC 0019 `host_adapters.rs` |
| 0016 | `ambient_resources.rs` | RFC 0020 `tasks.rs`; RFC 0019 `host_adapters.rs`; VBL broker lifecycle evidence |
| 0020 | `tasks.rs` | RFC 0019 `host_adapters.rs` ordinary-fallback coverage |
| 0019 | `host_adapters.rs` | VBL exact generated-artifact and installed-host gates |

Each owner-local suite also owns compile coverage for the complete Rust
construction paths shown in its Guide-Level Explanation. An intentionally
partial fragment is labeled as such and cannot stand in for the complete
builder fixture. When a guide composes an API introduced by a later RFC, that
later RFC's suite owns the complete composition example; the earlier RFC does
not publish a provisional method merely to make the snippet compile sooner.

Core landing is not the same claim as lifecycle completion. An RFC remains at
Stage 1 while any acceptance obligation in its document still depends on an
unlanded downstream projection. After RFC 0019 and the VBL exact-artifact gate
provide the final coordinated evidence, eligible RFCs may receive separate
lifecycle-only Stage 2 promotions in dependency order. A promotion never edits
the accepted body to erase an integration obligation that was merely deferred
during implementation.

Stage 1 is also the implementation contract boundary. Before promotion, every
public Rust name, serialized spelling, default, normalization rule, failure
owner, and required migration must appear exactly in the managed body or its
named canonical vectors. Stage-0 review may propose another name or ergonomic
shape only by amending that body and rerunning its design gates. After Stage 1,
an implementation PR follows the accepted spellings and cannot introduce a
parallel convenience API, choose among unnamed alternatives, or treat an
`Unresolved Questions` paragraph as implementation discretion. A newly found
choice returns to RFC review rather than being settled invisibly in code.

The Stage-1 review records one checklist for each RFC:

1. The guide and reference sections name every public Rust construction path,
   serialized field, enum spelling, default, normalization, and rejection
   boundary that the first implementation needs.
2. Required invariants have explicit owner-local acceptance bullets or name
   the downstream suite that proves their composition, without reversing the
   prerequisite graph.
3. Failure and disclosure sections identify the layer that owns each public
   outcome and the values that must remain private across plans, events,
   diagnostics, logs, snapshots, and generated artifacts.
4. The migration ledger states source breaks, omitted/default wire behavior,
   fingerprint or hash invalidation, and the adopter rewrite for every changed
   public contract.
5. Implementation phases can land in dependency order without provisional
   public APIs, parallel authoring paths, or an upstream lifecycle claim based
   only on downstream evidence.
6. `Unresolved Questions` contains no decision delegated to the implementer;
   later extensions are bounded as future RFC work.

A Stage-1 review may point to a named canonical vector or shared ledger entry
instead of repeating it in prose, but each checklist item must have one
authoritative location. Passing code tests alone does not fill a missing design
decision.

RFCs 0015 and 0020 share an earlier gate: neither advances from Stage 0 to
Stage 1 until their paired review confirms one ownership boundary, exact
disabled/legacy/extension snapshot vectors, and no duplicated lifecycle or
store authority. The review must account for the official Tasks Extension's
server-directed materialization, inline result status, polling/update,
cooperative cancellation, request-local capability, Streamable HTTP routing,
access, and persistence model. A generic
reference to “MCP tasks” or SDK availability is not sufficient.
