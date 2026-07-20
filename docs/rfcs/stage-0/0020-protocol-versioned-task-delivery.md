<!-- exo:20 ulid:01kxcwx83qmw082b6a6awwc70j -->

# RFC 0020: Protocol-Versioned Task Delivery

- Status: Draft
- Area: MCP adapter, deferred execution, task lifecycle, task storage, cancellation, authorization
- Target milestone: v0.4
- Depends on: RFC 0005 (effect-lane tool surface), RFC 0014 (application result contracts), RFC 0015 (catalog-derived native tool surfaces), RFC 0016 (ambient resource binding)

## Summary

This RFC introduces protocol-versioned task delivery for Twill commands. Commands continue to declare the protocol-neutral `TaskSupportSpec` owned by RFC 0015. An authored native surface separately chooses one exact task-delivery profile: ordinary-only delivery, the legacy MCP 2025-11-25 task dialect, or the official `io.modelcontextprotocol/tasks` extension. The compatibility effect-lane surface retains its shipped legacy 2025-11-25 profile and has no delivery declaration in version 1. Each surface compiler maps the same command declaration into its exact wire contract and rejects combinations it cannot represent truthfully.

The two task dialects share one prepared-invocation and execution-outcome pipeline, but they do not share a public state machine. Legacy delivery is selected by the client, retrieves terminal outcomes through `tasks/result`, maps `CallToolResult.isError: true` to `failed`, and treats successful cancellation as an atomic terminal transition. Extension delivery is selected by the server after the request advertises the extension, retrieves terminal outcomes inline through `tasks/get`, maps every `CallToolResult`—including `isError: true`—to `completed`, and treats cancellation as cooperative intent. Twill projects each dialect from the same validated application or framework outcome without relabeling its owner.

Task existence never grants dispatch authority. After the outer call envelope names a known public tool route, the selected profile's task controls are valid, the profile elects deferred delivery, and task access plus atomic store creation succeed, Twill creates the task before request-context normalization, grouped-operation selection, and command planning. The task initially owns only a private cancellable runner. Only the ordinary authorization boundary may move the exact prepared invocation into an authority-bearing execution capsule. Task records contain bounded protocol state and validated terminal outcomes; they never contain a replay token, conversation identity, workspace observation, ambient resource reference, raw argument, handler object, or execution capsule.

The initial implementation provides explicit scoped task stores, unguessable capability identifiers, optional embedding-derived authorization scopes, atomic state transitions and admission, a fixed 256-record bound per mounted runtime namespace, bounded redacted status text, and exact raw-wire fixtures for both dialects. Because every live runner first owns one stored working record, that bound also caps framework-owned concurrent runners. It does not claim process-restart execution resumption. A persistent external store may retain completed records, but Twill never reconstructs live authority from a serialized plan or task record.

## Motivation

RFC 0015 originally combined two independent concerns: compiling catalog operations into MCP tools and correcting the task lifecycle of the existing effect-lane adapter. That combination was plausible while MCP tasks had one experimental wire shape. The official Tasks Extension now replaces that shape with a deliberately incompatible design. Keeping both lifecycles inside the native-surface RFC would make a tool-projection proposal own task persistence, authorization scope, polling, cancellation races, and extension negotiation for every current and future serving surface.

The incompatibility affects application semantics rather than only field names. In the 2025-11-25 dialect, an application refusal represented by `CallToolResult { isError: true }` makes the task `failed`; the client later calls `tasks/result` to obtain that same tool result. In the extension, the identical tool result makes the task `completed` and is included in `tasks/get`; `failed` is reserved for JSON-RPC execution errors. The extension also removes the client `task` request member, makes task creation server-directed, adds `input_required` and `tasks/update`, and makes cancellation acknowledgement independent from the eventual terminal state.

Twill needs one internal execution model that preserves catalog, planning, authorization, binding, handler, and result-contract authority while allowing more than one protocol projection. It also needs to bind compiled surfaces to an exact delivery contract so neither a negotiated legacy connection nor a stateless request can cross-enable the other dialect, accept a legacy request field under the extension, or advertise extension capability while serving legacy methods.

Task access is a separate design pressure. Conversation identity and workspace roots are application context, not authorization principals. A task id may outlive the request that created it and, under a stateless transport, a later request may arrive on another connection. Twill therefore needs an explicit store boundary and an access policy derived from trusted transport context or possession of an unguessable identifier. Leaving those choices implicit in an adapter-local map would make extension support appear more durable and more isolated than it is.

## Guide-Level Explanation

A command declares whether deferred delivery is forbidden, optional, or required:

```rust
server.command("report generate", |command| {
    command
        .summary("Generate the project report")
        .task_support(TaskSupportSpec::Optional)
        .handle_result(generate_report);
});
```

That declaration is independent from MCP revision. The default effect-lane constructors retain legacy 2025-11-25 delivery as part of their compatibility contract; callers do not configure another profile through those constructors. A native surface chooses its delivery profile when it is compiled:

```rust
let surface = NativeToolSurface::builder("project-tools")
    .framework_help(FrameworkHelpProjection::Omitted)
    .confirmation_route(NativeConfirmationRoute::Unavailable)
    .task_delivery(TaskDeliveryDecl::tasks_extension(
        ExtensionOptionalPolicy::DeferredWhenAvailable,
        3_600_000,
    ))
    .direct("report_generate", "report generate")
    .build(&registry, McpProtocolTarget::V2026_07_28)?;
```

`Forbidden` always executes through the ordinary result path. `Required` means the operation cannot execute unless the active delivery profile can create a task. `Optional` follows the profile's selection rule. In the legacy profile the client chooses by supplying the legacy `task` member. In the extension profile the client advertises extension support and the server follows the compiled `ExtensionOptionalPolicy`: `Immediate` returns the ordinary result, while `DeferredWhenAvailable` creates a task. Request data cannot change that policy.

An extension surface also installs explicit storage and access policy:

```rust
let server = CliMcpServer::builder(registry)
    .surface(surface)
    .task_runtime(
        InMemoryTaskStore::server_instance(),
        TaskAccessPolicy::CapabilityId,
    )
    .build()?;
```

`CapabilityId` is appropriate when possession of a cryptographically random task id is the intended authority and the adapter does not list tasks. An authenticated shared transport instead installs `TaskAccessPolicy::Scoped(provider)`. The provider receives a private `TaskAccessContext` containing only embedding-authenticated transport extensions and returns a stable private scope digest. Conversation identity, workspace roots, task arguments, protocol request metadata, and other caller-asserted values never become an access scope.

The in-memory store is shared by clones of one finalized server instance, so a later request routed to another clone can poll the task. It is lost when that server process exits. The profile advertises no process-restart resumption claim. An embedding may provide another conforming store for retained records through the public store boundary, but live handler futures and authority-bearing execution capsules remain process-private.

Under extension delivery the caller does not add a task argument or select asynchronous execution. It advertises `io.modelcontextprotocol/tasks` in the request's protocol capability metadata. Twill may then return a result with `resultType: "task"`. The client polls `tasks/get`; a completed task includes the original `CallToolResult` inline. `tasks/cancel` acknowledges cancellation intent, while later polling reveals whether cancellation won the race or the operation completed another way.

Legacy delivery remains available only on a surface compiled for MCP 2025-11-25. A client uses that revision's `task` request member and retrieves the final outcome through `tasks/result`. Twill never enables this compatibility path because an extension key was present, and an extension surface never exposes `tasks/result`.

### How Agents Should Learn This

Task delivery creates no model-visible tool argument. Help and catalog projection describe an operation as `deferred: forbidden`, `deferred: optional`, or `deferred: required`; generated MCP metadata follows the active protocol profile. Agents should call the declared tool normally. The host and server negotiate whether the result is immediate or task-backed.

An agent should not invent a `task`, `taskId`, capability, polling, or cancellation argument. Hosts that expose task handles own polling and cancellation UX. A missing required extension is a client/host capability failure, not a request for the model to retry with a hidden field.

Application and framework steering remains identical after the terminal tool result is available. A task-backed application error retains its declared code, details, and recovery. Under the extension it is a successfully completed task whose result has `isError: true`; `completed` describes transport execution, not application success. Generated guidance must preserve that distinction.

Status messages remain bounded framework prose—`Task is running`, `Task completed`, `Task failed`, or `Task cancelled`—and never become an alternate diagnostic channel. Agents learn repair steps from the validated terminal tool result, not from task metadata.

## Reference-Level Explanation

### Declaration And Compilation

RFC 0015 owns `TaskSupportSpec` on `CommandSpec` and its projection into `OperationSpec`. This RFC adds one task-delivery choice to the authored native-surface declaration:

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TaskDeliveryDecl {
    #[default]
    Disabled,
    Legacy2025_11_25,
    TasksExtension {
        optional_policy: ExtensionOptionalPolicy,
        retention_ms: u64,
    },
}

