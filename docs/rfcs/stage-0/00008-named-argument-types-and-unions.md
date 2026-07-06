<!-- exo:8 ulid:01kwtevarxm732m5m2frnv7gt7 -->

# RFC 8: Named Argument Types And Unions

- Status: Draft
- Area: catalog model, argument binding, schema projection, help generation
- Target milestone: v0.2
- Depends on: RFC 0001 (authoritative command surface), RFC 0006 (author ergonomics)

## Summary

This RFC gives the catalog a vocabulary for declaring named argument types: unions of record variants whose fields are scalars, constant discriminators, or references to other named types. Commands reference these types by name from `ArgSpec`, the framework matches incoming values against variants at planning time, and every projection surface — the `cli://catalog` resource, generated JSON schemas, help text, diagnostics, and the invocation fingerprint — renders the same declaration.

Today an argument is one of five scalar types, and anything structured must be declared as `Json`, which the framework cannot validate, document, or diagnose. After this RFC, a catalog can declare a type like `element_target` once — "either an accessibility `ref`, or a `css` selector with an optional `frame_ref`" — and reference it from fifteen commands. The framework validates values against the union, records which variant matched on the plan, explains mismatches variant by variant, and emits shared `$defs` entries in generated schemas instead of fifteen copies.

The design deliberately supports two matching styles with one mechanism: structural variants (distinguished by which required fields are present) and discriminated variants (distinguished by a constant field). Ambiguous unions are rejected when the catalog is built, not discovered at dispatch time.

## Motivation

The five scalar `ArgType`s carried the framework through its own examples, but real tool surfaces have structured arguments. The motivating case is visible-browser-lab, a production MCP server with 27 tools whose interactive commands all accept an *element target*: either an accessibility reference obtained from a snapshot, or a CSS selector with an optional frame reference. Its wait command accepts one of five *conditions*, each tagged by a `kind` field. Its form-fill command accepts an *array* of tagged field actions, each of which itself contains an element target.

Under the current model every one of those arguments must be `ArgType::Json`. That single word costs the framework its core promises at exactly the moments they matter most:

- **The catalog stops being authoritative.** The real shape of the argument lives in prose, or in the handler's deserialization code, or nowhere. An agent reading `cli://catalog` sees `json` and learns nothing.
- **Validation degrades to the handler.** The framework's planning pipeline — the layer that exists to catch malformed calls before side effects — waves structured values through. Every server reimplements shape checking, inconsistently.
- **Help cannot teach.** Generated usage text renders `- $args.target: Json, required` where it should render the two ways to target an element.
- **Diagnostics go generic.** A mistyped target produces whatever the handler's serde error says, not a framework diagnostic that names the variants and what each one was missing.

