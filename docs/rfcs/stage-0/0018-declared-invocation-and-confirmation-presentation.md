<!-- exo:18 ulid:01kxc0z4r4be6p4k262c04nk5r -->

# RFC 0018: Declared Invocation And Confirmation Presentation

- Status: Draft
- Area: invocation presentation, confirmation, catalog projection, host integration, generated adapters
- Target milestone: v0.4
- Depends on: RFC 0001 (authoritative command catalog), RFC 0003 (confirmation and replay), RFC 0017 (authoritative argument schemas)

## Summary

This RFC lets a command declare its in-progress invocation message and how a pending confirmation should be presented. Invocation messages are bounded static text. Confirmation declarations contain bounded static text, explicit safe interpolation of validated model-visible string or boolean arguments, and disjoint cases selected by argument predicates. Twill validates both at registration and projects one evaluator through catalog data, permission previews, native confirmation bridges, generated host adapters, help, and contract tests.

Confirmation presentation never decides whether a call requires confirmation and never proves that a user approved it. RFC 0003's authorizer and replay or host bridge remain the policy and integrity boundaries. A presentation is evaluated only after policy requests confirmation, or by a host preparing an immutable logical snapshot of the invocation it expects to submit. A host API that separates preparation from invocation must establish value equivalence between those callback snapshots before its UI can be trusted as approval.

The initial model is deliberately small. Invocation messages and confirmation titles are static. Confirmation bodies are sequences of static text and explicitly declared string or boolean argument insertions. Cases may test whether an argument is present or equals a schema-valid string, boolean, or null constant. Cases must be pairwise disjoint and a default is required. Private request context, ambient resource bindings, workspace observations, resolved resources, and arbitrary handler state are never presentation inputs.

## Motivation

Tool annotations tell a host that an operation is read-only, destructive, idempotent, or open-world. They do not tell it how to explain a concrete action. RFC 0003 can render a generic effect preview, but an application may already have precise host confirmation copy tied to its public contract.

VBL demonstrates the gap with two TypeScript switches. One supplies in-progress messages such as “Starting a visible browser session,” “Capturing a browser snapshot,” “Navigating the owned browser tab,” and “Waiting for browser state.” The other asks “Claim browser tab?”, “Close browser tab?”, “Release browser tab?”, or “Bring Chrome forward?” and includes the selected target id. `release_tab` has a distinct durable-handoff case when `leave_visible` is true; that message quotes the required user instruction or explicitly says it is missing. Both switches live separately from the Rust tool catalog and broker policy.

RFC 0015 can generate VBL's tools, schemas, annotations, and dispatch from Twill, and its `NativeConfirmationBridge` can ask a host for a decision. Neither feature can generate VS Code's complete `prepareInvocation` result unless invocation and confirmation copy are also declared. Leaving either switch in the extension would preserve a parallel contract: command names, arguments, and presentation semantics could rename independently, and generated host adapters could not prove parity.

The declaration belongs to the command rather than one surface. “Close owned tab X” describes the operation regardless of whether it appears as a direct tool, a grouped member, or a CLI-shaped command. A surface translates operation selection, while the command supplies the presentation. Host-specific chrome and localization may wrap that core later.

## Guide-Level Explanation

A command with one confirmation message declares it alongside its effects:

```rust
server.command("tabs close", |command| {
    command
        .invocation_message("Closing an owned browser tab")
        .confirmation(
            ConfirmationPresentation::new(
                ConfirmationMessage::new("Close browser tab?")
                    .text("Close owned tab ")
                    .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                    .text("."),
            ),
        )
        .handle(close_tab);
});
```

When no invocation message is declared, the surface uses the exact bounded fallback `Running <display-title>`. Native direct routes use the authored tool title or command summary; native grouped routes use the authored group title or exact public tool name; effect-lane routes use the generated MCP annotation title `<tool-name> execution`. The title is substituted without quoting or case conversion. A grouped route therefore deliberately retains its group title after member selection, matching VBL's contributed-tool behavior. Declaring a command message replaces the fallback for that selected operation and makes the wording a catalog contract shared by every generated host.

This invocation message is presentation copy for a host's prepared invocation view. It neither declares nor emits MCP progress and does not replace `ProgressPhaseSpec`, adapter lifecycle notifications, RFC 0020 task status, or application progress events. The existing effect-lane progress sequence remains transport-owned; a future proposal may derive progress presentation only by defining its own timing, cancellation, and event contract. Merely adding or changing `invocation_message` changes presentation identity but cannot create a progress notification or task transition.

When policy requests confirmation and the command has no declaration, the same route supplies title `Confirmation required` and message `Run <display-title>?`. These three strings are compiled once into the RFC 0015 surface snapshot. Hosts consume them as data and never derive casing, punctuation, or effect prose.

`tab_id` must be a declared string argument. The fallback is presentation text for a malformed or pre-validation host preparation path; after Twill planning, a required argument is always present. Argument rendering is bounded and escaped before it reaches the host.

Conditional copy uses disjoint cases and a required default:

```rust
server.command("tabs release", |command| {
    command
        .confirmation(
            ConfirmationPresentation::new(
                ConfirmationMessage::new("Release browser tab?")
                    .text("Release owned tab ")
                    .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                    .text("; a VBL-created target remains eligible for expiry cleanup."),
            )
            .case(
                ConfirmationPredicate::argument_equals("leave_visible", true),
                ConfirmationMessage::new("Leave browser tab visible?")
                    .text("Release owned tab ")
                    .argument("tab_id", ArgumentRendering::Plain, "(unknown tab)")
                    .text(" and preserve it after this session expires. User instruction: ")
                    .argument(
                        "user_instruction",
                        ArgumentRendering::TrimmedJsonString,
                        "(missing; this request will be rejected)",
                    )
                    .text("."),
            ),
        )
        .handle(release_tab);
});
```

