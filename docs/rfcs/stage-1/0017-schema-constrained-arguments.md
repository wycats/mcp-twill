<!-- exo:17 ulid:01kxc025gh2awh74n5bs5a47se -->

# RFC 0017: Schema-Constrained Arguments

- Status: Accepted
- Area: command arguments, JSON Schema, planning, diagnostics, native projection
- Target milestone: v0.4
- Depends on: RFC 0001 (authoritative command catalog), RFC 0002 (diagnostics and response profiles), RFC 0008 (named argument types and unions), RFC 0014 (shared JSON Schema contract machinery)

## Summary

This RFC makes JSON Schema constraints on individual command arguments authoritative. Existing primitive, path, resource, and RFC 0008 named-union arguments remain available. Authors additionally gain integer arguments, enumerated strings, reusable named argument schemas, and self-contained inline schemas for arrays, maps, and nested records that the existing argument vocabulary cannot express.

Every schema attaches to one argument property. Twill compiles and validates it at registration, validates each supplied value during planning, records selected `oneOf` branches at their schema and instance paths without storing a second value, and projects the same canonical schema through the operation catalog, MCP tools, help, diagnostics, fingerprints, and contract tests. A schema never replaces a command's whole input object and never bypasses path, resource, or named-type semantics.

The initial schema dialect is the closed JSON Schema 2020-12 subset needed by VBL's shipped input contract: primitives including integer, enums and constants, bounded and integrally divisible integers, closed or typed-open objects, arrays, local definitions and references, and provably disjoint property-level `oneOf`. A narrow argument-presence relation expresses that supplying one argument requires another without accepting an arbitrary whole-input schema. Unsupported composition and remote references fail server construction.

## Motivation

RFC 0008 solved one important input problem: a named argument may be a structural or discriminated union of record variants, declared once and matched by the planner. It intentionally did not turn `ArgSpec` into a general JSON Schema language. `FieldShape` covers strings, booleans, numbers, integers, constants, enums, references, and repeated values inside those records. The top-level `ArgType` remains coarser: string, path, unconstrained JSON, boolean, number, named type, or resource reference.

That boundary leaves real application contracts inexpressible. VBL's 63-operation v0.4.9 baseline includes top-level integers, non-empty strings, string arrays, arrays with `minItems`, typed string maps, nested records, arrays whose items are discriminated record unions, and one screencast operation with bounded integer inputs. An audit of that baseline finds exactly these input-schema keywords: `type`, `properties`, `required`, `additionalProperties`, `enum`, `const`, `items`, `minItems`, `minLength`, `minimum`, `maximum`, `multipleOf`, `oneOf`, and `description`. The five bounded screencast properties contribute five `minimum` values and five `maximum` values, while its two optional dimensions also contribute `multipleOf: 2`. The baseline contains 21 property-level unions. The 27-tool hybrid surface adds one top-level `dependencies` object requiring `max_width` and `max_height` together; the released broker enforces the same pair. `ArgType::Json` can carry those values but advertises no shape and accepts any JSON. `ArgType::Number` accepts fractional values where VBL requires an integer. Hand-writing a richer MCP schema or relationship in the adapter would make the serving surface stricter than the planner and recreate the duplicate authority RFC 0015 is intended to remove.

The audit also exposes why typed extraction cannot simply equate every Rust primitive with a public schema. VBL publishes unconstrained `{"type":"integer"}` for timeout and count fields while its current broker structs often store them as `u64`. A negative integer is schema-valid but not `u64`-deserializable. Twill must either preserve that compatibility schema through a value type that accepts the complete declared domain, or require a genuinely narrower authored schema; it cannot erase Schemars range/format differences and call a later serde failure impossible. The guide's `JsonInteger` path makes that boundary explicit.

Output schemas in RFC 0014 do not close this gap. An output may be validated after the handler returns, while an input must be rejected before permission checks and dispatch. Native projection can promise exact schemas only when those schemas are catalog facts enforced by the same planner used by every surface.

This RFC supplies that missing layer without reopening RFC 0008. Named types remain the preferred authored model for reusable record unions because they provide variant names, branch-specific help, and build-time ambiguity checks. Schema-constrained arguments cover shapes that are naturally JSON Schema or must preserve an existing public contract.

## Guide-Level Explanation

Common scalar constraints have direct builders:

```rust
command
    .arg(
        arg::integer("limit")
            .summary("Maximum records to return")
            .optional(),
    )
    .arg(
        arg::enumerated("scope", ["owned", "global_readonly"])
            .summary("Which tab inventory to inspect"),
    );
```

An integer projects an integer validation core plus the ordinary summary-derived description and rejects fractional JSON numbers during planning. An enumerated argument projects the exact string enum with the same generated description and rejects values outside it. Optionality still means omission is valid; it does not make explicit `null` valid unless the schema says so.

Reusable complex shapes are declared once:

```rust
server.argument_schema(
    ArgumentSchemaDecl::new(
        "wait-condition",
        "A browser condition to wait for",
        serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "kind": { "const": "delay" },
                        "duration_ms": { "type": "integer" }
                    },
                    "required": ["kind", "duration_ms"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {
                        "kind": { "const": "text" },
                        "text": { "type": "string", "minLength": 1 },
                        "state": { "type": "string", "enum": ["visible", "hidden"] }
                    },
                    "required": ["kind", "text", "state"],
                    "additionalProperties": false
                }
            ]
        }),
    ),
);

server.command("page wait", |command| {
    command
        .arg(
            arg::named_schema("condition", "wait-condition")
                .summary("Condition that completes the wait"),
        )
        .arg(
            arg::integer("timeout_ms")
                .summary("Maximum wait time in milliseconds")
                .optional(),
        )
        .handle_constrained(handle_wait);
});
```

The generated command input remains a closed object with a populated top-level `properties` map. The `oneOf` appears inside `properties.condition`, following RFC 0008's property-level-composition rule. Planning validates the value against exactly one branch and reports each branch's first blocking problem when none match.

A one-off schema can be attached inline:

```rust
command.arg(
    arg::inline_schema(
        "headers",
        serde_json::json!({
            "type": "object",
            "additionalProperties": { "type": "string" }
        }),
    )
    .summary("Additional request headers"),
);
```

Inline and named schemas have identical runtime semantics. Named declarations are preferable when more than one command uses the shape or help should name it. These examples use RFC 0006's `arg::*` helpers because they are the ordinary `CommandBuilder` authoring path. Low-level `CommandSpec` construction has equivalent `ArgSpec::*` constructors. `repeated()` wraps the argument's base schema in an array after validation of the base declaration; authors use an explicit array schema when they need constraints such as `minItems` on the collection itself.

Numeric bounds remain property constraints, while a paired-input rule is an explicit relationship between arguments:

```rust
command
    .arg(
        arg::integer("max_width")
            .summary("Maximum screencast width")
            .with_inline_schema(serde_json::json!({
                "type": "integer",
                "minimum": 16,
                "maximum": 3840,
                "multipleOf": 2
            }))
            .requires_argument("max_height")
            .optional(),
    )
    .arg(
        arg::integer("max_height")
            .summary("Maximum screencast height")
            .with_inline_schema(serde_json::json!({
                "type": "integer",
                "minimum": 16,
                "maximum": 2160,
                "multipleOf": 2
            }))
            .requires_argument("max_width")
            .optional(),
    );
```

