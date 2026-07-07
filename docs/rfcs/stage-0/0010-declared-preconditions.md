<!-- exo:10 ulid:01kwwy3rzsb7krrrm0pkyaf5s4 -->

# RFC 0010: Declared Preconditions

- Status: Draft
- Area: command model, catalog, help, diagnostics
- Target milestone: v0.2
- Depends on: RFC 0001 (authoritative command surface), RFC 0002 (diagnostics and steering)

## Summary

This RFC gives commands a way to declare the capabilities they require — a live session lease, an owned tab — and gives servers a way to declare what those capabilities are and which commands establish them. The declarations project into generated help and the catalog resource, replace the prose and per-tool boilerplate that carry this contract today, and let the framework pre-validate that capability-carrying arguments are present with steering that names the establishing command.

Enforcement does not move. The server that checks leases today keeps checking leases; the declaration is a promise about what the server will enforce, exactly as a permission declaration is a promise about what the server will do. What changes is that the promise becomes a catalog fact: visible to agents before they call, checkable by contract tests, and rendered consistently everywhere instead of being re-written by hand in three places.

## Motivation

The motivating case is visible-browser-lab. Every vbl tool except `help` requires an `agent_session_id` from a live session lease. Every page-scoped tool additionally requires a `tab_id` naming a tab that session owns. This is the central contract of the entire surface — and the catalog cannot say it.

Today the contract lives in three unstructured places:

- **Prose in `SERVER_INSTRUCTIONS`.** "Start each browser task with start_session and retain its agent_session_id. Use only tab_id values owned by that session." Every agent pays the token cost of this paragraph on every conversation, whether or not it calls a browser tool.
- **Boilerplate on every tool description.** Each of vbl's ~30 VS Code tool descriptions appends a sentence restating the session requirement, because a description is the only per-tool surface the client shows. Thirty copies of one fact, maintained by hand.
- **Runtime errors.** `unknown_session`, `tab_not_owned` — discovered only after a failed call, with recovery guidance written per error site.

Each home has the same defect from the framework's perspective: the requirement is invisible to anything that reads structure. The catalog shows `agent_session_id` as a required string argument — indistinguishable from a search query or a file name. An agent that plans a call with a stale session id gets past every check the framework can run, because the framework has not been told the argument means anything. A contract test cannot verify that help explains the session workflow, because the workflow exists only as prose.

RFC 0001's premise is that the command declaration is the single authoritative source for every projected surface. Preconditions are the largest remaining class of contract that the declaration cannot express. Closing the gap follows the same pattern as workspaces (RFC 0004, 0009) and argument types (RFC 0008): name the concept once at the server level, reference it from commands, and let every surface project from the reference.

## Guide-Level Explanation

A server declares its capabilities the way it declares workspaces: once, with a name, a description, and the argument that carries proof of the capability across calls.

```rust
let server = Server::builder("vbl", "Visible Browser Lab")
    .capability(
        CapabilityDecl::new("session", "A live browser session lease")
            .carried_by("agent_session_id"),
    )
    .capability(
        CapabilityDecl::new("owned-tab", "A tab owned by the calling session")
            .carried_by("tab_id"),
    );
```

A command that establishes a capability says so with `provides`:

```rust
server.command("start_session", |command| {
    command
        .summary("Start one lease-scoped browser session")
        .provides("session")
        // ...
});
```

A command that needs one says so with `requires`:

```rust
server.command("click", |command| {
    command
        .summary("Click one accessibility reference")
        .requires("session")
        .requires("owned-tab")
        .arg(arg::string("agent_session_id").summary("The session issuing this action"))
        .arg(arg::string("tab_id").summary("The owned tab to act on"))
        // ...
});
```

That is the whole authoring surface. From those declarations the framework renders, in command help:

```
Requires:
  session    A live browser session lease (carried by `agent_session_id`;
             establish with `start_session`)
  owned-tab  A tab owned by the calling session (carried by `tab_id`;
             establish with `new_tab`, `claim_tab`)
```

