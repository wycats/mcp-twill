<!-- exo:11 ulid:01kwxg9g82ajd9shp9ce775dtm -->

# RFC 0011: Guidance Decomposition

- Status: Draft
- Area: command model, catalog, help
- Target milestone: v0.2
- Depends on: RFC 0001 (authoritative command surface), RFC 0008 (named argument types), RFC 0010 (declared preconditions)

## Summary

RFC 0010 moved the hard half of workflow prose into declarations: preconditions that must hold or the call fails. This RFC moves the soft half: the guidance that tells an agent which command to choose when several could plausibly serve, and which mechanisms are escape hatches to exhaust last. Commands gain a selection criterion (`use_when`), validated cross-references to the commands that serve neighboring cases (`alternative`), and a fallback annotation that names the preferred path and the condition that justifies bypassing it — applicable to whole commands and, via RFC 0008, to union variants. The prose that survives decomposition moves to a declared server preamble, and a contract check keeps command-routing facts from silting back into it.

Soft is the operative word. A precondition can be pre-validated, and its violation fails the call. A selection preference cannot fail anything: calling the escape hatch first is legal, merely worse. The framework therefore validates the *references* (an alternative must name a real command), projects the facts onto every surface, and enforces nothing. Whether the structured form actually improves agent selection is a measurable question, and this RFC treats it as one.

## Motivation

The motivating artifact is, again, visible-browser-lab — specifically the `SERVER_INSTRUCTIONS` paragraph every vbl conversation pays for. It is worth reading the blob against the vocabulary the framework now has, sentence by sentence:

| Sentence (abbreviated) | What kind of fact | Where it belongs |
| --- | --- | --- |
| "Start each browser task with `start_session` and retain its `agent_session_id`." | Precondition | RFC 0010 capability. Retired. |
| "Use only `tab_id` values owned by that session." | Precondition | RFC 0010 capability. Retired. |
| "Inspect an unfamiliar page with `snapshot`, then act through its element references." | Selection | `use_when` on `snapshot` |
| "Use `fill` for one ordinary field." | Selection | `use_when` on `fill` |
| "Use `fill_form` for two or more controls..." | Selection + contrast | `use_when` on `fill_form`; `alternative` edge from `fill` |
| "Use `type_text` for contenteditable controls..." | Selection | `use_when` on `type_text` |
| "Use `press_key` with a target for named keys..." | Selection | `use_when` on `press_key` |
| "Use targetless `press_key` and `click_at` only after `focus_tab` has focused the document." | Soft sequencing | `use_when` on those operations (see Unresolved Questions) |
| "Use `wait_for` for asynchronous state and `screenshot` for visual appearance." | Selection | `use_when` on each |
| "Use `console` and `network` for runtime diagnosis." | Selection | `use_when` on each |
| "Use `help` to select an operation in a specialized domain." | Framework-owned | Twill's own help surface already teaches this. Retired. |
| "Routine actions preserve the user's active application." | Behavioral contract | Server preamble. Stays prose. |
| "Target activation is reserved for `focus_tab`... manual inspection or handoff." | Selection (reserved-for) | `use_when` on `focus_tab` |
| "CSS and evaluate are escape hatches only when snapshot and the named semantic tools cannot represent the required state; do not use them to verify a semantic action." | Preference ordering | `fallback` on the `css` variant and the `evaluate` command |

Of roughly fourteen sentences, two retired with RFC 0010, one belongs to the framework, and all but one or two of the rest are facts about a *specific command*, stranded at server level. Stranding them there has the same three costs RFC 0010 catalogued for preconditions:

- **Every conversation pays for every sentence.** The routing fact for `fill_form` costs tokens in conversations that never touch a form. Placed on the command, it costs tokens exactly where the decision is made.
- **Prose references drift.** "Use `fill_form` for two or more controls" names a command in unchecked text; rename the command and the sentence silently orphans. The framework already solves this exact problem for runnable guidance templates — `validate_guidance` rejects a template that matches no catalog command — but routing references have no equivalent discipline.
- **Nothing is checkable.** A contract test cannot assert that the blob explains when to prefer `snapshot` over `evaluate`, because "explains" is not a property of a paragraph.

There is also a measurement waiting for this feature. The agent-surface-eval baseline scores `css_fallback` and `evaluate_fallback` — whether an agent reaches for the escape hatch when the semantic path would have served — alongside first-selection rate. Those metrics evaluate precisely the class of fact this RFC structures. The blob is the control; the decomposed surface is the treatment.

## Guide-Level Explanation

Selection guidance is written on the command it routes to, in the command builder:

