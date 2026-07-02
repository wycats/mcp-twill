<!-- exo:7 ulid:01kwfyscpm9xmswvkt6jkw0c9s -->

# RFC 0007: Workspace Resolution Crate

- Status: Draft
- Area: workspace resolution, path arguments, rmcp interoperability
- Target milestone: v0.5
- Depends on: RFC 0001, RFC 0002, RFC 0004, RFC 0006

## Summary

This RFC proposes a standalone Rust crate, `mcp-workspace-resolver`, for resolving named workspace roots from the workspace information an MCP server can observe. MCP Twill will use the crate for path-typed arguments, and other `rmcp` servers should be able to use the same crate without adopting Twill's command catalog.

The crate defines a precise vocabulary for workspace requirements, workspace observations, resolution policy, resolved workspace roots, and workspace diagnostics. MCP roots, Codex sandbox metadata, and server-declared workspaces become concrete workspace observations. The resolver applies a deterministic policy and returns the selected workspace root for each named workspace requirement.

The proposal also converts the repository to a Cargo workspace. The existing `mcp-twill` package moves under `crates/mcp-twill`, and the new resolver lives under `crates/mcp-workspace-resolver`. Twill continues to expose `WorkspaceDecl` in its public authoring API, but internally it resolves those declarations through the shared crate before planning path arguments.

## Motivation

Path arguments are part of Twill's typed command model. A command such as `repo read --path $args.path` needs more than a string value; it needs a root against which the path is interpreted, a boundary that constrains traversal, and diagnostics that explain which workspace was used.

MCP provides roots as a protocol feature. Codex provides sandbox metadata that can identify the command's current working directory. Server authors can also declare workspaces directly because some clients do not expose roots and some servers serve a known project or data directory. These inputs all describe workspace context, but they arrive through different mechanisms and have different levels of authority.

A reusable resolver gives those inputs one implementation contract. Twill can use it to plan path arguments, produce consistent diagnostics, and keep the workspace model out of handler-local code. Other `rmcp` servers can use it to get the same behavior for file and directory arguments even when they do not use Twill's command catalog, effect lanes, or response envelope.

## Guide-Level Explanation

A server describes the workspaces its commands need as workspace requirements. A requirement has a stable id such as `repo`, a human-readable name, and a selection policy. Path-typed arguments refer to one of those requirement ids. The resolver's job is to select a concrete root URI for each requirement before the command dispatches.

The resolver receives workspace observations from the host environment. An MCP client that advertises roots can provide the list returned by `roots/list`. Codex can provide `codex/sandbox-state-meta`, whose `sandboxCwd` field identifies the directory in which the current tool call is operating. A server can provide declared workspace roots in its own configuration or authoring API.

The ordinary Twill flow looks like this. The server author declares a workspace requirement, then declares a path argument that uses it:

```rust
let registry = CommandRegistry::build(
    "repo-tools",
    "Repository command server.",
    |server| {
        server.workspace(WorkspaceDecl::file("repo", "/workspace/repo"));

        server.command("repo read", |command| {
            command
                .summary("Read a repository file")
                .description("Reads a file inside the repository workspace.")
                .arg(arg::path("path", "repo"))
                .read("repo", "Reads a file from the repository workspace")
                .handle(read_repo_file);
        });
    },
)?;
```

At runtime, Twill asks `mcp-workspace-resolver` to resolve the `repo` requirement. If the connected client supplies MCP roots, the resolver selects the matching MCP root. If the command is running under Codex and sandbox metadata is present, the resolver derives a project root from `sandboxCwd`. If neither runtime source resolves the requirement, the resolver uses the server-declared workspace root.

The selected root is specific to a requirement. There is no global "the root" for the whole server unless the server has exactly one workspace requirement and the runtime has exactly one unambiguous workspace root. When a request contains several path arguments, each argument is planned against the root selected for its declared requirement.

Codex sandbox metadata produces a useful default root. The metadata gives a current directory, and the resolver derives the workspace root by walking upward to a project boundary. By default, version-control boundaries such as `.git`, `.jj`, and `.hg` are preferred. If no version-control boundary is visible, configured project markers such as `Cargo.toml`, `package.json`, `pyproject.toml`, or `go.mod` can identify the project directory. If no marker is found, the sandbox directory itself is the resolved root. This gives Codex-backed servers a practical project root while keeping the derivation visible in structured diagnostics.

### How Agents Should Learn This

Runtime agents should learn that path arguments belong to named workspaces. Help, resources, prompts, and diagnostics should say which workspace a path argument uses and which root was selected for that workspace. When a path is rejected, the diagnostic should name the workspace requirement, the selected root, the input path, and the failed boundary check.