The authorizer still decides whether either call requires confirmation. If it allows the call, Twill never pauses merely because presentation exists. If it requires confirmation, Twill evaluates the case against the validated bound arguments and supplies the resulting title and message to the active RFC 0003 or RFC 0015 confirmation route.

A grouped native tool first resolves its operation selector, then evaluates the selected command's presentation against the command-argument view obtained by excluding that one surface-owned selector. The complete public tool input still retains the selector for routing, dispatch, and callback value equivalence. The `network` tool can therefore dispatch many commands without one worst-case confirmation message or accidentally exposing its routing property as a command argument. A generated host adapter uses the same surface mapping and serialized presentation evaluator over one immutable logical input snapshot. A single-callback adapter can submit that snapshot directly; a separated-callback adapter must establish that its later invocation snapshot is value-equivalent before treating host UI as approval. Whether preparation returns Twill-authored confirmation messages is an explicit host-adapter trigger, separate from both this copy declaration and the server authorizer.

### How Agents And Hosts Should Learn This

Agents see confirmation as a host or framework pause tied to the invocation they attempted. They should not generate confirmation prose, add approval fields, or treat a presentation declaration as permission. Once approval succeeds, the same planned arguments and fingerprint continue through dispatch.

Hosts receive already-rendered bounded copy when confirmation happens after planning. A pre-invocation host adapter receives the declarative evaluator generated with the tool surface and applies it only to an immutable logical input snapshot. It never rereads its callback's mutable source object while rendering. RFC 0019's VS Code API constructs one snapshot in `prepareInvocation` and a separate snapshot in `invoke`, because the platform exposes no shared invocation identifier to the preparation callback and says preparation need not be followed by invocation. That adapter treats UI as presentation while ordinary server authorization remains active unless its hash-covered `HostConfirmationPolicy` explicitly accepts the platform's tested value-equivalence contract for the observed runtime range; RFC 0019 defines that equivalence over the complete public input, including any grouped selector, as byte-identical RFC 8785 argument snapshots and permits trusted satisfaction only when this same trigger matches. A missing or invalid grouped selector selects no command, so the command evaluator is never called; RFC 0019 owns the static local preparation/route failure. A non-object or non-representable host input likewise fails the generated host-input boundary before presentation. Once a command is selected from a representable object input, the evaluator sees the exact command-argument view the grouped dispatcher will bind. A missing, wrong-kind, or normalized-empty interpolated field uses that segment's declared fallback exactly as the reference evaluator specifies. Predicates that do not match reach the default message.

Help may show that a command has custom confirmation presentation and summarize its cases, but it does not teach agents to ask users directly. Confirmation remains host-mediated.

## Reference-Level Explanation

### Model

The command model gains optional presentation declarations:

```rust
pub struct CommandSpec {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<ConfirmationPresentation>,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationPresentation {
    pub default: ConfirmationMessage,
    pub cases: Vec<ConfirmationCase>,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationCase {
    pub when: ConfirmationPredicate,
    pub message: ConfirmationMessage,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfirmationPredicate {
    ArgumentPresent { argument: String },
    ArgumentEquals { argument: String, value: serde_json::Value },
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationMessage {
    pub title: String,
    pub body: Vec<ConfirmationSegment>,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfirmationSegment {
    Text(String),
    Argument {
        argument: String,
        rendering: ArgumentRendering,
        fallback: String,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ArgumentRendering {
    Plain,
    JsonString,
    TrimmedJsonString,
}

impl CommandBuilder {
    pub fn invocation_message(&mut self, message: impl Into<String>) -> &mut Self;
    pub fn confirmation(
        &mut self,
        presentation: ConfirmationPresentation,
    ) -> &mut Self;
}

impl ConfirmationPresentation {
    pub fn new(default: ConfirmationMessage) -> Self;
    pub fn case(
        self,
        when: ConfirmationPredicate,
        message: ConfirmationMessage,
    ) -> Self;
}

impl ConfirmationMessage {
    pub fn new(title: impl Into<String>) -> Self;
    pub fn text(self, text: impl Into<String>) -> Self;
    pub fn argument(
        self,
        argument: impl Into<String>,
        rendering: ArgumentRendering,
        fallback: impl Into<String>,
    ) -> Self;
}

impl ConfirmationPredicate {
    pub fn argument_present(argument: impl Into<String>) -> Self;
    pub fn argument_equals(
        argument: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self;
}
```

`Plain` supports string and boolean arguments. Booleans use `true` or `false`. Strings use the RFC's fixed presentation-string encoder without surrounding quotes. `JsonString` accepts only strings and retains the encoder's surrounding quotes, producing a valid JSON string literal so user instructions cannot visually escape their segment. `TrimmedJsonString` first removes the fixed set corresponding to ECMAScript's [White Space](https://tc39.es/ecma262/multipage/ecmascript-language-lexical-grammar.html#sec-white-space) and [Line Terminator](https://tc39.es/ecma262/multipage/ecmascript-language-lexical-grammar.html#sec-line-terminators) tables from both ends: U+0009, U+000B, U+000C, U+0020, U+00A0, U+1680, U+2000–U+200A, U+202F, U+205F, U+3000, and U+FEFF as whitespace, plus U+000A, U+000D, U+2028, and U+2029 as line terminators. The enumerated version-1 table is authoritative: no future Unicode category change or platform `trim` helper may add another scalar under the same contract. In particular, U+0085 is not trimmed; it remains in the value and the encoder renders it as `\u0085`. Rust and generated TypeScript use this same fixed trimming and escaping table rather than delegating to locale, a host renderer, or a drifting platform JSON helper.

