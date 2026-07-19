<!-- exo:19 ulid:01kxc1g5cb5c3m0wzb48jycvv7 -->

# RFC 0019: Catalog-Derived Host Adapters

- Status: Draft
- Area: host adapters, generated artifacts, private invocation transport, result projection
- Target milestone: v0.4
- Depends on: RFC 0009 (typed host workspace observations), RFC 0011 (guidance decomposition), RFC 0013 (conversation identity), RFC 0014 (application results), RFC 0015 (native surface snapshots), RFC 0016 (ambient resource binding), RFC 0018 (invocation and confirmation presentation), RFC 0020 (protocol-versioned task delivery)

## Summary

This RFC defines catalog-derived adapters for hosts that do not consume Twill's MCP tool list directly. A host adapter profile consumes one RFC 0015 native surface snapshot and declares only host-owned projection facts: public name prefixing, icons, prompt-reference aliases, host guidance, confirmation presentation and entrypoint authority, model-visible result shaping, private invocation transport, and unsupported-context behavior. Twill validates the profile and generates a canonical host snapshot, artifact hash, manifest data, and runtime adapter helpers.

The initial concrete host is VS Code's `LanguageModelTool` API. Generated contributions derive names, display and model descriptions, input schemas, prompt aliases, invocation messages, and confirmation messages from the Twill surface. Runtime calls send the selected tool, one immutable logical snapshot of the model arguments, and one immutable typed private-context snapshot through a versioned host envelope. Success and declared errors derive from RFC 0014; fixed result projection can omit host-inappropriate fields such as an explicit session handle without a hand-written JavaScript filter.

Confirmation authority is explicit in that profile. `ServerOnly` keeps generated UI as presentation and sends every base `RequireConfirmation` decision through the native confirmation route. `TrustedVsCodeUi` names an inclusive tested engine range; the registered generated adapter captures the runtime version it observes from VS Code, and the explicitly trusted host entrypoint may satisfy only a base requirement for which that asserted version is in range and the same compiled trigger matches. Twill validates the closed fact and its range but does not authenticate its origin from the value itself. Denial is never widened, no approval flag crosses the transport, and no reusable replay authority is created.

Host extraction of opaque platform objects remains host code. Twill cannot inspect VS Code's `toolInvocationToken`, so the extension supplies a small `HostContextProvider` that returns canonical conversation identity and typed workspace observations or a stable unsupported result. That hook cannot change tool schemas, dispatch mappings, result contracts, or authority order.

The host adapter hash is distinct from catalog and native-surface hashes. Editing a prompt alias or icon changes the generated artifact identity without pretending command semantics changed. Raw context and projected-away result fields never participate in model-visible host results, generated manifests, or logs.

## Motivation

RFC 0015 removes the largest duplicate authority in a VBL-on-Twill port: direct and grouped MCP tools derive from the same command catalog they dispatch. VS Code still has a second adapter contract around those tools.

The current extension and build task hand-maintain:

- a `visible_browser_lab_` name prefix and four prompt-reference aliases;
- display names, browser icons, model descriptions, and server instructions;
- removal of `agent_session_id` from contributed input schemas;
- invocation-status and conditional confirmation switches;
- extraction of conversation identity and workspace root from an opaque invocation token;
- a process-per-call request envelope;
- success/error envelope parsing and error-string formatting;
- removal of `agent_session_id` from model-visible results;
- cancellation of the wrapper process.

Some of those facts are genuinely host-owned. The prefix and icon have no meaning to an MCP client. Opaque-token extraction can only be implemented against VS Code. The problem is that they are mixed with facts Twill already owns: schemas, operation routing, identity payload shape, resource binding, result errors, and presentation. The extension can drift even when the Rust surface remains internally consistent.

The released v0.4.8 process wrapper also makes the operational boundary measurable. It concatenates stdout and stderr into unbounded strings, includes child stderr in a rejected JavaScript error, and resolves cancellation immediately after requesting `child.kill()` rather than waiting for process exit. Those choices were reasonable local glue, but a reusable generated transport needs bounded call/result/log paths, redacted failures, concurrent draining, and explicit reaping semantics. This RFC makes those properties part of the profile and acceptance contract instead of carrying the wrapper forward unchanged.

Generated manifest validation helps detect drift after two copies exist. This RFC removes the second copy. A host profile is declarative input to the same surface compiler, and the generated adapter consumes a versioned snapshot rather than reconstructing MCP semantics in TypeScript.

## Guide-Level Explanation

A server first builds its native surface, including the ambient-only binding appropriate for VS Code:

```rust
let vscode_surface = NativeToolSurface::builder("vbl-vscode")
    .framework_help(FrameworkHelpProjection::Omitted)
    .confirmation_route(NativeConfirmationRoute::Unavailable)
    // direct and grouped tool mappings
    .bind_resource::<Session>(
        AmbientResourceBinding::from_conversation_identity(session_binder)
            .omit_explicit_carrier()
            .missing_as("session_required"),
    )
    .build(&registry, McpProtocolTarget::V2025_11_25)?;
```

It then declares host-owned projection:

```rust
let vscode = HostAdapterProfile::vscode(
    "vbl-vscode-host",
    VsCodeVersion::new(1, 120, 0),
)
    .tool_name_prefix("visible_browser_lab_")
    .icon("$(browser)")
    .prompt_reference("help", "vbl")
    .prompt_reference("snapshot", "vbl_snapshot")
    .prompt_reference("screenshot", "vbl_screenshot")
    .prompt_reference("navigate", "vbl_navigate")
    .confirmation(HostConfirmationPolicy::trusted_vscode_ui(
        HostConfirmationTrigger::DeclaredPresentation,
        VsCodeEngineRange::inclusive(
            VsCodeVersion::new(1, 120, 0),
            VsCodeVersion::new(1, 128, 0),
        ),
    ))
    .omit_result_property("session start", "agent_session_id")
    .unsupported_context(
        UnsupportedContextPolicy::new()
            .allow("help")
            .reason(
                HostContextReason::UnknownTokenShape,
                "VS Code did not expose a compatible chat session resource; Visible Browser Lab requires VS Code 1.120 or newer with the supported invocation-token shape",
            )
            .reason(
                HostContextReason::InvalidSessionResource,
                "VS Code did not expose a compatible chat session resource; Visible Browser Lab requires VS Code 1.120 or newer with the supported invocation-token shape",
            )
            .reason(
                HostContextReason::InvalidWorkingDirectory,
                "VS Code did not expose a compatible chat session resource; Visible Browser Lab requires VS Code 1.120 or newer with the supported invocation-token shape",
            )
            .reason(
                HostContextReason::ProviderFailed,
                "VS Code did not expose a compatible chat session resource; Visible Browser Lab requires VS Code 1.120 or newer with the supported invocation-token shape",
            )
            .recover_by(
                "update_or_use_explicit_surface",
                "update and reload VS Code, or use the explicit MCP/CLI surface",
            ),
    )
    .absent_context_rejects(
        "session start",
        HostApplicationRejection::new("session_required")
            .runtime_message(
                "Global VS Code tool invocations have no conversation identity and do not expose explicit session handles",
            )
            .recover_by(
                "use_chat_or_explicit_surface",
                "invoke Visible Browser Lab from a supported VS Code chat, or use the explicit MCP/CLI surface",
            ),
    )
    .invocation_limits(HostInvocationLimits::new(
        1_048_576,
        1_048_576,
    ))
    .process_envelope(
        "bin/visible-browser-lab-mcp",
        ["host", "call"],
        HostProcessLimits::new(
            1_048_576,
            2_000,
        ),
    )
    .build(vscode_surface.snapshot())?;
```