```rust
server.command("fill", |command| {
    command
        .summary("Replace the value of one referenced editable control")
        .use_when("filling a single ordinary field")
        .alternative("fill_form", "updating two or more controls in one pass")
        // ...
});

server.command("fill_form", |command| {
    command
        .summary("Fill two or more referenced form controls sequentially")
        .use_when("updating two or more controls, including combined select and checkbox changes")
        // ...
});
```

`use_when` answers "when is this command the right choice?" — one sentence, positive polarity. `alternative` is a directed edge to a neighboring command with the condition that routes there. It renders on `fill`, which is where an agent about to misuse `fill` is looking:

```
Use when: filling a single ordinary field.
Use instead:
- `fill_form` — updating two or more controls in one pass
```

An escape hatch is declared with the opposite polarity. It names the commands to exhaust first and the condition that justifies reaching past them:

```rust
server.command("evaluate", |command| {
    command
        .summary("Evaluate JavaScript for page state")
        .fallback(
            &["snapshot", "console", "network"],
            "semantic snapshots and diagnostics do not expose the state you need",
        )
        // ...
});
```

which renders:

```
Fallback: prefer `snapshot`, `console`, `network`. Use only when semantic
snapshots and diagnostics do not expose the state you need.
```

and — derived, not written — the preferred commands render the reverse edge:

```
Fallbacks: `evaluate` — when semantic snapshots and diagnostics do not
expose the state you need.
```

The same annotation applies to union variants from RFC 0008, because a union is already a set of alternatives with the routing question built in. vbl's element target is the motivating case — prefer accessibility references, CSS as the escape hatch:

```rust
TypeDecl::union("element-target", "One element on the page")
    .variant(Variant::new("ref", "An accessibility reference from `snapshot`")
        .field(Field::string("ref", "The element reference")))
    .variant(Variant::new("css", "A CSS selector")
        .field(Field::string("css", "Playwright selector"))
        .fallback("the element is not represented in the accessibility tree"))
```

A variant-level fallback needs no `prefer` list: its siblings are the preferred path by construction.

### The preamble, and what keeps it honest

Decomposition leaves a residue — facts about the surface as a whole, not any command. For vbl, roughly one sentence survives: routine actions preserve the user's active application. That prose gets a declared home:

```rust
Server::builder("vbl", "Visible Browser Lab")
    .preamble("Routine actions attach to the owned target and preserve the user's active application.")
```

The preamble projects into the MCP server instructions and server help. What keeps it from regrowing into the blob is a contract check: the preamble must not contain a backticked catalog command name. A command name in the preamble is precisely the signal that the fact belongs *on that command* — as a `use_when`, an `alternative`, a `fallback`, or an RFC 0010 capability. The check turns the decomposition from a one-time cleanup into a maintained property.

### What the framework checks, and what it does not

The framework validates references and projection: every `alternative` and every `fallback` preference must name a command the catalog defines, so a routing edge cannot dangle. It does not check the prose conditions (no machine can), and it does not enforce the ordering — a call to an escape hatch plans and runs like any other call. Selection quality is not a plan-time property; it is an outcome, and the eval harness is the instrument that measures it. This is the same honesty boundary RFC 0010 drew: declarations the framework can make consistent and visible, but not true.

## Reference-Level Explanation

### Model

`CommandSpec` gains three fields:

```rust
pub struct CommandSpec {
    // ...
    /// One sentence: when this command is the right choice.
    pub use_when: Option<String>,
    /// Commands serving neighboring cases, with the condition that routes there.
    pub alternatives: Vec<Alternative>,
    /// Marks this command as an escape hatch for a preferred path.
    pub fallback: Option<Fallback>,
}

pub struct Alternative {
    pub command: String,
    pub when: String,
}

pub struct Fallback {
    /// Commands to exhaust first.
    pub prefer: Vec<String>,
    /// The condition that justifies using this command anyway.
    pub when: String,
}
```

`Variant` (RFC 0008) gains one:

```rust
pub struct Variant {
    // ...
    /// Marks this variant as dispreferred: use sibling variants unless
    /// this condition holds.
    pub fallback: Option<String>,
}
```

and the server gains a preamble, declared with `ServerBuilder::preamble(text)`. All new fields serialize with serde defaults and skip-if-empty, so existing catalogs are unchanged byte-for-byte until a server adopts the vocabulary.

The builder surface is `use_when(text)`, `alternative(command, when)`, and `fallback(prefer, when)` on the command builder, and `fallback(when)` on `Variant`.

`use_when` and `fallback` are mutually exclusive on one command. They answer the same question — when do I choose this? — with opposite polarity, and a command that carries both is contradicting itself. The fallback's `when` *is* its selection criterion.

