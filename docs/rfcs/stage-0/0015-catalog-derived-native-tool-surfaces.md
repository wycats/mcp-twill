<!-- exo:15 ulid:01kxby557nkkm470zndby6vtqd -->

# RFC 0015: Catalog-Derived Native Tool Surfaces

- Status: Draft
- Area: MCP tool projection, native tools, grouped tools, schemas, annotations, dispatch
- Target milestone: v0.4
- Depends on: RFC 0001 (authoritative command catalog), RFC 0003 (confirmation and replay), RFC 0005 (effect-lane tool surface), RFC 0008 (named argument types and unions), RFC 0009 (effective request metadata), RFC 0010 (declared preconditions), RFC 0011 (guidance decomposition), RFC 0012 (first-class resources), RFC 0013 (conversation identity), RFC 0014 (application result contracts), RFC 0017 (schema-constrained arguments), RFC 0018 (invocation and confirmation presentation)

## Summary

This RFC adds catalog-derived native MCP tool surfaces alongside Twill's CLI-shaped effect-lane surface. A server may project one command as one named tool or group several commands behind one named tool with an `operation` discriminator. The projection declares names and membership; input schemas, output schemas, annotations, help, request context, planning, authorization, and dispatch remain derived from the authoritative command catalog.

The same surface compiler makes deferred-execution support contractual across both profiles. Each generated execution tool has one exact `TaskSupportSpec`; operations sharing a grouped or effect-lane tool must agree. RFC 0020 maps that protocol-neutral declaration into one exact task-delivery profile and owns negotiation, lifecycle, storage, access scope, cancellation, and wire-specific result projection. Native surfaces author that choice in their declaration. The compatibility effect-lane compiler retains its shipped legacy 2025-11-25 delivery profile and exposes no parallel delivery authoring API in version 1. RFC 0015 owns only command authoring, grouping coherence, compiled routing, and the task-support inputs that participate in serving-surface identity.

The existing `help` plus effect-lane execution tools remain the default serving profile. Native projection is chosen by an embedding that needs a stable named-tool contract, such as an existing MCP server or a VS Code extension contribution. The same registry may be served through different profiles at different endpoints, and runtime identity records which profile produced the public schema.

RFC 0016 extends the authored native declaration with resource-binding projection because changing a carrier's presence or requiredness changes that native tool contract. It does not add an ambient-binding declaration to the compatibility effect-lane profile in version 1. Effect-lane and bare-registry calls remain argument-bound even though all profiles share the private prepare/dispatch machinery introduced here. A generated host receives ambient behavior only by consuming a compiled native surface that already declares it.

Grouped input schemas keep a populated top-level `properties` map. They contain the union of member argument properties plus a required discriminator, while selected-command planning enforces operation-specific required fields and rejects fields that do not belong to the selected command. This preserves model-facing schema compatibility in clients that do not type top-level `oneOf` correctly. Property-level unions from RFC 0008 and schema-constrained properties from RFC 0017 remain attached to their argument properties.

## Motivation

RFC 0005 deliberately rejected one MCP tool per command. For a new Twill application, a compact catalog plus a small family of effect-lane tools gives agents one discovery model and truthful worst-case annotations. That remains a strong default.

Migration has a different constraint. Visible Browser Lab already ships a public surface of 27 named tools. Some tools map directly to one operation (`new_tab`, `snapshot`); others group related operations (`network`, `memory`, `artifacts`) behind an `operation` field. Its ungrouped baseline would expose 63 tools. Codex and VS Code users, generated extension manifests, tests, and documentation all rely on the 27-tool contract.

Porting VBL to Twill through only the `run` surface would replace direct calls such as `new_tab {}` with a generic request containing a command template and nested argument map. The browser behavior could remain correct while the public API, annotations, schemas, host confirmations, saved prompts, and extension contributions all break. Asking VBL to keep a parallel hand-written tool router would preserve compatibility by retaining the duplication the port is meant to remove.

The grouped schemas also carry hard-earned client evidence. A top-level `oneOf` accurately describes one object variant per operation, but VS Code's model-facing type pipeline has treated such schemas as having no usable top-level properties and coerced arguments to strings. A flat object with an operation enum and property-level composition survives those pipelines. Twill needs to make that compatibility shape a checked projection rather than an application-specific schema generator.

Deferred delivery exposes the same need for one surface compiler. A direct tool can copy one command's `TaskSupportSpec`, while a grouped or effect-lane tool represents several operations through one public MCP entry point. Advertising one task contract for operations that disagree would make the public tool schema less authoritative than the catalog. This RFC therefore requires support homogeneity and leaves the selected protocol's client/server negotiation and state machine to RFC 0020.

## Guide-Level Explanation

A new Twill server continues to use the existing profile by default:

```rust
let server = CliMcpServer::new(registry)?;
```

An application preserving named tools declares a native surface when it constructs the adapter:

```rust
let surface = NativeToolSurface::builder("vbl")
    .application_errors(NativeApplicationErrorDialect::FlatSingleRecovery)
    .confirmation_route(NativeConfirmationRoute::Bridge)
    .framework_help(FrameworkHelpProjection::Omitted)
    .direct("new_tab", "tabs new")
    .direct("snapshot", "page snapshot")
    .group("network", |tool| {
        tool.selector("operation")
            .member("list", "network list")
            .member("get", "network get")
            .member("body", "network body");
    })
    .build(&registry, McpProtocolTarget::V2025_11_25)?;

let server = CliMcpServer::builder(registry)
    .surface(surface)
    .native_confirmation_bridge(bridge)
    .build()?;
```