There is also a hard-won compatibility lesson to encode. visible-browser-lab originally expressed each domain operation as a top-level `oneOf` variant. That design validated precisely and failed in production: model-facing schema pipelines (VS Code's chat layer among them) type arguments from the top-level `properties` map, and with an empty top-level map every argument reached the server as a string. The rewritten surface keeps composition at the *property* level, where those pipelines preserve it. This RFC adopts that constraint as a design rule rather than leaving each adopter to rediscover it: unions attach to arguments, never to a command's whole input.

## Guide-Level Explanation

A catalog author declares a named type once, alongside workspaces and commands:

```rust
let catalog = CommandCatalog::builder()
    .declare_type(
        TypeDecl::union("element_target", "A way to identify an element on the page")
            .variant(
                Variant::new("ref", "Accessibility reference from a snapshot")
                    .field(Field::string("ref", "Element reference")),
            )
            .variant(
                Variant::new("css", "CSS selector fallback")
                    .field(Field::string("css", "Playwright-style selector"))
                    .field(Field::string("frame_ref", "Frame to scope the selector").optional()),
            ),
    )
```

Commands then reference the type by name where they previously chose a scalar:

```rust
.command(
    CommandSpec::new("click", "browser click $args.target", "Click one element")
        .arg(ArgSpec::named("target", "element_target", "The element to click")),
)
```

From the author's perspective that is the whole feature: declare once, reference by name. Everything else is the framework keeping its existing promises for the new shapes.

When a call arrives, the planner matches the value against the union's variants. A variant matches when every required field is present with the right type and every constant field has its constant value. The matched variant's name is recorded on the plan — handlers dispatch on it instead of re-inspecting the JSON, and the permission preview can say "targeting by `css`" rather than echoing a blob.

When no variant matches, the framework explains the failure per variant. An agent that sent `{"selector": ".button"}` sees:

```
argument `target` does not match `element_target`:
  not `ref`: missing required field `ref`
  not `css`: missing required field `css`
```

That diagnostic names the declared variants and the first blocking field for each — the same steering philosophy as workspace mismatch errors, applied to types.

Discriminated unions use the same mechanism with a constant field:

```rust
TypeDecl::union("wait_condition", "A condition to wait for")
    .variant(
        Variant::new("delay", "Wait a fixed duration")
            .field(Field::constant("kind", "delay"))
            .field(Field::number("duration_ms", "How long to wait")),
    )
    .variant(
        Variant::new("text", "Wait for text to appear or disappear")
            .field(Field::constant("kind", "text"))
            .field(Field::string("text", "Text to wait for"))
            .field(Field::enumerated("state", &["visible", "hidden"], "Target state").optional()),
    )
```

Types compose by reference. A form-field action contains an element target:

```rust
TypeDecl::union("form_field", "One form control to fill")
    .variant(
        Variant::new("text", "Set a text control's value")
            .field(Field::constant("kind", "text"))
            .field(Field::reference("target", "element_target", "The control"))
            .field(Field::string("value", "The value to set")),
    )
```

and the command's argument is an array of them via the existing `repeated` flag: `ArgSpec::named("fields", "form_field", "Controls to fill").repeated()`. Composition always goes through a name. There are no anonymous inline unions: every level of structure is declared, so every level has a name that help text, diagnostics, catalog entries, and `$defs` can use.

### How Agents Should Learn This

Agents learn types through the same two surfaces they already use: help and the catalog resource.

Generated help renders a named type's variants as indented alternatives under the argument that uses them, with each variant's summary and field list. The usage line for `click` teaches both targeting styles at the point of use; an agent never needs to consult a separate type glossary to make its first call. When several arguments in one command share a type, help renders the variants once and references them for the other arguments.

The `cli://catalog` resource gains a `types` section listing every declared type with its variants and fields. Argument entries reference types by name. An agent that plans calls from the catalog (rather than from help) sees the same declaration the planner enforces — one source of truth, projected twice.

Mismatch diagnostics are the third teaching surface and the most important one for recovery: the per-variant explanation shows an agent exactly which fields would make its value match, preserving the data it already has. The diagnostic should always name the argument, the type, every variant, and each variant's first blocking problem. Generic phrasing ("expected one of 2 variants") is a bug.

## Reference-Level Explanation

### Type declarations

```rust
pub struct TypeDecl {
    pub name: String,          // unique within the catalog
    pub summary: String,
    pub variants: Vec<Variant>, // at least one
}

pub struct Variant {
    pub name: String,          // unique within the type
    pub summary: String,
    pub fields: Vec<Field>,
}

pub struct Field {
    pub name: String,          // unique within the variant
    pub summary: String,
    pub required: bool,        // default true
    pub shape: FieldShape,
}

pub enum FieldShape {
    String,
    Bool,
    Number,
    Constant(String),          // matches exactly this string value
    Enumerated(Vec<String>),   // matches one of these string values
    Reference(String),         // named type, matched recursively
    Repeated(Box<FieldShape>), // array of the inner shape
}
```

`ArgSpec` gains a new value type variant referencing a declaration:

```rust
pub enum ArgType {
    String, Path, Json, Bool, Number,
    Named(String),             // references a TypeDecl by name
}
```

A single-variant `TypeDecl` is permitted and useful: it declares a named record with no union semantics.

### Registration-time validation

Catalog construction fails with a `FrameworkError` when:

- two type declarations share a name, or a declared type is never referenced by any command argument or reachable field (dead types are drift);
- an `ArgType::Named` or `FieldShape::Reference` names a type that is not declared;
- a reference cycle exists among type declarations (`form_field` → `element_target` is fine; any path from a type back to itself is not);
- two variants of one type are indistinguishable: neither has a constant field that the other lacks or contradicts, and each variant's required field names are a subset of the other's. Distinguishability is decidable at registration because shapes are closed; the framework must reject ambiguity at build time rather than resolve it by variant order at dispatch time.

The existing template placeholder check extends to named types: a placeholder whose `ArgSpec` is `Named` binds the matched value as a single token (its JSON serialization) — in practice named-type arguments are for structured handlers, not command-line interpolation, and authors who need interpolation should keep using scalars.

### Matching

Given a value and a union, the planner tests variants in declaration order and selects the **first** variant where:

1. the value is a JSON object;
2. every required field is present and matches its shape (constants compare equal; enumerations contain the value; references recurse; repeated fields are arrays whose every element matches the inner shape);
3. every *present* optional field matches its shape;
4. no fields outside the variant's declaration are present (closed matching — unknown fields are a mismatch, consistent with the repo's strict-arguments stance).