The encoder emits `\"`, `\\`, `\b`, `\f`, `\n`, `\r`, and `\t` for their ordinary JSON characters. Other U+0000–U+001F code points use an uppercase four-digit `\uXXXX` escape. To prevent line breaking or bidirectional reordering inside confirmation UI, DEL/C1 U+007F–U+009F and the fixed presentation-unsafe set U+061C, U+200E–U+200F, U+2028–U+202E, U+2060–U+206F, and U+FEFF also use uppercase `\uXXXX`. Every other Unicode scalar is emitted unchanged. `Plain` removes only the encoder's outer quotes; it retains all escapes. Truncation occurs only between complete input scalars after their complete rendered escape, so it never splits an escape sequence. Authored static invocation text, titles, body text, and fallbacks reject every C0 scalar U+0000–U+001F, DEL/C1 U+007F–U+009F, and every scalar in that fixed presentation-unsafe set rather than escaping or relying on a host to display it safely.

Rust strings already contain only Unicode scalar values. The generated TypeScript evaluator validates UTF-16 rather than treating code units as characters: one well-formed surrogate pair is one scalar for escaping and every bound, while an unpaired high or low surrogate makes that interpolation value invalid and selects its declared fallback in the pure evaluator. The invalid code unit is never emitted, counted, normalized, or copied into prepared presentation. String equality against a declaration constant cannot match it, because every declared constant is a valid Rust string; raw-key presence retains its ordinary case-selection rule and the selected message still uses fallback for the invalid interpolation. RFC 0019's generated invocation path constructs its logical snapshot before invoking this evaluator and rejects a non-scalar host input through the static host-contract boundary, so an actual call never displays confirmation for input that cannot cross the transport. Presentation remains a non-validating view of representable JSON arguments, while every string Rust and TypeScript both accept renders byte-for-byte identically.

For every string rendering, a missing value, wrong-kind value on the pre-invocation path, or string empty after the rendering's normalization uses the declared fallback. Structured arrays and objects are never interpolated. A boolean wrong-kind value likewise uses fallback before planning. Integer and number arguments are not presentation inputs in the initial contract. Registration requires every accepted value of an interpolated property to be renderable by the selected mode: `JsonString` and `TrimmedJsonString` are string-only, while `Plain` may be string-only, boolean-only, or a provably disjoint string/boolean union. A schema branch admitting `null` or any other type is rejected for interpolation; optional omission remains valid and uses fallback. After planning, the authoritative schema therefore guarantees the rendering kind. The pre-invocation fallback remains presentation behavior and does not make an invalid value valid.

The command methods follow RFC 0006's `&mut self -> &mut Self` convention, while the standalone presentation/message builders consume and return their values for fluent composition. Each command presentation slot may be declared once: repeating `invocation_message` or `confirmation` records a command build error even when the values agree, so call order cannot hide one authored contract. `ConfirmationPresentation::case`, `ConfirmationMessage::text`, and `ConfirmationMessage::argument` are additive and preserve authored order; their validation rejects overlapping cases and invalid segments rather than deduplicating them. Every final field serializes into the operation catalog and participates in catalog identity.

### Registration Validation

Registration validates presentations after command arguments and RFC 0017 schemas are complete:

- invocation messages, titles, and static segments are non-empty where required, contain no C0, DEL, C1, or fixed presentation-unsafe scalar, and fit declaration limits;
- every confirmation body contains at least one segment and renders a non-empty body for every validated branch and fallback path;
- every interpolation fallback is non-empty, contains no C0, DEL, C1, or fixed presentation-unsafe scalar, fits the interpolation limit, and is included in the message's worst-case body bound;
- every predicate and interpolation names an argument on that command;
- interpolated arguments have a schema whose complete accepted type domain is compatible with the selected rendering and excludes `null`;
- equality constants are strings, booleans, or null and validate against the argument's authoritative schema;
- case predicates are pairwise disjoint over the argument schema;
- every case predicate is satisfiable, and no case is a tautology that makes the required default unreachable;
- the default is always present;
- no segment references a framework-derived private value, resource object, workspace root, stdin body, request metadata, or output field.

`ArgumentPresent(x)` overlaps `ArgumentEquals(x, value)` and therefore cannot coexist as sibling cases. Two equality cases on the same argument are disjoint only when their canonical constants differ. Predicates on different arguments are not provably disjoint in the initial model and are rejected as siblings. These rules make case selection independent of declaration order.

Presence means only that the argument key exists; it does not mean that the value is non-null, non-empty, truthy, or otherwise schema-valid. On the validated path, an explicitly supplied `null` can therefore satisfy `ArgumentPresent` only when the authoritative schema admitted it. On the pre-invocation path, a present `null` or other invalid value still selects the presence case, and the same logical snapshot may subsequently fail ordinary planning. This raw-key rule keeps presentation from becoming a partial validator and makes the Rust and generated TypeScript evaluators agree before and after validation on every input that planning accepts. Omission reaches the default. Equality uses exact JSON string, boolean, or null equality, matching the supported JSON Schema `const` validator without introducing a cross-language numeric canonicalization contract.

Presence on a required argument is a tautology and is rejected. Equality on a required argument whose schema admits only that constant is likewise rejected because the default could never run. A constant outside the argument schema is unsatisfiable and already fails schema validation. Optional arguments keep absence available to the default, including optional single-constant arguments.

