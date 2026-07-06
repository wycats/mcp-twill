<!-- exo:9 ulid:01kwwnx9ftgxwcmkhde200ceax -->

# RFC 0009: Handler-Visible Workspace Roots

- Status: Draft
- Area: command model, planning, handler context, projection surfaces
- Target milestone: v0.2
- Depends on: RFC 0004 (runtime and workspace contracts), RFC 0007 (workspace resolution crate)

## Summary

This RFC lets a command declare that it operates within a named workspace without binding a path argument to it. The framework resolves the workspace at planning time, records the selected root on the invocation plan, and hands it to the handler through `CommandContext`. Help, the catalog resource, the permission preview, and the invocation fingerprint all project the requirement from the same declaration.

RFC 0007 built the resolution machinery — requirements, observations, policy, diagnostics — and RFC 0004 wired resolved roots into planning. But the only way a resolved root reaches a handler today is through a path-typed argument: the plan keeps a root only when a bound argument names its workspace. A command whose relationship to the workspace is ambient — it writes artifacts under the workspace root, or spawns a process with the workspace as its working directory — has no way to say so. This RFC closes that gap with one declaration: `uses_workspace("project")`.

After this RFC, "which directory does this command operate in" is a catalog fact like every other catalog fact: declared once, validated at registration, resolved and enforced at planning time, visible in help and preview, and covered by the fingerprint that binds approvals to what was previewed.

## Motivation

The motivating case is visible-browser-lab. Its artifact-export command writes screenshots, traces, and heap snapshots to disk. The natural destination is a path inside the caller's workspace, and the natural source of that root is the environment the server already observes: MCP `roots/list` when the client provides it, Codex sandbox metadata when running under Codex, or server configuration otherwise. The one place that root must never come from is a tool argument — an agent should not be able to write `workspace_root: "/etc"` into a call and have the server treat it as authoritative.

Today a Twill server cannot express this. The resolver can resolve the root — the adapter already gathers MCP roots and Codex sandbox observations per call — but the plan discards every resolved root that no path argument references. The handler receives a plan whose `workspace_roots` list is empty, so the server author falls back to reading an environment variable or transport metadata inside the handler. That workaround costs the framework its core promises at each layer:

- **The catalog stops being authoritative.** The command's dependence on a workspace lives in handler code and deployment docs, not in the declaration an agent reads. `cli://catalog` shows a command with no workspace relationship at all.
- **Planning cannot fail early.** If the root is unresolved — no client roots, no sandbox metadata, no configured fallback — the command fails inside the handler with whatever error the handler produces, instead of failing at plan time with resolver diagnostics that explain which observation was missing.
- **The preview under-discloses.** A permission preview for a write-lane command should say where the writes will land. When the root arrives through a side channel, the preview cannot show it, and the fingerprint cannot bind the approval to it. An approval granted when the workspace was `~/project-a` silently covers execution against `~/project-b`.
- **Every server reinvents the plumbing.** Reading env vars, parsing transport meta, validating that the result is a directory — each server writes this code independently, with independent bugs, outside the framework's diagnostics.

The fix is small because RFC 0007 did the hard part. Requirements, observation gathering, resolution policy, and diagnostics all exist and run on every call. This RFC adds the missing declaration — a command-level workspace reference — and the missing projection: resolved roots that flow to the plan, the handler, and every derived surface even when no argument mentions them.

## Guide-Level Explanation

A server that operates on a workspace declares it once, as today:

```rust
let server = Server::builder("vbl", "Visible Browser Lab")
    .workspace(WorkspaceDecl::new("project", "file:///srv/default-project")
        .with_description("The project the browser session is inspecting"))
```

A command that operates inside that workspace — without taking any path argument — says so in its declaration:

```rust
server.command("artifacts export", |command| {
    command
        .summary("Export an artifact to the workspace")
        .arg(arg::string("artifact_id").summary("The artifact to export"))
        .uses_workspace("project")
        // ...
});
```

That is the whole authoring surface. The declaration means: this command needs the `project` workspace resolved before it runs, and its handler receives the selected root.

