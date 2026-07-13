<!-- exo:9 ulid:01kwwnx9ftgxwcmkhde200ceax -->

# RFC 0009: Handler-Visible Workspace Roots

- Status: Accepted
- Area: command model, planning, handler context, request metadata, projection surfaces
- Target milestone: v0.4
- Depends on: RFC 0004 (runtime and workspace contracts), RFC 0007 (workspace resolution crate)

## Summary

This RFC lets a command declare that it consumes a named workspace root without accepting that root as an argument. The framework resolves the root from host observations, records any selected root on the invocation plan, and hands it to the handler through `CommandContext`. Help, catalog data, previews, diagnostics, and invocation fingerprints all derive from the same declaration.

Workspace use has two modes. A required declaration means the command cannot run until the workspace resolves. An optional declaration means the command can use a resolved root when one is available and receives `None` otherwise. Required use fits operations that read or write files. Optional use fits operations whose ordinary behavior is independent of the filesystem but which may bind syntactically valid host context for later application policy. RFC 0007 remains lexical and does not make filesystem-liveness claims.

The rmcp adapter derives workspace observations from one effective application-context metadata map. It begins with embedding-owned request-context metadata and overlays protocol `params._meta`, key by key, so caller-visible request metadata wins without discarding unrelated private observations. This same effective map feeds workspace resolution and the request-context consumers defined by RFC 0013. Protocol controls such as `progressToken` remain request-owned and never acquire context fallback authority. Codex's legacy `codex/sandbox-state-meta` becomes a workspace source only under an explicit trusted-host compatibility policy; generic MCP callers cannot claim that authority by sending the key.

After this RFC, “which directory can this command use” is a catalog fact. The model cannot assert a root through tool arguments, a required root fails before dispatch, an optional root never blocks an unrelated operation, and every root that affects execution participates in the plan fingerprint.

## Motivation

Visible Browser Lab demonstrates both workspace modes. Artifact export and file upload need a trustworthy workspace root because they cross the browser/filesystem boundary. Tab listing, navigation, and snapshots do not. A host may still attach workspace context to every call, but an absent, unsupported, or unmatched observation must not prevent a browser-only operation from running. The command declaration, not the mere presence of metadata, decides whether non-resolution is fatal. A lexically valid file URI may still name a path later removed from the filesystem; optional declaration does not add a liveness check or promise that later application I/O will succeed.

The framework already has the hard pieces. RFC 0007 defines workspace requirements, observations, authority ordering, resolution, and diagnostics. RFC 0004 places selected roots on invocation plans. Twill also gathers MCP roots and, for configured Codex integrations, sandbox metadata. What remains is a command-level declaration that carries a root to the handler even when no path argument refers to it, plus an optional form for commands that adapt to presence.

Without that declaration, handlers read environment variables or transport metadata directly. The workaround makes the catalog incomplete, bypasses planning diagnostics, hides the root from previews, and prevents fingerprints from binding approval to the directory that execution will use. It also duplicates transport parsing in every application.

The released VBL conversation-identity work exposed a second boundary. Protocol `_meta` and embedding-owned `RequestContext.meta` are distinct observations in rmcp. Selecting one whole map discards keys from the other. A progress token on the request must not erase a host-injected workspace, and a private fallback must not overwrite an explicit application-context observation. The adapter needs one per-key merge before any workspace or identity normalization occurs. That merge does not redefine protocol control ownership: the progress token used for notifications still comes only from the active request's `params._meta`, as MCP requires.

That merge does not itself confer trust. A generic MCP client can populate arbitrary `_meta` keys, so automatically normalizing every `codex/sandbox-state-meta` observation would let it select the Codex compatibility source and outrank a private host root or server fallback. As with RFC 0013's legacy `threadId` normalization, compatibility belongs to embedding configuration for a known Codex integration and is disabled by default.

## Guide-Level Explanation

A server declares a workspace once:

```rust
server.workspace(
    WorkspaceDecl::new("project", "file:///srv/default-project")
        .with_description("The project associated with this invocation"),
);
```

A command whose operation requires that workspace uses the existing declaration:

```rust
server.command("artifacts export", |command| {
    command
        .summary("Export an artifact into the project")
        .uses_workspace("project")
        .handle(export_artifact);
});
```

Planning resolves `project` before permission checks or handler dispatch. Failure produces the resolver diagnostics from RFC 0007. Success places the selected root and its provenance on the plan, and the handler reads it through `workspace_root`:

```rust
async fn export_artifact(ctx: CommandContext) -> Result<CommandOutput> {
    let root = ctx
        .workspace_root("project")
        .expect("required workspace use is resolved before dispatch");
    let destination = root.path()?.join("artifacts");
    // ...
}
```

A command that can adapt to workspace presence declares optional use:

```rust
server.command("tabs list", |command| {
    command
        .summary("List tabs owned by the selected browser session")
        .uses_optional_workspace("project")
        .handle(list_tabs);
});

async fn list_tabs(ctx: CommandContext) -> Result<CommandOutput> {
    match ctx.workspace_root("project") {
        Some(root) => broker.list_and_observe_workspace(root.path()?).await,
        None => broker.list().await,
    }
}
```

Optional means absence is valid. The resolver still applies its normal authority ordering. If a higher-authority observation is present but does not satisfy `project`, lower-authority fallbacks do not silently replace it; the optional result is simply absent. If resolution succeeds, the selected root appears on the plan and affects the fingerprint exactly like a required root.

Different commands may use the same workspace in different modes. A browser-only command can observe `project` optionally while `artifacts export` requires it. The application may remember a valid optional observation for its own session policy, but Twill does not turn optional use into a later guarantee. Every invocation is planned from its own declared mode and current observations.

The root never becomes a model-visible field. MCP roots, explicitly enabled trusted Codex sandbox metadata, typed direct-host observations, and server-declared fallback are the recognized sources. Tool arguments and generic metadata cannot enable a compatibility source, select a raw directory, or change the authority order.

A dedicated Codex embedding enables the two legacy observations explicitly and
independently:

```rust
let config = CliMcpServerConfig::default()
    .with_conversation_identity_compatibility(
        ConversationIdentityCompatibility::TrustedCodexThreadId,
    )
    .with_workspace_metadata_compatibility(
        WorkspaceMetadataCompatibility::TrustedCodexSandboxState,
    );
```

The identity switch grants authority only to `threadId`; the workspace switch
grants authority only to `codex/sandbox-state-meta`. Enabling either one never
enables, validates, or changes fallback behavior for the other. An embedding
that needs only one compatibility source selects only that policy.

An embedding such as a VS Code extension may inject the invocation token's trusted working directory through RFC 0013's existing private host/test context:

```rust
let context = InvocationContext::new().with_host_workspace_roots(
    HostWorkspaceRootsObservation::new([
        HostWorkspaceRoot::new(
            "com.microsoft.vscode",
            working_directory,
        )?,
    ]),
);

let outcome = registry.run_with_context(request, context).await?;
```

The optional name is used when a host exposes several roots. For this trusted-host tier, the existing single-root convenience rule applies only when the registry declares exactly one workspace and the observation contains exactly one valid file root; it is not inferred from the current command's required/optional subset. The issuer is stable provenance for plans and diagnostics, not an authority chosen by model input.

### How Agents Should Learn This

Help renders required and optional workspace use separately. Required use teaches that the command needs host filesystem context and may fail before dispatch. Optional use teaches that the command can take advantage of a host workspace but needs no argument and remains callable without one.

When a required workspace fails, structured diagnostics name the workspace and the observations that were considered. The agent should repair the environment—such as exposing the correct MCP root—rather than inventing a path argument. Optional absence produces no error and no recovery instruction because the call remains valid.

Previews include every selected root, regardless of declaration mode. When a human approves an effectful command, the selected root is part of the approved plan. A later call that selects a different root, gains a root where none was present, or loses a previously selected root produces a different fingerprint.

## Reference-Level Explanation

### Declaration

Required declarations retain the existing additive API. Optional declarations are a parallel additive surface:

```rust
pub struct CommandSpec {
    // Existing hard requirements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
    // Workspaces delivered when resolution succeeds; absence remains valid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional_workspaces: Vec<String>,
}

impl CommandSpec {
    pub fn uses_workspace(self, name: impl Into<String>) -> Self;
    pub fn uses_optional_workspace(self, name: impl Into<String>) -> Self;
}

impl CommandBuilder {
    pub fn uses_workspace(&mut self, name: impl Into<String>) -> &mut Self;
    pub fn uses_optional_workspace(&mut self, name: impl Into<String>) -> &mut Self;
}
```

Required and optional workspace declarations are semantic sets. Repeated declarations in the same mode deduplicate, and registration sorts each completed list by workspace id before catalog serialization, help, and hashing. Registration rejects a workspace named in both lists on one command because the author must choose whether absence is valid. Every name must match a server `WorkspaceDecl`; authored declaration order carries no runtime or catalog authority.

A path argument remains a hard workspace use for that invocation. If a command also declares the same workspace optionally, the path argument makes resolution required because the caller supplied a value that must be validated against a root.

### Effective Application-Context Metadata

The rmcp adapter constructs one effective application-context `Meta` before deriving workspaces, conversation identity, or any later application-context fact:

```rust
struct EffectiveApplicationMeta(Meta); // private, redacted, non-serializing

fn effective_application_meta(
    request: Option<&Meta>,
    context: &Meta,
) -> EffectiveApplicationMeta {
    let mut merged = context.clone();
    if let Some(request) = request {
        merged.0.extend(request.0.clone());
    }
    merged.0.retain(|key, _| {
        key != "progressToken"
            && !key.starts_with("io.modelcontextprotocol/")
    });
    EffectiveApplicationMeta(merged)
}
```

`request` is `CallToolRequestParams.meta`, not a value reconstructed from `RequestContext`. The rmcp adapter computes `effective_application_meta(request.meta.as_ref(), &context.meta)` before moving `request.arguments` into parsing. The merge is per key. Protocol request keys override context keys with the same name. Context-only application keys survive when the request contains other metadata such as `progressToken`. Construction then removes `progressToken` and every reserved `io.modelcontextprotocol/*` key before wrapping the map. `EffectiveApplicationMeta` is crate-private, implements neither serialization nor JSON Schema, has redacted `Debug`, and offers access only to the application-context normalizers that own recognized keys. The source maps and filtered wrapper are dropped after those normalizers produce typed observations.

`progressToken` is not an application-context fact. The ordinary progress sink reads it only from `CallToolRequestParams.meta`, and RFC 0020 captures that same request value separately for a live deferred runner. A `progressToken` present only in `RequestContext.meta` is ignored; it cannot request notifications or replace the token on the protocol request. The same request-only rule applies to RFC 0020 extension capability, related-task, version, and routing controls under the reserved prefix. Filtering prevents those values from being duplicated into a deferred application-context carrier. This ensures that every progress notification refers to a token actually supplied by the active protocol request while still allowing an unrelated request token to coexist with host-injected workspace or conversation metadata.

Ordinary and RFC 0020 deferred execution use the same effective application-context metadata captured from their complete `tools/call` params. This directly covers the deployed rmcp boundary where protocol `_meta` deserializes onto `CallToolRequestParams.meta` while `RequestContext.meta` may be empty. RFC 0020 task capability/request controls remain protocol-owned and are read from the request itself rather than this effective private-context map. Non-execution surfaces continue to consume only the metadata relevant to their protocol operation.

### Trusted Codex Workspace Compatibility

The rmcp adapter makes Codex sandbox normalization an explicit embedding policy:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceMetadataCompatibility {
    #[default]
    Disabled,
    TrustedCodexSandboxState,
}

pub struct CliMcpServerConfig {
    // ...existing fields...
    pub workspace_metadata_compatibility: WorkspaceMetadataCompatibility,
}

impl CliMcpServerConfig {
    pub fn with_workspace_metadata_compatibility(
        self,
        compatibility: WorkspaceMetadataCompatibility,
    ) -> Self;
}
```

`Disabled` ignores `codex/sandbox-state-meta` as a workspace source, regardless of whether the key came from protocol request metadata or private context metadata. MCP roots, typed direct-host observations, and declared fallbacks remain available. `TrustedCodexSandboxState` is selected only by server or embedding construction for a known Codex integration; request data cannot enable it. In that mode the effective metadata map is normalized after request-over-context merging, so a request value for the same key wins and unrelated context keys survive.

`workspace_metadata_compatibility` and RFC 0013's
`conversation_identity_compatibility` are separate single-purpose config
fields. Their builder methods assign only their corresponding field. Enabling,
disabling, or repeating assignment of one field has no effect on the other;
each method follows the shipped `CliMcpServerConfig` visible-value setter
semantics, so a later call for that same field replaces the earlier value
before the completed config enters server construction.

When compatibility is enabled, a present Codex payload must be an object with a non-empty string `sandboxCwd` (the shipped `sandbox_cwd` alias remains accepted) and an optional string `permissionProfile`/`permission_profile`. When both spellings of one field are present they must be equal; unrelated host extension fields are ignored. Normalization runs on every execution-tool call before selected-command planning, even when that command declares no workspace use. A malformed recognized payload therefore fails as a redacted `InvalidRequestContext` before workspace resolution, authorization, or dispatch; a valid observation affects execution only when planning resolves a declared workspace. When compatibility is disabled, every shape of that key is ignored rather than validated. This refines RFC 0007's adapter integration without changing its plain-Rust `CodexSandboxObservation` or resolver APIs.

### Direct Host Observations

RFC 0007's observation set gains an additive trusted-host source:

```rust
pub struct HostWorkspaceRoot {
    // Private, validated fields.
}

pub struct HostWorkspaceRootsObservation {
    // Private collection preserving present-empty versus absent.
}

impl HostWorkspaceRoot {
    pub fn new(
        issuer: impl Into<String>,
        uri: impl Into<String>,
    ) -> std::result::Result<Self, HostWorkspaceRootError>;

    pub fn named(
        issuer: impl Into<String>,
        name: impl Into<String>,
        uri: impl Into<String>,
    ) -> std::result::Result<Self, HostWorkspaceRootError>;