The establishing commands are not written anywhere — they are derived from the `provides` declarations. When vbl adds a new way to acquire a tab lease, every requirement rendering updates itself.

The catalog resource carries the same facts structurally: a server-level `capabilities` array and per-operation `requires`/`provides` lists. An agent reading the catalog can now see the workflow shape — which commands are entry points, which need setup — without any prose at all. The `SERVER_INSTRUCTIONS` sentence and the thirty boilerplate copies retire.

### What the framework checks, and what it does not

The framework validates the *declaration* and pre-validates the *call shape*. At registration it rejects a `requires` naming an undeclared capability, a required capability whose carrier argument the command does not declare, and a capability nothing provides or nothing requires. At planning time, a call missing the carrier argument fails with a diagnostic that names the capability and the commands that establish it — "no `agent_session_id`: start a session with `start_session`" — instead of a generic missing-argument error.

What the framework does not do is verify the capability is *valid*. It cannot know whether a session lease is live or a tab is owned; that state lives in the server (for vbl, in a broker process shared across clients, with ownership checks at 26 call sites that this RFC does not touch). When the server rejects a stale lease, it can use the framework's `capability_denied` error to get the same declaration-derived steering that plan-time failures get — but the check itself stays where the state is.

### How Agents Should Learn This

An agent that reads command help sees the requirement, the argument that carries it, and the commands that establish it, before making any call. An agent that skips help and calls with a missing or stale capability gets a diagnostic that points at the establishing command rather than a bare validation error. Both paths teach the same fact from the same declaration; neither depends on the agent having read a server-level preamble.

## Reference-Level Explanation

### Declaration