The released VBL presentation corpus needs only one conditional predicate: `leave_visible == true` for the durable release-tab message. The single-argument disjoint model therefore covers the motivating compatibility surface completely; compound predicates remain a future extension rather than an open prerequisite for implementation.

Static invocation messages and titles are limited to 80 Unicode scalar values. Each rendered interpolation is limited to 256 final rendered scalar values, counting escape characters, quotes, and any truncation marker. The renderer first maps each accepted input scalar to one complete escaped chunk. An unquoted `Plain` string whose chunks exceed 256 reserves one slot for `…` and emits the longest whole-chunk prefix of width at most 255 followed by the marker. A `JsonString` or `TrimmedJsonString` whose opening quote, chunks, and closing quote exceed 256 reserves three slots for the opening quote, `…`, and closing quote, then emits the longest whole-chunk prefix of width at most 253 between them. Values that fit remain untruncated; plain booleans are the exact five-or-fewer-scalar `true`/`false` spellings. Empty values after the rendering's normalization select the fallback before this algorithm. Thus a simple 256-scalar plain string fits unchanged while 257 becomes 255 scalars plus `…`; a simple 254-scalar quoted string fits unchanged while 255 becomes 253 scalars plus `…` inside its quotes.

A fallback is authored static display copy rather than an argument value: it renders byte-for-byte, must fit the same 256-scalar interpolation slot, and is used in that slot's worst-case-width calculation. A string-capable rendering has worst-case width 256; a boolean-only `Plain` rendering has value width five; a disjoint string/boolean `Plain` union has width 256. The segment's registration-time contribution is the greater of that value width and its fallback width. Registration rejects a message whose static text plus those per-segment contributions could exceed 1,024. The larger body cap deliberately admits VBL's durable-release copy, which contains a bounded tab id and a separately bounded user instruction plus static explanation. These rules guarantee the final body bound without truncating static/fallback copy or producing an invalid escape sequence.

### Evaluation

Twill exposes one pure evaluator:

```rust
pub struct PermissionPreview {
    // ...existing fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<PreparedConfirmation>,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct PreparedInvocationPresentation {
    pub invocation_message: String,
    pub confirmation: Option<PreparedConfirmation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfacePresentationDefaults {
    invocation_message: String,
    confirmation_title: String,
    confirmation_message: String,
}

impl SurfacePresentationDefaults {
    pub fn invocation_message(&self) -> &str;
    pub fn confirmation_title(&self) -> &str;
    pub fn confirmation_message(&self) -> &str;
}

enum ConfirmationPresentationRequest {
    Omit,
    DeclaredOnly,
    DeclaredOrSurfaceDefault,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct PreparedConfirmation {
    pub operation_id: String,
    pub branch: ConfirmationBranch,
    pub title: String,
    pub message: String,
}

#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfirmationBranch {
    SurfaceDefault,
    Default,
    Case { predicate: ConfirmationPredicate },
}

impl ConfirmationPresentation {
    fn prepare_validated(
        &self,
        operation_id: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
    ) -> PreparedConfirmation;

    fn prepare_unvalidated(
        &self,
        operation_id: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
    ) -> PreparedConfirmation;
}

impl CommandSpec {
    fn prepare_validated_presentation(
        &self,
        defaults: &SurfacePresentationDefaults,
        operation_id: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
        confirmation: ConfirmationPresentationRequest,
    ) -> PreparedInvocationPresentation;

    fn prepare_unvalidated_presentation(
        &self,
        defaults: &SurfacePresentationDefaults,
        operation_id: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
        confirmation: ConfirmationPresentationRequest,
    ) -> PreparedInvocationPresentation;
}
```

`PermissionPreview` stores the authoritative prepared confirmation exactly once. A live preview whose authorizer decision is `RequireConfirmation` contains `Some`, sets its existing `requires_confirmation` field to `true`, and requires the nested `PreparedConfirmation.operation_id` to equal the preview's `operation_id`. An allow preview contains `None`; the existing deny path returns its structured permission error without synthesizing a permission preview. Framework constructors establish these invariants from one completed plan and one evaluator result. The type retains `Serialize` and `JsonSchema` and uses custom deserialization through a private wire helper: `Some` paired with `requires_confirmation: false` or unequal operation ids is rejected. Legacy JSON without the additive field and an explicit `None` normalize to the same omitted representation even when the older preview says confirmation was required; this preserves round trips for pre-RFC data, but no new live require-confirmation path constructs that incomplete form and RFC 0015's private bridge-request constructor accepts only the complete `Some` form.

The enclosing live `ResponseEnvelope` projects that same value into the existing compatibility display slot: `display.title` equals `PreparedConfirmation.title` and `display.summary` equals `PreparedConfirmation.message`. It does not run the evaluator again. `ResponseEnvelope` retains `Serialize` and `JsonSchema` and likewise uses a private wire helper for custom deserialization: whenever a nested `PreparedConfirmation` is present, an absent display or either unequal display field is rejected. An envelope whose preview has no prepared confirmation retains RFC 0002's existing display rules, so legacy envelopes remain accepted. `display_text()` consequently returns the exact prepared message through `display.summary`; the title remains available in both `display.title` and the structured prepared value. RFC 0002 continues to require structured permission data regardless of a requested response profile, so this is a display projection rather than a text-only permission result.

