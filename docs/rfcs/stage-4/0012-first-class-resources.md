<!-- exo:12 ulid:01kwyxetb10dysmb4cvx66cpme -->

# RFC 0012: First-Class Resources

- Status: Implemented
- Area: command model, runtime, catalog, MCP adapter
- Target milestone: v0.2
- Depends on: RFC 0008 (named argument types and unions), RFC 0009
  (handler-visible workspace roots), RFC 0010 (declared preconditions),
  RFC 0011 (guidance decomposition)

## Summary

RFC 0010 taught the catalog to say that a command needs a session and which
commands establish one. This RFC teaches the catalog what a session *is*: a
server-held resource with an identity, a lifetime, other resources scoped
inside it, and commands that mint, enumerate, and release references to it.
Servers declare resources alongside types, workspaces, and capabilities.
Handlers require, grant, and release them through their signatures, and the
framework derives the acquire/use/release graph from those signatures — the
declaration cannot drift from the behavior because the declaration *is* the
parameter. References become self-describing URIs. Every grantable resource
must have an enumeration path, so an agent recovers state by asking the
server rather than by remembering. Transports that can carry MCP resource
links receive them as progressive enhancement.

Two boundaries hold throughout. This is catalog-level ownership, not
Rust-level ownership: the acquire and the release are separate tool calls
with an agent in between, so a grant is a declared edge, not a borrow, and
actual revocation stays inside the server, where broker-owned leases already
live. And nothing load-bearing depends on host support: URIs, enumeration,
and lifecycle edges degrade to plain strings and ordinary commands, which is
what the weakest tier a real server ships — a text-only tool bridge — can
carry today.

## Motivation

### Agent memory is the wrong home for live references

visible-browser-lab's stewardship RFC locates the root of its leak bugs in a
memory hierarchy: broker state is reliable, agent context is not, and any
protocol that requires the agent to remember an obligation will erode when
context compacts. That RFC repaired the teardown half — lifetimes moved into
broker-owned leases, so a forgotten tab eventually closes itself.

The *use* half still rides on agent memory. Every resource vbl hands out is
an opaque string the agent must carry in its context window in order to act
again:

| Handle | Mint format | Lifetime | The reference dies when |
| --- | --- | --- | --- |
| session | `session_{uuid}` | session | the session idles out |
| tab | `tab_{uuid}` | session | closed, released, or session ends |
| element ref | `e_{base36}` | navigation | the document revision changes |
| network request | `request_{seq}` | navigation | the log evicts or the page navigates |
| console message | `console_{seq}` | navigation | the log evicts or the page navigates |
| heap node | `node_{id}` | snapshot | the owning snapshot closes |
| artifact | `artifact_{uuid}` | durable | explicitly deleted |
| pagination cursor | `{kind}_{offset}` | one call | the next call happens |

When compaction drops a `tab_{uuid}` from the conversation, the lease is
alive in the broker and gone from the agent. Recovery exists — `list_tabs`
re-derives every owned tab — but nothing in the surface says so. The catalog
does not record that `list_tabs` enumerates the same resource `new_tab`
grants and `click` requires; the agent is expected to infer the connection
from tool names. The stewardship RFC made teardown independent of agent
memory. This RFC does the same for continued use: when every resource can be
re-enumerated, and the catalog says how, agent memory becomes a cache over
server state instead of the source of truth.

### The reference is load-bearing but unmarked

The table above spans three durability tiers — navigation-lived,
session-lived, durable — and the difference matters enormously to an agent
deciding whether a saved reference is still good. A `tab_{uuid}` from twenty
turns ago is probably alive; an `e_{base36}` element ref from before a
navigation is certainly dead. Today both render as bare strings, and the
agent learns the difference by using a stale ref and reading the
`element_stale` error. vbl already ships the runtime half of the answer: its
errors carry a `RecoveryAction` hint (start a session, list tabs, re-snapshot)
telling the agent how to re-establish what it lost. That enum is a hand-rolled,
per-server reinvention of edges the catalog could derive — the granting
commands and the enumerator of the resource whose reference went stale.