Coding agents helping write Twill servers should declare workspace requirements where command behavior needs filesystem context. They should use path argument helpers such as `arg::path("path", "repo")`, provide a declared workspace root when the server has a natural fallback, and let the resolver supply runtime roots from MCP or Codex when those observations are available. Handler code should receive already-planned paths rather than reinterpreting raw strings.

Generated documentation should teach the resolution order in operational terms: MCP roots are client-declared workspace roots; Codex sandbox metadata supplies a current execution directory that the resolver turns into a project root; server-declared workspaces provide the server author's durable fallback. The docs should also teach the multiple-root rule: a command uses the root selected for the named workspace requirement, and ambiguous runtime roots produce a workspace diagnostic that the agent can report or repair through configuration.

## Reference-Level Explanation

The repository becomes a Cargo workspace with two member crates.

```text
mcp-twill/
  Cargo.toml
  crates/
    mcp-twill/
      Cargo.toml
      src/
    mcp-workspace-resolver/
      Cargo.toml
      src/
```

The root `Cargo.toml` defines the workspace. The `mcp-twill` package name and `mcp_twill` library name remain stable after moving into `crates/mcp-twill`. The new resolver package is named `mcp-workspace-resolver` and exposes the `mcp_workspace_resolver` library.

The resolver API centers on requirements and observations. Requirements are the workspaces the server needs. Observations are the workspace facts the runtime can provide.

```rust
pub struct WorkspaceRequirement {
    pub id: WorkspaceId,
    pub display_name: Option<String>,
    pub aliases: Vec<String>,
    pub selection: WorkspaceSelection,
    pub fallback: Option<DeclaredWorkspaceRoot>,
}

pub struct WorkspaceObservationSet {
    pub mcp_roots: Option<McpRootsObservation>,
    pub codex_sandbox: Option<CodexSandboxObservation>,
    pub declared: Vec<DeclaredWorkspaceRoot>,
}

pub struct ResolvedWorkspaceSet {
    pub roots: Vec<ResolvedWorkspaceRoot>,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}
```

`WorkspaceSelection` describes how a requirement chooses among runtime roots. The default policy selects an MCP root whose `name` matches the requirement id or one of the requirement aliases. A server may also configure URI-based matching or a primary-root policy for servers that intentionally use the only client root.

```rust
pub enum WorkspaceSelection {
    ByNameOrAlias,
    PrimaryWhenSingleRoot,
    ExplicitUri { uri: String },
}
```

`McpRootsObservation` represents the roots returned by an MCP client's `roots/list` request. The resolver treats the returned list as a set. If the list contains one root and the requirement allows `PrimaryWhenSingleRoot`, that root satisfies the requirement. If the list contains multiple roots, the requirement must select one by requirement id, requirement alias, or configured URI. If no root matches, the resolver returns an `unresolved_workspace_requirement` diagnostic. If several roots match, it returns an `ambiguous_workspace_root` diagnostic.

```rust
pub struct McpRoot {
    pub uri: String,
    pub name: Option<String>,
}

pub struct McpRootsObservation {
    pub roots: Vec<McpRoot>,
}
```

`CodexSandboxObservation` represents Codex's `codex/sandbox-state-meta` request metadata. The resolver uses `sandbox_cwd` as an input directory and applies a root derivation policy. The default policy chooses the nearest visible version-control boundary, then the nearest configured project marker, then the sandbox directory.

```rust
pub struct CodexSandboxObservation {
    pub sandbox_cwd: PathBuf,
    pub permission_profile: Option<String>,
}

pub enum RootDerivationPolicy {
    ProjectBoundary {
        vcs_markers: Vec<String>,
        project_markers: Vec<String>,
    },
    ExactDirectory,
}
```

`DeclaredWorkspaceRoot` represents a root supplied by the server author or deployment configuration. In Twill, `WorkspaceDecl::file("repo", "...")` projects into a `WorkspaceRequirement` with a declared fallback root. A non-Twill `rmcp` server can construct the same requirement directly.

```rust
pub struct DeclaredWorkspaceRoot {
    pub id: WorkspaceId,
    pub uri: String,
    pub display_name: Option<String>,
    pub capabilities: WorkspaceCapabilities,
}
```

The resolver records why each root was selected. Diagnostics and dry-run plans should expose this reason so agents and users can understand the active workspace context.

```rust
pub struct ResolvedWorkspaceRoot {
    pub id: WorkspaceId,
    pub root_uri: String,
    pub source: WorkspaceSource,
    pub selection_reason: WorkspaceSelectionReason,
    pub capabilities: WorkspaceCapabilities,
}

pub enum WorkspaceSource {
    McpRoots,
    CodexSandboxMeta,
    Declared,
}
```