The corpus's [representative VBL `new_tab`
composition](../README.md#representative-adoption-vbl-new_tab) follows this
direct mapping through RFC 0016 ambient session binding and RFC 0019 generated
host projection. This RFC owns the public tool name, schema, route, and compiled
surface identity in that walkthrough; the command and downstream adapters keep
their own authority.

`direct` changes no command semantics. The tool's input schema is the selected command's argument schema. Its output schema is the command's RFC 0014 success schema, provided the compiler proves every accepted root value is an object representable as MCP `structuredContent`; a root `$ref` or `oneOf` remains valid when every reachable branch is object-only. Version 1 rejects any direct result branch that can accept an array, scalar, string, or null rather than wrapping or weakening it. Its annotations derive from that command's effects and idempotency, and its task support is copied exactly. Calling the tool plans and dispatches the command by operation id; no command string is constructed or reparsed.

A grouped tool adds one surface-only selector. Its schema is a flat object:

```json
{
  "type": "object",
  "properties": {
    "operation": { "type": "string", "enum": ["list", "get", "body"] },
    "tab_id": { "type": "string" },
    "request_id": { "type": "string" },
    "offset": { "type": "integer" },
    "limit": { "type": "integer" }
  },
  "required": ["operation", "tab_id"],
  "additionalProperties": false
}
```

The property set is the union of member arguments whose schemas agree. Only arguments required by every member appear in the tool-level `required` list. After selecting `body`, Twill invokes the `network body` planner, which enforces that operation's additional required fields and rejects arguments belonging only to another member. Its structured diagnostics name the selected operation and field.

Grouped output schemas may use a discriminated `oneOf`, because outputs are consumed after generation rather than used by the model to type arguments. Every variant contains the selector property with a constant or non-overlapping enum. Members whose success schemas are otherwise identical coalesce into one variant with a selector enum; when every member shares one shape, the result is one object rather than a redundant `oneOf`. Version 1 has no flattened-superset authoring mode; a later measured host can propose one with its own truthfulness and client evidence.

Tool descriptions combine an authored surface summary with derived member guidance. A group names every selector value and each command's `use_when` criterion. Direct tools reuse the command summary and description. Help remains catalog-derived: an embedding may expose Twill's framework help tool, map an application help command, or expose both under distinct names.

### How Agents Should Learn This

Agents should follow the serving profile they can see. On a CLI-shaped profile, they start with the primary execution tool and follow effect-lane redirects. On a native profile, they call the named tool directly. A grouped tool description teaches the selector values and the condition for each operation.

The two profiles must not leak instructions into each other. Native server instructions never tell an agent to construct a command string. CLI-shaped instructions never enumerate every native tool name. Both profiles may link to the same catalog resources and command help because the underlying operations are identical.

Task delivery creates no model-visible argument. Help describes whether an operation forbids, permits, or requires deferred execution, while RFC 0020 makes the active protocol profile determine how a host negotiates and retrieves that result. Lane redirects and repair diagnostics remain the same validated tool outcomes under immediate and deferred delivery; agents never invent task controls to repair an application call.

When grouped planning rejects an operation-specific field, diagnostics point to the tool argument and name the selected operation. The agent repairs the direct call; it is not steered toward the generic `run` surface unless the embedding deliberately exposes that as an alternative.

## Reference-Level Explanation

### Surface Model

The rmcp adapter accepts one compiled surface. The authored declaration remains serializable and hashable, while the compiled value owns validated routing and one canonical snapshot:

```rust
pub enum McpToolSurface {
    EffectLanes(EffectLaneSurface),
    Native(NativeToolSurface),
}

pub struct NativeToolSurface { /* private compiled fields */ }
pub struct NativeToolSurfaceBuilder { /* private fields */ }
pub struct NativeToolGroupBuilder { /* private fields */ }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpProtocolTarget {
    V2025_11_25,
    V2026_06_30,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NativeToolSurfaceDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<NativeToolDecl>,
    #[serde(default, skip_serializing_if = "NativeExposurePolicy::is_complete")]
    pub exposure: NativeExposurePolicy,
    pub framework_help: FrameworkHelpProjection,
    #[serde(
        default,
        skip_serializing_if = "NativeApplicationErrorDialect::is_canonical"
    )]
    pub application_errors: NativeApplicationErrorDialect,
    pub confirmation: NativeConfirmationRoute,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NativeExposurePolicy {
    #[default]
    Complete,
    ExplicitSubset { omitted_operations: BTreeSet<String> },
}

impl NativeExposurePolicy {
    pub fn explicit_subset(
        omitted_operations: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self;

    pub(crate) fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FrameworkHelpProjection {
    Omitted,
    Tool { name: String },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub enum NativeApplicationErrorDialect {
    #[default]
    Canonical,
    FlatSingleRecovery,
}

impl NativeApplicationErrorDialect {
    pub(crate) fn is_canonical(&self) -> bool {
        matches!(self, Self::Canonical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum NativeConfirmationRoute {
    Bridge,
    Unavailable,
}

#[async_trait]
pub trait NativeConfirmationBridge: Send + Sync + 'static {
    async fn confirm(
        &self,
        request: NativeConfirmationRequest,
    ) -> std::result::Result<
        NativeConfirmationDecision,
        NativeConfirmationBridgeError,
    >;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NativeToolDecl {
    Direct {
        name: String,
        operation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Group {
        name: String,
        selector: String,
        members: Vec<NativeToolMember>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NativeToolMember {
    pub selector_value: String,
    pub operation_id: String,
}

impl NativeToolSurface {
    pub fn builder(name: impl Into<String>) -> NativeToolSurfaceBuilder;
    pub fn builder_from(
        declaration: NativeToolSurfaceDecl,
    ) -> NativeToolSurfaceBuilder;
    pub fn declaration(&self) -> &NativeToolSurfaceDecl;
    pub fn snapshot(&self) -> &NativeToolSurfaceSnapshot;
}

impl NativeToolSurfaceBuilder {
    pub fn exposure(self, policy: NativeExposurePolicy) -> Self;
    pub fn framework_help(self, projection: FrameworkHelpProjection) -> Self;
    pub fn application_errors(self, dialect: NativeApplicationErrorDialect) -> Self;
    pub fn confirmation_route(self, route: NativeConfirmationRoute) -> Self;
    pub fn tool(self, declaration: NativeToolDecl) -> Self;
    pub fn direct(
        self,
        name: impl Into<String>,
        operation_id: impl Into<String>,
    ) -> Self;
    pub fn group(
        self,
        name: impl Into<String>,
        build: impl FnOnce(&mut NativeToolGroupBuilder),
    ) -> Self;
    pub fn build(
        self,
        registry: &CommandRegistry,
        target: McpProtocolTarget,
    ) -> Result<NativeToolSurface>;
}

impl NativeToolGroupBuilder {
    pub fn selector(&mut self, argument: impl Into<String>) -> &mut Self;
    pub fn member(
        &mut self,
        selector_value: impl Into<String>,
        operation_id: impl Into<String>,
    ) -> &mut Self;
    pub fn title(&mut self, title: impl Into<String>) -> &mut Self;
    pub fn description(&mut self, description: impl Into<String>) -> &mut Self;
}
```

`NativeExposurePolicy::explicit_subset` copies each supplied operation id into the declaration's canonical `BTreeSet<String>`. Literal arrays, borrowed slices, and owned string collections therefore construct the same set and surface identity; duplicate spellings collapse at construction because subset omission is set-like, while unknown or otherwise invalid operation ids still fail surface compilation. An empty set normalizes to `Complete`, including when it arrived through direct struct construction or deserialization, because it omits no operation and creates no distinct serving contract.

The surface builder declares the route through `confirmation_route`. `NativeToolSurfaceBuilder::build` requires an explicit route but does not receive or validate a bridge object. When the compiled route is `Bridge`, `CliMcpServerBuilder::native_confirmation_bridge` supplies the matching private object before adapter finalization; when it is `Unavailable`, supplying a bridge is a construction error. The route is serializable and hash-covered, while the bridge object and its identity remain private runtime state:

```rust
pub struct NativeConfirmationRequest {
    preview: PermissionPreview,
    arguments: BTreeMap<String, serde_json::Value>,
    invocation_fingerprint: String,
}

impl NativeConfirmationRequest {
    pub fn preview(&self) -> &PermissionPreview;
    pub fn arguments(&self) -> &BTreeMap<String, serde_json::Value>;
    pub fn presentation(&self) -> &PreparedConfirmation;
    pub fn invocation_fingerprint(&self) -> &str;
}

pub enum NativeConfirmationDecision {
    Allow,
    Deny,
    Canceled,
}

pub struct NativeConfirmationBridgeError { /* private source */ }

impl NativeConfirmationBridgeError {
    pub fn new(
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self;
}
```

`arguments()` contains only the selected command's validated, model-visible arguments. RFC 0018 derives the one prepared confirmation from those arguments, using declared bounded copy such as “Close tab X?” when available and the surface's stored generic confirmation otherwise, and stores it in `preview().confirmation`. An authorizer's `RequireConfirmation` decision always uses the `DeclaredOrSurfaceDefault` request mode, so the bridge receives one prepared presentation without reimplementing title casing or effect prose. `presentation()` borrows that exact nested value; the private framework constructor guarantees the preview is require-confirmation and its outer and nested operation ids agree. There is no second presentation field, constructor, or mutable accessor that could disagree. The argument map remains available to deliberately custom trusted bridges. These values exclude conversation identity, ambient resource references, private binding facts, and raw request metadata. The request stays inside the embedding, implements neither serde nor JSON Schema, and never enters MCP responses, events, or framework logs. The bridge object is private runtime state. The declared confirmation route and generic presentation defaults, but never the bridge object's identity, errors, or host data, participate in the surface hash.

`Deny` is an explicit policy outcome and maps to the ordinary stable permission-denied family. `Canceled` means the host/user dismissed or abandoned the pending interaction and maps to `ConfirmationCanceled`. An `Err(NativeConfirmationBridgeError)` is an embedding infrastructure failure and maps to `ConfirmationFailed`. `NativeConfirmationBridgeError::new` retains an application source only for ownership/drop; its `Debug` and `Display` are static/redacted, it exposes no `Error::source`, and Twill never copies the source text into a response, event, or framework log. A bridge that wants deployment diagnostics logs through its embedding-owned channel before returning the redacted error.

The surface declaration maps public names to catalog operation ids. It cannot replace command schemas, effects, result contracts, workspace/resource requirements, or guidance. `NativeToolSurfaceBuilder::build(&registry, target)` is the only declaration-to-compiled-surface boundary. Every native caller explicitly selects `McpProtocolTarget::V2025_11_25` or `McpProtocolTarget::V2026_06_30`; there is no default-protocol `build` overload and no parallel `build_for` spelling. Existing `CliMcpServer::new` and `with_config` remain the separate effect-lane compatibility constructors and retain their shipped protocol behavior, including argument-bound RFC 0012 resource carriers. They expose no RFC 0016 ambient-binding authoring path in version 1. The target enum is closed and non-serializing; declaration data and request metadata cannot select it. `builder_from(declaration)` seeds that same finalizing builder from a constructed or deserialized declaration; it does not compile, validate, install, or generate from the declaration by itself. The builder owns both the serializable declaration and any private RFC 0016 binder sidecars needed to validate it; the final server builder separately validates adapter-owned authorizer and confirmation objects. Consequently `NativeToolSurfaceDecl` is inspectable and serializable but does not offer a declaration-only compile route that could omit required runtime objects. An uncompiled declaration cannot be installed on an adapter or consumed by a host generator. `direct` and `group` cover ordinary authoring; `tool(NativeToolDecl)` and `builder_from` admit generated or fully customized declarations through the identical compiler and are not unchecked installation routes.

`NativeToolSurface::declaration()` returns the normalized compiled declaration, not an unvalidated byte-for-byte copy of builder input. Compilation preserves authored order where order is caller-visible—tool order, group-member order, descriptions, and guidance—but completes and canonically sorts declaration facts whose order carries no public meaning, including RFC 0016 resource-binding coverage. Equivalent omitted/default and explicit/default spellings therefore converge before snapshot and hash construction. The native builder requires an explicit `FrameworkHelpProjection`: `Omitted` adds no framework tool, while `Tool { name }` exposes surface-filtered catalog help under that name. Help lists only mapped operations, renders their native direct or grouped call shape, and never teaches an omitted command as callable. An application help command remains an ordinary mapped operation, so both may coexist only under distinct names.

Presentation omission has one exact fallback rather than an implied “group summary” field. A direct tool's final display title is its authored `title` or the command summary; a group's is its authored `title` or the exact public tool name. A direct leading description is its authored `description` or the command description. A grouped leading description is its authored `description` or `Select one operation with `<selector>`.` using the actual selector name. Twill then appends derived selected-member, `use_when`, alternative, and fallback guidance in the fixed projection order. Authored title/description replaces only its stated slot, and every final string participates in the surface hash. RFC 0018 derives the stored generic presentation defaults from this final display title; neither an adapter nor a generated host repeats title casing or fallback prose.

For each native operation route, the version-1 generic presentation defaults are exactly `Running <display-title>`, `Confirmation required`, and `Run <display-title>?` for invocation message, confirmation title, and confirmation message respectively. `<display-title>` is the final direct or grouped title just defined, substituted without quoting or case conversion. The effect-lane compiler applies the same templates to its exact generated MCP annotation title—currently `<tool-name> execution`—so every served route has one surface-owned fallback even though effect-lane operations share a tool. Prefixes and punctuation count toward RFC 0018's bounds; a generated value outside those bounds fails surface construction.

One adapter instance exposes one profile. Serving the same registry through two profiles requires two adapter instances and produces two public schema identities.

This is the initial ownership rule, not a negotiation placeholder. One MCP adapter owns one stable tool list and runtime surface identity for its finalized lifetime. Serving another named surface means constructing another adapter and endpoint/router over the same registry; request metadata cannot select a different schema contract inside one adapter. A stateful legacy connection negotiates the adapter's compiled revision once, while a stateless 2026-06-30 request proves that same revision independently at ingress. Per-request versioning selects whether the request may use the adapter; it never recompiles or swaps the adapter's surface.

### Canonical Surface Snapshot

Compilation produces one immutable, versioned snapshot shared by MCP projection, contract fixtures, and RFC 0019 host adapters:

```rust
pub struct NativeToolSurfaceSnapshot {
    version: u32,
    protocol_version: String,
    name: String,
    catalog_hash: String,
    surface_hash: String,
    declaration: NativeToolSurfaceDecl,
    server_instructions: String,
    tools: Vec<rmcp::model::Tool>,
    operations: Vec<NativeSurfaceOperation>,
    document: serde_json::Value,
    canonical_json: Box<[u8]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeSurfaceOperation { /* private compiled fields */ }

#[derive(Debug, Clone, PartialEq)]
pub struct NativeSurfaceCall { /* private compiled fields */ }

impl NativeToolSurfaceSnapshot {
    pub fn version(&self) -> u32;
    pub fn protocol_version(&self) -> &str;
    pub fn name(&self) -> &str;
    pub fn catalog_hash(&self) -> &str;
    pub fn surface_hash(&self) -> &str;
    pub fn document(&self) -> &serde_json::Value;
    pub fn canonical_json(&self) -> &[u8];
    pub fn declaration(&self) -> &NativeToolSurfaceDecl;
    pub fn server_instructions(&self) -> &str;
    pub fn tools(&self) -> &[rmcp::model::Tool];
    pub fn operations(&self) -> &[NativeSurfaceOperation];
    pub fn operation(&self, operation_id: &str) -> Option<&NativeSurfaceOperation>;
}

impl NativeSurfaceOperation {
    pub fn spec(&self) -> &OperationSpec;
    pub fn call(&self) -> &NativeSurfaceCall;
    pub fn presentation_defaults(&self) -> &SurfacePresentationDefaults;
}

impl NativeSurfaceCall {
    pub fn tool(&self) -> &str;
    pub fn arguments(
        &self,
    ) -> Option<&BTreeMap<String, serde_json::Value>>;
}
```

The snapshot and its nested views are immutable compiled capabilities, not a second declaration or wire dialect. All fields are private, they expose no constructors or mutators, and `NativeToolSurfaceSnapshot`, `NativeSurfaceOperation`, `NativeSurfaceCall`, and RFC 0018's `SurfacePresentationDefaults` implement neither `Serialize`, `Deserialize`, nor `JsonSchema`. The identity accessors and semantic accessors borrow or copy the private fields shown above; no accessor lazily deserializes `document`. `protocol_version()` names the exact MCP wire revision against which the tool objects and capabilities were compiled. `arguments()` returns `None` for a direct route and `Some` containing exactly the one compiled selector/value entry for a grouped route. `operation()` indexes the same declaration-ordered slice returned by `operations()` and compares exact catalog operation ids. RFC 0020 additively extends the declaration, snapshot, and typed accessors with its compiled delivery view.

The native compiler first completes one private semantic representation, then moves its normalized declaration, instructions, tools, and operation views into the snapshot while constructing the canonical document from those same values. The document and typed fields are intentionally stored together: consumers need stable borrowed MCP and catalog objects, and regenerating or reparsing either form would create a second compiler boundary. Contract fixtures compare every typed accessor with its corresponding document member, including order, omission, MCP tool object, operation spec, call arguments, and presentation defaults. RFC 0020 extends the same one-representation rule to its delivery profile and capability objects. A consumer uses the typed views for Rust logic and the document/canonical bytes for identity or cross-language artifact embedding; it never parses the document back into Twill types.

Version 1's canonical document has this exact top-level shape (angle-bracket values denote the ordinary normalized serialization owned by the named type):

```json
{
  "version": 1,
  "protocolVersion": "2025-11-25",
  "name": "vbl",
  "catalogHash": "<lowercase SHA-256>",
  "declaration": "<normalized NativeToolSurfaceDecl>",
  "server": {
    "instructions": "<exact generated instructions>"
  },
  "tools": ["<exact MCP Tool>", "..."],
  "operations": [
    {
      "spec": "<normalized OperationSpec>",
      "call": {
        "tool": "network",
        "arguments": {
          "operation": "get"
        }
      },
      "presentationDefaults": {
        "invocationMessage": "Running Network Diagnostics",
        "confirmationTitle": "Confirmation required",
        "confirmationMessage": "Run Network Diagnostics?"
      }
    }
  ]
}
```

The JSON strings used as metavariables above are replaced by values, not embedded as strings. `declaration` includes complete/explicit-subset exposure, framework-help mapping, application-error dialect, confirmation route, and RFC 0016's normalized resource-binding projection. `tools` contains the generated public MCP objects, including names, input/output schemas, annotations, titles, descriptions, and any generated framework-help tool. The example uses RFC 0020's normalized `Disabled` task delivery, so the default-valued declaration member and task capability projection are omitted. Legacy or extension delivery adds only RFC 0020's exact `taskDelivery` and capability/tool members to this same canonical document; consumers never infer a profile from generic tool JSON. The document includes the compiler's exact protocol revision because MCP objects are revision-specific. It excludes generic implementation name/version and every connection- or request-observed protocol value. Adapter finalization proves registry/surface agreement; the protocol boundary separately requires either the legacy connection's negotiated revision or each stateless request's `io.modelcontextprotocol/protocolVersion` observation (and matching HTTP header when applicable) to equal the compiled revision before method routing or result projection.

`operations` contains exposed catalog operations only, ordered by declaration tool order and then group-member order. `spec` is the complete normalized operation-catalog object, so RFC 0010/0011/0012/0014/0017/0018 declarations retain their owning wire forms. `call.tool` is always present. Direct calls omit `call.arguments`; grouped calls contain exactly one entry whose key/value are the compiled selector and member value. `presentationDefaults` contains the three exact bounded RFC 0018 surface-owned fallback strings; the command-authored evaluator remains in `spec`. Together, the complete call map plus operation-owned recovery/guidance/resource edges is the compiled translation authority: consumers perform keyed lookup, not surface regrouping or schema generation. Snapshot version 1 has exact compiler targets for MCP revisions `2025-11-25` and `2026-06-30`; it never treats another date as compatible by ordering. The target is compiler/runtime configuration rather than an author-selectable declaration field, and RFC 0020 restricts which task-delivery profiles each target accepts.

The declaration name, top-level name, and snapshot accessor must agree; the top-level protocol version must equal the compiler target and the top-level catalog hash must equal the compiled registry identity. Framework help appears in `tools` but not `operations` because it is not a catalog operation. Tool order is framework help first when projected, then declaration order; operation order ignores that help prefix. Object keys canonicalize under RFC 8785, while every array order above is semantic and preserved. The document contains no `surfaceHash`, raw pre-normalization declaration spelling, registry handler, authorizer, confirmation bridge object, RFC 0020 task store/access provider/record, request context, binder, resolved resource, or process fact.

`NativeToolSurfaceSnapshot` implements neither `Serialize`, `Deserialize`, nor `JsonSchema`. It is an immutable compiled capability, not a second wire declaration. `document()` is the only JSON projection and `canonical_json()` is the only byte projection; the typed accessors expose borrowed semantics without making any Rust snapshot or view serializable. Consumers therefore cannot serialize the Rust fields and accidentally establish a competing shape.

The document excludes `surface_hash` to avoid a self-reference. `canonical_json()` returns the RFC 8785 bytes of that document. Twill computes `surface_hash` with the corpus shared framing and exact domain `io.github.wycats.mcp-twill/native-tool-surface`; the snapshot's `version` supplies `U32_BE(version)`. The document includes `catalog_hash`, so catalog growth or semantic drift invalidates the compiled surface even when an explicit-subset mapping would otherwise produce the same visible MCP tools. Consumers never reserialize `document()` or guess a prefix to establish identity.

The snapshot version identifies the complete compiler contract, not only the document's JSON shape. Every compiler-owned fact that can change published tools, wire behavior, dispatch, task behavior, or a typed accessor must either appear in the canonical document or require a snapshot-version increment. RFC 0020 therefore places its task runtime contract version and fixed record bound in the normalized `taskDelivery` member. A compiler refactor that preserves the exact document, typed accessors, wire fixtures, and runtime semantics may retain the version; reinterpreting the same version and payload to produce different behavior is nonconforming. Because the version participates in hash framing, a required increment changes the surface hash even when every authored declaration is unchanged.

The default effect-lane adapter compiles the catalog hash plus its existing generated tools, help, schemas, exact server instructions, RFC 0020 task-delivery projection, and presentation defaults into a private version-1 canonical surface document and hashes it with domain `io.github.wycats.mcp-twill/effect-lane-surface` under the same framing. It retains public surface name `effect-lanes`. This gives the default adapter an exact serving identity without turning its private compiler result into a second authored declaration API.

`mcp-twill-host` accepts `&NativeToolSurfaceSnapshot`, not `NativeToolSurfaceDecl` or a registry. It uses the borrowed typed views for validation and artifact generation, and nests the unchanged canonical document when establishing downstream identity. This forces generated adapters to consume the already-validated projection rather than parsing the document back into types or rerunning operation mapping, schema generation, guidance translation, or presentation compilation.

The constructor matrix is additive and explicit. New multi-part configuration uses one finalizing builder so bridge/surface agreement is validated only after all private runtime objects are present:

```rust
pub struct CliMcpServerBuilder { /* private fields */ }

impl CliMcpServer {
    pub fn builder(registry: CommandRegistry) -> CliMcpServerBuilder;

    // Existing constructors: default/configured effect-lane surface.
    pub fn new(registry: CommandRegistry) -> Result<Self>;
    pub fn with_config(
        registry: CommandRegistry,
        config: CliMcpServerConfig,
    ) -> Result<Self>;

    // New constructors.
    pub fn with_surface(
        registry: CommandRegistry,
        surface: impl Into<McpToolSurface>,
    ) -> Result<Self>;
    pub fn with_config_and_surface(
        registry: CommandRegistry,
        config: CliMcpServerConfig,
        surface: impl Into<McpToolSurface>,
    ) -> Result<Self>;
}

impl CliMcpServerBuilder {
    pub fn config(self, config: CliMcpServerConfig) -> Self;
    pub fn surface(self, surface: impl Into<McpToolSurface>) -> Self;
    pub fn authorizer(
        self,
        authorizer: Arc<dyn PermissionAuthorizer>,
    ) -> Self;
    pub fn native_confirmation_bridge(
        self,
        bridge: Arc<dyn NativeConfirmationBridge>,
    ) -> Self;
    pub fn build(self) -> Result<CliMcpServer>;
}
```

`with_surface` uses the default transport config and is a convenience only when adapter finalization can synthesize every private adapter-side choice: `DefaultPermissionAuthorizer`, an `Unavailable` native confirmation route, and RFC 0020 `Disabled` delivery or the legacy profile's exact connection/capability runtime default. `with_config_and_surface` changes only the transport config under that same boundary. A `Bridge` route, a custom `PermissionAuthorizer`, or `TasksExtension` delivery requires the finalizing builder so its bridge, authorizer, or atomic store/access pair has an authoring slot. RFC 0016 binder sidecars are already owned by the compiled `NativeToolSurface` and therefore move through either path without becoming adapter-builder arguments. The convenience constructors reject a surface requiring an unsupplied adapter sidecar rather than publishing a partial adapter. Omitting `authorizer` on the finalizing builder installs `DefaultPermissionAuthorizer`, preserving every existing constructor; supplying it installs exactly one base adapter authorizer, and a repeated assignment is a construction error rather than last-write replacement. Existing constructors retain the effect-lane profile, and all constructors delegate to the same validation path.

### Validation

Validation has two explicit owners. `NativeToolSurfaceBuilder::build` validates the serializable declaration against the registry and produces the compiled surface:

- the surface name is 1–64 ASCII characters matching `[a-z0-9]+(?:-[a-z0-9]+)*`;
- tool names are 1–128 ASCII characters from `[A-Za-z0-9_.-]`, and selector values use the same bounded portable grammar; tool names are unique across the surface and selector values are unique within their group;
- every operation id exists, and each group contains at least two members;
- the selector does not collide with a member command argument;
- a command argument name used by multiple members has an identical projected schema, including summary-sensitive constraints that affect callers;
- every member of a group has the same `TaskSupportSpec`, which becomes the group's exact tool-level task contract;
- source schemas have already rejected dead definitions; their reachable local `$defs` are copied, same-name definitions deduplicate when their complete canonical schemas are identical and otherwise make the group invalid, and every retained local `$ref` remains resolvable;
- every direct member has an RFC 0014 application result contract whose resolved root graph accepts objects exclusively; direct `$ref` and `oneOf` roots are valid only when every reachable branch is object-only, and any branch that can accept an array, scalar, string, or null makes the surface invalid;
- every grouped output member resolves through any root `$ref` to one object schema into which the surface selector can be inserted; a member-authored root `oneOf` is rejected even when all branches are objects, while nested unions remain valid. A pre-existing selector is accepted only when required and validation-semantically equal to `{ "type": "string", "const": member_selector }`;
- the selected application-error dialect can represent every projected error without dropping details or recovery branches;
- framework help and application tool names do not collide;
- under `Complete`, every catalog operation is mapped exactly once;
- under `ExplicitSubset`, every catalog operation is either mapped exactly once or named in `omitted_operations`, never both, and every omitted id resolves;
- the exposed operation set is closed over callable recovery, guidance alternative/fallback, resource establishment/enumeration/release, and RFC 0010 capability-bootstrap-provider edges that generated help can steer a caller toward; a self-dependent refresh provider cannot by itself close a missing-proof path;

`CliMcpServerBuilder::build` then validates the compiled surface against the runtime registry and its private sidecars. The surface snapshot's `catalog_hash` must equal the supplied registry's freshly computed catalog hash; a surface compiled from an earlier or different registry is rejected before any adapter is published. A `Bridge` surface has exactly one bridge and an `Unavailable` surface has none. The base authorizer and bridge slots are each single-assignment; repeating either records a construction error instead of replacing private runtime policy. RFC 0019 may compose a host-profile approval authority around that preserved base authorizer for one generated-host entrypoint, but cannot replace it or widen a hard registry policy denial. RFC 0020 applies the same finalization rule to its single-assignment task-runtime pair. Compiled-surface validity is independent of particular private objects, but no adapter can serve a surface until every owning RFC's second-phase validation succeeds. Equivalent surfaces may be compiled for separate adapters over byte-identical registries with distinct private objects without changing their snapshots or hashes.

`Complete` and `NativeApplicationErrorDialect::Canonical` are builder defaults. Group output projection is the single discriminated/coalesced contract described below rather than another author choice. Framework-help projection and confirmation route have no defaults: both affect externally visible availability or failure behavior, so `NativeToolSurfaceBuilder::build` rejects an omitted choice. A profile may intentionally expose only a subset of the registry by supplying a non-empty omitted-operation set through `NativeExposurePolicy::ExplicitSubset`; an empty set normalizes to `Complete`. Subset exposure belongs solely to the surface because the same command catalog may support complete and compatibility profiles. The active surface snapshot records the policy and exact omitted set, and contract tests compare it, so adding a catalog command without mapping or explicitly omitting it fails construction. Omission is allowed only when no exposed command's generated guidance or recovery graph can lead to the omitted operation; internal implementation dependencies that never become caller steering do not create a surface edge. The initial version has no alias facility: one operation has one location within a surface, and a public rename is a surface-contract change.

The serde defaults shown above are the declaration wire contract. `NativeExposurePolicy` is externally tagged as `"complete"` or `{ "explicitSubset": { "omittedOperations": [...] } }`; `FrameworkHelpProjection` is `"omitted"` or `{ "tool": { "name": "..." } }`; the application-error dialect is `"canonical"` or `"flatSingleRecovery"`; and the confirmation route is `"bridge"` or `"unavailable"`. Direct and group declarations use `{ "direct": { ... } }` and `{ "group": { ... } }`, with `operationId` and `selectorValue` field spellings. Missing and explicit empty tools both reach the same coverage validation; omitted and explicit `Complete`/`Canonical` normalize to one compiled declaration and are omitted when that declaration is serialized. Framework help and confirmation remain required deserialization fields because choosing either changes availability or failure behavior. Optional tool presentation omits `title` and `description` rather than writing null. Public additive declarations retain the corpus unknown-field policy; the compiled snapshot remains the closed canonical identity boundary.

Each scalar surface-builder method—`exposure`, `framework_help`, `application_errors`, and `confirmation_route`—may be authored once. A fresh builder's semantic default does not count as an authored assignment, so explicitly selecting `Complete` or `Canonical` is valid and normalizes to the same declaration; a second call for the same slot records a build error even when the values agree. `builder_from` treats every populated scalar in the supplied declaration as already authored and exists to validate that declaration and attach required private sidecars, not to provide last-write editing. `tool`, `direct`, and `group` are additive; their order remains public, while duplicate names, operations, or members fail the coverage rules below. Group `selector`, `title`, and `description` are likewise single-assignment slots, and `member` is additive. This gives every fluent path one construction meaning independent of call order.

### Input Schema Projection

A direct tool starts from `registry.arg_schema(command)`, including property-level named unions from RFC 0008, schema-constrained properties from RFC 0017, and `additionalProperties: false`. A surface cannot override that schema. RFC 0016's declared resource-binding projection is the one compositional refinement: it may omit a framework-derived carrier or change that carrier from required to optional while preserving its catalog-derived property schema. The native snapshot records that binding mode and the surface hash covers the resulting input schema.

A group schema is generated from each member's active direct-tool schema after that same resource-binding projection:

1. Start with the selector property, typed as a string enum of selector values.
2. Union the top-level properties from every member schema. Repeated property names must have identical schemas.
3. Set tool-level required fields to the selector plus the intersection of member required sets.
4. Keep `additionalProperties: false`.
5. Preserve each property's nested composition and copy its reachable local definitions. Source schemas already reject dead definitions. Same-name canonical definitions deduplicate; a same-name semantic disagreement fails grouping rather than renaming caller-visible schema components.
6. Do not attach a union to the command's whole input object.

After deserialization, the adapter removes the selector, resolves the member operation, and sends the remaining map to that command's ordinary binder. The binder enforces selected-member required fields, rejects member-inapplicable fields as unknown arguments, and performs all named-union, workspace, resource, permission, and request-context checks.

### Output Schema Projection

Direct tools use the selected command's full success schema from RFC 0014.

Grouped tools use one discriminated/coalesced output contract. Every member success schema must resolve through any root `$ref` to one object schema. A member-authored root `oneOf` is rejected even when all branches are object-only because selector insertion and shape-class coalescing would otherwise have two competing union owners; nested property unions remain valid. The generator removes any existing selector, canonicalizes each member success schema, and groups members with identical remaining schemas. It then restores the selector as a required property with a constant for a one-member shape class or an enum in surface declaration order for a multi-member class. One shape class projects as a single object; multiple classes project as the one generated root `oneOf`. A pre-existing selector is accepted only when it is required and, after annotation removal, validation-semantically equal to `{ "type": "string", "const": member_selector }`; runtime result validation then proves the application value already agrees before the adapter removes and restores the same value. Every other selector collision fails. Primitive, array, string, scalar, nullable, or member-unioned roots cannot be grouped in the initial version because adding a selector would require an undeclared envelope or composition rule. Reachable same-name definitions deduplicate only when their complete canonical schemas agree; disagreement fails, and every local reference is checked.

The VBL audit supports this boundary: every operation placed in a grouped hybrid tool has an object-root success schema. In the 63-operation baseline, the only root-level output union belongs to direct `list_tabs` and needs no injected selector. The 27-tool hybrid projection intentionally has seven root-level output unions after grouped shape-class discrimination; these occur only on outputs, while all 27 hybrid input schemas retain populated top-level properties and no root `oneOf`. Nested unions inside grouped object results remain supported.

### Dispatch

Native calls do not synthesize `RunRequest.command`. The registry gains an operation-id entrypoint that invokes the same binding and planning pipeline:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum InvocationOrigin {
    #[default]
    CommandTemplate,
    OperationId,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServingSurfaceIdentity {
    pub name: String,
    pub hash: String,
}

pub struct InvocationPlan {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "is_command_template")]
    pub origin: InvocationOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<ServingSurfaceIdentity>,
}
```

Existing command-template planning sets `raw_command: Some(original)` and retains its syntax tokens; the default origin is omitted from existing serialized plans. Operation-id planning sets `origin: OperationId`, `raw_command: None`, and an empty syntax-token list. The origin participates in the invocation fingerprint, so approval or idempotency records never treat a parser-shaped invocation and a native operation call as the same request accidentally. Both origins still bind the same operation id, command path, validated arguments, effects, context, resources, and output request.

Twill keeps one `InvocationPlan` because authorization, previews, replay, events, tasks, resources, and application outcomes consume the same semantic facts for both origins. A separate native-plan type would force every cross-cutting subsystem to become generic over two nearly identical plans or erase them immediately into a third common representation. The intentional pre-1.0 source migration is limited to `raw_command: Option<String>` and exhaustive handling of `InvocationOrigin`; existing command-template wire values retain their original `rawCommand` and omit the default origin and surface fields.

`ServingSurfaceIdentity` makes serving identity one closed Rust and wire authority. Omission produces no `surface` member; presence produces the exact nested object `{ "surface": { "name": ..., "hash": ... } }`. Missing or unknown nested fields fail deserialization, and constructors validate the bounded surface-name grammar plus lowercase 64-character SHA-256 spelling before a plan or runtime identity can carry it. The pair is public contract identity, not a request-selectable surface. These fields are new in this RFC, so the atomic nested shape has no legacy top-level spelling to preserve.

The private preparer records serving identity in the fingerprint for every route. Native MCP plans also set their public optional `surface` from the immutable compiled snapshot; effect-lane and bare-registry plans retain their existing serialized absence. The fingerprint's mandatory additive member is exactly:

```json
{
  "invocation": {
    "origin": "commandTemplate",
    "surface": {
      "kind": "effectLanes",
      "name": "effect-lanes",
      "hash": "<effect-lane surface hash>"
    }
  }
}
```

Native execution uses `origin: "operationId"` and `{ "kind": "native", "name": <surface name>, "hash": <surface hash> }`. Public bare-registry `run*` uses `{ "kind": "bareRegistry" }` with the appropriate `commandTemplate` or `operationId` origin and no invented name or hash. This includes public `run_in_lane*`: its lane argument is a routing assertion, not evidence that the call came through the compiled effect-lane MCP adapter. Only the adapter's private prepared route may attach the `effectLanes` name/hash. No other origin or surface spelling is accepted in version 1. The object joins the existing fingerprint input before its shipped stable JSON hash; it is not serialized on `InvocationPlan` as another field.

Consequently, parser-shaped and operation-id calls, bare and served calls, two native profiles, and a surface before and after a presentation/schema change have different fingerprints. The new mandatory member intentionally invalidates pre-suite approval/replay fingerprints once; after adoption, equivalent calls on the same route remain stable. Surface names and hashes are public execution identity, analogous to `catalog_hash`; they contain no request context or private runtime object and let previews, events, and external approval stores explain which advertised schema produced the fingerprint.

```rust
impl CommandRegistry {
    pub async fn run_operation_with_context(
        &self,
        operation_id: &str,
        arguments: JsonObject,
        invocation: InvocationContext,
    ) -> Result<CommandExecutionOutcome>;
}
```

The three parameters shown are the version-1 public API; the initial implementation does not replace them with an authored request wrapper. Operation resolution happens before dispatch and every existing planner invariant remains active. The convenience method performs one argument-bound prepare-and-dispatch cycle for direct registry integrations and resolves RFC 0009 workspace observations from `InvocationContext` exactly as `run_with_context` does. It does not also accept a pre-resolved workspace set, which would create two workspace authorities when the context carries host roots. The MCP and generated-host adapters use crate-private prepare/dispatch halves whose input is produced by RFC 0009's single observation-assembly step, so lane checks, the configured authorizer, and `NativeConfirmationBridge` inspect one prepared plan and approved dispatch consumes that exact state. RFC 0016 extends only the compiled-native operation-id path's private carrier with ambient binding selections; effect-lane and bare-registry paths use the same carrier shape without selecting ambient state, and no path adds private fields to `InvocationPlan`. RFC 0020 deferred execution invokes these same prepare/dispatch halves and retains RFC 0009 effective request context plus RFC 0013 conversation identity without making either a task-protocol control or access credential.

### Authorizer Configuration

RFC 0003 made `PermissionAuthorizer` public, while the shipped adapter still installs only `DefaultPermissionAuthorizer`. The finalizing `CliMcpServerBuilder::authorizer` slot shown above supplies the additive configuration hook shared by every surface. It captures one `Arc<dyn PermissionAuthorizer>` before the adapter is built; there is no post-construction replacement API.

Omitting the slot preserves `DefaultPermissionAuthorizer`. Each adapter instance therefore owns one base authorizer, so two surfaces over one registry may apply different embedding policy without changing catalog semantics. The authorizer object and identity never serialize or enter hashes; its decisions are runtime policy. Native `RequireConfirmation` decisions still pass through the declared `NativeConfirmationRoute`, including when a custom authorizer produces a decision the static command effects did not predict. RFC 0019 may let a hash-covered generated-host policy satisfy one such decision only on its private host entrypoint; the base authorizer still runs, `Allow` and `Deny` remain unchanged, and an unmatched `RequireConfirmation` still follows this RFC's route.

### Annotations And Effects

A direct tool derives annotations from one command. `readOnlyHint` is true only for pure/read effects; `destructiveHint` reflects delete or explicitly destructive policy; `idempotentHint` reflects the command declaration; `openWorldHint` reflects network or other open-world effects.

A group uses truthful worst-case aggregation:

- read-only only when every member is read-only;
- destructive when any member is destructive;
- idempotent only when every member is idempotent;
- open-world when any member is open-world.

Annotations remain advisory. Twill still performs planning, authorization, resource resolution, confirmation policy, and dispatch checks. Native projection does not reinterpret a tool annotation as permission.

Task support is contractual rather than advisory, so this RFC completes the authoring path that the catalog already projects:

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub enum TaskSupportSpec {
    Forbidden,
    #[default]
    Optional,
    Required,
}

impl TaskSupportSpec {
    pub(crate) fn is_optional(&self) -> bool {
        matches!(self, Self::Optional)
    }
}

pub struct CommandSpec {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "TaskSupportSpec::is_optional")]
    pub task_support: TaskSupportSpec,
}

impl CommandSpec {
    pub fn task_support(self, support: TaskSupportSpec) -> Self;
}

impl CommandBuilder {
    pub fn task_support(&mut self, support: TaskSupportSpec) -> &mut Self;
}
```