impl TaskDeliveryDecl {
    pub fn tasks_extension(
        optional_policy: ExtensionOptionalPolicy,
        retention_ms: u64,
    ) -> Self;
    pub(crate) fn is_disabled(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionOptionalPolicy {
    Immediate,
    DeferredWhenAvailable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompiledTaskDelivery {
    Disabled,
    Legacy2025_11_25(CompiledLegacyTaskDelivery),
    TasksExtension(CompiledTasksExtensionDelivery),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledLegacyTaskDelivery { /* private capability fields */ }

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTasksExtensionDelivery { /* private capability fields */ }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasksExtensionCapability { /* fixed empty capability */ }

impl CompiledLegacyTaskDelivery {
    pub fn capability(&self) -> &rmcp::model::TasksCapability;
    pub fn runtime_contract_version(&self) -> u32;
    pub fn max_stored_task_record_bytes(&self) -> usize;
    pub fn max_stored_tasks(&self) -> usize;
}

impl CompiledTasksExtensionDelivery {
    pub fn extension_id(&self) -> &'static str;
    pub fn capability(&self) -> &TasksExtensionCapability;
    pub fn optional_policy(&self) -> ExtensionOptionalPolicy;
    pub fn retention_ms(&self) -> u64;
    pub fn runtime_contract_version(&self) -> u32;
    pub fn max_stored_task_record_bytes(&self) -> usize;
    pub fn max_stored_tasks(&self) -> usize;
}

pub struct NativeToolSurfaceDecl {
    // ...RFC 0015 fields...
    #[serde(default, skip_serializing_if = "TaskDeliveryDecl::is_disabled")]
    pub task_delivery: TaskDeliveryDecl,
}

impl NativeToolSurfaceBuilder {
    pub fn task_delivery(self, delivery: TaskDeliveryDecl) -> Self;
}

impl NativeToolSurfaceSnapshot {
    pub fn task_delivery(&self) -> &CompiledTaskDelivery;
}
```

`TaskDeliveryDecl::default()` is the derived `Disabled` variant, so omitted and explicit `Disabled` normalize identically, the declaration's `#[serde(default)]` contract is directly compilable, and the API satisfies the workspace's warning-denied Clippy policy without a manual derivable implementation. `TasksExtension.retention_ms` is required, must be an integer in `1..=604_800_000` (seven days), and becomes the actual record-retention duration. The value-style native surface builder provides `task_delivery` exactly once; repeated assignment is a construction error even when the value is equal. `builder_from` accepts a complete deserialized declaration and does not reopen the scalar choice. The effect-lane compatibility declaration remains private and contains the fixed `Legacy2025_11_25` projection after compilation; there is no public effect-lane `TaskDeliveryDecl`, protocol-target selector, or task-delivery builder in version 1.

The compiled delivery enum and its inner views have private fields and expose only read-only semantic accessors. They implement neither `Serialize`, `Deserialize`, nor `JsonSchema`; the native snapshot's canonical document remains their only JSON projection. The legacy view owns the exact typed 2025-11-25 capability objects. The extension view owns the fixed identifier, optional policy, retention, and exact typed capability projection supported by Twill's selected SDK/model layer.

The compiler validates the delivery profile against the exact MCP protocol revision owned by RFC 0015's snapshot. `Legacy2025_11_25` is valid only for `2025-11-25`. Version 1 of `TasksExtension` is valid only for the exact supported extension contract associated with the `2026-07-28` base protocol and extension identifier `io.modelcontextprotocol/tasks`. A later base or extension revision requires a new compiler fixture or snapshot version; lexical date comparison never implies compatibility. On the stateless target, the snapshot revision is server configuration rather than remembered connection state: every request must carry the matching base-protocol observation before this RFC's extension dispatcher becomes reachable.

The checked-in wire corpus at `crates/mcp-twill/tests/fixtures/mcp/tasks/` pins the external contracts separately from Twill's compiled projection. Its `manifest.json` has `formatVersion: 1` and records three exact sources: legacy MCP `2025-11-25` at commit `38c84e9f93ad191d9eb26d92b945d17bd0efcaf3`, the locked core `2026-07-28-RC` contract at commit `9d700ed62dcf86cb77475c9b81930611a9182f46`, and final SEP-2663 plus the Tasks Extension normative contract at commit `8966bea9c4f4e6d71060cc8284a539086e9e234f`. It also records the extension identifier. Each payload entry contains its relative path, lowercase SHA-256, source id, derivation (`sourceCopy` or `reviewedVector`), and exact source paths. The manifest does not hash itself, lists every other file exactly once, sorts sources, paths, and source paths lexicographically, and rejects absolute or escaping paths.

These fixtures are external observations, not Twill declarations or generated output. A `sourceCopy` preserves exact source bytes; a `reviewedVector` records an exact request or response case whose relationship to the pinned normative text requires human review. Normal tests verify directory membership and hashes before semantic assertions and never fetch the network. Refresh resolves archived local repositories at the authored commits and produces a visible manifest and fixture diff. The locked RC commit is the implementation authority rather than a reason to delay the work until the release date. When the final `2026-07-28` tag appears, the importer records its peeled commit as final-release provenance only after proving the frozen normative inputs byte-identical to the RC bundle. A byte-identical final tag changes provenance only; a normative delta returns to design review before any fixture, accepted behavior, delivery variant, snapshot version, or task runtime contract changes. The unversioned extension URL can therefore never silently redefine `TasksExtension` version 1.

### SDK Integration Boundary

The implementation baseline pinned by this RFC is rmcp 1.7.0. Its typed task models and negotiated server path remain the compatibility authority for MCP `2025-11-25`. That path cannot also represent the stateless `2026-07-28` core, the extension's polymorphic call result and inline terminal result, `tasks/update`, cooperative cancellation, or request-local discovery and capability rules. Twill therefore does not route a `V2026_07_28` surface through rmcp's legacy initialization or `ClientRequest` union and does not wait for a future SDK release.

A finalized `V2026_07_28` native adapter is consumed into Twill's explicit stateless serving path:

```rust
pub struct StatelessMcpService { /* private dispatcher and server */ }

#[derive(Clone)]
pub struct StatelessMcpHttpService { /* private shared service */ }

impl CliMcpServer {
    pub fn into_stateless_service(
        self,
    ) -> mcp_twill::Result<StatelessMcpService>;
}

impl StatelessMcpService {
    pub async fn serve_stdio<R, W>(
        self,
        reader: R,
        writer: W,
    ) -> mcp_twill::Result<()>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static;

    pub fn into_http_service(
        self,
    ) -> StatelessMcpHttpService;
}
```

The HTTP associated types are part of the accepted public contract rather than an implementation-selected body dialect:

```rust
impl tower_service::Service<http::Request<bytes::Bytes>>
    for StatelessMcpHttpService
{
    type Response = http::Response<bytes::Bytes>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<
                        Self::Response,
                        Self::Error,
                    >,
                > + Send
                + 'static,
        >,
    >;
}
```

`StatelessMcpHttpService` therefore receives and returns one complete byte body. Protocol, authorization, application, and handler failures are encoded in the HTTP/JSON-RPC response and never escape through Tower's `Error` channel. Its fields and raw protocol request/response types remain private. It validates the stateless base headers and JSON-RPC envelope before method dispatch, injects only authenticated transport extensions into the private request context, and never creates an rmcp session or accepts `initialize`. `serve_stdio` applies the same dispatcher to newline-delimited JSON-RPC without HTTP routing headers. Clones of the HTTP service share the one finalized server instance and its RFC 0020 runtime pair.

`CliMcpServer::into_stateless_service` succeeds only for a native surface compiled as `V2026_07_28`. Effect-lane and `V2025_11_25` adapters retain the existing rmcp `ServiceExt::serve` construction path and negotiated behavior. Attempting either serving family with the other compiled target fails before publication or request processing and invokes no task store, access provider, binder, authorizer, bridge, or handler. Existing callers that compiled a provisional `V2026_06_30` surface migrate to `V2026_07_28` and consume the finalized server through `into_stateless_service`; the legacy serving path remains source-compatible.

`mcp-twill` owns one crate-private `stateless_wire` module for the exact `2026-07-28` base envelopes and one crate-private `tasks_extension_wire` module containing closed serde types for the exact version-1 extension capability, `tools/call` result alternatives, `tasks/get`, `tasks/update`, and `tasks/cancel` parameters and results. The profile-selected stateless dispatcher recognizes every supported base and extension method before any legacy rmcp type becomes reachable, deserializes every known member once, and converts immediately to the existing protocol-neutral registry and runner/store operations. Missing members, duplicate known members, and wrong kinds fail where their selected method contract requires them to fail. Request parameter objects retain MCP's additive field rule: unknown sibling members are ignored rather than rejected. In particular, a legacy `task` member on a `2026-07-28` `tools/call` is ignored as an unknown field and can never opt that call into task delivery. The version-1 capability object itself remains exact and empty; an extension-owned member inside it is a version mismatch rather than an additive base-request field. A `Disabled` delivery profile uses the same stateless base dispatcher without making extension lifecycle types reachable; a legacy compiled target never reaches either stateless module.

The stateless adapters may hold one raw request or result value while parsing or writing at the private transport seam. Those values never enter the task store, runner, catalog, plans, public Twill API, or application callbacks. Frozen raw-wire fixtures are authoritative over stdio and HTTP in both directions. If a later pinned rmcp release provides validation-semantically identical stateless base and extension models plus profile-safe dispatch, Twill may replace either private module with adapters to those types without changing this RFC's public API, snapshot, or wire contract; the SDK release number is not protocol authority.

`Disabled` rejects a surface containing a `Required` operation. `Legacy2025_11_25` and `TasksExtension` require each generated tool to have one coherent support value. Direct tools copy their operation. Grouped and effect-lane tools require all reachable operations to agree. Framework-help tools are always `Forbidden`.

The canonical native-surface document gains the normalized member:

```json
{
  "taskDelivery": {
    "kind": "tasksExtension",
    "extension": "io.modelcontextprotocol/tasks",
    "runtimeContractVersion": 1,
    "maxStoredTaskRecordBytes": 1048576,
    "maxStoredTasks": 256,
    "optionalPolicy": "deferredWhenAvailable",
    "retentionMs": 3600000
  }
}
```

`taskDelivery` is omitted for `Disabled`. Both active delivery variants contain compiler-owned `runtimeContractVersion: 1`, `maxStoredTaskRecordBytes: 1048576`, and `maxStoredTasks: 256`; legacy contains those members alongside its exact legacy kind, while the extension form above also contains its fixed extension identifier, policy, and retention. None of the three fixed members is authored input. The snapshot's typed read-only task-delivery accessor and canonical member come from the same compiled value. The complete member, generated capability objects, tool metadata, protocol revision, and task-support values all participate in the surface hash. Private stores, access providers, task records, runners, and execution capsules do not.

The task runtime contract version freezes task-visible state transitions, oversized-outcome replacement, atomic capacity admission, the record-count bound, exclusive server-instance mount semantics, orphan classification, the public store/runner obligations, the emitted private codec version and bytes, and the complete accepted decoder-version set. Changing any of those semantics, either storage bound, which fitting fallback is committed, the writer codec, or the accepted codec matrix requires a new runtime contract version and therefore a new surface hash. A pure codec implementation refactor may retain the version only when it emits byte-identical records and accepts exactly the same byte language. This ensures that two binaries advertising the same surface hash can read every record either one can write and enforce the same admission boundary; a future codec migration, capacity revision, or dual-read bridge requires an explicit versioned surface transition rather than reinterpreting an old namespace.

Legacy compilation emits the 2025-11-25 server task capability and tool-level `execution.taskSupport` metadata defined by that revision. Extension compilation emits the extension capability through the extension discovery surface and does not emit the removed legacy server capability, request contract, or tool-level handshake. `TaskSupportSpec` remains in Twill's operation and compiled routing views even when the extension has no equivalent per-tool wire field.

RFC 0019 version-1 generated hosts remain ordinary-call surfaces. They accept `Forbidden` and `Optional` operations and reject `Required`, whose contract forbids ordinary delivery. After local and Rust route validation, the private host entrypoint explicitly selects this RFC's immediate ordinary path; it does not infer that choice from missing protocol metadata and does not manufacture a legacy task request, extension capability, task object, or polling API. Even when the finalized server has an active task runtime for its MCP surface, host dispatch performs no task access, store, runner, record-codec, or cancellation operation. The shared command execution produces its ordinary validated outcome, and RFC 0019 projects that outcome directly into `HostCallResultV1`. The authored delivery profile still participates in the native surface hash, host hash, and serving-surface contribution to invocation identity. A future task-aware generated host consumes this RFC's compiled delivery view rather than inferring behavior from tool JSON.

### Negotiation And Materialization

Task protocol controls are read only from the protocol request. They are not taken from RFC 0009's effective application-context map, `InvocationContext`, tool arguments, generated host envelopes, or embedding fallback metadata. The general MCP `progressToken` follows the same request-only boundary: Twill captures the token from the complete `tools/call` request separately from application context and never accepts a context-only fallback token.

Legacy negotiation follows the 2025-11-25 request field and session capability. An undecodable outer call, malformed task request, unknown public tool name, or support mismatch creates no task. A `Forbidden` tool rejects task augmentation and a `Required` tool rejects ordinary invocation with the revision's exact protocol error. A valid augmented request for a known public route creates a legacy task before request-context normalization or any grouped-operation selection.

Extension negotiation begins only after the base ingress has validated the request's `io.modelcontextprotocol/protocolVersion` and, for Streamable HTTP, its matching `MCP-Protocol-Version` header against the compiled `2026-07-28` target. It then reads the request's `io.modelcontextprotocol/clientCapabilities` metadata and validates its `extensions["io.modelcontextprotocol/tasks"]` member as the exact supported empty capability object. Unknown metadata siblings remain available to their owners. Missing or mismatched base-version observations and malformed extension capability are invalid parameters and create no task. The server advertises the same fixed empty extension capability through `server/discover`; discovery never establishes remembered client capability or changes the compiled target.

Capability declaration is request-local for the complete extension lifecycle. Every extension `tasks/get`, `tasks/update`, and `tasks/cancel` request must carry the same exact extension member in its own `_meta`; neither the creation request nor another request establishes remembered capability. A missing declaration returns `-32003` / `Missing required client capability` with the exact `requiredCapabilities.extensions["io.modelcontextprotocol/tasks"] = {}` data before access-scope derivation, task-id lookup, or store access. A malformed declaration is invalid parameters at the same pre-access boundary. An extension surface answers `tasks/result` with `-32601` / `Method not found`; no capability or legacy task record makes that removed method reachable.

After resolving a known public tool route and its homogeneous `TaskSupportSpec`, the extension materialization rule is:

| Support | Extension absent | Extension present |
| --- | --- | --- |
| `Forbidden` | ordinary result | ordinary result |
| `Optional` + `Immediate` | ordinary result | ordinary result |
| `Optional` + `DeferredWhenAvailable` | ordinary result | task result |
| `Required` | `-32003` missing required client capability | task result |

The extension capability is generic client support, not a task request. A caller cannot force an optional operation into a task. The server applies the compiled policy without consulting arguments, conversation identity, workspace observations, or untrusted metadata. A task result uses `resultType: "task"`; an ordinary 2026-07-28 result uses its standard `resultType: "complete"` shape.

`Optional` plus `Immediate` is intentionally distinct from `Forbidden`. `TaskSupportSpec` describes the operation across serving surfaces, while `ExtensionOptionalPolicy` describes this compiled extension surface. One catalog may therefore expose an optional operation through client-selected legacy tasks on one surface and immediate extension delivery on another. An extension surface may also need task support for required tools while keeping its optional tools immediate. Authors use `Forbidden` only when the operation itself must never run as a task.

### Extension Task Request Routing

For `tasks/get`, `tasks/update`, and `tasks/cancel` over Streamable HTTP, the client routes the request with exactly one `Mcp-Name` header whose decoded value equals the already-decoded `params.taskId`. The extension ingress adapter reads the `http::request::Parts` supplied through rmcp's private request extensions after parameter and request-local capability validation and before access-scope derivation or store access. A missing, repeated, non-text, or unequal header returns `-32602` / `Invalid task routing` and performs no access-provider or store operation. The standard `Mcp-Method` header remains owned by the Streamable HTTP transport. Stdio and other non-HTTP transports perform no `Mcp-Name` check.

The routing header is transport data, not task authority. Possession of a matching header never replaces RFC 0020's capability-id or verified-scope access check, and the raw value never enters framework events, diagnostics, status text, plans, previews, fingerprints, snapshots, help, or logs. A custom transport-scope provider is trusted embedding code and can inspect the same authenticated transport extensions it already receives; Twill does not republish them.

Once either profile elects task delivery, Twill obtains the private access scope and creates the access-bound revision-zero `working` record atomically before request-context normalization, grouped-operation selection, and planning. Access-provider or store-creation failure returns the static pre-task JSON-RPC error defined below and creates no record. A successful `create` is the creation response's linearization point and guarantees that an immediate `tasks/get` through the same access scope can observe that record or a later revision. `CreateTaskResult` always projects the exact revision-zero `working` seed retained by the adapter; it does not reload the store before responding. If the runner commits a terminal revision first, the creation response may therefore be an older valid snapshot while the first poll observes the terminal record. This deterministic rule avoids making the initial response depend on scheduler timing and preserves one transition history beginning at the profile's initial working state.

This RFC refines only the deferred timing of RFC 0013's normalize-once rule. Before task creation, the adapter performs the non-validating RFC 0009 per-key application-metadata merge, filters its protocol-control keys, and captures the known public route, raw tool arguments, `EffectiveApplicationMeta`, adapter observations, and any separately extracted request-owned progress token in a private `DeferredInvocationInput`. That carrier implements neither serialization nor JSON Schema, has redacted `Debug`, never enters `TaskStore`, and is owned only by the live runner. After record creation the runner consumes the application-metadata portion exactly once, drops the filtered wrapper, and retains only validated `InvocationContext` plus typed workspace observations. The progress token remains a separate live-runner protocol fact for the lifetime of that execution and is dropped with the runner; it never enters the effective application map, task record, codec, plan, fingerprint, event, framework log, or application context. Operation selection then determines the command, workspace planning resolves only that command's declared/path-bound requirements, and the raw arguments and typed observations are dropped as their validated plan facts are produced. Immediate delivery invokes the same extraction, filtering, normalization, and planning functions in the same order without constructing the deferred carrier. Poll, update, cancel, retained-record recovery, and task-store codecs never normalize again or recover context or progress authority from a task id or record.

If normalization fails, the runner commits the ordinary redacted framework tool outcome to the already-created task and drops every remaining input. If it succeeds, RFC 0013's existing guarantee still applies: planning, fingerprinting, authorization, replay validation, and dispatch receive the same normalized context value. The task-order refinement therefore changes when a public record becomes observable, not which metadata source wins, what a handler can see, or how an invocation fingerprint is calculated.

### Internal Execution Outcome

The ordinary invocation pipeline produces one validated transport outcome:

```rust
enum StoredExecutionOutcome {
    Tool(CallToolResult),
    JsonRpc(JsonRpcError),
}
```

The stored tool result is the same result ordinary delivery would return after RFC 0014 validation and surface shaping. A task adapter may merge only protocol-owned metadata required by its selected dialect. It cannot relabel an application error, convert a framework result into an application result, copy a source error, or replace the declared recovery graph.

Version 1 bounds the complete private storage record, not only the visible task object, at the public `MAX_STORED_TASK_RECORD_BYTES` constant shown in the store API below. The versioned record writer encodes into a bounded sink and never allocates a second over-limit byte buffer. The revision-zero working seed, every fixed status, cancellation outcome, and static task-infrastructure JSON-RPC error are construction-proven to fit. If an otherwise valid terminal `Tool` or `JsonRpc` outcome would make the complete next record exceed `MAX_STORED_TASK_RECORD_BYTES`, Twill discards that candidate before store mutation and instead commits the fixed JSON-RPC `-32603` / `Task execution failed` outcome at the same successor revision. That replacement is task infrastructure failure: both dialects project it as `failed`, no partial application result or resource reference is retained, and the prior working record remains authoritative until the fitting fallback compare-and-set succeeds. A store error during that commit follows the ordinary single-candidate reconciliation rule. The cap bounds framework retention after an application outcome exists; it cannot prevent a handler or SDK from allocating the source value before returning it.

The private task runner performs the shared lifecycle in the README: context normalization; resolution of the already-known public route and any group selector to one catalog operation; planning; workspace and resource-binding selection; wrong-lane handling; availability gates; dry run; permission policy; adapter authorization; confirmation; realization; handler execution; and result validation. A malformed recognized context or invalid/missing grouped selector is therefore a terminal outcome of an already observable task, while an unknown public tool name never created one. Before approval the runner owns no execution capsule. After approval it moves the exact prepared invocation and fingerprint into one private capsule; it never reconstructs them from the task record.

A pre-dispatch framework or application tool result is stored without creating a capsule. Scheduler, worker-join, or capsule infrastructure failure stores only the static JSON-RPC error `-32603` / `Task execution failed`. Task-store failures follow the reconciliation contract below and never become an application or command-framework outcome. Source errors remain private and have redacted `Display`, `Debug`, and error chains.

### Public State Projection

The internal record distinguishes `Working`, an optional extension-only `InputRequired` payload, and one terminal execution outcome or honored cancellation. Its wire projection is profile-specific.

Legacy 2025-11-25 projection follows these rules:

- every accepted task begins `working`;
- `Tool { isError: false }` becomes `completed`;
- `Tool { isError: true }` and `JsonRpc` become `failed`;
- honored explicit cancellation becomes `cancelled` and stores static JSON-RPC `-32000` / `Task cancelled`;
- `tasks/result` blocks while working and returns the stored tool result or JSON-RPC error after the terminal transition;
- creation and tool-valued `tasks/result` merge the authoritative `io.modelcontextprotocol/related-task` metadata without discarding unrelated result metadata;
- `tasks/get`, `tasks/cancel`, and status notifications do not carry related-task metadata;
- the profile never emits `input_required`, `tasks/update`, or extension `resultType`.

Tasks Extension projection follows these rules:

- `CreateTaskResult.resultType` is `task` and always carries the exact revision-zero `working` seed, even when a later revision commits before the response is written;
- every `Tool` outcome, including `isError: true`, becomes `completed` with that result inline;
- only `JsonRpc` becomes `failed`, with the error inline;
- an honored cancellation becomes `cancelled`, but cancellation acknowledgement does not promise that transition;
- `tasks/get` returns the complete status-specific shape with `resultType: "complete"`; there is no `tasks/result`;
- `tasks/update` and `tasks/cancel` return empty acknowledgements with `resultType: "complete"`; because version 1 originates no input requests, update ignores every response key for a known in-scope task and never changes its state;
- related-task metadata is not added;
- version 1 never originates `input_required`, but it implements the closed update acknowledgement path and preserves room for a later RFC that composes protocol multi-round-trip requests with Twill planning.
- version 1 does not advertise task-status subscriptions or emit `notifications/tasks`; clients poll `tasks/get`.

The four status messages are fixed: `Task is running`, `Task completed`, `Task failed`, and `Task cancelled`. `InputRequired`, when a later RFC enables it, requires a separately fixed bounded message. Status text never contains task arguments, operation ids, presentation strings, application codes, diagnostics, result bytes, identity, workspace, resource references, provider values, or source errors.

### Store And Access Authority

Task records use 256 bits from the operating-system CSPRNG encoded as exactly 64 lowercase hexadecimal characters. No counter, timestamp, operation id, fingerprint, conversation identity, workspace, argument, or scope digest contributes to the identifier.

The public runtime boundary is an object-safe asynchronous `TaskStore` plus explicit store scope metadata. It uses boxed return-position futures so a finalized adapter can retain an embedding-supplied trait object without making store operations synchronous:

```rust
pub type TaskStoreFuture<'a, T> =
    Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub const TASK_RUNTIME_CONTRACT_VERSION: u32 = 1;
pub const MAX_STORED_TASK_RECORD_BYTES: usize = 1_048_576;
pub const MAX_STORED_TASKS: usize = 256;

pub trait TaskStore: Send + Sync + 'static {
    fn scope(&self) -> TaskStoreScope;
    fn create(
        &self,
        key: TaskStorageKey,
        record: StoredTaskRecord,
    )
        -> TaskStoreFuture<'_, std::result::Result<TaskStoreCreate, TaskStoreError>>;
    fn get(&self, key: TaskStorageKey)
        -> TaskStoreFuture<'_, std::result::Result<Option<StoredTaskRecord>, TaskStoreError>>;
    fn compare_and_set(
        &self,
        key: TaskStorageKey,
        expected: TaskRevision,
        next: StoredTaskRecord,
    ) -> TaskStoreFuture<'_, std::result::Result<TaskStoreWrite, TaskStoreError>>;
    fn remove(&self, key: TaskStorageKey)
        -> TaskStoreFuture<'_, std::result::Result<TaskStoreRemoval, TaskStoreError>>;
}

pub struct InMemoryTaskStore { /* private server-instance state */ }

impl InMemoryTaskStore {
    pub fn connection() -> Arc<Self>;
    pub fn server_instance() -> Arc<Self>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreScope {
    Connection,
    ServerInstance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreCreate {
    Created,
    Occupied,
    CapacityExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreWrite {
    Written,
    Conflict,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStoreRemoval {
    Removed,
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskStorageKey([u8; 32]);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskRevision(u64);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskExpiration(u64);
pub struct StoredTaskRecord { /* private validated bytes and revision */ }
pub struct TaskRecordCodecError { /* private reason */ }
pub struct TaskStoreError { /* private source */ }

impl TaskStorageKey {
    pub fn as_bytes(&self) -> &[u8; 32];
}

impl TaskRevision {
    pub fn get(self) -> u64;
}

impl TaskExpiration {
    pub fn unix_millis(self) -> u64;
}

impl StoredTaskRecord {
    pub fn revision(&self) -> TaskRevision;
    pub fn expires_at(&self) -> Option<TaskExpiration>;
    pub fn storage_bytes(&self) -> &[u8];
    pub fn into_storage_bytes(self) -> Box<[u8]>;
    pub fn from_storage_bytes(
        bytes: impl Into<Box<[u8]>>,
    ) -> std::result::Result<Self, TaskRecordCodecError>;
}

impl TaskStoreError {
    pub fn new(
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self;
}
```

`InMemoryTaskStore::connection()` returns a fresh legacy-session namespace with `TaskStoreScope::Connection`. Connection means one negotiated legacy MCP session, not one request or one Streamable HTTP response stream. Handler clones and request routers belonging to that session share the returned `Arc`; a separately negotiated session uses a separately constructed store. A connection store makes no reconnect or process-restart retention promise, cannot be mounted on `TasksExtension`, and becomes unreachable when its owning legacy session ends. Reusing the same connection store across independently negotiated sessions violates the store contract. This is the exact private default installed by the legacy compatibility constructors.

`InMemoryTaskStore::server_instance()` returns a fresh server-instance namespace with `TaskStoreScope::ServerInstance`. Cloning its returned `Arc` shares that namespace across one finalized server's clones and request routers. Calling the constructor again creates a disjoint namespace that may be mounted concurrently because it cannot observe the first store's task ids or records. The two concrete constructors return the same `Arc<InMemoryTaskStore>` type, which coerces directly to the `Arc<dyn TaskStore>` accepted by `CliMcpServerBuilder::task_runtime`; no second wrapping constructor or public mutation API exists.

`TaskStore::scope` is pure deterministic construction metadata. It performs no I/O, consults no changing backend state, and returns the same value for the store object's lifetime. Finalization may call it more than once, including while accumulating other validation failures. No rejected build invokes `create`, `get`, `compare_and_set`, `remove`, or a `TaskAccessScopeProvider`. `TaskStoreScope` is a closed runtime enum with no serde or JSON Schema representation.

The legacy compatibility constructors retain a `Connection` in-memory store, and an explicitly authored legacy runtime pair must report the same scope. `TasksExtension` requires `ServerInstance` scope and a store shared by every clone/request router for that finalized adapter. Neither profile accepts the other's scope in version 1: changing visibility from one negotiated legacy session to a server instance is a delivery-contract change, not an interchangeable private optimization. `ServerInstance` is also an exclusivity assertion: one live finalized server instance owns that logical persistence namespace, and its clones and request routers are part of the same owner. A conforming persistent-store adapter acquires any backend lease or deployment lock before exposing a `ServerInstance` store and holds it until that mounted store is dropped. It may remount the same namespace after the earlier owner exits, but it cannot expose the namespace concurrently to separately finalized live servers. Cross-process active/active runners require a future lease-and-worker-ownership protocol; returning `ServerInstance` without exclusive live ownership violates the store contract rather than weakening orphan detection. `InMemoryTaskStore::server_instance()` is the explicit initial implementation. Until a finite deadline passes, either store scope guarantees read-after-create visibility, monotonic revisions, atomic insert-if-absent creation, atomic compare-and-set, and atomic removal of task plus terminal outcome. A zero-TTL legacy record may expire immediately after successful insertion; the retained creation response remains valid evidence that creation occurred, while every later task operation may already return `Unknown task`. Neither in-memory scope makes a process-restart promise.

Twill derives `TaskStorageKey` rather than passing either the raw task id or an access scope through the public store API. The key is SHA-256 over `UTF8("io.github.wycats.mcp-twill/task-storage-key") || 0x00 || surface_hash || access_tag || raw_task_id`, followed by the private 32-byte scope digest only for scoped access. `surface_hash` is the 32 bytes decoded from the active compiled surface's canonical lowercase 64-character hash; that identity already binds its protocol revision and task-delivery profile. `access_tag` is `0x00` for `CapabilityId` and `0x01` for `Scoped`; `raw_task_id` is the 32 bytes decoded from the public id. All members after the domain are therefore fixed-width. A capability lookup derives the same key from the presented id only on the same compiled surface, while a scoped lookup additionally derives a different key for every unequal verified scope. `TaskStorageKey` is an opaque database key rather than an authorization credential. It has no public constructor, serde, JSON Schema, or unredacted `Debug`; `as_bytes` exists only so an external store can index its backend.

The store object defines the persistence namespace. Reusing the same `Arc<dyn TaskStore>` across request routers and clones of one finalized server intentionally joins their storage. Installing it into a second concurrently live finalized server is outside the `ServerInstance` contract even when the compiled surface hash and access mode agree. A different native/effect-lane surface, catalog revision, protocol target, or delivery profile derives another key even for the same public task id and verified scope, so its lookup remains indistinguishable from absence and never reaches record decoding. A persistent store can make a terminal record visible after a sequential process restart only when the embedding remounts that same store namespace after the earlier owner has exited and restores the same compiled surface identity and access policy. A deployment whose surface hash changes treats earlier task ids as unknown; it does not reinterpret or migrate their private record bytes.

`create` atomically checks the key and mounted-namespace capacity with insertion. An existing key returns `Occupied` without changing that record. Otherwise, a namespace already retaining `MAX_STORED_TASKS` records returns `CapacityExceeded` without mutation; working, terminal, cancelled, null-TTL, and reconciliation-pending records all consume one slot until atomic removal. Only a remaining slot may return `Created`, and then only after the inserted record is durable according to the store's advertised lifetime and an immediately following `get` through the same store and key can observe that record or a later revision. This order keeps a random-id collision distinct from capacity and makes concurrent claims for the last slot linearizable. An adapter over an eventually consistent backend waits for read visibility inside `create`; it may not acknowledge an invisible insertion and ask Twill or the client to poll speculatively. Twill generates a fresh task id and retries at most eight times only after `Occupied`; `CapacityExceeded` fails immediately with static JSON-RPC `-32603` / `Task creation failed`. Eight occupied ids use that same failure without replacing or exposing any existing record. The implementation never intentionally reuses an id after removal. `compare_and_set` writes only when the record still has the expected revision and `next.revision()` is its exact successor. `Conflict` and `Missing` do not mutate storage. `remove` atomically deletes the complete record and frees one capacity slot; it distinguishes a winning `Removed` from an already-absent `Missing`. `TaskStoreError` is reserved for store infrastructure failure rather than ordinary occupancy, capacity, or races. Every `Err(TaskStoreError)` guarantees that the logical operation did not mutate storage; an adapter over a backend with ambiguous commit acknowledgement must reconcile that ambiguity before returning from the trait method.

A create infrastructure error, exhausted collision retry, or `CapacityExceeded` returns static JSON-RPC `-32603` / `Task creation failed` and no task; the response never distinguishes those causes or reports the bound/current count. Before expiration, a store error while serving `tasks/get`, `tasks/update`, `tasks/cancel`, or legacy `tasks/result` returns static JSON-RPC `-32603` / `Task storage failed` and leaves the record unchanged. Expiration cleanup is the one public-mapping exception: once Twill has decoded a passed deadline, removal failure cannot make that expired capability observably exist and follows the unknown-task retry rule below. If terminal execution has already produced a validated `StoredExecutionOutcome` but its compare-and-set returns a store error, the live runner drops every execution capsule, retains only that private terminal candidate, and schedules reconciliation against the same working revision. A later get, result, or update observes the committed candidate, a competing terminal winner, or `Task storage failed`; cancellation races its profile-specific transition against the candidate through the same compare-and-set rule. After storage recovers, one transition wins atomically. A conflict reloads the record and either accepts an already-terminal winner or retries only while the authoritative record remains working, without overwriting cancellation. `Missing` discards the candidate and never recreates an expired or removed task. The candidate is never projected before a successful commit and is discarded with the server instance.

The semantic task record remains crate-private. `StoredTaskRecord` is the public persistence capsule: Twill constructs every new and successor value, while an external store may reconstruct only a previously emitted value through `from_storage_bytes`. The decoder rejects an input longer than `MAX_STORED_TASK_RECORD_BYTES` before allocating or parsing semantic state, then validates the codec version, all field bounds, task id, compiled surface hash, access mode and scope digest, timestamps, revision, status/outcome agreement, and absence of trailing data. It caches the read-only revision and optional expiration. There is no unchecked constructor and no accessor for semantic task fields. `expires_at()` exposes only the immutable storage-retention deadline as Unix epoch milliseconds: extension records always return `Some`, legacy records return `None` only when their accepted `ttl` was null, and no other task timestamp is public. A conforming persistent store uses that value for its backend TTL/index so physical cleanup does not depend on a live Twill runner; it never parses the opaque bytes to recover the deadline. After every store read, Twill decodes the private record, derives its storage key again, and verifies the requested key plus immutable task id, compiled surface hash, access mode, scope, creation time, expiration, and delivery-profile fields before treating it as authoritative. A malformed, over-bound, or mis-keyed record produces only `Task storage failed` and never reaches lifecycle logic.

`StoredTaskRecord::storage_bytes` is the one versioned private-storage encoding. It is an opaque Twill storage format rather than an embedding interoperability format: stores persist the bytes without parsing or synthesizing them, and frozen version-1 vectors bind every accepted field and rejection rule. The leading codec version remains a private parsing discriminator, while `TASK_RUNTIME_CONTRACT_VERSION` binds the exact writer and decoder compatibility matrix into compiled task behavior and surface identity. The codec may contain the raw task id, scope digest, and validated terminal outcome because the embedding store is a trusted private boundary. Those bytes are never a public MCP, catalog, snapshot, event, diagnostic, preview, help, or log projection. `TaskStorageKey`, `TaskRevision`, `TaskExpiration`, `StoredTaskRecord`, and both public error types have private fields, redacted `Debug`, and no serde or JSON Schema implementation. `TaskStoreError::new` retains an erased backend source privately, but its `Display` and `Debug` are fixed and its `std::error::Error::source` returns `None`; `TaskRecordCodecError` likewise exposes no rejected bytes or reason. Twill never forwards either source to a protocol response, framework event, application diagnostic sink, help, preview, or log.

An external store may retain terminal records across sequential restarts. It may not serialize the private runner, handler, authorizer, confirmation bridge, prepared invocation, execution capsule, ambient reference, or raw context. Under the exclusive-mount rule, a later get/update/cancel can load a retained `working` record without a matching live runner only after the server instance that could have owned that runner has exited. The new owner atomically transitions that orphan to the static JSON-RPC `Task execution failed` outcome before responding; it never races another conforming live owner, resumes, or replays application work from the record. A future RFC may add resumable external-job handles and active/active worker leases with their own authority model.

The adapter builder requires one explicit access mode for extension delivery:

```rust
pub enum TaskAccessPolicy {
    CapabilityId,
    Scoped(Arc<dyn TaskAccessScopeProvider>),
}

pub struct TaskAccessScope { /* private digest */ }
pub struct TaskAccessScopeError { /* private source */ }
pub struct TaskAccessContext<'a> { /* private transport extensions */ }

impl<'a> TaskAccessContext<'a> {
    pub fn transport_extensions(&self) -> &'a rmcp::Extensions;
}

impl TaskAccessScope {
    pub fn new(
        principal: impl AsRef<[u8]>,
    ) -> std::result::Result<Self, TaskAccessScopeError>;
}

impl TaskAccessScopeError {
    pub fn new(
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self;
}

pub trait TaskAccessScopeProvider: Send + Sync + 'static {
    fn scope(
        &self,
        context: TaskAccessContext<'_>,
    ) -> std::result::Result<TaskAccessScope, TaskAccessScopeError>;
}

impl CliMcpServerBuilder {
    pub fn task_runtime(
        self,
        store: Arc<dyn TaskStore>,
        access: TaskAccessPolicy,
    ) -> Self;
}
```

`CapabilityId` uses possession of the unguessable id and is valid only while `tasks/list` is absent. `Scoped` derives a stable non-empty digest from embedding-authenticated transport extensions. `TaskAccessContext` has no public constructor; the finalized adapter creates it only from its private transport context after the embedding has authenticated or otherwise verified those extensions. Protocol request `_meta`, RFC 0009 effective context, `InvocationContext`, arguments, and generated host envelopes cannot populate it. Direct conformance tests use an explicit test-only host injection path rather than a protocol field. `TaskAccessScope::new` accepts 1 through 4096 principal bytes and stores SHA-256 over `UTF8("io.github.wycats.mcp-twill/task-access-scope") || 0x00 || U64_BE(principal_byte_length) || principal_bytes`; empty or oversized input returns the same redacted `TaskAccessScopeError`, and comparisons are constant-time. `TaskAccessScopeError::new` lets an external provider map its authentication backend failure into that closed refusal without exposing the source. `TaskAccessContext`, `TaskAccessScope`, and its error have fixed redacted `Display`/`Debug`, implement neither serde nor JSON Schema, and return `None` from `std::error::Error::source`. The raw principal and digest never serialize into the task object, result, snapshot, event, help, diagnostic, or log; only the private versioned task-store codec may retain the digest required for a later lookup.

`task_runtime` assigns one complete private runtime pair and may be called once. There are no individual store or access builder methods and therefore no authorable half-configuration. Existing legacy compatibility constructors and RFC 0015's native convenience constructors over a legacy profile supply `InMemoryTaskStore::connection()` plus `CapabilityId` internally. A fresh finalizing builder targeting legacy delivery receives that same semantic default unless one explicit complete pair was authored; the default does not count as an assignment and is replaced only as a pair, and that explicit store must report `Connection`. `TasksExtension` has no runtime default and requires exactly one explicit pair whose store reports `ServerInstance` scope, so RFC 0015's `with_surface` and `with_config_and_surface` convenience paths reject that profile and direct authors to the finalizing builder. `Disabled` rejects an authored pair. A missing, repeated, scope-incompatible, or profile-inapplicable pair rejects before tool publication. Finalization reads only the store's pure `scope()` metadata; it invokes no store operation or scope provider. Sidecar identity never enters catalog or surface hashes.

Creation-provider failure returns static JSON-RPC `-32603` / `Task access scope unavailable` and creates no task. Lookup-provider failure, surface or scope mismatch, expired id, malformed id, and unknown id return the indistinguishable `-32602` / `Unknown task`. Pre-expiration store failures use only `Task creation failed` or `Task storage failed` according to the boundary above and expose no backend source; failure to remove a record already proven expired retains `Unknown task` and retries cleanup privately.

Neither profile advertises `tasks/list` in version 1. Listing requires a separately reviewed authorization and pagination contract; capability ids alone cannot safely define a caller's task collection.

### Retention And Polling

Legacy `task.ttl` validation and projection remain revision-specific: the accepted value is a finite non-negative integer representable as `u64` milliseconds, omission means `null`, and the legacy task object uses `ttl` plus `pollInterval`. The compatibility profile uses a fixed 100-millisecond poll interval.

Extension task objects use `ttlMs` and `pollIntervalMs`. A request does not author those values. Version 1 uses the compiled surface's required 1-through-604,800,000-millisecond retention and a fixed 100-millisecond suggested poll interval. The normalized retention declaration participates in surface identity; elapsed runtime timestamps do not.

Expiration is measured from creation. A finite deadline is checked addition of the accepted TTL to creation time; overflow or a value outside the Unix-epoch-through-RFC-3339-year-9999 domain rejects task creation before store insertion. Every timestamp is UTC RFC 3339 with millisecond precision. `lastUpdatedAt` changes only after a successful state transition.

The active server schedules removal for every finite deadline it creates. A persistent external store additionally indexes `StoredTaskRecord::expires_at()` and enforces its backend TTL across process loss; this is the only public retention metadata it receives. At every task operation, Twill independently compares the decoded immutable deadline with its controllable clock before interpreting status or outcome. Once expired, the task is immediately and permanently `Unknown task` to that adapter. Twill requests atomic removal of the complete record; a transient removal error retains only stale private storage, is retried without recreating a task, and never changes the public unknown-task response into `Task storage failed`. A later restart or lookup applies the deadline check again, so an expired record can never become visible merely because cleanup was delayed. A null legacy TTL has no deadline and remains subject only to explicit removal or store lifetime. A controllable clock owns all conformance tests.

### Cancellation And Races

The private runner and execution capsule expose a cancellation token. Cancellation before realization drops prepared authority and performs no binder, resolver, or handler work. Cancellation after application work begins is best-effort; resource ownership, cleanup, and idempotency declarations govern surviving effects. Task state never claims rollback.

Legacy cancellation races through the one atomic terminal transition. A winning request records `cancelled`, signals the private work, stores the static cancellation error, and discards any later worker outcome. A terminal task rejects cancellation with `-32602` and remains unchanged.

Extension cancellation is an idempotent acknowledgement of intent for every known in-scope task. The adapter records the request and signals private work, then returns an empty complete acknowledgement. The observable task may remain working and may later become completed, failed, or cancelled. If the worker confirms cancellation before another terminal result commits, compare-and-set records `cancelled`; otherwise the winning result remains authoritative. Acknowledging cancellation never overwrites a terminal record.

Dropping an ordinary outer request future drops its prepared invocation. RFC 0019 generated-host cancellation is one such ordinary-request boundary: it never maps to `tasks/cancel`, invokes a task sidecar, or leaves retained task state. After either task profile's atomic creation point, dropping the task-creating request future alone does not cancel the runner. An extension task has independent server-instance lifetime, so transport disconnect likewise does not imply cancellation. A legacy task remains owned by its negotiated connection scope: losing one response stream does nothing, while ending that complete session drops its records and signals any framework-owned runner. Application work that already accepted dispatch remains subject to the ordinary best-effort cancellation and no-rollback boundary.

### Diagnostics, Events, And Disclosure

Protocol-shape and capability failures use JSON-RPC errors before task creation. Once a task exists, application and framework outcomes retain their ordinary validated bodies and owning status. Task infrastructure and access failures use only the static messages defined here.

Version 1 adds no task-lifecycle field to public `FrameworkEvent`. Deferred execution emits the same ordinary command event, when the deferred command is actually executed, without a task id, task digest, delivery profile, transition, status prose, access scope, arguments, context, result body, store error, or execution capsule. Pre-command task protocol and infrastructure transitions emit no framework event. A later telemetry RFC may add explicitly named bounded correlation fields with its own migration and disclosure contract; this implementation does not reserve or synthesize them.

Plans, previews, invocation fingerprints, help, and catalog data contain `TaskSupportSpec` and compiled serving identity where already declared; they never contain a runtime task id or state. Task selection does not change the invocation fingerprint because the same approved operation and arguments retain one execution identity across immediate and deferred delivery. The active surface hash already binds the delivery profile and optional policy.

### Required Invariants

- `TaskSupportSpec` is catalog-owned; task wire negotiation and lifecycle are profile-owned.
- A compiled native surface selects exactly one of disabled, legacy 2025-11-25, or Tasks Extension delivery; the effect-lane compatibility compiler selects legacy 2025-11-25 and exposes no authored delivery choice.
- Legacy request members, capabilities, methods, metadata, status rules, and extension equivalents never cross-enable one another.
- The 2026-07-28 base-protocol observation is validated per request before extension dispatch; `server/discover`, prior requests, and request extension members cannot establish or replace it.
- A legacy `task` member on a 2026-07-28 `tools/call` is ignored as an additive unknown field and never affects extension materialization.
- Extension capability and progress token are read from protocol request metadata only; effective private application context and tool arguments cannot assert either one.
- Every extension task operation independently declares the extension capability; missing or malformed declaration fails before task access or storage.
- Streamable HTTP task operations carry one `Mcp-Name` equal to the task id and validate it before task access; the header is routing data rather than authority.
- An undecodable outer call, malformed task control, unknown public tool name, support mismatch, missing required extension capability, or task-access/store creation failure creates no task. Malformed recognized invocation context and invalid grouped-operation selection occur after task creation and terminate that task without a capsule.
- Task existence grants no dispatch authority; only ordinary authorization may create an execution capsule for the exact prepared invocation.
- Immediate and task delivery consume the same validated application/framework outcome without relabeling ownership.
- Legacy `isError: true` projects as failed; extension `isError: true` projects as completed with the tool result inline.
- Extension cancellation acknowledges intent and never guarantees a cancelled terminal state.
- Task records and stores contain no replay token, prepared invocation, execution capsule, raw context, raw argument, ambient reference, or handler object.
- The public store boundary consists only of opaque derived storage keys, revisions, an optional immutable retention deadline, and validated versioned record bytes; semantic task records and unchecked key/record construction remain framework-private.
- Every private version-1 task record is at most `MAX_STORED_TASK_RECORD_BYTES` (1,048,576 bytes). An oversized terminal candidate is discarded and replaced atomically by the fitting static task-execution failure; no partial result reaches storage or a task response.
- Every mounted task runtime retains at most `MAX_STORED_TASKS` (256) records. Atomic create distinguishes key occupancy from capacity, no rejected admission mutates storage, and removal frees exactly one slot; because a runner exists only after its working record, framework-owned live runners are bounded by the same number.
- Every active compiled task-delivery member exposes and hashes `TASK_RUNTIME_CONTRACT_VERSION`, `MAX_STORED_TASK_RECORD_BYTES`, and `MAX_STORED_TASKS`. A task-visible semantic, bound, writer-codec, or accepted-decoder-set change increments the runtime contract version; retaining it requires byte-identical writes, the identical accepted byte language, and the same admission behavior.
- Deferred execution retains raw arguments, protocol-control-filtered effective application metadata, typed-but-unresolved workspace observations, and any separately captured request progress token only in one private live `DeferredInvocationInput`; normalization runs once after record creation, each planning stage drops the raw/typed input it consumes, and no task protocol or retained-record path can rerun it or reconstruct progress authority.
- Task creation is atomic insert-if-absent and read-after-create visible before acknowledgement; random-id collision never overwrites or reveals an existing record.
- Successful store creation is the creation response's linearization point. Both task profiles return the retained revision-zero `working` seed without a post-create reload; polls observe the authoritative current revision.
- A store error means no logical mutation. Creation failure produces no task; later storage failure leaves the last committed record authoritative, retains at most one private terminal candidate in the live runner, and reconciles it without overwriting a competing terminal transition.
- `TaskStoreScope::Connection` means one negotiated legacy MCP session, including its internal handler clones and replacement response streams. It is neither reusable across independently negotiated sessions nor valid for `TasksExtension`; ending its owner makes every record unreachable and signals live framework work without claiming rollback of application effects.
- `TaskStoreScope::ServerInstance` means one exclusive live owner of a persistence namespace. Finalized-server clones and request routers share that owner; sequential remount after owner exit may recover retained records, while concurrently active finalized servers require disjoint namespaces or a later worker-lease protocol.
- Conversation identity, workspace roots, and arguments are never task access scopes.
- Task storage keys bind the exact compiled serving-surface hash before access mode, task id, or private scope. A task from another surface, catalog revision, protocol target, or delivery profile is indistinguishable from an absent task and is never decoded through the active adapter.
- Scoped access receives only the embedding-owned `TaskAccessContext`; protocol metadata and effective invocation context cannot populate it.
- Unknown, expired, malformed, cross-scope, and lookup-provider-failed task ids are publicly indistinguishable.
- Expiration authority is the immutable decoded record deadline. Active scheduling, persistent-store TTL indexing, lazy lookup validation, and delayed-delete retry all converge on `Unknown task`; cleanup failure never resurrects or publicly distinguishes an expired record.
- Version 1 advertises no task listing.
- Task metadata and framework telemetry never disclose application or private invocation values.
- Runtime task selection does not alter the invocation fingerprint; changing the compiled delivery profile changes the surface hash.
- Task store and access policy enter adapter construction as one complete single-assignment runtime pair; no public API can author or replace only one side.

### Implementation Phases

1. Extract the shipped legacy task lifecycle and fixtures from the current effect-lane rmcp adapter into a protocol-profile module without changing released effect-lane behavior. RFC 0015 contributes only its already-landed protocol-neutral task-support declarations and surface compiler.
2. Add `TaskDeliveryDecl`, `CompiledTaskDelivery`, the surface builder/snapshot accessors, exact profile/protocol validation, canonical snapshot projection, and surface-hash fixtures through RFC 0015's existing types. Keep `TaskSupportSpec` authoring and group homogeneity in RFC 0015; introduce no provisional or parallel surface API.
3. Introduce the shared private runner, authority-bearing execution capsule, stored execution outcome, explicit task store, access policy, atomic transitions, retention, and redacted errors.
4. Correct or update the legacy rmcp `CreateTaskResult._meta` model and pass raw 2025-11-25 wire fixtures.
5. Add the Twill-owned stateless stdio/HTTP serving adapters, crate-private closed base and extension wire modules, and profile-selected pre-legacy dispatcher; then implement request capability parsing, server-directed materialization, inline `tasks/get` outcomes, update acknowledgements, cooperative cancellation, and exact raw-wire fixtures.
6. Compose tasks with the already-landed workspace, conversation-identity, permission, confirmation, ambient-resource, result, event, and surface APIs; move RFC 0016's prepared binding authority into the task capsule; prove RFC 0019 generated hosts use only ordinary fallback in version 1; then run the complete lifecycle and migration suites.

### Acceptance Tests

Acceptance lives in `crates/mcp-twill/tests/tasks.rs`. RFC 0015 retains only surface-compilation cases and delegates task lifecycle vectors here.

- `TaskDeliveryDecl::default()` compiles as `Disabled`; omitted, explicit-default, and explicit native `Disabled` declarations compile identically. `Disabled` rejects any `Required` operation, and repeated delivery assignment fails construction. Compile-fail coverage proves effect-lane compatibility constructors expose no task-delivery or protocol-target authoring method, while their exact snapshot retains legacy 2025-11-25 delivery.
- Public-API coverage authors delivery only through `NativeToolSurfaceBuilder::task_delivery`, reads it only through `NativeToolSurfaceSnapshot::task_delivery`, and proves the returned enum, inner accessors, canonical member, and surface hash agree. `TaskDeliveryDecl::is_disabled` remains an internal serde-normalization helper rather than a public semantic query.
- Finalization rejects a task-runtime pair on `Disabled`, a missing/repeated pair, `ServerInstance` on legacy delivery, or `Connection` on `TasksExtension`, and may read only the rejected store's pure deterministic `scope()` metadata; instrumentation proves it invokes no operational store method or access provider. Legacy compatibility constructors, RFC 0015 native convenience constructors over a legacy profile, and a fresh legacy builder install exact `InMemoryTaskStore::connection()` plus `CapabilityId`; one equivalent explicit pair preserves behavior without changing surface identity. Native convenience construction over `TasksExtension` fails for the missing private pair, while the equivalent finalizing builder succeeds after one complete `ServerInstance` pair. Compile-fail coverage proves there are no public store-only or access-only builder methods.
- Direct, grouped, and effect-lane tools preserve exact `TaskSupportSpec`; every mixed group fails before publication.
- The MCP task fixture manifest verifies exact repository commits, protocol revision, SEP identity, extension identifier, derivation kinds, directory membership, source paths, and payload checksums before any wire assertion. Tampered, extra, missing, partially refreshed, or path-escaping fixtures fail closed; normal tests require no network or sibling checkout. Reviewed vectors remain visibly distinct from exact source copies, and changing the pinned extension source without changing the frozen wire corpus is accepted only as a provenance-only review.
- Frozen canonical snapshots prove exact disabled, legacy, and extension delivery members, capabilities, tool metadata, protocol revision, runtime contract version, record bound, typed accessors, and surface hashes. A legacy profile/revision mismatch rejects before publication; a stateless mismatch between the compiled extension target and either the per-request metadata or Streamable HTTP protocol header rejects before extension dispatch and task creation. A synthetic runtime-contract increment changes the surface hash. Codec fixtures prove that one runtime version has one exact writer byte language and decoder acceptance set; changing either requires the increment before a new server can address records under its surface identity.
- Public construction coverage proves `into_stateless_service` accepts only a finalized native `V2026_07_28` adapter, exposes both `serve_stdio` and `into_http_service`, and rejects effect-lane or legacy targets without invoking private sidecars. Existing legacy `.serve(...)` fixtures remain source-compatible, while compile fixtures migrate the provisional `V2026_06_30` name and serving call. Raw ingress fixtures prove the stateless service routes base methods plus extension `tools/call`, `tasks/get`, `tasks/update`, and `tasks/cancel` through Twill's closed types before rmcp's legacy request union; `Disabled` never reaches extension lifecycle types and legacy surfaces never reach either stateless module. Missing members, duplicate known members, wrong kinds, and non-empty version-1 capability objects fail at that seam. Unknown request-parameter siblings are ignored, including a legacy `task` member on `2026-07-28` `tools/call`; matrices prove none can select task delivery or reach a store, runner, plan, public API, or application callback. Instrumentation proves temporary raw request/result values likewise never cross the private adapter boundary. Equivalent future rmcp typed models must pass the same frozen fixtures before replacing either private compatibility module.
- Capability and method inventories for both legacy and extension surfaces omit task listing. A raw `tasks/list` call returns method-not-found before task access, storage, or application instrumentation fires, while the profiles' exact-id operations remain available under their declared access rules.
- Legacy fixtures prove the exact 2025-11-25 capability, task request, revision-zero `Working` creation seed, completed/failed mapping, `tasks/result`, related-task metadata merge, terminal cancellation error, TTL/poll fields, and absence of extension fields/methods. A zero-TTL creation still returns its retained working seed after successful insertion and every subsequent operation may return only `Unknown task`.
- Legacy support matrices prove task augmentation of `Forbidden` and ordinary invocation of `Required` return exact `-32601` before task creation, while `Optional` accepts either caller-selected path.
- Extension fixtures prove request-local capability parsing, server-directed `resultType: "task"` with the revision-zero `working` creation seed, ordinary `resultType: "complete"`, inline terminal `tasks/get`, complete update/cancel acknowledgements, ignored update responses when no input is outstanding, cooperative cancellation, `ttlMs`/`pollIntervalMs`, polling-only operation, and absence of every legacy capability/result/notification field. A legacy `task` request member is accepted only as an ignored additive field.
- Every `tasks/get`, `tasks/update`, and `tasks/cancel` fixture independently covers present, absent, malformed, and context-injected extension capability. Missing capability returns exact `-32003` required-capability data and malformed capability returns invalid parameters before access-provider or store instrumentation fires. `tasks/result` always returns `-32601` on the extension surface.
- Streamable HTTP fixtures require exactly one textual `Mcp-Name` equal to `params.taskId` for each extension task method. Missing, repeated, malformed, and unequal headers return only `Invalid task routing` before access or storage; stdio fixtures need no header. Redaction probes prove neither the header nor raw task id enters framework projections or logs.
- Extension declaration fixtures accept retention `1` and `604800000`, reject zero, larger, fractional, negative, non-number, missing, or duplicate values, and prove retention participates in canonical bytes/surface hash while runtime timestamps do not.
- Extension support matrices cover `Forbidden`, both optional policies, and `Required` with and without capability; missing required support returns exact `-32003` before task creation.
- Request extension metadata and `progressToken` cannot be injected through `InvocationContext` or context fallback. A valid request capability or progress token does not erase unrelated private context, malformed capability creates no task, and a context-only progress token produces no notification authority.
- Boundary fixtures distinguish pre-task protocol rejection from post-task invocation failure: an undecodable outer call and unknown public tool name create no record, while malformed canonical identity, malformed enabled workspace context, and missing/unknown grouped selectors create one observable task whose terminal envelope contains the same redacted tool outcome as ordinary delivery. None creates an execution capsule or invokes planning stages that follow the failure.
- Instrumented ordinary and deferred adapters apply the same RFC 0009 request-over-context application-metadata merge and the same RFC 0013/RFC 0009 normalizers exactly once. Deferred normalization drops the raw effective map while retaining only typed context/observations and the separately captured request progress token in the live runner; operation selection and planning then consume the arguments and resolve/drop those observations. A normalization failure drops arguments, observations, and progress authority after committing its redacted task outcome. Poll, update, cancel, store reload, and orphan reconciliation perform zero additional normalizer calls, while serialization, schema, and `Debug` probes expose none of the carrier's metadata, arguments, or progress token. Raw-wire fixtures prove a request token survives unrelated context metadata, a same-key context token cannot replace it, and a context-only token is ignored.
- A table-driven ordinary/legacy/extension matrix covers wrong lane, malformed context, invalid arguments, missing binding, dry run, permission denial, confirmation denial/cancellation/failure, binder/refusal, handler failure, result-contract violation, application error, and success. Every fitting case returns the same underlying tool body; only the profile-owned task status/result envelope differs. A separate oversized valid outcome deliberately follows the task-infrastructure replacement below rather than partially storing that body.
- Extension `CallToolResult.isError: true` is `completed` with the exact result inline, while a JSON-RPC execution error is `failed`; legacy maps both error tool results and JSON-RPC errors to `failed` and retains the exact terminal outcome through `tasks/result`.
- Confirmation and permission fixtures prove that a working task owns no capsule before approval, every non-allow path dispatches nothing, and approval moves the exact prepared invocation once without replanning.
- Deterministic race fixtures pause before planning, realization, handler completion, and result commit. Legacy cancellation atomically wins or loses; extension cancellation always acknowledges a known task and permits cancelled, completed, or failed terminal outcomes according to the actual winner.
- A creation-response race fixture pauses response writing after successful store creation, lets the runner commit a terminal revision, and proves both profiles still return the retained revision-zero `working` seed while the first `get` observes the terminal record. No post-create lookup participates in creation success or response shaping.
- `InMemoryTaskStore::{connection, server_instance}()` each return `Arc<InMemoryTaskStore>` and coerce directly into the builder's `Arc<dyn TaskStore>` slot. Clones of one returned `Arc` share read-after-create, atomic insert-if-absent, atomic compare-and-set, and atomic complete removal; two constructor calls create disjoint concurrently usable namespaces; dropping the last clone loses the first namespace without claiming restart resumption. Legacy fixtures share a connection store across one negotiated session's handler clones, keep it across a replacement response stream, reject reuse by a separately negotiated session, and prove session teardown makes every id unreachable while only best-effort-cancelling accepted work. Cross-profile fixtures reject `ServerInstance` on legacy and `Connection` on extension. A persistent-store fixture holds one exclusive live server-instance mount for its namespace: a second concurrent mount cannot expose `ServerInstance`, while a sequential remount after the first owner drops can recover terminal records and classify a retained working record as orphaned.
- An external-store fixture implements `TaskStore` using only `TaskStorageKey::as_bytes`, `TaskRevision::get`, `TaskExpiration::unix_millis`, `StoredTaskRecord::{revision, expires_at, storage_bytes, into_storage_bytes, from_storage_bytes}`, and `TaskStoreError::new`; it persists and reloads every state, installs finite backend TTLs, and handles null legacy TTL without semantic record access. A delayed-visibility backend blocks inside `create` until same-key `get` can observe the inserted record or a later revision, proving that `CreateTaskResult` never precedes read visibility. Compile-fail fixtures prove there is no public raw-id/scope accessor, other task-timestamp or semantic-state accessor, unchecked key or stored-record constructor, or direct semantic `TaskRecord` store API.
- The external-store fixture also uses key `Clone`/`Copy`/equality/hash, revision and expiration `Clone`/`Copy`/equality/order/hash, and scope/result `Debug`/`Clone`/`Copy`/equality without requiring access to a private field or an unredacted sensitive value.
- Scripted-store fixtures force one occupied generated id and prove Twill retries with a fresh id without changing or disclosing the existing record. Eight consecutive occupied ids return the static creation failure and create no task. Capacity fixtures atomically admit exactly `MAX_STORED_TASKS` records across mixed working/terminal/cancelled/null-TTL states, reject the next distinct key immediately as the same static creation failure without starting a runner, and admit one new task after atomic removal. Barrier-controlled calls for the final slot produce exactly one `Created` and only `CapacityExceeded` for other distinct keys; an existing key still returns `Occupied`. The in-memory and external stores pass the same vectors, and the compiled legacy/extension accessors plus canonical snapshot both expose `256`.
- Scripted-store fixtures inject errors at every store-operation boundary and prove that each returned `TaskStoreError` leaves storage unchanged. Create errors produce only `Task creation failed`; lookup/update/cancel/result errors produce only `Task storage failed` and preserve the prior revision. A terminal compare-and-set error retains one private candidate; after recovery exactly one of that candidate or a competing profile-specific cancellation transition commits, while expiration/removal discards the candidate without recreation. A backend adapter fixture with ambiguous commit acknowledgement must reconcile internally before returning a conforming result.
- Corrupt, unsupported, and correctly encoded but mis-keyed stored records all return only `Task storage failed`; no semantic field, rejected byte, codec reason, storage key, or backend source appears in direct formatting, protocol output, events, diagnostics, or logs. Constructor coverage proves external store and scope-provider errors retain their source privately while every public formatting/source path remains redacted.
- A retained external-store working record restored under the exact same store namespace, compiled surface identity, and access policy but with no live runner transitions only to static task-execution failure and never reconstructs or replays authority. Restart under a changed surface or access policy cannot address that record and returns only `Unknown task`.
- Frozen private-codec vectors round-trip every record state and finite/null expiration, reject unsupported versions, malformed lengths, trailing bytes, invalid revisions, timestamp overflow/range errors, and inconsistent status/outcome combinations, and never appear in a public projection. Exact-bound bytes decode; one byte beyond `MAX_STORED_TASK_RECORD_BYTES` fails before semantic allocation. A terminal outcome whose complete successor record is exactly one byte within the cap commits unchanged, while the first over-cap candidate is discarded and replaced by the fitting static `Task execution failed` record in both dialects without a partial store write or application-value disclosure.
- Task ids contain exactly 256 OS-random bits in 64 lowercase hex characters. No id or digest correlates with operations, fingerprints, context, or arguments.
- Frozen storage-key vectors cover native and effect-lane surface hashes, both access tags, equal and unequal private scopes, all fixed-width field boundaries, and domain separation. Two adapters sharing one store and exact surface/access configuration derive the same key; changing only catalog/surface identity, protocol target, delivery profile, access mode, task id, or scope derives a different key. Presenting a valid id through any mismatched adapter returns only `Unknown task` and performs no record decode.
- Event and log vectors prove task creation, polling, update, cancellation, expiry, store failure, and pre-command transitions add no public `FrameworkEvent` field or framework log. A deferred command event remains byte-compatible with the equivalent ordinary command event and contains no raw or derived task identity, delivery profile, transition, status prose, scope, result, or infrastructure source.
- Capability-id access permits exact-id operations and exposes no list. Scoped access accepts principal byte lengths 1 and 4096, rejects zero and 4097 with the same redacted provider error, permits equal verified scope, and makes mismatch, provider failure, expiry, malformed id, and unknown id the same `-32602` response. Request `_meta`, `InvocationContext`, arguments, and generated envelopes cannot inject `TaskAccessContext`; the explicit host/test path can.
- An external-store fixture imports `mcp_twill::Result` while implementing `TaskStore`, reconstructing `StoredTaskRecord`, constructing `TaskAccessScope`, and implementing `TaskAccessScopeProvider`; every custom-error signature remains the explicit two-parameter `std::result::Result` and compiles independently of the framework alias.
- Controllable-clock fixtures prove exact timestamps, transition-only `lastUpdatedAt`, bounded poll values, active scheduled expiration, persistent-store TTL indexing, lazy atomic expiration of state and outcome, and unknown-id behavior after expiry. A scripted removal failure leaves stale bytes private, returns `Unknown task` on every operation and restart, retries cleanup, and never emits `Task storage failed` or resurrects the record.
- Adversarial arguments, identities, workspaces, resource references, application errors, source errors, store errors, and results never enter task status text, plans, previews, fingerprints, events, framework logs, schemas, or snapshots.
- Dropping an ordinary request cancels only that request; dropping a task-creating request after creation or disconnecting the transport leaves extension task lifetime intact.
- RFC 0019 version-1 host profiles accept forbidden/optional ordinary delivery, reject required operations, and expose no task lifecycle or hidden task argument. A table across disabled, legacy, and extension native surfaces proves the private host entrypoint explicitly selects immediate delivery, yields the same fitting application outcome and final host text subject to each surface's distinct identity hashes, and invokes zero task access, store, runner, codec, polling, or cancellation hooks. Host cancellation likewise leaves zero task records and emits no task protocol operation.

## Drawbacks

The proposal introduces a third contract layer beside command catalogs and serving surfaces: a serving surface now binds one task-delivery profile. That extra type is necessary because the official extension deliberately does not preserve the legacy handshake or state semantics.

Supporting both dialects costs substantial raw-wire and race testing. Legacy support remains valuable for the pinned SDK and released effect-lane adapter, but it must be treated as compatibility code with an exact protocol boundary rather than as the default meaning of “MCP tasks.”

Until rmcp exposes profile-safe typed extension dispatch, Twill also owns a narrow raw JSON-RPC seam ahead of the SDK's legacy request union. The closed types and frozen fixtures keep that seam local and replaceable, but it is additional adapter code that must be audited whenever the SDK transport changes.

An explicit server-instance store and access policy make extension construction more verbose. The verbosity exposes real deployment choices: whether later requests share state and whether task possession or authenticated scope authorizes access.

The fixed per-runtime record limit is intentionally global to one mounted namespace rather than partitioned by access scope or principal. It gives every conforming implementation one enforceable storage and runner bound, but a caller that retains many tasks can create admission pressure for unrelated callers sharing that runtime. The static creation failure avoids disclosing occupancy; deployments that need stronger fairness also need an authenticated quota authority rather than an application-context heuristic.

Persistent terminal records remain addressable only while the embedding restores the same store namespace, compiled surface identity, and access policy. A catalog, protocol, delivery, or access-policy migration intentionally makes old ids unknown even if opaque bytes remain until their backend TTL. Cross-version task migration would require a separately authenticated translation protocol; treating a new surface as authority for an old record would weaken both snapshot identity and access isolation.

The initial extension implementation does not resume live handler execution after process restart and does not emit `input_required`. It still implements the official polling/result/cancellation contract for ordinary deferred tool execution, while leaving resumable jobs and multi-round-trip input to proposals that can define their authority safely.

## Rationale And Alternatives

**Retarget RFC 0015 entirely to the Tasks Extension.** This would remove legacy prose but leave the existing adapter behavior and pinned SDK without a truthful owner. It would also keep task persistence and cancellation inside a tool-surface RFC. Extraction preserves the compatibility path while giving the extension a cross-surface home.

**Expose the native delivery declaration on the effect-lane profile.** That would turn a private compatibility compiler into a second authored surface family with protocol-target, schema-identity, and migration choices. No current adopter needs extension delivery without also needing an authored native surface. Version 1 therefore preserves the effect-lane profile exactly and leaves a configurable effect-lane declaration to a later RFC with concrete host evidence.

**Treat the extension as a field-renamed legacy task.** This is incompatible with server-directed creation, inline terminal results, `isError` status mapping, update/input-required support, and cooperative cancellation. A shared internal execution outcome is useful; a shared public state machine is not.

**Let a request choose immediate versus task delivery under the extension.** The extension assigns that decision to the server. Twill therefore compiles a deterministic optional policy and treats request capability only as permission to return the task result shape.

**Use conversation identity as task scope.** Conversation identity is caller context with application continuity semantics, not authenticated transport authority. Binding task access to it would let a metadata assertion become an authorization credential and would make identity absence incompatible with tasks.

**Persist complete invocation plans and resume them.** Plans intentionally omit private authority and application runtime objects. Reconstructing a dispatch from a task record would bypass current authorization, binder, resolver, and handler ownership. Initial persistence retains bounded task state and outcomes only.

**Use one global in-memory task map.** A process-global map obscures server ownership and test isolation. One explicit server-instance store is shareable across request routers without crossing independently finalized adapters.

**Advertise task listing for convenience.** A capability id proves authority to one task but cannot define a safe collection. Listing waits for a separately reviewed principal, pagination, filtering, and disclosure contract.

## Prior Art

The [MCP 2025-11-25 task specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks) provides Twill's legacy compatibility contract. Final [SEP-2663](https://modelcontextprotocol.io/seps/2663-tasks-extension) and the [Tasks Extension](https://modelcontextprotocol.io/extensions/tasks/overview) provide the official server-directed contract, including polymorphic results, inline polling outcomes, task updates, cooperative cancellation, request-local capabilities, and Streamable HTTP task routing. The checked-in provenance manifest, rather than a future rendering at those moving URLs, identifies the exact external text version 1 implements.

The [MCP progress contract](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/progress) additionally requires the token from the original task-augmented request to remain associated with the task until terminal status. Twill therefore keeps that request-owned token only with the live runner rather than serializing it into the durable public record or accepting a replacement from context fallback.

Job systems commonly separate a durable public record from a process-private worker lease. This RFC applies the same separation to Twill: the record supports polling and terminal result retention, while the execution capsule remains non-serializing authority tied to one live runner.

Capability URLs and object-capability systems motivate unguessable task ids when no authenticated principal exists. Authenticated transports may strengthen that with a private scope, but application context never substitutes for transport authority.

## Unresolved Questions

No architectural question blocks the initial task-delivery boundary. The initial extension profile deliberately omits `input_required`; integrating multi-round-trip task input with native confirmation requires a later RFC that defines who owns the pending input, how renewed authorization binds it, and whether application protocols may use it independently from confirmation.

## Future Possibilities

A resumable-job RFC could let an application store a validated opaque external job handle and reattach a new worker after process restart without serializing a Twill invocation or replay authority. A separate active/active task-store RFC could add durable runner leases and ownership transfer for multiple concurrently live servers over one persistence namespace.

A task-aware generated-host profile could project native host progress, polling, and cancellation from the same compiled delivery snapshot. It would need a host-owned persistence and UI contract rather than hidden tool arguments.

Scoped task listing could become available when an authenticated transport supplies a stable principal and the framework defines pagination, retention, and disclosure rules. Multi-round-trip task input could similarly build on the extension's `input_required` state after its interaction with authorization and prepared plans is reviewed.

A later runtime-contract version could add per-scope quotas or admission-rate limits. Such a change must name the trusted principal source, define atomic accounting and retention interaction, project every behavior-affecting bound through the compiled delivery member and surface hash, and preserve the same non-disclosing public failure contract.