Supplying either dimension without the other fails planning before authorization or dispatch. The two directed declarations make the pair symmetric. They do not install a raw command-input schema or move `oneOf` above the property level. Twill compiles the relation into the command's canonical input schema and RFC 0015 later translates the same catalog fact into the pinned grouped-surface spelling.

Typed handlers using constrained arguments opt into `handle_constrained`. Their argument struct derives both `Deserialize` and `JsonSchema`; registration compares the validation-semantic schema of author-declared properties with the assembled command argument schema. Annotation keywords are ignored for this comparison, while every assertion and applicator must agree except for one defined `Option<T>` normalization. Schemars 1.x adds a `null` branch to `Option<T>`, while an optional Twill argument ordinarily means “the property may be absent” rather than “the supplied value may be null.” Twill canonicalizes Schemars' nullable type-array form or converts its two-branch `anyOf` to `oneOf` only when the non-null branch provably excludes null. It may then remove only that derived top-level null branch when the authored property rejects null and the remaining schema agrees exactly. Framework-derived resource carriers are excluded. Map-valued broker adapters keep their existing result dialect: legacy `CommandOutput` handlers use `handle`, while RFC 0014 dynamic application-result handlers use `handle_dynamic`. Both receive planner-validated bound values through their established dynamic argument path; neither gains a constrained-specific handler method merely because a property has a richer schema.

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(inline)]
enum WaitCondition {
    Delay {
        duration_ms: JsonInteger,
    },
    Text {
        #[schemars(length(min = 1))]
        text: String,
        state: WaitState,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
enum WaitState {
    Visible,
    Hidden,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    condition: WaitCondition,
    timeout_ms: Option<JsonInteger>,
}

async fn handle_wait(
    ctx: CommandContext,
    args: WaitArgs,
) -> Result<CommandOutput> {
    // schema validation and typed extraction have already succeeded
    todo!("application wait implementation")
}
```

The two `#[schemars(inline)]` annotations make the guide's derived property schema visibly match the authored inline branches rather than relying on a semantically equivalent `$defs` layout. `JsonInteger` supplies the unconstrained integer schema for both numeric fields, and the field-level length annotation supplies the authored `minLength`. The optional `timeout_ms` property is declared on the command as well as represented by `Option`, so this example passes the same whole-argument agreement check required of applications.

The command declaration installs `handle_wait` through the existing mutable
builder convention. The method uses an inferred sealed dialect marker so
resource parameters can compose without another method name:

```rust
pub trait ConstrainedCommandDialect<M>:
    private::Sealed<M> + Send + Sync + 'static
{
    // Framework-owned schema comparison, extraction, and erasure hooks.
}

impl CommandBuilder {
    pub fn handle_constrained<M, H>(&mut self, handler: H) -> &mut Self
    where
        H: ConstrainedCommandDialect<M>;
}
```

The constrained dialect reuses RFC 0012's public `ContextAndArgs<A, O>` and `WithResourcesAndArgs<P, A, O>` inferred markers. A crate-private replacement is not equivalent: an external caller cannot complete method inference when the selected trait parameter is private. The private supertrait is `private::Sealed<M>`, keyed by that marker, so Twill can implement the supported closure shapes without overlapping blanket implementations while external crates cannot implement the dialect. Authors never write these markers, while their public visibility preserves the same downstream callability as the existing resource dialect and avoids another marker family.

An RFC 0014 result-aware handler keeps `handle_result`; it does not switch to a combinatorial `handle_constrained_result` method. The argument-bearing result dialect activates the same constrained extractor when the command has a schema use, so resource parameters, checked arguments, and application outcomes compose in one signature:

```rust
async fn handle_wait_result(
    session: Res<Session>,
    ctx: CommandContext,
    args: WaitArgs,
) -> ApplicationResult<WaitResult, BrowserFailure, WaitErrors> {
    wait_for_condition(session, ctx, args).await
}

command.handle_result(handle_wait_result);
```

### How Agents Should Learn This

Agents see ordinary MCP input properties. Integer, enum, array, map, and nested-record constraints appear where the argument is supplied. Help summarizes important constraints and expands named schemas once, then references them by name from commands.

When a value fails, diagnostics point to the argument and nested JSON path. A failed property-level union explains why each branch did not match. Diagnostics may quote schema-owned names, enum members, bounds, and property names, but never echo an adversarial input value. The agent repairs the same call; it is not asked to serialize JSON into a string or switch to a less precise tool.

## Reference-Level Explanation

### Model

`ArgType` gains an integer primitive. `ArgSpec` gains an optional authoritative schema use:

```rust
pub enum ArgType {
    String,
    Path,
    Json,
    Bool,
    Number,
    Integer,
    Named(String),
    ResourceRef(String),
}

pub struct ArgSpec {
    pub name: String,
    pub value_type: ArgType,
    pub required: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub repeated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<ArgumentSchemaUse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_arguments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ArgumentSchemaUse {
    Named { name: String },
    Inline { schema: serde_json::Value },
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ArgumentSchemaDecl {
    pub name: String,
    pub summary: String,
    pub schema: serde_json::Value,
}

/// A JSON number satisfying JSON Schema `type: integer`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct JsonInteger(serde_json::Number);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonIntegerError {
    NotInteger,
}

impl JsonInteger {
    pub fn try_from_number(
        number: serde_json::Number,
    ) -> std::result::Result<Self, JsonIntegerError>;
    pub fn as_i64(&self) -> Option<i64>;
    pub fn as_u64(&self) -> Option<u64>;
    pub fn into_number(self) -> serde_json::Number;
}

impl TryFrom<serde_json::Number> for JsonInteger {
    type Error = JsonIntegerError;

    fn try_from(
        number: serde_json::Number,
    ) -> std::result::Result<Self, Self::Error>;
}

impl<'de> Deserialize<'de> for JsonInteger {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>;
}

impl JsonSchema for JsonInteger {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "JsonInteger".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "integer" })
    }
}

pub struct ResourceDecl {
    // ...RFC 0012 fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_schema: Option<ArgumentSchemaUse>,
}

impl ArgumentSchemaUse {
    pub fn named(name: impl Into<String>) -> Self;
    pub fn inline(schema: impl Into<serde_json::Value>) -> Self;
}

impl From<serde_json::Value> for ArgumentSchemaUse {
    fn from(schema: serde_json::Value) -> Self {
        Self::Inline { schema }
    }
}

impl From<schemars::Schema> for ArgumentSchemaUse {
    fn from(schema: schemars::Schema) -> Self {
        Self::Inline {
            schema: schema.into(),
        }
    }
}

impl ArgumentSchemaDecl {
    pub fn new(
        name: impl Into<String>,
        summary: impl Into<String>,
        schema: impl Into<serde_json::Value>,
    ) -> Self;
}

impl ArgSpec {
    pub fn integer(
        name: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self;

    pub fn enumerated(
        name: impl Into<String>,
        values: impl IntoIterator<Item = impl AsRef<str>>,
        summary: impl Into<String>,
    ) -> Self;

    pub fn named_schema(
        name: impl Into<String>,
        schema: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self;

    pub fn inline_schema(
        name: impl Into<String>,
        schema: impl Into<serde_json::Value>,
        summary: impl Into<String>,
    ) -> Self;

    pub fn with_named_schema(self, name: impl Into<String>) -> Self;
    pub fn with_inline_schema(self, schema: impl Into<serde_json::Value>) -> Self;
    pub fn requires_argument(self, name: impl Into<String>) -> Self;
    pub fn optional(self) -> Self;
}

impl ResourceDecl {
    pub fn reference_schema(
        self,
        schema: impl Into<ArgumentSchemaUse>,
    ) -> Self;
}

impl CommandRegistry {
    pub fn declare_argument_schema(self, decl: ArgumentSchemaDecl) -> Self;
}

impl ServerBuilder {
    pub fn argument_schema(&mut self, decl: ArgumentSchemaDecl) -> &mut Self;
}

pub mod arg {
    pub fn integer(name: impl Into<String>) -> ArgBuilder;
    pub fn enumerated(
        name: impl Into<String>,
        values: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> ArgBuilder;
    pub fn named_schema(
        name: impl Into<String>,
        schema: impl Into<String>,
    ) -> ArgBuilder;
    pub fn inline_schema(
        name: impl Into<String>,
        schema: impl Into<serde_json::Value>,
    ) -> ArgBuilder;
}

impl ArgBuilder {
    pub fn with_named_schema(self, name: impl Into<String>) -> Self;
    pub fn with_inline_schema(self, schema: impl Into<serde_json::Value>) -> Self;
    pub fn requires_argument(self, name: impl Into<String>) -> Self;
}
```

`ArgumentSchemaDecl` follows the additive declaration policy and emits only the exact camel-case object `{ "name": string, "summary": string, "schema": object }`. `ArgumentSchemaUse` retains the internally tagged forms `{ "kind": "named", "name": string }` and `{ "kind": "inline", "schema": object }`; neither a bare schema name nor a Rust externally tagged spelling is accepted as an alternate declaration dialect. `ArgSpec::requires_arguments` is the exact camel-case `requiresArguments` array. Missing and empty arrays normalize to omission, preserving existing declaration and catalog bytes.

RFC 0012's `ResourceDecl` gains the optional `reference_schema` shown above. Missing data defaults to `None` and is omitted when serialized, so existing resource declarations and catalog bytes remain unchanged. Registration accepts only schemas that imply a non-empty string domain. It intersects that schema with RFC 0012's resource-reference grammar only where the supported subset makes the result decidable: a `const` or finite `enum` with no syntactically valid resource id or URI is statically empty and fails registration. General string schemas need not restate the complete resource grammar; planning validates the schema first and then applies RFC 0012 parsing, liveness, and ownership as the stricter semantic authority. Every derived carrier uses that canonical schema before resource parsing and resolution.

`ArgSpec::integer` uses `ArgType::Integer` with the derived integer schema. `ArgSpec::enumerated` uses `ArgType::String` plus an inline string enum and copies each `AsRef<str>` item, so arrays, borrowed slices, and owned string collections follow one API. `ArgSpec::named_schema` and `inline_schema` use `ArgType::Json` plus their schema use. The explicit `named_schema` spelling distinguishes a schema reference from RFC 0008's named argument type and from an inline schema value. Low-level authors may refine `String`, `Number`, `Integer`, `Bool`, `Json`, or `Path` with a compatible schema. Path refinements must imply string and run before workspace containment; they cannot replace path normalization or authority checks. RFC 0008 `Named` arguments reject overrides because their declaration remains the sole union authority.

`JsonInteger` is the exact typed-extraction counterpart of an unconstrained JSON Schema integer *within Twill's `serde_json::Value` number domain*. The MCP/JSON decoder determines whether a number can enter that domain before Twill planning; this type adds no narrower signedness or machine-width contract and does not claim to preserve source lexeme bytes the decoder has already normalized. `try_from_number`, its `TryFrom<serde_json::Number>` delegation, and `Deserialize` apply the same mathematical-integer predicate as the planner, including representable decimal or exponent forms such as `1.0`; deserialization checks independently and never trusts a prior validation marker. They retain the accepted `serde_json::Number` and reject every fractional value with static `JsonIntegerError::NotInteger`. That error implements `Display` and `std::error::Error`, its `Display`/`Debug` contains no rejected number, and the custom serde visitor uses the same static text. The field is private, so no unchecked value can be constructed. Its `JsonSchema::json_schema` implementation and `SchemaGenerator::subschema_for::<JsonInteger>()` result are exactly `{ "type": "integer" }`, with no `$ref`, `$defs`, title, format, or range. A standalone `schema_for!(JsonInteger)` may add Schemars' ordinary root `$schema` and `title` annotations; that root wrapper is not the property subschema compared with a command declaration and does not create another input contract. The checked accessors accept integral floating representations when their parsed numeric value fits exactly in the requested machine type; otherwise they return `None`. Applications return a declared application error for an out-of-domain but schema-valid value. Rust primitives such as `u64` and `i32` carry Schemars storage formats and storage-wide ranges that do not automatically equal an application's authored bounds. Registration neither erases those facts nor substitutes a narrower range to make a handler appear compatible. Authors use `JsonInteger` when they need the complete declared integer domain, derive an exactly matching constrained type when its schema fits the supported dialect, or keep a dynamic validated-value adapter.

Resource carriers receive compatible string refinements from the resource declaration rather than a hand-authored carrier `ArgSpec`:

```rust
ResourceDecl::new("tab", "A session-owned browser tab")
    .carrier("tab_id")
    .reference_schema(serde_json::json!({
        "type": "string",
        "minLength": 1
    }))
```

The framework injects that schema with every `ArgType::ResourceRef("tab")` carrier and still parses the URI, resolves liveness, and enforces ownership afterward. A reference schema may strengthen the string representation but never accept a non-string or bypass the resolver.

The low-level refinement methods are `with_named_schema(name)` and `with_inline_schema(schema)`. The `named_schema` and `inline_schema` constructors are conveniences for the common `ArgType::Json` case; all four routes populate the same `ArgumentSchemaUse` field. `ArgumentSchemaUse::named` is the explicit reusable form. `ArgumentSchemaUse::inline` and `From<serde_json::Value>` are equivalent inline forms, so the resource builder accepts the guide's raw JSON value without giving strings an ambiguous implicit named-schema conversion. The additive consuming `ArgSpec::optional()` sets `required` to false for every argument kind, matching `ArgBuilder::optional()` and making the guide's low-level integer example compilable without a public struct literal.

The schema-use declaration has one exact tagged wire shape. A named use serializes as `{ "kind": "named", "name": "wait-condition" }`; an inline use serializes as `{ "kind": "inline", "schema": { ... } }`. It never uses Rust's externally tagged enum spelling and never treats an arbitrary JSON string as an implicit declaration name. Public declaration deserialization follows the corpus additive unknown-field policy, while normalized catalog emission contains only the known tagged fields. The compiler resolves either form to one canonical effective property schema, but preserves named-versus-inline authorship in the catalog because named declarations have reusable identity, help, and dead-reference rules.

These refinement methods are ordinary value transformations rather than finalizing-builder assignments: each `with_named_schema` or `with_inline_schema` call replaces the `schema` field on the returned `ArgSpec`/`ArgBuilder`, including the generated schema seeded by `integer` or `enumerated`. The final visible value therefore contains exactly one schema authority, and chaining refinements has the explicit same semantics as assigning that public field again. `optional()` is idempotent. This is the same visible-value model RFC 0014 uses for its standalone error and result declarations. It is intentionally different from competing assignments to finalization slots such as RFC 0014's command handler/result/output slots, RFC 0015's completed native-surface configuration, and RFC 0016's ambient-binding source/carrier choices; those builders retain multiple construction inputs long enough to diagnose duplicate semantic authority.

`requires_argument` appends one directed presence edge to that visible value. Registration sorts and deduplicates each argument's target set, rejects an empty, unknown, or self target, and accepts an edge only when both arguments are optional; an unconditionally required endpoint makes the edge redundant or hides ordinary requiredness and must instead be expressed through `required`. Cycles remain valid because two opposite edges are the canonical spelling of a required pair and a longer cycle expresses an optional all-or-none group. If a present argument names several targets, every target must also be present. The command-schema compiler emits those edges as draft-2020-12 `dependentRequired`; an author-supplied property schema cannot use `dependentRequired` or legacy `dependencies` as a second relationship authority. The planner enforces the same catalog edges directly, so a serving surface can translate their schema spelling but cannot add, remove, or infer an edge.

RFC 0006's builder DSL mirrors the low-level surface through `arg::integer`, `arg::enumerated`, `arg::named_schema`, `arg::inline_schema`, and refinement methods on `ArgBuilder`. Catalog and MCP projection use the same canonical property schema that registration compiled; the builder never maintains a second constraint model.

Schema names use the same lower-kebab grammar and collision rules as other catalog declarations but occupy a distinct `argumentSchemas` namespace. Unreferenced declarations fail registration. Repeated names, dangling references, and empty summaries fail registration. Inline reference schemas live on their `ResourceDecl`, participate in catalog identity, and are not counted as dead declarations.

### Supported Schema Dialect

Twill fixes `schemars::generate::SchemaSettings::draft2020_12()`. Inline and named constructors accept `serde_json::Value` or Schemars 1.x `Schema` through the conversion APIs above. An optional top-level `$schema` is accepted only when it is exactly `https://json-schema.org/draft/2020-12/schema`, then removed before canonical projection, semantic comparison, and hashing because the dialect is already fixed. Nested, alternate, or non-string `$schema` values fail registration.

The initial self-contained vocabulary proven by VBL's complete 63-operation input corpus is:

- annotations: `title` and `description`;
- primitives: `type` with one of `null`, `boolean`, `object`, `array`, `number`, `integer`, or `string`, or a nullable two-member array containing exactly one non-null primitive and `null`;
- equality: `const` and homogeneous `enum`;
- integers: inclusive `minimum` and `maximum`, plus positive integral `multipleOf`;
- strings: `minLength`;
- arrays: `items` and `minItems`;
- objects: `properties`, `required`, and `additionalProperties` as a boolean or schema;
- composition: `oneOf` anywhere inside an argument property's schema, with branches pairwise provably disjoint at every occurrence;
- reuse: `$defs` and local `#/$defs/...` references.

Root and branch boolean schemas, remote or recursive references, `$dynamicRef`, `anyOf`, `allOf`, `not`, conditional schemas, exclusive or non-integral numeric bounds, non-integral divisibility, regular-expression patterns, string/array maximums and uniqueness assertions, property-count assertions, raw `dependentRequired` or legacy `dependencies`, `unevaluatedProperties`, tuple-array keywords, content keywords, custom vocabularies, and unknown assertion keywords fail registration. Annotation keywords unknown to the validator may be preserved only when explicitly classified as annotations and excluded from validation semantics.

This boundary is evidence-driven rather than a claim that the omitted JSON Schema vocabulary is unimportant. The released VBL v0.4.9 input corpus uses no omitted assertion: its only numeric assertions are inclusive integral bounds and `multipleOf: 2`, and its only cross-property rule is the paired screencast relation represented through `requires_arguments`. A later adopter can add one keyword with its planner, stable redacted diagnostic, canonicalization, client-projection, and acceptance evidence instead of requiring the first implementation to standardize semantics it cannot yet test against a real public surface.

Every object schema must state `additionalProperties`. This prevents an omitted keyword from accidentally making a supposedly exact record open. Object variants inside `oneOf` normally use `false`; typed maps use a schema value or `true` deliberately.

Numeric literals in `const`, `enum`, `minimum`, `maximum`, or `multipleOf` must be represented exactly by RFC 8785's I-JSON number domain. Registration rejects an out-of-domain integer or decimal rather than allowing a later native/effect-lane snapshot compiler to round it while establishing cross-language identity. The initial numeric-assertion subset accepts these three assertion keywords only on an integer schema and requires each assertion value to be a mathematical integer; `multipleOf` must also be positive, and a present minimum cannot exceed a present maximum. Planning applies the same mathematical-integer predicate as `JsonInteger` over the retained `serde_json::Number` representation. Values retained as `i64` or `u64` compare and divide after exact widening to `i128`. A value retained as `f64` must be finite with zero fractional part; the exact-I-JSON assertion integer converts exactly to `f64`, inclusive bounds use ordinary IEEE comparisons, and divisibility requires an IEEE remainder equal to positive or negative zero. These rules make accepted zero-fraction or exponent values obey the assertions over their decoded numeric value. Twill neither reparses source text nor claims access to lexeme precision the JSON decoder discarded. It evaluates the decoded numeric value exactly as retained and applies no epsilon tolerance: any nonzero IEEE remainder fails divisibility, including one caused by precision already lost during JSON decoding. Runtime number values remain governed by the declared type and assertions; this restriction applies to schema literals that become contract bytes.

### Registration And Ambiguity

Registration compiles every schema once and canonicalizes it for hashing and projection. A schema attached to a coarse primitive must imply that primitive's type; it may refine but not contradict it. `repeated()` wraps the canonical base schema and cannot be combined with a base schema whose root is already an array.

After every command argument is known, registration resolves each `requires_arguments` target by canonical argument name. Empty names, self edges, dangling targets, and edges with a required trigger or target fail construction. Repeated targets collapse, target order becomes lexical, and opposite or longer cycles remain valid because they express presence equivalence or a deliberate optional dependency group rather than reference recursion. The compiler rejects no call value while establishing this graph and invokes no handler.

The compiler rejects statically empty argument domains it can express in the supported subset: an empty enum, an impossible `required`/closed-property combination, contradictory `const`, enum, primitive-type, or numeric-bound facts, and an integer domain whose positive `multipleOf` has no value inside its inclusive bounds. Boolean `false` remains valid only as `additionalProperties: false`. A command cannot publish a required argument or union branch that no caller can ever satisfy.

Every `oneOf` is checked pairwise, including unions reached through `items` or a local reference. Twill accepts branches it can prove disjoint through incompatible primitive types, non-overlapping `const` or enum values, or RFC 0008-style closed-object required-property contradictions. If it cannot prove two branches disjoint, registration fails and names the union and branch schema pointers. Runtime `oneOf` validation still requires exactly one match at every instance location reached during validation.

Local references must resolve and form an acyclic graph, and every `$defs` entry must be reachable from the schema root. Dead definitions fail registration rather than disappearing during projection. When command-schema assembly copies definitions from several argument properties, same-name definitions deduplicate only when their complete canonical schemas are identical; a semantic disagreement fails registration rather than renaming either declaration's public vocabulary. Canonicalization sorts object keys, the set-like `required` inventory, and a nullable `type` array into `[non-null-type, "null"]`. It rejects duplicate `enum` members but preserves authored `enum` and `oneOf` order because those orders are caller-visible in schemas, help, and diagnostics. It preserves every other array whose order affects validation and strips no validation keyword. Different projected schemas produce different catalog hashes, including deliberate reordering of caller-visible alternatives.

### Planning And Diagnostics

The planner's value domain is `serde_json::Value`, so every string and object key already consists of Unicode scalar values. A well-formed escaped UTF-16 surrogate pair in raw JSON decodes to its one non-BMP scalar before schema validation. An unpaired surrogate cannot become a Rust string and fails at the protocol JSON decoder before Twill planning; it never becomes `ArgumentSchemaMismatch`, a bound argument, a fingerprint fact, or a presentation value. RFC 0019's generated JavaScript input boundary applies the corresponding validation before invoking either host transport, so native JSON and generated-host calls admit the same string domain.

Argument binding proceeds in this order:

1. reject unknown and missing arguments through the existing planner;
2. evaluate every supplied value against its canonical argument schema and every applicable presence edge, collecting redacted mismatch candidates without selecting one early;
3. select one candidate through the global instance-path, keyword, and canonical-property ordering below;
4. apply specialized named-type, path, workspace, or resource-reference validation when that argument kind owns it;
5. record the bound value and any named variant or schema branch identity;
6. derive the invocation fingerprint from the completed plan and the selected command's canonical presence-relation inventory.

A candidate selected at step 3 terminates binding with that mismatch; step 4 is reached only when step 2 collected no mismatch candidates. Path-containment, resource-liveness, and named-type failures from step 4 therefore never compete with or override value-schema or presence failures from step 2.

Presence checking reads only argument names. A failing edge contributes one candidate with the trigger as `ArgumentSchemaMismatch.argument`, the empty root instance path, `DependentRequired`, and the missing target name as `expected`. It does not preempt a value-schema candidate: both enter the same ordering, so a root `Type` failure beats a root `DependentRequired` failure while a shallower dependency failure beats a nested value failure. When several dependency candidates remain tied, canonical trigger order and then canonical target order select the result. The relation adds no `BoundArg` or `schema_match` field, but its canonical sorted trigger/target inventory enters the invocation-fingerprint preimage directly for every origin, including `bareRegistry`, as the optional top-level member `"argumentRequirements": { <trigger>: [<sorted targets>] }` with trigger keys sorted by canonical JSON. Missing and explicit-empty inventories normalize to omission. A changed relation therefore invalidates replay even when the same call supplies both arguments and the serialized plan is otherwise unchanged.

`BoundArg` gains an additive defaulted `schema_match` containing every constrained-union selection. Missing data and an explicit empty selection list normalize to the same empty value, and `is_empty` omits the field so unconstrained arguments retain their serialized plan shape. A flat argument union has one selection at the empty instance pointer; an array of union items has one selection for each matching item; nested schemas may contribute more than one selection:

```rust
pub struct BoundArg {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "ArgSchemaMatch::is_empty")]
    pub schema_match: ArgSchemaMatch,
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ArgSchemaMatch {
    pub selections: Vec<SchemaBranchSelection>,
}

impl ArgSchemaMatch {
    pub(crate) fn is_empty(&self) -> bool {
        self.selections.is_empty()
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct SchemaBranchSelection {
    pub schema: String,
    pub instance_pointer: String,
    pub one_of_pointer: String,
    pub branch_pointer: String,
}
```

`schema` is the declaration name or the stable synthetic identity `@inline:<argument>`. `instance_pointer` identifies where the union was evaluated inside the supplied argument, while the other pointers identify the canonical schema occurrence and selected branch. Every pointer uses RFC 6901 JSON Pointer: root is the empty string, `~` escapes as `~0`, `/` as `~1`, and array/branch indices use canonical unsigned decimal with no leading zero. A branch pointer therefore ends in `/oneOf/<index>` under the canonical schema path. Selections are sorted lexicographically by instance pointer, then `one_of_pointer`, then branch pointer, and participate in the invocation fingerprint. Pointers carry structure but no input data. Help may supplement a branch pointer with its schema-owned `title` or discriminator label.

Local-reference traversal does not create a second address space. When validation reaches a `oneOf` through `#/$defs/...`, `one_of_pointer` and `branch_pointer` name the physical occurrence in the canonical declaration or inline schema document, after following the reference, rather than the `$ref` use site or an implementation-generated dereferenced copy. Evaluating that definition at several instance locations therefore produces several records with the same schema pointers and distinct `instance_pointer` values. A chain of local references resolves to the final physical `oneOf` occurrence. Because the graph is acyclic and every definition is reachable, this spelling is total, stable across validator implementations, and never needs a synthetic expansion index.

These selections remain serialized structural `BoundArg` facts in the initial contract, paralleling RFC 0008's matched variants. A repeated or nested union contributes at most one record per evaluated `oneOf` occurrence, so the record count is linear in the number of union evaluations over an argument value the plan already retains. Serialized pointers add depth and index-text overhead; the RFC makes no stronger byte-for-byte linear-size claim. The records make previews and audit/debug plans explain the branch identity bound into the fingerprint without copying any additional caller value.

A caller mismatch has a dedicated framework variant that continues to map to the public `InvalidArgumentType` family:

```rust
pub enum FrameworkError {
    // ...existing variants...
    ArgumentSchemaMismatch {
        argument: String,
        path: String,
        keyword: ArgumentSchemaKeyword,
        expected: String,
        branches: Vec<SchemaBranchProblem>,
    },
    ArgumentContractViolation {
        operation_id: String,
        argument: Option<String>,
        reason: ArgumentContractReason,
    },
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct SchemaBranchProblem {
    pub pointer: String,
    pub path: String,
    pub keyword: ArgumentSchemaKeyword,
    pub expected: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentSchemaKeyword {
    Type,
    Const,
    Enum,
    Minimum,
    Maximum,
    MultipleOf,
    MinLength,
    Items,
    MinItems,
    Required,
    DependentRequired,
    AdditionalProperties,
    OneOf,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentContractReason {
    DerivedSchemaDrift,
    TypedDeserializationFailed,
}

pub enum ErrorCode {
    // ...existing codes...
    ArgumentContractViolation,
}
```

`ArgumentSchemaMismatch` maps to `ResponseStatus::InvalidInput` and the existing `ErrorCode::InvalidArgumentType`. It contains the argument, nested path, stable keyword, expected schema-owned constraint, and branch problems. It never includes the rejected value or a validator library's unredacted error string.

The enum order above is also the fixed tie-break order after instance-path depth across both value-schema and presence-edge candidates. `Minimum`, `Maximum`, and `MultipleOf` place the assertion's canonical RFC 8785 integer spelling in `expected`. `DependentRequired` places only the lexically selected missing argument name there. Diagnostics never include the supplied numeric value, the set of present arguments, or a synthesized whole-input object.

`ArgumentContractViolation` represents an author-side Rust/schema disagreement at two distinct phases. `DerivedSchemaDrift` is construction-only: registration returns it before publishing a registry, and it can never become a tool response, task result, framework event, or generated-host outcome. The embedding's ordinary build channel may describe the author-side type while handling that construction error. After successful registration, the compiled agreement removes that possibility; `TypedDeserializationFailed` is the only runtime reason and receives `ResponseStatus::Failed` plus `ErrorCode::ArgumentContractViolation` because caller repair cannot fix the defect. Its static message identifies an argument contract defect, while public details contain only the catalog operation id, an argument name when the extractor can attribute the boundary to one property, and that stable reason. They never contain `std::any::type_name`, a crate/module path, serde text, or the materialized argument object. Task delivery, generated-host projection, events, and logs preserve the runtime framework code; events and logs retain only operation, optional argument, and reason.

`ArgumentSchemaMismatch.path` and every `SchemaBranchProblem.path` use the same RFC 6901 instance-pointer grammar as bound branch selections. `SchemaBranchProblem.pointer` is exactly the canonical physical branch pointer for the branch whose blocking problem it reports: it has the same spelling as that branch's `SchemaBranchSelection.branch_pointer`, including after local-reference traversal. It never names the containing `oneOf`, a `$ref` use site, or an implementation-generated expansion. The outer non-union mismatch needs no parallel schema-pointer field because its argument plus instance path, keyword, and expected constraint already identify the failed property contract. When several assertions fail, Twill selects the shallowest instance path, then the fixed `ArgumentSchemaKeyword` order above, then canonical property/schema-pointer order. A union reports one selected blocking problem per branch in authored branch order. The framework translates validator-library output into these enums and ordering rules, so dependency upgrades cannot silently change the public diagnostic contract.

Registration compares the canonical validation-semantic schema produced by the Rust argument type with the assembled command input schema after removing annotations, framework-derived carrier properties, and the catalog-owned root `dependentRequired` map. The relationship map is compiled and validated independently before that subtraction; the derived Rust schema may not supply `dependentRequired` or legacy `dependencies`, because either spelling would create a second relationship authority. This exclusion is safe because planning enforces the catalog relation before extraction and an `Option<T>` field accepts the planner-validated absent-or-present domain. Twill first normalizes the *derived Rust schema only*: a two-member nullable `type` array receives canonical non-null/null order, and `anyOf: [T, {type: "null"}]` becomes `oneOf` only when `T` provably rejects null. If the authored property rejects null, Twill may then remove exactly that derived top-level null alternative. The resulting property schemas, object required set, and every other remaining assertion/applicator must be canonically equal. This admits a Rust deserializer that accepts a harmless null superset of planner-validated values without making `null` part of the public contract; it also lets an explicitly nullable authored `oneOf` match the equivalent Schemars `Option<T>` representation. It does not add authored `anyOf` to Twill's dialect or attempt general schema subsumption. An ambiguous nullable `anyOf`, an independently derived relationship keyword, or any residual difference fails registration. If comparison passed but typed deserialization later fails, Twill returns a redacted `ArgumentContractViolation`. That is an author/framework defect, not caller repair. A plain typed handler cannot be registered for a command with constrained arguments; it must opt into checked constrained extraction or dynamic validated values.

`handle_constrained` uses its own private checked extractor over `DeserializeOwned + JsonSchema`; it does not specialize or change the existing blanket `FromCommandArgs` implementation. Existing typed handlers for coarse arguments remain source-compatible. RFC 0014's `handle_result` reuses this extractor automatically for its argument-bearing signature variants whenever the command declares a constrained schema, so resources plus results do not require a combinatorial method name. The checked extractor consumes the already validated bound-argument object once and maps any impossible serde disagreement to `ArgumentContractViolation` without retaining serde's raw error text.

The explicit `handle_constrained` entry point remains the migration boundary for legacy typed handlers. Automatically changing every existing `handle` registration to derive and compare JSON Schema would add new trait bounds and registration failures to commands that use only the shipped coarse argument model. New result-aware handlers already opt into a richer contract, so `handle_result` can select checked extraction automatically without creating another method family.

### Projection

- **Operation catalog.** Referenced declarations appear once under `argumentSchemas`; each argument records its schema use and canonical `requiresArguments` targets. Inline schemas remain on their argument. All participate in catalog identity.
- **MCP schema.** Named declarations are inlined at the argument property site. Reachable same-name local `$defs` are deduplicated only when canonically identical; conflicts fail before projection, and names are never silently rewritten. Canonical presence edges compile into a sorted top-level `dependentRequired` object. A command's top-level input remains `{type: object, properties, required, additionalProperties: false}` with that optional relationship assertion and never gains a whole-input union. RFC 0015 derives its version-1 grouped compatibility spelling from the same edges.
- **Descriptions.** Explicit inline and named schemas preserve their authored annotation set exactly. If a root schema declares `description`, it must equal `ArgSpec::summary`; omission remains omission so compatibility schemas do not gain bytes the established surface never served. Help always uses the required argument summary. Generated primitive schemas retain the existing summary-derived description behavior.
- **Help.** Scalar constraints render inline. Named schemas render once with their nested properties and branches, then commands reference the name. A presence edge renders with its triggering argument as “when supplied, also requires `<target>`,” using canonical target order.
- **Contracts.** `check_argument_schema_projection` compares declaration, planner, catalog, help, MCP schema, typed-handler schema when applicable, and canonical hashes.

### Required Invariants

- Every projected validation keyword is enforced by planning before authorization and dispatch.
- Every enforced argument constraint projects through catalog, schema, help, diagnostics, and contract tests from one declaration.
- Schema composition remains inside an argument property's schema, including nested arrays and records, and never becomes composition of a command's whole input object.
- Argument-presence relationships are explicit catalog edges, enforced before dispatch and projected without becoming an author-supplied whole-input schema.
- General schemas do not weaken path containment, resource resolution, workspace authority, or RFC 0008 named-union matching. Path and resource refinements are validated before their specialized checks and may only strengthen their string representation.
- Every accepted `oneOf` is pairwise provably disjoint at registration and matches exactly one branch at runtime.
- Diagnostics expose schema-owned constraints and paths but never rejected values or raw validator messages.
- Typed constrained extraction is schema-checked at registration; dynamic extraction receives only planner-validated values.
- Catalog identity changes when any validation-semantic schema or argument-presence fact changes.
- Every invocation fingerprint binds the selected command's canonical argument-presence inventory even though that inventory adds no serialized plan field.

### Implementation Phases

1. Add integer arguments, schema declarations and uses, argument-presence edges, JSON Schema 2020-12 subset compilation, canonicalization, and registration validation.
2. Integrate numeric, presence, and schema validation plus redacted diagnostics into planning, including branch identity and ambiguity rejection.
3. Add catalog, MCP schema, help, fingerprint, and contract projections.
4. Add `handle_constrained`, typed-schema comparison, and redacted extraction-contract failures.
5. Import all 63 VBL per-operation input schemas as the authoritative source fixture. Reproduce the released baseline projection exactly, then record the screencast command's catalog-owned `dependentRequired` map as the one explicit Twill adoption delta from the ungrouped release schema. RFC 0015 later owns direct/grouped native projection, consuming these canonical property schemas without reimplementing validation.

### Acceptance Tests

Acceptance lives in `crates/mcp-twill/tests/argument_schemas.rs`; shared JSON Schema compiler cases are table-driven with RFC 0014's result fixtures. The RFC 0017 source fixture consumes `baseline-tools.json` from RFC 0015's already-landed evidence-only fixture bootstrap; the new Twill argument authoring lives in `crates/mcp-twill/tests/support/vbl.rs`. Comparison is exact for every released property schema and for every released root fact after subtracting only the catalog-owned presence map. The final direct schemas are then compared separately: 62 operations remain byte-identical, while `screencast_start` differs only by the sorted symmetric `dependentRequired` map for `max_width` and `max_height`. That one documented adoption addition captures the grouped release schema and broker behavior that the ungrouped baseline omitted. The bootstrap supplies only provenance-checked observations, while RFC 0015's later public implementation separately owns native projection comparison with `surface-catalog.json`.

- Integer arguments accept integral JSON numbers and reject fractional values before authorization and dispatch.
- Inclusive integer `minimum`/`maximum` and positive integral `multipleOf` accept every boundary value and reject the adjacent mathematical integer. Zero-fraction and exponent spellings agree with the numeric value retained by the configured JSON decoder, and raw lexemes outside that domain never reach planning. Registration rejects reversed bounds, zero/negative/non-integral divisors, non-integral bounds, unsupported numeric keywords, and bounded divisible domains with no satisfying integer.
- Two optional dimensions with opposite `requires_arguments` edges accept neither or both and reject either singleton before authorization and dispatch. A request with an invalid trigger value and an absent partner contributes both candidates and selects the same failure through the documented global keyword/path ordering. Missing/empty relation fields preserve legacy bytes; repeated targets normalize, while empty, self, dangling, or required-endpoint edges fail registration. Low-level `ArgSpec` and `ArgBuilder` construction produce identical catalog, help, schema, diagnostics, and hash results.
- Raw JSON fixtures accept well-formed surrogate pairs as one Unicode scalar and reject isolated high/low surrogate escapes at protocol decoding before schema planning; no rejected code unit or decoder text enters Twill diagnostics. RFC 0019 generated-host fixtures enforce the same accepted string/key domain before transport invocation.
- `ServerBuilder::argument_schema` and `CommandRegistry::declare_argument_schema` produce the same declaration graph, catalog bytes, and validation failures for equivalent input.
- JSON-value and Schemars-`Schema` constructors compile equivalently; the exact top-level draft-2020-12 `$schema` marker normalizes to omission, while alternate, nested, or malformed markers fail before projection.
- Named and inline `ArgumentSchemaUse` declarations round-trip through their exact camel-case tagged forms and normalize additive unknown fields out of canonical catalog output; externally tagged enum spellings, a bare string pretending to be a name, and a tagged inline use whose `schema` is not supported all fail before publication.
- Low-level and ergonomic schema refinements replace the one visible schema slot deterministically, including constructor-derived primitive schemas; `optional()` is idempotent, and the final value compiles identically to an equivalent direct `ArgSpec`.
- Compile-fail coverage proves there is no ambiguous `ArgSpec::schema` or `arg::schema` constructor: named references use `named_schema`, inline values use `inline_schema`, and RFC 0008 named unions retain their existing `named` constructor.
- Legacy `ArgSpec` and `ResourceDecl` JSON without `schema`/`referenceSchema`/`requiresArguments`, explicit `None` schema values, and explicit empty relationship arrays normalize to byte-identical catalog data and hash input; coarse argument and resource-carrier schemas remain unchanged until adopted.
- Enumerated strings reject undeclared values with a redacted diagnostic listing the allowed schema-owned members.
- A non-empty string array and an array of discriminated record variants enforce `minLength`, `minItems`, item schemas, and branch selection.
- Nested and repeated unions record every selected branch with stable schema and instance pointers; changing any selection changes the fingerprint without storing or echoing the value. RFC 6901 fixtures with property names containing `~` and `/`, nested arrays, and branch indices prove exact escaping, empty-root spelling, no-leading-zero indices, canonical selection sorting, and cross-platform fingerprint equality. Direct, single-`$ref`, and chained-`$ref` access to the same physical `oneOf` use the definition's canonical pointer rather than a use-site or validator-generated expansion pointer; repeated evaluations differ only by their instance pointers. Every mismatch-side `SchemaBranchProblem.pointer` equals that same physical branch-pointer spelling for its branch.
- Adversarial nesting and large arrays produce no more than one `SchemaBranchSelection` per evaluated `oneOf`. Size accounting tests distinguish that linear record-count guarantee from the additional serialized pointer depth/index overhead and prove no selection copies a caller value.
- Non-empty `schemaMatch` plans serialize `instancePointer`, `oneOfPointer`, and `branchPointer` in camel case, `requiresArguments` is the sole relationship-field spelling, and diagnostic keywords and contract reasons use their exact lower-snake-case spellings, including `minimum`, `maximum`, `multiple_of`, and `dependent_required`. JSON Schema fixtures and round trips reject accidental Rust field or variant spellings.
- Legacy omission, an explicitly empty `schemaMatch`, and the default Rust value normalize to one omitted plan spelling and identical fingerprint input; non-empty selections serialize exactly once in canonical order.
- A typed-open string map accepts string values and rejects non-strings; a closed record rejects unknown properties.
- Table-driven fixtures enforce every supported primitive, numeric, `minLength`, array, and object assertion at its boundary; typed `additionalProperties` and `minItems` project and validate identically.
- A named wait-condition schema inlines as a property-level `oneOf`, selects one branch, and explains every branch on mismatch.
- Multiple simultaneous failures select the same stable keyword/path under the framework ordering rule, and union branch problems remain in authored order independent of validator-library iteration.
- Registration rejects ambiguous or statically empty branches, impossible argument roots, dangling or cyclic local references, remote references, boolean schemas, unsupported keywords, contradictory coarse types, unnamed schemas, dead declarations, empty or duplicate enum members, numeric schema literals outside the exact RFC 8785 I-JSON domain, invalid numeric assertions, raw property-schema dependency keywords, invalid argument-presence edges, and repeated-array double wrapping.
- Registration rejects unreachable `$defs`. Command-schema assembly deduplicates identical reachable same-name definitions across argument declarations and rejects a same-name canonical disagreement without renaming public definitions; RFC 0015 applies the same rule when commands are grouped.
- A path refinement and a resource reference schema enforce `minLength` before retaining path containment, URI parsing, liveness, ownership, and their specialized diagnostics. Resource `const`/finite-enum fixtures with no syntactically valid id or URI fail as a statically empty composite domain, while a broader string schema remains valid and lets RFC 0012 reject a schema-valid but malformed reference. RFC 0008 named arguments reject overrides.
- `ArgSpec::enumerated` and `arg::enumerated` compile with an owned string collection, an array of `&str`, and the guide's borrowed slice, then copy all three to byte-identical inline enum declarations.
- `handle_constrained` rejects a Rust argument schema that drifts from the command declaration as construction-only `DerivedSchemaDrift` before registry publication, handler invocation, or framework-event creation. `Option<T>` matches an absent-but-non-null Twill property only through the exact derived-null normalization, an authored nullable property retains exact null semantics, reversed nullable `type` order normalizes identically, and ambiguous derived `anyOf` fails. A typed struct with two optional fields matches catalog-owned paired presence edges because the compiled root relation is explicitly excluded after independent validation; any relationship keyword emitted by the derived Rust schema fails as a second authority. After successful registration, a deliberately inconsistent deserializer becomes the only served reason, `TypedDeserializationFailed`, under `ArgumentContractViolation`/`Failed`. Public details name only the catalog operation, optional attributable argument, and stable reason; Rust type/module names and serde text are absent from ordinary, RFC 0020 deferred, generated-host, event, and framework-log projections.
- The guide's inline tagged `WaitCondition` plus optional `timeout_ms` compiles as written and its Schemars validation-semantic property schemas match the named/primitive command declarations exactly, including `const`, enum, `minLength`, closed objects, integer, and optional-but-non-null behavior.
- Table-driven planner, `try_from_number`/`TryFrom`, and serde fixtures agree on every representable negative, unsigned, negative-zero, decimal-spelled, and exponent-spelled mathematical integer and reject the same fractional boundaries. A raw-number fixture that the configured `serde_json` decoder rejects as outside its domain fails at protocol decoding before Twill planning; `JsonInteger` never reparses or claims access to the discarded lexeme. The type preserves the bound `serde_json::Number`, serializes transparently, and `SchemaGenerator::subschema_for` generates exactly the inline `{ "type": "integer" }` schema with no reference, definition, title, storage format, or range; a root `schema_for!` fixture separately proves that ordinary root annotations do not enter property agreement. Exact checked accessors handle integral floating representations and report `None` when conversion would overflow or lose numeric value, its direct and serde errors never display the rejected number, and compile-fail coverage proves the private field cannot be bypassed. A primitive Rust integer whose derived `format` or range differs from the authored schema fails registration rather than relying on a later serde error.
- The guide's RFC 0014 `handle_result` command with resource parameters and constrained typed arguments compiles without a combined method name and applies the same schema comparison and redacted extraction path before producing its application outcome.
- An external-crate fixture calls both context-plus-args and resources-plus-context-plus-args `handle_constrained` forms without naming a marker; inference reuses RFC 0012's public markers and passes warning-denied clippy. A crate-private inferred-marker fixture fails to type-check externally and therefore cannot substitute for this boundary. Compile-fail coverage proves an external implementation cannot satisfy the matching `private::Sealed<M>`, and compile-pass coverage proves Twill's marker-keyed blanket implementations do not overlap.
- Catalog, help, MCP schema, and fingerprint identity change with semantic constraint or presence-edge changes and remain stable across declaration order. A relation changes no `BoundArg` or `schemaMatch` field beyond the already bound argument inventory; its canonical inventory enters the fingerprint preimage directly. Bare command-template and bare operation-id fixtures prove that adding or removing an edge changes the fingerprint even when both arguments are present and the serialized plan is otherwise identical.
- An explicit compatibility schema that omits `description` remains byte-identical in MCP projection while help renders `ArgSpec::summary`; a conflicting authored description fails registration.
- A raw native `tools/call` and the CLI-shaped execution surface accept and reject the same constrained argument values.
- The VBL fixture represents all 63 ungrouped operation input schemas, including the measured 21 property-level unions, five inclusive minima, five inclusive maxima, and two integral divisors. Twill reproduces every released property schema and all root facts other than the explicitly catalog-owned relation addition. The final 62 unaffected command schemas are byte-identical to the release baseline; `screencast_start` differs only by the exact symmetric `dependentRequired` map. Those edges capture the released grouped schema and broker rule that the ungrouped baseline omitted; RFC 0015 owns their exact grouped `dependencies` projection, and RFC 0016 owns the final ambient-binding requiredness needed for canonical 27-tool parity.

## Drawbacks

This introduces a real JSON Schema compiler and validator into planning. The supported subset, canonicalization, ambiguity proof, and redacted diagnostics are materially more machinery than the current coarse type checks.

Raw schemas are less approachable than RFC 0008 builders and can hide domain intent in JSON. Named declarations, direct common builders, help rendering, and dead-declaration checks mitigate that cost but do not remove it.

The initial dialect deliberately rejects valid JSON Schema features. A closed subset makes planner enforcement, client projection, and diagnostics agree; applications needing unsupported composition must propose it explicitly rather than publishing a schema Twill only partly understands.

`handle_constrained` adds another typed handler entry point. It is necessary to keep a derived Rust input type from silently drifting away from an explicitly authored schema, but it increases the builder surface.

## Rationale And Alternatives

**Extend RFC 0008's `FieldShape` until it covers everything.** That would turn a focused named-union model into a second JSON Schema AST and reopen a completed RFC. RFC 0008 remains the ergonomic union dialect; this RFC supplies a separate general contract boundary.

**Let RFC 0015 override generated tool schemas.** A surface-only override could preserve VBL's advertised JSON while the planner continued accepting different values. That is documentation drift at an authorization boundary and is rejected.

**Treat `ArgType::Json` as implicitly trustworthy.** Unconstrained JSON remains useful for intentionally open values, but it cannot justify a precise native schema or typed extraction contract.

**Accept arbitrary JSON Schema and delegate to a library.** Library acceptance does not guarantee stable canonicalization, branch diagnostics, client compatibility, or redaction. Twill fixes a reviewed subset and expands it deliberately.

**Derive every command input from a Rust struct.** This is attractive for new typed applications but does not fit dynamic broker adapters, framework-derived resource carriers, or compatibility schemas authored outside Rust. `handle_constrained` provides checked derivation without making it the only declaration dialect.

## Prior Art

OpenAPI and web frameworks validate request bodies against the same schemas they publish. Serde plus Schemars derives Rust-facing shapes, while JSON Schema validators cover dynamic adapters. GraphQL input objects similarly reject fields and values before resolver execution.

VBL's shipped agent-surface contract provides the concrete compatibility corpus. RFC 0008 provides Twill's property-level union and ambiguity precedent. This RFC combines those lessons while keeping the command catalog—not the MCP adapter—the authority.

## Unresolved Questions

No architectural questions remain for the initial argument-schema boundary. The builder and declaration names in this body become the implementation contract when the RFC is accepted at Stage 1; implementation may not add an alternate authoring path. Any later review-driven rename or ergonomic change must return the RFC to design review and amend the managed body before implementation proceeds. Such a revision must retain the measured schema vocabulary, catalog-owned presence edges, explicit checked migration for legacy typed handlers, automatic checking for result-aware handlers, and serialized redacted branch selections.

## Future Possibilities

Typed builder macros could derive `ArgSpec` summaries and schema declarations from Rust field annotations while retaining explicit path, workspace, and resource semantics.

Later RFCs may add reviewed string patterns and maximums, exclusive or non-integral numeric bounds and divisibility, array uniqueness/maximums, property-count assertions, formats, tuple arrays, boolean schemas, recursive schemas, or richer composition when applications provide acceptance cases, stable diagnostics, and client-compatibility evidence. A broader relationship RFC could add conditional presence or value-dependent command rules without turning `ArgSpec::schema` into an arbitrary whole-input authority. Generated TypeScript and JSON clients could consume the same canonical argument contracts as RFC 0014 result contracts.