### The session argument tax

vbl's `agent_session_id` is a required argument on roughly sixty operations.
It is the same fact sixty times: this command acts within the session. The
stewardship RFC names the direction (worked example two): hosts that can
identify the conversation should inject session identity ambiently, and the
model should never see a handle at all. But with session identity written
into sixty argument schemas, that future forks the surface — every tier that
goes ambient needs a different catalog than the tier that cannot. Session
identity needs to be a *requirement the command declares* whose *binding is
chosen per tier*: an argument on bare MCP, ambient context on hosts that can
inject it. RFC 0009 already built exactly this shape for workspace roots.

### The host cannot be relied on, and must not be required

MCP has a resource primitive — `resources/list`, `resources/read`, resource
links in tool results, templates, subscriptions — and rmcp 1.7.0 exposes all
of it. It is tempting to build on that surface directly. The support matrix
says no. vbl reaches agents through four front doors: MCP over stdio
(structured content, full resource support available), the Codex plugin
(same stdio server, ambient facts injected via an experimental `_meta`
handshake), the Claude Code plugin, and a VS Code extension that is not an
MCP client at all — it spawns a CLI per call and flattens every result to
text parts. The tier that most needs first-class resources has no channel
for MCP's resource primitive.

So the principle, the same one every Twill RFC lands on: the catalog is the
source of truth, and richer transports get richer projections. Identity,
enumeration, lifecycle, and typed references are catalog facts that project
as strings and commands everywhere. MCP resource links are an enhancement
the adapter emits where the transport carries them.

## Guide-Level Explanation

### Declaring resources

A resource is a server-level declaration, a sibling of `TypeDecl`,
`WorkspaceDecl`, and `CapabilityDecl`:

```rust
server.resource(
    ResourceDecl::new("session", "A leased browser session")
        .uri("vbl://session/{id}")
        .carrier("agent_session_id")
        .expiry("idle sessions expire and release their tabs"),
);

server.resource(
    ResourceDecl::new("tab", "A browser tab leased to one session")
        .uri("vbl://tab/{id}")
        .within("session")
        .lifetime("live until closed, released, or the owning session ends"),
);
```

`within` scopes one resource inside another. `lifetime` and `expiry` are
prose — validity windows are server semantics no framework can check — but
they are *declared* prose, projected everywhere a reference appears, instead
of folklore an agent assembles from error messages.

Each declared resource derives a named reference type (RFC 0008): declaring
`tab` derives `tab-ref`, usable as an argument type. The derived type
accepts either the bare id (`tab_a1b2…`) or the full URI
(`vbl://tab/tab_a1b2…`) and normalizes; a dangling reference gets
nearest-match suggestions, the same treatment unknown commands already get.

### Signatures carry the ownership facts

Handlers state their resource relationships in their parameter and output
types:

```rust
// Requiring: this handler cannot run without a live, resolved tab.
server.command("click", |command| {
    command.handle(|tab: Res<Tab>, context: CommandContext, args: ClickArgs| async move {
        // `tab` is proof of resolution, not a string to re-validate.
    });
});

// Granting: the output component mints a reference.
server.command("new_tab", |command| {
    command.handle(|session: Res<Session>, context: CommandContext, args: NewTabArgs| async move {
        let id = broker.create_tab(&session, &args).await?;
        Ok(CommandOutput::structured(payload).grant(Grant::<Tab>::new(id)))
    });
});

// Releasing: consuming the reference declares the teardown edge.
server.command("close_tab", |command| {
    command.handle(|tab: Release<Tab>, _context: CommandContext| async move {
        broker.close_tab(tab.id()).await
    });
});

// Enumerating: the listing component marks the recovery path.
server.command("list_tabs", |command| {
    command.handle(|session: Res<Session>, _context: CommandContext| async move {
        let tabs = broker.owned_tabs(&session).await?;
        let ids = tabs.iter().map(|tab| tab.id.clone()).collect();
        Ok(CommandOutput::structured(payload).listing(Listing::<Tab>::new(ids)))
    });
});
```

