<!-- exo:11 ulid:01kwxg9g82ajd9shp9ce775dtm -->

# RFC 0011: Guidance Decomposition

- Status: Accepted
- Area: command model, catalog, help
- Target milestone: v0.4
- Depends on: RFC 0001 (authoritative command surface), RFC 0008 (named argument types), RFC 0010 (declared preconditions)

## Summary

RFC 0010 moved the hard half of workflow prose into declarations: preconditions that must hold or the call fails. This RFC moves the soft half: the guidance that tells an agent which command to choose when several could plausibly serve, and which mechanisms are escape hatches to exhaust last. Commands gain a selection criterion (`use_when`), validated cross-references to the commands that serve neighboring cases (`alternative`), and a fallback annotation that names the preferred path and the condition that justifies bypassing it — applicable to whole commands and, via RFC 0008, to union variants. The prose that survives decomposition moves to a declared server preamble, and an opt-in contract check flags explicit backticked command references that should move back onto command declarations.

Soft is the operative word. A precondition can be pre-validated, and its violation fails the call. A selection preference cannot fail anything: calling the escape hatch first is legal, merely worse. The framework therefore validates the *references* (an alternative must name a real command), projects the facts onto every surface, and enforces nothing. Whether the structured form actually improves agent selection is a measurable question, and this RFC treats it as one.

Guidance references canonical catalog operation ids, not public MCP tool names. A later direct or grouped surface translates those references through its checked operation map; generated hosts consume the translated structured facts rather than searching prose. This keeps one guidance graph usable on effect-lane, native, subset, and host-packaged surfaces without teaching one surface's call syntax on another.

## Motivation

The motivating artifact is, again, visible-browser-lab — specifically the `SERVER_INSTRUCTIONS` paragraph every vbl conversation pays for. It is worth reading the blob against the vocabulary the framework now has, sentence by sentence:

| Sentence (abbreviated) | What kind of fact | Where it belongs |
| --- | --- | --- |
| "Call browser operations directly. Conversation-aware hosts select the browser session automatically." | Serving-profile behavior | RFCs 0015, 0016, and 0019. Not selection guidance. |
| "Omit `agent_session_id` unless this conversation received `session_required` and established an explicit fallback." | Resource binding and recovery | RFCs 0014 and 0016, with host-specific rendering in RFC 0019. Not free prose. |
| "Use only `tab_id` values owned by the selected session." | Resource ownership | RFC 0012 resource resolution. Retired from selection guidance. |
| "Inspect an unfamiliar page with `snapshot`, then act through its element references." | Selection | `use_when` on `snapshot` |
| "Use `fill` for one ordinary field." | Selection | `use_when` on `fill` |
| "Use `fill_form` for two or more controls..." | Selection + contrast | `use_when` on `fill_form`; `alternative` edge from `fill` |
| "Use `type_text` for contenteditable controls..." | Selection | `use_when` on `type_text` |
| "Use `press_key` with a target for named keys..." | Selection | `use_when` on `press_key` |
| "Use targetless `press_key` and `click_at` only after `focus_tab` has focused the document." | Soft sequencing | `use_when` on those operations; version 1 keeps the single observed sequence local |
| "Use `wait_for` for asynchronous state and `screenshot` for visual appearance." | Selection | `use_when` on each |
| "Use `console` and `network` for runtime diagnosis." | Selection | `use_when` on each |
| "Use `help` to select an operation in a specialized domain." | Selection | `use_when` on the application help operation; framework help remains a separate RFC 0015 projection. |
| "Routine actions preserve the user's active application." | Behavioral contract | Server preamble. Stays prose. |
| "Target activation is reserved for `focus_tab`... manual inspection or handoff." | Selection (reserved-for) | `use_when` on `focus_tab` |
| "CSS and evaluate are escape hatches only when snapshot and the named semantic tools cannot represent the required state; do not use them to verify a semantic action." | Preference ordering | `fallback` on the `css` variant and the `evaluate` command |