The corpus's [representative VBL `new_tab`
composition](../README.md#representative-adoption-vbl-new_tab) starts with the
command's optional workspace and `Res<Session>` declarations, then follows the
compiled ambient-only native route into this profile. The generated provider
contributes typed identity and workspace observations; RFC 0016 consumes them
to resolve the session, while the handler receives neither the opaque host
token nor the raw conversation tuple. Because the native snapshot already
omits the explicit carrier, generated TypeScript copies its input schema
unchanged and never performs an `agent_session_id` transform.

Build tooling generates both host-owned artifacts from that compiled snapshot;
it never accepts the declaration or native surface directly:

```rust
let generated = generate_vscode_artifacts(vscode.snapshot())?;

std::fs::write(
    "generated/vscode-manifest-projection.json",
    generated.manifest_projection_json(),
)?;
std::fs::write(
    "generated/vscode-host-adapter.ts",
    generated.adapter_typescript(),
)?;
```

At runtime, the process-backed application binds the same compiled profile to
the finalized server that owns the matching native surface:

```rust
let server = CliMcpServer::builder(registry)
    .surface(vscode_surface)
    .build()?;

let mut hosts = HostProcessRouter::new();
hosts.register(vscode, server)?;

hosts
    .serve_stdio_v1(
        &selected_profile_id,
        &selected_host_adapter_hash,
    )
    .await?;
```

`selected_profile_id` and `selected_host_adapter_hash` are the generated
selector arguments parsed by the application entrypoint. They select one
already-registered immutable pair; they cannot construct a profile, surface,
or server. An in-process deployment replaces the router with
`vscode.bind_in_process(server)?` and gives that bound adapter to its thin
embedding bridge.

The first `process_envelope` argument is a logical binary name, not an executable path. It may use a packaged path-like spelling such as `bin/visible-browser-lab-mcp`, but generated code passes it only to `resolveLaunch`; the returned validated absolute `HostProcessLaunch::executable` is the sole value that reaches `spawn`.

The generated contribution for each tool uses its native public name after prefixing, its derived title and input schema, RFC 0018 invocation presentation, and host guidance. Prompt aliases are explicit because they affect how users attach tools by hand and must remain globally unique.

The constructor names only the host profile. `build(vscode_surface.snapshot())` binds the exact RFC 0015 surface name and hash into the compiled declaration, so the ergonomic path cannot repeat or mistype that identity. A directly constructed or deserialized `HostAdapterProfileDecl` retains its explicit `surface` field; `compile` rejects a name disagreement and always takes the hash from the validated snapshot rather than an authored string. Host artifact identity therefore remains distinct from native surface identity even when an application chooses similar labels.

The extension implements the platform-owned hooks that Twill cannot derive. One resolves opaque invocation context. A process-backed adapter also receives deployment-specific launch resolution and an optional diagnostic sink; generated code retains spawning, selector arguments, bounded I/O, cancellation, decoding, and reaping:

```ts
export interface HostContextProvider {
  resolve(options: unknown): HostInvocationContextV1;
}

export interface HostDiagnosticSink {
  write(chunk: Uint8Array): void | Promise<void>;
}

export interface HostProcessLaunch {
  readonly executable: string;
  readonly workingDirectory: string;
  readonly environment: Readonly<Record<string, string>>;
}

export interface HostProcessRuntime {
  resolveLaunch(logicalName: string): HostProcessLaunch;
  readonly diagnosticSink?: HostDiagnosticSink;
}

export interface HostInProcessRuntime {
  call(
    tool: string,
    input: Readonly<Record<string, unknown>>,
    context: HostInvocationContextV1,
    runtime: HostRuntimeFactsV1,
    token: vscode.CancellationToken,
  ): Promise<HostCallResultV1>;
}

export interface HostRuntimeFactsV1 {
  readonly kind: "vs_code";
  readonly engineVersion?: Readonly<HostVsCodeVersionV1>;
}

export interface HostVsCodeVersionV1 {
  readonly major: number;
  readonly minor: number;
  readonly patch: number;
}

export type HostInvocationContextV1 =
  | {
      readonly kind: "ambient";
      readonly conversationIdentity: Readonly<ConversationIdentity>;
      readonly workspaceRoots?: readonly Readonly<HostWorkspaceRoot>[];
    }
  | {
      readonly kind: "absent";
      readonly workspaceRoots?: readonly Readonly<HostWorkspaceRoot>[];
    }
  | {
      readonly kind: "unsupported";
      readonly reason:
        | "unknown_token_shape"
        | "invalid_session_resource"
        | "invalid_working_directory"
        | "provider_failed";
    };
```

For a process profile, the generated module exports this exact registration surface:

```ts
export function registerGeneratedHostTools(
  extensionContext: vscode.ExtensionContext,
  contextProvider: HostContextProvider,
  runtime: HostProcessRuntime,
): void;
```

An in-process profile emits the same function with `HostInProcessRuntime` in the third position. A generated module contains exactly one concrete signature, never a union or overload that lets activation choose a transport at runtime. It accepts no runtime-fact argument: generated code parses and captures `vscode.version` once into the closed `HostRuntimeFactsV1`.

Registration is synchronous, one-shot, and all-or-nothing. Before calling `vscode.lm.registerTool`, generated code reads the required provider/runtime callable members once, validates that each is callable, captures its original object as the invocation receiver, captures the optional diagnostic sink and its `write` member when present, and constructs the fixed tool implementations. It then registers every compiled contribution into a local disposable set. Success combines that complete set with `vscode.Disposable.from(...)` and appends the single composite to `extensionContext.subscriptions`; the function returns only after that ownership transfer. If member capture or any registration fails, it disposes every contribution already registered in reverse registration order, continues cleanup past any disposal exception, appends nothing, remains eligible for a later retry, and propagates the original activation failure. A call after one success fails before reading either supplied hook or registering another contribution. Neither successful nor failed registration calls the context provider, launch resolver, diagnostic sink, or in-process bridge.

Later invocation uses each captured function with its captured original object as `this`; it does not reread the member from that object. Replacing a method, optional sink, or property after registration therefore cannot change invocation behavior, while a class-based provider or runtime may rely on its ordinary receiver state. This is method-lifetime stability rather than object freezing: application-owned state intentionally read inside the already-captured method remains deployment behavior. Process transport writes the captured runtime fact directly. A conforming in-process hook forwards the received fact unchanged to `HostInProcessAdapter`; as with its result provenance, arbitrary trusted hook code could substitute it, and installed conformance tests cover the supported bridge. A process hook supplies data to the generated launcher but cannot replace its command, selector suffix, bounds, parser, result formatter, or cancellation state machine. An in-process hook delegates to the Rust `HostInProcessAdapter` (or a trusted embedding bridge over it) and returns the same closed `HostCallResultV1` identity envelope that process decoding produces; generated code validates version, host hash, surface hash, outcome inventory, bounds, and VS Code projection identically. When the token has not been cancelled, runtime-hook exceptions, cross-wired identities, and malformed return values are discarded and become the static local `HostContractMismatch` failure. Cancellation follows the distinct settlement contract below and is never relabeled as host/profile drift.

Generated VS Code code has two callback-local snapshot phases because the platform separates `prepareInvocation` from `invoke`. The preparation callback first checks its cancellation token, constructs one private deeply immutable logical snapshot from its `options.input`, resolves the direct/grouped operation, and evaluates RFC 0018 from that snapshot. A direct operation passes the complete snapshot to the command evaluator. A grouped operation retains the selector in the complete snapshot but gives the evaluator a read-only selected-command view with exactly that compiled selector property excluded, matching RFC 0015 dispatch. It invokes no context provider or runtime hook and retains no snapshot, digest, object-identity key, approval record, or other state after returning `PreparedToolInvocation`. This preserves VS Code's side-effect-free preparation rule and its explicit allowance for a preparation call with no later invocation.

The generated return object has one exact mapping. `invocationMessage` is always the prepared invocation string. When the selected trigger produces `PreparedConfirmation`, `confirmationMessages` is present with exactly `{ title: confirmation.title, message: confirmation.message }`; otherwise the property is omitted. Both are plain strings, never `MarkdownString`, and the internal operation id and branch are not projected. A token already canceled at callback entry throws `vscode.CancellationError`. An invalid, over-bound, non-representable, or unroutable preparation input throws `Error("Generated host adapter could not prepare this invocation")` before returning any UI value and without exposing a validator, value, key, path, selector, or size. Invocation has its ordinary typed host-result channel for the corresponding local failures; preparation deliberately has only this fixed redacted host-API failure.

The invocation callback independently checks cancellation, constructs one private deeply immutable logical snapshot from its own `options.input`, and resolves the direct/grouped operation from that snapshot. Only after successful routing does it call `HostContextProvider.resolve(options)` exactly once and synchronously. It validates the returned closed tagged value and constructs a second private deeply immutable context snapshot before applying context gates, counting the complete call, or invoking a runtime hook. The provider is never called for an already-cancelled invocation, an invocation input that failed snapshot construction, or an unroutable direct/grouped input, and preparation never calls it. Provider exceptions, reflection failures, and malformed observed structures discard every partial fact and normalize to `Unsupported { reason: ProviderFailed }`; their values and text never escape. A structurally valid context that makes the complete call exceed `max_call_bytes` instead returns the ordinary static `HostPayloadTooLarge` call-direction result. Rust repeats the closed context validation after either transport boundary. Invocation retains neither its `options` nor any provider-owned reference after those two snapshots exist.

Both callback-local input snapshots use the same generated validator. The accepted source is a top-level object whose recursively observed value is a finite JSON tree. Record objects report exactly `Object.prototype` or `null`; every own key is an enumerable string-named data property. Arrays report exactly `Array.prototype` and contain only the ordinary non-enumerable `length` property plus a dense sequence of own enumerable index data properties. Numbers are finite IEEE-754 binary64 values and normalize through RFC 8785's ECMAScript number spelling, including `-0` becoming `0`; strings contain only well-formed Unicode scalar sequences. A direct Rust in-process value whose JSON number cannot round-trip through binary64 without changing its mathematical value is outside the version-1 host domain and fails `HostContractMismatch` rather than rounding. Other prototypes, accessors, symbol members, inherited enumerable state, non-enumerable record members, extra array members, sparse arrays, unsupported JavaScript values, cycles, and repeated object identities fail through that same static result.

Snapshot construction uses an explicit work stack and the common bounded RFC 8785 logical-envelope writer/counter rather than recursive calls or `JSON.stringify` coercion. Canonical object-member ordering is UTF-16 code-unit order as required by RFC 8785; array order remains significant. Version 1 admits at most 128 nested object/array containers, counting the top-level arguments object as depth 1. Generated TypeScript and both Rust transport paths enforce those same definitions. The traversal copies accepted scalars, arrays, and objects incrementally into framework-owned null-prototype/frozen storage and aborts without retaining a partial snapshot as soon as the input alone cannot fit `max_call_bytes`; invocation applies the complete-envelope cap after context capture and before dispatch. JavaScript cannot identify every transparent `Proxy`, so the contract is the single reflected view: engine-rejected invariants, throwing traps, or a nonconforming view fail closed, while a stable conforming view is copied once and later target or trap mutation has no effect. Context output follows the same reflected-view, depth, and bounded-copy rules. A conforming observed value is copied once, so later provider-owned mutation has no effect. The cap bounds what generated code copies and retains; it cannot prevent VS Code, a proxy trap, or the provider from allocating its source value before returning control.

For VS Code, the provider converts `sessionResource` into the RFC 0013 canonical tuple with issuer `com.microsoft.vscode` and converts a usable file `workingDirectory` into RFC 0009's typed `HostWorkspaceRoot`. The VBL provider retains its released three-way observation boundary with more precise reason ownership. An absent `toolInvocationToken` produces `Absent`. A present token outside the supported object family produces `Unsupported { reason: UnknownTokenShape }`; a missing, malformed, throwing, or empty `sessionResource` produces `InvalidSessionResource`; and a present `workingDirectory` outside the observed URI family produces `InvalidWorkingDirectory`. An unexpected exception while reflecting or copying an otherwise unclassified provider value becomes `ProviderFailed`. Every unsupported result discards all identity and workspace facts.

A supported token with no `workingDirectory` produces `Ambient` with `workspaceRoots` omitted. A URI-shaped non-file working directory, or a file URI with no usable filesystem path, likewise contributes no root while preserving the valid identity; it is an unavailable workspace observation rather than an invalid conversation. Crucially, the provider does not consult editor fallback in any `Ambient` case. This lets a later identity-bearing call supply a usable workspace according to VBL's application binding policy without silently binding the conversation to whichever editor happened to be active during an earlier call.

Only `Absent` may carry VBL's independent editor fallback. The provider selects the workspace folder containing the active text editor when one exists, otherwise the first workspace folder, and otherwise omits `workspaceRoots`. It copies the selected folder's URI into one `HostWorkspaceRoot` with issuer `com.microsoft.vscode`; a non-file folder remains a typed observation for RFC 0009 to reject or ignore according to the selected command rather than being rewritten as a local path. `Unsupported` never consults this fallback. A host that positively reports an empty workspace collection still supplies `[]`. Present-empty and omitted observations retain RFC 0009's blocking versus fall-through distinction.

VBL's process runtime also has one exact migration from the released handwritten launcher. `resolveLaunch` accepts only the profile-authored logical name and snapshots configuration once per invocation. An empty `visibleBrowserLab.binaryPath` selects the packaged platform binary under the captured absolute extension path. A configured value is trimmed and must already be an absolute platform path; relative and bare PATH names are rejected locally, and the setting description changes to say “absolute path.” The returned `workingDirectory` is that invocation's copied absolute `process.cwd()`, preserving the released inherited-cwd behavior as explicit launch data rather than ambient process state.

The returned environment starts from the own string-valued entries of `process.env`, copied into a fresh null-prototype record. In this order, non-empty trimmed VS Code settings then replace `VISIBLE_BROWSER_LAB_STATE_DIR`, `VISIBLE_BROWSER_CDP_ENDPOINT`, `VISIBLE_BROWSER_CDP_PORT`, and `VISIBLE_BROWSER_LAB_CHROME_PATH` from `stateDir`, `cdpEndpoint`, `cdpPort`, and `chromePath`. Empty settings leave the inherited entry unchanged, matching the released overlay behavior; `cdpEndpoint` and `cdpPort` may both be present because VBL's application configuration owns their precedence. The generated adapter's ordinary launch validation then rejects NULs, invalid keys, non-absolute cwd/executable values, or Windows case-insensitive duplicates. No other environment filtering, defaulting, PATH lookup, cwd inheritance, or retry occurs.

The generated adapter sends a private versioned envelope to the application binary or in-process host entrypoint. This expanded example is formatted for readability; process transport emits the same value as whitespace-free RFC 8785 bytes:

```json
{
  "arguments": { "focus": false },
  "context": {
    "conversationIdentity": {
      "id": "vscode-chat://...",
      "issuer": "com.microsoft.vscode",
      "version": 1
    },
    "kind": "ambient",
    "workspaceRoots": [
      {
        "issuer": "com.microsoft.vscode",
        "uri": "file:///workspace/project"
      }
    ]
  },
  "hostAdapterHash": "...",
  "hostProfile": "vbl-vscode-host",
  "runtime": {
    "engineVersion": { "major": 1, "minor": 128, "patch": 0 },
    "kind": "vs_code"
  },
  "surfaceHash": "...",
  "tool": "new_tab",
  "version": 1
}
```

This is an embedding-trusted private host transport, not model input and not an MCP argument stamp. The Twill host entrypoint verifies the envelope version and contract hashes, revalidates the typed context and host runtime facts, constructs RFC 0013's private `InvocationContext` with its canonical identity and RFC 0009 host roots, and dispatches by the surface tool name. Those checks detect drift; they do not authenticate an arbitrary local caller or prove where a supplied runtime fact originated. The registry resolves host roots through the same context-aware planning path used by direct and rmcp execution. The adapter never inserts identity, workspace, runtime version, or approval state into `arguments`.

On success, the Rust host adapter applies the profile's fixed result projection and renders its final compact JSON text before crossing either in-process or process transport. Generated VS Code code emits that validated text in one `LanguageModelTextPart` without parsing the application value. On an ordinary application failure, Rust renders the RFC 0014 code, message, and recovery as bounded text and the generated adapter raises that text as the host's tool error. A configured absent-context rejection instead uses a profile-scoped application declaration whose code and message obey one server-wide RFC 0014 identity and whose recovery is non-callable and host-owned. Framework failures retain their framework code and are never relabeled.

### How Agents And Hosts Should Learn This

Agents see the host's ordinary tool contribution. They never see the host envelope, canonical identity, host adapter hash, or projected-away session handle. Host-specific model descriptions may explain ambient identity and explicit recovery, but they derive from typed guidance declarations and the active binding mode.

Hosts treat the compiled host snapshot as immutable input. They let generated preparation and invocation code snapshot their callback inputs independently, supply opaque-platform context through the narrow invocation-only provider, and render the generated result. They do not parse command strings, strip arguments, normalize identity, choose resource precedence, or interpret application codes independently.

## Reference-Level Explanation

### Host Profile

```rust
pub struct HostAdapterProfile { /* private compiled fields */ }

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct HostAdapterProfileDecl {
    pub id: String,
    pub surface: String,
    pub kind: HostAdapterKind,
    #[serde(default, skip_serializing_if = "HostToolNameProjection::is_identity")]
    pub tool_names: HostToolNameProjection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prompt_references: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "HostGuidanceProjection::is_empty")]
    pub guidance: HostGuidanceProjection,
    pub confirmation: HostConfirmationPolicy,
    #[serde(default, skip_serializing_if = "HostResultProjection::is_default")]
    pub results: HostResultProjection,
    pub unsupported_context: UnsupportedContextPolicy,
    #[serde(default, skip_serializing_if = "AbsentContextPolicy::is_empty")]
    pub absent_context: AbsentContextPolicy,
    pub invocation_limits: HostInvocationLimits,
    pub transport: HostInvocationTransport,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostAdapterKind {
    VsCodeLanguageModelTools {
        engine_floor: VsCodeVersion,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct VsCodeVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl VsCodeVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self;
    pub fn caret_range(&self) -> String;
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct VsCodeEngineRange {
    pub minimum_inclusive: VsCodeVersion,
    pub maximum_inclusive: VsCodeVersion,
}

impl VsCodeEngineRange {
    pub const fn inclusive(
        minimum_inclusive: VsCodeVersion,
        maximum_inclusive: VsCodeVersion,
    ) -> Self;
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub enum HostToolNameProjection {
    #[default]
    Identity,
    Prefix(String),
}

impl HostToolNameProjection {
    pub(crate) fn is_identity(&self) -> bool {
        matches!(self, Self::Identity)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum HostConfirmationTrigger {
    None,
    EffectDefault,
    DeclaredPresentation,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostConfirmationAuthority {
    ServerOnly,
    TrustedVsCodeUi {
        engine_range: VsCodeEngineRange,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct HostConfirmationPolicy {
    pub trigger: HostConfirmationTrigger,
    pub authority: HostConfirmationAuthority,
}

impl HostConfirmationPolicy {
    pub const fn presentation_only(
        trigger: HostConfirmationTrigger,
    ) -> Self;
    pub const fn trusted_vscode_ui(
        trigger: HostConfirmationTrigger,
        engine_range: VsCodeEngineRange,
    ) -> Self;
}

pub struct HostAdapterProfileBuilder { /* private fields */ }

impl HostAdapterProfile {
    pub fn declaration(&self) -> &HostAdapterProfileDecl;
    pub fn snapshot(&self) -> &HostAdapterSnapshot;

    pub fn vscode(
        id: impl Into<String>,
        engine_floor: VsCodeVersion,
    ) -> HostAdapterProfileBuilder;
}

impl HostAdapterProfileDecl {
    pub fn compile(
        self,
        surface: &NativeToolSurfaceSnapshot,
    ) -> Result<HostAdapterProfile>;
}

impl HostAdapterProfileBuilder {
    pub fn tool_name_prefix(self, prefix: impl Into<String>) -> Self;
    pub fn icon(self, icon: impl Into<String>) -> Self;
    pub fn guidance(self, projection: HostGuidanceProjection) -> Self;
    pub fn prompt_reference(
        self,
        operation_id: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self;
    pub fn confirmation(self, policy: HostConfirmationPolicy) -> Self;
    pub fn omit_result_property(
        self,
        operation_id: impl Into<String>,
        property: impl Into<String>,
    ) -> Self;
    pub fn unsupported_context(self, policy: UnsupportedContextPolicy) -> Self;
    pub fn absent_context_rejects(
        self,
        operation_id: impl Into<String>,
        rejection: HostApplicationRejection,
    ) -> Self;
    pub fn invocation_limits(self, limits: HostInvocationLimits) -> Self;
    pub fn in_process(self) -> Self;
    pub fn process_envelope(
        self,
        logical_binary_name: impl Into<String>,
        subcommand: impl IntoIterator<Item = impl AsRef<str>>,
        limits: HostProcessLimits,
    ) -> Self;
    pub fn build(
        self,
        surface: &NativeToolSurfaceSnapshot,
    ) -> Result<HostAdapterProfile>;
}
```

`process_envelope` copies every subcommand item into the owned profile declaration. Literal arrays, borrowed slices such as `&[&str]`, and owned `Vec<String>` values therefore produce the same ordered `Vec<String>` and host hash.

Every profile declaration type above implements `Serialize`, `Deserialize`, and `JsonSchema` with the shown camel-case rules and the corpus additive unknown-field policy. The exact host-kind spelling is `{ "vsCodeLanguageModelTools": { "engineFloor": { "major": 1, "minor": 120, "patch": 0 } } }`; tool-name projection is `"identity"` or `{ "prefix": string }`; confirmation triggers are `"none"`, `"effectDefault"`, or `"declaredPresentation"`; and authorities are `"serverOnly"` or `{ "trustedVsCodeUi": { "engineRange": { "minimumInclusive": version, "maximumInclusive": version } } }`. The complete confirmation member is `{ "trigger": trigger, "authority": authority }`. Normalized profile snapshots contain only these known spellings; host generators do not infer Rust variant names.

`HostAdapterProfileDecl::compile` is the single declaration-to-runtime boundary. The `vscode(...).build(surface)` builder is the ergonomic authoring path and delegates to it, while a declaration loaded from configuration receives identical validation. Neither path accepts a registry or an uncompiled native surface. Compilation reads `NativeToolSurfaceSnapshot::declaration`, `server_instructions`, `task_delivery`, `tools`, and `operations`; it never deserializes or string-indexes `document()` to recover semantics.

Host profile ids use the same exact 1–64 character lowercase-kebab grammar as RFC 0015 surface names: `[a-z0-9]+(?:-[a-z0-9]+)*`. The bound is part of process fallback sizing, and the grammar keeps generated selector arguments distinct from option spellings. Empty, uppercase, underscore, control-bearing, leading-hyphen, and overlong ids fail before snapshot or artifact generation.

Version-1 host adapters submit one ordinary invocation and expose no MCP task object or polling API. Profile compilation therefore accepts native tools with `TaskSupportSpec::Forbidden` or `Optional` and rejects any direct or grouped tool marked `Required`. `Required` forbids ordinary delivery; its rejection is a surface-construction fact rather than a runtime host refusal.

After generated preflight and Rust route selection, the private host entrypoint explicitly selects RFC 0020's immediate ordinary execution path. That selection is an adapter-owned input to dispatch, not a default inferred from an absent task member, extension capability, envelope property, or context value. It supplies neither a legacy task request nor Tasks Extension capability, and it never calls an installed task store, access provider, runner, record codec, or cancellation operation. A finalized server may retain RFC 0020 runtime sidecars for its MCP surface, but calls through this host profile leave them untouched. An `Optional` operation therefore produces the same validated command outcome as its ordinary MCP execution; RFC 0019 then performs the host-specific result projection directly into `HostCallResultV1` without constructing an MCP task result or retained task state.

The compiled delivery profile remains nested in the native and host snapshots, participates in their hashes and the invocation fingerprint's serving identity, and is never reinterpreted by generated code. Two otherwise equivalent host profiles compiled over distinct delivery surfaces therefore retain distinct identities even when their immediate application text matches. The adapter honors the command's task contract by rejecting `Required` at compilation and selecting the permitted ordinary path for `Forbidden` and `Optional`; waiting for a host result grants no exemption. A future task-aware host kind must consume RFC 0020's typed delivery view and declare how host cancellation, progress, polling, retention, and terminal results map to that exact profile.

`HostAdapterProfile::vscode` requires the platform engine floor whose opaque-token and contribution APIs the generated adapter targets. `VsCodeVersion::new(1, 120, 0)` serializes as the typed `{major, minor, patch}` fact and generates the manifest range `^1.120.0`; raw semver-range strings are not an alternate authoring path in version 1. The initial generator rejects a floor below 1.120.0 or outside VS Code's 1.x line because its private `sessionResource`/`workingDirectory` provenance is established only for that contract family. Supporting another engine line requires a reviewed host-kind version rather than silently reusing the extractor. The constructor otherwise initializes the VS Code kind, identity tool-name projection, no icon, no prompt aliases, empty host-guidance segments, the fixed compact-JSON/application-error/framework-error result dialects, and no absent-context rejections. Those are semantic empty or single-choice defaults and normalize identically to an explicit declaration. Confirmation policy, unsupported-context policy, common invocation limits, and invocation transport have no builder defaults: each changes host behavior or trust, so `build` rejects an omitted choice. `in_process` and `process_envelope` are mutually exclusive explicit transport choices. `.guidance(...)` replaces the complete host-owned guidance projection; the RFC 0011 and native-surface portions remain derived and cannot be overridden through this builder.

The profile declaration's serde defaults encode the same boundary. Tool-name identity, absent icon, empty prompt/guidance/result-omission/absent-context collections, and the three fixed result dialects may be omitted and reserialize in their omitted form after normalization. Confirmation policy, unsupported-context policy, invocation limits, and transport remain required deserialization fields because omission would choose behavior, resource bounds, or trust. A direct declaration therefore has the same semantic-default spelling as the ergonomic builder without making policy-bearing fields optional.

Every scalar profile-builder method (`tool_name_prefix`, `icon`, `guidance`, `confirmation`, `unsupported_context`, `invocation_limits`, and the transport choice) is a single authored assignment. A fresh semantic default does not count, so one explicit identity/empty spelling normalizes with omission; a second assignment records a build error even when equal. `prompt_reference`, `omit_result_property`, and `absent_context_rejects` are keyed additions and reject a repeated key rather than replacing it. The nested policy builders follow the same rule for their one recovery slot, while their keyed `allow` and `reason` additions reject repeats as described below. This prevents fluent call order from becoming hidden profile authority.

`None` requests RFC 0018's `Omit` mode and returns only the Twill-authored invocation message. `EffectDefault` adds Twill-authored `confirmationMessages` for the standard effects RFC 0003's default authorizer classifies as require-confirmation (`Write`, `Delete`, `Exec`, and `Network`), not for `Pure`, `Read`, or unknown custom effects. It uses the selected member's effect for a grouped call after selector resolution and requests `DeclaredOrSurfaceDefault`, so undeclared commands use the exact generic copy stored in the native surface snapshot. `DeclaredPresentation` requests `DeclaredOnly` and adds `confirmationMessages` only for commands with RFC 0018 confirmation copy. Omitting that generated property makes no broader claim about UI a host version may add around extension tools; the compiled trigger records only whether Twill supplied its reviewed confirmation title and message. These are deterministic host-presentation rules and do not inspect or predict a custom server authorizer.

`HostConfirmationAuthority::ServerOnly` is the default trust posture authors select explicitly through `HostConfirmationPolicy::presentation_only`. Generated host UI is presentation only, the finalized server's base authorizer remains authoritative, and every `RequireConfirmation` decision follows the compiled RFC 0015 native confirmation route. The earlier generated UI never counts as that bridge decision.

VS Code's preparation options expose input but no `toolInvocationToken` or other correlation identifier, and the API permits preparation without invocation. Generated code therefore cannot prove that a later `invoke` callback corresponds to one earlier `prepareInvocation` callback without accepting the platform as an authority, and it retains no ambiguous approval state.

`HostConfirmationAuthority::TrustedVsCodeUi` is that explicit host-only authority. Its inclusive engine range must lie within the profile's generated `^engine-floor` manifest family and contain at least one version. The generated adapter parses the public `vscode.version` constant as exactly three unsigned decimal components; an unrecognized spelling becomes an absent engine observation rather than raw text. It carries only the parsed version or absence through the closed private runtime facts described below. The Rust host entrypoint checks that fact against the hash-covered range and evaluates the same compiled trigger over the invocation callback's accepted logical snapshot. The finalized server's base authorizer still runs after planning. `Allow` remains allow, `Deny` remains deny, and `RequireConfirmation` becomes allow only when the runtime version is inside the trusted range and the compiled trigger produced a confirmation for that exact selected operation and snapshot. Every other `RequireConfirmation` follows RFC 0015's compiled bridge or unavailable route. Registry hard-policy denial always precedes this composition and can never be widened.

Profile compilation rejects `TrustedVsCodeUi` for another host kind, with `HostConfirmationTrigger::None`, with reversed or out-of-family endpoints, or when no exposed operation can produce confirmation under the selected trigger. `DeclaredPresentation` remains operation-specific: a mapped command without declared copy produces no Twill-authored confirmation payload and therefore receives no trusted satisfaction. `EffectDefault` remains effect-specific: a custom base authorizer requiring confirmation for a `Pure`, `Read`, or unknown custom-effect plan receives no trusted satisfaction merely because another operation in the profile uses a standard confirming effect. Platform-generic UI outside this payload is never inferred as Twill approval. This closed composition prevents an embedding from pairing a broad permissive authorizer with a narrower UI trigger.

For this contract, value equivalence means that the preparation and invocation callbacks' complete accepted public-tool argument snapshots have byte-identical RFC 8785 canonical JSON; a grouped snapshot includes its selector even though the selected command evaluator and binder both exclude that surface-owned property. Object insertion order is irrelevant, while selector choice, array order, and every normalized scalar value remain significant. The trusted range asserts that VS Code showed the generated Twill confirmation whenever the invocation-side trigger matches and supplied a value-equivalent later invocation. Deployment configuration, release notes, and installed-host acceptance name and test both endpoints before that range is enabled. Widening either endpoint changes the host hash and requires renewed evidence. The envelope carries no `approved` flag, a generic MCP caller cannot supply host runtime facts or inherit this entrypoint policy, and context is extracted only from the invocation callback after host UI, so the claim binds model arguments but does not pretend the preparation callback observed identity or workspace.

### Guidance Projection

Host guidance is declarative text layered over RFC 0011 catalog guidance. The server preamble, `use_when`, alternatives, and fallback edges flow into host server/tool descriptions before this profile adds host facts:

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub struct HostGuidanceProjection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_prefix: Vec<HostGuidanceSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_suffix: Vec<HostGuidanceSegment>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operation_suffixes: BTreeMap<String, Vec<HostGuidanceSegment>>,
}