Because registration rejected ambiguity, at most one variant can match; "first" is a proof obligation discharged at build time, not a semantic.

On success the binding records the variant:

```rust
pub struct BoundArg {
    pub name: String,
    pub value_type: ArgType,
    pub value: Value,
    pub workspace: Option<String>,
    pub variant: Option<String>,   // Some(name) when value_type is Named
}
```

`variant` participates in the invocation fingerprint through `bound_args` exactly as other binding facts do: two calls that match different variants are different invocations even if their raw JSON is coincidentally similar.

On failure the planner produces a mismatch error listing, for each variant in declaration order, the first blocking problem: a missing required field, a field with the wrong shape, a constant that did not match, or an unknown field. Nested reference failures report the path (`fields[2].target`).

### Projection

- **Catalog resource**: `cli://catalog` gains a top-level `types` array (name, summary, variants with fields and shapes). `args` entries with `Named` types carry the type name. The catalog hash covers the `types` section; a type change is a contract change.
- **JSON schema**: generated schemas emit each named type as a `$defs` entry containing a property-level `oneOf` of the variant object schemas (each variant: `type: object`, its properties, its `required` list, `additionalProperties: false`; constants become `const`). Argument sites emit `$ref`. Unions never appear at the top level of a tool's input schema — composition stays at the property level, the shape that survives model-facing schema pipelines.
- **Help text**: arguments with named types render the type summary plus one indented line per variant (variant name, summary, field list with required/optional markers). A type referenced multiple times in one command renders fully once.
- **Preview**: the permission preview includes the matched variant name alongside the argument name.

### Required Invariants

- Every `Named` reference (from `ArgSpec` or `FieldShape::Reference`) resolves to a declared type; the catalog cannot be built otherwise.
- Variant distinguishability is verified at registration; dispatch never depends on declaration order for correctness.
- The matched variant name appears on the bound argument and in the invocation fingerprint.
- Union mismatch diagnostics name every variant and its first blocking problem; nested failures carry a path.
- Generated schemas express named types as `$defs` + property-level `oneOf`; no top-level `oneOf` in any tool input schema.
- The `types` projection in `cli://catalog` round-trips the declarations the planner enforces (one source of truth).

### Implementation Phases

1. Type declaration model (`TypeDecl`, `Variant`, `Field`, `FieldShape`), catalog storage, and registration-time validation (uniqueness, resolution, cycles, distinguishability, dead types).
2. Planner matching: variant selection, `BoundArg::variant`, fingerprint participation, per-variant mismatch diagnostics with paths.
3. Projections: catalog resource `types` section, `$defs` schema generation, help rendering, preview variant display.
4. Builder DSL surface (`declare_type`, `ArgSpec::named`, `Field` constructors) and example coverage in `issues_server` or a dedicated example.
5. Contract coverage: referenced-types-exist and no-dead-types rules; schema projection rule (named types appear in `$defs`, never top-level `oneOf`).

### Acceptance Tests

- A structural union (`element_target` shape) matches both variants, records the right `variant` on the plan, and rejects a value matching neither with a per-variant diagnostic naming both blocking fields.
- A discriminated union (five-variant condition shape) selects by constant field and reports a wrong-constant mismatch naming the expected constants.
- A repeated named-type argument whose variants reference another named type (the form-field shape) matches end to end and reports a nested failure with an indexed path.
- Catalog construction fails for: dangling reference, reference cycle, two indistinguishable variants, dead type.
- The generated schema for a command using a named type contains a `$defs` entry and a `$ref`, and no top-level `oneOf`.
- Generated help for such a command renders every variant with its fields.
- Contract tests fail when a declared type loses its last reference.
- Two calls matching different variants of the same type produce different invocation fingerprints.

## Drawbacks

This is the largest addition to the catalog model since effect lanes. It introduces four new public types and a matching algorithm where previously there was a five-way enum, and every projection surface (catalog JSON, schema generation, help, preview, diagnostics) grows a types-aware branch. Authors face a new question — "scalar, or named type?" — and the answer requires judgment for borderline cases like a string that is secretly an enum.