At planning time the framework resolves `project` exactly as it would for a path argument — MCP roots first when the client provides them, Codex sandbox metadata when present, the server-declared fallback otherwise. The selected root lands on the plan's `workspace_roots` list alongside any roots that path arguments selected. The handler reads it from the context:

```rust
async fn export(ctx: CommandContext) -> Result<CommandOutput> {
    // `artifact_id` is the command's declared string argument.
    let artifact_id = ctx.plan.bound_args["artifact_id"].value.as_str().unwrap_or_default();
    let root = ctx.workspace_root("project")
        .expect("planning guarantees a declared workspace resolves");
    let dest = root.path()?.join("artifacts").join(artifact_id);
    // ...
}
```

The `expect` is honest: a declared workspace that fails to resolve fails the call at planning time, before the handler runs and before any side effect. The agent sees a workspace diagnostic — the same shape RFC 0007 defined — explaining which observations were considered and why none satisfied the requirement:

```
command `artifacts export` requires workspace `project`, which did not resolve:
  no client root named `project` (client sent 2 roots: `frontend`, `backend`)
  no server-declared fallback root
```

The agent's repair path is the same as for path arguments: configure the client to expose the right root, or run under an environment that provides one.

Because the root is on the plan, every derived surface sees it. A dry run shows which directory the command would operate in. The permission preview for a write-lane command lists the selected root next to the effects, so the human approving the call knows where writes will land. And the invocation fingerprint covers the selected roots, so an approval binds to the specific root that was previewed — re-running after the workspace changed requires a fresh preview.

Help teaches the requirement. A command that uses a workspace renders a `Workspaces:` section naming the requirement and its description, so an agent reading help before calling knows the command has filesystem context and where that context comes from.

### How Agents Should Learn This

Agents should learn that some commands have an ambient workspace: a directory the command operates in that is not an argument. Help and the catalog teach which commands these are. The agent's job is never to supply the root — there is no argument to supply it through — but to ensure the environment provides one: expose MCP roots from the client, or rely on the server's declared fallback.

When a workspace fails to resolve, the diagnostic names the requirement and lists what was observed. The agent should treat this like a missing capability, not a malformed call: retrying with different arguments will not help, and the diagnostic says so by locating the failure at the command level rather than on any argument. The steering text should point at the repair that can actually work given what was observed. RFC 0007's authority ordering is preserved: when the client has already sent MCP roots, that observation is authoritative — a missing or wrong-named client root leaves the requirement unresolved, and lower-authority fallbacks do not apply. In that case the steering says to expose a client root named `project`; only when no higher-authority observation is present does it suggest the server's declared fallback.

The preview is part of the lesson. When an agent relays a permission preview to a human, the selected workspace root is part of what is being approved. Agents should preserve the fingerprint discipline they already learned from RFC 0003: approval covers exactly the plan that was previewed, including its roots.

## Reference-Level Explanation

### Declaration

`CommandSpec` gains a list of workspace requirements:

```rust
pub struct CommandSpec {
    // ...existing fields...
    /// Workspaces this command requires resolved, beyond those referenced
    /// by path arguments. Names must match server-declared workspaces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
}
```

The builder surface is `CommandBuilder::uses_workspace(name)` (repeatable) and `CommandSpec::uses_workspace(name)` on the low-level API. Registration validates each name against the server's declared workspaces with the same rule that validates `ArgSpec::workspace` references: an unknown name fails `finish()` with a build error naming the command and the missing workspace.

Declaring a workspace both ways — on the command and on a path argument — is valid and redundant; the requirement resolves once.

### Planning

`build_plan_with_workspaces` resolves command-declared workspaces before binding arguments. For each name in `spec.workspaces`:

1. Look up the requirement's resolved root in the `ResolvedWorkspaceSet` the adapter supplied.
2. If resolution failed, return `FrameworkError::WorkspaceUnresolved` carrying the command name, the workspace name, and the resolver diagnostics filtered to that requirement — the same filtering `WorkspaceMismatch` uses today.
3. If resolution succeeded, add the workspace to the plan's used set so the root projects into `plan.workspace_roots`.

