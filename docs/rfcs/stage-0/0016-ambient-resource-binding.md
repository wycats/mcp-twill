<!-- exo:16 ulid:01kxby598d0anc4b287vc5srtg -->

# RFC 0016: Ambient Resource Binding

- Status: Draft
- Area: resources, ambient context, binding, planning, fingerprints, handler extraction
- Target milestone: v0.4
- Depends on: RFC 0009 (handler-visible workspace roots), RFC 0012 (first-class resources), RFC 0013 (conversation identity), RFC 0014 (application result contracts), RFC 0015 (native tool surfaces)

## Summary

This RFC completes RFC 0012's per-tier resource binding model for authored native surfaces. A native surface may satisfy a command's resource requirement from an explicit carrier argument, from private host context, or from an explicit-first combination of both. Generated hosts inherit the same behavior by consuming that compiled native snapshot. The command catalog continues to declare which resources an operation requires; the native surface declares how references arrive. Version 1 leaves the compatibility effect-lane profile and bare-registry entrypoints argument-bound.

Ambient binding has two phases. Planning selects a source and fingerprints the private binding fact without contacting the application broker or creating live state. Dispatch realizes the selected binding through an application-supplied binder and then runs the ordinary RFC 0012 resolver. This keeps previews and dry runs side-effect free while allowing a first real call to create or recover an application session.

The framework never inserts an ambient reference into bound arguments, plans, responses, events, help examples, or logs. A surface may keep the resource's carrier as an optional explicit override. When present, the explicit value is authoritative: invalid explicit input fails through the normal resolver and never falls back to ambient identity. When absent, the binder may derive a private reference from normalized conversation identity and selected workspace context.

## Motivation

RFC 0012 made resource requirements structural. `Res<Session>` and `Res<Tab>` derive catalog edges, argument carriers, resolution, help, and recovery. It shipped argument binding: a required session projects as `agent_session_id`, and the framework reads that argument before invoking the resolver.

Conversation-aware hosts can do better. Codex and VS Code already identify a persisted conversation outside model-visible arguments. Released VBL v0.4.8 maps that identity to an internal browser session, so ordinary `new_tab`, `snapshot`, and `close_tab` calls carry no session handle. An explicit `agent_session_id` remains available for clients without ambient identity and takes precedence when supplied.

Leaving that behavior in a VBL-specific adapter preserves duplicate authority. The resource declaration says a session is required, RFC 0013 says conversation identity is available, and a hand-written router privately chooses between them. The catalog cannot explain why the carrier is optional on one surface and required on another, fingerprints cannot name the selected binding source consistently, and every application must reimplement the same non-disclosure rules.

Naive ambient resolution also threatens planning purity. Mapping an identity to a session may start a broker, allocate a lease, or create application state. Preview, dry run, wrong-lane routing, and permission checks must not perform those effects. Twill needs to select and commit to a binding during planning, then realize it only after authorization permits dispatch.

## Guide-Level Explanation

The server declares resources and handler requirements exactly as RFC 0012 defines:

```rust
server.resource(
    ResourceDecl::new("session", "A leased browser session")
        .uri("vbl://session/{id}")
        .carrier("agent_session_id")
        .expiry("idle sessions expire and release their owned targets"),
);

type NewTabApplicationResult =
    ApplicationResult<NewTabResult, BrowserFailure, TabsNewErrors>;

async fn handle_new_tab(
    session: Res<Session>,
    ctx: CommandContext,
    args: NewTabArgs,
) -> NewTabApplicationResult {
    // `session` is resolved regardless of which binding supplied it.
    create_tab(session, ctx, args).await
}

server.command("tabs new", |command| {
    command
        .uses_optional_workspace("project")
        .handle_result(handle_new_tab);
});
```

`TabsNewErrors` includes the `session_required` use configured below and every code in `SessionBinder::ErrorFootprint` that can reach this command. The result-aware registration is part of the binding contract: an ambient binder that can produce application failures cannot be attached to a legacy consumer with no RFC 0014 application-error set.

An argument-bound surface needs no additional configuration. This includes the default effect-lane adapter, bare-registry calls, and a native surface with no ambient override. `agent_session_id` is required and accepts the resource id or URI.

A conversation-aware native surface selects explicit-first ambient binding:

```rust
let surface = NativeToolSurface::builder("vbl")
    .framework_help(FrameworkHelpProjection::Omitted)
    .confirmation_route(NativeConfirmationRoute::Unavailable)
    .bind_resource::<Session>(
        AmbientResourceBinding::from_conversation_identity(
            SessionBinder::new(broker.clone()),
        )
            .with_optional_explicit_carrier()
            .missing_as("session_required"),
    )
    // ...tool mappings...
    .build(&registry, McpProtocolTarget::V2025_11_25)?;
```