Closed matching (unknown fields reject) is stricter than typical serde-style tolerant deserialization and may surprise authors porting existing handlers. It is the right default for a framework whose planning layer exists to catch malformed calls, but it is a migration cost for surfaces that previously tolerated extra fields.

The restriction to non-recursive types means genuinely recursive shapes (a tree of nested conditions, say) cannot be declared. Nothing in the motivating surface needs recursion, but a future adopter might, and lifting the restriction later touches the cycle check, the matcher, and schema generation.

## Rationale And Alternatives

**Named declarations versus inline unions.** An inline union attached directly to one `ArgSpec` would be less ceremony for a one-off type. But the motivating shape is the opposite: `element_target` is referenced by roughly fifteen commands. Inline unions would mean fifteen copies in the catalog projection, fifteen expansions in generated schemas with no `$defs` sharing, and no stable name for diagnostics to teach. Naming is what makes every downstream surface — schemas, help, errors, the VS Code extension generation this adoption arc is building toward — coherent. The one-off cost is a single-variant `TypeDecl`, which is cheap.

**One mechanism for structural and discriminated unions.** A design with separate `Tagged` and `Untagged` union kinds (serde's split) would make the discriminated case marginally more declarative. It would also double the vocabulary and leave the framework unable to check what actually matters: that variants are distinguishable. Distinguishability subsumes both styles — a constant field is simply one way to be distinguishable — so the framework checks the property directly and lets authors mix styles.

**Registration-time ambiguity rejection versus first-match-wins.** First-match-wins is simpler to implement and matches serde's untagged behavior. It was rejected because it makes variant *order* semantically load-bearing in a document that agents read as declarative, and it turns an authoring mistake (two overlapping variants) into silent misclassification at dispatch time. The catalog-is-authoritative value demands that the catalog's meaning not depend on invisible ordering rules.

**Raw JSON Schema escape hatch.** `ArgType::Schema(Value)` — letting authors attach arbitrary schema — was rejected outright. It is the `Json` problem with extra steps: the framework can project it but cannot match against it, explain it, or record variants from it. Every capability this RFC exists to provide would silently degrade for schema-typed arguments. Adopters with shapes the vocabulary cannot express should extend the vocabulary (an RFC-sized event, appropriately) rather than route around it.

**Top-level unions.** Expressing a whole command's input as a union of operation shapes was rejected on direct evidence: visible-browser-lab shipped that design and rewrote it after model-facing schema pipelines flattened it. Property-level composition survives those pipelines; the framework should guarantee the surviving shape by construction.

## Prior Art

- **serde's tagged/untagged enums** are the closest Rust precedent for the two matching styles. This design differs by checking distinguishability eagerly instead of trusting variant order, because a catalog is a public contract rather than a private deserialization detail.
- **JSON Schema `oneOf` + `$defs`** is the projection target. The design deliberately generates a disciplined subset (object variants, closed properties, property-level composition) rather than admitting full schema expressiveness, trading generality for the ability to match, diagnose, and teach.
- **visible-browser-lab's agent-surface-contract** is both the motivating consumer and prior art: its hand-built `element_target()` helper, compact-domain flattening comment, and per-operation required-field enforcement are the artifacts this RFC turns into framework features.
- **Ember and Rust RFC practice** shape the document itself: the merged text records the rejected alternatives (top-level unions, escape hatches) with their reasons, so future contributors inherit the constraint knowledge, not just the API.

## Unresolved Questions

- Should contract coverage eventually require every *variant* to be exercised by at least one example (the stronger form of the dead-type rule)? Deferred until the visible-browser-lab port shows whether variant-level examples are natural to write.
- Should `Path`-typed fields inside variants participate in workspace containment the way top-level `Path` arguments do? The motivating surface has no workspace-scoped paths inside unions; the design leaves field shapes workspace-blind until a consumer needs otherwise.
- Whether the preview should render the matched variant's full field values or only its name. Name-only is the conservative default shipped here.

## Future Possibilities

- **Enumerated scalar arguments.** `FieldShape::Enumerated` exists inside variants; promoting it to a top-level `ArgType` would let plain string arguments declare closed value sets and get validation and help for free.
- **Recursive types** behind an explicit opt-in, if an adopter presents a genuinely recursive surface.
- **Generated client bindings.** Named types give the planned VS Code extension generator (and any future typed client) stable identifiers to generate enums and interfaces from — the union declaration becomes a cross-language contract.
- **Workspace-aware fields**, per the unresolved question, if a surface declares path fields inside unions.