The existing `TaskSupportSpec` declaration gains the derived default marker shown above, satisfying the workspace's warning-denied Clippy policy without a manual derivable implementation. `CommandSpec::new` and `CommandBuilder` default to `Optional`. Omitted and explicit `Optional` command declarations normalize identically, and `OperationSpec::from_command_spec` copies the value into its existing always-projected `taskSupport` field. Existing operation-catalog bytes therefore remain unchanged, while `Forbidden` and `Required` become authorable through both low-level and ergonomic paths and change catalog/surface identity. The crate-private `TaskSupportSpec::is_optional` serde predicate is shown only to define the omission spelling across the existing catalog/model modules; authors use the enum and the two declaration methods.

A direct tool copies its operation's exact `TaskSupportSpec`; a native group copies the identical value required of all members. The existing effect-lane compiler applies the same equality rule to the operations reachable through each generated lane execution tool. Existing builder-authored commands remain `Optional` until an author explicitly chooses another value, so their effect-lane behavior is unchanged; mixed support inside one grouped or effect-lane tool fails serving-surface construction instead of advertising a misleading tool contract. Generated framework-help tools are always `Forbidden` because they perform no command execution.

RFC 0020 adds one explicit delivery profile to the native surface declaration and compiler. It maps these support values into the exact disabled, legacy, or extension capability contract, including the extension's server-directed optional policy. The effect-lane compatibility compiler instead selects its fixed legacy 2025-11-25 profile as part of that profile's existing construction contract. RFC 0015 does not infer a native delivery profile from `TaskSupportSpec`, an observed protocol revision, request metadata, or the presence of any task-capable command, and no request can replace the effect-lane compiler's fixed choice.