The existing path-argument flow is unchanged. `plan.workspace_roots` becomes the union of roots selected for path arguments and roots selected for command-level requirements. The fingerprint input already serializes `workspaceRoots`, so command-level roots participate in approval binding with no further change.

### Error Shape

```rust
pub enum FrameworkError {
    // ...existing variants...
    #[error("{}", workspace_unresolved_message(.command, .workspace, .diagnostics))]
    WorkspaceUnresolved {
        command: String,
        workspace: String,
        diagnostics: Vec<WorkspaceDiagnostic>,
    },
}
```

The response layer maps `WorkspaceUnresolved` to the workspace error family RFC 0002 established, with a diagnostic located at the command (not at an argument) and steering text that names the observation sources that could satisfy the requirement. The message renders each resolver diagnostic on its own indented line, following the union-mismatch precedent from RFC 0008.

### Handler Surface

`CommandContext` gains a lookup and `PlanWorkspaceRoot` gains a path accessor:

```rust
impl CommandContext {
    /// The resolved root for a workspace this command declared or one of
    /// its path arguments referenced. Planning guarantees presence for
    /// declared workspaces; path-argument workspaces are present when a
    /// bound argument referenced them.
    pub fn workspace_root(&self, id: &str) -> Option<&PlanWorkspaceRoot>;
}

impl PlanWorkspaceRoot {
    /// The root as a filesystem path. Errors when the root URI is not a
    /// `file:` URI, using the resolver's normalization rules.
    pub fn path(&self) -> Result<PathBuf>;
}
```

`workspace_root` reads `plan.workspace_roots`; it introduces no new state. Handlers that need the raw URI keep using `root_uri` directly.

### Projection

- **Catalog.** The per-command catalog entry serializes the `workspaces` list. The catalog hash covers it through the existing spec serialization, so adding or removing a requirement changes catalog identity.
- **Help.** Command help renders a `Workspaces:` section when the command declares requirements, showing each workspace's name and server-declared description. Server help continues to list all declared workspaces.
- **Preview.** The permission preview includes the selected roots for command-declared workspaces, rendered the same way as roots selected for path arguments: the workspace name, the root URI, and the source (client roots, sandbox metadata, or declared fallback).
- **Dry run.** No change needed: the plan already serializes `workspace_roots`, and command-declared roots now appear there.

### Contract Checks

`check_workspace_projection` extends to cover the new declaration: every command-level workspace reference names a declared workspace, and the catalog projection of each command's `workspaces` list matches the spec. The generated contract test suite picks this up through the existing `contract_tests!` macro with no new authoring surface.

### Required Invariants

- A command-declared workspace that does not resolve fails the call at planning time, before permission checks and before the handler runs.
- A resolved command-declared root appears in `plan.workspace_roots` and therefore in the invocation fingerprint. Two plans that select different roots for the same command have different fingerprints.
- Registration rejects a command-level workspace reference that names an undeclared workspace.
- `workspace_root(id)` returns `Some` inside a handler for every workspace the command declared.
- Resolution inputs are unchanged: command-level requirements consume the same observation set (MCP roots, Codex sandbox metadata, declared roots) and the same policy RFC 0007 defined. This RFC adds no new observation source.

### Acceptance Tests

- A command with `uses_workspace("project")` and no path arguments plans successfully against a declared fallback root, and the handler observes the root through `workspace_root("project")`.
- The same command planned with a client root named `project` selects the client root over the declared fallback, and the plan's `workspace_roots` records the source.
- Planning fails with `WorkspaceUnresolved` when no observation satisfies the requirement, and the error carries the resolver diagnostics for that requirement.
- Registration fails when `uses_workspace` names an undeclared workspace.
- Two plans for the same call against different resolved roots produce different invocation fingerprints.
- Command help for a declaring command renders the `Workspaces:` section; the catalog entry carries the `workspaces` list; the contract suite validates the projection.
- A command that declares a workspace and also binds a path argument to it resolves once and lists the root once.

### Implementation Phases

1. **Model and registration.** `CommandSpec::workspaces`, builder surface, registration validation, catalog projection and hash coverage.
2. **Planning and error.** Resolution enforcement in `build_plan_with_workspaces`, `WorkspaceUnresolved` with response mapping and steering.
3. **Handler and projection surfaces.** `CommandContext::workspace_root`, `PlanWorkspaceRoot::path`, help section, preview rendering, contract check.
4. **Example and acceptance tests.** Extend the issues example with a workspace-using command; land the acceptance list above.