impl HostGuidanceProjection {
    pub(crate) fn is_empty(&self) -> bool {
        self.server_prefix.is_empty()
            && self.tool_suffix.is_empty()
            && self.operation_suffixes.is_empty()
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostGuidanceSegment {
    Text(String),
    Operation { operation_id: String },
    ResourceCarrier { resource: String },
}
```

The common suffix explains host facts such as “conversation identity is supplied out of band.” Operation-specific suffixes handle real host exceptions such as the explicit `session start` recovery entrypoint. Structured operation segments render the direct tool name or grouped tool plus selector for the active surface. Resource-carrier segments render the carrier and its active RFC 0016 binding guidance. General operation-selection advice remains in RFC 0011 rather than being repeated here.

Registration validates every reference and rejects a raw text segment containing an exact known operation id, active tool name, selector-qualified call spelling, or resource carrier as a token-delimited substring under a fixed ASCII identifier tokenizer. This lets VBL say “Never call [session start] or invent [session carrier]” while keeping both names derived. Ordinary prose is not guessed semantically; the lint only catches the finite structural vocabulary already known to the profile.

The generated model description is command/surface description plus common suffix plus optional operation suffix. Server instructions combine the active surface instructions with `server_prefix`. All text participates in the host adapter hash.

### Prompt References And Names

Name prefixing is a pure injective mapping over native tool names. `Identity` is the sole no-prefix spelling; `Prefix("")` and control-bearing prefixes fail rather than creating a second canonical default. Every generated name must satisfy host grammar and remain unique after prefixing. Prompt-reference aliases are optional, globally unique within the profile, and are authored by catalog operation id so renames flow through the active surface mapping. Version 1 accepts only operations mapped to direct tools. A grouped member still requires its selector, while one VS Code contribution has only one `toolReferenceName`; accepting an operation-level alias there would either lose the selector or ambiguously alias every member. Omitted and grouped operations therefore fail profile compilation. Prompt references do not create tool aliases, prefill arguments, or change dispatch.

VS Code contribution projection fills `engines.vscode` from the typed engine floor and fills each tool's `name`, `displayName`, `userDescription`, `modelDescription`, `icon`, `inputSchema`, `canBeReferencedInPrompt`, and `toolReferenceName` from the profile and native snapshot. It never reconstructs an argument schema. The engine floor participates in the host snapshot/hash and generated-source compatibility constants because it is part of the context-extraction and host-API trust contract, not release packaging decoration.

### Model-Visible Result Projection

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub struct HostResultProjection {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub omit_top_level_properties: BTreeMap<String, BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "HostSuccessDialect::is_default")]
    pub success: HostSuccessDialect,
    #[serde(default, skip_serializing_if = "HostApplicationErrorDialect::is_default")]
    pub application_error: HostApplicationErrorDialect,
    #[serde(default, skip_serializing_if = "HostFrameworkErrorDialect::is_default")]
    pub framework_error: HostFrameworkErrorDialect,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub enum HostSuccessDialect {
    #[default]
    CompactJsonText,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub enum HostApplicationErrorDialect {
    #[default]
    ThrowBoundedText,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub enum HostFrameworkErrorDialect {
    #[default]
    ThrowBoundedText,
}

impl HostResultProjection {
    pub(crate) fn is_default(&self) -> bool {
        self.omit_top_level_properties.is_empty()
            && self.success.is_default()
            && self.application_error.is_default()
            && self.framework_error.is_default()
    }
}

impl HostSuccessDialect {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Self::CompactJsonText)
    }
}

impl HostApplicationErrorDialect {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Self::ThrowBoundedText)
    }
}

impl HostFrameworkErrorDialect {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Self::ThrowBoundedText)
    }
}
```

The map keys are catalog operation ids. In the initial version, each target must map to a direct native tool; grouped, nested, and conditional result omission are rejected. Registration requires each omitted property to exist in that operation's object success schema. The compiler removes it from the projected host result schema and required set, then recomputes local-definition reachability and removes only definitions made unreachable by those declared property omissions. Remaining definition names and schemas stay unchanged. The runtime adapter removes the same top-level property from a successful value.

This projection is fixed at host profile construction, hash-covered, and tested against the result contract. It cannot inspect values or conditionally reveal a field. The registry and native MCP surface retain the full application value; only the generated host result changes. For VBL's VS Code profile, `session start` omits `agent_session_id`, so no explicit or ambient session handle reaches model-visible chat results.

The VBL result audit finds `agent_session_id` as a top-level declared success property only on `start_session`; nested properties with the same spelling can be page-controlled application data and remain untouched. Operation-scoped top-level omission therefore replaces the extension's broad object filter with a narrower schema-proven rule.

`CompactJsonText` serializes the projected application value once in Rust and returns one final host text part. The private host outcome carries that text rather than the JSON value, so generated JavaScript never reparses application numbers or reorders object properties. RFC 0012 resource links remain native-MCP content components and never enter `HostCallOutcomeV1::Success`; the host profile consumes their declaration edges for validation but does not expose resource URIs or references through a text-only result.

Application `ThrowBoundedText` joins these non-empty parts with `. ` in this exact order:

1. `<native-tool-name> failed with <application-code>`;
2. the validated application message;
3. `Details: <compact-json>` when validated details are not the empty object;
4. `Recovery: <value>` when the runtime selection is non-empty.

`<native-tool-name>` is the active RFC 0015 direct or grouped tool name before RFC 0019 host prefixing. It is never the catalog operation id, grouped member selector, prefixed contribution name, or display title. This matches VBL's released formatter, which strips `visible_browser_lab_` and reports `new_tab` or the selected grouped tool name. Under RFC 0015's canonical dialect, `<value>` is the compact JSON serialization of the complete ordered `NativeApplicationRecovery` array. Under `FlatSingleRecovery`, it is the one validated flattened tool/action token without JSON quotes, preserving VBL's established text. The flat dialect's injectivity check lets the compiled host snapshot recover whether that token denotes a callable tool or a non-callable action when rewriting an absent-context recovery. Details and canonical recovery JSON are serialized from the already validated native body; the host renderer never reparses an application value or accepts an application-authored display fragment. An empty details object or empty recovery selection omits its whole part rather than printing `{}`, `[]`, or `undefined`.

Framework `ThrowBoundedText` uses the same first two-part shape with RFC 0002's redacted framework code and message, followed only by framework-owned safe steering that the host dialect explicitly supports. After a native route resolves, it uses that compiled native tool name. A Rust envelope failure that occurs before valid route resolution uses the fixed subject `generated host call`; it never echoes the untrusted envelope `tool`. A generated-adapter-local failure already knows the compiled tool whose contribution was invoked and uses that native name even when the child result is missing or malformed. `UnsupportedHost` occurs after route resolution and uses the native name plus its profile-declared recovery action. The complete rendered error is limited to 1,024 Unicode scalar values; static declaration validation reserves room for the fixed subject or longest native name and the code, and any final truncation retains a safely escaped prefix plus `…`. Rendering emits no raw line breaks or control characters and never includes raw validator errors or private context. Truncation affects only this text-host projection; the native structured application body remains complete.

For the VBL profile above, those rules deliberately reproduce the released v0.4.9 strings rather than merely their meaning. An unsupported `new_tab` call renders:

```text
new_tab failed with unsupported_host. VS Code did not expose a compatible chat session resource; Visible Browser Lab requires VS Code 1.120 or newer with the supported invocation-token shape. Recovery: update and reload VS Code, or use the explicit MCP/CLI surface
```

An absent-context `start_session` call renders:

```text
start_session failed with session_required. Global VS Code tool invocations have no conversation identity and do not expose explicit session handles. Recovery: invoke Visible Browser Lab from a supported VS Code chat, or use the explicit MCP/CLI surface
```

The declarations store only the message and recovery summary; the renderer owns the tool name, failure phrase, code, separators, and `Recovery:` label. This division prevents a profile from embedding a second formatter inside its prose.

Text-only generated hosts categorically omit RFC 0012 resource links in the initial contract. A resource link is an ownership-bearing protocol component, not display text, and flattening it into JSON would bypass both the host result schema and operation-scoped omission rules. A future host with a native typed-resource result API may add a separately declared, schema-checked projection; arbitrary text embedding is not an option on this profile.

### Private Host Invocation Context

```rust
struct ConversationIdentityTransportSchemaV1;

impl JsonSchema for ConversationIdentityTransportSchemaV1 {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ConversationIdentityTransportV1".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "version": { "type": "integer", "const": 1 },
                "issuer": {
                    "type": "string",
                    "pattern": "^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*$"
                },
                "id": { "type": "string", "minLength": 1 }
            },
            "required": ["version", "issuer", "id"],
            "additionalProperties": false
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
)]
pub enum HostInvocationContextV1 {
    Ambient {
        #[schemars(with = "ConversationIdentityTransportSchemaV1")]
        conversation_identity: ConversationIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_roots: Option<HostWorkspaceRootsObservation>,
    },
    Absent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_roots: Option<HostWorkspaceRootsObservation>,
    },
    Unsupported { reason: HostContextReason },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HostContextReason {
    UnknownTokenShape,
    InvalidSessionResource,
    InvalidWorkingDirectory,
    ProviderFailed,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedContextPolicy {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_operations: BTreeSet<String>,
    pub reasons: BTreeMap<HostContextReason, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<HostRecoveryAction>,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema, Default,
)]
#[serde(rename_all = "camelCase")]
pub struct AbsentContextPolicy {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rejections: BTreeMap<String, HostApplicationRejection>,
}

impl AbsentContextPolicy {
    pub(crate) fn is_empty(&self) -> bool {
        self.rejections.is_empty()
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct HostRecoveryAction {
    pub code: String,
    pub summary: String,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct HostApplicationRejection {
    pub application_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<HostRecoveryAction>,
}

impl UnsupportedContextPolicy {
    pub fn new() -> Self;
    pub fn allow(self, operation_id: impl Into<String>) -> Self;
    pub fn reason(
        self,
        reason: HostContextReason,
        summary: impl Into<String>,
    ) -> Self;
    pub fn recover_by(
        self,
        action_code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self;
}

impl HostApplicationRejection {
    pub fn new(application_code: impl Into<String>) -> Self;
    pub fn runtime_message(self, message: impl Into<String>) -> Self;
    pub fn recover_by(
        self,
        action_code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self;
}
```

The TypeScript provider mirrors these stable variants and reason codes. It may inspect opaque host objects because Twill cannot. It may only construct public canonical context types; arbitrary metadata maps are not accepted. Each `workspaceRoots` member uses RFC 0009's exact closed `{ issuer, name?, uri }` wire form; generated TypeScript types and the Rust JSON Schema come from that shared contract rather than a host-local interface. The invocation callback calls the captured provider once, catches an exception, and validates and snapshots its returned tagged value before any runtime await; preparation never calls it, and provider-owned objects are never retained or reread. An exception or invalid reflected result discards every partial fact and produces `Unsupported { reason: ProviderFailed }`; neither can escape through the host error formatter or process envelope. Rust revalidates the conversation identity and host root issuer/name/URI before planning, then moves accepted facts into the non-serializing `InvocationContext`. `Ambient` always contains one identity and `Absent` contains none. In either variant, omitted `workspaceRoots` creates no host workspace observation and permits RFC 0009 lower-authority fall-through; a present array creates `HostWorkspaceRootsObservation`, including a present empty array that deliberately blocks fall-through. `Unsupported` contains no partially trusted facts. The immutable tagged snapshot crosses the process envelope unchanged, so unsupported, absent, and empty-observation policy are enforced again in Rust rather than trusted solely to generated TypeScript control flow.

`UnsupportedContextPolicy::new` is exactly `Self::default()` and begins with no allowed operations, reason summaries, or recovery. The consuming builder methods add one fact at a time; duplicate operations, duplicate reasons, and conflicting repeated action codes fail profile compilation. Every `HostContextReason` must have exactly one summary before compilation succeeds. `HostApplicationRejection::new` starts without a runtime message or recovery. `.runtime_message` installs the one optional RFC 0014 bounded-runtime value, and `.recover_by` installs the one optional non-callable host action. Repeating either assignment fails profile compilation even when equal. These methods are the ergonomic counterparts to the fully serializable public declaration fields and do not bypass profile validation.

`UnsupportedContextPolicy` lists application operations allowed without ambient context, such as VBL's mapped `help` command. For an allowed operation, Twill plans with no identity or host roots; it does not reinterpret unsupported context as a partially valid observation. A generated `FrameworkHelpProjection::Tool` is always allowed and discards invocation context because it does not dispatch a command. Other unsupported calls fail with framework `ErrorCode::UnsupportedHost` before dispatch. The public message is the static summary declared for the observed reason, and optional recovery is the profile's static non-callable host action. Every `HostContextReason` must have a non-empty bounded summary; reason and recovery declarations are hash-covered and cannot include token data. `Absent` is not malformed: commands may proceed without identity, retain independently valid host roots, and receive their declared missing-binding behavior.

`HostRecoveryAction.code` uses lower snake case, and repeated codes within one profile must have identical summaries. Host actions are presentation and never resolve through the native operation map; any callable recovery must remain an RFC 0014 operation edge.

```rust
pub enum ErrorCode {
    // ...existing codes...
    HostContractMismatch,
    HostPayloadTooLarge,
    UnsupportedHost,
}
```

`HostContextReason` and the gate's internal error stay in `mcp-twill-host`; core `mcp_twill::FrameworkError` does not depend on host-profile types. A rejected gate constructs a redacted RFC 0002 `ErrorBody` with `ResponseStatus::Failed` and `ErrorCode::UnsupportedHost` directly in the host adapter before registry planning. Hash, profile, or native-tool disagreement in a syntactically valid version-1 envelope uses `ResponseStatus::Failed` and `HostContractMismatch`; malformed typed context uses the existing `ResponseStatus::InvalidInput` and `InvalidRequestContext` mapping. `HostPayloadTooLarge` is the stable generated-transport family for a preflight call-envelope or streamed result limit: an oversized caller-supplied call is `InvalidInput`, while an oversized generated result is `Failed`. It reports only direction (`call` or `result`) plus the configured public limit, never actual bytes or content. These bodies contain only stable profile/contract facts and optional declared host recovery, not the opaque token, provider exception, application arguments, or mismatching supplied hashes. Host-adapter events and application-owned diagnostics may record the profile, direction, public code, and configured limit; they never retain the rejected envelope, actual size, decoder text, or provider value. Once planning begins, ordinary outer `FrameworkError` values retain their existing core ownership and conversion path.

`AbsentContextPolicy` may reject a named operation with a profile-scoped application use whose code resolves to one existing server-wide RFC 0014 `ApplicationErrorDecl` present in the consumed native snapshot and whose details schema accepts the gate's empty object. `HostApplicationRejection` is the serialized authoring type for that use. Presence means at least one exposed operation already uses that identity; the host compiler never consults an omitted catalog operation or the registry behind its snapshot input. The code need not be in the target command's `ApplicationErrorUse`: adding it there would incorrectly advertise a host-only outcome on native MCP. The compiled rejection belongs to the host snapshot and constructs `HostCallOutcomeV1::ApplicationError` directly before registry planning; it never constructs `CommandExecutionOutcome::ApplicationError`, mutates the command result contract, or reaches another profile.

This is distinct from RFC 0016's `missing_as` mapping. That declaration-only emitter can construct `CommandExecutionOutcome::ApplicationError` only because every reachable required consumer already owns the selected code, static message/details policy, and full recovery in its joined command spec; the surface contributes only the structural missing-binding condition. A host rejection instead owns an additional profile-local use and therefore remains entirely outside the command outcome family.

The profile-scoped message obeys the referenced RFC 0014 identity without restating its static text. Under `ApplicationMessageDecl::DeclarationSummary`, `runtime_message` must be absent and the compiler copies the identity summary into the private compiled rejection. Under `RuntimeBounded`, `runtime_message` is required, non-empty, and must fit the declared bound under RFC 0014's exact escaping/scalar rules. Profile compilation materializes that already validated message and never calls an exception's `Display`. The gate always supplies the empty details object, so a schema that rejects it fails compilation. Its optional `HostRecoveryAction` is a profile-owned non-callable projection, not a command recovery declaration, and is serialized and hash-covered with the use. A callable recovery remains impossible before planning.

This narrow profile use is valid only when the profile makes the operation's successful resource establishment unusable: the command has an RFC 0012 grant edge for a resource, the active RFC 0016 surface omits that resource's explicit carrier, and the fixed host result projection omits the top-level success property named by the carrier. Registration proves all three facts. VBL's VS Code profile rejects `session start` as `session_required` with host-specific guidance because the ambient-only host removes both directions in which `session start`'s explicit handle could be used; its differing host message therefore requires the server-wide `session_required` identity to use `RuntimeBounded` with a sufficient bound. A context gate cannot invent an application code or details, synthesize a callable recovery, rewrite a framework failure, inspect arguments, or bypass the identity's message policy.

The initial host model retains this single application-owned gate because the three structural proofs establish that the host has removed the command's usable success contract, and VBL's released surface already names the resulting condition `session_required`. Returning a framework-owned host error would preserve transport ownership by changing the application's established recovery protocol. The explicit profile-scoped use preserves that protocol without claiming the command produced the outcome. Any absent-context rejection lacking the identity/message/details checks or the grant, omitted-carrier input, and omitted-carrier result proofs remains framework-owned; the exception cannot generalize into arbitrary host relabeling.

While context is absent, application recovery projection also knows that a rejected operation is not callable. A callable recovery targeting that operation becomes the rejection's non-callable host action. When the emitted application code equals the target rejection's `application_code`, the host uses the complete rejection message and recovery so it does not first tell the agent to call a tool the same profile will reject. For another code, it retains that error's code and message and replaces only the dead callable recovery with the host action. Registration rejects such a recovery edge unless the target rejection declares an action. Ambient calls and native MCP keep the original RFC 0014 recovery unchanged.

The VBL editor fallback is therefore a host observation only for `Absent`: active editor workspace, first workspace folder, then omission. It never augments an identity-bearing or unsupported token and never becomes a process current-directory heuristic inside the handler. This provider policy and the launch resolver remain handwritten trusted host glue, excluded from host snapshot identity; generated fixtures and installed-artifact evidence bind their behavior.

### Versioned Invocation Transport

```rust
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCallEnvelopeV1 {
    pub version: u32,
    pub host_profile: String,
    pub host_adapter_hash: String,
    pub surface_hash: String,
    pub tool: String,
    pub arguments: BTreeMap<String, serde_json::Value>,
    pub context: HostInvocationContextV1,
    pub runtime: HostRuntimeFactsV1,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
)]
pub enum HostRuntimeFactsV1 {
    VsCode {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        engine_version: Option<HostVsCodeVersionV1>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostVsCodeVersionV1 {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCallResultV1 {
    pub version: u32,
    pub host_adapter_hash: String,
    pub surface_hash: String,
    pub outcome: HostCallOutcomeV1,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
)]
pub enum HostCallOutcomeV1 {
    Success { text: String },
    ApplicationError { code: String, text: String },
    FrameworkError { code: String, text: String },
}
```

`HostInvocationContextV1`, `HostContextReason`, `HostRuntimeFactsV1`, `HostVsCodeVersionV1`, `HostCallEnvelopeV1`, `HostCallResultV1`, and `HostCallOutcomeV1` implement `Serialize`, `Deserialize`, and `JsonSchema` with exactly the attributes shown. RFC 0013 deliberately gives `ConversationIdentity` no `JsonSchema` implementation, so this private transport uses the field-local `ConversationIdentityTransportSchemaV1` adapter rather than broadening that reusable type's public projection capabilities. The adapter describes only the canonical version-1 wire object and exact issuer/id grammar; runtime deserialization still invokes `ConversationIdentity`'s validating implementation. Their generated schemas are closed at every struct and tagged-variant object (`additionalProperties: false`), preserve the omitted-versus-present workspace and engine-version observations, and use the exact snake-case tags/reasons plus camel-case fields. The generated TypeScript transport types come from these schemas and checked constants rather than a parallel handwritten interface. The schema adapter and closed host-version type are private to `mcp-twill-host`, never appear in catalog or model-visible schema APIs, and carry no identity value. Public profile declarations continue to use additive `VsCodeVersion`; the compiler copies validated components into the closed transport constant without reusing its serde shape. Profile declaration types are not accepted as invocation envelopes.

`HostRuntimeFactsV1` is generated framework data, separate from the handwritten `HostContextProvider`. A VS Code profile accepts only the `vs_code` variant. An exact stable `major.minor.patch` value is compared with the profile's inclusive trust range; omission or a value outside that range is valid host transport but supplies no host-UI approval authority. A mismatched host-kind variant or malformed closed object is `HostContractMismatch`. The version is never copied into `InvocationContext`, plans, fingerprints, responses, events, framework logs, or application values. Process transport accepts this observation as an assertion from the configured launcher boundary because that boundary already has private-context and, for a trusted profile, narrow approval authority; parsing and range validation do not prove the assertion's provenance. The public in-process call parameter is likewise an explicit trusted-host/test injection point, and the supported generated bridge must forward its captured fact unchanged. Neither hash is treated as authentication for an arbitrary local caller.

Those serde implementations define the typed data model and schema fixtures; arbitrary `serde_json::to_vec` output is not the version-1 process codec. Only the bounded RFC 8785 writer used by `HostProcessRouter`, the generated adapter, and their contract helpers produces conforming outer-envelope bytes. The process entrypoint rejects another spelling even when ordinary serde could deserialize it. In-process callers use the typed path and the same writer in count mode, so no public alternate byte codec becomes transport authority.

Both outcome code fields are transport strings validated against the compiled host snapshot rather than Rust enums serialized directly. Application codes use the snapshot's reachable RFC 0014 inventory. Framework codes use the snapshot's complete version-1 framework inventory described below. The Rust entrypoint obtains a framework wire code only through the response-profile mapping and then checks that exact string before constructing the outcome. This prevents adding a future core `ErrorCode` variant from silently extending the closed version-1 host transport through a derived serde implementation.

The initial process transport writes exactly one envelope to stdin and reads exactly one result from stdout. Logs go to stderr. Before spawn, the generated adapter encodes the already-counted logical call through the common bounded RFC 8785 writer and rejects an envelope larger than `max_call_bytes` with static `HostPayloadTooLarge`; it never constructs an over-limit JSON string merely to measure it. The shared snapshot validator accepts only the exact plain-data JSON tree defined above and validates every JavaScript string and object key as a Unicode scalar sequence: a valid UTF-16 surrogate pair is accepted as one scalar, while an unpaired high or low surrogate fails locally as static `HostContractMismatch` without quoting the key, value, or path. The process writer therefore never relies on JavaScript's well-formed-JSON lone-surrogate escaping to create a string Rust cannot represent. Its number formatter and UTF-16 member ordering follow the same RFC 8785 algorithm established by the corpus snapshot fixtures, not JavaScript `JSON.stringify` or Rust `serde_json` defaults. The in-process generated path uses the same snapshot and counts the same canonical `HostCallEnvelopeV1` bytes before calling its runtime hook; the bound Rust adapter repeats the authoritative count, number-domain validation, and 128-container depth check from its typed inputs. Neither path silently applies JavaScript's array-null/object-omission coercions or numeric rounding to an invalid host input. The generated launch arguments identify the exact compiled host profile and adapter hash before stdin is opened. The Rust process router verifies that public identity against its registered immutable profile, obtains that profile's common invocation limits, and only then reads stdin incrementally up to `max_call_bytes`. A bypassed adapter therefore cannot select a looser limit from envelope data or make the entrypoint allocate without bound; it also receives the same over-depth and number-domain rejection. Crossing the cap exits nonzero with no stdout.

Both process decoders use duplicate-detecting JSON tokenization rather than a last-key-wins map parser. After JSON string unescaping, every object member name must be unique at every depth. On the Rust call side, one unique top-level `version: 1` lets a duplicate inside the `context` subtree become redacted `InvalidRequestContext`; a duplicate anywhere else in the call envelope, including inside `arguments`, becomes `HostContractMismatch`. A duplicated call-envelope `version` supplies no unique version authority and follows the child process's nonzero/no-stdout unsupported-envelope path. On the generated result side, every duplicate—including `version`—becomes the same static local `HostContractMismatch` used for other invalid wrapper output. After duplicate/version handling, both sides require the complete outer envelope to be its exact RFC 8785 byte sequence: insignificant whitespace, noncanonical member order, escape spelling, or number spelling is `HostContractMismatch`. The result's `success.text` remains an opaque string at this outer step; its separate scanner tracks decoded member names per nested application object and rejects duplicate or escape-equivalent keys without constructing an application value or converting a number. Rust-produced `serde_json::Value` application text already satisfies that duplicate invariant; the check closes the hostile or drifted wrapper boundary.

After bounded input collection, decoding is staged: Rust first parses a JSON object and reads only `version`; version 1 then enters the closed version-1 decoder. A syntactically valid object declaring version 1 always produces one `HostCallResultV1` and exit status zero, including missing, wrong-kind, or unknown envelope fields, hash, profile, tool, context-validation, planning, application, and framework failures. The profile and hash in the envelope must equal the already selected launch identity; they cannot choose the runtime profile. A malformed `context` subtree maps to redacted `InvalidRequestContext`; every other closed-envelope shape mismatch maps to `HostContractMismatch`. Neither error quotes a field value or serde diagnostic. This keeps typed context under the request-context owner while treating host/profile/transport drift as an artifact contract failure.

Every `HostCallResultV1` contains the runtime profile's required expected host and surface hashes; it never echoes caller-supplied mismatching hashes as though those values were authoritative. The launch profile and hash already resolved before stdin was read, so there is no version-1 result state in which those fields are absent. An unknown or stale launch identity exits nonzero before input and emits no stdout. After launch selection, envelope hash/profile mismatch, unknown tool, and non-context closed-shape drift return redacted `HostContractMismatch` outcomes with the authoritative hashes before planning. Unreadable JSON, a non-object envelope, an unsupported version, or failure to initialize enough runtime to construct the version-1 result exits nonzero with no partial stdout envelope and only a bounded safe stderr diagnostic. Tool resolution uses the profile's native surface mapping and then RFC 0015 operation-id dispatch; no command string is synthesized.

`HostCallResultV1` and its tagged outcome are closed too. Success contains only final compact JSON text. Application and framework errors contain their family-owned code plus final bounded host text. An application variant accepts only codes compiled into the host snapshot from exposed RFC 0014 identities or a validated absent-context gate; a framework variant accepts only the snapshot's version-1 framework code inventory and cannot carry `ApplicationError`. After decoding or receiving the outer result, generated code validates success text with a lexical JSON scanner that requires exactly one value, no insignificant whitespace, and no trailing data but never constructs an application object or converts a number lexeme. Application/framework text must be non-empty, control-free, and within the profile's 1,024-scalar error bound. A violation becomes the same static local `HostContractMismatch`; neither invalid text nor its position is reported. Rust process and in-process adapters pass the completed result through the common bounded RFC 8785 encoder using `max_result_bytes`. If an otherwise valid success or application error crosses the cap, they discard the partial encoding and emit the fitting static `HostPayloadTooLarge` framework result for direction `result`; profile construction proves that every transport-owned fallback envelope itself fits. The generated in-process adapter repeats the canonical count before projection. The generated process adapter independently reads stdout incrementally up to the same limit. A buggy or hostile wrapper that crosses it is terminated and reaped, and the adapter produces that same static local error. At EOF the process adapter requires exactly one canonical outer JSON value with no leading or trailing byte. A missing, wrong-kind, unknown, concatenated, whitespace-padded, or otherwise noncanonical result becomes the same static local failure on every generated host: framework code `host_contract_mismatch` and message `Generated host adapter received an invalid result envelope`. Raw stdout and decoder text never become a model-visible tool error. Successful validation performs no application-value parse or semantic formatting: it places success text into the host text part and raises application/framework text through the host error channel.

The Rust host entrypoint converts `CommandExecutionOutcome` or outer `FrameworkError` into `HostCallOutcomeV1` after applying the fixed result projection, active native application-error dialect, framework redaction, and final text rendering. It checks the selected family and wire code against the compiled snapshot before encoding. A framework mapping absent from that inventory indicates a stale or defective profile/runtime pairing and is replaced by the static `host_contract_mismatch` framework outcome; the unlisted code and original text are discarded. The three transport-owned fallback codes are mandatory inventory members, so this replacement cannot recurse. Plans, bound arguments, structured application values and error-detail objects, unprojected values, and workspace roots never cross stdout. Only the final success text or family code plus final bounded error text crosses; that text may contain the validated compact details/recovery rendering declared above. The generated TypeScript adapter validates and forwards the already-rendered outcome for its host API and performs no semantic filtering or JSON application-value round trip.

The envelope is private host-to-application transport and may serialize canonical identity and the runtime engine version by necessity. Framework-owned shaping never adds either to `HostCallResultV1`, framework stderr logs, generated artifacts, model-visible values, plans, events, or errors. RFC 0016's application-producer contract separately forbids binders and typed resolvers from copying private context into declared application values; application-owned logging carries the same deployment obligation. Implementations redact `Debug` for the envelope, context, and runtime facts.

Generated adapters emit no telemetry containing `options.input`, the logical input or context snapshots, observed runtime version, serialized call envelopes, raw stdout, decoded outcome text, or rendered confirmation text. They drain stdout and stderr concurrently so either pipe can make progress. `max_stderr_bytes` counts child-origin stderr bytes offered to the embedding-owned sink. The adapter keeps no queued sink chunks and at most one in-flight `write`. When the sink is available, it copies at most the remaining child-byte allowance into a fresh `Uint8Array` and invokes `write` once. A synchronous return immediately frees the slot; a returned promise receives rejection handling but is never awaited by pipe draining, result projection, cancellation, or reaping. While one promise remains unsettled, later child bytes are drained and dropped. A synchronous throw or rejected promise disables later child-byte delivery for that call, and neither error text nor value escapes.

Any cap, backpressure, or sink-failure drop marks the call's diagnostic stream truncated. If the sink remains enabled and its slot next becomes available before call-local cleanup, generated code offers exactly one fresh copy of the fixed 30-byte UTF-8 notice `[mcp-twill: stderr truncated]\n`; otherwise it offers none. It never waits for that notice, retries it, or retains another chunk behind it. The notice is framework-origin data and does not count against `max_stderr_bytes`, so the absolute bytes offered to `write` are bounded by `max_stderr_bytes + 30`. If an in-flight promise never settles, the call offers no notice, retains at most that one bounded copied chunk in the promise chain, and still completes the tool path. Sink mutation of any supplied array cannot affect stdout, decoding, another chunk, or a tool result. Stderr is never accumulated for an exception, joined to stdout, or copied into a model-visible host error. Decoder, launcher, premature-exit, and isolated child-panic failures use only the bounded static host-contract diagnostics declared above. Artifact generation likewise never embeds fixture invocation values, context samples, or an observed runtime version beyond the explicitly authored schema/example corpus.

The surface and adapter hashes prove version agreement, not caller authentication. Enabling `ProcessEnvelopeV1` explicitly grants host-context and runtime-fact injection authority to the configured local launcher boundary. Under `TrustedVsCodeUi`, asserting an in-range runtime version can satisfy a trigger-matching confirmation requirement, so access to that subcommand is approval authority as well as context authority. The host subcommand is not exposed as a generic MCP transport or enabled implicitly by compiling the crate. Deployments that do not trust every process able to invoke that configured entrypoint must use the exact generated `InProcess` bridge or place the process entrypoint behind an authenticated launcher/channel. The initial RFC makes no stronger cross-user or hostile-local-process claim.

`HostInvocationTransport` initially supports:

```rust
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostInvocationTransport {
    InProcess,
    ProcessEnvelopeV1 {
        logical_binary_name: String,
        subcommand: Vec<String>,
        limits: HostProcessLimits,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct HostInvocationLimits {
    pub max_call_bytes: u32,
    pub max_result_bytes: u32,
}

impl HostInvocationLimits {
    pub fn new(
        max_call_bytes: u32,
        max_result_bytes: u32,
    ) -> Self;
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct HostProcessLimits {
    pub max_stderr_bytes: u32,
    pub termination_grace_ms: u32,
}

impl HostProcessLimits {
    pub fn new(
        max_stderr_bytes: u32,
        termination_grace_ms: u32,
    ) -> Self;
}

pub struct HostInProcessAdapter { /* bound compiled host and server */ }

impl HostAdapterProfile {
    pub fn bind_in_process(
        self,
        server: CliMcpServer,
    ) -> Result<HostInProcessAdapter>;
}

impl HostInProcessAdapter {
    pub async fn call(
        &self,
        tool: &str,
        arguments: BTreeMap<String, serde_json::Value>,
        context: HostInvocationContextV1,
        runtime: HostRuntimeFactsV1,
    ) -> HostCallResultV1;
}

#[derive(Default)]
pub struct HostProcessRouter { /* registered immutable host runtimes */ }
pub struct HostProcessEntrypointError { /* private redacted reason */ }

impl HostProcessRouter {
    pub fn new() -> Self;

    pub fn register(
        &mut self,
        host: HostAdapterProfile,
        server: CliMcpServer,
    ) -> Result<&mut Self>;

    pub async fn serve_stdio_v1(
        &self,
        profile_id: &str,
        host_adapter_hash: &str,
    ) -> std::result::Result<(), HostProcessEntrypointError>;
}
```

The exact common limit spelling is the required profile member `"invocationLimits": { "maxCallBytes": u32, "maxResultBytes": u32 }`. The transport spellings are `"inProcess"` and `{ "processEnvelopeV1": { "logicalBinaryName": string, "subcommand": [string], "limits": { "maxStderrBytes": u32, "terminationGraceMs": u32 } } }`. They are public profile declarations under the additive policy; the version-1 call, context, runtime-fact, and result types are closed private transport values.

`bind_in_process` accepts only an `InProcess` profile and a finalized native server whose active surface matches the profile. Its typed `call` path applies the same host context gates, runtime-fact validation, operation mapping, explicit ordinary-delivery selection, base-authorizer/host-confirmation composition, result projection, and redaction as process dispatch without serializing a private call envelope. It returns `HostCallResultV1` populated from the bound profile's authoritative version and hashes, so generated code can reject an accidentally cross-wired in-process bridge exactly as it rejects stale process output. The returned Rust future is the cancellation boundary: dropping it cancels framework-owned pending work, with the ordinary best-effort boundary after application work has accepted dispatch. It never invokes RFC 0020 task cancellation or mutates a task store because the call created no task. A process profile cannot bind through this API, and an in-process profile cannot enter `HostProcessRouter`.

`logical_binary_name` is the deployment-independent packaged path or launcher name emitted into the host artifact; it is never treated as an executable path or passed directly to `spawn`. `subcommand` is the authored fixed argument prefix preceding generated `--host-profile` and `--host-adapter-hash` arguments. The logical name and every argument must be non-empty and contain no NUL. The prefix cannot contain `--`, either reserved flag, a reserved `--flag=value` spelling, or any token beginning with those reserved names; the generator owns the complete selector suffix. It always launches the resolver-supplied absolute executable with an argument vector and `shell: false`, never by concatenating a command string. The generated adapter appends the compiled public values, and the application routes them to `HostProcessRouter::serve_stdio_v1`.

The router is a single-assignment map keyed by validated profile id. `HostProcessRouter::new()` is exactly `Self::default()`, so its public zero-argument constructor satisfies the workspace's warning-denied Clippy contract. `register` accepts only a `ProcessEnvelopeV1` profile plus an already finalized `CliMcpServer`, preserving its native surface, request-context configuration, base authorizer, confirmation route, RFC 0020 runtime sidecars when present, and private RFC 0016 sidecars. The host profile supplies the only generated-host confirmation authority; registration composes it around the preserved base authorizer for calls through this profile without mutating the server or affecting another adapter entrypoint. It validates the complete pair before insertion, including exact active surface name/hash agreement. Repeating a profile id is an error even when the host hash and server are equivalent; registering a newer hash under the same id is likewise rejected rather than silently replacing or retaining two launch authorities. A failed registration leaves the map unchanged and invokes no server sidecar. Applications that need an old and new artifact concurrently assign distinct profile ids and generate both snapshots explicitly.

After insertion the pair is immutable. `serve_stdio_v1` first resolves the id and compares the supplied hash with that one registered profile before reading stdin. Unknown id and stale hash take the same static nonzero/no-stdout path and do not reveal which comparison failed. The envelope fields repeat the same identities for closed-message validation but never select configuration. The router never reconstructs a server from a registry or snapshot. `HostProcessEntrypointError` has static bounded `Display`/`Debug`, exposes no source, and is the only nonzero-exit error returned to the application `main`; the router itself never hands a serde, I/O, panic, profile, or application string to generic error reporting.

Limits are explicit, nonzero, hash-covered facts; their `u32` byte domain has one exact JSON and generated-TypeScript representation without bigint conversion. `HostInvocationLimits` applies identically to process and in-process calls. Both paths run the same bounded RFC 8785 version-1 envelope encoder in write-or-count mode: process transport writes those canonical bytes, while in-process transport counts the exact bytes without materializing or crossing a serialization boundary. Oversized call input returns `HostPayloadTooLarge` for direction `call` before process spawn or runtime-hook dispatch. An oversized otherwise valid result is replaced by the fitting static result-direction failure on both paths. Runtime streaming counters may widen declared values internally but comparisons always use the exact `u32` cap. Profile construction rejects a call cap smaller than the minimal valid envelope and a result cap too small for every static transport-owned fallback result carrying the profile's fixed hashes.

`HostProcessLimits` owns only process-specific diagnostic and termination behavior. `termination_grace_ms` is bounded to 1 through 30,000. `max_stderr_bytes` is nonzero and limits child-origin bytes offered to the embedding diagnostic sink; the sole fixed 30-byte truncation notice is additional framework-origin data, and all unoffered child bytes are still drained and discarded. Resolution to installation-specific absolute executable and working-directory paths happens at runtime and is excluded from identity. The complete child environment likewise remains application launch state; Twill does not copy arbitrary environment variables into the profile or implicitly inherit them in generated code.

The guide's one-MiB call, result, and forwarded-stderr limits plus two-second grace are VBL profile declarations, not framework defaults. Fixture acceptance serializes every released example plus adversarial near-boundary cases and proves the call/result caps exceed the largest compatibility payload; stderr fixtures prove truncation without process deadlock or model-visible leakage. Another host selects its own reviewed limits; changing one regenerates the host snapshot and artifacts.

Both transports are part of the initial contract. `InProcess` is the reusable embedding path, while `ProcessEnvelopeV1` is required to replace VBL's released VS Code process-per-call adapter without preserving a private transport dialect. A later long-lived transport can extend this enum under its own version and cancellation contract; it is not a prerequisite for the first generated adapter.

### Cancellation

For in-process transport, generated code passes the invocation's exact `vscode.CancellationToken` to the captured `HostInProcessRuntime::call` hook and checks it again after the hook settles but before validating or projecting a result. The conforming bridge registers cancellation before starting the bound Rust call, drops the `HostInProcessAdapter::call` future if cancellation is observed while it remains pending, and settles its hook promise only after that future has either been dropped or completed. Generated code waits for that settlement rather than resolving cancellation while the bridge still owns pending framework work. If cancellation was observed, it discards either a racing returned envelope or rejection and propagates host cancellation; it never turns that settlement into `HostContractMismatch`. If the hook settles while the token remains live, a rejection is the static host-contract failure and a returned envelope follows ordinary closed validation. A hook that ignores the token, resolves while it still owns a pending Rust future, or lets framework-owned work remain pending after cancellation is nonconforming trusted embedding code; the generated adapter cannot mechanically cancel an arbitrary promise behind the hook.

Cancellation before dispatch performs no binder or handler effect. Once application work accepts dispatch, dropping the future is best-effort and makes no rollback claim. In either transport, host cancellation terminates only this ordinary host request: it never emits `tasks/cancel`, updates a task record, invokes task access or storage, or leaves a retained task for later polling. A future task-aware long-lived transport uses RFC 0020's selected cancellation operation and semantics rather than a generic task-cancel abstraction.

For process-per-call transport, the generated adapter spawns the one-shot wrapper in a platform process group or job object, closes stdin after its one write, and always awaits/reaps the wrapper. Cancellation or stdout overflow first requests termination, waits `termination_grace_ms`, then force-terminates any still-running non-detached wrapper descendants and awaits final exit before resolving cancellation/failure. A broker that intentionally detached before the signal is application-owned state rather than an accidental wrapper descendant; Twill neither kills it nor claims rollback after it accepted an operation. Lease ownership, TTL, release declarations, and downstream survivor checks govern that broker. A profile cannot advertise stronger cancellation semantics than its transport implements.

### Identity And Artifacts

```rust
pub struct HostAdapterSnapshot {
    version: u32,
    profile_id: String,
    native_protocol_version: String,
    catalog_hash: String,
    surface_hash: String,
    host_adapter_hash: String,
    document: serde_json::Value,
    canonical_json: Box<[u8]>,
}

impl HostAdapterSnapshot {
    pub fn version(&self) -> u32;
    pub fn profile_id(&self) -> &str;
    pub fn native_protocol_version(&self) -> &str;
    pub fn catalog_hash(&self) -> &str;
    pub fn surface_hash(&self) -> &str;
    pub fn host_adapter_hash(&self) -> &str;
    pub fn document(&self) -> &serde_json::Value;
    pub fn canonical_json(&self) -> &[u8];
}

pub struct VsCodeGeneratedArtifacts { /* private exact outputs */ }

impl VsCodeGeneratedArtifacts {
    pub fn manifest_projection(&self) -> &serde_json::Value;
    pub fn manifest_projection_json(&self) -> &str;
    pub fn adapter_typescript(&self) -> &str;
}

pub fn generate_vscode_artifacts(
    snapshot: &HostAdapterSnapshot,
) -> Result<VsCodeGeneratedArtifacts>;
```

Profile compilation produces one immutable version-1 `HostAdapterSnapshot`. Its fields are private and its accessors expose only borrowed or copied validated facts; it has no public constructor or mutator. Its canonical document contains the complete validated host declaration; generated contribution records; translated guidance and presentation triggers; result projection; context gates; logical transport; artifact-generation inputs; the sorted application-code inventory reachable from exposed RFC 0014 identities plus validated host gates; and the sorted closed framework-code inventory accepted by the version-1 result decoder. The framework inventory is the exhaustive current set of response-profile `ErrorCode` wire spellings valid on this host transport, excluding `application_error` because that family has its own outcome variant and including the host-owned `host_contract_mismatch`, `host_payload_too_large`, and `unsupported_host` spellings. Construction obtains this inventory from one exhaustive Rust mapping over `ErrorCode`; no caller supplies strings and no generated source maintains a parallel list. Adding, removing, or renaming a framework response code therefore requires an exhaustive compiler edit, changes the host snapshot/hash, and regenerates the decoder instead of silently changing version-1 serde output. The snapshot also contains the exact consumed native snapshot under `nativeSurface: { version, protocolVersion, name, catalogHash, surfaceHash, document }`. The first five values copy the compiled RFC 0015 snapshot accessors, and `document` is an unchanged clone of `NativeToolSurfaceSnapshot::document()`, carrying routes, tools, and projection facts without reserialization or re-derivation. Host compilation obtains every semantic input through the snapshot's typed read-only accessors and uses `document()` only for this unchanged nested identity value. `HostAdapterSnapshot::native_protocol_version()`, `catalog_hash()`, and `surface_hash()` repeat those authoritative nested identities and compilation rejects disagreement.

`HostAdapterSnapshot` implements neither `Serialize`, `Deserialize`, nor `JsonSchema`. Like the native snapshot, it is a compiled capability with one structured document and one canonical byte sequence, not a second public wire form. Duplicate codes within either result inventory reject before sorting; the tagged outcome keeps the application and framework namespaces distinct, so the same spelling may exist once in each without ambiguity. Generated TypeScript receives both inventories from this document; it never scans Rust source or guesses family membership. Adding or removing an accepted code therefore changes the host hash and regenerates the decoder even when no manifest field changes. The document contains the declared confirmation authority and trusted range but no runtime context-provider object, observed engine version, invocation context, resolved executable or working-directory path, environment entry, cancellation token, or application value. It excludes `host_adapter_hash`; `canonical_json()` returns its RFC 8785 bytes. Twill computes the hash with the corpus shared framing and exact domain `io.github.wycats.mcp-twill/host-adapter`, using the snapshot's `version`. Generators consume the returned bytes and embedded computed hash rather than reserializing the public value or reproducing framing logic.

The host snapshot version freezes the complete generated-artifact and private-runtime contract, not only the canonical document shape. Version 1 includes callback snapshot order, bounded canonical encoding, process-envelope and result decoding, in-process cancellation settlement, process termination/reaping, launch reflection, and the diagnostic sink's one-in-flight state machine plus exact truncation notice. A semantic change to any of those fixed behaviors requires a new snapshot version and therefore a new host hash and regenerated artifacts, even when authored profile fields are unchanged. A change to the logical process wire contract additionally introduces a new transport variant rather than reinterpreting `ProcessEnvelopeV1`. Pure implementation restructuring may retain version 1 only when all frozen source, wire, state-machine, and runtime vectors remain byte-for-byte or behaviorally identical as applicable.

The host adapter hash covers profile kind, tool naming, icon, prompt references, structured guidance, the complete confirmation policy including any trusted engine range, result projection, unsupported/absent-context policies, common invocation limits, logical transport contract, and canonical native surface snapshot. It excludes the observed runtime engine version, resolved installation-specific absolute paths, and all invocation context values.

The host adapter hash is a generated-artifact and private-transport compatibility identity, not another invocation-fingerprint input. A host context gate runs before native preparation: rejection never invokes the registry authorizer, consults or consumes replay authority, constructs a prepared invocation, or dispatches application work. Once the gate accepts, it contributes only the normalized typed context facts owned by RFC 0009, RFC 0013, and RFC 0016. RFC 0015 then fingerprints the catalog operation, active surface hash, validated arguments, and those context facts. Fixed host result projection runs only after the registry has produced one validated outcome. A confirmation trigger alone remains presentation; a `TrustedVsCodeUi` policy may satisfy the base authorizer only through the narrow entrypoint composition above and never changes the attempted command effect, plan, fingerprint, or reusable replay authority.

Host-only facts such as a name prefix, icon, prompt alias, confirmation policy, text result projection, runtime engine observation, or unsupported/absent-context rejection therefore do not change the fingerprint of an invocation that reaches dispatch. A fact that changes operation semantics, active schema, confirmation presentation text bound to replay, or resource binding belongs in the native surface and changes its hash. Two host profiles intentionally consuming the same surface produce the same invocation fingerprint for the same dispatched call, even when their packaging, entrypoint approval policy, rejection policy, or result projection differs. Host-UI satisfaction creates no replay record; independent replay authority for that fingerprint still authorizes the same attempted native dispatch through either profile. The host adapter hash is neither an additional replay scope nor an authentication credential. A deployment requiring execution or approval isolation across profiles must compile distinct native surface identities rather than overloading host packaging identity.

`generate_vscode_artifacts` is the public snapshot-to-artifacts boundary. It accepts only the compiled `HostAdapterSnapshot`, verifies the VS Code host kind and supported snapshot version, and returns exact in-memory outputs without reading or writing the filesystem. `manifest_projection` is the closed object owning exactly `/engines/vscode` and `/contributes/languageModelTools`; application build tooling replaces those two package-manifest slots and fails if either ancestor is not an object. `manifest_projection_json` is the fixed two-space/LF rendering of that same value, ending in one newline. `adapter_typescript` is UTF-8 TypeScript with LF endings, no BOM, and one final newline. The adapter source embeds the exact surface and host hashes; the manifest projection is hash-derived and verified byte-for-byte without adding a non-platform hash field. An identical snapshot produces byte-identical strings; generated code contains only the launch-resolver interface and never a resolved executable, working directory, environment entry, runtime context value, stderr byte, or fixture call.

`ProcessEnvelopeV1` freezes its version-1 wire and lifecycle semantics as one logical transport contract. A later process contract receives a distinct transport variant and cannot reuse that discriminant, even when its authored launch fields happen to be identical.

The emitted TypeScript declares `HostContextProvider` plus only the runtime hook appropriate to the snapshot transport. For `ProcessEnvelopeV1`, generated code owns `spawn`, the exact profile-authored subcommand and generated selector suffix, bounded envelope encoding, concurrent pipe draining, cancellation, reaping, and closed result decoding. `HostProcessRuntime.resolveLaunch` supplies one atomic deployment-specific launch record plus an optional raw-byte diagnostic sink. The resolved executable and working directory must be non-empty, NUL-free absolute paths under the generated adapter's current platform path rules; a bare command name or relative path is a host-contract mismatch and never invokes PATH lookup or an inherited current directory. Environment keys must be non-empty and contain neither `=` nor NUL, values must contain no NUL, and Windows rejects ASCII-case-insensitive duplicate keys. The generated launcher passes exactly the resolved working directory and environment map to `spawn`: it does not inherit or overlay `process.cwd()` or `process.env`, fill missing variables, or reinterpret `undefined`. An embedding that wants ambient deployment values must select and copy them explicitly into its launch record. The adapter never serializes or logs any launch value and treats resolver exceptions or invalid entries as the same static local `HostContractMismatch` failure. The working directory is a process deployment fact, not RFC 0009 workspace context. Environment entries may configure application deployment facts such as a state directory or broker endpoint, but Twill never reads either launch field as conversation identity, workspace observation, operation selection, profile/hash selection, or model arguments; those per-call facts have only the typed envelope authorities above. Sink delivery follows the exact one-in-flight, zero-queue, copied-chunk state machine above. It offers at most the declared child-byte cap plus one fixed 30-byte notice, handles throws/rejections, and never delays or changes the tool result. For `InProcess`, generated code calls only `HostInProcessRuntime::call` and validates the returned closed `HostCallResultV1` under the same version/hash, code/text inventory, and bound rules used after process decoding.

The in-process runtime hook is an explicit trusted dispatch boundary, just as the configured executable is for process transport. TypeScript can prove that a returned envelope matches the compiled profile and surface; it cannot prove that arbitrary application code actually delegated to `HostInProcessAdapter`, honored cancellation, or refrained from fabricating an otherwise valid result. A conforming integration therefore implements the hook as a thin bridge to the bound Rust adapter and keeps application business logic behind Twill handlers. The common result cap bounds what that Rust adapter returns and what generated code accepts/projects; it cannot retroactively prevent an arbitrary hook from allocating an oversized JavaScript object before returning it. Installed-artifact tests exercise the exact supported binding. The RFC claims structural drift detection and closed result validation at this seam, not provenance or allocation isolation against a malicious or incorrectly implemented embedding.

The two generated callback sequences are fixed. `prepareInvocation` checks cancellation, constructs and bounds its input snapshot, resolves the direct/grouped operation, evaluates RFC 0018 presentation against the direct snapshot or selector-excluded grouped command view, returns the exact plain-string `PreparedToolInvocation` projection above, and discards every private value. Cancellation and preparation failure use the two exact local exceptions above. It calls no provider, gate, launcher, in-process bridge, or Rust adapter. `invoke` independently checks cancellation, constructs and bounds its input snapshot, resolves the direct/grouped operation, calls and snapshots the context provider once, applies host context gates, proves the complete logical version-1 call including the registration-captured runtime facts fits `max_call_bytes`, checks cancellation again, and passes its complete input, context, and runtime snapshots to the selected transport. Process transport bounded-encodes them without rereading any source object; in-process transport passes the deeply frozen snapshots to the captured bridge and the bound Rust adapter repeats the same logical byte count. Rust plans first, runs the hard registry policy and preserved base authorizer, and only then lets a matching trusted confirmation policy satisfy `RequireConfirmation`; no TypeScript callback sends an approval bit. A route, gate, validation, size, cancellation, hard denial, or unmatched confirmation result invokes no later hook.

Runtime launch resolution occurs only after invocation preflight has succeeded and a final cancellation check still permits dispatch. `resolveLaunch` is called exactly once for that one spawn attempt; generated code performs no automatic launch retry. It receives exactly the profile-authored `logical_binary_name`. After the synchronous hook returns, generated code checks cancellation again, validates and snapshots the complete launch record, and only then spawns. Cancellation or failure before that point starts no process; cancellation after spawn follows the process-tree state machine below.

`HostProcessLaunch` is an exact plain-data boundary despite TypeScript's structurally open interface. The returned value's reflected view must be a non-null object with reported prototype `Object.prototype` or `null`, exactly the three own enumerable data properties `executable`, `workingDirectory`, and `environment`, no accessors, and no symbol properties. `environment` follows the same reflected prototype/data-property rule, permits arbitrary own string keys subject to the key/value validation above, and has no symbol properties. Accessors, inherited entries, and extra launch fields fail closed through static `HostContractMismatch`; any reflection trap or getter exception text is discarded. JavaScript supplies no reliable generic test for a transparent `Proxy`, so the RFC does not claim to reject one by identity. A proxy is accepted only when every reflected operation presents one valid stable data view. The validator copies accepted strings into a fresh null-prototype environment and private immutable launch record before any asynchronous boundary, so later mutation, target replacement, or changed proxy behavior cannot alter the executable, working directory, or environment that reaches `spawn`. The optional diagnostic sink and resolver function are captured once when `registerGeneratedHostTools` installs the generated contributions; changing properties on the caller's runtime object later cannot alter an already registered adapter.

Platform generators therefore consume the compiled `HostAdapterSnapshot` to produce manifest projection and adapter source. Generated runtime source embeds `surface_hash` and `host_adapter_hash`; build validation regenerates and compares both source and manifest projection byte-for-byte. A declaration cannot be passed directly to a generator or runtime helper.

The canonical profile compiler and initial VS Code manifest/TypeScript generators are supported library surfaces of `mcp-twill-host`, which already owns host policy, envelopes, and artifact identity. Profile compilation accepts only RFC 0015's immutable `NativeToolSurfaceSnapshot`; it never receives a command registry or an authored surface declaration from which it could re-derive semantics. A thin build tool or application `xtask` may invoke those APIs, but it does not own a second generator contract or release cadence. A future collection of independently versioned host backends may justify a dedicated generator crate without changing the snapshot format.

### Required Invariants

- Host adapters consume a canonical RFC 0015 surface snapshot through typed read-only accessors and never parse its JSON document or re-derive command schemas, operation routing, annotations, results, or presentation.
- The VS Code engine floor is explicit, validated, hash-covered, and generated into the manifest. Any trusted-UI range is separately explicit, non-empty, contained within that manifest family, hash-covered, and enforced from the registration-captured runtime version; the opaque context extractor is never emitted for an unreviewed engine family.
- Host profile construction accepts no registry or uncompiled surface declaration; every semantic input arrives through the immutable snapshot and is covered by its surface hash.
- Version-1 generated hosts expose no task lifecycle and reject a native snapshot containing any `Required` tool because required support forbids ordinary delivery. Their private entrypoint explicitly selects RFC 0020's immediate ordinary path for `Forbidden` and `Optional`, preserves compiled delivery metadata and identity, and leaves every installed task-runtime sidecar untouched.
- Host generators consume the immutable `HostAdapterSnapshot`; runtime helpers bind only a compiled `HostAdapterProfile` to a finalized matching native server. Unvalidated declarations, bare registries, and uncompiled surfaces cannot reach artifact or dispatch paths.
- Process and in-process runtimes both return the same closed `HostCallResultV1` identity envelope. Generated code validates the exact version, host hash, surface hash, family code inventory, and text bounds before projecting either result.
- An in-process TypeScript hook is a trusted bridge to `HostInProcessAdapter`, not a mechanically proven result provenance boundary. Its conformance claim requires exact delegation and cancellation settlement: its promise settles only after the Rust future has been dropped or completed, while generated code waits, discards any cancellation-racing result, and propagates host cancellation without relabeling it as a contract failure. Closed-envelope validation detects accidental cross-wiring and shape drift but does not authenticate arbitrary embedding code.
- Native and host snapshots expose only read-only typed accessors plus one canonical document/byte representation; they have no public construction/mutation path or serde/schema implementation, and the host document nests the exact consumed native fields and document plus its authoritative surface hash.
- Host identity has one public source: `HostAdapterSnapshot` accessors. No parallel serializable identity aggregate can drift from the snapshot or become a second process-envelope shape.
- The public VS Code generator returns exact in-memory manifest and TypeScript outputs from one compiled snapshot; it performs no filesystem I/O, and generated runtime hooks cannot replace routing, bounds, decoding, result shaping, or cancellation semantics.
- Each generated module exports one transport-specific `registerGeneratedHostTools` signature. Registration captures callable members with their original receivers before side effects, installs the complete contribution set under one extension-owned composite disposable, rolls back every partial registration on failure, and rejects a second successful registration before observing new hooks.
- Opaque host extraction produces only typed RFC 0013 identity and RFC 0009 workspace observations; it cannot stamp model arguments.
- Generated `prepareInvocation` and `invoke` each construct one deeply immutable callback-local complete public-input snapshot. Each resolves direct/grouped routing before any command evaluator or context provider. The selector remains in grouped equivalence/transport snapshots but is excluded from the selected command view. Preparation renders and discards its snapshot without application hooks or retained approval state; invocation constructs the only snapshot that may cross a transport, then captures one typed-context snapshot before runtime work.
- Ambient, absent, and unsupported context remain distinct in the versioned envelope and are revalidated and policy-checked by Rust.
- Identity presence and workspace presence remain independent; omitted host roots permit RFC 0009 lower-authority fallback, while a present empty collection deliberately blocks it.
- Tool arguments cross the host transport as the invocation callback's exact complete logical snapshot. RFC 0018 presentation used the preparation callback's direct snapshot or selector-excluded grouped command view; only byte equivalence of the complete public snapshots, including any selector, may bind the two under an explicitly trusted host contract. Neither object identity nor original JavaScript wire spelling is portable authority.
- Version-1 logical input and context snapshots admit at most 128 nested containers and are copied under `max_call_bytes`; over-depth or over-bound input cannot reach presentation, a runtime hook, or retained partial framework state.
- Version-1 call/result envelope bytes use bounded RFC 8785 canonical JSON in generated TypeScript and Rust. Object insertion order cannot change value equivalence or byte counts; array order remains significant, `-0` normalizes to `0`, and a direct Rust number that cannot round-trip exactly through finite binary64 is rejected rather than rounded.
- Every generated host input string and object key is a well-formed Unicode scalar sequence before either transport runs; valid surrogate pairs count as one scalar and isolated surrogates fail locally without disclosure or coercion.
- Result omission is fixed, schema-validated, hash-covered, and applied identically to schema and runtime values.
- Text-only generated hosts carry only Rust-rendered projected success text or redacted error code/text; application values, error details, and RFC 0012 resource links never enter the private result envelope or bypass host result omission.
- A generated-host confirmation trigger by itself never changes server-side authorization or creates approval metadata. `ServerOnly` always preserves the base authorizer decision and RFC 0015 route.
- `TrustedVsCodeUi` preserves hard-policy denial plus base-authorizer `Allow` and `Deny`; it satisfies only `RequireConfirmation` when the closed runtime version lies in the tested range and the same compiled trigger matches the invocation snapshot. Every other requirement follows the compiled RFC 0015 route. Generated code retains no approval record or flag.
- Host context gates are operation-specific, argument-independent, and may emit only framework-owned host errors or an application code with one existing server-wide RFC 0014 identity.
- An absent-context application rejection is an explicit profile-scoped use, never a `CommandExecutionOutcome`; it obeys the referenced server identity's message/details contract, remains hash-covered in one host profile, and never adds an error use to the target command, so native application semantics remain unchanged.
- Under absent context, recovery never recommends an operation that the same host policy rejects; it translates through that rejection's declared host action.
- Unsupported-context summaries and recovery are static host-profile declarations selected by a stable reason enum; opaque token values and provider error strings can never become output.
- Framework-owned host shaping never carries private identity, host roots, plans, bound arguments, application values/error details, full unprojected results, or adapter envelopes across the result envelope or into model-visible results and logs; registered application producers retain RFC 0016's no-copy obligation.
- Generated adapter telemetry never records tool input, private context, envelopes, raw process output, decoded outcome text, or rendered presentation; application stderr routing remains an explicit embedding-owned diagnostic policy and never feeds a tool error.
- Process diagnostic delivery keeps at most one copied sink chunk in flight and no queue. Child-origin bytes offered are capped by `max_stderr_bytes`; one optional fixed 30-byte truncation notice is the only additional sink data. Sink delay, mutation, throw, rejection, or non-settlement cannot block pipe draining, process reaping, cancellation settlement, or tool results.
- Every profile declares one common call/result byte bound enforced before either transport dispatches and after either transport produces a result. In-process delivery counts the exact logical version-1 envelope bytes without treating absence of serialization as absence of a resource limit.
- The captured context provider runs once only in `invoke`, after cancellation, invocation-input snapshot construction, and local direct/grouped route preflight. Its closed result is snapshotted before transport, authoritative Rust route validation, context gating, or runtime work; provider exceptions, invalid reflected values, and later mutation cannot become invocation authority.
- Generated process launch resolution runs once only after invocation context, input, bounded encoding, and cancellation preflight succeed. It snapshots one exact reflected plain-data launch record before spawn; accessors, inherited/extra/symbol fields, and later object or proxy mutation cannot become deployment authority.
- Process transport selects one registered immutable profile from generated launch identity before reading stdin, bounds call/result encoding and pipe reads plus forwarded stderr with that profile's hash-covered limits, drains both pipes concurrently even after stderr forwarding truncates, accepts exactly one result value, and reaps its wrapper on every exit, overflow, and cancellation path.
- `HostProcessRouter` assigns each profile id once. Duplicate equivalent or changed-hash registration never replaces/coexists with the first pair; failed registration is atomic, and unknown-id versus stale-hash launch selection is publicly indistinguishable before stdin is read.
- Wrapper process-tree cleanup stops accidental descendants but never treats an intentionally detached application broker as disposable transport state; application resource lifecycle remains authoritative.
- Catalog, surface, and host adapter identities remain distinct and compose in that order.
- Host-only artifact changes invalidate the host adapter hash without perturbing invocation fingerprints. Host approval authority remains an entrypoint-local runtime policy that mints no replay; any host fact that changes planning, command semantics, reusable approval scope, or application-visible context must first become a catalog, native-surface, or typed-context fact.
- Adapter hashes detect contract drift but are never treated as authentication; process context, runtime-fact, and any resulting narrow approval authority exist only at the explicitly configured launcher trust boundary.
- Generated artifacts are reproducible from the canonical snapshot; runtime source contains the exact hashes, and the platform-only manifest projection remains byte-for-byte hash-derived without inventing an unsupported manifest field.
- Generated result decoders consume the snapshot's sorted application/framework code inventories; no accepted result code exists only in hand-written TypeScript or an un-hashed Rust match.
- Core `ErrorCode` never serializes directly into `HostCallOutcomeV1`; every framework wire string comes from the snapshot's exhaustive version-1 mapping, and an inventory disagreement fails closed through the mandatory static contract-mismatch outcome.
- Host adapter hash input is the RFC 8785 host snapshot document under the exact shared domain/version/length framing; private provider objects, invocation context, resolved paths, cancellations, and runtime results never enter it.
- Host snapshot version 1 freezes the generated callback, encoding, cancellation, launch, process-tree, and diagnostic-sink contracts. Any semantic change increments the snapshot version and host hash; a process-wire change also selects a new transport variant instead of reinterpreting `ProcessEnvelopeV1`.

### Acceptance Test Ownership

Twill acceptance tests live in `crates/mcp-twill/tests/host_adapters.rs`. They use a compact generic server for failure, privacy, transport, and determinism cases. VBL parity first validates RFC 0015's manifest, then compares generated contributions against `vscode-package.json`, native mappings and schemas against `surface-catalog.json`, and generated presentation against `presentation-vectors.json`; new host-profile declarations come from `crates/mcp-twill/tests/support/vbl.rs`. Tests do not launch an installed editor or depend on a sibling repository.

The final installed VS Code/Codex artifact smokes remain VBL-owned downstream gates. The VBL port must regenerate from the released Twill APIs, build exact extension and server artifacts, install them, exercise conversation continuity and cleanup, and record artifact checksums/provenance. Those gates prove integration and packaging; Twill's local tests prove the reusable contract and generator.

### Implementation Phases

1. Add host adapter profiles, identity, validation, canonical snapshot, and contract checks in `mcp-twill-host`.
2. Add versioned in-process dispatch plus the registered process router, generated launch identity, bounded envelope I/O, and typed context.
3. Add fixed result projection and bounded host success/error rendering with fitting transport fallbacks.
4. Add the VS Code manifest and TypeScript adapter generator with the exact one-shot registration lifecycle, context-provider interface, and presentation trigger.
5. Complete Twill's owner-local VBL profile and frozen v0.4.9 parity in `host_adapters.rs`.
   The later VBL-owned port consumes released Twill crates to replace its manifest transformations, invocation/confirmation switches, result filter, process wrapper, envelope parser, and error formatter with generated artifacts and owns TypeScript, exact-artifact, and installed-host evidence.
   It is not part of this RFC's owner-local implementation PR.

### Acceptance Tests

- The guide's complete Rust lifecycle compiles as written: one compiled host snapshot feeds `generate_vscode_artifacts`, then that same profile and its exact matching finalized native server enter either `HostProcessRouter::register` or `bind_in_process`. Artifact generation accepts no declaration or surface, runtime binding accepts no snapshot or registry, and generated selector arguments can select only an already-registered process pair.
- Prefixing 27 VBL tools produces 27 unique valid VS Code contribution names and preserves native dispatch mapping.
- Process and in-process artifacts type-check against the pinned VS Code floor with exactly one transport-specific `registerGeneratedHostTools(extensionContext, contextProvider, runtime): void` signature and no runtime-selectable overload. Registration captures every required member before calling `vscode.lm.registerTool`, preserves the original receiver for class methods, parses the runtime version once for the successful transaction, and transfers one composite disposable into `extensionContext.subscriptions` only after all contributions register. Fault injection at every capture/registration position proves reverse cleanup leaves zero active tools and no subscription entry, continues after a throwing disposable while preserving the original activation failure, permits retry after failure, and makes a second call after success fail before hook observation or tool registration. Registration invokes no provider, resolver, sink, bridge, or application handler; post-registration member/sink replacement has no effect while state intentionally read by a captured receiver method remains live.
- `Identity` and valid non-empty prefixes produce their single canonical spellings; empty/control-bearing prefixes and any prefix yielding an invalid or colliding host tool name fail profile compilation.
- Display names, user/model descriptions, browser icon, server instructions, and four prompt-reference aliases match the established extension manifest.
- Prompt references resolve catalog operation ids through the active surface and accept exposed direct mappings only. Unknown, omitted, grouped, duplicate-reference, or two-operations-to-one-reference declarations fail rather than creating a selector-less alias or silently choosing a member.
- Contributed input schemas equal the ambient-only native surface schemas exactly; no TypeScript transform removes `agent_session_id`.
- `DeclaredPresentation` emits exact VBL invocation and confirmation messages while `None` emits only the Twill-authored invocation message. Generated `PreparedToolInvocation` values always contain the plain-string `invocationMessage`; they contain exactly the two plain-string `confirmationMessages.title`/`message` properties only when the trigger returns a prepared confirmation, omit that property otherwise, and never project operation id, branch, or `MarkdownString`. Under `ServerOnly`, neither changes server authorizer decisions; under `TrustedVsCodeUi`, the same Twill-authored confirmation trigger is also the sole compiled predicate that may satisfy a matching `RequireConfirmation` decision. Platform-generic UI is outside that predicate.
- Generated TypeScript executes RFC 0018's complete pure-evaluator vector set byte-for-byte with Rust, including raw-key presence on schema-invalid pre-invocation values, every fixed JSON short escape, C0/C1 and presentation-unsafe `\uXXXX` escaping, ECMAScript trimming, BMP/non-BMP scalar counting, complete-escape truncation, valid surrogate-pair handling, isolated-surrogate fallback, and ordinary fallback selection. Actual `prepareInvocation` rejects a non-scalar input before returning presentation, while representable preparation input uses one immutable callback-local complete snapshot for selector resolution and a direct or selector-excluded grouped command view for rendering. It never turns presentation fallback into an argument value or validation result. Direct and grouped fixtures prove the invocation-side dispatcher removes exactly the same selector and binds the remaining values.
- Callback-snapshot fixtures mutate the preparation source during reflection and after return, and mutate the invocation source from the context provider and while runtime work is pending. They prove each callback uses only its own accepted snapshot. Preparation calls no provider or runtime hook and retains no snapshot, digest, object key, or approval record; invocation calls the provider once for a valid uncancelled and successfully routed input and never for cancellation, logical-input failure, or invalid grouped selector, and provider output mutation after return has no effect. A host harness supplies preparation/invocation objects with reordered object members, changed selectors, changed array order, and changed scalar values; RFC 8785-equal complete objects satisfy the explicit value-equivalence definition, while changed selectors, arrays, or scalars do not. Generated code conveys only the captured runtime version, never approval. `ServerOnly` preserves every base-authorizer decision. `TrustedVsCodeUi` mechanically checks the range, invocation-side trigger, and base decision; it deliberately trusts the tested platform to supply a value-equivalent later callback because no correlation state exists. Separate installed-host evidence at both range endpoints proves that platform behavior before the profile enables it. A manually forged in-range process or direct in-process call is an exercise of the explicitly trusted entrypoint, not a detected mismatch.
- Exact-data snapshot fixtures cover ordinary/null/alternate prototypes, own/inherited/accessor/non-enumerable/symbol properties, dense/sparse/extra-property arrays, finite/non-finite numbers, cycles, repeated identities, valid surrogate pairs, isolated surrogates, stable transparent proxies, throwing or inconsistent traps, and static redacted failures. Number vectors cover `-0`, the RFC 8785 exponent cutovers, smallest subnormal and largest finite binary64 values, the exact 2^53 boundary, exactly representable larger integers, and a direct Rust integer whose binary64 conversion would change value. Rust and TypeScript produce byte-identical canonical envelopes; the last and every non-finite value fail without rounding or disclosure. Depth 128 succeeds identically through preparation and both invocation transports. Preparation cancellation throws `vscode.CancellationError`; its invalid, depth-129, over-bound, or unroutable input throws the one exact static preparation error and returns no `PreparedToolInvocation`. Invocation depth 129 fails before provider, while an invocation input or context crossing `max_call_bytes` retains no partial snapshot and returns the declared call-direction bound failure. Both transports receive the same invocation logical tree and Rust repeats context, depth, number-domain, and byte-bound validation.
- `EffectDefault` follows the fixed RFC 0003 standard-effect table for direct and selected grouped operations, uses the native snapshot's generic confirmation when no declaration exists, and never treats an unknown custom effect as host approval or confirmation.
- A custom base authorizer that returns `Deny` for a trigger-matching write remains denied. One that returns `RequireConfirmation` for a read under `EffectDefault`, or for an operation without declared copy under `DeclaredPresentation`, follows the native bridge/unavailable route. A trigger-matching `RequireConfirmation` becomes allow only under `TrustedVsCodeUi` with an in-range runtime version. Hard registry denial runs before every case. No case creates or consumes replay authority.
- The VS Code context provider normalizes a supported invocation token and usable file working directory into canonical identity and a typed host root; generated `invoke` code captures, calls, validates, and snapshots it once before runtime work, while `prepareInvocation` cannot observe the token and never calls the provider. Rust revalidates both, injects them through one non-serializing `InvocationContext`, and keeps absent and unsupported token shapes distinct. Frozen VBL vectors cover the exact reason split, identity-only ambient tokens, non-file/empty-path working directories, active-editor then first-folder fallback only under `Absent`, omission with no folder, and zero partial facts or fallback under every `Unsupported` result.
- Rust rejects malformed tagged context, including unknown or invalid fields in RFC 0009's closed host-root wire object; preserves omitted versus present-empty versus populated workspace observations on both `Ambient` and `Absent`; permits declared fallback only for omission; discards partial facts on `Unsupported`; and independently applies both context policies even if generated host control flow is bypassed. A VS Code token with no `workingDirectory` omits the field rather than manufacturing an empty observation.
- Unsupported context allows `help` and rejects other configured operations before dispatch with stable `unsupported_host` output. The VBL fixture renders the exact released v0.4.9 string shown above for every unsupported reason and substitutes only the compiled unprefixed native tool name.
- Generated framework help remains available under absent or unsupported context without normalizing or retaining host facts.
- Each unsupported reason, including a caught `ProviderFailed`, selects its exact declaration-owned message and optional host recovery action; provider exception values/text and token contents never appear in the bounded host error or envelope.
- Absent context rejects VS Code `session start` under the server-wide `session_required` identity with the exact released v0.4.9 `start_session` string shown above, through a hash-covered profile-scoped use and without constructing `CommandExecutionOutcome::ApplicationError` or adding that host-only outcome to `session start`'s native error set. The VBL identity uses `RuntimeBounded` and the fixture proves the required `runtimeMessage` fits its exact bound; a declaration-summary identity requires omission and derives its summary. An ordinary missing-session error that would recover through `session start` projects the same host guidance immediately rather than recommending the rejected tool. Native MCP and ambient host calls retain the RFC 0014 message and recovery.
- Registration rejects an absent-context policy that makes an exposed recovery target uncallable without providing a non-callable host action.
- Registration rejects a host application rejection whose code has no unique server-wide identity among exposed operations in the consumed native snapshot, whose identity exists only on an omitted catalog operation, whose declaration-summary policy receives a runtime message, whose bounded-runtime policy omits one or receives an empty/over-bound value, whose declared details cannot accept the gate's empty object, or whose target lacks the exact grant/carrier-omission/result-omission proof required for host-unusable establishment. A fixture proves the host profile compiles while the target command's native `result.errors` remains byte-identical and omits the host-only rejection.
- A version-1 process envelope dispatches by native tool name, returns `HostContractMismatch` for envelope hash/profile/tool/runtime-fact and other non-context closed-envelope drift, returns `InvalidRequestContext` for malformed or extension-bearing typed context, and includes required authoritative host/surface hashes in every result. A pre-route Rust error uses the fixed `generated host call` subject and never echoes the envelope tool; post-route and generated-adapter-local errors use the compiled unprefixed native name. Unknown/stale launch identity, unreadable input, and unsupported version exit nonzero without stdout. The path never constructs a command string and keeps logs off stdout. Table-driven fixtures cover every missing, wrong-kind, and unknown top-level/context/runtime field without echoing its value or serde text.
- Rust serde/JSON-Schema fixtures and generated TypeScript types agree exactly on camel-case fields, snake-case tags/reasons, closed objects, and omitted versus present-empty workspace roots for every version-1 context, call, result, and outcome variant. The transport-local identity schema accepts exactly version 1, the RFC 0013 issuer grammar, and a non-empty id; malformed values still fail through `ConversationIdentity` validation, while compile-fail coverage proves that `ConversationIdentity` itself remains non-`JsonSchema`.
- Duplicate-key fixtures cover top-level calls/results, nested context, nested arguments, outcomes, and success text. A duplicated call-envelope version exits nonzero without stdout; unique-version context-subtree duplicates map to redacted `InvalidRequestContext`; other call duplicates and every result/success duplicate—including a duplicated result version—map to static local `HostContractMismatch`. Escape-equivalent member names collide, and no path exposes the key or decoder position.
- Canonical-envelope fixtures reorder members, add leading/internal/trailing whitespace, substitute escape-equivalent string spellings, vary exponent/zero spellings, and append a second value. Generated TypeScript and Rust emit the same RFC 8785 bytes; the process call entrypoint and generated result decoder reject every noncanonical outer spelling through their static contract-mismatch paths without exposing the byte or location. Application success text remains opaque to this outer check and retains its separately declared compact-JSON dialect.
- Common invocation limits and process-only limits use exact nonzero `u32` values, serialize identically into canonical JSON and generated TypeScript, and are hash-covered; termination grace is within 1–30,000 ms, and construction proves the common caps can contain the minimal call plus every static fallback result. Generated bounded call encoding/counting returns `host_payload_too_large`/`InvalidInput` without spawning, invoking an in-process hook, or first materializing an over-limit string; cyclic or non-JSON JavaScript input returns static `host_contract_mismatch`/`Failed` without JSON.stringify-style coercion or value disclosure. Shared process/in-process input fixtures accept BMP and non-BMP strings plus valid surrogate pairs, count pairs as one scalar where bounded presentation observes them, and reject isolated high/low surrogates in values and keys before spawn/runtime-hook invocation without exposing their path or code unit. Direct Rust in-process calls repeat the same logical-envelope count. The child selects a registered profile from generated launch id/hash before reading, rejects unknown/stale launch identity without stdin consumption, rejects an envelope whose repeated identity disagrees, and exits nonzero without stdout when bypassed stdin crosses that profile's cap. Bounded Rust result encoding replaces an oversized valid outcome with the fitting `host_payload_too_large`/`Failed` result-direction error on both transports; generated in-process counting rejects an oversized hook result, while an independently streamed oversized or nonconforming wrapper result terminates and reaps the wrapper. Child-origin stderr offered to the sink never exceeds `max_stderr_bytes`; all later or backpressured bytes are drained and discarded, and the only additional offer is one fixed 30-byte truncation notice. Diagnostics expose only direction and configured limit.
- Process launch rejects an empty or NUL-bearing logical binary name or argument, `--`, and every reserved-selector spelling/prefix in the authored subcommand. Generated source passes only the resolver-supplied absolute executable plus an argument vector with `shell: false`; spaces, quotes, dollar signs, and shell metacharacters in reviewed literal arguments remain data and cannot alter the selector suffix or spawn another command.
- Process subcommands authored from a literal array, borrowed slice, and owned string vector normalize to the same ordered profile declaration and host hash.
- Every `HostProcessEntrypointError` path has exact static bounded `Display` and `Debug`, no `source`, no partial stdout, and a nonzero exit; adversarial profile/hash, I/O, and decoder sources never reach a generic `main` error report.
- Generated result decoding accepts every valid canonical success/application/framework outer envelope and rejects missing, wrong-kind, unknown, cross-family, concatenated, whitespace-padded, or otherwise noncanonical result data with the exact static local `host_contract_mismatch` code/message without exposing raw stdout or decoder text. A Rust-side fixture removes an otherwise reachable framework code from a synthetic snapshot and proves entrypoint shaping discards that code/text and emits the mandatory static contract-mismatch outcome; exhaustive compile-time mapping coverage proves every current core code except `application_error` appears exactly once. Its inner success-text scanner accepts nested compact JSON and large number lexemes byte-for-byte, rejects insignificant whitespace/trailing data without constructing a value, and its error-text check rejects empty, control-bearing, or over-bound strings without quoting them.
- Process results contain only final projected success text or family-owned error code plus final bounded text. Raw application values and structured error-detail objects, plans, arguments, workspace roots, identity, and unprojected fields never cross stdout; the declared text renderer may incorporate only the validated compact details and recovery representation specified above.
- Application `ThrowBoundedText` fixtures cover direct and grouped native names, empty and non-empty details, empty/single/multiple canonical recoveries, and flat callable/action tokens. They prove the first part uses the unprefixed native tool name rather than catalog id, selector, host contribution name, or display title; preserve exact part order, omission rules, recovery declaration order, compact JSON, and VBL's unquoted flat token; and apply the 1,024-scalar bound without exposing an unvalidated value or changing the native structured body.
- Generated-source inspection and an adversarial runtime fixture prove that stdout and stderr are drained concurrently; no adapter log/telemetry path receives host input, context, observed runtime version, serialized envelopes, raw stdout, decoded values, or rendered presentation; and streamed child stderr remains separated from stdout and model-visible errors. Sink fixtures cover synchronous return/throw, resolved/rejected/never-settling promises, chunk mutation, cap splitting, and stderr flood. They prove the adapter offers at most `max_stderr_bytes` of fresh-copied child data plus at most one exact 30-byte `[mcp-twill: stderr truncated]\n` notice, keeps one write in flight with no queued chunk, handles rejection without an unhandled promise, continues draining and reaping, never waits for sink settlement, and does not change an otherwise valid result.
- Success projection removes `agent_session_id` from `session start` schema and runtime value while leaving every other value field unchanged, then Rust renders the projected compact JSON text; the generated host result also omits RFC 0012 session resource links. Native MCP retains its configured full value and resource-link result.
- A success fixture containing integer-like object keys and integers outside JavaScript's exact-number range produces the exact Rust compact JSON text in process and in-process host outcomes. Generated TypeScript never parses that application JSON, preserves the text byte-for-byte in `LanguageModelTextPart`, and therefore neither reorders keys nor rounds numbers.
- A result-omission fixture whose removed property solely references a local definition prunes that newly unreachable definition from the host schema; shared and independently reachable definitions remain byte-identical, and the native source schema/hash remain unchanged.
- Application errors format exact code, message, and zero-or-one recovery. Framework errors use their separate declared dialect, retain their framework family and redaction, and never pass through the application-error formatter.
- One instrumented end-to-end matrix follows the corpus `Shared Invocation Lifecycle`: transport/context failure, route/context-gate failure, argument failure, workspace/binding-source failure, wrong lane or dry run, permission/confirmation failure, binder/resource refusal, handler failure, and result-contract failure each produce their owning public family and prove that every later hook remained uncalled.
- The corpus's representative VBL `new_tab` fixture compiles one `tabs new` command into an ambient-only VS Code surface and an optional-explicit-carrier MCP/Codex sibling. Generated-host calls with identity only, identity plus a matching workspace, identity plus a blocking unmatched workspace observation, absent context, and unsupported context prove the exact RFC 0009/RFC 0016 source selection and earliest-failure ownership. The binder receives only the validated identity and selected canonical workspace slice, the handler receives only `Res<Session>`, `CommandContext`, and checked arguments, and every schema, host result, plan, event, diagnostic, and log remains free of the raw identity and internal session reference. The sibling surface retains optional `agent_session_id`; the VS Code schema omits it without a generated-code transform. Both dispatch the same operation and validate the same RFC 0014 success plus `Granted<Tab, _>` edge.
- Cancellation before dispatch performs no work. In-process fixtures cancel before the Rust future starts, while it is pending before application dispatch, after application work accepts dispatch, and after the Rust future completes but before generated projection. The supported bridge drops a still-pending future and settles only after the future has been dropped or completed; generated code then propagates cancellation without validating a racing result or converting a cancellation rejection into `HostContractMismatch`. A non-cancellation hook rejection still produces the static contract failure. Process cancellation closes input, terminates the wrapper process group/job, escalates after the declared grace, and reaps it without resolving early. Across both transports, instrumented RFC 0020 sidecars observe no cancellation call, task mutation, retained record, or protocol operation. A fixture distinguishes an accidental non-detached descendant, which is terminated, from an intentionally detached application broker, which remains governed by resource ownership and explicit survivor cleanup; no rollback claim is made after dispatch.
- A child panic/abort or premature nonzero exit produces only the static generated host-contract failure; panic payload, backtrace, and stderr never reach the tool error. An in-process panic follows the embedding unwind policy and is never relabeled as an application outcome.
- Generated manifest projection and adapter source are byte-stable and regenerate cleanly in contract tests; adapter source embeds exact surface/host hashes, while the manifest adds no unsupported hash field.
- Compiling an equivalent public `HostAdapterProfileDecl` directly and through `HostAdapterProfileBuilder` produces byte-identical `HostAdapterSnapshot` values and the same validation failures.
- Host profile ids accept exactly the bounded lowercase-kebab grammar; invalid or option-like ids fail before hashing, generation, launch argv construction, or runtime registration.
- The VS Code builder's semantic-empty defaults, `UnsupportedContextPolicy::new`, and `HostProcessRouter::new` use derived `Default` implementations and pass warning-denied Clippy, then compile byte-identically to their explicit empty construction. Omitted confirmation policy, unsupported-context policy, common invocation limits, or transport fails construction; every repeated scalar assignment fails even when equal, keyed additions reject repeated keys, and selecting both transport methods fails.
- Serialized profile declarations with omitted versus explicit identity tool names, absent icon, empty prompt/guidance/absent-context collections, and fixed result projection normalize to one declaration and host hash. Process transport emits exactly `logicalBinaryName` for its deployment-independent resolver input and never treats that value as an executable path. Confirmation, unsupported-context policy, invocation-limit, and transport omission fail deserialization rather than selecting behavior or resource bounds.
- `VsCodeVersion::new(1, 120, 0)` generates exact `engines.vscode: "^1.120.0"`, matches the pinned v0.4.9 manifest, and changes the host hash when edited. Floors below 1.120.0 or outside 1.x fail profile compilation. A trusted range with reversed endpoints, an endpoint below the floor, or an endpoint outside the caret family fails profile compilation. Generated callback/provider tests run at the floor and current supported VS Code version. Approval acceptance runs at both trusted endpoints and proves that an invocation following prepared confirmation receives a value-equivalent input; widening either endpoint changes the host hash and requires renewed installed-host evidence.
- Generated registration parses exact stable `vscode.version` once. In-range values cross both transports as closed `HostRuntimeFactsV1`; prerelease, malformed, or overflowing spellings become an omitted version without retaining raw text. Rust rejects a mismatched runtime-fact kind, while omission and valid out-of-range versions preserve ordinary server authorization. Runtime facts enter only the generated process envelope or required in-process bridge parameter; they never enter the handwritten context provider or launch resolver, plans, fingerprints, results, events, diagnostics, application handlers, or model-visible schema. A conformance fixture proves the supported in-process bridge forwards the captured value unchanged, while direct typed injection remains explicitly trusted test/host code.
- Builder-authored `HostGuidanceProjection` segments appear exactly once after derived RFC 0011/native guidance, resolve operation and resource references through the active surface, and produce the same snapshot as an equivalent direct declaration.
- Host snapshot hash vectors prove the exact shared domain/version/length framing, RFC 8785 bytes, surface hash, VS Code engine floor, complete confirmation policy and trusted range, both common invocation-limit fields, both process-only limit fields when applicable, and both sorted result-code inventories, plus exclusion of context-provider objects, observed runtime version, invocation values, and installation-specific paths; the same payload under the native-surface domain cannot collide. In-process snapshots carry the common fields and no process-only limits.
- Version-ownership fixtures hold an authored profile constant while incrementing only the host snapshot version and prove a new host hash plus rejection by the version-1 generator. Frozen version-1 source and runtime vectors bind callback order, both cancellation paths, launch reflection, process-tree cleanup, result decoding, and the exact diagnostic-sink notice/state machine; changing any of those semantics requires the new version before artifact generation succeeds.
- Host snapshot fixtures contain the exact unchanged native canonical document under `nativeSurface.document`, repeat its version/protocol/name/catalog/surface identities, and reject disagreement. Compile-fail coverage proves neither snapshot implements serde or JSON Schema or exposes public construction/mutation.
- Compile-fail coverage proves there is no independently constructible `HostAdapterIdentity`; generators, routers, and bindings obtain catalog, surface, and host hashes only from the compiled snapshot or a bound profile that owns it.
- Host profile construction from the checked-in snapshot succeeds, while a direct declaration's surface-name mismatch or any attempt to supply a declaration in place of a compiled snapshot fails before artifact generation; the consumed surface hash always comes from that snapshot.
- Host profile construction accepts `Forbidden` and `Optional` direct/grouped tools and rejects every `Required` tool before generation or runtime binding. A table over disabled, legacy, and extension native surfaces proves both host transports explicitly select immediate ordinary delivery, return the same fitting application outcome and projected host text subject to each surface's distinct identity hashes, and invoke zero task access, store, runner, codec, polling, or cancellation hooks. No path manufactures a task or ordinary-call exemption.
- `HostProcessRouter` accepts a finalized native `CliMcpServer` with the exact process-profile surface and preserves its configured authorizer, request-context policy, RFC 0020 runtime sidecars, and ambient binder sidecars. An in-process profile, effect-lane server, or native server with another name/hash is rejected at registration, and dispatch never rebuilds a runtime from the snapshot. Duplicate same-hash and changed-hash registration under one profile id both fail without replacing the first pair or invoking any sidecar; distinct profile ids may coexist. Unknown-id and stale-hash selectors produce the same static nonzero/no-stdout error before reading stdin.
- `HostAdapterProfile::bind_in_process` accepts only an `InProcess` profile and the same exact finalized-server pairing; its typed call path matches process context gates, routing, result projection, and redaction without a serialized call envelope and returns the bound version/host/surface identities in `HostCallResultV1`. Cross-binding either transport mode through the other's runtime helper fails construction. Generated TypeScript rejects a shape-valid result from another in-process profile or surface before projection, and an installed integration fixture proves the hook delegates to the exact bound Rust adapter and propagates cancellation.
- Two profiles over one native surface differ in icon, prompt alias, confirmation policy, fixed result projection, and unsupported/absent-context policy. Their host hashes differ. Calls that both profiles dispatch with identical normalized typed context produce the same surface hash and invocation fingerprint; a trusted profile may satisfy one matching `RequireConfirmation` while a server-only or out-of-range profile follows the compiled route, without either minting replay. A context-rejecting profile stops before native preparation, authorizer invocation, replay lookup or consumption, confirmation bridging, and dispatch; an accepting profile applies result projection only after the registry returns its validated outcome. Changing a surface schema, server-confirmation fallback, or binding mode instead changes the surface hash and fingerprint.
- The VS Code generator accepts a compiled VS Code snapshot and rejects an authored declaration, non-VS-Code kind, or unsupported snapshot version. Repeated generation returns byte-identical manifest projection and TypeScript strings with fixed LF/final-newline spelling; TypeScript embeds the exact hashes while the manifest adds no unsupported hash field. Neither output contains fixture context, resolved launch values, or stderr.
- Generated process TypeScript type-checks with only the context provider, atomic launch resolver, optional byte sink, and VS Code registration glue supplied by the application. It owns the exact spawn arguments, bounded streams, cancellation, decoder, and result projection; hook throws and invalid hook results become static local host-contract failures without their text. Generated in-process TypeScript accepts only `HostCallResultV1`, rejects mismatched version/host/surface identities and invalid family codes/text before projection, and has no outcome-only compatibility overload. Executable and working-directory vectors cover POSIX, drive-letter, and UNC absolute paths on their owning platforms plus rejected relative/PATH names and NUL. Launch vectors prove the exact returned working directory and environment replace rather than inherit `process.cwd()` or overlay `process.env`; the resolver receives the exact logical name and is called once only after every invocation preflight and encoding check, with no call on preparation, earlier invocation failure, or cancellation and no automatic retry. VBL vectors additionally prove empty `binaryPath` selects the packaged absolute executable, configured values must be absolute, `process.cwd()` is copied explicitly, only string-valued own environment entries survive, and the four named non-empty trimmed settings replace their exact environment keys while empty settings retain inherited values. Exact-object fixtures reject null/non-object values, alternate reported prototypes, accessors, inherited or extra launch fields, symbol properties, and non-data environment entries; thrown reflection traps are redacted. Transparent-proxy fixtures prove a stable valid reflected view is copied once, while inconsistent or throwing traps fail closed and later target/trap mutation cannot change spawn data. Mutation fixtures likewise prove the copied null-prototype environment and private launch record, plus the registration-time captured resolver/sink hooks, are unaffected by later changes. Environment validation rejects empty/`=`/NUL keys, NUL values, `undefined`, and Windows case-insensitive duplicates, and no launch value appears in output or diagnostics. Slow, mutating, throwing, rejecting, and never-settling sink implementations stay within the one-in-flight state machine and cannot change a valid tool result.
- Adversarial environment variables resembling conversation identity, workspace roots, profile/hash selectors, tools, or arguments never become request authority; only the typed generated envelope supplies those facts. Deployment variables still reach application-owned configuration without serializing into artifacts, envelopes, outcomes, or framework logs.
- The VBL VS Code extension passes TypeScript checks and installed-host smokes after deleting its schema transform, invocation and confirmation switches, result filter, process wrapper, envelope parser, and error formatter. Only opaque-token extraction, application settings, atomic deployment launch resolution, an optional application diagnostic sink, and VS Code activation glue remain handwritten. Its context provider preserves token-only workspace precedence and its launch resolver explicitly copies the selected released `process.cwd()` and string-valued `process.env` entries, applies the four reviewed setting overrides, and requires an absolute configured binary rather than receiving deployment state implicitly from generated code.
- Installed VS Code chat preserves conversation continuity and model-change continuity, exposes no session handle, and cleans up ambient browser state through the released artifact path.

## Drawbacks

This adds a third identity layer after catalog and native surface. The distinction is real: host names and icons can change without changing MCP tools, and both can change without changing command semantics. Tooling and documentation must make the three hashes legible.

Generated TypeScript becomes a supported artifact. Cross-language deterministic rendering, escaping, Unicode bounds, and schema canonicalization require fixtures on both sides.

The opaque context provider remains handwritten host code. The RFC narrows and types its output but cannot eliminate platform knowledge that Twill cannot observe.

The generated bounds govern framework copying and retention after a host value is returned. They cannot prevent VS Code, a proxy trap, or the handwritten context provider from allocating its source graph first. The supported provider remains a small synchronous extractor, and installed-host tests exercise that exact implementation.

VS Code does not expose a correlation identifier to `prepareInvocation`, so generated code cannot mechanically bind that callback to a later `invoke`. The framework keeps server authorization as its explicit `ServerOnly` posture. A `TrustedVsCodeUi` profile accepts the platform boundary only for its hash-covered range and matching trigger, carries the parsed runtime version rather than approval state, and must renew evidence whenever either range endpoint changes.

An in-process generated hook remains trusted embedding code. Returning the full identity-bearing result envelope catches stale or cross-wired bindings, but no TypeScript type can prove that a hook delegated to Twill rather than fabricating a valid bounded result or allocating outside the declared cap before return. The supported integration therefore keeps this hook as thin glue over `HostInProcessAdapter` and relies on installed-artifact conformance evidence for provenance and effective bounds.

Fixed result omission can hide useful data from one host. Requiring explicit fields, schema validation, and a separate host hash makes the choice reviewable, but product judgment remains with the profile author.

Process-per-call transport has weaker cancellation and higher startup cost than a long-lived connection. It remains useful for editor extension isolation and matches VBL's current deployment; long-lived task-aware transport is preferable when the host supports it.

The process envelope also inherits the configured local launcher's trust boundary. Its public hashes prevent accidental contract drift but do not authenticate a hostile local caller. A trusted-UI profile lets an embedding-trusted launcher assert the bounded runtime version that activates its narrow confirmation authority; another local caller able to invoke the same subcommand can assert the same fact. Deployments needing caller authentication require the exact generated in-process embedding or a future authenticated channel rather than treating a build hash as a secret.

## Rationale And Alternatives

**Keep generated manifest validation but hand-write runtime glue.** This detects schema drift while leaving result filtering, context envelopes, error formatting, and presentation switches duplicated. The adapter snapshot should drive both build-time and runtime surfaces.

**Make host facts part of RFC 0015.** MCP tool projection and editor packaging have different identities and consumers. Separating RFC 0019 keeps native surfaces reusable by Codex and generic clients without carrying VS Code prefixes or icons.

**Send identity as a hidden argument.** This violates RFC 0013, leaks host authority into model-visible schemas, and requires stamp/strip logic. Typed private context is the only supported path.

**Let result filters be arbitrary callbacks.** A callback can conditionally leak or drop data and cannot project a truthful schema. Fixed top-level omission is intentionally limited and auditable.

**Treat every host confirmation as server approval.** A UI message is not a portable authorization proof, and VS Code exposes no preparation/invocation correlation token. `ServerOnly` remains presentation. `TrustedVsCodeUi` is an explicit hash-covered profile choice whose closed runtime version and compiled trigger may satisfy only a matching base `RequireConfirmation`; it never carries an approval flag or overrides denial.

**Generate opaque-token extraction.** The token is platform-owned and intentionally not visible to Twill. A small typed provider keeps that knowledge at the host boundary without duplicating application contracts.

## Prior Art

OpenAPI generators separate service schemas from language/client packaging. Editor extension generators derive manifest contributions while retaining small platform hooks. VS Code explicitly permits `prepareInvocation` without a following `invoke` and exposes the opaque invocation token only to the latter, motivating callback-local snapshots and an explicit host trust boundary. RPC systems version private envelopes and carry schema identities to reject client/server drift.

VBL provides the concrete VS Code manifest, process envelope, context provider, result filter, and installed-host acceptance evidence. RFCs 0013 and 0009 provide the typed context boundary; RFC 0015 provides the host-neutral tool snapshot.

## Unresolved Questions

No architectural questions remain for the initial generated-host boundary.
The profile, builder, transport, and generated-hook names in this body are the proposed Stage-1 implementation contract.
Promotion accepts these spellings, and implementation may not publish an alternate compatibility surface.
Any later review-driven rename must return the RFC to design review and amend the managed body and generated-artifact vectors before implementation proceeds.
Such a revision must retain compiled snapshots, the version-1 process envelope, typed provider failure, fixed text-only projection, and the narrowly proven absent-context application rejection.

## Future Possibilities

Additional host kinds may project ChatGPT workspace agents, other editor tool APIs, or generated SDK clients from the same native snapshot. Each host kind needs its own naming, context, result, and installed-artifact acceptance contract.

Long-lived authenticated host channels could bind pre-invocation approval and cancellation more strongly than process-per-call transport. Richer result projection may be added only with a schema-preserving, non-conditional model.