The command-level evaluator chooses the declared invocation message or `SurfacePresentationDefaults::invocation_message()`. `Omit` produces no confirmation. `DeclaredOnly` evaluates a command declaration when present and otherwise produces `None`. `DeclaredOrSurfaceDefault` evaluates the command declaration or produces the surface's static generic confirmation with `ConfirmationBranch::SurfaceDefault`. Surface compilation creates the three defaults from the exact templates above and validates them within the same title/body scalar and control-character bounds for each direct tool and grouped-member routing context; an overlong generated title or message fails surface construction rather than truncating public copy. All three camel-case fields are required in each RFC 0015 snapshot operation entry. Generated host adapters consume those stored defaults and do not recreate title casing, punctuation, or effect prose.

Generic fallback copy is surface-owned in the initial contract. Display titles and grouped routing context differ across effect-lane, native, and generated-host projections, so one catalog-wide fallback would either mention the wrong call shape or force command semantics to absorb packaging text. Declared command copy remains catalog-owned; only the generated fallback belongs to and is fingerprinted through the active surface.

`SurfacePresentationDefaults` is a public read-only compiled view because `mcp-twill-host` is a separate crate that must consume the values without parsing RFC 0015's JSON document. It has private fields, no constructor or mutator, and no serde or schema implementation; its three accessors are the only public surface. `ConfirmationPresentationRequest` and the validated/unvalidated evaluator entrypoints remain crate-private compilation machinery. Authors publicly construct declarations; bridges and host integrations publicly receive bounded `PreparedInvocationPresentation` or `PreparedConfirmation`; generated adapters consume the compiled RFC 0015 snapshot. This exposes the already-chosen fallback without exposing a knob that could create a second fallback or trigger policy outside the surface compiler.

A declared invocation or confirmation message is part of catalog identity. It also enters every origin's existing fingerprint object under `presentationContract`, using the exact normalized camel-case object `{ "invocationMessage": string | null, "confirmation": object | null }`; the member is omitted only when both declarations are absent. Unlike omission-oriented catalog serialization, the present fingerprint object writes an explicit null for whichever half is absent, giving each adopted contract one spelling. Case order is declaration order because it is caller-visible, while predicate/message object keys follow stable JSON ordering. This direct public-contract input gives bare-registry execution, whose RFC 0015 `bareRegistry` marker carries no surface hash, command-local replay binding without depending on the complete catalog hash. Compiled MCP adapters are deliberately more conservative: their surface hash embeds complete catalog identity, so any catalog change invalidates served fingerprints even when this command's presentation is unchanged. That broader serving invalidation belongs to RFC 0015 and does not replace the explicit command-local member.

`confirmation` is the ordinary serialized `ConfirmationPresentation` from the catalog, not rendered copy. The derives above fix predicates and segments as externally tagged camel-case values: for example `{"argumentEquals":{"argument":"leave_visible","value":true}}`, `{"text":"Release owned tab "}`, and `{"argument":{"argument":"tab_id","rendering":"plain","fallback":"(unknown tab)"}}`. `ArgumentRendering::Plain` serializes as `"plain"`. Prepared values use camel-case struct fields such as `invocationMessage` and `operationId`; `ConfirmationBranch` serializes as `"surfaceDefault"`, `"default"`, or `{"case":{"predicate":...}}`. Public declaration deserialization follows the corpus additive unknown-field policy and canonical emission contains only known fields. The fingerprint builder clones the validated declaration value rather than maintaining a second serializer.

Surface-generated fallbacks are part of that surface's identity rather than the command catalog because the fixed templates consume surface-specific display titles and grouped routing context. The initial contract gives embeddings no alternate generic-wording knob: changing `Running <display-title>`, `Confirmation required`, or `Run <display-title>?` requires a reviewed snapshot-version contract. RFC 0015's mandatory `invocation.surface` object binds the effect-lane or native compiled surface hash, so changing a display title or future versioned template changes the fingerprint without copying rendered text into it. An RFC 0003 or RFC 0015 authorizer decision of `RequireConfirmation` uses `DeclaredOrSurfaceDefault`, guaranteeing one `PreparedConfirmation`; a generated host's `DeclaredPresentation` trigger uses `DeclaredOnly`, while its effect-default trigger uses `DeclaredOrSurfaceDefault` only after the selected effect asks for host confirmation. `PreparedConfirmation` remains the policy-gated portion passed to replay or a native bridge.

After planning, `arguments` is the validated model-visible bound-argument map with framework-private facts removed. Case evaluation selects at most one case by construction, otherwise the default. Rendering cannot fail after registration and validation.

`prepare_validated` is the infallible post-planning path. `prepare_unvalidated` is the portable pre-invocation path used by generated host adapters after resolving any RFC 0015 group selector. A direct route passes its complete argument snapshot. A grouped route passes a read-only selected-command view of the same snapshot with exactly the compiled selector property excluded; RFC 0015 already rejects selector collisions with member arguments. Once an operation is selected, a missing, empty-after-normalization, structured, null, or wrong-kind interpolation uses its declared fallback. An equality predicate with a wrong-kind value simply does not match and reaches another disjoint case or the default, matching ordinary JSON equality; a presence predicate follows the raw-key rule above. Invalid group selectors or unmapped tools fail in surface routing before either command evaluator is called. Unrelated arguments are not treated as validated by this evaluator. The actual Twill planner remains authoritative and may reject the call after host confirmation for any input constraint. The evaluator observes one framework-owned JSON tree and never inserts fallback values. An adapter claiming preparation/dispatch binding submits the complete direct snapshot, submits the complete grouped snapshot whose dispatcher removes the same selector, or proves its invocation callback received a value-equivalent complete tree; object identity and original JavaScript wire spelling are not part of the portable contract. Surface defaults contain no interpolation and therefore render identically on validated and unvalidated paths.