    pub fn issuer(&self) -> &str;
    pub fn name(&self) -> Option<&str>;
    pub fn uri(&self) -> &str;
}

impl HostWorkspaceRootsObservation {
    pub fn new(
        roots: impl IntoIterator<Item = HostWorkspaceRoot>,
    ) -> Self;

    pub fn roots(&self) -> &[HostWorkspaceRoot];
    pub fn is_empty(&self) -> bool;
}

pub enum HostWorkspaceRootError {
    InvalidIssuer,
    InvalidName,
    InvalidUri,
}

pub enum WorkspaceSource {
    // ...existing variants...
    TrustedHost,
}

pub struct ResolvedWorkspaceRoot {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_issuer: Option<String>,
}

pub struct PlanWorkspaceRoot {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_issuer: Option<String>,
}

impl WorkspaceObservationSet {
    pub fn with_host_roots(self, roots: HostWorkspaceRootsObservation) -> Self;
}

impl InvocationContext {
    pub fn with_host_workspace_roots(
        self,
        roots: HostWorkspaceRootsObservation,
    ) -> Self;
}

impl CommandRegistry {
    pub fn resolve_workspaces(
        &self,
        observations: &WorkspaceObservationSet,
    ) -> ResolvedWorkspaceSet;
}
```

`HostWorkspaceRoot`, `HostWorkspaceRootsObservation`, and `HostWorkspaceRootError` are defined by `mcp-workspace-resolver` alongside the other observation types and re-exported by `mcp-twill` for embedding authors. The collection wrapper preserves the difference between no host observation and a present observation containing zero roots, following RFC 0007's existing `McpRootsObservation` precedent. `CommandRegistry::resolve_workspaces` is the public counterpart to the existing declared-only convenience and always resolves the registry's complete requirement set against the supplied observations in canonical workspace-id order; its returned root vector is sorted by that id before it can be reused as pre-resolved input.

`InvocationContext::with_host_workspace_roots` is the ordinary direct-registry and embedding injection path. `InvocationContext` retains its RFC 0013 guarantees: private fields, no `Serialize` or `JsonSchema`, and redacted `Debug`. Its crate-private workspace accessor feeds the same observation set the rmcp and generated-host adapters use; handlers receive only successfully selected `PlanWorkspaceRoot` values through `CommandContext::workspace_root`. Existing identity-free/workspace-free constructors represent no host observation, and `build_plan_with_context`/`run_with_context` automatically combine an injected observation with declared roots.

RFC 0013 records RFC 0009 as the conceptual precedent for declared ambient request context, while its implementation reached `main` before this RFC 0009 reconciliation. The implementation boundary is therefore explicit: RFC 0013 continues to own the shared private `InvocationContext` container and its identity privacy contract, and this RFC adds workspace observation state and workspace-owned accessors to that existing type. It introduces no parallel request-context container and requires no RFC 0013 body, wire, declaration, or identity-semantic change. RFC 0009 remains the sole owner of workspace declaration, authority, resolution, projection, and diagnostics.

Workspace resolution has one authority input per call. Context-only entrypoints extract any host-root observation, combine it with the adapter's protocol/Codex observations and server declarations, and resolve exactly once. Existing public `*_with_workspaces_and_context` entrypoints already accept a pre-resolved `ResolvedWorkspaceSet`; after this RFC they require the supplied `InvocationContext` to contain no host-root observation and return redacted `InvalidRequestContext` before planning if both are present. They also validate that every selected root id names a registered workspace and occurs exactly once. An unknown id or duplicate id rejects before first-root lookup or operation planning; duplicate equal roots do not silently deduplicate because vector order must never choose authority. Conversation identity may still accompany a valid pre-resolved set because it is an independent fact. Adapter internals do not pair unrelated values: a crate-private assembly step produces one coupled prepared input from the effective observation set and private context, then planning consumes that result. This prevents a trusted host root from disagreeing with the selected workspace while preserving the existing pre-resolved API for callers that already own resolution.

This API is for an embedding or test harness already trusted to construct private invocation state. It is not populated from tool arguments or arbitrary application metadata. `HostWorkspaceRoot::new` constructs the common unnamed root; `named` makes the several-root case explicit without an `Option<String>` constructor argument. Both validate the issuer, optional non-empty name, and URI shape before the observation enters a set. The fields are private, and custom `Deserialize` delegates to the same validation, so neither Rust struct literals nor a host envelope can create an unchecked root. `HostWorkspaceRootsObservation` likewise exposes only read-only accessors over validated roots. Issuers use a non-empty lowercase reverse-DNS name whose labels begin and end with an ASCII letter or digit and may contain interior hyphens. A selected host root uses `WorkspaceSource::TrustedHost` and carries that issuer through `ResolvedWorkspaceRoot::source_issuer` into `PlanWorkspaceRoot::source_issuer`; all existing sources serialize the new field as absent. Root matching and path diagnostics continue to follow RFC 0007.

`HostWorkspaceRoot` and `HostWorkspaceRootsObservation` implement custom
redacted `Debug`. Formatting either value emits only its public type name and a
fixed `<redacted>` marker; it never formats a root count, issuer, name, URI, or
validation input. This keeps RFC 0013's existing derived `InvocationContext`
`Debug` safe after the private observation field is added. Direct accessors and
the explicit trusted-host serialization form remain available to embedding
code that deliberately consumes or transports the value; generic framework
debugging does not become a second disclosure path.

The serialized and schema-projected `HostWorkspaceRoot` form is the exact closed camel-case object `{ "issuer": string, "name"?: string, "uri": string }`; `name` is omitted when absent and unknown fields are rejected. Serialization can therefore cross a versioned trusted-host envelope without making Rust fields public. Deserialization first enforces that wire shape and then calls the same constructor validation used by direct Rust callers. `HostWorkspaceRootsObservation` serializes transparently as an array of those objects; presence remains the responsibility of its containing optional observation slot, so an absent slot and a present empty array stay distinguishable. Custom `JsonSchema` implementations describe these public wire forms rather than exposing private storage details.

Resolution authority is MCP roots, trusted Codex sandbox context when explicitly enabled, direct trusted-host roots, then server-declared roots. Presence blocks fall-through at every enabled tier. With Codex compatibility disabled, a metadata key creates no tier and cannot block a direct host root or declared fallback. A present host observation containing zero roots, or roots that do not match the requirement, therefore leaves a required use unresolved and an optional use absent; it never widens to the declared fallback. Complete absence of a host observation still permits the declared tier.

### Planning

The adapter resolves all workspaces declared by the selected command and any bound path arguments into one `ResolvedWorkspaceSet`. Planning applies the declarations in this order:

1. Path-argument workspaces and `workspaces` are required. An unresolved requirement returns `FrameworkError::WorkspaceUnresolved` before permission checks and dispatch.
2. `optional_workspaces` are inspected next. A successfully resolved root joins the plan's used set. An unresolved optional workspace contributes no root and does not fail the call.
3. The plan deduplicates roots by workspace id. Required use dominates optional use when a path argument makes the same workspace required for this invocation.

Framework resolution and validated pre-resolved input both produce one canonical root map keyed by workspace id. `plan.workspace_roots` is the complete selected subset projected in ascending workspace-id order. No placeholder entry represents optional absence. The command spec and operation catalog carry the declaration mode, while the plan carries only execution facts that actually apply. Handler lookup, ambient-binder iteration, preview rendering, and fingerprint construction all consume this same sorted vector; none performs an independent first-wins normalization.

Selected required and optional roots are available to dispatch-time resource resolution and ambient binders through the typed invocation plan. Those consumers receive resolved roots, not raw request metadata. This lets RFC 0016 bind an application resource to a workspace when one is available without making optional workspace absence fatal.

The fingerprint already includes the selected root list, including `source` and the additive `source_issuer`. Therefore present versus absent optional context, different root URIs, and different provenance produce different fingerprints without storing raw transport metadata.

### Handler Surface

The existing lookup remains the only handler API:

```rust
impl CommandContext {
    pub fn workspace_root(&self, id: &str) -> Option<&PlanWorkspaceRoot>;
}
```

For required command-level use, planning guarantees `Some`. For optional use, `None` is ordinary. For path-argument use, `Some` is guaranteed when the argument bound successfully. `PlanWorkspaceRoot::path()` continues to apply RFC 0007 normalization.

### Errors And Diagnostics

Required failures retain `WorkspaceUnresolved`, with a command-level `DiagnosticLocation::Workspace`. Optional non-resolution is not an error and produces no steering. Public and debug plans omit unsatisfied optional workspace ids: the declaration is already visible in catalog and help, while the plan remains a record of selected execution facts.

A malformed enabled Codex workspace payload uses the existing `InvalidRequestContext` response family through a workspace-owned framework variant:

```rust
pub enum FrameworkError {
    // ...existing variants...
    InvalidWorkspaceMetadata {
        key: String,
        field: Option<String>,
        reason: WorkspaceMetadataProblem,
    },
    ConflictingWorkspaceInputs,
    InvalidPreResolvedWorkspaceSet {
        workspace: Option<String>,
        reason: PreResolvedWorkspaceProblem,
    },
}