Native confirmation never adds model-visible arguments. Under `NativeConfirmationRoute::Bridge`, the bridge receives `NativeConfirmationRequest` after planning and before dispatch, performs the host-owned confirmation interaction, and returns one of the typed outcomes above for that exact pending call. Because the call remains pending, approval needs no model-carried replay token and cannot weaken an established `additionalProperties: false` schema. Denial, cancellation, and bridge failure carry no host-authored reason into protocol output. The ordinary effect-lane surface retains RFC 0003's response-and-replay flow.

The registry's existing `PermissionPolicy` is a hard capability gate and runs before the adapter authorizer. A policy denial returns `PermissionDenied` without invoking a confirmation bridge. `PermissionAuthorizer` may further deny or require confirmation but cannot widen the registry policy. The prepared-operation refactor applies this order consistently to effect-lane, native, task, and host entrypoints.

`NativeConfirmationBridge` remains owned by the native MCP adapter because it completes an in-flight MCP invocation after authoritative planning. RFC 0019 generated hosts use their own hash-covered pre-invocation presentation/trust policy on a private entrypoint; they preserve this RFC's base authorizer and may satisfy only a trigger-matching `RequireConfirmation` under that RFC's typed host evidence. They do not share or generalize the in-flight bridge callback. A future transport-level confirmation protocol can introduce a common abstraction only if it preserves these distinct timing and trust contracts.

