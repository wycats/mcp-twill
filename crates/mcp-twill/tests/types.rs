//! Acceptance tests for named argument types and unions (RFC 0008).

use std::collections::BTreeMap;

use mcp_twill::{
    ArgSpec, ArgVariants, CommandOutput, CommandRegistry, CommandSpec, Field, FrameworkError,
    HelpRequest, PermissionEffect, PermissionSpec, RunRequest, TypeDecl, Variant,
};
use serde_json::json;

fn request(command: &str, args: serde_json::Value) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: serde_json::from_value::<BTreeMap<String, serde_json::Value>>(args).unwrap(),
        stdin: None,
        output: None,
        mode: mcp_twill::RunMode::Execute,
        approval: None,
        dry_run: false,
    }
}

/// A structural union: variants distinguished by their required fields.
fn element_target_type() -> TypeDecl {
    TypeDecl::union("element-target", "How to locate an element on the page")
        .variant(
            Variant::new("reference", "Locate by accessibility reference")
                .field(Field::string("ref", "Accessibility reference id")),
        )
        .variant(
            Variant::new("css", "Locate by CSS selector")
                .field(Field::string("css", "CSS selector"))
                .field(Field::string("frame_ref", "Frame reference").optional()),
        )
}

/// A discriminated union: variants selected by a constant `kind` field.
fn condition_type() -> TypeDecl {
    TypeDecl::union("condition", "A wait condition")
        .variant(
            Variant::new("delay", "Wait a fixed duration")
                .field(Field::constant("kind", "delay"))
                .field(Field::integer("duration_ms", "How long to wait")),
        )
        .variant(
            Variant::new("text", "Wait for text")
                .field(Field::constant("kind", "text"))
                .field(Field::string("text", "Text to wait for"))
                .field(
                    Field::enumerated("state", &["visible", "hidden"], "Desired state")
                        .optional(),
                ),
        )
        .variant(
            Variant::new("element", "Wait for an element state")
                .field(Field::constant("kind", "element"))
                .field(Field::reference(
                    "target",
                    "element-target",
                    "Element to wait on",
                ))
                .field(Field::string("state", "Desired element state")),
        )
        .variant(
            Variant::new("url", "Wait for a URL")
                .field(Field::constant("kind", "url"))
                .field(Field::string("value", "URL fragment or pattern")),
        )
        .variant(
            Variant::new("load", "Wait for a load state")
                .field(Field::constant("kind", "load"))
                .field(Field::enumerated(
                    "state",
                    &["dom_content_loaded", "load", "network_idle"],
                    "Load state to wait for",
                )),
        )
}

/// A union whose variants carry nested references, used repeated.
fn form_field_type() -> TypeDecl {
    TypeDecl::union("form-field", "One form field operation")
        .variant(
            Variant::new("text", "Fill a text control")
                .field(Field::constant("kind", "text"))
                .field(Field::reference("target", "element-target", "Control to fill"))
                .field(Field::string("value", "Text value")),
        )
        .variant(
            Variant::new("checked", "Set a checkbox")
                .field(Field::constant("kind", "checked"))
                .field(Field::reference("target", "element-target", "Checkbox to set"))
                .field(Field::boolean("checked", "Desired checked state")),
        )
}

fn registry() -> CommandRegistry {
    CommandRegistry::new("types-test", "Named type test server")
        .declare_type(element_target_type())
        .declare_type(condition_type())
        .declare_type(form_field_type())
        .register(
            CommandSpec::new(["page", "click"], "Click", "Click an element")
                .with_arg(ArgSpec::named(
                    "target",
                    "element-target",
                    "Element to click",
                ))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "page",
                    "Clicks an element",
                )),
            |context: mcp_twill::CommandContext| async move {
                Ok(CommandOutput::structured(json!({
                    "variants": context.plan.bound_args["target"].variants,
                })))
            },
        )
        .register(
            CommandSpec::new(["page", "wait"], "Wait", "Wait for a condition")
                .with_arg(ArgSpec::named("condition", "condition", "What to wait for"))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Read,
                    "page",
                    "Waits for a condition",
                )),
            |context: mcp_twill::CommandContext| async move {
                Ok(CommandOutput::structured(json!({
                    "variants": context.plan.bound_args["condition"].variants,
                })))
            },
        )
        .register(
            CommandSpec::new(["page", "fill-form"], "Fill form", "Fill multiple fields")
                .with_arg(
                    ArgSpec::named("fields", "form-field", "Fields to fill").repeated(),
                )
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "page",
                    "Fills form fields",
                )),
            |context: mcp_twill::CommandContext| async move {
                Ok(CommandOutput::structured(json!({
                    "variants": context.plan.bound_args["fields"].variants,
                })))
            },
        )
}