The default resolution policy processes observations in authority order. MCP roots are client-declared workspace boundaries. Codex sandbox metadata is a runtime execution context. Declared workspace roots are server-authored defaults. A higher-authority observation that resolves a requirement supplies the root for that requirement. A higher-authority observation that is present but ambiguous produces a diagnostic rather than allowing an accidental lower-authority expansion for the same requirement.

Presence blocks fall-through. When an MCP roots observation is present but no root matches a requirement, the resolver emits `unresolved_workspace_requirement` and leaves the requirement unresolved rather than falling through to a lower-authority source. An empty roots list is still an authoritative observation: the client declared that no roots exist, so every requirement is unresolved and nothing falls through. Falling through would let server configuration widen filesystem access beyond the boundaries the client declared, which RFC 0004 forbids. Lower-authority sources participate only when the higher-authority observation is absent.

Name and alias matching between requirement ids and MCP root names is case-sensitive on every platform; names are protocol identifiers, not paths. Path boundary comparison is platform-appropriate: case-sensitive for POSIX-style paths and case-insensitive for Windows drive-letter paths. The standard library's lexical path methods compare components case-sensitively apart from the drive letter, so the Windows behavior requires explicit case-insensitive component comparison in the resolver's normalization (or a platform-semantics crate); implementations must not assume `Path::starts_with` alone satisfies this contract. This deliberately tightens the first implementation's unconditionally case-insensitive comparison, which widened boundaries on case-sensitive filesystems.

File roots are compared lexically in the first implementation: normalization resolves `.`/`..` segments and separators without touching the filesystem, so symlinks are not resolved during boundary checks. The exception is Codex root derivation, which already walks the filesystem and may canonicalize `sandboxCwd`. Only `file:` URIs participate in boundary checks; a root or path with any other scheme produces an `unsupported_root_scheme` diagnostic rather than a silent skip.

The crate should expose an `rmcp` feature that converts `rmcp` roots into `McpRootsObservation` and a Codex feature or module that parses `codex/sandbox-state-meta` from MCP request metadata. The core resolver should remain usable with plain Rust structs so non-`rmcp` tests and other MCP SDKs can exercise the same behavior. `WorkspaceObservationSet` must keep its fields private and be constructed only through methods (`Default` plus `with_*` builders or setters), so later observation sources, such as a Codex global-state observation for plugin launch contexts, can be added without breaking construction. Public fields would leave struct-literal construction available to downstream code, which any added field would break.

`WorkspaceDecl` remains a Twill-owned type that projects into a resolver `WorkspaceRequirement` with a `DeclaredWorkspaceRoot` fallback. The resolver stays free of Twill's schema derives, and Twill's catalog JSON schema stays decoupled from resolver versioning. The projection is the single conversion point, and Twill's generated contract coverage should verify that every declared workspace projects into exactly one requirement so the two vocabularies cannot drift silently.

### Required Invariants

- A path argument resolves against the root selected for its named workspace requirement.
- MCP roots are treated as a root set, and a requirement must select one root from that set.
- A single MCP root may satisfy a primary workspace requirement when the requirement allows single-root selection.
- Multiple MCP roots require name, alias, or URI selection.
- Codex sandbox metadata resolves through the configured root derivation policy.
- The default Codex derivation policy produces a project root from `sandboxCwd` when a boundary marker is visible.
- The default derivation policy prefers version-control boundaries (`.git`, `.jj`, `.hg`) and falls back to the configured project markers, which default to `Cargo.toml`, `package.json`, `pyproject.toml`, and `go.mod`.
- Declared workspace roots are used when no higher-authority observation is present; a present-but-unmatched higher-authority observation blocks fall-through and produces a diagnostic.
- Ambiguous roots produce structured diagnostics.
- Resolved roots include their source and selection reason.
- Path normalization and boundary checks happen before command dispatch.

### Implementation Phases

1. Convert the repository to a Cargo workspace and move the existing crate to `crates/mcp-twill`.
2. Add `crates/mcp-workspace-resolver` with requirements, observations, resolved roots, diagnostics, and policy types.
3. Implement MCP roots observation conversion behind an `rmcp` feature.
4. Implement Codex sandbox metadata parsing and default project-boundary derivation.
5. Project Twill `WorkspaceDecl` and path `ArgSpec` values into resolver requirements.
6. Update Twill planning so path-typed arguments consume `ResolvedWorkspaceSet`.
7. Add help, resource, dry-run, and diagnostic projections for selected workspace roots.
8. Add workspace resolver tests and Twill integration tests.