### Authorization And Replay

The decision sequence is fixed:

1. planning validates arguments and computes the invocation fingerprint;
2. the registry `PermissionPolicy` hard gate allows the plan to proceed;
3. the adapter authorizer returns allow, deny, or require confirmation;
4. only require-confirmation requests `DeclaredOrSurfaceDefault` presentation;
5. RFC 0003 replay or an RFC 0015 host bridge binds approval to the fingerprint;
6. dispatch proceeds once.

Presentation cannot return an authorization decision. Changing presentation changes catalog identity and generated-host artifacts but does not change effects, lane, or authorizer policy. A replay token issued under one catalog identity does not approve an invocation after presentation changes.

The RFC 0015 `NativeConfirmationRequest` owns the same `PermissionPreview` used by the response path and exposes its nested prepared confirmation through a read-only `presentation()` accessor. It does not store another `PreparedConfirmation`. Its validated argument map remains available to a deliberately custom bridge, while every framework-generated bridge and adapter renders the prepared copy directly rather than rebuilding it.

Cancellation or denial discards the prepared copy and never dispatches. The initial framework event and log contract records no rendered copy, invocation-message source, or confirmation branch. A generated host evaluates presentation before the server sees the call, so a server event could not truthfully assert which host UI branch was shown without accepting a new host-authored observation; native bridges and permission previews also need no duplicate event archive. Prepared previews and bridge requests are the declared protocol destinations for those facts. A trusted embedding or application may apply its own logging policy after receiving prepared copy, but Twill supplies no switch that turns presentation into framework telemetry.

### Projection

- **Operation catalog.** Invocation message, confirmation cases, predicates, segments, renderings, and bounds project under `presentation` and participate in the catalog hash.
- **Help.** Full help renders the declared invocation message and says that host confirmation may use declared copy, summarizing predicate conditions without fabricating argument values.
- **Permission preview.** A `RequireConfirmation` decision always includes `PreparedConfirmation` in structured preview. The enclosing display hint projects its exact title and message, with deserialization rejecting disagreement. Allow and deny decisions include no confirmation presentation; the existing deny path returns no permission preview. Preview never invokes a confirmation bridge.
- **Native surface.** Direct tools carry the command evaluator. Group tools dispatch presentation by selector. Surface snapshots include the evaluator needed by generated host adapters.
- **Generated host adapters.** Preparation always returns the declared or generic invocation message and returns selected confirmation messages only when the host profile's trigger requests them. A single-callback host submits the same snapshot; a separated-callback host uses the same operation mapping and must establish value equivalence before host UI can stand in for server approval.
- **Contracts.** `check_confirmation_projection` compares declaration, catalog, help, preview, native snapshot, and generated-host fixtures.

### Required Invariants

- Confirmation presentation never changes whether confirmation is required or whether an invocation is allowed.
- Generated-host trigger policy and server-side authorization remain distinct, explicitly configured boundaries.
- Permission preview projects confirmation presentation exactly when its reported decision is `RequireConfirmation`; the structured value and compatibility display hint derive from one prepared value without invoking a bridge or evaluator twice.
- Invocation presentation never performs work and never substitutes for progress or task state.
- Declaring or editing `invocation_message` emits no MCP progress, changes no `ProgressPhaseSpec`, framework event, or RFC 0020 task transition, and does not replace the adapter's transport-owned lifecycle notifications.
- Every non-fallback interpolated argument value comes from an explicitly named, model-visible string or boolean property and satisfies its authoritative schema; pre-invocation missing/wrong-kind/empty values contribute only the declaration-owned fallback, never the rejected value.
- Private identity, workspace observations, ambient references, resolved resources, and raw metadata can never enter presentation.
- Rendered invocation and confirmation text, source, and branch identity never enter framework events or framework-owned logs.
- Conditional cases are pairwise disjoint and select the same prepared message independently of declaration order. Authored case order remains caller-visible in help and serialized declarations, so reordering it intentionally changes catalog identity and the `presentationContract` fingerprint member.
- Rendered copy is bounded, escaped, and deterministic. Post-planning copy is tied to the same arguments and fingerprint dispatch uses; pre-invocation copy is tied to its callback-local snapshot and requires an explicit host value-equivalence contract before it can stand in for approval of a later callback.
- Surface defaults use the three exact version-1 templates and surface-specific display title; no embedding can author alternate generic wording under the same snapshot version.
- Rust and generated TypeScript count Unicode scalar values identically; a valid UTF-16 surrogate pair is one scalar, while an unpaired surrogate is never displayed. The pure pre-validation evaluator selects its declared fallback, and RFC 0019 callback snapshot construction rejects the non-representable input before preparation returns UI or invocation dispatches it.
- Each pre-invocation callback evaluates one immutable logical input snapshot without inserting fallbacks or approvals. A binding-capable host submits that snapshot or an explicitly proven value-equivalent invocation snapshot; otherwise presentation remains separate from server authorization.
- Catalog, preview, native bridge, and generated-host presentation derive from one declaration.

### Implementation Phases

1. Add invocation and confirmation presentation models, builder DSL, registration validation, catalog identity, and pure bounded renderer.
2. Integrate prepared copy with RFC 0003 previews and replay without changing authorization policy.
3. Add catalog/help projection, the exact `presentationContract` fingerprint member, and the host-neutral serialized evaluator consumed by later surface compilers.
4. Import VBL's portable presentation vectors and add owner-local non-disclosure, escaping, preview, replay, and fingerprint acceptance coverage.
5. RFC 0015 later integrates the already-public evaluator and `PreparedConfirmation` with direct/grouped surfaces, generic surface defaults, and `NativeConfirmationBridge`; that downstream slice introduces no alternate presentation declaration or renderer.
6. RFC 0019 later consumes the same evaluator in generated VS Code `prepareInvocation` source and owns its TypeScript execution gate.