## Drawbacks

This adds a second way for a command to relate to a workspace, and readers must learn the distinction: path arguments validate *caller-supplied paths* against a root, while command-level requirements deliver *the root itself* to the handler. The declaration surface stays small, but the conceptual split is real and the documentation must teach it.

Making declared workspaces hard requirements means a server author who wants opportunistic behavior — "use the workspace if one is available, fall back to a temp directory otherwise" — cannot express it with this feature alone. That author declares no requirement and reads the plan's roots directly, losing the fail-early guarantee. The Future Possibilities section sketches an optional requirement if real servers need one.

## Rationale And Alternatives

**Why not a hidden or injected argument?** An alternative design adds a path argument the agent never supplies, with the framework injecting the resolved root as its value. This keeps "everything is an argument" uniform, but it lies about the contract: the argument would appear in schemas and help as something a caller might provide, or require a new "hidden" argument concept to suppress it — more machinery than a command-level list, and it still invites the exact failure the motivating case forbids, where a caller supplies the root explicitly. The goal is that the root is never caller-supplied; the declaration should say so structurally.

**Why not let handlers read transport metadata directly?** That is the status quo workaround. It works, but it bypasses planning, preview, fingerprinting, and diagnostics — the four surfaces that make Twill commands trustworthy. Every promise the framework makes about arguments should hold for ambient workspace context too.

**Why hard requirements instead of optional ones?** Failing at plan time with resolver diagnostics is strictly better steering than a handler-level fallback the agent cannot see. The vbl case wants a guaranteed root. Optional consumption is expressible later without breaking this design, and starting with the strict semantics keeps the first release's behavior easy to state: declared means resolved, resolved means visible.

**Why reuse `plan.workspace_roots` instead of a new plan field?** The plan already models "roots this invocation selected, with provenance"; the fingerprint already covers it; the preview already has access to it. A parallel field for command-level roots would force every consumer to merge two lists and would create two places for the same fact.

## Prior Art

- **LSP workspace folders.** Language servers receive workspace folders at the session level and consume them ambiently; no request carries the workspace as a parameter. This RFC gives Twill commands the same shape with per-call resolution and per-command declaration.
- **MCP roots.** The protocol already models client-granted filesystem scope as session state rather than call arguments. This RFC completes the delivery path from that session state to handler code.
- **systemd `WorkingDirectory=`.** Unit files declare the directory a service runs in; the init system guarantees it before the process starts. The declaration-then-guarantee shape is the same.
- **RFC 0007 / RFC 0004.** The resolver vocabulary, observation model, and plan-level root projection this RFC builds on. RFC 0007's guide-level explanation already describes handler code "receiving already-planned paths rather than reinterpreting raw strings"; this RFC extends that stance to the root itself.

## Unresolved Questions

- Should the preview visually distinguish command-level roots from path-argument roots, or is the workspace name plus source enough context? The draft says render them uniformly; review may want the distinction.
- Should `WorkspaceUnresolved` share an error code with `WorkspaceMismatch` in the response envelope's code taxonomy, or get its own code? Sharing keeps the family small; splitting lets clients branch without parsing messages.
- Does the `Workspaces:` help section belong in usage-topic help, full-topic help, or both?

## Future Possibilities

- **Optional workspace requirements.** `uses_workspace("project").optional()` for commands that adapt to workspace presence, delivering `None` to the handler instead of failing the plan.
- **Working-directory semantics for exec-lane commands.** A command that spawns processes could declare that its subprocess runs *in* the workspace root, letting the framework set the working directory and the preview say so.
- **Per-workspace permission scoping.** Effects could be scoped to declared workspaces — "write, but only under `project`" — tightening previews from "this command writes" to "this command writes here."
- **Multiple-root selection surfaces.** If servers need "operate on whichever root the agent chooses," a workspace-typed argument (distinct from a path argument) could make root selection explicit and validated, reusing the same resolution machinery.