Under `NativeConfirmationRoute::Unavailable`, allow and deny decisions behave normally. If the base authorizer's `RequireConfirmation` remains unsatisfied, execution fails closed before dispatch with the additive framework code `ConfirmationUnavailable`. This runtime rule is required because a custom authorizer may depend on the completed plan and cannot in general be predicted at server construction. RFC 0019's private host policy may satisfy one matching requirement; an absent, out-of-range, or trigger-mismatched case leaves it unsatisfied and reaches this same fail-closed rule. Tool annotations and presentation declarations by themselves never count as approval.

```rust
pub enum FrameworkError {
    // ...existing variants...
    ConfirmationUnavailable { operation_id: String },
    ConfirmationCanceled { operation_id: String },
    ConfirmationFailed { operation_id: String },
}

pub enum ErrorCode {
    // ...existing codes...
    ConfirmationUnavailable,
    ConfirmationCanceled,
    ConfirmationFailed,
}
```

All three confirmation variants map to `ResponseStatus::Failed` and their correspondingly named `ErrorCode`. The status describes an invocation that did not receive approval and never dispatched; the stable code preserves whether the route was unavailable, the interaction was canceled, or the bridge failed. Each diagnostic names only the public operation and active surface and contains no arguments, permission targets, bridge identity, source error, or host reason. `Unavailable` means the compiled route lacks a bridge, `Canceled` means the bridge returned its explicit cancellation decision, and `Failed` means the configured bridge returned its redacted infrastructure error; adapters never collapse one into another. Framework events and logs may record the operation, surface, and stable code only.

For an ordinary call, native confirmation completes while the outer invocation remains pending. Under RFC 0020 deferred delivery, the public task may already exist, but it receives no authority-bearing execution capsule until confirmation allows the exact prepared invocation. Deny, cancellation, bridge failure, or missing approval produces the same ordinary pre-dispatch `CallToolResult` without invoking an ambient binder, resource resolver, or handler; RFC 0020 alone maps that result into the selected task profile. Dropping a non-task outer invocation future while the bridge is pending cancels that future and drops the prepared state with no framework response. After allow, cancellation is best-effort at the current dispatch boundary: before binder realization it performs no application work, while cancellation after a binder, resolver, broker, or handler accepted work makes no rollback claim. Resource ownership and the operation's idempotency contract govern any surviving effect.

### Task Delivery Composition

RFC 0015 owns `TaskSupportSpec`, command/builder authoring, operation-catalog projection, and the rule that every operation reachable through one generated execution tool has the same support value. It also includes RFC 0020's normalized task-delivery declaration, typed compiled view, public MCP capability objects, and tool metadata in the immutable surface snapshot and surface hash.

RFC 0020 owns the meaning of `Disabled`, legacy 2025-11-25, and Tasks Extension delivery. It validates each profile against the compiled MCP revision, interprets request protocol controls, decides whether an optional extension call materializes a task, stores and authorizes task records, projects terminal outcomes, and defines polling, updates, retention, cancellation, and task telemetry. This RFC never derives a wire lifecycle from `TaskSupportSpec` alone.

Immediate and deferred execution enter the same operation-id planner and prepared-invocation boundary. Native confirmation always approves or refuses the exact prepared invocation. Under deferred delivery, task existence conveys no authority and RFC 0020 creates the private execution capsule only after this RFC's registry policy, adapter authorizer, and confirmation route allow dispatch. The same validated RFC 0014 tool outcome reaches ordinary delivery and RFC 0020's selected task projection.

The surface compiler treats RFC 0020's profile as an identity-bearing input. A profile/revision mismatch, missing runtime store/access sidecar, or stale task capability rejects adapter finalization before tools are published. Runtime task ids, records, stores, access providers, runners, and capsules remain private and excluded from catalog, snapshot, plan, preview, event payload, and invocation fingerprint. The surface hash binds the delivery profile; selecting immediate versus deferred execution at runtime does not change the invocation fingerprint.

### Results And Errors

Application successes and failures project according to RFC 0014. Direct success becomes the declared object application value. Grouped success includes the selector required by the output dialect. Native success follows rmcp's structured-result shape: `structuredContent` contains the value, the first content part is its compact JSON text representation, and `isError` is false. RFC 0012 resource links append after that text part and do not alter the output schema.

Declared application failures set `isError: true`, put the compact JSON representation of the native application-error body in the first text content part, and omit `structuredContent`. The advertised `Tool.outputSchema` remains exactly the successful object contract, so returning a differently shaped structured error would violate MCP's requirement that every structured result conform to that schema. Framework tool errors follow the same transport rule: their redacted response body is text content, never success-schema `structuredContent`. This deliberately corrects VBL's released `structured_error` envelope while preserving its validated code, message, and recovery object as the text JSON source; compatibility does not justify a schema-invalid result.