The command itself does not call `uses_conversation_identity()` merely because
this surface binds `Session` from identity. RFC 0016's compiled binder is the
private identity consumer; `handle_new_tab` receives only the resolved
`Res<Session>`. The command adds RFC 0013's declaration only if its handler also
needs the raw validated tuple. When the binder may use workspace to choose or
validate the logical session slot, the command separately declares
`uses_optional_workspace("project")`, giving the binder only RFC 0009's selected
root rather than raw host metadata. The corpus's [representative VBL `new_tab`
composition](../README.md#representative-adoption-vbl-new_tab) shows those
declarations together with the direct tool and generated host.

If a deployment loads the serialized surface declaration instead of authoring
it fluently, it attaches only the private binder sidecar. It does not restate
or reconstruct the declaration's carrier and missing-error policy:

```rust
let declaration: NativeToolSurfaceDecl =
    serde_json::from_slice(&stored_declaration)?;

let surface = NativeToolSurface::builder_from(declaration)
    .attach_resource_binder::<Session>(
        SessionBinder::new(broker.clone()),
    )
    .build(&registry, McpProtocolTarget::V2025_11_25)?;
```

`attach_resource_binder` succeeds only when the loaded declaration already
contains an ambient binding for that exact resource. The declaration remains
the sole authority for context source, explicit-carrier policy, and
missing-source behavior.

The generated schema still contains `agent_session_id`, but it is optional and described as an explicit fallback handle. A normal conversation-aware call omits it. Twill sees normalized conversation identity in private invocation context, selects the ambient source, and fingerprints the source without exposing the id.

At dispatch, `SessionBinder` receives the private identity and any selected optional workspace root. It asks the broker for the logical session slot associated with that identity, creating or rebinding an underlying lease if application policy requires it, and returns a private session reference. The ordinary `Session` resolver validates and loads the resource before the handler runs. Rebinding may change an ephemeral resource id, but it must not change the authority scope represented by that identity/resource slot.

If the caller supplies `agent_session_id`, Twill selects the explicit source and does not invoke the ambient binder. A stale, foreign, or malformed explicit handle fails; ambient identity cannot rescue it. This preserves application authority and prevents surprising cross-session fallback.

If neither source exists, a required `Res<Session>` returns the declared `session_required` application error before the handler. A command that deliberately adapts to resource absence may use an optional extractor:

```rust
type StartApplicationResult =
    ApplicationResult<StartResult, BrowserFailure, SessionStartErrors>;

async fn handle_session_start(
    session: Option<Res<Session>>,
    ctx: CommandContext,
    args: StartArgs,
) -> StartApplicationResult {
    match session {
        Some(session) => reuse_ambient_session(session, ctx, args).await,
        None => create_explicit_session(ctx, args).await,
    }
}

server.command("session start", |command| {
    command.handle_result(handle_session_start);
});
```

`SessionStartErrors` is exempt from `missing_as` coverage because structural absence reaches `None`, but it still covers every `SessionBinder::ErrorFootprint` code because a present ambient source may invoke that binder. If the binder itself can emit `session_required`, its producer footprint independently requires that code on the optional command. `Option<Res<T>>` declares optional consumption through the existing extractor family. It never creates a required carrier on an argument-bound surface. The compiled native profile decides whether an optional carrier is visible. Absence reaches the handler as `None`; a present selected source that refuses resolution remains an error.

### How Agents Should Learn This

On an ambient surface, help says that the session is selected by the host and that the explicit carrier is a fallback override. Examples omit the carrier. The model should call application operations directly and preserve an explicit handle only after receiving the declared missing-binding recovery and establishing one.

On an argument-bound surface, including the effect-lane compatibility profile, help continues to show the carrier as required. The command and resource names stay the same; only a compiled native profile may change how the requirement is satisfied.

Errors preserve authority. A missing source teaches the establishing command. A refused explicit handle teaches resource recovery and does not suggest deleting the argument to reach another session. An ambient realization failure reports the binder's declared application error; the producer contract forbids copying the conversation id or private resource reference into that application-owned value.

## Reference-Level Explanation

### Binding Model

Resource bindings belong to serving-surface configuration:

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBindingDecl {
    pub resource: String,
    pub mode: ResourceBindingMode,
}

pub struct NativeToolSurfaceDecl {
    // ...RFC 0015 fields...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_bindings: Vec<ResourceBindingDecl>,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ResourceBindingMode {
    Argument,
    Ambient {
        context: AmbientContextSource,
        explicit: ExplicitCarrierPolicy,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing_error: Option<String>,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum AmbientContextSource {
    ConversationIdentity,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ExplicitCarrierPolicy {
    Omitted,
    OptionalOverride,
}

pub struct AmbientResourceBinding<B> { /* private fields */ }

impl<B> AmbientResourceBinding<B> {
    pub fn from_conversation_identity(binder: B) -> Self;
    pub fn with_optional_explicit_carrier(self) -> Self;
    pub fn omit_explicit_carrier(self) -> Self;
    pub fn missing_as(self, application_code: impl Into<String>) -> Self;
}

impl NativeToolSurfaceBuilder {
    pub fn bind_resource<T>(
        self,
        binding: AmbientResourceBinding<impl BindAmbientResource<T>>,
    ) -> Self
    where
        T: Resource;

    pub fn attach_resource_binder<T>(
        self,
        binder: impl BindAmbientResource<T>,
    ) -> Self
    where
        T: Resource;
}
```

RFC 0016 adds `resource_bindings: Vec<ResourceBindingDecl>` to the native surface declaration and canonical snapshot. The serde default is a migration spelling for an RFC 0015 declaration: omission means every resource used by an exposed operation is argument-bound. Compilation expands that omission into exactly one effective entry per used resource, rejects duplicate, unknown, or unused entries, and sorts the completed list by resource name before storing the compiled declaration and snapshot. An explicitly authored complete list of `Argument` entries therefore compiles to byte-identical canonical data and the same surface hash as an omitted list. A non-empty authored list may contain ambient overrides without restating other resources; compilation fills their `Argument` defaults before canonicalization. No parallel resource-binding declaration is added to the effect-lane profile in version 1. Its private compiler continues to derive requiredness directly from RFC 0012 resource use.

The exact declaration wire forms are `{ "resource": "session", "mode": "argument" }` and `{ "resource": "session", "mode": { "ambient": { "context": "conversationIdentity", "explicit": "optionalOverride", "missingError": "session_required" } } }`. `missingError` is omitted when absent rather than serialized as null. These are additive catalog declarations under the corpus unknown-field policy; the compiler emits only the normalized known fields.

`AmbientResourceBinding::from_conversation_identity` requires the context source up front; before registration, the author must also choose either `with_optional_explicit_carrier` or `omit_explicit_carrier`. There is no implicit carrier-precedence default. Fresh declarations and declarations loaded through RFC 0015's `NativeToolSurface::builder_from` pass through the same completion step. Concrete binder objects live in a separate private runtime map keyed by resource name and never enter the declaration, snapshot, equality, or hash. Every effective ambient entry must have exactly one matching private binder sidecar, and no argument entry may have one.

The two builder methods have deliberately separate authority. `bind_resource::<T>(AmbientResourceBinding<B>)` is the complete fresh-authoring path: it records `T::NAME`, the serializable ambient mode, `B::ErrorFootprint::codes()`, and the erased `B: BindAmbientResource<T>` sidecar. It rejects any resource entry already present in a declaration seeded by `builder_from`, even when the proposed mode agrees. `attach_resource_binder::<T>(B)` is the rehydration path: it requires one already-declared ambient entry for `T::NAME`, records only `B::ErrorFootprint::codes()` and the erased binder, and never authors or changes context source, carrier policy, or missing error. It rejects a missing entry, an `Argument` entry, or a second sidecar. Therefore a loaded declaration has one serialized authority and never asks runtime setup to reproduce declaration data merely to prove equality.

The private ambient-binding builder records carrier policy and missing-source behavior as single-assignment slots. Repeating either carrier method, selecting both methods, or calling `missing_as` twice records a surface build error even when repeated values agree. The constructor's context source is immutable. This delayed-error shape preserves fluent `Self` returns while ensuring a call chain cannot acquire last-write authority. Attaching the completed binding twice for one resource remains the duplicate resource-entry error above.

`Argument` is the RFC 0012 default. A required `Res<T>` makes its carrier required; `Option<Res<T>>` leaves the same argument-bound carrier optional. `Ambient { explicit: Omitted }` removes the carrier from the active surface. `OptionalOverride` keeps the carrier property but removes it from the required set. The first version supports explicit-first precedence only; a surface cannot prefer ambient context over a supplied carrier.

The core resource and command catalog remains unchanged. The active surface projection records the binding mode, and the surface hash covers it. Help and schemas are generated from the combination of catalog requirement and surface binding. This refines RFC 0012's pre-implementation expectation that the catalog hash itself would cover binding mode: now that Twill distinguishes command semantics from serving-surface identity, the command catalog hash stays stable and the surface hash identifies the binding projection.

### Binding Plan

Planning selects a source without realizing a resource:

```rust
enum PlannedResourceBinding {
    Argument {
        resource: String,
        reference: String,
    },
    Ambient {
        resource: String,
        source: AmbientContextSource,
        private_fingerprint: [u8; 32],
    },
    Absent {
        resource: String,
        behavior: PreparedAbsentBinding,
    },
}

enum PreparedAbsentBinding {
    Optional,
    RequiredFramework,
    RequiredApplication(ApplicationErrorBody),
}
```

The public invocation plan does not serialize these enums. Argument references already appear in bound arguments. Ambient plans supply only the private `privateDigest` defined below, whose stable JSON domain names the binding, context source, and resource before the canonical identity tuple. Absence is represented publicly by its distinct source entry without a private value. Its private behavior is compiled for the selected command during preparation: optional continuation, the static framework missing family, or the already validated declaration-derived `ApplicationErrorBody`. The later execution gate consumes that retained choice and never re-reads a mutable surface or result declaration.

For authorization and preview, a surface-prepared plan carries only a redacted selected-source fact:

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct PlanResourceBindingFact {
    pub resource: String,
    pub source: PlanResourceBindingSource,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum PlanResourceBindingSource {
    Argument,
    Ambient,
    Absent,
}

#[serde(rename_all = "camelCase")]
pub struct InvocationPlan {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_binding_facts: Vec<PlanResourceBindingFact>,
}

#[serde(rename_all = "camelCase")]
pub struct PermissionPreview {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_binding_facts: Vec<PlanResourceBindingFact>,
}

#[serde(rename_all = "camelCase")]
pub struct FrameworkEvent {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_binding_facts: Vec<PlanResourceBindingFact>,
}

pub struct PlanFacts {
    // ...existing fields...
    pub resource_binding_facts: Vec<PlanResourceBindingFact>,
}
```

The active RFC 0016 native-surface preparer records one fact per resource binding it selected. Every effect-lane and bare-registry call remains argument-bound, keeps the lists empty, and preserves its serialized shape. A native optional-override call records `Argument` or `Ambient`; an optional extractor with no source records `Absent`. The fact contains no reference, identity, digest, workspace value, or binder state. It participates in the invocation fingerprint alongside the private binding digest, and lets `PermissionAuthorizer`, previews, trusted confirmation bridges, and events distinguish authority paths without receiving private context. `PermissionPreview` and `FrameworkEvent` copy the exact sorted facts from the prepared plan; `PlanFacts` carries that same slice into event construction without retaining the rest of the plan. The three serialized fields use the exact camel-case spelling `resourceBindingFacts`, default to empty when absent, and are omitted when empty.

The redacted fact remains serialized on native plans and framework events in the initial contract. Binding source is an execution and authorization fact—an explicit override and an ambient selection may carry different human meaning—even though the selected identity/reference is private. Keeping the three-value source makes previews and audit records self-describing, parallels RFC 0015's public surface identity, and does not disclose more than active help/schema already promises about available binding modes.

The registry/surface execution layer holds binding selections in a private prepared carrier rather than adding skipped fields to the public plan:

```rust
struct PreparedInvocation {
    plan: InvocationPlan,
    invocation_context: InvocationContext,
    resource_bindings: Vec<PlannedResourceBinding>,
}
```

`PreparedInvocation` implements neither `Serialize` nor `JsonSchema`, has a redacted `Debug`, and never crosses a public response or event boundary. Internal prepare entrypoints combine the registry with the active compiled-native binding policy and return the carrier to the adapter, which exposes only `&InvocationPlan` to lane checks, the authorizer, preview construction, and a native confirmation bridge. Approved dispatch consumes the same prepared value. The ordinary argument-bound public `build_plan*`, `run*`, and effect-lane adapter paths use the same carrier shape with no ambient policy and never rebuild source state. Native and generated-host runtimes use the ambient-capable operation-id surface preparer.

Replay prepares a fresh private state from the new host request, verifies its public fingerprint against the replay record, and dispatches that freshly verified state. Replay records therefore retain no raw context or ambient binding. For an RFC 0003 single-use record, validation and atomic token removal complete after every fingerprint/operation/lane check but before an RFC 0020 task may receive an authority-bearing capsule, binder realization, resource resolution, or handler dispatch. A later binder, resolver, handler, application-result, or transport failure never restores the token: approval authorized one attempted dispatch, not an indefinite retry capability. A reusable record remains until expiry but revalidates the complete freshly prepared fingerprint on every use. Invalid, expired, mismatched, or concurrently consumed records drop the new private state without calling application code; RFC 0020 stores the same pre-dispatch tool outcome with no capsule and applies its selected profile's status/result envelope.

RFC 0020 deferred execution moves the prepared carrier into a private execution capsule only after authorization and any confirmation/replay check succeeds. No public task record or terminal result contains invocation context, ambient selections, references, digests, or the capsule. Store/capsule loss, process lifetime, terminal projection, and recovery behavior remain RFC 0020-owned; this RFC only requires that authority is never reconstructed from events, plans, fingerprints, replay records, or public task payloads.

Wrong-lane, preview, dry-run, denial, confirmation cancellation/failure, invalid replay, pre-dispatch task cancellation, and private-capsule loss drop the carrier without realizing a resource.

Source selection runs after argument binding and request-context normalization but before permission checks:

1. Under `Argument`, a required `Res<T>` must have a carrier; `Option<Res<T>>` selects `Absent` when the carrier is missing.
2. Under `OptionalOverride`, the presence of the carrier key selects `Argument`; ambient context is neither delivered to this binding nor included in its binding fingerprint. An empty, malformed, stale, or foreign supplied value therefore follows ordinary argument/schema/resource validation and can never acquire ambient fallback authority.
3. Otherwise, a valid configured ambient observation selects `Ambient`.
4. Otherwise, both required and optional extractors record `Absent`. For `Res<T>`, the prepared invocation carries the compiled missing behavior to the pre-authorization execution gate described below; for `Option<Res<T>>`, absence is ordinary.

Private planned binding state records the selected source kind and a non-serializing binding fact, never the raw identity. Public and debug plans expose only `PlanResourceBindingFact`; explicit references remain visible only where they already occur as model-supplied bound arguments. Changing source, explicit reference, ambient identity, or presence changes the invocation fingerprint.

When at least one resource binding fact exists, the preparer adds one `resourceBindings` array to the existing fingerprint object, sorted by resource name. Argument and absent entries have the exact stable JSON shape `{ "resource": name, "source": "argument" | "absent" }`. An ambient entry has the exact shape `{ "resource": name, "source": "ambient", "privateDigest": digest }`, where `digest` is the lowercase 64-character SHA-256 string returned by the shipped `stable_hash_value` over `["ambient-resource-binding", "conversationIdentity", resource, identity.version, identity.issuer, identity.id]`. Argument entries need no second digest because the explicit carrier already occurs in bound arguments; absent entries carry no private value. The array is omitted when empty, preserving pre-RFC 0016 bare/effect-lane fingerprint input apart from RFC 0015's intentional serving-identity migration. Neither the array nor its private digest is added to `InvocationPlan`; only the final complete fingerprint is public.

For an ambient source, the fingerprint binds the logical resource slot `(context source, canonical identity, resource)` rather than an ephemeral reference that does not exist yet. A binder may rotate or recreate the underlying resource after expiry only when every resulting reference remains inside that same application-defined authority scope. It must not map one approved slot to another user's or workspace's resource. Permission targets and presentation therefore describe the logical ambient scope, not a lease id that may change between planning and realization. Applications that cannot provide this stability must keep the resource argument-bound or use a planning-safe concrete resolver.

This binding scope is not an authentication claim. RFC 0013 conversation identity remains forgeable correlation metadata unless the embedding supplies a separate trust boundary. The binder and ordinary resource resolver still own tenancy and ownership checks, and a deployment must not grant privileges solely because a caller asserted a canonical tuple. Ambient binding automates selection within the application's declared policy; it does not strengthen the credential quality of its context source.

An ambient resource binding consumes conversation identity privately. It does not automatically make raw identity available through `CommandContext::conversation_identity()`. A command that also needs the tuple declares `uses_conversation_identity()` under RFC 0013. This separation minimizes handler authority and prevents explicit override calls from fingerprinting an unused ambient identity.

RFC 0016 therefore adds one surface-owned consumer to RFC 0013's normalized `InvocationContext`; it does not weaken RFC 0013's command declaration. The adapter still normalizes recognized metadata once for every execution request and applies RFC 0013's malformed/conflict rules before routing. After operation selection, a compiled ambient resource binding may inspect the validated identity solely to select and later realize that resource binding. A command without `uses_conversation_identity()` still receives `None` in `CommandContext` and omits RFC 0013's `conversationIdentity` fingerprint member. When ambient binding is selected, RFC 0016 independently emits its domain-separated `resourceBindings[].privateDigest`; when the command also declares handler identity use, both facts are present because they authorize distinct consumers. With an explicit override, RFC 0016 emits only the argument-source fact, while RFC 0013's digest appears only if the command separately declared handler access.

### Dispatch-Time Realization

An application binder realizes ambient plans after lane checks, authorization, and replay validation:

```rust
pub trait BindAmbientResource<T: Resource>: Send + Sync + 'static {
    type Error: ApplicationError;
    type ErrorFootprint: ApplicationErrorFootprint<Self::Error>;

    fn bind(
        &self,
        context: AmbientBindingContext<'_>,
    ) -> impl Future<
        Output = std::result::Result<
            PrivateResourceReference,
            AmbientBindingFailure<Self::Error, Self::ErrorFootprint>,
        >,
    > + Send;
}

pub enum AmbientBindingFailure<E, F = AllApplicationErrorCodes<E>> {
    Application(ProducedApplicationError<E, F>),
    Infrastructure(AmbientBindingInfrastructureError),
}

impl<E, F> From<E> for AmbientBindingFailure<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: E) -> Self {
        Self::Application(error.into())
    }
}

pub struct AmbientBindingInfrastructureError { /* private source */ }

pub struct PrivateResourceReference { /* private validated id */ }

pub enum PrivateResourceReferenceError {
    Empty,
    InvalidCharacter,
}

impl AmbientBindingInfrastructureError {
    pub fn new(source: impl std::error::Error + Send + Sync + 'static) -> Self;
}

impl PrivateResourceReference {
    pub fn from_id(
        id: impl Into<String>,
    ) -> std::result::Result<Self, PrivateResourceReferenceError>;
}

impl<E, F> From<PrivateResourceReferenceError>
    for AmbientBindingFailure<E, F>
{
    fn from(error: PrivateResourceReferenceError) -> Self {
        Self::Infrastructure(AmbientBindingInfrastructureError::new(error))
    }
}

impl<E, F> From<AmbientBindingInfrastructureError>
    for AmbientBindingFailure<E, F>
{
    fn from(error: AmbientBindingInfrastructureError) -> Self {
        Self::Infrastructure(error)
    }
}

pub struct AmbientBindingContext<'a> {
    pub operation_id: &'a str,
    pub conversation_identity: &'a ConversationIdentity,
    pub workspaces: &'a [PlanWorkspaceRoot],
}
```

`ProducedApplicationError` and both failure enums implement the exact bounded `From<E>` conversions shown across RFC 0014 and this RFC, so a binder or resolver whose return type fixes `F` may return `error.into()` without constructing private fields. The conversion exists only when `F: ApplicationErrorFootprint<E>` and tags the value with that producer footprint; it does not choose or replace the active command's recovery-bearing error set. `AmbientBindingFailure` also implements the two infrastructure conversions shown above, so `PrivateResourceReference::from_id(id)?` works directly inside a binder returning that failure type.

`PrivateResourceReference::from_id` accepts exactly RFC 0012's mintable bare-id grammar: one or more URI-unreserved ASCII characters (`A-Z a-z 0-9 - . _ ~`). It never accepts a full URI, whitespace, controls, separators, or an empty value. `PrivateResourceReferenceError` has framework `Display` and `Error` implementations with static reason text; it names only `Empty` or `InvalidCharacter` and never retains or formats the rejected input. A binder already holding a full application URI must parse it through its own typed broker boundary and return the bare id; Twill later applies the selected resource's ordinary URI construction/normalization rules in one place.

The reference has no serialization or schema implementation and redacts `Debug`. Its id accessor remains crate-private: application code can create and return the value but cannot recover it through Twill after construction or use it as a second public carrier. The framework passes the validated id directly to the existing `ResolveResource<T>` implementation. The resolver's success enters `ResolvedResources`; refusals use RFC 0012 recovery edges.

Returning a private reference and reusing the ordinary resolver is the initial contract. It keeps explicit and ambient references under one liveness, ownership, and recovery implementation and lets a typed resolver expose the same application-error footprint on both paths. A binder that already holds a resolved application value may encode an opaque one-use reference in its private broker boundary, but Twill does not add a second binder-direct-to-`T` resolution dialect.

The existing explicit-reference `FrameworkError::ResourceRefused` includes the caller-supplied reference and resolver detail. That shape is unsafe for an ambient reference, so an ambient ordinary-resolver refusal uses a new redacted variant with the same public error family. Required source absence is a different, pre-realization condition and receives its own stable framework family:

```rust
pub enum FrameworkError {
    // ...existing variants...
    ResourceBindingMissing {
        resource: String,
        establish: Box<[String]>,
    },
    AmbientResourceRefused {
        resource: String,
        enumerate: Box<[String]>,
        establish: Box<[String]>,
    },
}

pub enum ErrorCode {
    // ...existing codes...
    ResourceBindingMissing,
}
```

`ResourceBindingMissing` maps to `ResponseStatus::InvalidInput` and `ErrorCode::ResourceBindingMissing`. Its static message says that the named required resource has no available binding; details contain only `resource`, `binding: "absent"`, and catalog-derived `establish` operation ids. Its steering actions use those operation ids through the existing retry-with-tool projection. There is no diagnostic argument location because no model-visible carrier was supplied or required.

`AmbientResourceRefused` maps to `ResponseStatus::InvalidInput` and the existing `ErrorCode::ResourceRefused`; it does not add a second public refusal code merely because the selected carrier was private. The framework discards the ambient private reference and `ResourceRefusal.detail` before response, event, or framework-log shaping. Public details contain only the resource name, `binding: "ambient"`, and catalog-derived `enumerate` and `establish` operation ids. Explicit argument resolution retains the existing `ResourceRefused { reference, detail, ... }` behavior because that reference was model-visible input.

Both variants are framework-owned tool outcomes. RFC 0020 retains either `CallToolResult` unchanged and applies only its selected profile's status/result envelope; task status never changes framework ownership. RFC 0019 host projection carries only the ordinary final framework family/code and bounded rendered text, not a private reference or raw diagnostic. Framework events and logs may name the operation, public error code, resource, and redacted binding source, but never identity, digest, reference, resolver detail, or establishing/enumerating response payloads.

`AmbientBindingInfrastructureError::new(source)` retains an application-owned error only for ownership and drop. Its `Display` and `Debug` are static and redacted, and its `std::error::Error` implementation exposes no `source`, so generic framework error-chain logging cannot recover application text. A binder that needs deployment diagnostics logs through its application-owned channel before constructing the wrapper. The wrapper maps to the existing `HandlerFailed`-family infrastructure response. Binders cannot return an arbitrary `FrameworkError` and thereby manufacture framework-owned planning, permission, or request-integrity failures.

The identity is non-optional because an ambient plan is created only after RFC 0013 normalization produced a valid observation. Missing identity selects `Absent` and never calls the binder. A future ambient context source with a different typed value must define its own binding context rather than weakening this guarantee into an optional bag of facts.

Binders may read only the selected workspace roots already present on the plan through the command's required workspace declarations, optional workspace declarations, or bound path arguments; they cannot inspect an undeclared or unresolved observation. `AmbientBindingContext.workspaces` borrows RFC 0009's canonical ascending-workspace-id plan slice, exactly as fingerprinting, previews, and handler lookup observe it. Binders may select by workspace id and may iterate deterministically, but declaration order, path-argument order, and pre-resolved input order cannot become binding policy. They receive neither raw request metadata nor model arguments. If a binder needs another host fact, that fact requires its own typed request-context RFC and source declaration.

When selected workspace context participates in the application's binding key, it is part of the logical slot and is already fingerprinted through RFC 0009. A binder may decline or return a declared error when the workspace no longer satisfies application policy at realization time; it may not silently substitute a different workspace.

Argument bindings skip the ambient binder and pass the bound carrier to the resolver. A refusal never retries another source.

The existing `ResolveResource<T>` remains the default and continues to produce framework-owned `ResourceRefused` failures. Applications whose resource protocol already has stable domain errors may opt into an additive typed resolver:

```rust
pub enum ResourceResolutionFailure<E, F = AllApplicationErrorCodes<E>> {
    Refused(ResourceRefusal),
    Application(ProducedApplicationError<E, F>),
}

impl<E, F> From<E> for ResourceResolutionFailure<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: E) -> Self {
        Self::Application(error.into())
    }
}

pub trait ResolveResourceWithErrors<T: Resource>: Send + Sync + 'static {
    type Error: ApplicationError;
    type ErrorFootprint: ApplicationErrorFootprint<Self::Error>;

    fn resolve(
        &self,
        reference: &str,
        plan: &InvocationPlan,
    ) -> impl Future<
        Output = std::result::Result<
            T,
            ResourceResolutionFailure<Self::Error, Self::ErrorFootprint>,
        >,
    > + Send;
}

impl ServerBuilder {
    pub fn resolver_with_errors<T>(
        &mut self,
        resolver: impl ResolveResourceWithErrors<T>,
    ) -> &mut Self
    where
        T: Resource;
}

impl CommandRegistry {
    pub fn with_resolver_with_errors<T>(
        self,
        resolver: impl ResolveResourceWithErrors<T>,
    ) -> Self
    where
        T: Resource;
}
```

`ResourceResolutionFailure::Application(error)` becomes an RFC 0014 application outcome only when the value's code belongs to the resolver footprint and the selected command declares that code, message/details contract, and selected recovery. Registration checks the code footprint against every command that can use the resolver. `Refused` preserves RFC 0012 behavior. This gives VBL a truthful path for `unknown_tab`, `tab_not_owned`, and `session_expired` without a surface rewriting generic framework failures.

The additive ergonomic entry point is `server.resolver_with_errors::<T>(resolver)`. Its low-level equivalent is `registry.with_resolver_with_errors::<T>(resolver)`, matching RFC 0012's existing `resolver`/`with_resolver` construction pair. Both paths record the same typed resolver and producer footprint, compile to the same catalog and runtime behavior, and reject a resource that registers both the existing plain resolver and the typed-error resolver. The concrete resolver names its error type and producer footprint through associated types.

Both public traits follow RFC 0012's existing return-position-future style. The public trait spells `fn -> impl Future + Send` so the `Send` contract is explicit and the crate remains clean under Rust's `async_fn_in_trait` lint. A downstream implementation may write the corresponding `async fn bind` or `async fn resolve`; Rust accepts that implementation against the RPITIT declaration, and this is the preferred warning-free authoring form when the body would otherwise return only `async move { ... }`. Implementations that return a meaningful named or composed future may retain the explicit return form. Their associated `ErrorFootprint` names the exact producer code inventory rather than requiring every command to accept every code in a broad application enum. Recovery declarations remain command-owned under RFC 0014. Registration wraps the concrete binder or resolver in a private erased adapter that boxes the future and erases the concrete types only after recording their resource and application-error footprint. The public traits do not need to be object-safe, and existing `ResolveResource<T>` implementations remain untouched.

`Send + Sync` permits real overlap. Twill invokes one binder and then one resolver at most once for each approved prepared dispatch, but it does not serialize or coalesce separate dispatches—even when their operation, conversation identity, logical resource slot, workspace, and complete fingerprint are equal. Two ordinary calls may therefore enter the same binder concurrently; process-per-call hosts may do so from separate processes. The application binder or broker must make logical-slot lookup/create/rebind atomic and keep every winning reference inside the planned authority scope. A canceled or failed call does not cancel a sibling or release a reference the sibling still owns. Single-use replay atomicity prevents duplicate consumption of that token only; it is not a general request-deduplication service. `CommandSpec.idempotent` remains a truthful retry declaration, not mutual exclusion.

The application-producer no-copy rule is necessarily a trusted-code obligation rather than a Rust taint guarantee. Code that receives a private identity can copy or log it before constructing any return type; preventing that mechanically would require reducing every application failure to framework-authored static text. Twill instead makes framework-owned disclosure paths structurally impossible, constrains producer code inventories and result schemas, redacts infrastructure/refusal paths, and requires each registered producer to pass adversarial non-disclosure fixtures. Applications remain accountable for values their trusted binders and resolvers deliberately publish.

RFC 0020 deferred execution moves the private prepared carrier, including its planned bindings and typed invocation context, into the execution capsule, then realizes the resource after the same authorization boundary. Cancellation before realization creates no application resource. Cancellation after a binder, resolver, broker, or handler accepts work is best-effort and makes no rollback claim; resource ownership, cleanup declarations, and idempotency remain authoritative for any surviving effect. RFC 0020 owns the profile-specific terminal race and prevents surviving application work from serializing the capsule or overwriting a terminal record.

### Optional Resource Extraction

Optional extraction composes the existing resource extractor with `Option`:

```rust
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional_resources: Vec<String>,
}

#[serde(rename_all = "camelCase")]
pub struct OperationSpec {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional_resources: Vec<String>,
}

pub trait ResourceParam: Sized + Send + Sync + 'static {
    // ...existing methods...
    fn is_optional_requirement() -> bool {
        false
    }
}

pub trait ResourceParams: Sized + Send + Sync + 'static {
    // ...existing methods...
    fn optional_resources() -> Vec<&'static str> {
        Vec::new()
    }
}

impl<T: Resource> ResourceParam for Option<Res<T>> {
    fn resource_use() -> ResourceUse {
        <Res<T> as ResourceParam>::resource_use()
    }
    fn is_optional_requirement() -> bool {
        true
    }
    fn extract(context: &CommandContext) -> Result<Self> {
        Ok(context.resources.get::<T>().map(Res))
    }
}
```

The new trait methods are additive defaults, so existing `ResourceParam`/`ResourceParams` implementations remain required and source-compatible. Twill updates its single-parameter and tuple implementations to aggregate optional names while preserving the existing `ResourceUse` shape. `Option<Res<T>>` derives `optionalResources: [T::NAME]` rather than `requiresResources`; `OperationSpec::from_command_spec` copies that canonical set into the public operation catalog and native snapshot. Both fields default to empty on deserialization and are omitted when empty, preserving existing catalog bytes. Registration rejects optional release semantics and still requires a resolver. A selected argument or ambient source is resolved; refusal remains an error. Only source absence becomes `None`.

Repeated uses in one signature deduplicate within a mode. Registration rejects the same resource appearing as both required and optional because the optional extractor could never observe absence, and rejects any optional/release combination. Authors choose one consumption mode per resource per command.

`Option<Res<T>>` does not grant, establish, or enumerate a resource. Those edges still derive from output types and other command signatures.

### Missing And Refused Bindings

An ambient binding may select RFC 0014's declaration-only command emitter for required source absence. `missing_as("session_required")` is the complete static producer footprint for this surface gate; it is valid only when every exposed command that *requires* that resource already declares the application code with `ApplicationMessageDecl::DeclarationSummary`, a details schema that accepts the framework-supplied empty object, and a declared-all recovery selection that satisfies its cardinality. Optional-only consumers are excluded because absence reaches them as `None` and the missing outcome cannot occur; a binding with no required exposed consumer rejects `missing_as` as dead configuration. Twill constructs the message, empty details, and declared recovery entirely from the selected command's joined spec; the binding adds no identity, `ApplicationErrorUse`, recovery edge, or runtime selection. Source absence has no application value from which to obtain runtime fields or choose among contextual alternatives. A richer, bounded-runtime, or choice-dependent error must be produced by a binder or resolver after a source exists. A surface cannot augment command result contracts implicitly.

Native surface preparation records `PlanResourceBindingSource::Absent`, completes the plan and fingerprint, and retains the compiled missing behavior only in `PreparedInvocation`. The public registry `build_plan*` methods and effect-lane adapter remain argument-bound and therefore never pretend to prepare an ambient surface; native and generated-host adapters use the ambient-capable operation-id preparer. An adapter first performs wrong-lane routing with that completed plan. On the correct lane, every execution mode—including permission preview and dry run—then applies the required-absence gate. It produces the configured `CommandExecutionOutcome::ApplicationError { plan, ... }`—or `ResourceBindingMissing` when no application code is configured—before dry-run success, registry permission policy, adapter authorizer, confirmation bridge, replay consumption, capsule creation, binder, resolver, or handler. RFC 0020 stores that same pre-dispatch tool outcome with no private capsule and applies only its selected delivery envelope. Optional absence is non-failing: dry run returns the completed plan, permission preview follows the ordinary registry-policy and adapter-authorizer path without realizing the resource or invoking a confirmation bridge, and approved execution reaches extraction as `None`. This is one execution availability gate, not a second planning pass.

The missing-source path remains RFC 0014's only version-1 declaration-only command emitter. Its mapping participates in the native surface declaration and hash. It runs after lane routing but before dry-run success, permission preview, or authorization so the framework can report that no required source exists without asking policy about work that cannot begin. It must not execute application code, inspect private broker state, or synthesize runtime details at that boundary. Applications needing contextual refusal first select a source and use the authorized dispatch-time binder or resolver path. RFC 0019's profile-scoped absent-context use is a different authority: it need not be a target-command error, constructs only a host result, and can never enter this `CommandExecutionOutcome` path.

Without a configured application error, absence produces the framework `ResourceBindingMissing { resource, establish }` variant defined above. Its public `resource_binding_missing` code, status, details, steering, task delivery, host projection, and redaction remain identical across ordinary execution, permission preview, and dry run. Raw identity and references are omitted.

Binder failures are expected application failures only when their codes are declared by the command's result contract. `Infrastructure` becomes a redacted `HandlerFailed`-family failure. Ordinary explicit resolver refusals remain `ResourceRefused`; ordinary ambient refusals become redacted `AmbientResourceRefused`; a typed `ResolveResourceWithErrors` may instead produce a declared application error at the application boundary. The initial implementation provides no framework-to-application compatibility mapping and no binder-to-arbitrary-framework-error path.

### Projection

- **Input schema.** Argument binding follows extractor requiredness: `Res<T>` marks the carrier required and `Option<Res<T>>` leaves it optional. Ambient-only omits it. Optional override includes the same derived reference schema but leaves it optional.
- **Catalog.** Core resource edges remain catalog facts. The active surface projects binding source, explicit-carrier policy, and missing behavior.
- **Help.** Requirements say `supplied by argument`, `supplied by host`, or `supplied by host; explicit override accepted`.
- **Examples.** Ambient examples omit the carrier. Explicit-fallback examples appear only in help for the missing-binding recovery path.
- **Preview.** The compiled surface declaration names the configured binding mode. Public preview copies the plan's resource and redacted selected source (`argument`, `ambient`, or `absent`). It never reveals the ambient identity, digest, or private reference.
- **Events and logs.** Framework-owned events may record the same redacted selected-source enum for auditability, never the identity, digest, workspace value, reference, or resolver prose.
- **Contracts.** `check_resource_binding_projection` verifies schema/help/example agreement, binding coverage, missing-error declarations, and non-disclosure.

### Required Invariants

- Planning and dry run never call an ambient binder, resource broker, or resolver with side effects.
- Explicit carrier presence wins over ambient context; explicit refusal never falls back.
- Framework-owned paths never serialize ambient references or their digests into model-visible schemas, arguments, plans, responses, events, help examples, or framework logs; ordinary ambient resolver prose is discarded with the reference.
- Source selection and the private binding fact participate through the exact sorted `resourceBindings` fingerprint member; only ambient entries carry the domain-separated private digest.
- Authorizers and confirmation previews receive the redacted selected-source fact from the exact prepared plan, never raw binding context.
- Authorization, bridge confirmation, and dispatch use one private prepared invocation; no post-approval replan can silently select another source.
- Single-use replay approval is atomically consumed after complete prepared-state validation and before application work; downstream failure never restores it, and replay records never retain private binding state.
- An ambient binder preserves the logical authority scope fingerprinted at planning even when it creates or rotates an ephemeral underlying reference.
- Ambient realization happens only after lane, authorization, and replay checks permit dispatch.
- Every ambient binding names a typed context source and has exactly one registered binder; fresh authoring records both through `bind_resource`, while declaration rehydration preserves the serialized mode and attaches only the sidecar through `attach_resource_binder`. Binders cannot inspect raw metadata.
- Ambient binding never upgrades conversation identity into authentication; application binders and resolvers retain all ownership and tenancy enforcement.
- Binding, resolution, and handler ordering is per dispatch; Twill never promises same-identity serialization or coalescing, and application sidecars own atomic logical-slot creation under concurrent ordinary calls.
- Every typed binder or resource-resolver application error is declared by the command being prepared; ordinary refusals remain framework-owned.
- `missing_as` is a hash-covered static code footprint over already-declared required-consumer errors; it cannot add a command error use or author runtime message, details, or recovery selection.
- Binder infrastructure failures are redacted as infrastructure and cannot impersonate planner, permission, or request-context framework variants.
- Binders and typed resolvers are application code with access to private facts; their `ApplicationError` implementations must not copy identity or ambient references into declared messages/details, and downstream application contract tests enforce that obligation for each registered producer.
- Required absence fails before the handler with declared recovery; `Option<Res<T>>` alone converts absence into `None`.
- The same command/resource catalog may be served with argument or ambient bindings; surface schemas and hashes distinguish them.
- Argument-bound behavior from RFC 0012 remains the default.
- RFC 0020 public task data never contains or reconstructs the private prepared carrier or execution capsule; store/runner loss follows RFC 0020's redacted profile-independent authority rule.

### Implementation Phases

1. Add surface resource-binding declarations, validation, schema/help projection, and surface hashing.
2. Add private planned bindings and fingerprint integration without public serialization.
3. Add ordinary dispatch-time binder realization, resolver composition, and non-disclosure checks through RFC 0015's existing prepared-invocation boundary.
4. Add `Option<Res<T>>` extraction, declared missing errors, contract checks, and owner-local VBL acceptance fixtures.
5. RFC 0020 later moves the already-prepared private binding authority into its execution capsule and proves deferred-delivery parity without adding another binder, resolver, or resource-declaration API.

### Acceptance Tests

Acceptance lives in `crates/mcp-twill/tests/ambient_resources.rs` and uses instrumented binders/resolvers plus a deterministic test broker to prove only Twill-owned selection, ordering, logical-slot fingerprinting, and non-disclosure. The VBL-shaped resource and binding declarations are new Twill inputs in `crates/mcp-twill/tests/support/vbl.rs`; they compare their carrier/schema/error projections with RFC 0015's validated `baseline-tools.json`, `surface-catalog.json`, `vscode-package.json`, and `application-error-vectors.json`. Bullets naming RFC 0020 deferred delivery are delegated to `tasks.rs` after this owner-local suite passes; the later task slice reuses this RFC's public binder API and prepared binding facts. Production lease reuse, TTL expiry, rebinding, and ownership enforcement remain explicitly labeled downstream VBL evidence rather than Twill framework acceptance.

- An argument-bound `Res<Session>` retains a required `agent_session_id` schema and resolves exactly as RFC 0012 specifies.
- Default effect-lane adapters and bare-registry entrypoints expose no ambient-binding authoring path, retain their argument-derived carrier requiredness, produce no binding-source facts, and never invoke a binder; a generated host acquires ambient behavior only from the compiled native snapshot it consumes.
- An ambient-only surface omits the carrier; an optional-override surface includes it as optional with the same derived reference schema.
- The guide's required `tabs new` and optional `session start` handlers compile through RFC 0014's single `handle_result` path. The required command declares the structural `session_required` use and both commands cover the binder's producer footprint; replacing either with a legacy handler or an incomplete error set makes surface compilation fail before publication or binder invocation.
- Registration rejects an ambient binding whose author has not explicitly chosen `omit_explicit_carrier` or `with_optional_explicit_carrier`, chose or repeated more than one carrier policy, or repeated `missing_as`; no constructor default or last-write behavior silently changes the model-visible authority path.
- An RFC 0015 declaration loaded through `builder_from` with no `resourceBindings`, one with an empty list, and one explicitly listing every used resource as `Argument` compile to the same sorted effective declaration, snapshot bytes, and surface hash. A deserialized ambient declaration plus `attach_resource_binder::<Session>(binder)` compiles to the same normalized declaration, snapshot bytes, and surface hash as the fresh `bind_resource` guide path. Attachment to an absent or `Argument` entry, a repeated attachment, an ambient entry without a sidecar, and fresh `bind_resource` over any seeded entry fail construction; the rehydration API has no mode parameter with which it could rewrite or disagree with the declaration. Duplicate, unknown, and unused serialized entries also fail construction.
- A direct `new_tab {}` call with conversation identity selects ambient binding, reaches a resolved session, and exposes neither identity nor internal session handle.
- Two identities produce different logical binding slots, while repeating one identity and selected-workspace set produces the same slot fingerprint even when the deterministic test binder returns a fresh ephemeral reference. Twill binds approval to that logical slot rather than the reference and never exposes either private value.
- Downstream VBL acceptance proves the application broker's separate policy: distinct identities isolate production sessions, a repeated identity reuses its live lease, TTL expiry may rebind that same identity to a fresh lease, and every resulting reference remains inside the selected identity/workspace authority scope.
- A barrier-controlled binder receives two approved ordinary calls for the same identity/resource concurrently, proving Twill adds no hidden same-key lock. An atomic test broker returns references in the same logical authority scope, canceling one call does not cancel or release the other's work, and a deliberately unsafe fixture demonstrates why binder-owned same-slot coordination is required rather than supplied by the framework.
- A binder contract fixture attempts to return a reference outside the planned identity/workspace slot and proves the test broker rejects it as application policy; Twill neither treats the logical digest as authentication nor silently substitutes another source.
- A supplied explicit handle wins over ambient identity. A stale or foreign explicit handle fails and the ambient binder is never called.
- A supplied empty or schema-invalid explicit carrier fails through the explicit argument/resource path and never calls the ambient binder; omission alone makes the ambient source eligible.
- A VBL typed resource resolver preserves declared `unknown_tab`, `tab_not_owned`, and `session_expired` outcomes; the ordinary resolver dialect still produces `ResourceRefused`.
- An ordinary resolver that embeds an adversarial ambient reference in `ResourceRefusal.detail` produces redacted `AmbientResourceRefused` with only catalog-derived recovery, while the same resolver on an explicit carrier retains the existing caller-visible reference/detail diagnostic.
- A binder infrastructure source containing adversarial text becomes redacted `HandlerFailed`; its wrapper has static `Display`/`Debug`, exposes no `Error::source`, and never reaches framework-owned logs. The binder API cannot emit a forged permission, workspace, or request-context framework error.
- `PrivateResourceReference::from_id` accepts every RFC 0012 URI-unreserved id and rejects empty, URI-shaped, whitespace, control-bearing, slash-bearing, and non-ASCII values without retaining or displaying them. `?` converts either constructor reason into the redacted binder infrastructure path. Compile-pass fixtures exercise the exact `From<E>` bounds for binder and resolver application failures; compile-fail coverage rejects a mismatched footprint and proves application code cannot read or construct the private field directly.
- An external crate implements `BindAmbientResource` and `ResolveResourceWithErrors` with `async fn` methods against the public `fn -> impl Future + Send` declarations and passes `cargo clippy --all-targets -- -D warnings`; the same fixture proves the returned futures may borrow both `&self` and the supplied context. The public traits themselves require no `async_fn_in_trait` allowance, and implementations need no `manual_async_fn` allowance. `ServerBuilder::resolver_with_errors` and `CommandRegistry::with_resolver_with_errors` compile the same typed resolver footprint and runtime behavior, while either path rejects coexistence with RFC 0012's plain resolver.
- Registration checks `missing_as` against required exposed consumers only and rejects it when any such command omits the code or declares a runtime message/non-empty-required details shape that absence cannot construct. Optional-only consumers need not declare the unreachable code, and a binding with no required consumer rejects dead `missing_as` configuration. Typed binders and resolvers are rejected when commands omit any error they may emit.
- With neither source, instrumented surface preparation proves it completed one `Absent` plan and wrong-lane execution still redirects first. On the correct lane, permission preview, dry run, and ordinary execution return the declared `session_required` outcome with that completed plan before policy, authorizer, replay consumption, or application hooks. RFC 0020 deferred delivery carries the identical application-error tool result and never creates an authority-bearing capsule. `Option<Res<Session>>` instead continues and reaches its handler as `None` on approved execution. Public bare-registry `build_plan*` remains argument-bound and never claims an ambient preparation mode.
- The same required-absence fixture without `missing_as` returns `ResourceBindingMissing`/`InvalidInput` with only resource, `binding: "absent"`, and catalog-derived establishment operations. Ordinary, preview, dry-run, RFC 0020 deferred, generated-host, event, and framework-log projections preserve that owner and contain no identity, digest, reference, or resolver detail.
- `Option<Res<Session>>` receives an error rather than `None` when a selected explicit or ambient source refuses resolution.
- Existing custom `ResourceParam` and `ResourceParams` implementations remain required through the default methods; optional extraction changes no public `ResourceUse` fields or existing source behavior. `CommandSpec` and `OperationSpec` project the same canonical `optionalResources` set, and omitted/empty fields preserve pre-adoption catalog and snapshot bytes.
- Registration rejects mixed required/optional or optional/release use of one resource in a handler signature; duplicate uses within one mode deduplicate.
- Preview, dry run, wrong-lane routing, and denied authorization do not call the binder or create a session. Approved dispatch calls it once.
- Instrumented preparation proves ordinary and native execution select bindings once, pass the same private carrier through authorization/confirmation, and never rebuild source state before dispatch; replay alone prepares fresh state and must match the stored fingerprint.
- A valid single-use replay is consumed before binder realization. Binder, resolver, handler, result-contract, and transport failures do not restore it; invalid, expired, mismatched, and concurrently consumed tokens call no binder or handler. A reusable token revalidates a freshly prepared context and fingerprint on every dispatch.
- Exact fingerprint vectors cover present/absent, explicit/ambient, different explicit references, different resource names, reordered declaration input, and different ambient identities. Canonical sorting makes declaration order irrelevant while every semantic difference produces a distinct fingerprint without serializing private binding facts.
- Native plan, preview, authorizer, and event fixtures expose the same sorted selected-source facts; their `resourceBindingFacts` fields are omitted when empty, and adversarial identity/reference values never appear in those projections. A private comparison-helper fixture deletes the preview or event copy and proves `check_resource_binding_projection` reports the mismatch without adding a public fault-injection hook.
- An ambient binder receives selected optional workspace context when available and no raw request metadata.
- Reordering workspace declarations, path arguments, or valid pre-resolved roots gives the binder the same ascending-id workspace slice and the same fingerprint; duplicate or unknown pre-resolved ids fail under RFC 0009 before source selection or binder invocation.
- A VBL session binder may return its declared `workspace_context_conflict` application outcome when a valid selected root disagrees with the session's application-owned workspace binding. Malformed enabled metadata and dual raw/pre-resolved workspace authority fail as RFC 0009 `InvalidRequestContext` before the binder is called and never acquire the application code.
- Ordinary and RFC 0020 deferred execution use the same source selection and identity digest; cancellation before realization leaves no resource, and a late binder/resolver/handler result cannot serialize private binding state or overwrite RFC 0020's winning terminal record.
- RFC 0020 task status/outcome serialization contains no private prepared carrier or execution capsule. Cross-RFC fixtures prove store/runner loss invokes no binder/handler and never reconstructs identity or binding from plans, events, replay records, or public task data; effects that escaped before loss remain application-owned.
- Catalog, help, schemas, examples, previews, events, and debug serialization contain no ambient reference, identity, or standalone digest.
- A VBL fixture binds `session` ambiently with optional explicit override while keeping `tab` argument-bound. Ordinary calls require neither `start_session` nor `agent_session_id`, while the explicit `start_session` tool remains available as declared recovery.
- The completed VBL native surface reproduces all 27 released input schemas and every grouped required-set intersection after ambient session binding; the RFC 0015 routing, grouping, output, annotation, and instruction snapshot remains otherwise byte-identical.

## Drawbacks

Binding becomes a serving-surface concern as well as a resource concern. Understanding one deployed server may require reading the core catalog and its active surface profile together.

Two-phase binding adds private planning state and another dispatch boundary. The split is necessary for side-effect-free preview, but it increases the number of invariants task, replay, and cancellation paths must preserve.

Optional explicit override means one resource has two potential authorities. Explicit-first behavior is easy to state and test; every additional precedence mode would multiply security and fingerprint cases.

## Rationale And Alternatives

**Resolve ambient resources during planning.** This makes the plan concrete and may create broker state before permission checks, during dry run, or on a wrong-lane call. Source selection is planning; realization is dispatch.

**Let the application router inject a hidden carrier argument.** Hidden mutation can reuse the argument resolver but puts a private handle into bound arguments and plans, where serialization and diagnostics may expose it. Private binding state keeps the handle outside the model-visible request model.

**Prefer ambient identity even when an explicit carrier is present.** This makes explicit fallback unreliable and can redirect a caller away from the resource it deliberately named. The first version fixes authority at explicit-first.

**Remove the carrier entirely on ambient surfaces.** Ambient-only is supported, but VBL and generic MCP clients need an explicit fallback when host identity is absent. Optional override lets one deployed surface degrade without inventing a second command catalog.

**Add ambient authoring to the effect-lane compatibility profile.** That would require a second authored surface declaration, canonical migration, and endpoint-level configuration contract for a profile that currently derives its tool schemas privately. VBL and generated hosts already require the native surface for name and schema compatibility, so version 1 keeps the effect-lane profile argument-bound. A later RFC may add a general authored effect-lane profile when a concrete adopter needs ambient carriers without native tools.

**Give binders raw metadata.** This recreates adapter-specific parsing and bypasses RFC 0009/0013 validation. Binders receive typed context only.

**Treat ambient identity as an authenticated principal.** RFC 0013 deliberately promises correlation, not authentication. Applications may isolate resources by the tuple within a trusted embedding, but resource authorization still belongs to their binder and resolver policy.

## Prior Art

Dependency-injection containers select a provider during graph construction and instantiate it only when execution reaches the dependent component. Authentication middleware derives a principal from host credentials while preserving an explicit administrative override under a fixed precedence rule. Twill combines those shapes with its side-effect-free invocation planner.

RFC 0012 supplies typed resource extraction and resolver-owned liveness. RFC 0013 supplies normalized, private conversation identity. VBL's released broker supplies the concrete evidence that identity-to-session binding, explicit fallback, TTL expiry, and non-disclosure work as application policy.

## Unresolved Questions

No architectural questions remain for the initial binding boundary. The public type and method names in this body become the accepted Stage-1 implementation contract on promotion; implementation may not substitute an unreviewed spelling. Any later review-driven rename or ergonomic change must return the RFC to design review and amend the managed body before implementation proceeds. Such a revision must retain serialized redacted source facts, private logical-slot digests/references, ordinary-resolver reuse, application-producer non-disclosure tests, and a declaration-only missing-source path.

## Future Possibilities

Additional typed context sources could bind authenticated principals, tenant ids, or deployment environments under separate RFCs. A binder could return a framework-owned opaque capability instead of a string reference when an application can resolve without an intermediate id.

Surface negotiation might eventually choose argument or ambient binding dynamically per client capability. The first version keeps binding fixed at server construction so schemas, hashes, help, and authority remain stable for the life of the endpoint.