`CapabilityDecl` is a server-level declaration alongside `WorkspaceDecl` and `TypeDecl`:

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
    pub requires: Vec<String>,
    /// Capabilities this command establishes.
    pub provides: Vec<String>,
}
```

The builder surface is `ServerBuilder::capability(CapabilityDecl)` and, on the command builder, `requires(name)` and `provides(name)`. Both dedupe: declaring the same requirement twice is a no-op, not an error.

The carrier is declared once, on the capability, not per command. This matches the uniformity the pattern serves: a capability that traveled under a different argument name on every command would defeat the point of naming it. vbl's surface is uniform today (`agent_session_id` and `tab_id` everywhere), and the registration rules below make uniformity a checked property rather than a convention.

### Registration Validation

Registration (and the serving path) rejects:

- a `requires` or `provides` naming an undeclared capability;
- a command that requires a capability but does not declare the carrier argument, or declares it optional — the requirement means the proof must be present, so the carrier must be a required argument;
- a declared capability that no command provides — the catalog would name a capability with no way to establish it;
- a declared capability that no command requires — a dead declaration, following RFC 0008's dead-type rule;
- duplicate capability names.

A command may both provide and require capabilities (vbl's `new_tab` requires `session` and provides `owned-tab`). A command that provides a capability does not implicitly require its carrier — `start_session` takes no `agent_session_id`.

### Planning

Preconditions add one plan-time check: for each required capability, the carrier argument must be bound. Because registration already forces the carrier to be a required argument, this check subsumes the existing missing-required-argument failure for that argument — but replaces its diagnostic. The failure locates at the carrier argument and the steering names the capability and its establishing commands, derived from the declarations:

```
argument `agent_session_id` carries the `session` capability, which this
command requires. Establish it with `start_session`.
```

No resolution happens at plan time and nothing new lands on the plan. A capability is not a workspace: there is no observation to resolve, no root to select, and therefore no per-invocation variance for the fingerprint to cover. The requirement is a static spec fact, and the catalog hash — which covers the command spec serialization — is the identity surface that changes when requirements change.

### Runtime Errors

The framework adds one error constructor for handlers:

```rust
FrameworkError::CapabilityDenied {
    capability: String,
    detail: String,
}
```

A handler (or serving adapter) that determines a presented capability is invalid — stale lease, foreign tab — returns `capability_denied` with the capability name and a server-specific detail. The response layer locates the diagnostic at the carrier argument and appends the same establishment steering the plan-time check uses. Servers are not required to use it; vbl's broker errors can map onto it incrementally. What the constructor buys is that recovery guidance is written zero times instead of once per error site.

### Projection

- **Catalog.** The server catalog carries a `capabilities` array (name, summary, carrier). Each operation entry carries `requires` and `provides` lists. Both are covered by the catalog hash.
- **Help.** Command help renders a `Requires:` section (capability, summary, carrier, establishing commands) and a `Provides:` line on establishing commands. Server help renders a `Capabilities:` section listing each capability with its establishing commands — the structured replacement for the workflow-ordering sentence in `SERVER_INSTRUCTIONS`.
- **Preview.** No change. Preconditions carry no per-invocation state; the preview's job is disclosing what varies per call.
- **Discrete tool descriptions.** When a Twill catalog projects back out to discrete tool definitions (the vbl VS Code manifest path), the generator can append a requirement sentence derived from the declaration. This RFC specifies the structured source; the manifest generator's rendering is that project's concern.

### Contract Checks

A `check_capability_projection` rule joins the contract suite: every required capability is declared, every capability has a provider and a consumer, every carrier argument exists and is required on requiring commands, and command help for a requiring command mentions each required capability by name. Registered in `contract_tests!` like the existing rules.

### Required Invariants

- Registration and serving both reject the invalid declarations listed above; a served surface cannot name a capability the catalog does not define.
- A call to a command that requires a capability, made without the carrier argument, fails at planning time with a diagnostic naming the capability and its establishing commands.
- Establishing commands in help and steering are derived from `provides` declarations, never written as prose.
- Adding, removing, or renaming a capability requirement changes the catalog hash.
- The framework performs no capability validity checks. Liveness, ownership, and revocation are server concerns, unchanged by this RFC.

## Drawbacks

**The declaration can lie.** A permission declaration is checked against nothing; a precondition declaration is the same kind of promise. If vbl declares `requires("session")` on a command whose broker check was removed, the catalog over-claims and no contract test can notice, because the enforcement lives in a process the test suite cannot see. This is the standing tradeoff of the authoritative-surface design: declarations are promises, and the framework makes them consistent and visible, not true.

**The model is deliberately partial.** Capabilities in this RFC have establishment and requirement but no lifecycle: nothing expresses that `close_tab` revokes `owned-tab`, or that a lease expires. An agent reasoning purely from the catalog sees how to acquire a capability but not how it ends. The prose being replaced was equally silent on this, so nothing regresses — but the structured surface may invite the assumption that it is complete.

**One more vocabulary.** Server authors now have workspaces, types, permissions, and capabilities as named server-level declarations. Each earns its place by replacing a worse ad-hoc mechanism, but the cumulative authoring surface is growing, and the boundaries (when is something a capability vs. a permission?) need to stay teachable: permissions describe what the *server* will do to the world; capabilities describe what the *caller* must hold.

## Rationale and Alternatives

**Status quo — prose and boilerplate.** The contract exists today, so the question is whether structure beats prose. Three specific losses recur with prose: agents pay for it whether or not it is relevant (the `SERVER_INSTRUCTIONS` paragraph taxes every conversation); it drifts (thirty hand-maintained copies of one sentence); and nothing can check it (a contract test cannot assert a paragraph explains a workflow). The declared form fixes all three at the cost of one server-level declaration and one line per command.

**Argument-level semantic types instead of command-level requirements.** RFC 0008's type vocabulary could grow a `session-id` semantic type, making the carrier argument self-describing. This was rejected as the primary mechanism because it inverts the emphasis: the fact an agent needs is "this command requires a session" (a command-level workflow fact), not "this string is a session id" (an argument-level shape fact). It also cannot express `provides` — the establishment side is what makes the derived steering possible, and establishment is a property of commands, not arguments.

**A full protocol model.** The `requires`/`provides` graph is a fragment of a session-type or state-machine description of the tool surface, and one could imagine declaring the whole protocol: capability lifetimes, revocation, valid orderings. Rejected for v1 as speculative generality — vbl needs requirement and establishment, the framework cannot enforce lifecycle claims it cannot observe, and a partial lifecycle model that looks complete is worse than an honest fragment. If eval evidence shows agents failing on lifecycle reasoning (reusing closed tabs), that is the signal to design the extension.

**Framework-side enforcement.** Twill could reject calls whose session id it has never seen issued. Rejected as structurally impossible in vbl's architecture — the broker is a separate process shared across clients, so Twill's view of issued capabilities would be incomplete — and undesirable even where possible, because it would turn the declaration from a promise the server keeps into a promise the framework only partially keeps. A half-enforcing framework invites servers to skip their own checks.

**Free-text precondition slots.** A structured list of prose strings ("requires a session lease from start_session") would render in help with far less machinery. Rejected because it re-creates the drift problem inside the structure: the establishing command is named in prose, so renaming `start_session` silently orphans every mention, and no derived steering is possible. The entire value of the feature is that establishment is derived, not written.

## Prior Art

**HTTP 401 and `WWW-Authenticate`.** The closest analog to `capability_denied`: a failure that carries, in-band, the machine-readable description of what credential was missing and how to establish it. This RFC's steering is the same move applied at plan time as well as runtime.

**OAuth scopes on API operations.** Published per-operation requirement metadata (`requires scope: repo`) that clients read before calling, with enforcement at the server. The same declaration/enforcement split this RFC adopts.

**CLI authentication flows.** `gh pr create` failing with "run `gh auth login`" is exactly the derived steering this RFC generates — except `gh` writes that hint by hand at each error site. The capability declaration is the factored form.

**Typestate and session types.** The `provides`/`requires` graph is a deliberately lightweight instance of encoding protocol structure in declarations. The restraint is the lesson: full session types earn their complexity when the compiler enforces them; a declaration surface that cannot enforce should describe less, honestly.

**Object-capability discipline.** RFC 0005 already distinguishes naming a capability from exercising it. This RFC gives the naming side a declaration; exercising remains behind the server's checks.

## Unresolved Questions

- **External capabilities.** Should a capability be declarable with no providing command — established out-of-band (an API key in the environment)? The no-provider registration rule would need an explicit `external()` escape. vbl does not need it; deferred until a real adopter does.
- **Output contract linkage.** `start_session` provides `session`, and its output contains the `agent_session_id` value — but nothing links the `provides` declaration to the output field that carries the new capability. Declaring that link would let help say "retain `agent_session_id` from the result" with the field name derived rather than written. Worth designing when output contracts are next revised.
- **Lifecycle and revocation.** Whether `revokes("owned-tab")` on `close_tab`/`release_tab` joins the vocabulary, and what help should derive from it. Deliberately deferred; see Drawbacks.
- **Per-command carrier overrides.** The carrier is uniform per capability. If an adopter's surface genuinely varies the argument name, a per-requirement override is a compatible extension; supporting it now would weaken the uniformity check for everyone.

## Acceptance Tests

- A server declaring `session`/`owned-tab` with providers and consumers registers successfully; its catalog carries the `capabilities` array and per-operation `requires`/`provides`; command help renders the `Requires:` section with derived establishing commands.
- Registration failures: `requires` naming an undeclared capability; a requiring command missing the carrier argument; a requiring command with an optional carrier; a capability with no provider; a capability with no consumer; duplicate capability declarations. Each with a message naming the command and capability.
- The serving path rejects the same invalid registries.
- A call omitting the carrier argument fails at plan time with a diagnostic located at the carrier argument, naming the capability and every establishing command.
- `capability_denied` from a handler produces the same steering.
- Adding a requirement to a command changes the catalog hash; removing it changes it back.
- `check_capability_projection` fails a registry whose command help omits a required capability, and passes the example server.
- The example server demonstrates a provider (`session start`-style command) and a consumer, covered by `contract_tests!`.