The canonical native error dialect has its own serving-surface projection:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NativeApplicationErrorBody {
    pub code: String,
    pub message: String,
    pub details: serde_json::Value,
    pub recoveries: Vec<NativeApplicationRecovery>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NativeApplicationRecovery {
    Tool {
        tool: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        arguments: BTreeMap<String, serde_json::Value>,
    },
    Action {
        code: String,
        summary: String,
    },
}
```

A direct recovery target becomes `{ "kind": "tool", "tool": "name" }` with omitted empty `arguments`. A grouped target becomes `{ "kind": "tool", "tool": "name", "arguments": { "selector": "member" } }`, where the actual selector field and member value come from the compiled route. Those arguments are deliberately partial: they contain only surface-owned routing facts and never fabricate the target operation's required model arguments. An action remains `{ "kind": "action", "code": "...", "summary": "..." }`. The array preserves the RFC 0014 declaration order. Twill materializes and validates one `NativeApplicationErrorBody`, then compact-serializes that body once into text content; it does not construct a second structured projection that could disagree with the successful `outputSchema`.

Every native member is result-aware, so legacy-only `CommandOutput.text`, `stderr`, and `next_cursor` components cannot appear and require no lossy native projection. Applications needing those facts on a native surface include them in the RFC 0014 value schema.

`NativeApplicationErrorDialect::Canonical` is the default and projects the exact `NativeApplicationErrorBody` above. `FlatSingleRecovery` is a checked compatibility dialect with this exact body:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlatNativeApplicationErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}
```

Absent recovery omits the member rather than writing null. This dialect is valid only when every projected error has an empty closed details schema and declares `RecoveryCardinality::AtMostOne`. An error may declare several contextual alternatives, but each runtime selection must contain zero or one. Static and bounded-runtime message policies both retain their RFC 0014 rendering unchanged. A selected callable recovery must map to a direct tool and serializes as that public tool name; a selected manual or host action serializes as its action code. Surface compilation computes that flattened string for every possible recovery on each command/error use and rejects any collision between distinct recovery keys, including an action code equal to a mapped tool name. Grouped recovery targets, unconstrained cardinality, multi-selection, non-empty details, or a non-injective flattened token set fail rather than silently lose contract information.

Callable recovery edges stored as catalog operation ids are translated through the active surface. A direct target becomes `{tool}`. A grouped target becomes `{tool, arguments: {selector: value}}`. Only the surface-owned selector is prefilled; command arguments still come from the error's validated details, current context, or a fresh observation. Registration fails when a projected application error names a recovery operation omitted from the surface. Declared manual or host actions remain non-callable action codes and summaries; they require no tool mapping and are never synthesized into calls.

Framework planning failures remain framework-owned tool errors and use their redacted compact text projection with `isError: true` and no `structuredContent`. The initial implementation provides no framework-to-application error relabeling; surface compatibility cannot make a planning or authorization failure look application-owned. Protocol-shape errors that prevent `tools/call` deserialization remain MCP protocol errors.

### Surface Identity And Projection

The command catalog hash continues to identify operation semantics. A separate surface hash covers:

- selected profile and native declarations;
- generated tool names, schemas, annotations, execution metadata, titles, and descriptions;
- generated server instructions plus RFC 0020's compiled task-delivery profile and capability objects;
- confirmation-presentation routing and generic presentation defaults in the host-neutral surface snapshot;
- application-error dialect and its validated projection;
- help projection and subset policy;
- any resource binding mode selected by RFC 0016.

`RuntimeIdentity` exposes both hashes. MCP tool lists, host-neutral surface snapshots, and contract fixtures compare the surface hash. Events and public plans retain catalog identity and add native surface identity when an active native adapter prepared the call. Surface-prepared invocation fingerprints bind the same serialized surface hash as specified under Dispatch.

```rust
pub struct RuntimeIdentity {
    // ...existing registry and process fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<ServingSurfaceIdentity>,
}
```

A bare `CommandRegistry::runtime_identity()` retains its existing catalog, run-schema, and help-schema hashes and leaves only the additive `surface` field absent because it has no adapter. Every constructed MCP adapter preserves those registry hashes and fills `surface`: the default effect-lane adapter uses the stable name `effect-lanes` and hashes its generated tool/profile snapshot, while a native adapter uses its declared surface name and hash.

The core `CommandCatalog` remains surface-neutral and retains one catalog hash when the same registry is served through several adapters. The adapter-owned catalog resource gains an `activeSurface` projection containing snapshot version, protocol version, name, surface hash, exposure policy, and route summaries without duplicating full command declarations. That projection is derived from the installed `NativeToolSurfaceSnapshot`; a bare registry resource has no active surface.

### Compatibility Fixture Provenance

Twill's acceptance suite does not depend on the VBL crate or a sibling checkout. The released-observation bundle lives at `crates/mcp-twill/tests/fixtures/vbl/v0.4.9/` and is pinned to VBL tag `v0.4.9`, peeled commit `f2bd478fa5506df7530b3fd60d7d0114f0ed3160`. Relative to v0.4.8, its `SERVER_INSTRUCTIONS` bytes are unchanged while screencast descriptions, bounded and divisible numeric inputs, paired dimensions, and durable output states and metrics change. The bundle records those observations without assigning their declaration ownership. It contains these files:

| File | Provenance | Contract Consumers |
| --- | --- | --- |
| `baseline-tools.json` | Programmatic export of `agent_surface_contract::baseline_catalog()` | RFCs 0014, 0015, and 0017 |
| `surface-catalog.json` | Exact JSON emitted by the archived `visible-browser-lab-mcp surface catalog` command, including server instructions and the 27 served tools | RFCs 0011, 0014, 0015, 0017, and 0019 |
| `vscode-package.json` | Exact `vscode-extension/package.json` blob from the pinned commit | RFCs 0011, 0018, and 0019 |
| `application-error-vectors.json` | Programmatic complete error-code/recovery inventories plus fixed-input serialization vectors for the public `BrowserToolError` constructors | RFCs 0014 and 0016 |
| `presentation-vectors.json` | Reviewed input/output vectors extracted from `vscode-extension/src/confirmation.ts` and `extension.ts` | RFCs 0018 and 0019 |
| `manifest.json` | Bundle identity and derivation ledger | Every consumer before reading another file |

References to v0.4.8 that remain in RFC 0014 and RFC 0018 are historical claims about what that earlier release did not expose, not fixture pins. Both RFCs consume the v0.4.9 bundle and validate its manifest before reading their listed files; no second v0.4.8 fixture or ambiguous shared-bootstrap identity exists.

`manifest.json` has `formatVersion: 1`; a `source` object containing repository `https://github.com/wycats/visible-browser-lab`, tag `v0.4.9`, and the peeled commit above; an `importer` object containing `version: 1` and the stable command template; and one `files` entry for every other directory member. Each entry contains `path`, lowercase hexadecimal `sha256`, `derivation` (`rustExport`, `sourceCopy`, or `reviewedVector`), and the exact pinned source paths whose bytes justify it. Entries and source paths are sorted lexicographically. The manifest does not hash itself. The directory contains no unlisted payload file, and paths are relative, normalized, and cannot escape the bundle root.

The evidence-only fixture bootstrap adds an unpublished workspace `xtask` package and Cargo alias, supplying `cargo xtask import-vbl-fixture --repository <local-git-repository> --ref v0.4.9` plus `--check`. Landing that data/tooling slice does not implement or advance RFC 0015; it establishes the provenance-checked input corpus consumed by earlier owner-local RFCs. The tool is orchestration around the archived application's exports, not a second catalog or host generator. The importer resolves and verifies the peeled ref, reads an archive of that commit rather than the caller's worktree, runs the archived server's existing `surface catalog` export, builds temporary helpers for the baseline/error inventories the release did not expose as commands, copies source-owned JSON bytes, canonicalizes programmatic JSON with the fixture's fixed formatter, and writes the manifest last. Canonical fixture JSON recursively sorts object keys, preserves array order, uses two-space indentation and LF, and ends with one newline; hashes cover those exact UTF-8 bytes. A `sourceCopy` instead preserves and hashes the exact Git blob bytes. Normal tests never invoke Git, Cargo against VBL, Node, or the network; they verify the checked-in manifest, exact directory inventory, and payload hashes before semantic assertions.

`reviewedVector` is intentionally distinct from a generated export. VBL v0.4.9 does not expose its complete presentation switch as a machine-readable contract, so `presentation-vectors.json` records the finite behavior table with pinned source-file hashes and explicit sample inputs. The importer verifies those source hashes and the reviewed payload hash but never claims to derive one from the other automatically. Refreshing a reviewed vector requires a visible fixture diff and reviewer confirmation of every changed case. The VBL port later executes the same vectors against generated TypeScript, turning the reviewed extraction into a downstream executable contract.

The released bundle records observations, not Twill declarations. VBL's per-command application-error footprints, resource bindings, and structured guidance were not machine-readable in v0.4.9 and therefore cannot be presented as exports. Twill's adoption declarations live in `crates/mcp-twill/tests/support/vbl.rs`, are reviewed as new framework input, and must account for the corresponding released bundle projections. RFC 0014's exhaustive ownership mapping separates application declarations from request-context, workspace, and planner vectors that become precise framework outcomes. This separates “what VBL shipped” from “how Twill now declares it” instead of laundering a migration judgment into provenance data.

The earlier `docs/adoption/visible-browser-lab/baseline/` capture remains historical adoption evidence from commit `29d47d0a8a7d28fc7e9f1f6db492b2253c52a160`. The implementation neither rewrites nor silently treats it as the release fixture. Compatibility assertions use the v0.4.9 bundle; token/progression analysis may cite the earlier capture with its own README provenance. Keeping both identities visible prevents an older measurement snapshot from being mistaken for the shipped release contract.

VBL owns the reverse integration gate: its Twill port regenerates the active native and host snapshots from the same application declarations and compares them with its current agent-surface contract. Updating a frozen released-observation bundle is therefore an explicit reviewed compatibility change, not a network fetch during Twill tests or an accidental consequence of a nearby VBL branch.

### Required Invariants

- Every native tool member resolves to one authoritative catalog operation.
- Every adapter and generated host consumes the same immutable versioned `NativeToolSurfaceSnapshot`; declarations cannot be served or generated before compilation against the registry, and Rust consumers use typed views rather than reparsing its canonical document.
- Every snapshot names the exact MCP protocol revision used to compile its tool and capability objects. Adapter finalization compares the compiled catalog hash with the runtime registry and rejects stale or cross-registry pairings before publication. The protocol ingress separately requires a legacy connection-negotiated or stateless per-request revision equal to the compiled target before routing any method or projecting any result; an observed revision can reject a request but can never select another surface.
- Native surface construction has one public `build(&registry, target)` method and requires the closed protocol target explicitly; compile-fail coverage proves no one-argument default-target overload or alternate `build_for` authority exists.
- Native plans represent operation-id origin directly and contain no synthesized raw command or parser tokens.
- Native plans expose the compiled surface name/hash that produced their public call shape; bare and existing command-template plans omit those additive fields.
- Every fingerprint contains the exact version-1 `invocation` object. Surface-prepared native fingerprints bind the same name/hash serialized on the plan; effect-lane calls bind their compiled default-surface identity; bare command-template and operation-id registry calls use the distinct `bareRegistry` marker.
- Native dispatch uses the ordinary binder, planner, authorization, context, resource, and handler pipeline.
- Every generated execution tool has one exact `TaskSupportSpec`; mixed support among direct/grouped/effect-lane members fails surface compilation.
- The immutable snapshot contains RFC 0020's one normalized compiled delivery profile and its exact protocol capability/tool projection. Profile, protocol revision, declaration, typed accessor, canonical document, and surface hash agree.
- Task lifecycle, storage, access, polling, result status, and cancellation are wholly RFC 0020-owned; this RFC never interprets runtime task controls or stores a task record.
- Native authorization and confirmation inspect the exact prepared operation later consumed by dispatch; adapters never approve one plan and rebuild another.
- Native confirmation distinguishes explicit denial, explicit cancellation, unavailable routing, and bridge infrastructure failure; every non-allow outcome drops prepared authority before binder, resolver, handler, or RFC 0020 execution-capsule creation. Deferred delivery receives the same validated non-allow tool result as ordinary delivery.
- Registry `PermissionPolicy` denial precedes the adapter authorizer and every confirmation route; embedding policy can narrow but never widen it.
- Every adapter uses its explicitly configured or default RFC 0003 authorizer; authorizer identity remains private and decisions cannot bypass the native confirmation route.
- Grouped input schemas always populate top-level properties and never attach a union to the whole command input.
- Repeated group fields have identical schemas; incompatible commands cannot be grouped.
- Grouped input and output fixtures receive source schemas with no dead `$defs`, deduplicate identical reachable same-name definitions, and reject a same-name canonical disagreement without renaming either member's public schema vocabulary.
- Tool annotations are truthful for every operation a tool may dispatch.
- `CommandSpec` and `CommandBuilder` expose the same task-support declaration; omission and explicit `Optional` normalize identically, while non-default values flow unchanged into the operation catalog.
- Direct tools project their operation's exact task support; native groups and existing effect-lane execution tools require/project one identical reachable-operation value, mixed task-support tools fail construction, and generated framework help is always `Forbidden`.
- Successful output schemas and application errors come from RFC 0014 result contracts; version-1 native mappings accept only success schemas proven object-only across every resolved root branch.
- Successful `structuredContent` conforms to the advertised success-only `outputSchema`; application and framework errors set `isError`, emit compact text content, and omit `structuredContent`.
- Application recovery operation ids translate to valid calls on the active surface.
- Application recovery actions remain explicitly non-callable and retain their declared codes and summaries.
- The CLI-shaped effect-lane profile remains the default and requires no native declarations.
- Effect-lane and bare-registry resource carriers remain argument-bound in version 1; RFC 0016 ambient carrier omission or optionality appears only in a compiled native surface and in generated hosts consuming that exact snapshot.
- Surface configuration, including confirmation capability, changes the surface hash without changing command semantics or the command catalog hash.
- Surface hash input is the canonical versioned snapshot document including catalog identity; private runtime objects and invocation values never enter it.
- Snapshot versioning covers compiler and runtime semantics as well as document shape: every behaviorally relevant compiler-owned fact is either present in the canonical document or forces a version increment, so one surface hash never names two serving contracts.
- A native invocation either receives bridge approval for its exact fingerprint or fails closed when confirmation is unavailable; dynamic authorizer behavior can never bypass this route.
- Application-error dialect changes participate in the surface hash and cannot discard declared details or recovery branches.
- A profile cannot silently omit a command unless subset exposure is declared and contract-tested.
- Complete and explicit-subset exposure form an exhaustive partition of catalog operations; registry growth cannot silently change a surface.
- Every caller-steering edge from an exposed command resolves to an exposed native call, and framework help never renders an omitted operation as callable.

### Implementation Phases

1. Land the pinned VBL observation bundle, manifest validator, and local-checkout importer as an evidence-only fixture bootstrap. This preparatory slice introduces no surface declaration, compiler, adapter, runtime API, or RFC lifecycle advancement; it exists so RFCs 0011, 0014, 0017, and 0018 can consume one provenance-checked source corpus before this RFC's public implementation.
2. Add authored surface declarations, compiled surfaces, the versioned canonical snapshot, validation, surface hashing, and adapter-owned catalog projection.
3. Add operation-id planning/dispatch and direct tool generation.
4. Add grouped input generation, selector dispatch, and discriminated output projection.
5. Add protocol-neutral `TaskSupportSpec` authoring plus direct/group/effect-lane homogeneity. The owner-local RFC 0015 slice has ordinary native delivery and introduces no provisional public task-delivery enum, snapshot accessor, store, or lifecycle API.
6. Derive annotations, help, server instructions, presentation defaults, and the host-neutral native snapshot; integrate RFC 0018's already-landed presentation evaluator and bridge types without changing their public API.
7. Add owner-local parity fixtures for operation routing, grouping, output projection, annotations, instructions, presentation, and the native snapshot. RFC 0016 later adds ambient carrier/input requiredness. RFC 0020 later adds `TaskDeliveryDecl`, `CompiledTaskDelivery`, protocol capabilities, and task-runtime finalization through the existing surface/server builders in one downstream integration slice. RFC 0019 consumes only the completed snapshot after both extensions land.

### Acceptance Tests

The evidence-only bootstrap is accepted by manifest, directory-inventory, payload-hash, importer reproduction, and no-network tests without claiming that RFC 0015 is implemented. Public RFC 0015 acceptance then lives in `crates/mcp-twill/tests/native_surfaces.rs` and consumes those versioned frozen fixtures. The owner-local implementation proves the surface compiler plus `TaskSupportSpec` authoring and homogeneity. Any bullet requiring `TaskDeliveryDecl`, `CompiledTaskDelivery`, a disabled/legacy/extension delivery profile, a task capability/runtime/lifecycle, RFC 0016 ambient binding, or RFC 0019 host generation is a downstream integration obligation in that owner's PR and updates the same fixtures. Those obligations do not authorize a provisional public delivery, binding, or host API in RFC 0015.

- The default adapter exposes the existing help and effect-lane tools unchanged.
- The guide's effect-lane constructor and native surface/finalizing-builder paths compile as written. Native construction requires explicit framework-help and confirmation-route choices; omission fails at surface compilation. `Omitted` adds no help tool, while `Tool { name }` exposes catalog help and rejects collisions with mapped application tools.
- The public surface declaration and task-support defaults compile under warning-denied Clippy through derived default markers. Fresh-builder defaults and one explicit default assignment normalize identically; repeated scalar/group assignments fail even when equal, while additive tool/member order remains caller-visible. `builder_from` accepts the completed declaration without permitting a later scalar call to overwrite it.
- Complete exposure and an empty `explicit_subset` declaration normalize identically, while literal-array, borrowed-slice, and owned-vector inputs to a non-empty `explicit_subset` produce the same sorted omission set and surface hash; repeated ids collapse and unknown ids fail compilation.
- Deserialized declarations with omitted versus explicit empty/default tools, exposure, application-error dialect, title, and description compile to identical normalized declarations, validation failures, canonical bytes, and surface hashes; framework-help and confirmation omission fails deserialization rather than selecting policy.
- Surface names accept the exact bounded lowercase-kebab grammar and reject empty, uppercase, underscore, option-like, control-bearing, or overlong spellings before snapshot construction.
- A direct `new_tab` mapping exposes the command's exact input schema, output schema, annotations, description, and application errors, then dispatches the mapped handler without a command string.
- Direct and grouped declarations with omitted title/description derive stable catalog-owned presentation; authored overrides replace only their declared slots, and the fluent convenience methods, `tool(NativeToolDecl)`, and serialized-declaration `builder_from` paths compile through the same validation and canonical snapshot.
- `with_surface` and `with_config_and_surface` succeed with default authorization, an `Unavailable` route, and `Disabled` or legacy-default task delivery while preserving surface-owned RFC 0016 binders. They reject a `Bridge` route or `TasksExtension` delivery because those require explicit adapter sidecars; the equivalent finalizing-builder calls succeed only after the matching bridge or complete task-runtime pair is supplied.
- Native plan serialization records `origin: operationId`, omits `rawCommand`, has no syntax tokens, and carries the compiled surface name/hash; effect-lane plans retain their existing raw command and default-origin wire shape with surface fields absent.
- Native success results contain matching object `structuredContent` and compact JSON text, set `isError: false`, and append resource links in deterministic order. Application and framework errors set `isError: true`, contain the exact compact JSON error body as text, and omit `structuredContent`, so the success-only `outputSchema` remains truthful.
- A released VBL application-error vector retains its exact validated native body and compact JSON text while the Twill projection omits VBL's old `structuredContent`; a strict client validating every present structured result against the advertised success schema accepts every emitted result.
- A grouped tool exposes a required selector, unioned top-level properties, common required fields, `additionalProperties: false`, and property-level unions from RFCs 0008 and 0017.
- Group dispatch enforces operation-specific required and unknown fields through the selected command planner.
- Group output projection coalesces identical member shapes into selector enums, emits a single object when all shapes agree, and uses `oneOf` only across genuinely distinct shape classes.
- Registration accepts direct root `$ref` and `oneOf` graphs only when every reachable branch is object-only, and rejects direct or grouped roots with any array, scalar, string, or null branch. A grouped member may resolve one root `$ref` to an object but rejects a member-authored root `oneOf`; only the surface compiler owns the generated group-level union. For groups it also rejects every pre-existing selector property except a required validation-semantic match for the generated singleton string/`const` schema; a matching value is validated before the adapter removes/restores it, so the surface never overwrites application-owned disagreement.
- Registration rejects duplicate names, one operation mapped more than once, selector collisions, dangling operations, incompatible repeated fields, unresolved schema references, accidental omission under `Complete`, mapped/omitted/dangling inconsistencies under `ExplicitSubset`, and any subset that breaks caller-steering closure. A capability consumer exposed with only self-dependent refresh providers and no exposed bootstrap provider fails that closure check.
- Surface-filtered framework help renders direct and grouped call shapes for exposed operations only; contract tests fail if omitted names appear as callable or any exposed recovery/help edge lacks a mapping.
- Group annotations equal truthful worst-case aggregation; direct annotations equal the selected command.
- An approved bridge call is bound to the planned invocation fingerprint and dispatches once. Deny returns ordinary permission denial; explicit cancellation returns `ConfirmationCanceled`/`Failed`; and a redacted bridge error returns `ConfirmationFailed`/`Failed`. None dispatches or creates an authority-bearing capsule. RFC 0020 proves the same outcomes under each deferred-delivery profile. With an unavailable route, allow still dispatches, deny remains denied, and a dynamically produced require-confirmation decision returns `ConfirmationUnavailable`/`Failed` without dispatch. Events and logs contain only operation, surface, and stable code.
- Surface compilation accepts a `Bridge` route without a concrete bridge and rejects no declaration for bridge identity. Adapter finalization then rejects a bridge route with zero or multiple sidecars and an unavailable route with any sidecar; two equivalent compiled surfaces may use distinct private bridge objects while retaining byte-identical snapshots and hashes.
- A native surface compiled from registry A installs on a byte-identical registry but is rejected against registry B after any command, schema, result, guidance, presentation, resource, or capability contract changes; no stale tool list is published before that check.
- Under `ExplicitSubset`, changing only one omitted operation leaves the generated MCP tool list byte-identical but changes the complete catalog hash, surface hash, every fingerprint prepared through that surface, and any downstream host hash or RFC 0020 storage key. The same selected call through bare-registry execution retains its fingerprint unless one of its direct inputs changes. This proves that whole-surface invalidation is intentional rather than an incidental tool-list hash.
- A failed surface or adapter finalization calls no binder/bridge/authorizer trait method, publishes no partially usable adapter, and emits no framework event. Owned sidecars run ordinary Rust `Drop`; destructor counters prove cleanup occurred, and a fixture sidecar tolerates never reaching publication. Construction-time panics follow the embedding unwind policy rather than becoming protocol errors.
- Dropping a call while its bridge future is pending drops the exact prepared invocation without binder/resolver/handler work. After allow, cancellation before realization remains effect-free, while cancellation after application work begins makes no rollback claim and leaves ownership/idempotency policy authoritative.
- A registry read-only policy denies a native write before either a permissive custom authorizer or bridge runs.
- One builder `authorizer` assignment changes runtime decisions for one adapter without changing catalog or surface hashes; omission retains `DefaultPermissionAuthorizer`, and a repeated assignment fails construction without invoking either object.
- Confirmation requests expose validated model-visible arguments, fingerprint, permission preview, and the preview's exact nested presentation through read-only accessors. Compile-fail coverage rejects external struct construction or mutation; structured preview and `presentation()` observe one stored `PreparedConfirmation`, while RFC 0018's compatibility display hint is a validated title/message projection of that value. No ambient identity, private resource reference, or raw request metadata enters the request, and denial or cancellation never dispatches.
- Direct and grouped tools select RFC 0018 declared confirmation copy from the mapped command or the active surface's generic fallback for an in-process bridge or host-neutral snapshot.
- Ordinary and RFC 0020 deferred calls preserve progress, workspaces, conversation identity, fingerprints, resources, and application result contracts through the same operation-id prepare/dispatch boundary.
- Equivalent low-level `CommandSpec::task_support` and mutable `CommandBuilder::task_support` declarations produce byte-identical operation catalogs and surfaces. Omitted and explicit `Optional` preserve the pre-RFC catalog bytes; `Forbidden` and `Required` change catalog/surface identity and project their exact metadata.
- Direct, native-grouped, and effect-lane `Forbidden`, `Optional`, and `Required` fixtures prove exact catalog and compiled-routing values; every pair of differing values within one generated tool is rejected at surface construction, and generated framework help is always `Forbidden`.
- RFC 0020 disabled, legacy, and extension fixtures consume the same compiled support values and prove exact protocol capability and lifecycle behavior in `crates/mcp-twill/tests/tasks.rs`.
- Protocol-bound snapshot fixtures include exact `protocolVersion`, normalized RFC 0020 delivery declaration, typed delivery accessor, capability/tool objects, and canonical bytes. Incomplete private runtime configuration rejects at adapter finalization. A legacy connection negotiated to another revision rejects before publication, while a stateless request carrying another revision rejects at ingress before method routing or result projection.
- A compile-time public-API fixture calls `run_operation_with_context(&str, JsonObject, InvocationContext)` directly and proves the version-1 method has exactly those three inputs rather than a replaceable request wrapper. Bare and adapter-native execution resolve a host-root-bearing invocation context through RFC 0009's single observation assembly. No public operation API accepts that raw observation alongside a pre-resolved set, and the legacy pre-resolved-plus-context family fails closed on the same dual-authority combination.
- Canonical direct and grouped application errors serialize the exact tagged native body: direct recoveries omit empty arguments, grouped recoveries contain only the compiled selector argument, actions retain code and summary, declaration order is preserved, and compact JSON text is byte-derived from the same structured body. No projection claims that missing target-command arguments are already supplied.
- Manual recovery actions such as `start_chrome` remain actions rather than dangling tool calls.
- `FlatSingleRecovery` reproduces VBL's `code/message/recovery` error object for empty-details, `AtMostOne` declarations whose possible callable targets are direct. Contextual alternatives remain legal only when every possible mapped tool name and action code is unique; an action/tool collision, incompatible declaration, or runtime multi-selection fails before an ambiguous string can project.
- A framework planning error remains framework-owned even when the application declares a similarly named error code; the surface performs no relabeling.
- Two profiles over one registry have the same command catalog hash and different surface hashes.
- MCP tools, adapter-owned catalog projection, and RFC 0019 generators consume byte-identical canonical snapshot data; attempting to install or generate from an uncompiled declaration is impossible through the public API.
- Native and effect-lane snapshot hash vectors prove the exact shared domain/version/length framing, RFC 8785 bytes, protocol-revision and catalog-hash inclusion, cross-domain separation for identical payloads, and exclusion of private bridge, authorizer, binder, and process objects. Changing only the compiled protocol revision changes the snapshot hash. Legacy negotiation to another revision rejects before publication; every mismatched stateless request rejects before method routing or result projection, and neither observation can select another compiled surface. A declaration number outside RFC 8785's exact I-JSON domain fails compilation, while RFCs 0014/0017 reject such schema literals earlier at registration; neither path rounds identity.
- A compiler-version fixture holds the normalized declaration constant while incrementing only the snapshot version and proves a new surface hash; frozen version-1 fixtures bind the exact document, typed accessors, wire projection, task-runtime facts, and dispatch semantics so a behavior change cannot retain the old identity.
- Compile-fail coverage proves `NativeToolSurfaceSnapshot`, `NativeSurfaceOperation`, `NativeSurfaceCall`, and `SurfacePresentationDefaults` have no serde or schema implementation and expose no public construction/mutation path. Typed-accessor fixtures match every canonical document member, while canonical-byte hash vectors match the read-only identity accessors.
- Exact fingerprint vectors cover command-template versus operation-id origin and bare-registry, effect-lane, and two native surface identities. The same operation and arguments across any two cases differ, and changing only generic surface presentation changes the surface-prepared fingerprint without changing catalog identity.
- A native preview/event is self-describing through its optional surface identity, and the serialized hash equals both the fingerprint input and active runtime identity; bare operation-id calls omit it and use their distinct marker.
- `ServingSurfaceIdentity`'s documented derives compile in an external public-API fixture. Invocation plans and runtime identities omit `surface` or serialize the complete validated `{name, hash}` object; missing/unknown nested fields fail deserialization, and no public constructor accepts an invalid name or noncanonical hash.
- Bare registry identity omits surface fields; effect-lane and native adapters populate stable names and hashes without changing existing registry hash fields.
- A VBL compatibility fixture projects exactly 27 production tools from 63 catalog operations, with the expected direct/group membership, property-schema composition, output discriminators, annotations, and instructions. RFC 0016 acceptance owns the final ambient carrier omission/optionality and resulting complete input-schema parity.
- The VBL fixture's input schemas contain populated top-level properties and no top-level `oneOf`; nested named and schema-constrained arguments retain property-level composition.
- RFC 0016's delegated acceptance completes the same fixture by proving every grouped required set equals the member-required intersection after ambient session binding; this later evidence does not make RFC 0016 a reverse dependency of the host-neutral grouping algorithm.
- The VBL fixture's grouped output branch counts equal the canonical member shape-class counts for every group, including single-object interaction/emulation results and coalesced memory-analysis variants.
- The VBL baseline has one root output union and the hybrid projection has exactly seven, all justified by direct or grouped output contracts; all hybrid input roots remain ordinary objects.
- Generated MCP tools, server instructions, RFC 0020 task-delivery capabilities, and the host-neutral native surface snapshot derive from the same declarations and remain byte-stable after canonicalization. Disabled, legacy, and extension profiles produce their exact distinct identity-bearing projections.
- The released-observation manifest verifies source/tag/peeled-commit, derivation kinds, exact directory membership, source paths, and payload checksums without network access; a tampered, extra, missing, partially refreshed, or path-escaping fixture fails before parity assertions. Importer `--check` reproduces every machine-generated/source-copy payload from an archived local checkout, while reviewed presentation-vector changes remain an explicit review surface.
- Released observations and Twill adoption declarations remain separate: tests construct the new catalog/resource/error/guidance model from `crates/mcp-twill/tests/support/vbl.rs` and compare its projections with the pinned bundle rather than deserializing old behavior as if it were already a Twill declaration.

## Drawbacks

Native surfaces reintroduce a larger tool list and public naming layer, the complexity RFC 0005 intentionally avoided for new applications. The separation into serving profiles keeps that cost opt-in but does not make it small.

Grouping weakens schema-time validation of operation-specific required fields because client compatibility requires a flat input object. Runtime planning restores correctness, but a client may allow a call the selected operation will reject.

Task support still affects surface compilation even though RFC 0020 owns delivery. Native groups and effect lanes must remain homogeneous, the snapshot must carry an exact typed delivery view, and protocol/capability objects remain part of surface identity. The split removes lifecycle complexity from this RFC without making deferred execution free.

Surface mappings are authored facts. A wrong operation id is caught, but a semantically poor grouping cannot be proven incorrect by the framework. Descriptions, `use_when`, parity fixtures, and agent evaluation remain necessary.

The native entrypoint requires one intentional Rust source migration beyond additive model fields: `InvocationPlan.raw_command` changes from `String` to `Option<String>` so an operation-id invocation cannot fabricate parser input. Direct plan consumers must handle absence. Its serialized default-origin shape remains compatible because command-template plans still carry the original string and native plans omit it explicitly. The mandatory fingerprint `invocation` member intentionally invalidates approvals and replay records issued by an earlier framework version; deployment must discard those records rather than attempting a cross-version translation.

## Rationale And Alternatives

**Replace effect lanes with native tools.** This is rejected. The CLI-shaped surface remains a better default for new catalogs, and stable RFC 0005 behavior should not change for applications that do not need compatibility projection.

**Let applications keep hand-written routers and schemas.** That preserves every legacy detail by preserving duplicate authority. The port succeeds only when Twill generates the public surface from the same command declarations it dispatches.

**Expose every command as one tool automatically.** This removes mapping configuration and turns VBL's 27-tool surface into 63 tools. Native projection must support deliberate grouping because tool count and established names are product contracts.

**Use top-level `oneOf` for grouped inputs.** It is more precise in JSON Schema and has failed in a real model-facing VS Code pipeline. Flat properties plus selected-command validation preserve both client usability and runtime correctness.

**Construct CLI command strings internally.** String synthesis would route native calls through the parser by creating syntax the caller never supplied. Operation-id dispatch reaches the same planner without introducing quoting, ordering, or template ambiguity.

**Return structured application errors under a success-only `outputSchema`.** Some SDKs skip output validation when `isError` is true, but MCP requires every supplied `structuredContent` value to conform to the advertised schema. Widening the schema to a success/error union would make the success contract less useful and would expose adapter error shapes as successful output alternatives. The native adapter instead preserves `isError` and the exact compact JSON error body in text while reserving `structuredContent` for schema-valid success.

## Prior Art

Web frameworks derive route schemas and OpenAPI operations from one handler registry while supporting aliases and grouped routers. GraphQL exposes one schema through many operation names. MCP itself treats tools as named schema-bearing operations. This RFC applies those projection patterns while retaining Twill's command catalog as the semantic authority.

MCP's legacy and extension task contracts demonstrate why serving-surface compilation must preserve protocol-neutral operation support without inventing one universal wire projection. RFC 0020 owns those specifications and their raw-wire evidence; this RFC supplies the coherent generated tool and immutable snapshot they consume.

VBL's hybrid catalog is direct evidence for the grouping and schema-dialect rules: 63 ungrouped operations, including 45 operations in grouped domains, compress into 27 tools; flat input properties survive VS Code typing, and discriminated outputs retain exact result shapes.

## Unresolved Questions

The surface/plan ownership model is decision-complete: one compiled surface per adapter, one shared invocation plan with explicit origin, public protocol-bound native surface identity, an adapter-owned in-flight confirmation bridge, and RFC 0020-owned protocol-versioned task delivery. The Rust names and serialized spellings in this body are the current Stage-0 proposal; any review-driven change must amend the managed RFC and its canonical vectors before Stage 1, and implementation may not select an alternate spelling. A revision may not describe `TaskSupportSpec` as one generic delivery state machine, omit the compiled protocol/delivery identity, or move runtime task records into this RFC's catalog or snapshot.

## Future Possibilities

Host adapters can consume the canonical surface snapshot to generate TypeScript types, editor manifests, or SDK clients without re-deriving tool semantics. A measurement harness could compare tool count and token cost across direct, grouped, and effect-lane profiles before an application chooses its public shape.

Dynamic tool-list changes could rebuild one native profile from a changing catalog and emit MCP list-changed notifications. A later compatibility layer could also serve versioned surfaces from one command catalog during a public API migration.

A future native transport could standardize deferred confirmation and replay across process boundaries. Alias declarations could arrive with explicit deprecation metadata and removal policy once a compatibility migration needs them.

A later measured host may add a flattened grouped-output superset under a new snapshot version. It must define required-field truthfulness, selector ownership, runtime shaping, and client evidence rather than weakening the initial discriminated contract speculatively.