### Registration Validation

Registration (and the serving path) rejects:

- an `alternative` or `fallback` preference naming an undeclared command;
- a command listing itself as an alternative or a preference;
- duplicate alternative targets on one command, or duplicate names in one `prefer` list;
- a `fallback` with an empty `prefer` list — an escape hatch must say what it is an escape from;
- a command declaring both `use_when` and `fallback`;
- a union whose variants all declare `fallback` — a set of alternatives that are all dispreferred prefers nothing, following RFC 0010's no-provider rule;
- a cycle in the fallback-preference graph (two escape hatches preferring each other describe no ladder);
- empty prose in `use_when`, any `when`, or the preamble.

### Projection

- **Catalog.** Operations carry `useWhen`, `alternatives`, and `fallback`; variants carry `fallback`; the server entry carries `preamble`. All are covered by the catalog hash — reclassifying a command as an escape hatch is a surface change and reads as one.
- **Command help.** Usage text renders `Use when:` after the description, then `Use instead:` with one line per alternative, then `Fallback:` with the preferred commands and condition. Commands named in another command's `prefer` list render a derived `Fallbacks:` section — the reverse edge is never written by hand, exactly as establishing commands are derived in RFC 0010.
- **Type help.** A fallback variant renders its condition inline: `` `css` (fallback — the element is not represented in the accessibility tree) ``.
- **Server surfaces.** The preamble renders in server help after the description and joins the framework-written sentence in the MCP `instructions` field and the `getting_started` prompt.
- **Discrete tool descriptions.** When the catalog projects back out to per-tool definitions (the vbl VS Code manifest path), the generator can append `use_when` to each description — the structured source for the boilerplate those descriptions carry today. As with RFC 0010, the rendering is the generator's concern; this RFC specifies the source.

Diagnostics and steering are untouched. No error changes shape, and no plan-time check is added: there is nothing to check, because preference violations are not errors.

### Contract Checks

A `check_guidance_projection` rule joins the contract suite:

- every command declaring `use_when`, `alternatives`, or `fallback` renders them in its help text;
- every command named in a `prefer` list renders the derived `Fallbacks:` section;
- the preamble contains no backticked catalog command name.

The backtick convention is deliberate. Matching bare words would flag English ("click", "help") that happens to collide with command names; a backticked name in prose is an intentional command reference, and intentional command references are what the decomposition exists to relocate.

### Required Invariants

- Every `alternative` and `fallback` reference resolves to a catalog command; registration and serving both reject danglers.
- Reverse fallback edges in help are derived from declarations, never authored.
- Adding, removing, or editing any selection fact changes the catalog hash.
- The framework enforces no selection ordering at plan or run time.
- The preamble projects verbatim; the contract check bounds what it may contain.

## Drawbacks

**Decomposition trades a synopsis for locality.** The blob, whatever its costs, gave an agent one paragraph with the whole workflow shape. The decomposed facts are only synoptic in the catalog, which an agent may not read. If evals show agents performing worse without a narrative overview, a derived overview (assembled from the structured facts, not written) is the remedy — see Unresolved Questions.

**The conditions are still prose.** `use_when` and `when` texts are unchecked and can drift from behavior, like every summary and description in the catalog. What the structure buys is position (the fact sits where the decision is made), reference integrity, and a designated slot that evals and generators can act on — not truth.

**Soft sequencing has no first-class home.** "Targetless `press_key` only after `focus_tab` has focused the document" is ordering guidance the vocabulary can only express as `use_when` prose that names a command — reintroducing, on a small scale, the drift this RFC eliminates elsewhere. The honest fragment costs something here.

**A polarity rule to teach.** Authors must learn that `use_when` and `fallback` are two answers to one question. The registration error makes the rule discoverable, but it is one more thing the vocabulary asks authors to hold.

## Rationale and Alternatives

**Status quo — the blob, plus description prose.** The three costs in Motivation recur with any server whose surface is large enough to need routing. vbl re-derived them empirically: its per-tool descriptions already duplicate routing sentences from the blob because a description is the only per-tool surface clients show. Structure removes the duplication instead of maintaining it.

**Fold everything into `description`.** The prose could live in each command's description with no model change. Rejected because it erases the fact's kind: downstream surfaces cannot extract "when to use" from an undifferentiated paragraph, contract tests cannot assert its presence, cross-references stay unchecked, and the eval cannot toggle a field that does not exist. The typed slot is the decomposition thesis in miniature.