### Acceptance Tests

Acceptance lives in `crates/mcp-twill/tests/presentation.rs` and the checked-in JSON vector set supplied by RFC 0015's evidence-only fixture bootstrap. That bootstrap provides provenance-checked observations without introducing a surface compiler or runtime API. The owner-local RFC 0018 landing completes the Rust renderer suite before RFC 0015's public implementation integrates it. RFC 0015 then validates member routing and surface defaults against the relevant titles in `surface-catalog.json`, while RFC 0019 owns generated TypeScript and contribution parity with `vscode-package.json`. The vectors are a reviewed extraction because v0.4.8 exposed presentation as TypeScript control flow, not machine-readable data. Rust executes the complete portable vector set before either downstream compiler consumes it, and VBL's final TypeScript gate executes the same vectors before installed-host acceptance.

- A declared invocation message projects through catalog, help, the host-neutral serialized evaluator, and the exact `presentationContract` fingerprint member; editing it changes catalog identity and the invocation fingerprint without changing authorization policy. RFCs 0015 and 0019 own native-snapshot and generated-host consumption respectively.
- Invocation-message fixtures leave existing MCP progress text/count, authored `ProgressPhaseSpec`, framework events, and task state byte-for-byte unchanged; the message appears only on the declared presentation projections and identities.
- Fingerprint vectors prove absent declarations omit `presentationContract`, explicit `None` normalizes to omission, declaration object keys are canonical, case order remains significant, and changing only declaration copy changes the fingerprint for bare, effect-lane, and native origins.
- Declaration and prepared-value round trips prove the exact externally tagged camel-case predicate/segment/branch forms, camel-case struct fields, and lower-camel rendering strings. Additive unknown declaration fields normalize out of canonical emission; Rust-default variant names or snake-case prepared fields fail fixtures and generated TypeScript parity.
- Direct `CommandSpec` presentation fields and the mutable `CommandBuilder` methods produce byte-identical catalog declarations and validation failures for equivalent input.
- Repeating either command presentation setter fails construction even when equal; additive case and segment order remains caller-visible and hash-significant where specified.
- Legacy `CommandSpec` JSON without `invocationMessage`/`confirmation` and explicit `None` values normalize to byte-identical catalog data and hash input; no prepared or projected presentation appears until adopted.
- The guide's complete close-tab and conditional release-tab command-builder paths compile as written. The static close-tab confirmation renders the validated `tab_id` with exact title and message and changes catalog identity when edited.
- `leave_visible: true` selects the durable-handoff case; false or absent selects the default. Reordering disjoint cases leaves the selected prepared copy unchanged but intentionally changes serialized declaration order, help order, catalog identity, and the `presentationContract` fingerprint member.
- An optional nullable argument distinguishes omission from explicit `null` under `ArgumentPresent`; string, boolean, and null equality vectors prove registration and Rust/TypeScript evaluators use the same `const` semantics. On the pre-invocation path a present schema-invalid `null` still selects a presence case, while the same logical snapshot later fails planning; accepted inputs select the same branch before and after validation.
- `JsonString` quotes and escapes a user instruction, truncates it deterministically at complete escape boundaries, and never emits raw line breaks, C0/C1 controls, bidi controls, line separators, or the other fixed presentation-unsafe code points. Exact vectors cover every short JSON escape, each unsafe range with uppercase `\uXXXX`, BMP/non-BMP boundaries, a valid surrogate pair counted as one scalar in TypeScript, and isolated high/low surrogates selecting fallback without appearing in output; `Plain` removes only outer quotes. Boundary vectors prove that 256 simple plain scalars and 254 simple quoted scalars fit unchanged, while the next scalar produces respectively a 255-scalar or 253-scalar whole-chunk prefix plus the correctly placed marker and quotes. `TrimmedJsonString` removes every scalar in the fixed `TrimString` table at both ends in Rust and generated TypeScript, sends a table-only string to fallback, and proves U+0085 is retained then escaped rather than trimmed.
- Body-bound vectors prove the two-interpolation durable-release guide message compiles, renders within 1,024 scalars when both values reach their 256-scalar slots, accepts an exactly 1,024-scalar worst case, and rejects a 1,025-scalar declaration before publication without truncating static or fallback copy.
- Missing optional interpolation uses display fallback without adding the value to the submitted call.
- Registration rejects missing arguments, structured, numeric, nullable, or otherwise rendering-incompatible interpolation domains, numeric equality constants, schema-invalid constants, unsatisfiable or tautological cases, overlapping cases, predicates on different arguments, empty titles, bodies, segments, or fallbacks, C0, DEL, C1, or fixed presentation-unsafe scalars in any authored copy, and declarations exceeding static or worst-case rendered bounds.
- An allow permission preview contains no confirmation presentation, and a deny decision retains its existing structured error with no permission preview; neither invokes a confirmation bridge. Require-confirmation produces one `PreparedConfirmation`; permission preview returns it without invoking the bridge, while ordinary execution binds bridge/replay approval of that same prepared contract to the fingerprint and dispatches once after approval.
- `PermissionPreview` omits `confirmation` for allow, while deny retains its existing no-preview error path; every new live require-confirmation preview stores exactly one authoritative prepared value. Legacy omission and explicit `None` normalize identically, including a historical `requiresConfirmation: true` preview; custom preview deserialization rejects `Some` with false or unequal outer/nested operation ids. Live envelope construction copies that value's title and message into `DisplayHint` without reevaluation, and custom envelope deserialization rejects a missing or unequal display whenever `Some` is present. RFC 0015's bridge accessor borrows the nested object rather than storing another prepared copy, and the bridge constructor rejects the accepted legacy-incomplete form.
- Allow, denial, confirmation, cancellation, and successful dispatch never place rendered invocation/confirmation text, invocation source, or confirmation branch identity in framework events or logs. Preview and bridge outputs retain those facts only at their declared destination.
- Conversation identity, host workspace roots, ambient session references, and private digests are absent from catalog serialization, prepared copy, previews, events, and generated adapters.
- A grouped tool selects the member presentation from its operation discriminator; an invalid discriminator returns no custom confirmation. Wrong-kind or empty interpolations use fallback, equality predicates with wrong-kind values select the default, and ordinary planning still rejects invalid input.
- A require-confirmation decision for a command without declared confirmation copy produces the active surface's stable generic title/message; `DeclaredOnly` returns no confirmation for that same command, and changing the surface fallback changes the surface identity and complete invocation fingerprint without changing catalog identity.
- Portable VBL vectors for start-session, snapshot, screenshot, navigation, click, form filling, wait, claim-tab, close-tab, ordinary/durable release-tab, and focus-tab match established titles and messages in the Rust evaluator. RFC 0019 owns proof that generated VS Code source removes both hand-written switches and executes the same vectors.
- CLI-shaped preview and the RFC 0015 in-process native bridge render identical copy for the same command arguments. RFC 0019 acceptance extends that equality to the generated pre-invocation host adapter.
- A separated-callback host fixture proves preparation and invocation each use one immutable snapshot, and that presentation alone conveys no server approval. RFC 0019 acceptance owns the only initial trust composition: a hash-covered, runtime-range-checked host policy may satisfy one base `RequireConfirmation` only when this same compiled trigger matches the later value-equivalent invocation snapshot.
- Effect-lane, native-direct, and native-grouped fixtures compile their distinct display titles through the same three fixed fallback templates. No builder or serialized declaration accepts alternate generic fallback prose; changing a title changes surface identity, while changing template prose requires a new supported snapshot version.