The framework reads `Res<Tab>`, `Release<Tab>`, `Grant<Tab>`, and
`Listing<Tab>` at registration and derives what RFC 0010 asked authors to
write by hand: `click` requires `tab`; `new_tab` provides it; `close_tab`
releases it; `list_tabs` enumerates it. A hand-written `requires("tab")`
that contradicts the signature is unnecessary at best and a lie at worst;
the signature form makes the lie inexpressible, because the handler
literally cannot run without the parameter being resolved.

Resolution is the server author's job, not the framework's. A resource binds
a resolver — the piece that talks to the broker:

```rust
impl Resource for Tab {
    const NAME: &'static str = "tab";
}

server.resolver::<Tab>(TabResolver::new(broker.clone()));
```

The framework hands the resolver a reference and the invocation plan; the
resolver answers with the resource or a structured refusal. Twill never sees
a lease table. It sees resolved-or-refused, and it knows — from the catalog
— what to say next when the answer is refused.

### What the agent sees

Every minted reference is a URI. `new_tab` returns
`vbl://tab/tab_a1b2c3…` in its structured output, and the text projection
prints it. A URI found in context after compaction is self-describing in a
way `tab_a1b2c3…` never was: it names its server, its resource type, and —
through the catalog — its enumerator, its releasers, and its validity
window.

Command help renders the derived lifecycle, in the same voice as RFC 0010's
derived establishing commands and RFC 0011's derived fallback edges:

```
click
  Requires: a live `tab` — a browser tab leased to one session.
  Recover with: list_tabs

new_tab
  Grants: `tab` (vbl://tab/{id}) — live until closed, released, or the
  owning session ends. Enumerate with `list_tabs`; release with
  `close_tab` or `release_tab`.
```

And when a resolver refuses, the error carries the same edges as structured
recovery hints — vbl's `RecoveryAction` enum, derived instead of
hand-rolled: a stale `tab` reference points at `list_tabs`; a missing
session points at `start_session`.

### Per-tier binding, one catalog

A `Res<Session>` parameter says nothing about *how* the session reference
arrives. That is a binding, chosen where the server meets the transport:

- **Argument binding** (bare MCP, CLI): the requirement projects as an
  argument of the derived reference type — `agent_session_id: session-ref` —
  exactly what vbl's surface does today.
- **Ambient binding** (Codex `_meta`, VS Code extension flags, conversation
  identity): the host injects the reference; the argument disappears from
  the projected surface entirely, and the model never handles a session id.

RFC 0009 is the precedent: workspace roots are required by commands,
resolved by hosts, invisible as arguments. Session identity gets the same
treatment, and the sixty copies of `agent_session_id` become a per-tier
projection choice instead of sixty schema facts.

### Progressive enhancement, not dependency

On transports that carry it (MCP with a capable client), a server that also
binds a reader for the resource gets each grant emitted as a
`resource_link` content part — a link the host can render, attach, offer
back, and follow with `resources/read`. On transports that cannot (the
VS Code text bridge), or for resources without a reader, the URI in the
structured payload and the derived help are the whole story, and they are
sufficient: every recovery path is an ordinary command call. No property of
this design requires the enhancement.

## Reference-Level Explanation

### Model

```rust
pub struct ResourceDecl {
    pub name: String,
    pub summary: String,
    /// URI template with exactly one `{id}` slot, e.g. "vbl://tab/{id}".
    pub uri: String,
    /// Argument name used when a tier binds this resource's references to
    /// an argument, e.g. "agent_session_id". Defaults to `{name}_id`.
    pub carrier: Option<String>,
    /// Resource this one is scoped within.
    pub within: Option<String>,
    /// Prose: the window in which references stay valid.
    pub lifetime: Option<String>,
    /// Prose: how the resource leaves the world without an explicit
    /// releasing command (lease expiry, session end).
    pub expiry: Option<String>,
}
```

Declaring a resource derives:

- a named reference type `{name}-ref` (RFC 0008) accepting bare id or full
  URI, normalizing to the id;
- a capability `{name}` (RFC 0010), so the existing `requires`/`provides`
  vocabulary and its projections keep working unchanged.

`CommandSpec` gains derived fields (never written by authors):

```rust
pub struct CommandSpec {
    // ...
    /// Resources this command requires live references to.
    pub requires_resources: Vec<String>,
    /// Resources this command grants references to.
    pub grants: Vec<String>,
    /// Resources this command releases.
    pub releases: Vec<String>,
    /// Resources this command enumerates.
    pub enumerates: Vec<String>,
}
```

The handler-side vocabulary:

```rust
/// Marker trait connecting a handler-side type to a declared resource.
pub trait Resource: Send + Sync + 'static {
    const NAME: &'static str;
}

pub struct Res<T: Resource>(/* resolved value */);
pub struct Release<T: Resource>(/* resolved value, marks teardown edge */);
pub struct Grant<T: Resource>(/* minted id */);
pub struct Listing<T: Resource>(/* enumerated ids */);

pub trait ResolveResource<T: Resource>: Send + Sync {
    fn resolve(&self, reference: &str, plan: &InvocationPlan)
        -> impl Future<Output = Result<T, ResourceRefusal>> + Send;
}
```

`Res<T>` and `Release<T>` are extractor parameters: `handle` accepts
functions whose leading parameters implement resource extraction, with the
typed-args parameter last (the existing `handle_typed` shape generalized).
`Grant<T>` and `Listing<T>` are *typed* output components: attaching one
changes the handler's output type (`.grant(…)` moves the output into a type
that names `Tab`), so the command's full resource footprint — parameters
and output components alike — is readable from the handler's type at
registration, before any invocation runs. The registry derives grant and
listing edges statically; at runtime the framework mints the URI from the
declared template and writes reference objects into the structured
envelope. The exact plumbing (`FromInvocation` trait, tuple impls, output
typestate) is implementation latitude; the catalog-visible contract — edges
derived from types, never from observed runtime values — is fixed.

### Registration Validation