**A task taxonomy.** Name tasks at server level ("form entry", "element targeting"), have commands reference them, and derive routing tables per task. This is the RFC 0004/0008/0010 pattern — name the concept once, reference it — and it was seriously considered. Rejected for v1 because the noun does no derivation work: capabilities earned their name because `provides` derivation needed a shared referent; a task name would only group commands that `alternative` edges already connect pairwise. If a real surface accumulates edge sets that clearly want a shared name, the taxonomy is a compatible extension.

**Numeric priorities or tiers.** `priority: 1/2/3` orders commands without saying why, and the *why* is the load-bearing content — an agent needs "when the element is not in the accessibility tree", not "tier 2". Ordering falls out of the fallback edges; justification cannot fall out of an integer.

**Framework-enforced preference.** Reject or warn on escape-hatch calls unless preferred commands were tried first. Rejected: it requires cross-call state the framework does not hold, punishes legitimate direct uses (the agent may *know* the page needs `evaluate`), and converts guidance into gatekeeping — the same wrong layer RFC 0010 declined for capability validity.

**One untyped guidance list per command.** A `Vec<String>` of advice sentences per command would be simpler. Rejected because it recreates the blob at smaller scale: unextractable, uncheckable, unrenderable except as a paragraph. The existing server-level `CommandGuidance` (runnable recipe templates) stays for what it is; this RFC adds the typed facts it cannot express.

## Prior Art

**man page SEE ALSO and "use X instead" notices.** Cross-references placed at the decision point, maintained per page rather than in a manual-wide preamble. The `alternative` edge is this, made checkable.

**`@deprecated(since, note, replacement)` annotations.** Machine-readable directed edges from a dispreferred item to its replacement, with prose justification, rendered by every downstream tool. `alternative` generalizes the shape to non-pejorative routing ("this is not wrong, it is for a different case"); `fallback` keeps the pejorative reading and inverts the direction.

**Rust's `unsafe`.** The canonical named escape hatch: visible to tooling, culturally paired with a justifying `// SAFETY:` comment, and linted when the justification is missing. The `fallback` annotation is the same move — the escape hatch is legal, marked, and owes the reader its reason.

**ARIA's first rule.** "If you can use a native HTML element, do so" — a preference ordering between a semantic path and a powerful low-level one, taught as doctrine because the platform cannot enforce it. Prefer-`ref`-over-`css` is the same rule in miniature.

**Kubernetes scheduling constraints.** `requiredDuringScheduling` versus `preferredDuringScheduling`: the same contract split this RFC pair adopts, with RFC 0010 as the required half and this RFC as the preferred half — and the same design consequence, that only the required half can fail anything.

## Unresolved Questions

- **Soft sequencing.** Whether "do X before Y" deserves first-class edges (`after("focus_tab", when)`) or stays `use_when` prose. The vbl surface has exactly one such fact; one datum is not a design input. Revisit if adopters accumulate them.
- **A derived overview.** Server help could assemble a workflow synopsis from the structured facts — capabilities first, then selection clusters, then escape hatches. Deferred until evals show the locality trade-off costing something.
- **Variant-level selection.** Variants take `fallback` but not `use_when`/`alternative`. The union's variant summaries carry selection prose today; whether variants need the full vocabulary should wait for a case the summary cannot serve.
- **Preamble check escalation.** The backtick residue check ships as a contract rule. If it proves reliable, promoting it to a registration error would make the property universal rather than opt-in.
- **Promotion evidence.** The agent-surface-eval fallback and first-selection metrics are the instrument for this RFC's central bet. Stage promotion past draft should carry a measured comparison — blob versus decomposed surface — not just a shipped implementation.

## Acceptance Tests

- A server declaring `use_when`, `alternative` edges, a command-level `fallback`, a variant-level `fallback`, and a preamble registers successfully; the catalog carries all five and each is covered by the catalog hash (adding or removing any changes the hash).
- Command help renders `Use when:`, `Use instead:`, and `Fallback:` sections from the declarations, and the preferred command renders the derived `Fallbacks:` reverse edge.
- Type help renders the variant fallback condition.
- Registration failures, each with a message naming the command and the offending reference: alternative to an unknown command; alternative to itself; duplicate alternative targets; fallback preferring an unknown command; fallback with an empty prefer list; `use_when` and `fallback` on one command; a union whose variants are all fallbacks; a fallback-preference cycle; empty prose.
- The serving path rejects the same invalid registries.
- The MCP `instructions` field and `getting_started` prompt include the declared preamble.
- `check_guidance_projection` fails a registry whose preamble backticks a catalog command name, and passes the example server.
- The example server demonstrates a selection pair (`fill`/`fill_form`-shaped) and an escape hatch with a derived reverse edge, covered by `contract_tests!`.
