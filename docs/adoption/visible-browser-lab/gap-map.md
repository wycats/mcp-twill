# Gap Map: visible-browser-lab → Twill

Every visible-browser-lab tool walked through Twill's catalog model, with a
disposition for each mismatch. Grounded in vbl commit `29d47d0a` and the
mcp-twill main branch as of 2026-07-05 (all seven RFCs implemented).

Dispositions:

- **catalog models it** — Twill's existing vocabulary covers it; port work only
- **Twill feature** — Twill needs something new; becomes a phase-2 goal
- **server-specific** — stays in vbl; Twill deliberately does not model it

## What fits today

### Domain tools are commands in disguise — catalog models it

vbl's 9 domain tools (`interact`, `console`, `network`, `emulation`,
`performance`, `audit`, `memory`, `screencast`, `artifacts`) each take a
required `operation` enum selecting among 45 total operations, with
per-operation argument requirements enforced at runtime because the published
flat schema cannot express them. In Twill each operation is its own command
(`memory dominators`, `network get`), so per-command required arguments come
back for free and the flat-schema compromise dissolves. The 18 flat tools map
1:1 to commands.

### Effect metadata improves strictly — catalog models it

vbl annotates tools with the MCP annotation quadruple (readOnly / destructive /
idempotent / openWorld). Domain tools are deliberately coarse: the most
conservative operation wins the annotation for the whole tool. Twill classifies
per command, and `CommandSpec::idempotent()` is already a catalog declaration
projected onto the plan. The port makes vbl's effect story more precise than
what it ships today.

### The hand-rolled help tool — catalog models it

vbl's `help` tool (topic routing, preferred-tool result schemas) is a manual
version of what Twill generates. It disappears wholesale.

### Structured errors with recovery actions — server-specific, portable as-is

`BrowserToolError { code, message, recovery? }` with 18 snake_case codes and 9
machine-readable recovery verbs is a house convention that rides inside tool
results. Twill's response envelope carries it without modeling it. One cleanup
the port should do in passing: the surface layer mints an ad-hoc
`unsupported_operation` error not present in the enum; Twill's unknown-command
handling (with `nearest_commands` suggestions) replaces that path.

## Gaps requiring Twill features (phase-2 candidates)

### 1. Sum-type arguments (`element_target`)

`click`, `fill`, `type_text`, `press_key`, `fill_form` fields, and several
`interact` operations target elements with a two-variant union: `{ref}` or
`{css, frame_ref?}`. Twill's typed template arguments have no union shape; the
port would degrade to two optional arguments plus runtime XOR validation —
exactly the "published schema says less than the real contract" pattern the
contract tests exist to kill.

**Disposition: Twill feature.** Argument unions with variant-aware projection
into both the JSON schema (`oneOf`) and the catalog. This is the single most
load-bearing gap: element targeting appears in every interactive command.

### 2. Session-scoped preconditions

Every tool except `help` requires `agent_session_id`; every page-scoped tool
requires an owned, active `tab_id`. Today that contract lives in three
unstructured places: prose in `SERVER_INSTRUCTIONS`, a boilerplate sentence
appended to every VS Code tool description, and runtime errors
(`unknown_session`, `tab_not_owned`, ...). The catalog cannot say "this command
requires a session lease."

**Disposition: Twill feature.** A declared precondition vocabulary on commands
(at minimum: named capability requirements like `session`, `owned-tab`) that
projects into generated help and the catalog. Enforcement stays in vbl's broker
process — the declaration is a promise, like every catalog fact; Twill renders
it and contract-checks its projection, and the runtime can pre-validate that
the arguments carrying the capability are present.

### 3. Ambient context injection (`workspace_root`)

vbl injects the workspace root into `start_session` from transport metadata
(Codex `sandboxCwd`) or environment variables — never as a tool argument.
Twill's RFC 0007 resolver already models exactly this problem (client roots,
env observation, containment), but the current integration binds workspaces to
*path arguments*. vbl needs the resolved root delivered to the *handler* even
when no argument carries a path (artifact export containment happens
server-side).

**Disposition: Twill feature, small.** Expose the resolved workspace set on the
invocation plan / handler context so a server can consume roots without
declaring a path argument. Most of the machinery shipped in RFC 0007; this is a
projection gap.

### 4. Structured workflow guidance (`SERVER_INSTRUCTIONS`)

One prose blob carries workflow sequencing: start_session first, snapshot
before acting, CSS/evaluate are escape hatches. Twill has per-command guidance
and usage headers, but no way to express cross-command workflow ("do X before
Y") or preference ordering ("prefer ref targeting; css is a fallback").

**Disposition: split.** Per-command guidance and precondition declarations
(gap 2) absorb most of the blob. A residual server-level preamble is already
supported (server help text). Whether Twill should model "escape hatch"
preference ordering as structured metadata is a phase-2 design question — the
eval's `css_fallback`/`evaluate_fallback` scoring gives us a way to measure
whether structure beats prose here.

### 5. VS Code manifest generation

vbl generates its VS Code extension manifest (`languageModelTools` in
package.json) from the same catalog via xtask, with schema-equality CI. A
help+run compact surface changes what the extension contributes: VS Code has no
"run" indirection today, so either the extension keeps N discrete tools (and
Twill's catalog must project back out to discrete definitions — which
`baseline_catalog()` proves is mechanical) or the extension adopts the compact
surface too.

**Disposition: defer to the generated-extension epic.** The port keeps vbl's
MCP surface and the VS Code manifest decoupled; nothing breaks. This gap is the
front door to the "generate a VS Code front-end from Twill definitions" idea
already recorded in exo, and deserves its own design pass rather than a
phase-2 slice.

## Explicitly server-specific (no Twill work)

- **Lease enforcement** — the broker is a separate process shared across
  clients; ownership checks (26 call sites) stay there regardless of what the
  catalog declares.
- **Observation modes** (`observe: none|diff|snapshot`) — a plain enum
  argument on ~6 commands; Twill's typed args handle it today.
- **Artifact ids** — plain string arguments with server-side existence checks;
  nothing for the catalog to add beyond good help text.
- **Synthetic eval server** — agent-surface-eval fakes the browser behind the
  real catalog; the port swaps the served catalog and keeps the fixtures,
  scoring, and codex harness untouched.

## Phase-2 goal candidates, ordered

1. **Argument unions** (gap 1) — load-bearing for every interactive command
2. **Declared preconditions** (gap 2) — replaces prose with catalog facts;
   biggest token-quality lever after schema compression
3. **Handler-visible workspace roots** (gap 3) — small; completes RFC 0007
4. **Guidance decomposition** (gap 4) — partially design work; measurable via
   the eval's fallback scoring

Gap 5 routes to the generated-extension epic, not phase 2.
