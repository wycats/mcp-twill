<!-- exo:14 ulid:01kxby50v4z2zzrc8gsgchahhk -->

# RFC 0014: Application Result Contracts

- Status: Accepted
- Area: command results, output schemas, application errors, recovery, MCP projection
- Target milestone: v0.4
- Depends on: RFC 0001 (authoritative command catalog), RFC 0002 (diagnostics and response profiles), RFC 0010 (declared preconditions), RFC 0012 (first-class resources)

## Summary

This RFC makes command results part of Twill's authoritative catalog. A command declares the schema of its successful structured value and the application errors it may return. The framework validates those declarations, derives catalog and help projections from them, and preserves the distinction between framework failures and application refusals through every serving surface.

Legacy handlers continue to return `CommandOutput` with its existing text, structured data, cursors, grants, and listings. A result-aware handler instead returns one portable schema-validated application value plus typed RFC 0012 grant/listing wrappers. Application failures gain stable application-owned codes, structured details, and validated recovery edges. A handler returns an application failure deliberately; an unexpected Rust error remains `HandlerFailed`.

RFC 0002's response envelope remains the contract of CLI-shaped execution tools. A later native-tool projection may expose a protocol-compatible object-only success value directly as MCP `structuredContent` and an application failure as an `isError` tool result whose compact JSON body stays in text content. Both projections derive from one `ApplicationResultContract`, so changing transport shape does not change command semantics.

## Motivation

Twill's catalog currently says only that an operation produces text or structured output. `OutputContract` has a format and summary, while the actual structured value is an unconstrained `serde_json::Value`. MCP tools can advertise an `outputSchema`, but Twill's generated execution tools cannot project a command-specific one because many commands share the same `RunRequest` and response envelope.

The error boundary is similarly coarse. Planning, permissions, workspaces, resources, and request context have typed framework errors. A handler's domain refusal becomes `FrameworkError::Handler(String)` and projects as `HandlerFailed`. That is correct for an unexpected implementation failure and wrong for an expected application result such as `session_required`, `element_stale`, `tab_not_owned`, or `artifact_not_found`. Agents need those codes and their recovery data as public contract.

Visible Browser Lab makes the gap concrete. Its public tools publish exact success schemas and stable error objects. A generated-catalog audit across 63 baseline and 27 hybrid outputs finds only primitives, objects, arrays, constants, enums, `minLength`, `oneOf`, and local `$defs`/`$ref`; six hybrid schemas use local definitions. `session_required` tells the caller to establish an explicit fallback session; `element_stale` tells it to obtain a fresh snapshot; ownership refusals point back to tab enumeration. Replacing those values with a generic handler failure would make a VBL-on-Twill port behaviorally incompatible even if every browser action still worked.

The framework also needs output truthfulness before it can generate native tools. Input schemas are authoritative because planning validates them. A projected output schema deserves the same standing. Rust derives the declaration for typed handlers, but custom `Serialize` and `JsonSchema` implementations can still disagree, so Twill validates the one serialized value it is already preparing for output for both typed and dynamic handlers before publishing it.

## Guide-Level Explanation

The smallest result-aware command defines one typed success value and uses the one-parameter alias:

```rust
#[derive(Serialize, JsonSchema)]
struct BrowserStatusResult {
    ready: bool,
}

async fn handle_browser_status(
    ctx: CommandContext,
) -> ApplicationResult<BrowserStatusResult> {
    let ready = broker_ready(&ctx).await?;
    Ok(BrowserStatusResult { ready }.into())
}

server.command("browser status", |command| {
    command.handle_result(handle_browser_status);
});
```

The default `NoApplicationError` means this command declares no expected application failures. Framework failures remain available through `?`; the default changes only the application-owned outcome set.

A command with expected domain failures extends the same path with an application error type and, when needed, a command-specific error-set marker. Its success value remains an ordinary typed value:

```rust
#[derive(Serialize, JsonSchema)]
struct NewTabResult {
    tab_id: String,
    url: String,
    title: String,
}
```

Expected application failures implement a framework trait and provide a static catalog:

```rust
#[derive(Debug, Serialize, JsonSchema)]
struct NoDetails {}

#[derive(Debug, Serialize, JsonSchema)]
struct TabNotOwnedDetails {
    tab_id: String,
}

#[derive(Debug, thiserror::Error)]
enum BrowserFailure {
    #[error("no ambient or explicit browser session is available")]
    SessionRequired,
    #[error("the selected session does not own this tab")]
    TabNotOwned(TabNotOwnedDetails),
}

impl ApplicationError for BrowserFailure {
    fn declarations() -> Vec<ApplicationErrorDecl> {
        vec![
            ApplicationErrorDecl::new(
                "session_required",
                "No ambient or explicit browser session is available",
            )
            .details_schema(schema_for!(NoDetails)),
            ApplicationErrorDecl::new(
                "tab_not_owned",
                "The selected session does not own this tab",
            )
            .details_schema(schema_for!(TabNotOwnedDetails)),
        ]
    }

    fn code(&self) -> &'static str {
        match self {
            Self::SessionRequired => "session_required",
            Self::TabNotOwned(_) => "tab_not_owned",
        }
    }

    fn details(&self) -> serde_json::Value {
        match self {
            Self::SessionRequired => serde_json::json!({}),
            Self::TabNotOwned(details) => serde_json::json!({
                "tab_id": &details.tab_id,
            }),
        }
    }
}

struct TabsNewErrors;

impl ApplicationErrorSet<BrowserFailure> for TabsNewErrors {
    fn uses() -> Vec<ApplicationErrorUse> {
        vec![
            ApplicationErrorUse::new("session_required")
                .recover_with("session start")
                .at_most_one_recovery(),
        ]
    }
}
```

`ApplicationError::details` returns an already materialized JSON value and is deliberately infallible. The guide therefore constructs that value directly instead of calling a fallible serializer and unwrapping it. An application with reusable serializable detail structs may provide its own separately tested infallible conversion, but Twill does not catch a panic or turn a serialization failure into the declared application error. Runtime panics retain the suite's ordinary embedding or isolation-boundary policy.

The typed handler registers both sides of the result contract:

```rust
async fn handle_new_tab(
    ctx: CommandContext,
    args: NewTabArgs,
) -> ApplicationResult<NewTabResult, BrowserFailure, TabsNewErrors> {
    // ...
}

server.command("tabs new", |command| {
    command.handle_result(handle_new_tab);
});
```

Resource parameters compose in the same signature rather than selecting another handler API:

```rust
type WaitApplicationResult =
    ApplicationResult<WaitResult, BrowserFailure, WaitErrors>;

async fn handle_wait(
    session: Res<Session>,
    ctx: CommandContext,
    args: WaitArgs,
) -> WaitApplicationResult {
    // resources, checked arguments, and result contracts share one adapter
}

command.handle_result(handle_wait);
```

An application-owned proof refusal can reuse RFC 0010's establishment graph without copying operation ids into its result declaration. The command still validates the proof itself; the binding says only that this declared refusal concerns a required explicit capability:

```rust
application_error_set! {
    pub struct DeployPublishErrors for DeployFailure {
        ApplicationErrorUse::new("validation_stale")
            .for_capability("validated-build"),
    }
}
```

If `build validate` is the capability's bootstrap provider, Twill projects it as the callable recovery. A provider that itself requires `validated-build` remains refresh behavior and is not offered to a caller whose proof is missing or stale.

Resource-bearing success values use the output-oriented alias without changing the registration method:

```rust
type NewTabApplicationResult = ApplicationOutputResult<
    Granted<Tab, ApplicationSuccess<NewTabResult>>,
    BrowserFailure,
    TabsNewErrors,
>;

async fn new_tab(/* ... */) -> NewTabApplicationResult {
    let (tab_id, result) = create_tab().await?;
    Ok(ApplicationSuccess::value(result).grant(Grant::<Tab>::new(tab_id)))
}
```

`TabsNewErrors` and `WaitErrors` are zero-sized `ApplicationErrorSet<BrowserFailure>` markers naming only the codes and recovery alternatives their commands may emit. `TabsNewErrors` deliberately selects `session_required` without advertising the unrelated `tab_not_owned` variant from the shared error enum. Applications with a small error enum and no recovery edges can omit the marker and use the default all-errors set; recovery-bearing commands declare a set so recovery remains command-owned.

Twill derives the success schema from `NewTabResult`, joins `BrowserFailure`'s server-wide identities with `TabsNewErrors`' command uses, and derives the command's resource edges from RFC 0012 output wrappers. The handler can return three conceptually distinct outcomes:

- success, carrying the declared application value and any typed resource wrappers;
- a declared application failure, carrying an application code, details, and selected recoveries;
- an unexpected framework/implementation failure, which remains `HandlerFailed`.

Dynamic applications may bind an explicit contract instead of a Rust result type:

```rust
let new_tab_contract = ApplicationResultContract::for_type::<NewTabResult>()
    .with_errors::<BrowserFailure, TabsNewErrors>()?;

command
    .result_contract(new_tab_contract)
    .handle_dynamic(call_broker);
```

The dynamic closure returns public constructor-controlled values rather than the private registry outcome:

```rust
async fn call_broker(
    _ctx: CommandContext,
    args: BTreeMap<String, serde_json::Value>,
) -> DynamicApplicationResult {
    match broker_call(args).await {
        Ok(value) => Ok(ApplicationSuccess::value(value)),
        Err(error) => Err(DynamicApplicationError::new(error.code)
            .details(error.public_details)
            .recovery(error.declared_recovery)
            .into()),
    }
}
```

Every structured success is checked against the declared schema before projection. Typed handlers derive that schema and dynamic handlers bind it explicitly; both validate the same materialized JSON value that later reaches output shaping. The example's `session_required` declaration uses `DeclarationSummary`, so the dynamic value deliberately omits `.message(...)`; a `RuntimeBounded` declaration would require it. “Dynamic” applies to the application value and error payload, not to RFC 0012 resource edges: a dynamic handler that grants or lists resources still returns the sealed `Granted<R, O>`/`Listed<R, O>` wrappers around its `ApplicationSuccess<Value>`. There is no explicit `.grants(...)` escape hatch. A dynamic application error must use a code declared on that command. Contract mismatch is a framework failure and never escapes as a value that falsely claims to satisfy the public schema.

Help lists expected application errors with their callable operations and non-callable recovery actions. Agents should treat these as ordinary domain outcomes, not as evidence that the MCP server malfunctioned. Framework errors continue to teach planning repair, permission escalation, and transport integrity through RFC 0002 diagnostics.

### How Agents Should Learn This

Generated help teaches the success shape in compact prose and lists stable application errors under `Errors:`. Each error names the condition and distinguishes callable recovery operations from manual or host actions. An agent encountering `element_stale` should call the declared recovery operation and preserve still-valid identifiers; an agent encountering `start_chrome` should follow the host action rather than inventing a tool call; an agent encountering `HandlerFailed` should treat it as an implementation or service failure instead.