pub enum WorkspaceMetadataProblem {
    ExpectedObject,
    MissingSandboxCwd,
    InvalidSandboxCwd,
    InvalidPermissionProfile,
    ConflictingAliases,
}

pub enum PreResolvedWorkspaceProblem {
    DuplicateWorkspace,
    UnknownWorkspace,
}
```

All three variants map to `ResponseStatus::InvalidInput` and the existing `ErrorCode::InvalidRequestContext`. `InvalidWorkspaceMetadata` uses `DiagnosticLocation::RequestContext { key }`. Public details may contain only the recognized metadata key, optional schema-owned field name, and stable reason enum; raw metadata values, paths, validator text, and private context never appear. `ConflictingWorkspaceInputs` uses `DiagnosticLocation::RequestContext { key: "hostWorkspaceRoots" }` and a static message explaining that raw host roots and pre-resolved workspaces cannot be supplied together. `InvalidPreResolvedWorkspaceSet` uses key `preResolvedWorkspaces`; its details contain only the stable reason and include `workspace` only after that id is verified as a registered catalog name, so an unknown caller string is never echoed. Both pre-resolved failures expose neither root/provenance input and are returned before operation or command planning. RFC 0020 preserves any of these ordinary framework `CallToolResult` values and applies only its selected delivery profile's status/result envelope. Generated hosts retain the same framework code; events and logs may record only the public code, recognized key, optional catalog-owned workspace, optional field, and stable reason. Optional workspace semantics do not downgrade request-integrity failures from this or another context consumer. For example, malformed canonical conversation identity still fails under RFC 0013 even when every workspace declaration is optional.

### Projection

- **Catalog.** Required names remain in `workspaces`; optional names serialize as `optionalWorkspaces`. Both are hash-covered.
- **Help.** Command help renders `project (required, supplied by host)` or `project (optional, supplied by host)` under `Workspaces:`.
- **Preview and dry run.** Every selected root renders with workspace id, URI, and provenance. Optional absence renders no fabricated root.
- **Schema and examples.** Neither mode creates an input property or example value.
- **Contract checks.** `check_workspace_projection` validates names, mode exclusivity, spec/catalog equality, help wording, schema absence, and fingerprint coverage.

### Required Invariants

- Workspace roots are never model-visible tool arguments.
- Required workspace use fails before authorization and dispatch when unresolved.
- Optional workspace use never fails solely because the workspace is absent, unmatched, unavailable, or unsupported.
- A selected optional root reaches the handler and participates in previews and fingerprints exactly like a selected required root; trusted-host issuer provenance is preserved on both paths.
- The fixed authority order is MCP roots, explicitly enabled trusted Codex sandbox context, typed trusted-host roots, then server-declared roots. Presence at an enabled tier blocks fall-through; optional use changes only whether non-resolution is fatal.
- Host-root observation presence is explicit: a present empty collection blocks declared fall-through, while an absent observation permits it.
- Codex sandbox metadata is ignored by default and becomes authoritative only under `TrustedCodexSandboxState`; request data cannot enable the policy, and malformed enabled payloads fail before planning.
- Enabled Codex workspace metadata is an execution-request integrity claim even for commands without workspace declarations; valid values remain unavailable to handlers unless a root is selected for a declared use.
- Direct host observations are typed, trusted embedding inputs carried only through `InvocationContext`, with fixed authority below an enabled call-specific Codex source and above server fallback; model arguments and generic metadata cannot create them.
- Raw direct-host observations have redacted `Debug`; adding them to `InvocationContext` cannot make its existing derived formatter disclose an issuer, name, URI, or collection shape.
- Required/optional declaration lists and selected plan roots are canonical workspace-id sets. Pre-resolved roots must be unique and registered; every public consumer receives the same ascending-id plan vector.
- A call has exactly one workspace authority input: raw observations resolved by Twill, or a caller-supplied pre-resolved set. Public APIs reject a host-root-bearing `InvocationContext` paired with a pre-resolved set rather than choosing between them.
- Protocol request metadata overlays private context metadata per key for application-context normalization; unrelated context keys survive, while `progressToken` and reserved `io.modelcontextprotocol/*` controls are removed from the resulting private wrapper.
- Ordinary and deferred execution derive workspace and conversation context from the same effective application metadata. Progress, RFC 0020 task capability, related-task, and routing controls remain request-owned and never use context fallback.
- Registration rejects undeclared workspace names and a command declaring the same workspace as both required and optional.

### Implementation Phases

1. Add optional workspace declarations, registration validation, catalog projection, help, and contract coverage.
2. Extend resolution and planning so successful optional roots join the used set, unresolved optional roots remain absent, and trusted-host source/issuer provenance reaches plans and fingerprints.
3. Centralize effective application-metadata merging in the rmcp adapter and route workspace, conversation identity, and ordinary/deferred application context through it. Read progress and RFC 0020 protocol controls separately from the complete request metadata that owns them.
4. Extend the non-serializing `InvocationContext` with a presence-preserving `HostWorkspaceRootsObservation` and route direct registry, rmcp, deferred, and generated-host calls through the same observation assembly.
5. Add default-disabled `WorkspaceMetadataCompatibility` and strict trusted-Codex normalization without changing the plain resolver API.
6. Add a representative server and raw-protocol acceptance tests for required, optional, and mixed workspace use.

### Acceptance Tests

Acceptance extends `crates/mcp-twill/tests/workspace.rs`. The owner-local landing
proves direct-registry and ordinary rmcp declaration, normalization, planning,
handler visibility, projection, fingerprint, and non-disclosure behavior.
Bullets that explicitly require RFC 0020 deferred delivery or RFC 0019
generated-host execution are delegated to `tasks.rs` and `host_adapters.rs`
respectively; those suites reuse this RFC's public context and workspace APIs
and cannot redefine its authority order or diagnostic ownership.

- A command with `uses_workspace("project")` and no path argument resolves against MCP roots, explicitly enabled trusted Codex sandbox metadata, direct host roots, and a declared fallback according to the fixed authority order.
- The required command fails with `WorkspaceUnresolved` before authorization and dispatch when no observation satisfies it.
- A command with `uses_optional_workspace("project")` runs with `None` when the workspace is absent, unavailable, unsupported, or blocked by an unmatched higher-authority observation.
- The optional command receives `Some(root)` when resolution succeeds; changing between absent, one root, and another root changes the invocation fingerprint.
- An optional browser-only command succeeds with `None` for an absent, unsupported, or unmatched observation, while a file-sensitive required command against the same non-resolving observation fails before dispatch. A lexically valid file URI remains selected even if the path is later deleted, matching RFC 0007's no-I/O resolution semantics.
- A command that declares optional use but binds a path argument to the same workspace treats the invocation as required and validates the path.
- Registration rejects unknown workspace names and required/optional mode conflicts; duplicate declarations in one mode deduplicate.
- Reordering required declarations, optional declarations, path arguments, or a valid pre-resolved root vector leaves normalized catalog data, selected plan roots, previews, handler lookup, and fingerprints unchanged. Duplicate pre-resolved ids fail even when roots agree; unknown ids fail without echoing the supplied id, and neither case reaches operation planning.
- Legacy `CommandSpec` JSON without `optionalWorkspaces` and legacy resolved/plan roots without `sourceIssuer` deserialize to their absent defaults, reserialize without the new fields, and retain identical catalog and workspace-specific fingerprint inputs when no new workspace fact is adopted. Explicit empty/`None` values normalize to the same bytes; RFC 0015's separate mandatory serving-identity member still performs its documented one-time complete-fingerprint migration.
- Catalog, help, preview, schema, and contract projections distinguish required and optional use without exposing a workspace input property.
- With the default compatibility policy, every `codex/sandbox-state-meta` shape is ignored and cannot override or block a direct host root or declared fallback. With `TrustedCodexSandboxState`, a valid raw `tools/call` payload reaches planning; request metadata overrides the same context key, while a request carrying only `progressToken` retains context-injected workspace and conversation metadata. That request token alone drives progress; a context-only token is ignored and cannot replace it. A four-way configuration matrix proves workspace and conversation-identity compatibility are independent: enabling either one alone normalizes only its owned key, enabling both normalizes both through the same effective map, and assigning either field twice changes only that field according to ordinary config setter semantics.
- Under trusted Codex compatibility, every malformed sandbox shape maps to the corresponding stable `WorkspaceMetadataProblem` and a request-context location without serializing the raw payload or path; the same malformed values are ignored when compatibility is disabled.
- A command with no required or optional workspace still rejects malformed enabled Codex metadata before its authorizer and handler, while valid metadata creates no plan root or fingerprint input for that command.
- A raw request-only canonical conversation identity reaches RFC 0013 normalization from `CallToolRequestParams.meta` even when `RequestContext.meta` is empty; the same fixture covers ordinary and RFC 0020 deferred calls while proving progress and task protocol controls never come from context fallback or survive in `EffectiveApplicationMeta`. A request progress token remains usable when context contributes unrelated identity/workspace keys, while a context-only token produces no progress notification authority.
- A VS Code-style root injected through `InvocationContext` reaches both direct `run_with_context` and generated-host execution, resolving an optional or required workspace as `TrustedHost` with exact issuer provenance. A present unmatched host root blocks declared fallback; `HostWorkspaceRoot::new` rejects an invalid issuer, and `HostWorkspaceRoot::named` rejects an empty name before either observation can reach planning. Exact JSON and JSON Schema fixtures prove the closed `{ issuer, name?, uri }` wire form, omitted optional name, transparent observation array, and rejection of unknown or malformed fields. Custom deserialization rejects the same malformed values, and compile-fail coverage proves external code cannot use a struct literal to bypass validation. Changing only the trusted issuer changes the plan fingerprint while existing source serialization remains unchanged.
- Every public pre-resolved-workspace-and-context entrypoint accepts an identity-only context but rejects a context containing host-root observations as the exact redacted `ConflictingWorkspaceInputs`/`InvalidRequestContext` diagnostic before planning. Context-only and adapter-private paths assemble raw MCP, enabled Codex, trusted-host, and declared observations once and cannot select a workspace from a set that disagrees with the retained private context.
- No host observation allows declared fallback. A present empty `HostWorkspaceRootsObservation` blocks that fallback, producing required non-resolution or optional absence exactly like a present unmatched host-root list.
- Adversarial issuer, name, and URI strings never appear when formatting `HostWorkspaceRoot`, `HostWorkspaceRootsObservation`, or a containing `InvocationContext` with either `Debug` formatter. Raw injected observations never serialize through `InvocationContext`, responses, events, or RFC 0020 task state; only selected plan roots expose the URI and issuer provenance promised by workspace previews.
- Deferred and ordinary calls use the same merged application context and produce the same selected root and fingerprint.
- An external-construction fixture imports `mcp_twill::Result` and still compiles `HostWorkspaceRoot::{new, named}` against their explicit `std::result::Result<_, HostWorkspaceRootError>` signatures, proving that the framework result alias cannot capture this constructor-owned validation channel.

## Drawbacks

Optional use weakens the simple guarantee that every declared workspace is present in the handler. Authors must read the declaration mode before treating `workspace_root` as guaranteed, and generated help must make the distinction visible.

Keeping separate required and optional lists is less elegant than a single typed list. It preserves the shipped catalog shape and makes the change additive, at the cost of two fields and a cross-list validation rule.

Ignoring optional non-resolution can conceal a host configuration problem until a required command runs. That is the intended trade: an operation that does not need the filesystem should remain available, while the first operation that does need it produces the full resolver diagnostic.

Default-disabled Codex compatibility is an intentional runtime migration for embeddings that relied on the adapter's previous unconditional `codex/sandbox-state-meta` parsing. A dedicated Codex integration must now enable `TrustedCodexSandboxState` during server construction. Enabling it on a generic MCP endpoint would let arbitrary callers select the compatibility workspace source; Twill makes that deployment assertion explicit but cannot prove the embedding's trust boundary.

## Rationale And Alternatives

**Always require a declared workspace.** This is the existing strict model and remains the right default for filesystem operations. It is insufficient for applications that receive ambient workspace observations on every call but use them only on a subset of operations.

**Let handlers inspect raw metadata opportunistically.** This preserves flexibility but abandons catalog authority, provenance, preview, fingerprint, and shared normalization. Optional declarations provide the flexibility without reopening those holes.

**Represent optionality as `uses_workspace("project").optional()`.** A nested declaration builder is possible, but it does not fit the existing mutable `CommandBuilder` method shape and makes the low-level model more complex. Parallel methods and additive catalog fields preserve source and wire compatibility.

**Fall through to lower-authority roots when an optional observation is unusable.** This is rejected because optionality governs whether absence is fatal, not who is authoritative. An observed client or host root must not be silently replaced by server configuration.

**Always normalize `codex/sandbox-state-meta`.** This preserves the original adapter shortcut but lets any generic MCP caller assert a compatibility-only workspace source with higher authority than private host and server roots. Default-disabled trusted-host configuration preserves Codex integration without turning a recognizable key into authority.

**Store unresolved optional diagnostics on every plan.** This would make debug plans richer but risks carrying raw or high-cardinality host details through otherwise unrelated operations. The first implementation keeps plans execution-focused and omits absent optional roots.

## Prior Art

LSP workspace folders and editor extension contexts are ambient, host-owned facts that operations may consume without accepting a path argument. Dependency-injection systems similarly distinguish required and optional dependencies while preserving one lookup surface in the consumer.

MCP roots provide the protocol-level source, RFC 0007 provides the authority and normalization rules, and RFC 0013 provides the private invocation-context precedent. This RFC connects those pieces at command granularity.

The [MCP progress contract](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/progress) requires notification tokens to have been provided in the active request. That is the protocol precedent for separating request-owned controls from the application-context metadata merge even though both occupy `_meta` on the wire.

## Unresolved Questions

No architectural question blocks the initial workspace-context boundary. Unresolved optional observations remain absent from plans and framework events; a future deployment-diagnostic channel may expose a separately reviewed bounded reason without changing invocation behavior or model-visible output.

## Future Possibilities

Commands that spawn processes could declare working-directory semantics, allowing the framework to set the child process directory from a required workspace and render that fact in previews.

Effects could declare workspace-relative scopes, tightening “write” into “write under project.” A future runtime might also expose redacted optional-observation telemetry for deployment diagnostics without adding those observations to plans or model-visible responses.

A versioned catalog format could eventually replace the two additive lists with `WorkspaceUse { name, required }` once consumers can negotiate that wire change.
