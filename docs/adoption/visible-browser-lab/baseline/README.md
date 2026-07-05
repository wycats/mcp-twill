# visible-browser-lab Baseline

Frozen measurements of visible-browser-lab's current MCP surface, captured
before any Twill porting work. Phase-4 comparisons run against these numbers;
do not regenerate them from a newer vbl commit without recording why.

## Provenance

- Source repo: `~/plugins/visible-browser-lab`
- Commit: `29d47d0a8a7d28fc7e9f1f6db492b2253c52a160` (clean tree)
- Captured: 2026-07-05

## Files

- `surface-catalog.json` — the exact payload an MCP client sees: server
  instructions plus all 27 tool definitions with input/output schemas and
  annotations. Produced by
  `cargo run -p visible-browser-lab --bin visible-browser-lab-mcp -- surface catalog`,
  which serializes the same structs `list_tools` serves.
- `catalog-measurement.json` — vbl's own token measurement, produced by
  `cargo xtask catalog-measurement`.

## Numbers

| Measurement | Value |
|---|---|
| Tools (hybrid surface, served today) | 27 |
| Tools (baseline flattened comparison) | 63 |
| Domain operations behind the 9 domain tools | 45 |
| Hybrid catalog tokens | 15,002 |
| Flattened catalog tokens | 30,307 |
| Hybrid/flattened ratio | 0.495 |

Tokenizer: `o200k_base` via tiktoken-rs, `encode_with_special_tokens`, applied
to the serialized catalog JSON. A Twill comparison must use the same encoder
and tokenize the equivalent surface: generated tool definitions (help + run +
effect-lane tools) plus the `cli://catalog` resource if agents are expected to
read it during discovery.

## Context

vbl already ran one compression experiment: 63 flat tools were collapsed into
27 (9 domain tools carry 45 operations behind an `operation` enum), gated on a
maximum token ratio of 0.6. The Twill port is the next step on the same curve.
The number to beat for definition payload is **15,002 tokens**.

The eval baseline (agent-surface-eval: 30 fixtures, task success rate +
first-selection rate, driven by live `codex exec`) is deferred; when it runs,
record scores here before the Twill surface exists.