The distinction is visible but transport-appropriate. CLI-shaped tools return one response envelope whose error family says whether the failure is framework-owned or application-owned. Native tools set MCP `isError: true` and carry the compact application-error object in their first text content part, while omitting `structuredContent` so a success-only `outputSchema` remains truthful. Text-only surfaces render the same code and typed recovery paths in prose.

## Reference-Level Explanation

### Result Contract

`OutputContract` grows a success schema and a declared error set:

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationResultContract {
    /// JSON Schema for the application's successful structured value.
    pub success_schema: serde_json::Value,
    /// Fully joined application-error contracts this command may return.
    pub errors: Vec<ApplicationErrorSpec>,
}

pub struct OutputContract {
    pub format: OutputFormat,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<ApplicationResultContract>,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorDecl {
    pub code: String,
    pub summary: String,
    pub message: ApplicationMessageDecl,
    pub details_schema: serde_json::Value,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorUse {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub recoveries: Vec<ApplicationRecoveryDecl>,
    pub recovery_cardinality: RecoveryCardinality,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorSpec {
    pub code: String,
    pub summary: String,
    pub message: ApplicationMessageDecl,
    pub details_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub recoveries: Vec<ApplicationRecoveryDecl>,
    pub recovery_cardinality: RecoveryCardinality,
}

impl ApplicationResultContract {
    pub fn new(success_schema: impl Into<serde_json::Value>) -> Self;

    pub fn for_type<T>() -> Self
    where
        T: Serialize + JsonSchema + Send + 'static;

    pub fn with_errors<E, S>(self) -> Result<Self>
    where
        E: ApplicationError,
        S: ApplicationErrorSet<E>;

    pub fn with_error_spec(self, error: ApplicationErrorSpec) -> Self;
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ApplicationMessageDecl {
    DeclarationSummary,
    RuntimeBounded { max_scalar_values: u16 },
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationActionDecl {
    pub code: String,
    pub summary: String,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ApplicationRecoveryDecl {
    Operation { operation_id: String },
    Action(ApplicationActionDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationRecoveryKey {
    Operation(String),
    Action(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationRecoverySelection {
    Declared,
    None,
    Only(Vec<ApplicationRecoveryKey>),
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryCardinality {
    Any,
    AtMostOne,
}

impl ApplicationErrorDecl {
    pub fn new(
        code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self;

    pub fn details_schema(self, schema: impl Into<serde_json::Value>) -> Self;
    pub fn runtime_message(self, max_scalar_values: u16) -> Self;
}

impl ApplicationErrorUse {
    pub fn new(code: impl Into<String>) -> Self;
    pub fn for_capability(self, capability: impl Into<String>) -> Self;
    pub fn recover_with(self, operation_id: impl Into<String>) -> Self;
    pub fn recover_by(
        self,
        action_code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self;
    pub fn at_most_one_recovery(self) -> Self;
}
```

These declaration types use the corpus additive unknown-field policy and emit only known camel-case fields. `ApplicationMessageDecl` serializes as `"declarationSummary"` or `{ "runtimeBounded": { "maxScalarValues": 256 } }`; `RecoveryCardinality` serializes as `"any"` or `"atMostOne"`. `ApplicationRecoveryDecl` uses the exact externally tagged forms `{ "operation": { "operationId": "session start" } }` and `{ "action": { "code": "start_chrome", "summary": "Start Chrome" } }`. `capability` is omitted when absent, preserving the pre-adoption bytes of every application error. `ApplicationRecoveryKey` and `ApplicationRecoverySelection` are runtime authoring values rather than catalog declarations and intentionally implement no serde or JSON Schema traits.

Schemas use JSON Schema 2020-12. Typed `handle_result` registration and `ApplicationResultContract::for_type::<T>()` generate with explicit `schemars::generate::SchemaSettings::draft2020_12()` settings rather than Schemars' movable default. `ApplicationResultContract::new` and `details_schema` accept anything convertible into `serde_json::Value`, including Schemars 1.x `Schema`, for explicit or imported contracts. Passing `schema_for!(T)` is therefore a valid conversion, but the resulting schema remains an explicit contract and must already fit the supported explicit dialect; it does not receive typed-only storage or nullable normalization. `for_type` is the stable typed authoring path. The compiler accepts an optional top-level `$schema` only when it is exactly `https://json-schema.org/draft/2020-12/schema`, verifies it, and removes it before canonical projection and hashing because the dialect is already fixed; nested, alternate, or non-string `$schema` values fail.

The initial self-contained result profile is the measured union of VBL's 63 baseline and 27 hybrid outputs plus RFC 0017's input core: `title` and `description`; `type` with one primitive or a nullable two-member array containing exactly one non-null primitive and `null`; `const` and homogeneous `enum`; string `minLength`; array `items` and `minItems`; object `properties`, `required`, and boolean-or-schema `additionalProperties`; `oneOf`; and acyclic local `$defs`/`$ref`. Every local definition must be reachable from the root validation graph; dead definitions fail rather than disappearing during projection. Nullable type arrays are set-like and normalize to `[non-null-type, "null"]` so equivalent authored order has one snapshot spelling. Numeric literals in `const` or `enum` must be represented exactly by RFC 8785's I-JSON number domain so compiled surface identity can never round them. Root and branch boolean schemas, remote or dynamic references, recursive graphs, custom vocabularies, unsupported composition, numeric bounds/divisibility, patterns, maximum/uniqueness assertions, property-count assertions, out-of-domain numeric literals, and unknown assertion keywords fail registration. Error-detail object schemas default to `additionalProperties: false`; an application that needs extension keys must declare their schema explicitly.

The broader JSON Schema vocabulary remains future work because the released compatibility corpus supplies no runtime value, diagnostic, or generated-host evidence for it. The shared compiler can add one assertion at a time once an adopter contributes both acceptance and rejection values and every projection can preserve its semantics.

The schema compiler, canonicalizer, local-reference rules, and redaction machinery are shared with argument contracts, but their accepted profiles differ deliberately. Model-facing inputs impose stricter composition and ambiguity rules for client compatibility and planning diagnostics. Results are validated after production and may use output unions that need not satisfy input-side disjointness proofs. The catalog hash covers the canonical schema, error declarations, summaries, and recovery edges.

Automatically derived typed-result schemas also apply two output-safe Schemars normalizations before entering that profile. First, Schemars' `Option<T>` form `anyOf: [T, {"type":"null"}]` becomes the supported `oneOf` form only when the compiler proves `T` excludes null; the direct two-member `type` form receives the canonical order above. Explicit authored `anyOf` remains unsupported. Second, Rust storage annotations in the fixed set `int8`, `int16`, `int32`, `int64`, `int128`, `uint8`, `uint16`, `uint32`, `uint64`, `uint128`, `float`, and `double`, plus the corresponding primitive-width `minimum`/`maximum` assertions Schemars emits, are removed from the derived schema. The projected contract intentionally describes the broader JSON `integer` or `number` domain; every value the Rust type can serialize remains valid, and the materialized value is still checked against that public schema. These transforms apply only to the framework-derived schema for a typed result. An explicit dynamic contract containing unsupported `anyOf`, `format`, or numeric bounds still fails, and a custom assertion outside the exact recognized patterns is never stripped.

RFC 0017 does not apply that broadening to typed *inputs*: admitting a value outside a Rust argument type's deserialization domain would turn a caller-valid request into an author defect. Its `JsonInteger` or dynamic validated-value path represents unconstrained integer inputs instead. The shared compiler therefore has one canonical core while each direction retains the variance rule its authority boundary permits.

Application codes use lower snake case and are scoped to the server contract. `ApplicationErrorDecl` is the server-wide identity: code, summary, message policy, and details schema. A code may appear on multiple commands only through that one identity. `ApplicationErrorUse` is command-scoped: it selects a declared code and normally owns recovery alternatives and cardinality because the useful next operation can depend on where the error arose. A capability-bound use instead names one RFC 0010 explicit capability and delegates its callable recovery inventory to that capability graph. Registration joins each use with its identity and, when present, the selected command's capability graph into the flat composed `ApplicationErrorSpec` projected by the result contract. The authoring split therefore adds no nested wire shape. Conflicting identities, unknown uses, duplicate uses, and invalid capability bindings fail registration before a surface can observe them.

That command-scoped use is the only authority for `CommandExecutionOutcome::ApplicationError`. One narrow serving-surface extension can exercise that authority without receiving an application error value: an owning RFC may compile a declaration-only emitter for a structural availability condition known after operation selection. The emitter names only a code already present in the selected command's joined `ApplicationErrorSpec`; its compiled code is its complete static producer footprint, and it cannot add an identity, command use, message, details value, recovery, or runtime selection. Surface compilation requires `DeclarationSummary`, verifies that the empty details object validates, and verifies that `Declared` recovery satisfies the command's cardinality. The owning RFC fixes the gate's lifecycle position, includes the mapping in surface identity, and supplies cross-mode acceptance. The emitter can then construct the ordinary `CommandExecutionOutcome::ApplicationError` body entirely from that command spec. It executes no application code and consumes no application-owned runtime value. RFC 0016's `missing_as` required-binding absence is the initial and only version-1 consumer; another structural condition requires its own RFC rather than a general adapter-authored error hook.

A later serving adapter may instead define an additional profile-scoped application use for a pre-planning gate that the adapter RFC itself owns. Such a use must be serialized and hash-covered in that profile, reference one existing server-wide `ApplicationErrorDecl`, obey its message policy and details schema, and project only on that adapter's own result surface. It does not add an `ApplicationErrorUse` to the selected command, cannot construct `CommandExecutionOutcome::ApplicationError`, and cannot appear in native MCP or another profile. RFC 0019's structurally proven absent-context rejection is the initial consumer of this separate extension point. This preserves one application identity while keeping command results, command-scoped structural availability, and packaging-specific availability contracts distinct.

Application summaries and recovery-action summaries are non-empty, contain no C0, DEL, C1, or fixed presentation-unsafe scalar, and are limited to 512 Unicode scalar values. The presentation-unsafe set is U+061C, U+200E–U+200F, U+2028–U+202E, U+2060–U+206F, and U+FEFF. These summaries are public catalog text and are rejected rather than rewritten. `DeclarationSummary` is the default message policy. `RuntimeBounded` is an explicit compatibility contract for applications such as VBL whose established error message includes model-visible identifiers or runtime conditions; its declared limit must be between 1 and the framework cap of 512 final rendered scalar values, including any truncation marker. Recovery operation ids inherit catalog naming limits, while action codes use the application-code grammar.

`ApplicationErrorDecl::new` defaults to `DeclarationSummary` and the canonical closed empty-object details schema `{ "type": "object", "properties": {}, "additionalProperties": false }`. These standalone declaration values expose their complete semantic state, so their consuming methods are ordinary value transformations rather than finalizing-builder assignments. `.details_schema(...)` replaces the visible details schema. `.runtime_message(max_scalar_values)` replaces the visible message policy, including a previously selected bound. `ApplicationErrorUse::new` starts with no capability, no recoveries, and `RecoveryCardinality::Any`; `.for_capability(...)` replaces the optional capability field, `.recover_with` and `.recover_by` append one ordered recovery, and `.at_most_one_recovery()` idempotently selects `AtMostOne`. Registration rejects a capability-bound value with authored recoveries or non-`Any` cardinality instead of making call order significant. Duplicate recovery keys remain invalid at registration. `ApplicationResultContract::with_error_spec` appends one composed spec, and `with_errors::<E, S>()` appends the typed join; duplicate application codes remain invalid at the join or registration boundary. The final declaration value is therefore equivalent to directly constructing its public fields, with no hidden call-order authority. Dynamic outcome construction accepts `message: Option<String>`; static declarations require `None`, and bounded declarations require `Some`.

`ApplicationResultContract::with_errors::<E, S>()` performs the complete context-free typed join and returns a build error for an unknown, duplicate, conflicting, or capability-bound use. A standalone result contract has no selected command or server catalog from which to derive capability recovery, so it never publishes a provisional half-joined spec. Typed `handle_result` instead retains the materialized `E` declarations and `S` uses in private builder state until registry finalization supplies that context. The named capability must then be a hand-declared RFC 0010 capability required by that command. Resource-derived compatibility capabilities are rejected because RFCs 0012 and 0016 own their resolution and refusal paths. The compiler copies the capability name and RFC 0010's canonical sorted bootstrap-provider operations into the composed spec; refresh providers are never callable recovery from missing or invalid proof. A fully dynamic adapter may add composed `ApplicationErrorSpec` values explicitly; registration accepts a capability-bound low-level spec only when its recoveries exactly equal that bootstrap set, contain no actions, and use `RecoveryCardinality::Any`. Registration still rejects duplicate server-wide identities and validates every command-owned recovery edge. The operation catalog always exposes the completed spec, never the split authoring machinery or private pending declaration state.

Error identities, command uses, composed specs, and producer-footprint codes are set-like by application code. Registration rejects duplicates, then sorts them by lowercase code before catalog serialization and hashing; changing only declaration callback order is identity-neutral. Recovery alternatives remain in authored declaration order because help and protocol output expose that sequence. `ApplicationRecoverySelection::Declared` emits that order. `Only(keys)` rejects duplicate keys, treats its input as a selection set, and emits selected recoveries in declaration order rather than caller-provided runtime order. Reordering declared recoveries is therefore an intentional catalog/hash and public-guidance change; reordering a runtime `Only` vector is not.

### Handler Outcomes

The framework distinguishes expected application outcomes from framework errors:

```rust
pub type ApplicationResult<
    T,
    E = NoApplicationError,
    S = AllApplicationErrors<E>,
> = ApplicationOutputResult<ApplicationSuccess<T>, E, S>;

pub type ApplicationOutputResult<
    O,
    E = NoApplicationError,
    S = AllApplicationErrors<E>,
> = std::result::Result<O, CommandFailure<E, S>>;

pub enum NoApplicationError {}

pub struct DeclaredApplicationError<E, S> {
    error: E,
    set: PhantomData<fn() -> S>,
}

pub enum CommandFailure<E, S = AllApplicationErrors<E>> {
    Application(DeclaredApplicationError<E, S>),
    Framework(FrameworkError),
}

pub type DynamicApplicationResult<O = ApplicationSuccess<serde_json::Value>> =
    std::result::Result<O, DynamicCommandFailure>;

pub enum DynamicCommandFailure {
    Application(DynamicApplicationError),
    Framework(FrameworkError),
}

pub struct DynamicApplicationError { /* private owned fields */ }

impl DynamicApplicationError {
    pub fn new(code: impl Into<String>) -> Self;
    pub fn message(self, message: impl Into<String>) -> Self;
    pub fn details(self, details: serde_json::Value) -> Self;
    pub fn recovery(self, selection: ApplicationRecoverySelection) -> Self;
}

pub struct ApplicationSuccess<T> {
    value: T,
    resources: ApplicationResourceComponents,
}

struct ApplicationResourceComponents {
    grants: Vec<ResourceRef>,
    listings: Vec<ResourceRef>,
}

impl<T> ApplicationSuccess<T> {
    pub fn value(value: T) -> Self;
}

impl<T> From<T> for ApplicationSuccess<T> {
    fn from(value: T) -> Self {
        Self::value(value)
    }
}

impl<E, S> From<E> for CommandFailure<E, S>
where
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn from(error: E) -> Self {
        Self::Application(DeclaredApplicationError {
            error,
            set: PhantomData,
        })
    }
}

impl<E, S> From<FrameworkError> for CommandFailure<E, S>
where
    E: ApplicationError,
    S: ApplicationErrorSet<E>,
{
    fn from(error: FrameworkError) -> Self {
        Self::Framework(error)
    }
}

impl From<DynamicApplicationError> for DynamicCommandFailure {
    fn from(error: DynamicApplicationError) -> Self {
        Self::Application(error)
    }
}

impl From<FrameworkError> for DynamicCommandFailure {
    fn from(error: FrameworkError) -> Self {
        Self::Framework(error)
    }
}

pub trait ApplicationOutput: private::Sealed + Send + 'static {
    type Value: Serialize + JsonSchema + Send + 'static;

    fn granted() -> Vec<&'static str>;
    fn enumerated() -> Vec<&'static str>;
    fn into_success(self) -> ApplicationSuccess<Self::Value>;

    fn grant<R: Resource>(self, grant: Grant<R>) -> Granted<R, Self>
    where
        Self: Sized,
    {
        Granted {
            output: self,
            grant,
        }
    }

    fn listing<R: Resource>(self, listing: Listing<R>) -> Listed<R, Self>
    where
        Self: Sized,
    {
        Listed {
            output: self,
            listing,
        }
    }
}

// RFC 0012's existing wrappers gain a source-compatible defaulted payload.
pub struct Granted<R: Resource, O = CommandOutput> {
    output: O,
    grant: Grant<R>,
}

pub struct Listed<R: Resource, O = CommandOutput> {
    output: O,
    listing: Listing<R>,
}

pub trait ApplicationError: std::error::Error + Send + Sync + 'static {
    fn declarations() -> Vec<ApplicationErrorDecl>;
    fn code(&self) -> &'static str;
    fn details(&self) -> serde_json::Value;
    fn runtime_message(&self) -> Option<Cow<'_, str>> {
        None
    }
    fn recovery(&self) -> ApplicationRecoverySelection {
        ApplicationRecoverySelection::Declared
    }
}

pub trait ApplicationErrorSet<E: ApplicationError>: Send + Sync + 'static {
    fn uses() -> Vec<ApplicationErrorUse>;
}

pub struct AllApplicationErrors<E>(PhantomData<fn() -> E>);

pub trait ApplicationErrorFootprint<E: ApplicationError>: Send + Sync + 'static {
    fn codes() -> Vec<&'static str>;
}

pub struct AllApplicationErrorCodes<E>(PhantomData<fn() -> E>);

pub struct ProducedApplicationError<E, F> {
    error: E,
    footprint: PhantomData<fn() -> F>,
}

impl<E, F> From<E> for ProducedApplicationError<E, F>
where
    E: ApplicationError,
    F: ApplicationErrorFootprint<E>,
{
    fn from(error: E) -> Self {
        Self {
            error,
            footprint: PhantomData,
        }
    }
}
```

`NoApplicationError` is the framework-provided uninhabited error type with framework implementations of `Display`, `std::error::Error`, and `ApplicationError`; its declaration set is empty and its value-producing trait methods are exhaustive matches over the uninhabited value. It is the default for handlers that have an authoritative success schema but no expected application failures; framework errors remain available through `CommandFailure::Framework`. `AllApplicationErrors<NoApplicationError>` therefore implements the default empty command error set rather than requiring a special result alias.

`DynamicApplicationError::new` starts with no runtime message, the empty object for details, and `ApplicationRecoverySelection::Declared`. Its consuming methods replace those owned values. The exact `From<DynamicApplicationError>` and `From<FrameworkError>` implementations above construct the corresponding `DynamicCommandFailure`, so broker adapters can use `?` without gaining access to the registry's private erased outcome. The typed `CommandFailure` conversions are bounded by the same `ApplicationError` and `ApplicationErrorSet` pair carried by the result alias: `From<E>` attaches that selected marker, while `From<FrameworkError>` preserves the framework channel. The explicit `ApplicationResultContract` remains authoritative: code, message presence, details, and recovery selection are all checked after the dynamic handler returns. These constructors make both paths authorable; they do not let an adapter invent a declaration.

One compatibility exception is intentionally directional. RFC 0010's public `FrameworkError::CapabilityDenied` remains valid for legacy `CommandHandler` implementations. A result-aware handler already has the application channel above, so returning that framework variant is an unexpected handler contract defect. At the result-dialect boundary Twill replaces the complete variant with `FrameworkError::Handler("result-aware handler returned legacy capability denial".into())`; capability, detail, carrier, and providers are dropped before the outer error reaches a direct registry caller. Every serving surface consequently projects the ordinary static, empty-details `HandlerFailed` family with no capability detail or legacy steering. Twill never guesses an application code or converts the framework value into `CommandExecutionOutcome::ApplicationError`.

`ApplicationError::declarations`, `ApplicationErrorSet::uses`, and `ApplicationErrorFootprint::codes` follow the corpus construction rule: they are pure deterministic declarations and may be evaluated more than once across registration and contract tests. The framework materializes and validates their returned values before publishing a registry, then uses the compiled copies at runtime; it never calls declaration functions while shaping an application failure. For typed handler registration, those materialized copies are the private pending state resolved against the completed command and capability graph. They never serialize, enter a catalog, or survive failed finalization. A panic unwinds construction under the embedding's Rust panic policy and is never captured as an application message or `HandlerFailed` response.

`AllApplicationErrors<E>` implements `ApplicationErrorSet<E>` by creating one capability-free, recovery-free `ApplicationErrorUse` for every code in `E::declarations()` and is the ergonomic default for small error enums without recovery edges. Capability binding always requires an explicit set marker, making adoption visible at the command boundary. A command needing a subset, authored recovery, or capability binding defines a zero-sized set marker—normally through `application_error_set!`—whose uses can name only codes from `E` and own only command-specific recovery/cardinality/binding. The type system no longer gives a command set any field with which it could restate or alter summary, details schema, or message policy. `DeclaredApplicationError` has private fields; the exact bounded `From<E>` implementation above attaches the selected marker without exposing a way to swap sets at runtime.

The result-dialect adapter infers `S` from the handler return type, joins exactly `S::uses()` with `E::declarations()`, and projects the resulting `ApplicationErrorSpec`s rather than every variant in `E`. Emitting a code or selecting a recovery outside that set becomes a runtime contract violation. This gives registration a statically named command error footprint without pretending Rust can infer which enum variants a function body reaches.

`ApplicationErrorSet` is command-scoped because it owns recovery declarations and cardinality. A pre-handler producer such as an ambient binder or typed resource resolver instead names only the codes it can produce through `ApplicationErrorFootprint<E>`. `AllApplicationErrorCodes<E>` is the broad default; `application_error_footprint!` defines a checked subset. `ProducedApplicationError<E, F>` has private fields, and its exact bounded `From<E>` implementation attaches the producer footprint. Runtime conversion first verifies that `E::code()` belongs to `F::codes()`, then validates the value's message, details, and recovery selection against the active command's `ApplicationErrorSet`. This separation lets one shared producer feed commands with different command-scoped recovery declarations without letting it invent a code.

The two marker roles are intentional despite their similar shapes. The error type owns server-wide identity. A command result set owns recovery alternatives and cardinality because those semantics vary by operation. A reusable binder or resolver can own only the finite code inventory it may emit; giving it a command error set would either import one command's recovery into another or force every consumer to advertise the producer's entire error enum.

The optional declarative macros expand only to a zero-sized marker and the corresponding public trait implementation:

```rust
application_error_set! {
    pub struct TabsNewErrors for BrowserFailure {
        ApplicationErrorUse::new("session_required")
            .recover_with("session start")
            .at_most_one_recovery(),
    }
}

application_error_footprint! {
    pub struct SessionBinderErrors for BrowserFailure {
        "session_required",
        "session_expired",
    }
}
```

The body of `application_error_set!` is a comma-separated sequence of `ApplicationErrorUse` expressions with an optional trailing comma. The body of `application_error_footprint!` is the same shape for `&'static str` code expressions returned by `codes()`. Both accept ordinary Rust visibility before `struct`. They generate no declaration, global registration, runtime value, recovery inference, or private constructor access; registration validates their results exactly as it validates the manual implementations shown in the guide. The macros keep ordinary declarations compact while the distinct traits preserve authority.

`ApplicationOutput` is a public sealed trait: only Twill's portable success value and framework-provided output wrappers implement it. Applications compose those wrappers rather than implementing the trait and supplying arbitrary resource-edge inventories. This makes handler-type extraction an authoritative declaration rather than an honor-system callback.

`ApplicationSuccess<T>` is the canonical portable typed success shape. Its value and resource-component fields are private. It implements `ApplicationOutput<Value = T>` and provides `ApplicationSuccess::value(value)` plus `From<T>`, so a simple handler may return `Ok(value.into())` through the `ApplicationResult<T, E>` alias. Only framework wrapper methods can attach grant or listing references.

Result-aware handlers deliberately do not expose supplemental `CommandOutput.text`, `stderr`, or `next_cursor` components. Those fields remain source-compatible on legacy `CommandOutput` handlers, but they have no single truthful projection across CLI envelopes, native MCP output schemas, and generated hosts. Portable display text, logs intended for the caller, and pagination state belong in `T` and its schema. A result-aware success text projection is the newline-free compact JSON serialization of the already materialized and validated `serde_json::Value`; string values retain their JSON quotes and escapes rather than becoming an undeclared prose channel. CLI text profiles and an unmodified compact-JSON generated host use those bytes for every result shape; a native MCP surface uses the same bytes as its first content part only for a protocol-compatible object-only success contract. A host that declares fixed result-property omission serializes its separately validated projected value under RFC 0019. This rendering is not a second schema or hash canonicalizer, and it never reserializes the original Rust type. A future RFC may add another typed cross-surface component only with schema and transport semantics for every serving profile.

RFC 0012's existing `Granted<R>` and `Listed<R>` wrappers gain a second defaulted payload parameter: `Granted<R, O = CommandOutput>` and `Listed<R, O = CommandOutput>`. Existing `Granted<T>`/`Listed<T>` annotations and `CommandOutput::grant`/`listing` behavior remain source-compatible through the default. Each wrapper retains the concrete `Grant<R>` or `Listing<R>` value alongside its inner output until recursive conversion into `ApplicationSuccess`; the runtime references therefore travel with the same wrapper type that supplies the static edge. Calling the sealed `ApplicationOutput::grant` or `listing` methods returns `Granted<R, O>` or `Listed<R, O>` and preserves `O::Value`; wrappers may nest to derive and carry multiple edges. Handlers with such components return `ApplicationOutputResult<O, E, S>` for their concrete `O: ApplicationOutput`; the simpler `ApplicationResult<T, E, S>` is the value-oriented alias for `ApplicationOutputResult<ApplicationSuccess<T>, E, S>`. `handle_result` derives the success schema from `O::Value`, recursively derives resource edges from `O`, and recursively moves every retained grant or listing into the final success value.

Wrapper conversion visits the inner output first and then appends the outer component. Within the separate runtime `grants` and `listings` arrays, this preserves fluent call order; each `Listing<R>` also preserves its input iterator's id order. Repeated runtime references are retained because one call may legitimately grant or enumerate several values of the same resource. Static `granted()` and `enumerated()` inventories are instead declaration sets: registration rejects no valid repeated wrapper, deduplicates by resource name, and sorts by canonical resource name before command/catalog hashing. Grant and listing inventories remain separate, so their relative cross-category wrapper order has no public meaning. Equivalent wrapper nestings that differ only by interleaving grant versus listing calls therefore derive identical static edges while retaining the same within-category runtime order.

The exact erased tuple machinery beyond those public relationships is implementation latitude, but the inferred dialect marker cannot be crate-private: Rust must name the selected marker while type-checking an external `handle_result` call. The result dialect therefore reuses RFC 0012's already-public `ContextOnly`, `ContextAndArgs`, `WithResources`, and `WithResourcesAndArgs` marker family. Its output slot carries the complete `ApplicationOutputResult<O, E, S>`; the dynamic dialect uses the same family with `DynamicApplicationResult<O>`. Authors still never write the marker, and Twill adds no parallel public marker vocabulary. Typed registration derives the success schema and selected `ApplicationErrorSet` before serving. Dynamic registration marks its schema and error declarations as explicit and does not substitute `serde_json::Value`'s unconstrained derived schema; it validates runtime values against the bound contract. Both paths continue deriving resource edges only from sealed output wrapper types.

Application errors may originate in a handler or in a framework-integrated application component such as an RFC 0016 ambient binder or typed resource resolver. Such a component registers its error type and exact `ApplicationErrorFootprint`; every command it can fail must declare those codes in its result contract. This is deliberate production of an application outcome, not translation of a framework error after the fact. Planning, authorization, request-integrity, and ordinary RFC 0012 refusals remain framework-owned.

The declaration-only structural emitter above is not an application component and registers no `E` value or Rust `ApplicationErrorFootprint`. Its hash-covered surface mapping is a fixed one-code footprint checked against every reachable command at surface compilation. Runtime can only select whether its declared structural condition occurred; it cannot supply any application-owned field of the outcome.

The runtime representation is private but behaves as:

```rust
enum HandlerOutcome {
    Success(CommandOutput),
    ApplicationError(ApplicationErrorBody),
}

pub enum CommandExecutionOutcome {
    Success(RunResponse),
    ApplicationError {
        plan: InvocationPlan,
        error: ApplicationErrorBody,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationErrorBody {
    pub code: String,
    pub message: String,
    pub details: serde_json::Value,
    pub recoveries: Vec<ApplicationRecovery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ApplicationRecovery {
    Operation { operation_id: String },
    Action { code: String, summary: String },
}
```

`ApplicationErrorBody` is the canonical command-layer application failure. Its recovery array uses the exact internally tagged wire forms `{ "kind": "operation", "operationId": "..." }` and `{ "kind": "action", "code": "...", "summary": "..." }` in declaration order. Operation entries retain catalog operation ids at this layer; they never guess a serving tool name or required model arguments. Action entries are explicitly non-callable. Empty recoveries serialize as an empty array so framework emission has one predictable canonical shape. Deserialization follows the corpus additive unknown-field policy; accepted extensions never enter normalized framework output unless a later RFC declares them.

The existing public `CommandHandler` trait remains source-compatible and continues to return `Result<CommandOutput>`. Registry storage changes to a private common erased-handler trait. A legacy adapter maps `Ok(CommandOutput)` to `HandlerOutcome::Success` and preserves `Err(FrameworkError)` as the outer framework failure. The new `handle_result` and explicit dynamic-result builder paths install result-aware adapters that alone can construct `HandlerOutcome::ApplicationError`. Downstream implementations of `CommandHandler` therefore need no signature change and cannot accidentally turn an ordinary framework error into an expected application outcome.

`CommandBuilder::handle_result` uses a result-dialect inference family parallel to RFC 0012's existing `ResourceDialect`: closure marker implementations cover context-only, context-plus-args, resources-plus-context, and resources-plus-context-plus-args signatures. One adapter derives resource use from `P: ResourceParams`, result/resource-output facts from `O: ApplicationOutput`, and the command error set from `S: ApplicationErrorSet<E>`. RFC 0017 refines the argument-bearing implementations to use checked extraction whenever the command declares constrained schemas. This keeps composition in the handler signature and avoids separate `handle_result_with_resources`, `handle_constrained_result`, or other combinatorial entry points.

The builder methods retain RFC 0006's mutable command-builder convention:

```rust
pub trait ApplicationResultDialect<M>:
    private::Sealed<M> + Send + Sync + 'static
{
    // Framework-owned registration and erasure hooks.
}

pub trait DynamicApplicationDialect<M>:
    private::Sealed<M> + Send + Sync + 'static
{
    // Framework-owned resource/argument extraction and erasure hooks.
}

impl CommandBuilder {
    pub fn result_contract(
        &mut self,
        contract: ApplicationResultContract,
    ) -> &mut Self;

    pub fn handle_result<M, H>(&mut self, handler: H) -> &mut Self
    where
        H: ApplicationResultDialect<M>;

    pub fn handle_dynamic<M, H>(&mut self, handler: H) -> &mut Self
    where
        H: DynamicApplicationDialect<M>;
}

impl CommandRegistry {
    pub fn register_result<M, H>(
        self,
        spec: CommandSpec,
        handler: H,
    ) -> Self
    where
        H: ApplicationResultDialect<M>;

    pub fn register_dynamic<M, H>(
        self,
        spec: CommandSpec,
        handler: H,
    ) -> Self
    where
        H: DynamicApplicationDialect<M>;
}
```

`ApplicationResultDialect` is the typed result-aware analogue of RFC 0012's `ResourceDialect`; its marker parameter is inferred and never written by authors. For example, the resources-plus-context-plus-arguments implementation selects `WithResourcesAndArgs<P, A, ApplicationOutputResult<O, E, S>>`. `DynamicApplicationDialect` selects the same public marker shape with `DynamicApplicationResult<O>`. The private supertrait is generic over that marker: Twill implements `private::Sealed<M>` and the public dialect for each supported closure shape. This distinguishes otherwise overlapping blanket closure implementations while preventing an external crate from implementing either dialect and forging extraction or erasure hooks. A non-generic private supertrait cannot represent the same boundary: shape-specific blanket implementations may overlap, while implementing it for every type would let external types satisfy the supposed seal. The dialect implementations derive input resource uses from `P: ResourceParams`, derive grant/listing edges from `O: ApplicationOutput<Value = serde_json::Value>`, and check the public dynamic error against the explicit contract stored on the completed builder. Applications construct only `ApplicationSuccess<Value>`, its sealed resource wrappers, `DynamicApplicationError`, or an outer `FrameworkError`.

`CommandRegistry::register_result` and `register_dynamic` are the explicit/generated-server counterparts of the two builder handler paths. They retain `CommandRegistry::register`'s consuming receiver convention and compile through the same private erased-handler and registry-validation path as the builder. `register_result` derives the sole application contract from `H` and rejects a `CommandSpec` whose present `output` already contains an `application` contract, exactly like `CommandBuilder::handle_result`. `register_dynamic` requires `spec.output` to contain exactly one explicit `application` contract and treats its absence as a registration error, exactly like `CommandBuilder::handle_dynamic` plus `result_contract`. The existing `CommandRegistry::register(CommandSpec, handler)` remains the source-compatible legacy success/framework-only path; pairing it with an application-bearing spec remains invalid because `CommandHandler` cannot emit that contract.

Builder call order is not a second contract. Finalization requires exactly one effective handler. A dynamic registration additionally requires exactly one explicit result contract, regardless of whether `result_contract` or `handle_dynamic` was called first, and rejects replacement or absence. Every handler-installing method—legacy `handle`/`handle_typed`, RFC 0017 `handle_constrained`, `handle_result`, and `handle_dynamic`—shares one installation slot and records a build error when any second method targets it; the existing builder's incidental last-write behavior is not retained as hidden authority. Repeating `result_contract` likewise fails even when the two values normalize equally. A typed result handler derives its sole application contract at registry finalization and rejects every explicit application contract, including a semantically identical one; equality testing is contract-test work rather than a second authoring path. Legacy and constrained non-result handlers likewise reject an application contract they cannot emit. Failed finalization invokes no runtime handler or producer method; pure declaration functions retain the construction semantics defined below.

`CommandBuilder` holds output presentation (`format` and `summary`) separately from one explicit application-contract slot until finalization. Calling `.output(OutputContract { application: None, ... })` and `.result_contract(...)` in either order combines them for a dynamic result handler; when presentation is absent, the existing `OutputContract::default()` supplies it before the explicit contract is attached. An `OutputContract` whose `application` is `Some` seeds that same explicit slot; pairing it with `result_contract` or another application-bearing `output` is a duplicate build error even when values agree. A typed result handler combines its derived application contract with application-free output presentation and rejects an application-bearing `output`. A later application-free `output` cannot clear an existing explicit contract. Repeating the whole `.output(...)` presentation declaration is also rejected rather than last-write-wins. Finalization emits one ordinary `OutputContract`, so this authoring separation creates no second catalog or wire shape.

Argument-bearing typed result handlers require `A: DeserializeOwned + JsonSchema + Send + Sync + 'static`. For coarse arguments the additional schema is available for contract tooling but does not replace `ArgSpec`; for RFC 0017 constrained arguments it becomes the required semantic-agreement check. Dynamic result handlers receive the planner-validated argument map, bind their application schema explicitly, and still carry resource authority in typed `ResourceParams` and sealed output wrappers rather than mutable declaration lists.

The existing RFC 0012 `ResourceOutput` dialect likewise remains available for legacy `CommandOutput` handlers. A handler declaring an RFC 0014 typed result uses the sealed `ApplicationOutput` family instead; resource-edge extraction for that path never trusts an application-defined `ResourceOutput` implementation. This additive split avoids retroactively changing a Stage-4 public trait while giving new result-bearing handlers the stronger unforgeable contract.

Under `DeclarationSummary`, `ApplicationErrorBody.message` is the summary from the matching declaration and any supplied runtime message is a contract violation. Under `RuntimeBounded`, the producer must supply a non-empty message. Twill uses the same fixed public-text encoder as RFC 0010 legacy denial detail: it emits `\"`, `\\`, `\b`, `\f`, `\n`, `\r`, and `\t` for their ordinary JSON characters, uses uppercase four-digit `\uXXXX` for every other C0 control, DEL, C1 control, and fixed presentation-unsafe scalar, and leaves every other Unicode scalar unchanged. Truncation occurs only between complete input scalars after their complete escape. If the rendered value exceeds the declared bound, Twill reserves one scalar for `…` and retains the longest complete escaped prefix whose width plus that marker fits; a bound of one therefore renders only `…` for any over-bound non-empty message. The final message, including the marker, never exceeds the declared bound or contains a partial escape. Ordinary safe messages below the bound publish byte-for-byte. Typed producers use `runtime_message`, while dynamic producers supply the equivalent optional field at their erased outcome boundary. Neither mode implicitly calls `std::error::Error::to_string` or copies a broker exception. Variable structured facts should still use validated details when the public contract has them; the bounded mode exists for established flat error dialects that intentionally carry variability in `message`.

Recovery has two authored forms. `recover_with(operation_id)` adds a callable operation declaration. `recover_by(action_code, summary)` adds a stable manual or host action such as `start_chrome` when no Twill command can perform the repair. A capability-bound use has a third, derived form: its callable declarations are exactly RFC 0010's canonical sorted bootstrap providers, with no authored actions. `ApplicationRecoverySelection::Declared` emits every recovery in the composed declaration and is the default ergonomic path for zero-or-one authored recovery or the complete derived bootstrap set. `None` emits none. `Only(keys)` selects a subset by operation id or action code; every key is validated against that command's composed declaration before shaping. Dynamic handlers provide the equivalent owned selection. A runtime value can choose among declarations but can never invent recovery text or a target.

`RecoveryCardinality::Any` is the default. `.at_most_one_recovery()` selects `AtMostOne`, allowing a command to declare several authored alternatives while promising that each emitted error selects zero or one. Runtime validation enforces the promise. Compatibility projections such as RFC 0015's flat VBL dialect may require it. Capability-bound recovery stays `Any`: `Declared` may expose every bootstrap provider, while `Only` may select the subset appropriate to the application refusal.

This permits one stable error code to have contextual recovery. For example, VBL's `target_missing` may select `release_tab` when an owned tab lost its target and `list_tabs` when a raw Chrome target vanished. Help lists both command-declared possibilities where both can arise, while each runtime error carries only its selected path.

`HandlerOutcome` is the erased handler boundary. The registry combines it with the completed plan as `CommandExecutionOutcome`. Framework failures remain the outer `Result::Err`; adapters therefore cannot confuse an expected application failure with planning or infrastructure failure while shaping CLI or native responses.

The canonical registry execution methods change their success type accordingly:

```rust
impl CommandRegistry {
    pub async fn run(&self, request: RunRequest)
        -> Result<CommandExecutionOutcome>;
    // Every existing run_with_context / run_in_lane / workspace-aware
    // execution variant changes its Ok type in the same way.
}
```

`RunResponse` remains the payload of `CommandExecutionOutcome::Success`; dry run also returns that variant with no output. This is an intentional source migration for direct registry callers, which must match the outcome before reading a success response. Keeping `Result<RunResponse>` would force expected application failure into `FrameworkError` and erase the central ownership boundary. Existing `CommandHandler` implementations remain source-compatible as described above; only callers of the execution methods change.

The direct-call migration is one exhaustive match rather than a second execution API:

```rust
match registry.run(request).await? {
    CommandExecutionOutcome::Success(response) => {
        consume_success(response);
    }
    CommandExecutionOutcome::ApplicationError { plan, error } => {
        consume_declared_failure(plan, error);
    }
}
```

The outer `?` continues to handle framework failure. Application code that intentionally wants one local error enum may translate the two branches after this ownership-preserving boundary; Twill does not provide an adapter that collapses them first.

Twill does not retain a parallel legacy `run* -> Result<RunResponse>` family. Such a method could invoke a result-aware command only by relabeling its declared application outcome as a framework error, rejecting an otherwise valid registry, or introducing a second hidden response dialect. The workspace currently has one adapter execution call and direct integration/test callers over the same family, so the pre-1.0 migration is mechanical and leaves one truthful execution contract for every command.

RFC 0020 deferred execution stores the same validated tool outcome as ordinary delivery. Application and framework errors retain their original `CallToolResult` shape and ownership. The selected task profile alone decides whether that tool result appears through legacy `tasks/result` with `failed` status or inline in an extension `completed` task. Task infrastructure, cancellation, polling, and result envelopes cannot relabel the application/framework family defined here.

A serving surface translates operation ids into its own call shape: a CLI-shaped surface renders command help, while a native surface renders a tool name and selector when needed. Declared actions retain their lower-snake-case code and summary on every surface. Runtime details may supply values such as the stale reference or expected state, subject to the declared details schema and normal non-disclosure rules.

### Validation

Registration validates the result graph:

- the success and details schemas are supported, self-contained, and canonicalizable;
- application codes satisfy the naming grammar and do not conflict across declarations;
- every `ApplicationErrorUse` names one identity from `E::declarations()`, and every joined `ApplicationErrorSpec` preserves that identity byte-for-byte;
- a capability-bound use names one hand-declared RFC 0010 capability required by the selected command, authors no recoveries, retains `Any` cardinality, and composes exactly the canonical sorted bootstrap-provider operations; unknown, unrequired, resource-derived, or otherwise mismatched bindings fail;
- application and recovery summaries satisfy the public-text bounds and contain no control characters;
- runtime-message policies have valid bounds and are identical wherever an application code is reused;
- every `recover_with` name resolves to a command in the same catalog;
- every `recover_by` code satisfies the application-code grammar, has a non-empty summary, and is consistent wherever reused;
- command-scoped recovery declarations have unique keys, and every runtime `Only` selection is a subset of the current command/code declaration;
- every runtime recovery selection satisfies its declared cardinality;
- error/spec/footprint inventories are unique and canonical by code, while recovery declarations preserve authored order and runtime subsets emit in that declaration order;
- a typed result handler has no explicit application-contract authority, a dynamic result handler has exactly one, and legacy/constrained non-result handlers have none; every cross-pairing or duplicate fails construction before publication;
- standalone `ApplicationResultContract::with_errors` rejects a capability-bound use rather than serializing an unresolved join; only command-scoped typed finalization or an already complete low-level spec may adopt the binding;
- a low-level capability-bearing `ApplicationErrorSpec` contains exactly the same capability, bootstrap recoveries, and cardinality the typed join would derive;
- every registered pre-handler `ApplicationErrorFootprint` contains unique codes from its error type, and every affected command's declared error set covers that footprint; an infallible producer may use the empty `NoApplicationError` footprint;
- every declaration-only structural emitter names one code already covered by every reachable selected command, can derive its entire body from each joined spec under the static message/details/recovery restrictions above, and participates in the owning surface identity;
- the typed output wrappers' RFC 0012 grant and listing edges agree with the registered resource graph; those links remain output components and do not have to appear in the application value schema.

At runtime, Twill serializes a typed or dynamic success value once, validates that materialized JSON against the compiled success schema, and only then applies output shaping. This catches custom `Serialize`/`JsonSchema` disagreement without double serialization. Dynamic application details, the presence or absence of a runtime message, and the selected recovery keys/cardinality are validated against the matching command/code declaration; bounded-message escaping and truncation then occur before response shaping. An undeclared code or recovery, invalid recovery selection, missing or empty required message, unexpected static-mode message, invalid details value, or schema-invalid success becomes `ResultContractViolation` with a redacted reason; the invalid value or message is not returned to the caller.

Output selection and limits apply after validation. The contract describes the full application value, while `OutputSpec` describes a caller-selected CLI projection of that value. A native tool that advertises the full success schema returns the full structured application value; it may expose shaping only when the surface can advertise and validate a separate schema for the shaped result. Grants are never dropped by shaping, following RFC 0012.

`OutputContract.format` and `OutputSpec.format` remain presentation choices. A command with an application result contract always materializes and validates its structured value even when the CLI-shaped response selects text-only rendering; choosing text cannot turn the declared value into an optional implementation detail.

### Framework And Application Error Families

RFC 0002 gains an application-error response family and one additive framework code:

```rust
pub enum ErrorCode {
    // ...existing framework codes...
    ApplicationError,
    ResultContractViolation,
}

pub enum FrameworkError {
    // ...existing variants...
    ResultContractViolation {
        boundary: ResultContractBoundary,
        reason: ResultContractReason,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ResultContractBoundary {
    Success,
    ApplicationError,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ResultContractReason {
    SerializationFailed,
    SchemaMismatch,
    UndeclaredCode,
    InvalidMessage,
    InvalidDetails,
    UndeclaredRecovery,
    InvalidRecoverySelection,
}

pub struct FrameworkEvent {
    // ...existing metadata-only fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_error_code: Option<String>,
}
```

`ResultContractBoundary` serializes as `"success"` or `"applicationError"`. `ResultContractReason` serializes as `"serializationFailed"`, `"schemaMismatch"`, `"undeclaredCode"`, `"invalidMessage"`, `"invalidDetails"`, `"undeclaredRecovery"`, or `"invalidRecoverySelection"`. JSON Schema uses those same stable spellings.

On CLI-shaped surfaces, an application failure uses `ResponseStatus::Failed`, `ErrorCode::ApplicationError`, and details containing the application code, validated details, and canonical recovery entries:

```json
{
  "status": "failed",
  "error": {
    "code": "application_error",
    "message": "No browser session is available",
    "details": {
      "applicationCode": "session_required",
      "details": {},
      "recoveries": [
        {
          "kind": "operation",
          "operationId": "session start"
        }
      ]
    }
  },
  "steering": [
    {
      "kind": "help",
      "label": "Recover with `session start`",
      "request": {
        "tool": "help",
        "arguments": { "command": "session start" }
      },
      "priority": "primary"
    }
  ]
}
```

The public envelope constructor fixes that projection for direct adapters:

```rust
impl ResponseEnvelope {
    pub fn application_error(
        plan: InvocationPlan,
        error: ApplicationErrorBody,
        profile: ResponseProfile,
    ) -> Self;
}
```

It sets `command` from the completed plan, leaves `output`, `diagnostics`, `replay`, `preview`, and `retry` absent, derives only the operation-recovery steering described below, and uses display title `Application error` with the validated application message as its summary. The plan is included only for `ResponseProfile::Debug`; text, structured, and compact-structured requests retain the same structured application-error envelope without a plan. No profile may collapse the application outcome into display text, a framework diagnostic, or `FrameworkError`. This constructor consumes the already validated body and never re-runs application declaration functions or schema validation.

Each operation recovery also produces one RFC 0002 `SteeringAction::Help` in the same declaration order, targeting command help for that operation. The command layer cannot construct a retry because the recovery declaration does not own the target command's required arguments. An action recovery remains in `details.recoveries` and produces no callable steering request. Native surfaces instead translate the same operation id through their compiled direct/group mapping as specified by RFC 0015; generated hosts consume that translated form and render only their declared bounded text.

Framework planning and transport failures retain their existing codes. `HandlerFailed` is reserved for unexpected handler errors. Its framework `ErrorBody` uses `ResponseStatus::Failed`, the static message `Command handler failed`, and empty public details; MCP, response-envelope, generated-host, task-result, event, and framework-log projections may apply only their declared static operation/code formatting and never serialize the string carried by legacy `FrameworkError::Handler(String)` or an underlying source error. Direct Rust registry callers still receive the original `FrameworkError` and may apply application-owned diagnostics. An application that intentionally exposed handler text moves that condition into a declared static or bounded-runtime application error instead of depending on an infrastructure string.

RFC 0010's legacy `FrameworkError::CapabilityDenied` follows that RFC's declaration-derived steering only for a legacy handler. The result-aware adapter performs the fixed replacement above and emits this static `HandlerFailed` projection. A result-aware application validator exposes a stale or refused proof through a capability-bound declared application error, whose code, message, details, and runtime recovery selection remain application-owned while its callable establishment operations remain RFC 0010-derived.

`ResultContractViolation` identifies an author or broker bug at the declared result boundary and maps to `ResponseStatus::Failed` plus `ErrorCode::ResultContractViolation`. Its static message states that the declared result contract was violated. Public details may name only the operation, `ResultContractBoundary`, and stable `ResultContractReason`; they never contain the invalid value, validator rendering, serialization error text, or application details that triggered it. RFC 0020 stores this ordinary framework tool outcome unchanged and applies only the selected task envelope; generated hosts retain its framework code, and framework events/logs retain only the operation, code, boundary, and reason.

The VBL compatibility fixture classifies released error vectors by their new authority rather than treating the source enum as one application declaration. The initial reconciliation is:

| Released VBL code | Twill owner |
| --- | --- |
| `invalid_request_context` | Framework `InvalidRequestContext` |
| `workspace_context_conflict` | Application declaration produced by the RFC 0016 ambient binder when VBL's session/workspace policy rejects a valid selected root. Malformed or dual-authority raw workspace observations remain Twill `InvalidRequestContext` and are not relabeled to this code. |
| `workspace_unavailable` | Vector-specific: missing/unmatched declared root maps to framework `UnresolvedWorkspaceRequirement`; runtime filesystem/broker unavailability after selection remains an application code |
| `path_outside_workspace` | Vector-specific: model path containment maps to framework `WorkspaceMismatch`; a residual application-derived/runtime path refusal remains an application code |
| `invalid_input` | Vector-specific: planner/schema cases map to their precise framework code; residual broker-domain validation remains an application code |
| Other browser/broker codes | Application declarations on the commands that can emit them |

This is an ownership migration, not missing evidence. The released vector remains in the immutable observation bundle, while `tests/support/vbl.rs` records its application declaration or expected framework mapping. Every vector must have exactly one mapping. A framework mapping uses Twill's framework response shape and redaction even when its serialized code happens to match the released spelling; the adapter never manufactures an application value to preserve an old enum owner.

Framework events remain metadata records rather than response archives. A successful application result records status and existing plan facts but never the value. A declared application failure additionally records only its catalog-declared `application_error_code`; it does not copy the runtime message, details, selected recoveries, serialized response body, or task payload into `diagnostics` or another event field. Framework-owned logs follow the same rule and may name operation, status, and declared code only. The application error body still reaches the immediate tool response or terminal task result because that is its declared protocol destination. An application may log its own domain values in application-owned code, subject to its deployment policy; Twill does not do so implicitly.

RFC 0015's native tool projection may render `ApplicationErrorBody` directly while setting MCP `isError: true`. It must not render framework failures as application errors or application errors as protocol errors.

### Projection

- **Operation catalog.** Each operation projects `result.successSchema` and `result.errors`.
- **Help.** Full command help renders the output summary and expected application errors with callable operations and non-callable actions labeled distinctly.
- **MCP.** A surface that corresponds to one protocol-compatible object-only result contract may set `Tool.outputSchema` to the success schema. Successful `structuredContent` must validate against it. Application and framework tool errors set `isError: true`, put their transport-owned compact body in text content, and omit `structuredContent`; a shared CLI-shaped execution tool keeps its generic envelope schema because it dispatches multiple commands.
- **Text.** Application errors render their code, message, and typed recovery paths without requiring structured-content support.
- **Text success.** Result-aware successes render the compact JSON bytes of the validated materialized value; transports do not invent prose or call application `Display` implementations.
- **Events and framework logs.** Success values never enter them. Application failures contribute only status and the declared application code; messages, details, recoveries, response bodies, and task payloads remain absent.
- **Contracts.** `check_result_projection` verifies spec/catalog/help/schema agreement and exercises declared dynamic examples.

### Required Invariants

- Every projected success schema comes from the command's declared result contract.
- Typed results derive schemas from `ApplicationOutput::Value` and resource edges from the output wrapper type; every materialized typed or dynamic success value is schema-validated before projection.
- Expected application failures never collapse into `HandlerFailed`.
- An RFC 0010 proof refusal from a result-aware handler is an explicitly capability-bound application error; using legacy framework `CapabilityDenied` on that path is an unexpected handler defect whose complete denial value is replaced by the fixed framework-owned `Handler` diagnostic and produces static `HandlerFailed` without relabeling.
- Existing `CommandHandler` implementations remain valid success/framework-only handlers; application outcomes require an explicit result-aware registration path.
- One result-aware handler signature composes resource parameters, schema-checked arguments, application output wrappers, and declared errors without duplicate builder methods.
- A typed application component may emit only errors declared by the command it is preparing; framework failures are never relabeled to satisfy an application contract.
- Unexpected handler errors and result-contract violations retain distinct framework codes and never masquerade as declared application errors. Model-visible `HandlerFailed` projections are static and empty-details; the legacy handler string remains available only to direct Rust callers and application-owned diagnostics.
- Every application code emitted by command preparation, a binder/resolver, or a handler is declared for that command, and every callable recovery resolves in the catalog.
- A declaration-only structural emitter names only a code already joined into the selected command, constructs its complete static body from that spec, carries no application runtime value, and is hash-covered by the owning surface; no generic adapter hook can author another command application outcome.
- A pre-planning adapter-owned application-family rejection is never a `CommandExecutionOutcome`; it is valid only through an explicit profile-scoped use that references one server-wide identity, obeys its message/details contract, is hash-covered by that profile, and cannot project on another serving surface.
- Every emitted recovery is selected from the current command/code declaration; sharing an error code never imports another command's recovery edges.
- Capability-bound application recovery contains only the required explicit capability's canonical bootstrap providers; refresh providers, resource-derived capabilities, authored actions, and duplicated recovery prose never enter that binding.
- Every public application-error message follows its declared static or bounded-runtime policy; exception `Display`, provider failures, and undeclared broker strings never become protocol output implicitly.
- Every manual or host recovery action is declared with a stable code and non-empty summary; it is never presented as a callable command.
- Framework errors and application errors remain distinguishable on every transport.
- Framework events and logs remain metadata-only: they never retain successful application values or application-error message/details/recovery payloads; only a declared application code may identify the domain outcome.
- Direct registry execution represents application failure in `CommandExecutionOutcome`, never as `FrameworkError`; all `run*` variants share that return contract.
- Pre-handler application producers declare code-only footprints; the active command remains the sole owner of recovery declarations and cardinality.
- Output shaping never validates a partial value as though it were the full declared success; validation precedes shaping.
- Text versus structured response formatting never changes whether the declared application value is produced and validated.
- CLI text and unmodified compact-JSON hosts render every result-aware success value with the same compact JSON bytes before transport-specific limits; a native MCP surface does so for the object-only contracts it can truthfully advertise as `structuredContent`.
- Result contracts and recovery edges participate in catalog identity.

### Implementation Phases

1. Add result-contract and application-error declarations, schema canonicalization, registration validation, and catalog identity.
2. Add typed and dynamic handler outcomes with success/details validation.
3. Extend RFC 0002 projection for application errors and text degradation.
4. Add help, per-operation MCP output-schema source hooks, contract checks, examples, and acceptance coverage; RFC 0015 later owns native direct/grouped projection of those hooks.

### Acceptance Tests

Acceptance lives in `crates/mcp-twill/tests/results.rs`, with shared schema-dialect fixtures reused by RFC 0017. The RFC 0014 VBL source fixture reads `baseline-tools.json` and `application-error-vectors.json` from RFC 0015's evidence-only fixture bootstrap only after validating their manifest. That bootstrap lands the pinned observation bundle and importer before its consumer RFCs but introduces no RFC 0015 public or runtime API; RFC 0015's later owner-local implementation separately owns comparison of direct/grouped output projection with `surface-catalog.json`. The new per-command `ApplicationErrorSet`, producer footprints, and exhaustive released-vector ownership mapping are authored in `crates/mcp-twill/tests/support/vbl.rs`; tests prove that application-owned declarations reproduce their released observable codes, messages, and recovery values, while framework-owned vectors map to their precise Twill families. This does not claim that VBL v0.4.9 contains a Twill error graph: the fixture records released error observations, while Twill authors the graph in tests/support/vbl.rs.

- The guide-shaped `BrowserFailure` fixture constructs `details()` without a fallible serializer or panic, emits exact empty and `tab_id` objects, and validates both against their declarations before projection.
- A typed handler projects its derived success schema and declared application errors into catalog and help; editing either changes the catalog hash.
- `ApplicationResultContract::for_type::<T>()` uses the fixed draft-2020-12 generator and matches typed-handler derivation. For a fixture needing no typed-only normalization, `new(schema_for!(T))` under the pinned Schemars 1.x default fits the explicit dialect and normalizes to the same canonical schema/hash. A numeric-storage or derived-`Option` fixture proves that the explicit path instead retains and rejects unsupported `format`/range/`anyOf`, while `for_type` applies the exact typed-only rules. A simulated default with an alternate, nested, or malformed `$schema` marker fails rather than drifting the contract.
- Typed result structs containing each recognized Rust integer/float storage type normalize Schemars-only format/range facts to the broader projected JSON primitive, while explicit dynamic contracts with those unsupported keywords fail. A custom assertion that merely resembles part of the recognized pattern is retained and rejected rather than stripped.
- Typed `Option<T>` results normalize Schemars' nullable type-array or provably non-null `anyOf` form into the canonical nullable type/`oneOf` dialect. Reversed nullable type order hashes identically; an `anyOf` whose non-null branch may itself accept null and an explicit dynamic `anyOf` both fail rather than changing union semantics.
- `ApplicationErrorDecl::new` and `ApplicationErrorUse::new` produce byte-identical composed specs to explicit declaration-summary/closed-empty-details and empty-recovery/`Any` defaults; builder defaults cannot create a second hash spelling.
- A capability-free `ApplicationErrorUse` and legacy JSON without `capability` normalize identically; `AllApplicationErrors` remains capability-free until an explicit set marker adopts `.for_capability(...)`.
- `ApplicationResultContract::with_errors` accepts the same context-free set it accepted before capability adoption and rejects a capability-bound set with a stable build error. `handle_result` over that set resolves it only at command finalization, and no pending declaration serializes or survives a failed build.
- Standalone declaration-value fixtures prove that repeated `details_schema` and `runtime_message` calls replace their visible fields, `at_most_one_recovery` is idempotent, recovery methods preserve append order, and explicit or typed result-error methods append. Each accepted chain compiles identically to the equivalent direct public value; duplicate recovery keys and application codes fail at the documented join or registration boundary rather than depending on hidden call history.
- An existing custom `CommandHandler` implementation compiles unchanged and maps success/framework failure exactly as before; it cannot emit a declared application error without moving to a result-aware builder.
- Instrumented declaration callbacks may be evaluated across equivalent builds but are never called during runtime shaping; failed construction calls no handler/producer trait method and publishes no registry. Owned sidecars still run ordinary Rust `Drop` and must tolerate never reaching publication. A panicking declaration unwinds the test construction boundary, and its payload never appears in framework output.
- A legacy `OutputContract` without `application` and a newly constructed contract with `application: None` normalize to byte-identical catalog data and hash input; result-aware fields appear only after explicit adoption.
- A legacy `FrameworkEvent` without `applicationErrorCode` and an event with `None` serialize identically; declared application failures set only that additive field and do not change existing event metadata or diagnostic shapes.
- One VBL-style handler combines `Res<Session>`, constrained `WaitArgs`, `Granted`/`Listed` output wrappers, and `BrowserFailure`; registration derives every contract and dispatch extracts each layer once.
- A dynamic handler returning a schema-valid value succeeds; a schema-invalid value becomes `ResultContractViolation`/`Failed`, exposes only operation, boundary, and stable reason, and never reaches structured output. Ordinary, RFC 0020 deferred, and generated-host projections retain that framework owner.
- The guide's success-only `ApplicationResult<BrowserStatusResult>` path compiles without an error marker, accepts `?` for `FrameworkError`, and makes `Ok(value.into())` the canonical success wrapper. Typed and dynamic result handlers likewise compile with `?` for their declared application and framework error channels. Compile-pass coverage fixes the exact conversion bounds; compile-fail coverage rejects a mismatched error set or producer footprint rather than falling through to an unconstrained blanket conversion.
- An external-crate fixture calls every context/argument/resource form of `handle_result` and `handle_dynamic` without naming a marker. Inference resolves through RFC 0012's public marker family and passes warning-denied clippy; an equivalent crate-private marker fixture fails externally, preventing an implementation from accidentally narrowing the documented callability. Compile-fail coverage also proves an external type cannot implement either dialect because it cannot implement the matching `private::Sealed<M>`, while Twill's marker-keyed blanket implementations remain non-overlapping.
- A dynamic broker closure can construct success, declared application failure, and outer framework failure through `DynamicApplicationResult` without access to `HandlerOutcome`. Resource-parameter and sealed grant/listing variants derive the same static edges as typed handlers; compile-fail coverage rejects an explicit mutable resource-edge list or an application implementation of the sealed dialect/output traits.
- Dynamic builder fixtures set `result_contract` before and after `handle_dynamic` and compile to the same command contract. Missing or repeated contracts fail finalization without invoking application code; every pairwise combination of legacy, constrained, typed-result, and dynamic handler installers rejects the second installation rather than silently replacing the first. Typed result handlers reject any explicit application contract, including a semantically identical `result_contract` or application-bearing `output`; legacy and constrained non-result handlers reject the same impossible pairing.
- Low-level `CommandRegistry::register_result` and `register_dynamic` fixtures compile to the same command spec, catalog, hash, validation result, and runtime outcome as their `CommandBuilder::handle_result` and `handle_dynamic` counterparts. The typed low-level path rejects an explicit application contract, the dynamic low-level path rejects a missing contract, and legacy `register` rejects an application-bearing spec without changing existing `CommandHandler` source compatibility.
- Builder fixtures set application-free `output` and `result_contract` in both orders around `handle_dynamic` and compile to the same `OutputContract`. An application-bearing `output` is the same explicit slot, duplicate application contracts and repeated output presentation fail, a typed result handler combines only application-free presentation with its derived contract, and a later application-free output call never clears a stored explicit contract.
- A typed value with deliberately inconsistent custom `Serialize` and `JsonSchema` implementations becomes the same redacted `ResultContractViolation`, proving derivation alone cannot bypass runtime truthfulness.
- A declared application error returns `ApplicationError` in the CLI envelope with exact application code, validated details, and internally tagged declaration-derived recovery entries. Each operation entry also yields one ordered RFC 0002 help-steering action for its catalog operation; actions remain non-callable and yield no steering request. Empty, operation, action, and mixed recovery fixtures round-trip through JSON and JSON Schema with no alternate `recoverWith` spelling.
- `ResponseEnvelope::application_error` produces the exact command, error, display, steering, and empty-field shape above. Only `Debug` includes the completed plan; `Text`, `Structured`, and `CompactStructured` retain the same structured application-error body without a plan, and none relabels it as a framework diagnostic or invokes declaration/schema code again.
- A result-aware command requiring an explicit RFC 0010 capability binds one declared application error to it. With one bootstrap provider and two self-dependent refresh providers, registration derives only the canonical bootstrap operation into catalog, help, CLI/native recovery, and hash input; provider registration order is identity-neutral, `Declared` emits the full bootstrap set, and `Only` accepts a canonical subset.
- Capability binding rejects an unknown, unrequired, or RFC 0012 resource-derived capability, any authored recovery or `AtMostOne` cardinality on the use, and a low-level composed spec whose recoveries differ from the sorted bootstrap set. The same handler returning legacy `FrameworkError::CapabilityDenied` yields the exact fixed `FrameworkError::Handler` diagnostic to a direct registry caller and static `HandlerFailed` with no capability, denial detail, or recovery steering on every serving surface, while an unchanged legacy handler retains RFC 0010 behavior.
- One command declares two possible recoveries for a stable code and two typed error values select different single paths; undeclared selections become redacted `ResultContractViolation`.
- Reordering error declarations, command uses, or producer-footprint codes yields byte-identical catalog/hash data; reordering declared recovery alternatives changes help/catalog identity, while reordering the same unique runtime `Only` selection yields byte-identical application output and duplicate runtime keys fail redacted.
- `AtMostOne` accepts either single alternative at runtime and turns a multi-selection into a redacted contract violation.
- Static-message typed and dynamic producers emit the identical declaration summary even when internal `Display` or broker strings differ. Registration rejects summaries containing C0, DEL, C1, or any fixed presentation-unsafe scalar. A bounded-runtime declaration requires an explicit non-empty message and shares RFC 0010's exact short-escape, uppercase `\uXXXX`, complete-escape, and final-bound rules.
- A VBL-style `unknown_tab` declaration with empty details and bounded runtime messaging preserves a normal identifier-bearing message and `FlatSingleRecovery`; exact vectors cover a bound of one, an escape ending exactly at the bound, every short escape and unsafe range, and truncation that reserves the final scalar for `…` without splitting an escape or exceeding the declared bound.
- An undeclared application code or invalid details object becomes `ResultContractViolation` without leaking the invalid payload.
- Error-detail objects reject undeclared extension keys by default; a declaration with an explicit extension schema accepts and validates them.
- Table-driven result fixtures exercise every supported assertion keyword at its acceptance and rejection boundaries, the observed nullable number type array, reachable acyclic local references, and result-side `oneOf`; dead definitions, boolean schemas, unsupported keywords/reference forms, and numeric literals outside the exact RFC 8785 I-JSON domain fail registration without silent deletion or rounding.
- A contract violation caused by an adversarial value exposes only operation, boundary, and stable reason in diagnostics, responses, events, and logs; adversarial values and validator error strings never serialize.
- A legacy handler returning `FrameworkError::Handler` with adversarial source text remains inspectable by a direct Rust registry caller but produces the static `HandlerFailed` error-body message and empty details through CLI-shaped MCP, native MCP, generated-host, and RFC 0020 deferred projections. Transport formatters may add only their declared static operation/code framing; the source text is absent from every projection, event, and framework log.
- MCP `isError` distinguishes application failure from success; native application and framework tool errors omit `structuredContent`, while malformed MCP requests remain protocol errors and framework planning failures retain their framework codes.
- Ordinary and RFC 0020 deferred execution produce the same declared application body. RFC 0020 acceptance separately proves exact legacy and extension status/result envelopes without changing that body or its application ownership.
- Success and application-failure events contain no application value, runtime message, details, recovery selection, response body, or task payload. A failure event contains only the declared application code plus existing metadata, and adversarial valid domain values are absent from framework logs.
- Direct registry callers receive `CommandExecutionOutcome::ApplicationError` with the completed plan, while planning and infrastructure faults remain outer `Err` values.
- A command returns an RFC 0014 application value alongside RFC 0012 grants and listings; the registry exposes the value's authoritative per-operation output-schema source, resource wrappers retain their MCP content components, and CLI output shaping preserves them after validating the full pre-shaped value. RFC 0015 acceptance owns native direct/grouped schema projection from that source.
- A typed `Granted<Tab, ApplicationSuccess<NewTabResult>>` derives the `NewTabResult` success schema and the `tab` grant edge independently. Nested `Listed`/`Granted` wrappers preserve every concrete id and within-category fluent order, including repeated references to one resource; static resource-name edges deduplicate and sort canonically. Interleaving grant and listing wrappers changes no static identity or within-category output, while changing or dropping a component is caught by contract tests.
- Compile-fail coverage proves application code cannot implement the sealed `ApplicationOutput` trait or construct private success values/wrappers to forge grant or listing edges; all accepted output composition uses framework constructors and wrappers.
- Legacy `CommandOutput` handlers retain text, stderr, and cursor behavior. Result-aware registration rejects attempts to smuggle those supplemental components outside the declared application value.
- A result-aware object fixture produces identical newline-free compact JSON text in the CLI text profile, native MCP first content part, and unmodified host projection. Array, scalar, and string fixtures produce the same bytes on CLI and unmodified hosts, strings remain quoted/escaped, and no application `Display` implementation is invoked; RFC 0015 acceptance proves that version-1 native surface compilation accepts a root union or reference only when every reachable branch is object-only and rejects any branch that can accept a non-object rather than publishing an unfulfillable MCP `outputSchema`.
- Registration rejects dangling recovery operations, duplicate recovery keys, invalid or conflicting recovery actions, conflicting server-wide identity declarations for one error code, unsupported schemas, and every handler/application-contract ownership mismatch.
- Registration rejects an `ApplicationErrorSet` that invents or repeats a code; the use type has no fields capable of changing the error identity, while per-command recovery differences remain accepted.
- Manual marker implementations and the two declarative macro forms above produce byte-identical result contracts, producer footprints, validation failures, and catalog hashes.
- Two commands may reuse one server-wide error identity while declaring different recovery alternatives; catalog projection preserves identical code, summary, message policy, and details schema while keeping each command's recovery set distinct.
- Registration rejects empty, overlong, or control-bearing application and recovery summaries before they can reach catalog, help, or responses.
- Registration rejects a binder or typed resource resolver whose `ApplicationErrorFootprint` repeats or invents a code or is not covered by every command it may prepare; the framework-provided empty footprint remains valid for an application-infallible producer, and commands may retain different recovery declarations for the same covered code.
- RFC 0016's delegated `missing_as` fixture registers one declaration-only static code footprint against every reachable required consumer. It emits the selected command's declaration summary, empty validated details, and complete declared recovery without constructing an error value or expanding the command contract; runtime-message, contextual-details, subset-recovery, dead, or uncovered mappings fail surface compilation. The equivalent RFC 0019 host-only use never becomes this command outcome.
- Text-only projection includes the application code and distinguishes callable recovery commands from manual or host actions.
- A declared `start_chrome` action projects as a non-callable recovery action, while `tabs list` projects as a catalog operation; surfaces do not confuse the two.
- Generated contract tests catch drift among handler type, result contract, operation catalog, help, and tool output schema.
- The VBL fixture represents all 63 baseline per-operation output contracts and exports their canonical source schemas, including local definition ownership, after documented canonicalization. RFC 0015 acceptance owns the canonical 27-tool direct/grouped output projection and cross-operation definition deduplication.
- The released VBL error vectors cover the complete browser error-code and recovery-action inventories with fixed safe sample values. The separately authored Twill support fixture assigns every vector exactly once to an application declaration or the reconciliation table's framework family; duplicate and unclassified vectors fail. Application mappings supply per-command `ApplicationErrorSet` markers, use bounded runtime messages only for established variable-message codes, and reproduce ordinary code/message/contextual-recovery values without advertising the whole inventory on every operation. Framework mappings preserve Twill ownership/redaction, and controls or overlong broker text never cross either boundary.

## Drawbacks

Schema validation adds runtime work and registration complexity for every schema-bearing command. Twill validates the already-materialized JSON value rather than serializing twice, but typed handlers still pay the assertion walk because Rust permits custom `Serialize` and `JsonSchema` implementations to disagree.

The error model gains another public distinction. Authors must decide whether a condition is an expected application refusal or an unexpected handler failure, and maintain stable codes once agents depend on them.

Direct callers of `CommandRegistry::run*` must migrate from reading `RunResponse` directly to matching `CommandExecutionOutcome`. This is a deliberate pre-1.0 source break in service of keeping application outcomes out of `FrameworkError`; handler implementations and commands that never emit application errors retain their runtime behavior.

MCP consumers that scraped the legacy `HandlerFailed` detail string lose that undeclared channel. This is intentional: expected, caller-actionable failures move to application declarations, while unexpected implementation text remains available only at the direct Rust/application diagnostics boundary.

MCP `outputSchema` describes successful structured content, not every error form. The protocol requires every returned `structuredContent` value to conform to that schema, so native tool errors omit `structuredContent` and carry their compact declared body in text with `isError: true`; Twill does not rely on SDKs that happen to skip schema validation for error results. Clients still need help/catalog metadata or observed errors to learn application failures. Twill keeps the error declarations in its own catalog and projects them where each transport can carry them.

## Rationale And Alternatives

**Keep output as unconstrained `Value`.** This is flexible and prevents truthful native-tool schemas, generated clients, and result contract tests. The catalog should describe both sides of a command.

**Use `FrameworkError::Handler` for every failure.** This preserves a small enum by erasing the application's recovery protocol. Expected refusals are part of the application contract and deserve stable, declared identities.

**Allow handlers to return raw `CallToolResult`.** That preserves any wire shape but bypasses response profiles, resources, diagnostics, task handling, and contract checks. Projection remains an adapter responsibility.

**Put application strings directly into the framework `ErrorCode` enum.** An open string would mix framework and server namespaces and make framework compatibility guarantees impossible. The additive `ApplicationError` family keeps ownership explicit while preserving the application code in details or native projection.

**Trust typed or dynamic output without validation.** A schema that is not checked is documentation, not authority. Derives make disagreement unlikely but cannot make it unrepresentable, so runtime validation applies uniformly.

## Prior Art

OpenAPI and JSON Schema describe successful values and reusable error objects as API contract. GraphQL distinguishes transport failure from typed application payloads. Rust web frameworks infer response schemas and status families from typed handler returns. Twill applies the same principle while preserving its command planner and multi-transport response profiles.

The [MCP 2025-11-25 tool contract](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) requires servers to return only `structuredContent` that conforms to the advertised `outputSchema`, while tool-execution errors travel in a result with `isError: true`. RFC 0015 therefore keeps its native schema success-only, restricts version-1 native success projection to schemas proven to accept objects exclusively, and carries application-error JSON in text rather than manufacturing a success/error union or returning schema-invalid structured content.

RFC 0012 provides the closest internal precedent: handler signatures derive resource edges, and runtime minting refuses values outside the declared footprint. Result contracts extend that type-and-runtime agreement to the complete application value.

## Unresolved Questions

No architectural questions remain for the initial result boundary. The Rust names and macro forms in this body are the accepted Stage-1 implementation contract; implementation may not substitute an unreviewed spelling. Any later review-driven rename or ergonomic change must return the RFC to design review and amend the managed body before implementation proceeds. Such a revision must retain one outcome-aware execution family, distinct command and producer error footprints, and portable application values without supplemental legacy components.

## Future Possibilities

Generated Rust, TypeScript, or JSON clients could consume result contracts directly. Task progress and partial results could declare schemas from the same type family. A later RFC could let recovery edges carry pre-filled typed retry requests rather than command names alone.

Result schemas may later gain reviewed string patterns and maximums, numeric constraints, array uniqueness/maximums, property-count assertions, boolean schemas, and richer composition when real outputs provide cross-surface validation and compatibility fixtures.