The released v0.4.9 paragraph mixes four layers: serving-profile behavior, resource authority and recovery, command selection, and one server-wide behavioral promise. Its `SERVER_INSTRUCTIONS` bytes are identical to v0.4.8, while the v0.4.9 surface changes screencast descriptions and schemas; current-surface evidence therefore pins v0.4.9 without changing this RFC's selection decomposition. RFCs 0012–0019 give the first two layers typed homes. This RFC owns the third and leaves only the genuine server-wide promise in the preamble. Stranding command-selection facts at server level has the same three costs RFC 0010 catalogued for preconditions:

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
        .alternative("fill_form", "updating two or more controls in one pass");
        // ...
});

server.command("fill_form", |command| {
    command
        .summary("Fill two or more referenced form controls sequentially")
        .use_when("updating two or more controls, including combined select and checkbox changes");
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
            ["snapshot", "console", "network"],
            "semantic snapshots and diagnostics do not expose the state you need",
        );
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
let registry = CommandRegistry::build("vbl", "Visible Browser Lab", |server| {
    server.preamble(
        "Routine actions attach to the owned target and preserve the user's active application.",
    );
    // ...command declarations...
})?;
```

The preamble projects into the MCP server instructions and server help. An opt-in contract check supplies one narrow maintenance guard: the preamble must not contain a backticked catalog command name. Such a reference is a strong signal that the fact belongs *on that command* — as a `use_when`, an `alternative`, a `fallback`, or an RFC 0010 capability. The check does not prove that arbitrary prose contains no routing advice; review and the paired VBL evaluation own that semantic judgment. It does make explicit catalog-command references a maintained structural property instead of a one-time cleanup convention.

### What the framework checks, and what it does not

The framework validates references and projection: every `alternative` and every `fallback` preference must name a command the catalog defines, so a routing edge cannot dangle. It does not check the prose conditions (no machine can), and it does not enforce the ordering — a call to an escape hatch plans and runs like any other call. Selection quality is not a plan-time property; it is an outcome, and the eval harness is the instrument that measures it. This is the same honesty boundary RFC 0010 drew: declarations the framework can make consistent and visible, but not true.

### How Agents Should Learn This

Agents should encounter selection guidance at the decision point. `Use when:` explains the command's ordinary case, `Use instead:` points to a neighboring operation for a different case, and `Fallback:` marks a legal escape hatch whose preferred paths should normally be exhausted first. These are recommendations rather than authorization or workflow state: a valid fallback call remains valid even when no preferred call preceded it.

The catalog operation id is the stable referent, while each active serving surface renders the call shape the agent can actually make. Direct tools use their public tool names, grouped tools include the selector, and generated hosts consume the same translated edge. Agents should therefore follow the active surface's rendered guidance rather than retain catalog ids, command strings, or another surface's spelling. The server preamble teaches only behavior that genuinely applies to the surface as a whole.

## Reference-Level Explanation

### Model

`CommandSpec` gains three fields:

```rust
pub struct CommandSpec {
    // ...
    /// One sentence: when this command is the right choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_when: Option<String>,
    /// Commands serving neighboring cases, with the condition that routes there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Alternative>,
    /// Marks this command as an escape hatch for a preferred path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}
```

The server model gains the corresponding optional preamble:

```rust
pub struct ServerSpec {
    // ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
}
```

Authors declare it with `ServerBuilder::preamble(text)`. All new fields serialize with serde defaults and skip-if-empty, so existing catalogs are unchanged byte-for-byte until a server adopts the vocabulary.

The public field names `Alternative::command` and `Fallback::prefer` remain unchanged for source and wire compatibility. Their values are canonical catalog operation ids—the current command path joined by spaces—not surface tool names. Surface declarations never rewrite the catalog model; they translate these ids when rendering.

The low-level model and ergonomic builder expose the same facts while retaining their established receiver conventions:

```rust
impl CommandSpec {
    pub fn use_when(self, text: impl Into<String>) -> Self;
    pub fn alternative(
        self,
        command: impl Into<String>,
        when: impl Into<String>,
    ) -> Self;
    pub fn fallback(
        self,
        prefer: impl IntoIterator<Item = impl AsRef<str>>,
        when: impl Into<String>,
    ) -> Self;
}

impl CommandBuilder {
    pub fn use_when(&mut self, text: impl Into<String>) -> &mut Self;
    pub fn alternative(
        &mut self,
        command: impl Into<String>,
        when: impl Into<String>,
    ) -> &mut Self;
    pub fn fallback(
        &mut self,
        prefer: impl IntoIterator<Item = impl AsRef<str>>,
        when: impl Into<String>,
    ) -> &mut Self;
}

impl ServerBuilder {
    pub fn preamble(&mut self, text: impl Into<String>) -> &mut Self;
}

impl CommandRegistry {
    pub fn declare_preamble(self, text: impl Into<String>) -> Self;
    pub fn preamble(&self) -> Option<&str>;
}

impl Variant {
    pub fn fallback(self, when: impl Into<String>) -> Self;
}
```

Under the proposed API, both fallback builders copy each preference into the owned declaration. After implementation, arrays of string literals, borrowed slices such as `&[&str]`, and owned `Vec<String>` values are equivalent authoring forms and normalize to the same ordered `Vec<String>`. Implementing this proposal changes the current shipped iterator item bound from `Into<String>` to `AsRef<str>`; acceptance coverage must compile the borrowed-slice form before this RFC advances. Ordinary `String` and string-reference collections remain source-compatible. A custom item type that implements only `Into<String>` must implement `AsRef<str>` or be converted explicitly to `Vec<String>` before calling `fallback`; this is a Rust source migration with no serialized or runtime change.

Every authored guidance string is static public catalog text. After confirming
that it is not whitespace-only, registration accepts at most 1,024 Unicode
scalar values and rejects C0 controls, DEL, C1 controls, and the fixed
presentation-unsafe set U+061C, U+200E–U+200F, U+2028–U+202E,
U+2060–U+206F, and U+FEFF. The text is otherwise preserved byte-for-byte; it
is never trimmed, escaped, or truncated. The same rule covers `use_when`,
every alternative/fallback condition, variant fallback conditions, and the
server preamble, so every projection has one accepted spelling.

The low-level `CommandSpec`, `Variant`, and `CommandRegistry` methods transform public declaration
values: `use_when`, command `fallback`, variant `fallback`, and
`CommandRegistry::declare_preamble` replace their visible optional field, while
`alternative` appends one ordered edge. `CommandRegistry::preamble` returns the
selected server preamble without creating a second authority. The mutable
`CommandBuilder` and `ServerBuilder` are finalizing builders instead.
Their `use_when`, `fallback`, and `preamble` slots may each be authored once; a
second assignment records a build error even when the values agree. An
`alternative` remains an ordered keyed addition and duplicate targets fail
registration. Authored alternative order and fallback preference order remain
public and hash-significant. Derived reverse-fallback entries are sorted by
the fallback command's canonical operation id, so registry insertion order
never changes help or a compiled surface.

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
- empty, over-1,024-scalar, control-bearing, or presentation-unsafe prose in
  `use_when`, any `when`, a variant fallback, or the preamble;
- repeated mutable-builder assignment of `use_when`, command `fallback`, or
  the server preamble, even when the repeated values agree.

### Projection

- **Catalog.** Operations carry `useWhen`, `alternatives`, and `fallback`; variants carry `fallback`; the server entry carries `preamble`. Every edge target remains a catalog operation id. All facts are covered by the catalog hash.
- **Command help.** Usage text renders `Use when:` after the description, then `Use instead:` with one line per alternative, then `Fallback:` with the preferred operations and condition. Operations named in another operation's `prefer` list render a derived `Fallbacks:` section.
- **Type help.** A fallback variant renders its condition inline: `` `css` (fallback — the element is not represented in the accessibility tree) ``.
- **Server surfaces.** The preamble renders in server help after the description and joins the framework-written sentence in the MCP `instructions` field and the `getting_started` prompt.
- **Native direct tools.** RFC 0015 translates every referenced operation id through the active surface. A direct target renders as its public tool name.
- **Native grouped tools.** A grouped target renders as its public tool plus selector value. Group descriptions list each selector with the member summary and `use_when`; alternative, fallback, and reverse-fallback edges retain their conditions while using translated call shapes.
- **Subset surfaces.** A surface may omit an operation only when no exposed guidance edge can steer a caller to it. RFC 0015 validates this closure and surface-filtered help never renders an omitted target as callable.
- **Generated hosts.** RFC 0019 consumes the canonical translated guidance snapshot. Host-specific prose may be layered through structured operation/resource segments, but generators never parse raw guidance text to discover a tool or selector.
- **Rendering order.** Direct and grouped descriptions use one deterministic order: authored leading description, `use_when`, alternatives in authored order, fallback preferences in authored order, reverse fallbacks in canonical fallback-operation order, then later host-owned suffixes. A surface may shorten prose only through an explicit hash-covered dialect that preserves every structured edge elsewhere in its snapshot.

Guidance is intentionally public, static catalog content. Its authored prose and translated references may appear in catalog, help, MCP instructions, prompts, native descriptions, and generated host artifacts. Runtime arguments, request metadata, private invocation context, application outcomes, and observed model behavior are never guidance inputs. Guidance therefore adds no per-invocation field to plans, previews, fingerprints, responses, events, or framework-owned logs.

Diagnostics and steering are otherwise untouched. No error changes shape, and no plan-time check is added: there is nothing to check, because preference violations are not errors.

### Contract Checks

A `check_guidance_projection` rule joins the contract suite for catalog and command-help agreement:

- every command declaring `use_when`, `alternatives`, or `fallback` renders them in its help text;
- every command named in a `prefer` list renders the derived `Fallbacks:` section;
- the preamble contains no backticked catalog command name.

The backtick convention is deliberate. Matching bare words would flag English ("click", "help") that happens to collide with command names; a backticked name in prose is an intentional command reference, and intentional command references are what the decomposition exists to relocate.

RFC 0015's `check_native_surface_projection` and RFC 0019's host-adapter contract check extend this evidence across translated direct/grouped call shapes, subset closure, deterministic description ordering, and generated artifacts. Those checks consume the structured guidance graph; they do not duplicate its declarations.

### Required Invariants

- Every `alternative` and `fallback` reference resolves to a catalog command; registration and serving both reject danglers.
- Reverse fallback edges in help are derived from declarations, never authored.
- Adding, removing, or editing any selection fact changes the catalog hash.
- The framework enforces no selection ordering at plan or run time.
- The preamble projects verbatim; the opt-in contract check flags backticked catalog-command references but does not infer routing semantics from arbitrary prose.
- Guidance edges always retain catalog operation ids as their authority and translate through an active surface without rewriting the catalog.
- Direct, grouped, subset, and generated-host descriptions preserve every exposed selection edge in deterministic order.
- Raw prose is never parsed to infer an operation, tool name, selector value, or resource carrier.
- Guidance contains only authored static catalog facts; runtime values never enter it, and it adds no per-invocation plan, preview, fingerprint, response, event, or framework-log field.
- Every guidance string is accepted once as bounded display-safe static text and then projected byte-for-byte; builders cannot grant a later scalar assignment hidden last-write authority, and derived reverse-edge order never depends on registry insertion.


### Implementation Phases

1. Preserve the shipped command/variant guidance model, validation, catalog projection, help rendering, preamble, and core contract checks.
2. Treat every alternative and fallback target as a canonical catalog operation id while retaining the existing serialized field names.
3. Add RFC 0015 direct/grouped translation, deterministic description ordering, surface-filtered help, and subset closure over guidance edges.
4. Add RFC 0019 structured host guidance segments and generated-artifact checks that reject raw structural names in host prose.
5. After RFC 0015's evidence-only v0.4.9 fixture bootstrap lands, consume its manifest and released `surface-catalog.json`/`vscode-package.json` observations, then author the structured guidance graph in `crates/mcp-twill/tests/support/vbl.rs`. RFC 0015 owns the importer, provenance, and frozen bundle; this RFC owns the declarations and guidance comparison.
6. After RFC 0015 and RFC 0019 land the translated native and generated-host projections, VBL owns the paired control/treatment evaluation used for this RFC's later `Implemented` lifecycle gate.

### Acceptance Tests

Acceptance lives in `crates/mcp-twill/tests/guidance.rs`. The owner-local
landing proves declaration, validation, catalog identity, help, preamble, and
contract-check behavior. RFC 0015 owns native direct/grouped/subset
translation in `native_surfaces.rs`; RFC 0019 owns generated-host translation
in `host_adapters.rs`; and the promotion evidence below remains a downstream
VBL evaluation gate. None of those integrations may infer structure from
prose or replace catalog operation ids as the guidance authority.

- RFC 0015's v0.4.9 manifest validates before VBL guidance parity; the test reads released descriptions and instructions from `surface-catalog.json` and `vscode-package.json`, while the new structured declarations remain visibly authored in `tests/support/vbl.rs`.
- A server declaring `use_when`, `alternative` edges, a command-level `fallback`, a variant-level `fallback`, and a preamble registers successfully; the catalog carries all five and each is covered by the catalog hash (adding or removing any changes the hash).
- Equivalent low-level `CommandSpec`/`Variant` construction with `CommandRegistry::declare_preamble` observed through `preamble()`, and mutable `CommandBuilder`/`ServerBuilder` authoring produce byte-identical catalog facts, help, hashes, and validation failures.
- Low-level visible-value setters retain only their final `use_when` or fallback value, while mutable command/server builders reject repeated `use_when`, command `fallback`, or preamble assignment even when equal. Alternative append order and fallback preference order remain byte-visible and hash-significant; reversing registry insertion alone leaves canonically sorted reverse-fallback help unchanged.
- Command fallback preferences authored from a literal array, borrowed slice, and owned string vector normalize to the same ordered catalog declaration and hash.
- Legacy `CommandSpec`, `Variant`, and `ServerSpec` JSON without the new guidance fields and explicit empty/`None` values normalize to byte-identical catalog data and hash input; guidance projection remains absent until adopted.
- Projection and execution fixtures vary model arguments, request metadata, private invocation context, and application outcomes while the catalog/help guidance remains byte-identical; serialization probes find no guidance-owned per-invocation field in plans, previews, fingerprints, responses, events, or framework-owned logs.
- Command help renders `Use when:`, `Use instead:`, and `Fallback:` sections from the declarations, and the preferred command renders the derived `Fallbacks:` reverse edge.
- Type help renders the variant fallback condition.
- Registration failures, each with a message naming the owning command/type/server and the offending field or reference: alternative to an unknown command; alternative to itself; duplicate alternative targets; fallback preferring an unknown command; fallback with an empty prefer list; `use_when` and `fallback` on one command; a union whose variants are all fallbacks; a fallback-preference cycle; repeated mutable-builder scalar assignment; and empty, 1,025-scalar, C0, DEL, C1, or presentation-unsafe prose. Exactly 1,024 safe scalars succeed and project byte-for-byte without trimming or escaping.
- The serving path rejects the same invalid registries.
- Planning and execution fixtures call a preferred operation and its declared fallback directly, in both declaration orders and before and after calls to the alternative; every valid call reaches its handler and succeeds. Guidance state is never consulted as an execution prerequisite.
- The MCP `instructions` field and `getting_started` prompt include the declared preamble.
- `check_guidance_projection` fails a registry whose preamble backticks a catalog command name, and passes the example server.
- The example server demonstrates a selection pair (`fill`/`fill_form`-shaped) and an escape hatch with a derived reverse edge, covered by `contract_tests!`.
- RFC 0015 acceptance maps one alternative directly and one through a grouped selector, rejects a subset that omits an exposed guidance target, and proves surface-filtered help and description ordering contain no untranslated operation spelling.
- RFC 0019 acceptance regenerates host descriptions from the translated snapshot and fails when raw host prose spells an operation, tool, selector-qualified call, or resource carrier that should be a structured segment.

### Promotion Evidence

Design acceptance does not depend on a model evaluation: the ownership, validation, projection, and compatibility rules above are independently reviewable. Promotion to `Implemented` does require downstream evidence for the behavioral claim that decomposing the released VBL guidance preserves or improves command selection.

VBL runs the complete 30-fixture `agent-surface-eval` suite twice from one source commit. The control uses the released v0.4.9 surface at peeled commit `f2bd478fa5506df7530b3fd60d7d0114f0ed3160`: its server-level guidance blob is byte-identical to v0.4.8, while its screencast descriptions and schemas reflect the current release. The new guidance fields remain absent from the control. The treatment keeps the same operations, schemas, fixtures, skill, model id, reasoning effort, authentication source, and infrastructure-retry policy while replacing routing prose with this RFC's structured declarations and minimal preamble. Both runs are fresh full-suite runs rather than selected or resumed trials. Their evidence bundle records the VBL commit, model and reasoning settings, catalog and surface hashes, JSON summaries, and per-fixture tool sequences so the comparison cannot hide unrelated surface drift.

The control must reproduce the harness's existing acceptance floor: 30 trials, at least 27 successful tasks, at least 26 correct first selections, zero semantic fallback violations, and zero unowned backend actions. The treatment must meet the same absolute floor, retain zero violations/actions, and have no lower successful-task or correct-first-selection count than that paired control. The evidence reports every per-fixture selection change even when aggregate counts pass. A failure keeps the RFC at `Accepted` while the declarations, projections, or derived-overview design are revised; it does not weaken the thresholds or silently restore the blob.

## Drawbacks

**Decomposition trades a synopsis for locality.** The blob, whatever its costs, gave an agent one paragraph with the whole workflow shape. The decomposed facts are only synoptic in the catalog, which an agent may not read. If the paired promotion evidence shows worse selection without a narrative overview, a derived overview assembled from the structured facts is the next design move rather than restoring authored routing prose.

**The conditions are still prose.** `use_when` and `when` texts are unchecked and can drift from behavior, like every summary and description in the catalog. What the structure buys is position (the fact sits where the decision is made), reference integrity, and a designated slot that evals and generators can act on — not truth.

**Soft sequencing has no first-class home.** "Targetless `press_key` only after `focus_tab` has focused the document" is ordering guidance the vocabulary can only express as `use_when` prose that names a command — reintroducing, on a small scale, the drift this RFC eliminates elsewhere. The honest fragment costs something here.

**A polarity rule to teach.** Authors must learn that `use_when` and `fallback` are two answers to one question. The registration error makes the rule discoverable, but it is one more thing the vocabulary asks authors to hold.

## Rationale And Alternatives

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

No architectural question blocks the initial guidance boundary. Version 1 deliberately leaves the single observed sequencing fact in local `use_when` prose, derives no workflow overview until the paired evaluation demonstrates a locality cost, limits variant-level structure to `fallback`, and keeps the preamble residue rule in opt-in contract tests. The exact evidence required for `Implemented` status is a lifecycle gate above rather than an unresolved API decision.

## Future Possibilities

A failed or regressed paired evaluation could motivate a derived workflow overview assembled from capability/resource establishment, selection clusters, and escape-hatch ladders without restoring a hand-written server blob. Repeated sequencing examples could justify first-class `after` edges; a union whose summaries cannot route callers could justify variant-level `use_when` or alternatives; and sustained clean contract evidence could justify promoting the preamble residue rule to registration. Localized rendering catalogs or named selection tasks remain compatible extensions once more than one adopter needs them.