### Acceptance Tests

- A single MCP root resolves a primary workspace requirement.
- Multiple MCP roots resolve by matching the requirement id to the MCP root name.
- Multiple MCP roots with no matching requirement id, alias, or URI produce an `unresolved_workspace_requirement` diagnostic.
- Multiple MCP roots with several matching names, aliases, or URIs produce an `ambiguous_workspace_root` diagnostic.
- Codex sandbox metadata resolves to a version-control project boundary when `.git`, `.jj`, or `.hg` is present above `sandboxCwd`.
- Codex sandbox metadata resolves to a configured project marker when no version-control marker is visible.
- Codex sandbox metadata resolves to `sandboxCwd` when no marker is visible.
- Server-declared workspaces resolve requirements when no runtime workspace observation is available.
- A present but unmatched MCP roots observation produces `unresolved_workspace_requirement` without falling through to declared roots.
- A present but empty MCP roots list leaves every requirement unresolved without falling through.
- A declared workspace fallback is visible in diagnostics and dry-run output.
- Path traversal outside the selected root is rejected before dispatch.
- Twill help and resources show the workspace requirement for path arguments.
- Twill dry runs show the selected root, source, and selection reason.
- The resolver crate can be used in a minimal `rmcp` server test without depending on Twill.

## Drawbacks

This proposal adds a second crate before the workspace model is fully mature. The extra boundary creates versioning and documentation work: Twill authors need to understand the Twill-facing helpers, while lower-level `rmcp` users need to understand the resolver crate directly.

Root derivation from Codex sandbox metadata also needs careful documentation. A current working directory is not the same fact as an MCP root. The default derivation policy makes it useful as a project root, and the selected root must explain that derivation so users can tell whether the resolver chose the directory they expected.

Moving the existing package into a Cargo workspace will touch paths, examples, and CI commands. The move is worthwhile if the resolver crate is truly reusable, but it should be done as a focused migration with compatibility checks for the package name, library name, examples, and tests.

## Rationale And Alternatives

Keeping workspace resolution inside Twill is the smallest implementation step. The standalone crate is proposed because the workspace problem is shared by `rmcp` servers that accept file paths, even when those servers do not use Twill's command catalog or effect-lane model. A crate boundary lets the resolution contract stabilize independently and lets Twill consume the same public API other servers use.

Using only MCP roots would give the cleanest protocol story. It would serve clients that implement roots well, but current clients may expose workspace context through other mechanisms or through no runtime mechanism at all. The resolver therefore treats MCP roots as the highest-authority observation while still supporting Codex sandbox metadata and declared roots.

Using only the process current directory would be simple for local stdio tools. It gives weak results in plugin and desktop environments where the server process can start from a cache directory or installation directory while tool calls operate on a different workspace. Codex sandbox metadata is a better runtime observation because it is attached to the tool call.

Requiring every server to configure a root explicitly is predictable, but it prevents clients from supplying more precise runtime workspace context. Declared roots remain the durable fallback; runtime observations let the same server behave correctly in project-scoped clients.

## Prior Art

MCP roots provide the protocol precedent for client-declared workspace roots. This RFC uses roots as a root set and specifies how a named workspace requirement selects one root from that set.

Language Server Protocol workspace folders provide a similar multi-root model. A server receives a set of folders and must decide which folder a file operation belongs to. The key lesson is that multi-root workspaces need explicit selection and clear diagnostics.

Cargo and other build tools commonly derive project roots from marker files. This RFC applies that practice to Codex sandbox metadata: the current directory is an input, and marker discovery turns it into a project boundary.

Codex's `codex/sandbox-state-meta` mechanism provides a practical current-directory observation for MCP tool calls. The resolver treats that metadata as a Codex-specific observation and records it as the source when it selects a root from it.

## Unresolved Questions

- Should the resolver canonicalize file roots (resolving symlinks) in a later version, and if so, how do plans and replay records account for the boundary change?
- When should the Codex global-state observation for plugin launch contexts be added, and what does it observe when request metadata is unavailable?

## Future Possibilities

The resolver could later support `roots/list_changed` notifications by refreshing the resolved workspace set and invalidating stale plans. Twill could include the resolved workspace fingerprint in replay records so approvals remain bound to the workspace root that was previewed.

The crate could also grow richer capabilities for read-only roots, generated-output roots, temporary roots, remote URI roots, and named root aliases. Those features would let MCP servers describe workspace access in a portable way without each server inventing its own path policy vocabulary.

If Codex formalizes workspace metadata beyond sandbox state, the resolver can add a first-class observation type for that contract while preserving the same requirement and resolution policy model.