Registration (and the serving path, per RFC 0009's lesson) rejects:

- a `uri` template that is malformed, lacks exactly one `{id}`, has no
  scheme (a relative template mints strings nothing can route back to the
  server), matches another resource's template exactly, or *overlaps*
  another template — two distinct templates that can mint the same URI
  (like `x://{id}/bar` and `x://foo/{id}`) would make reads route by
  declaration order;
- an explicitly declared argument whose name collides with a resource's
  carrier on a command that requires that resource — the carrier is
  injected with the derived reference type, and a hand-written duplicate
  would shadow it;
- a derived name colliding with an explicit declaration: declaring resource
  `tab` alongside a hand-declared `tab-ref` type or `tab` capability is
  rejected, naming both sites — the resource owns those names, and the fix
  is to delete the hand-written declaration;
- `within` naming an undeclared resource, or a `within` cycle;
- a signature referencing an undeclared resource (`Res<T>`, `Grant<T>`,
  `Release<T>`, `Listing<T>` where `T::NAME` has no `ResourceDecl`);
- a declared resource with no bound resolver, when any command requires or
  releases it;
- **an unpaired grant**: a resource some command grants but no command
  releases and no declared `expiry` retires — the stewardship review rule
  ("name the owner whose drop revokes it") as a structural check, the same
  shape as RFC 0010's no-provider rule;
- **an unenumerable grant**: a *scoped* resource (one declaring `within`)
  that some command grants but no command enumerates —
  enumeration-as-recovery is mandatory, not advisory. Root resources are
  exempt: enumerating a lost session would require the very scope that was
  lost, so a root resource's recovery edge is its establishing command,
  which the catalog already derives.

Hand-written `requires`/`provides` naming a resource-derived capability the
signature already covers is accepted and deduplicates to one catalog fact —
migration depends on the two dialects coexisting on one command. There is
no contradictory combination to reject: a hand-written declaration can only
repeat an edge the signature derives or add one the signature does not
express; it cannot negate one.

### Runtime Resolution

Before the handler runs, the framework resolves each `Res<T>`/`Release<T>`
parameter: it obtains the reference from the bound source (argument or
ambient binding), normalizes URI to id, and calls the resolver. A refusal
short-circuits into a structured error in RFC 0010's error shape, extended
with derived recovery edges:

```json
{
  "code": "resource_refused",
  "resource": "tab",
  "reference": "vbl://tab/tab_a1b2",
  "detail": "tab is owned by another session",
  "recover": {
    "enumerate": ["list_tabs"],
    "establish": ["new_tab", "claim_tab"]
  }
}
```

The `detail` is the resolver's prose; the `recover` edges are the catalog's.
The framework enforces no ordering and no session semantics — it asks the
resolver, and the resolver owns the truth.

Release is catalog bookkeeping plus resolution: `Release<T>` resolves
exactly like `Res<T>`, and the handler body still performs the actual
teardown against its broker. Twill cannot drop what it does not own.

### Binding

Each serving surface declares how each resource's references arrive:

- **Argument** (default): the requirement projects as a required argument
  of the derived `{name}-ref` type, named by the resource's `carrier` —
  declared once on the resource, defaulting to `{name}_id`. vbl keeps its
  existing `agent_session_id` by declaring it as the session's carrier, so
  every argument-bound tier projects the name the surface already teaches;
- **Ambient**: the adapter supplies the reference from transport context
  (RFC 0009's mechanism — Codex `_meta`, host-injected identity). The
  argument is omitted from that tier's projected schema, and the catalog
  hash covers the binding mode so the surfaces are visibly different.
  *Implementation status:* argument binding shipped with this RFC; ambient
  binding remains designed-but-unbuilt until a serving surface needs it
  (the unresolved question on the adapter API stands).

### Projection

- **Catalog.** A `resources` section carries every declaration plus the
  derived edges (`grantedBy`, `releasedBy`, `enumeratedBy`, `requiredBy`).
  Commands carry their derived resource fields. All hash-covered.
- **Command help.** Requirements render with the resource summary and
  recovery edge; grants render the URI template, validity prose, enumerator,
  and releasers — all derived, in RFC 0010/0011's derived-edge voice.
- **Server help.** A `Resources:` section lists each resource with its
  scope tree and lifecycle prose.
- **Results.** Structured output always carries grants and listings as
  `{resource, id, uri}` objects; the text projection prints URIs.
- **MCP adapter.** MCP defines a resource link as a resource the server is
  capable of reading, so the enhancement ships as a pair: a server that
  binds a *reader* for a resource gets `resources/read` served for its
  minted URIs and a `resource_link` content part on each grant (rmcp 1.7.0
  carries both today). No reader, no links — a link the server cannot read
  is a dead link, and the structured payload already carries the URI as
  data. Live enumeration through `resources/list` remains a possible future
  extension; nothing in this RFC depends on it.
- **Refusals.** Resource errors carry derived `recover` edges as above.

### Contract Checks

`check_resource_projection` joins the contract suite:

- every declared resource with any edge renders in server help;
- every command with resource fields renders them in its help text;
- every `Listing<T>` producer's structured output schema carries the
  reference array;
- grant URIs round-trip through the derived reference type (mint → parse →
  same id).

### Required Invariants

- Every reference the framework mints renders as a URI from the declared
  template; ids and URIs are interchangeable as inputs. Granted ids must
  use URI-unreserved characters (`A–Z a–z 0–9 - . _ ~`) so substitution and
  parse-back are exact inverses; the framework refuses to mint a grant
  whose id would not round-trip, and parse-back applies the same character
  discipline, so a URI that could not have been minted never resolves.
  (Every vbl mint format already conforms.)
- Minting is bounded by the signature: a handler that emits a grant or
  listing for a resource its type does not name is a handler bug, refused
  at mint time rather than projected into the envelope. Listings honor the
  command's declared output limit the way rows do — truncated with the
  standard truncation marker, never silently dropped.
- No resource can be granted without a release path (command or declared
  expiry) and a recovery path — an enumerator for scoped resources, the
  establishing command for root resources. Unpaired acquisition is
  undeclarable.
- All lifecycle edges in catalog, help, and refusals are derived from
  signatures, never authored.
- The framework performs no lifetime enforcement: resolvers own liveness,
  brokers own revocation, Twill owns visibility and consistency.
- Every projection has a plain-text degradation; no property of the design
  requires host resource support.

## Drawbacks

**Extractor machinery is real complexity.** `FromInvocation`, tuple impls,
typed output components — well-trodden Rust (axum, bevy), but a real
addition to a framework that has stayed small. The alternative (builder
declarations) is simpler and reintroduces exactly the drift this RFC exists
to close.

**Two dialects during migration.** `requires("session")` and
`Res<Session>` coexist until servers adopt signatures. Derivation plus
deduplication keeps them consistent, but readers of builder code see two
ways to say one thing.

**URIs cost tokens.** `vbl://tab/tab_a1b2…` is longer than `tab_a1b2…`, on
every grant, every listing, every reference argument. The structured
envelope already double-emits payloads on MCP (text + structuredContent);
resource links make a third copy. The token budget bought by RFC 0008–0011
is the budget this spends; the eval harness measures whether recovery
reliability is worth it.

**The floor rises.** A server with one grantable resource must now write an
enumerator and a releaser (or declare expiry) to register at all. That is
the point — but it is a cost a quick prototype feels.

## Rationale And Alternatives

**Builder-declared lifecycle without signatures** (`.grants("tab")`,
`.releases("tab")`) — the declaration can lie: nothing connects the string
to what the handler does. The signature form was chosen precisely because
the parameter is the proof. This is the same argument RFC 0010 made against
prose preconditions, applied to Twill's own declaration layer.

**MCP resources as the primary mechanism** — build on `resources/list` and
subscriptions directly. Dead on arrival at the support matrix: vbl's
VS Code tier flattens everything to text and is not an MCP client. Anything
load-bearing that lives only in the resource primitive abandons that tier.

**Status quo: opaque ids and prose** — the baseline being ported. Sixty
copies of the session argument, folklore validity windows, hand-rolled
recovery enums, and references that die silently in compaction.

**Session-scoped URIs** (`vbl://session/{sid}/tab/{tid}`) — nests the
session into every reference, re-coupling what ambient binding decouples;
an agent on a shim tier would carry session ids inside tab URIs it was never
supposed to see. Ids are globally unique; scope is a catalog fact, not a
path fact.

**Framework-generated enumerators** — auto-derive `list_tabs` from the
declaration. Rejected: real enumerators return rich state (vbl's
`list_tabs` reports lease status, activity, ownership), not bare id lists.
The author writes the enumerator; the framework checks that it exists and
that its output carries the references.

## Prior Art

**axum extractors / bevy system params.** The signature as the requirement
declaration, resolved by the framework before the body runs. Twill borrows
the shape and the lesson that tuple-trait plumbing is a one-time cost.

**MCP resources and `resource_link`.** The protocol's own answer to "tools
return things agents refer back to." This RFC treats it as a projection
target rather than a foundation, for the tier-support reasons above.

**Hypermedia (HATEOAS).** A response that carries links to the operations
valid on the thing it returns, so the client navigates by following edges
rather than reconstructing them from memory. The derived
enumerate/release/establish edges in grants and refusals are exactly this,
projected into a tool catalog.

**File descriptors and object capabilities.** The classical forms of
reference-as-proof-of-access. The honesty gap: Twill references are
forgeable strings, not unforgeable handles — resolution, not possession, is
the access check. The resolver is where forgery dies.

**Rust ownership.** The analogy that names the boundary: `Grant`/`Release`
read like move semantics, but the agent sits between acquire and release,
so this is declared pairing, checked at registration — not borrowing,
enforced at compile time. The RFC keeps the vocabulary and disclaims the
guarantee.

**vbl's `RecoveryAction`.** The hand-rolled runtime form of the derived
edges this RFC generates from the catalog. Its existence is evidence the
edges are load-bearing; its hand-rolledness is evidence they belong in the
framework.

## Unresolved Questions

- **URI authority.** *Settled during implementation.* A template must be an
  absolute URI with a scheme, and no two templates on one server may be able
  to mint the same URI — both checked at registration. Cross-server scheme
  uniqueness is a host concern the server cannot check; the convention
  (scheme = declared server name) stays advisory, and nothing in the design
  routes by scheme alone: a reader serves only URIs its own templates mint.
- **Cursors and other call-lived handles.** Pagination cursors die at the
  next call; making them resources would be ceremony without recovery
  value. Current line: resources are things whose references outlive one
  call. Is that the right line, and where do navigation-lived element refs
  sit — full resources, or a lighter "revision-scoped reference" kind?
- **Signatures as the only dialect.** Should `requires`/`provides` for
  resource-backed capabilities eventually be rejected in favor of
  signatures, or kept as the escape hatch for handlers that cannot type
  their resources?
- **Ambient binding surface.** The exact adapter API for per-tier binding
  (and how the catalog hash represents "same command, different binding")
  still needs design; implementation shipped argument binding only. RFC
  0009's root-resolution machinery is the template when a serving surface
  needs it.

## Future Possibilities

**Live enumeration over MCP `resources/list`.** A capable client could list
live tabs as MCP resources rather than calling the enumerating command.
Attractive, unnecessary, and unfunded by evidence — deferred until a host
demonstrably benefits. The catalog already knows every enumerator, so the
adapter could grow this projection without touching any server.

**Subscriptions.** MCP's `resources/subscribe` could notify a capable host
when a granted resource is released or expires, turning the validity prose
into a live signal. Same posture as resource links: enhancement only,
nothing load-bearing.

**Lighter reference kinds.** If revision-scoped references (element refs
that die on navigation) turn out to deserve catalog presence, a
"revision-scoped" lifetime kind could join the model without changing the
grant/release/enumerate vocabulary. The cursors question above decides
whether this is wanted.

**Framework-checked enumerator output.** Today the contract suite checks
that a `Listing<T>` producer's schema carries the reference array. A
stronger check — that the enumerator's runtime output actually contains
every live reference — needs broker cooperation and belongs to the eval
harness, not registration.

## Acceptance Tests

- A server declaring `session` and `tab` resources with grant, require,
  release, and listing signatures registers; the catalog carries the
  resources section and derived command fields, each hash-covered (adding,
  removing, or editing any moves the hash).
- Command help renders requirements with recovery edges, grants with URI
  template, validity prose, enumerator, and releasers; server help renders
  the resource tree. All derived — no authored lifecycle prose anywhere.
- Grants and listings appear in structured output as `{resource, id, uri}`;
  the derived reference type accepts bare id and full URI and normalizes.
- Registration failures, each with a message naming the resource and
  command: unpaired grant (no releaser, no expiry); scoped grant with no
  enumerator; dangling `Res<T>`; missing resolver; `within` cycle;
  malformed, schemeless, colliding, or overlapping URI template;
  derived-name collision with a hand-declared type or capability; a
  hand-declared argument shadowing an injected carrier.
- A root resource (no `within`) granting without an enumerator registers;
  its refusal recovery edge names the establishing command.
- A grant with an id containing URI-reserved characters is refused at mint
  time; a URI whose id segment could not have been minted does not parse
  back. A grant or listing for a resource outside the handler's signature
  is refused at mint time. Listings truncate under the command's declared
  output limit with the standard marker.
- A refusing resolver produces the structured refusal with derived
  `recover` edges; the handler body never runs.
- The MCP adapter emits `resource_link` parts for grants of resources with
  a bound reader and serves `resources/read` for their URIs; without a
  reader no link emits; the CLI/text projection carries the URI in text;
  all from one catalog.
- Contract check fails when a listing producer's schema omits the reference
  array or a resource with edges is missing from server help.