// --- Structural union matching ---

#[test]
fn structural_union_matches_by_required_fields() {
    let plan = registry()
        .build_plan(&request(
            "page click --target $args.target",
            json!({ "target": { "ref": "node-4" } }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["target"].variants,
        Some(ArgVariants::Single("reference".to_string()))
    );

    let plan = registry()
        .build_plan(&request(
            "page click --target $args.target",
            json!({ "target": { "css": "#submit" } }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["target"].variants,
        Some(ArgVariants::Single("css".to_string()))
    );
}

#[test]
fn structural_union_mismatch_reports_every_variant() {
    let error = registry()
        .build_plan(&request(
            "page click --target $args.target",
            json!({ "target": { "selector": "#submit" } }),
        ))
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("argument `target` does not match `element-target`"));
    assert!(message.contains("not `reference`: missing required field `ref`"));
    assert!(message.contains("not `css`: missing required field `css`"));
}

#[test]
fn union_matching_rejects_unknown_fields() {
    let error = registry()
        .build_plan(&request(
            "page click --target $args.target",
            json!({ "target": { "ref": "node-4", "extra": true } }),
        ))
        .unwrap_err();
    assert!(error.to_string().contains("not `reference`: unknown field `extra`"));
}

// --- Discriminated union matching ---

#[test]
fn discriminated_union_selects_by_constant() {
    let plan = registry()
        .build_plan(&request(
            "page wait --condition $args.condition",
            json!({ "condition": { "kind": "delay", "duration_ms": 250 } }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["condition"].variants,
        Some(ArgVariants::Single("delay".to_string()))
    );

    let plan = registry()
        .build_plan(&request(
            "page wait --condition $args.condition",
            json!({ "condition": {
                "kind": "element",
                "target": { "css": "#done" },
                "state": "visible"
            } }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["condition"].variants,
        Some(ArgVariants::Single("element".to_string()))
    );
}

#[test]
fn wrong_constant_names_the_expected_constants() {
    let error = registry()
        .build_plan(&request(
            "page wait --condition $args.condition",
            json!({ "condition": { "kind": "sleep", "duration_ms": 250 } }),
        ))
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("not `delay`: field `kind` must be the constant `delay`"));
    assert!(message.contains("not `text`: field `kind` must be the constant `text`"));
    assert!(message.contains("not `load`: field `kind` must be the constant `load`"));
}

#[test]
fn mismatch_reports_variants_in_declaration_order() {
    let error = registry()
        .build_plan(&request(
            "page wait --condition $args.condition",
            json!({ "condition": { "kind": "sleep" } }),
        ))
        .unwrap_err();
    let message = error.to_string();
    let delay = message.find("not `delay`").unwrap();
    let text = message.find("not `text`").unwrap();
    let element = message.find("not `element`").unwrap();
    let url = message.find("not `url`").unwrap();
    let load = message.find("not `load`").unwrap();
    assert!(delay < text && text < element && element < url && url < load);
}

#[test]
fn nested_reference_failure_names_the_nested_path() {
    let error = registry()
        .build_plan(&request(
            "page wait --condition $args.condition",
            json!({ "condition": {
                "kind": "element",
                "target": { "selector": "#done" },
                "state": "visible"
            } }),
        ))
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("`condition.target` does not match `element-target`"));
}

// --- Repeated named arguments ---

#[test]
fn repeated_argument_records_per_element_variants_in_order() {
    let plan = registry()
        .build_plan(&request(
            "page fill-form --fields $args.fields",
            json!({ "fields": [
                { "kind": "text", "target": { "css": "#name" }, "value": "Ada" },
                { "kind": "checked", "target": { "ref": "node-2" }, "checked": true },
                { "kind": "text", "target": { "ref": "node-3" }, "value": "ada@example.com" }
            ] }),
        ))
        .unwrap();
    assert_eq!(
        plan.bound_args["fields"].variants,
        Some(ArgVariants::PerElement(vec![
            "text".to_string(),
            "checked".to_string(),
            "text".to_string(),
        ]))
    );
}

#[test]
fn repeated_element_failure_is_indexed() {
    let error = registry()
        .build_plan(&request(
            "page fill-form --fields $args.fields",
            json!({ "fields": [
                { "kind": "text", "target": { "css": "#name" }, "value": "Ada" },
                { "kind": "checked", "target": { "selector": "bad" }, "checked": true }
            ] }),
        ))
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("argument `fields[1]` does not match `form-field`"));
    assert!(message.contains("`fields[1].target` does not match `element-target`"));
}

#[test]
fn repeated_named_argument_requires_an_array() {
    let error = registry()
        .build_plan(&request(
            "page fill-form --fields $args.fields",
            json!({ "fields": { "kind": "text" } }),
        ))
        .unwrap_err();
    assert!(matches!(error, FrameworkError::InvalidArgumentType(..)));
}

// --- Registration validation ---

fn base_registry(types: Vec<TypeDecl>, spec_type: &str) -> mcp_twill::Result<()> {
    let mut registry = CommandRegistry::new("validation-test", "validation");
    for decl in types {
        registry = registry.declare_type(decl);
    }
    let registry = registry.register(
        CommandSpec::new(["demo"], "Demo", "Demo command")
            .with_arg(ArgSpec::named("value", spec_type, "The value"))
            .with_permission(PermissionSpec::new(
                PermissionEffect::Read,
                "demo",
                "Reads",
            )),
        |_context| async { Ok(CommandOutput::text("ok")) },
    );
    registry.validate_types()
}

#[test]
fn dangling_type_reference_fails_validation() {
    let error = base_registry(vec![], "missing-type").unwrap_err();
    assert!(error.to_string().contains("missing-type"));
}

#[test]
fn dangling_field_reference_fails_validation() {
    let types = vec![
        TypeDecl::union("outer", "Outer").variant(
            Variant::new("only", "Only variant")
                .field(Field::reference("inner", "missing-inner", "Nested value")),
        ),
    ];
    let error = base_registry(types, "outer").unwrap_err();
    assert!(error.to_string().contains("missing-inner"));
}

#[test]
fn reference_cycles_fail_validation() {
    let types = vec![
        TypeDecl::union("a", "A").variant(
            Variant::new("only", "Only").field(Field::reference("b", "b", "B value")),
        ),
        TypeDecl::union("b", "B").variant(
            Variant::new("only", "Only").field(Field::reference("a", "a", "A value")),
        ),
    ];
    let error = base_registry(types, "a").unwrap_err();
    assert!(error.to_string().contains("cycle"));
}

#[test]
fn dead_types_fail_validation() {
    let types = vec![element_target_type(), condition_type(), form_field_type()];
    // Only element-target is referenced; condition and form-field are dead.
    let error = base_registry(types, "element-target").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("condition") || message.contains("form-field"));
}

#[test]
fn empty_unions_fail_validation() {
    let types = vec![TypeDecl::union("empty", "No variants")];
    let error = base_registry(types, "empty").unwrap_err();
    assert!(error.to_string().contains("empty"));
}

#[test]
fn duplicate_variant_names_fail_validation() {
    let types = vec![
        TypeDecl::union("dup", "Duplicate variants")
            .variant(Variant::new("same", "First").field(Field::string("a", "A")))
            .variant(Variant::new("same", "Second").field(Field::string("b", "B"))),
    ];
    let error = base_registry(types, "dup").unwrap_err();
    assert!(error.to_string().contains("same"));
}

#[test]
fn ambiguous_variants_fail_validation() {
    // Both variants require `name`; the second's extra field is optional, so a
    // value with just `name` matches both.
    let types = vec![
        TypeDecl::union("ambiguous", "Ambiguous union")
            .variant(Variant::new("first", "First").field(Field::string("name", "Name")))
            .variant(
                Variant::new("second", "Second")
                    .field(Field::string("name", "Name"))
                    .field(Field::string("extra", "Extra").optional()),
            ),
    ];
    let error = base_registry(types, "ambiguous").unwrap_err();
    assert!(error.to_string().contains("ambiguous"));
}

#[test]
fn contradictory_constants_disambiguate_variants() {
    // Same required field names, but contradictory constants: not ambiguous.
    let types = vec![
        TypeDecl::union("tagged", "Tagged union")
            .variant(
                Variant::new("one", "One")
                    .field(Field::constant("kind", "one"))
                    .field(Field::string("value", "Value")),
            )
            .variant(
                Variant::new("two", "Two")
                    .field(Field::constant("kind", "two"))
                    .field(Field::string("value", "Value")),
            ),
    ];
    assert!(base_registry(types, "tagged").is_ok());
}

// --- Schema projection ---

#[test]
fn named_argument_schema_is_inlined_as_property_level_one_of() {
    let registry = registry();
    let spec = registry
        .command_specs()
        .find(|spec| spec.path == ["page", "click"])
        .unwrap();
    let schema = registry.arg_schema(spec);

    assert!(schema.get("oneOf").is_none(), "no top-level oneOf");
    let target = &schema["properties"]["target"];
    let variants = target["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0]["properties"]["ref"]["type"], "string");
    assert_eq!(variants[0]["additionalProperties"], false);

    let rendered = schema.to_string();
    assert!(!rendered.contains("$ref"));
    assert!(!rendered.contains("$defs"));
}

#[test]
fn repeated_named_argument_schema_wraps_the_union_in_an_array() {
    let registry = registry();
    let spec = registry
        .command_specs()
        .find(|spec| spec.path == ["page", "fill-form"])
        .unwrap();
    let schema = registry.arg_schema(spec);

    let fields = &schema["properties"]["fields"];
    assert_eq!(fields["type"], "array");
    let variants = fields["items"]["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), 2);
    // Nested reference is fully inlined.
    let nested = &variants[0]["properties"]["target"]["oneOf"];
    assert!(nested.is_array());
}

#[test]
fn discriminated_variants_project_constants_and_enums() {
    let registry = registry();
    let spec = registry
        .command_specs()
        .find(|spec| spec.path == ["page", "wait"])
        .unwrap();
    let schema = registry.arg_schema(spec);

    let variants = schema["properties"]["condition"]["oneOf"].as_array().unwrap();
    assert_eq!(variants[0]["properties"]["kind"]["const"], "delay");
    let load = &variants[4];
    let states = load["properties"]["state"]["enum"].as_array().unwrap();
    assert_eq!(states.len(), 3);
}

// --- Help rendering ---

#[test]
fn help_renders_referenced_types_with_variants() {
    let help = registry().help(HelpRequest {
        command: Some("page wait".to_string()),
        topic: None,
        detail: None,
    });
    assert!(help.text.contains("Type `condition`: A wait condition"));
    assert!(help.text.contains("- delay: Wait a fixed duration"));
    // Transitively referenced type renders too.
    assert!(help.text.contains("Type `element-target`"));
}

#[test]
fn help_renders_each_type_once() {
    let help = registry().help(HelpRequest {
        command: Some("page fill-form".to_string()),
        topic: None,
        detail: None,
    });
    // form-field references element-target from both variants; it renders once.
    assert_eq!(help.text.matches("Type `element-target`").count(), 1);
}

// --- Fingerprints ---

#[test]
fn fingerprint_diverges_for_different_matched_variants() {
    let by_ref = registry()
        .build_plan(&request(
            "page click --target $args.target",
            json!({ "target": { "ref": "node-4" } }),
        ))
        .unwrap();
    let by_css = registry()
        .build_plan(&request(
            "page click --target $args.target",
            json!({ "target": { "css": "#submit" } }),
        ))
        .unwrap();
    assert_ne!(by_ref.invocation_fingerprint, by_css.invocation_fingerprint);
}

#[test]
fn fingerprint_diverges_for_different_per_element_sequences() {
    let text_first = registry()
        .build_plan(&request(
            "page fill-form --fields $args.fields",
            json!({ "fields": [
                { "kind": "text", "target": { "css": "#a" }, "value": "x" },
                { "kind": "checked", "target": { "css": "#b" }, "checked": true }
            ] }),
        ))
        .unwrap();
    let checked_first = registry()
        .build_plan(&request(
            "page fill-form --fields $args.fields",
            json!({ "fields": [
                { "kind": "checked", "target": { "css": "#b" }, "checked": true },
                { "kind": "text", "target": { "css": "#a" }, "value": "x" }
            ] }),
        ))
        .unwrap();
    assert_ne!(
        text_first.invocation_fingerprint,
        checked_first.invocation_fingerprint
    );
}

// --- Catalog projection ---

#[test]
fn catalog_includes_declared_types() {
    let catalog = registry().catalog();
    let names: Vec<&str> = catalog.types.iter().map(|decl| decl.name.as_str()).collect();
    assert_eq!(names, vec!["condition", "element-target", "form-field"]);
}

#[test]
fn catalog_hash_covers_type_declarations() {
    let without_extra_variant = registry().catalog_identity().catalog_hash;
    let with_extra_variant = CommandRegistry::new("types-test", "Named type test server")
        .declare_type(element_target_type().variant(
            Variant::new("label", "Locate by label text")
                .field(Field::string("label", "Visible label text")),
        ))
        .declare_type(condition_type())
        .declare_type(form_field_type())
        .register(
            CommandSpec::new(["page", "click"], "Click", "Click an element")
                .with_arg(ArgSpec::named(
                    "target",
                    "element-target",
                    "Element to click",
                ))
                .with_permission(PermissionSpec::new(
                    PermissionEffect::Write,
                    "page",
                    "Clicks an element",
                )),
            |_context| async { Ok(CommandOutput::text("ok")) },
        )
        .catalog_identity()
        .catalog_hash;
    assert_ne!(without_extra_variant, with_extra_variant);
}