## Drawbacks

Invocation and confirmation presentation become additional catalog contracts that authors must maintain. Precise copy is valuable, but it increases builder and contract-test surface for commands that need it.

The restricted predicate language cannot express arbitrary application state or compound conditions. That is intentional: evaluating application callbacks during confirmation would make previews impure and generated adapters impossible. More predicates should arrive only with a disjointness and projection story.

Pre-invocation host confirmation necessarily occurs before the authoritative planner. A generated adapter can render safe copy from an immutable snapshot, but the server may still reject the later invocation snapshot. The declaration does not turn host UI into validation or prove that a separated host callback received an equivalent value.

Static English copy is not a localization system. The initial contract prioritizes deterministic parity and generated adapters; localization requires stable message identities and host negotiation beyond this RFC.

## Rationale And Alternatives

**Keep invocation and confirmation copy in each host adapter.** This preserves host flexibility by preserving duplicate authority. Command names, argument renames, conditional cases, and generated tools can drift from the UI that explains them.

**Generate copy only from effects.** Generic “This command writes” previews remain a useful fallback. They cannot express “close tab X” or VBL's explicit durable-handoff distinction.

**Use an arbitrary Rust callback.** A callback could inspect anything and render any string, but it would not serialize into the catalog, run in generated TypeScript, or prove non-disclosure. The declarative evaluator is intentionally portable and pure.

**Let presentation require confirmation.** This would collapse UI copy into authorization policy and make a missing declaration a security decision. RFC 0003 remains the sole policy source.

**Interpolate the whole argument object.** This is easy and risks leaking tokens, large payloads, or hostile formatting. Authors opt string or boolean arguments in one at a time under bounded rendering.

## Prior Art

VS Code's `LanguageModelTool.prepareInvocation` separates invocation presentation from execution, permits preparation without invocation, and exposes no shared correlation token to the preparation callback. Command frameworks commonly declare confirmation prompts alongside destructive operations. Structured UI message systems separate static text from escaped interpolations rather than assembling untrusted strings.

RFC 0003 supplies Twill's authorization and replay boundary. RFC 0017 supplies authoritative scalar schemas. VBL supplies the concrete conditional-copy and generated-host acceptance fixture.

## Unresolved Questions

No architectural questions remain for the initial presentation boundary. The declaration and builder names in this body are the current Stage-0 proposal; any review-driven rename must amend the managed RFC before Stage 1, and implementation may not introduce an alternate renderer or authoring path. Such a revision must retain public declarations and prepared copy, crate-private compilation policy, string/boolean interpolation, and one-argument disjoint predicates.

## Future Possibilities

Stable message ids and host-provided translations could localize static segments while preserving argument interpolation and fingerprints. Reviewed conjunction predicates could support additional cases once the framework can prove their disjointness.

Numeric equality and interpolation may be added with a fixed Rust/TypeScript canonical-number contract and safe-range rules when a real presentation needs them. Structured values remain outside interpolation; hosts may eventually render them only through separately declared typed presentation components.

Hosts may eventually render structured confirmation cards from effects, workspace scopes, and resource summaries alongside the declared title and message. Those additions must continue excluding private request context and must not change authorization policy.
