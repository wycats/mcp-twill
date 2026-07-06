//! Named argument types: declared unions of object variants that commands
//! reference by name (RFC 0008). Declarations are validated at registration,
//! matched by the planner, and projected into the catalog, help, and schemas.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{FrameworkError, Result};

/// A named union of object variants, declared once per catalog and
/// referenced by name from `ArgSpec` or `FieldShape::Reference`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeDecl {
    pub name: String,
    pub summary: String,
    pub variants: Vec<Variant>,
}

impl TypeDecl {
    pub fn union(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            variants: Vec::new(),
        }
    }

    pub fn variant(mut self, variant: Variant) -> Self {
        self.variants.push(variant);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
}

impl Variant {
    pub fn new(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            fields: Vec::new(),
        }
    }

    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    pub summary: String,
    pub required: bool,
    pub shape: FieldShape,
}

impl Field {
    fn new(name: impl Into<String>, summary: impl Into<String>, shape: FieldShape) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            required: true,
            shape,
        }
    }

    pub fn string(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self::new(name, summary, FieldShape::String)
    }

    pub fn boolean(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self::new(name, summary, FieldShape::Bool)
    }

    pub fn number(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self::new(name, summary, FieldShape::Number)
    }

    pub fn integer(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self::new(name, summary, FieldShape::Integer)
    }

    pub fn constant(name: impl Into<String>, value: impl Into<String>) -> Self {
        let value = value.into();
        let summary = format!("Always `{value}`");
        Self::new(name, summary, FieldShape::Constant(value))
    }

    pub fn enumerated(
        name: impl Into<String>,
        values: &[&str],
        summary: impl Into<String>,
    ) -> Self {
        Self::new(
            name,
            summary,
            FieldShape::Enumerated(values.iter().map(|value| (*value).to_string()).collect()),
        )
    }

    pub fn reference(
        name: impl Into<String>,
        type_name: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self::new(name, summary, FieldShape::Reference(type_name.into()))
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Wraps this field's shape in an array of that shape.
    pub fn repeated(mut self) -> Self {
        self.shape = FieldShape::Repeated(Box::new(self.shape));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FieldShape {
    String,
    Bool,
    Number,
    /// A JSON number with no fractional part.
    Integer,
    /// Matches exactly this string value.
    Constant(String),
    /// Matches one of these string values.
    Enumerated(Vec<String>),
    /// A named type, matched recursively.
    Reference(String),
    /// An array of the inner shape.
    Repeated(Box<FieldShape>),
}

impl FieldShape {
    /// The type name this shape references, looking through `Repeated`.
    pub(crate) fn referenced_type(&self) -> Option<&str> {
        match self {
            FieldShape::Reference(name) => Some(name),
            FieldShape::Repeated(inner) => inner.referenced_type(),
            _ => None,
        }
    }

    /// Human-readable label for help text.
    pub fn label(&self) -> String {
        match self {
            FieldShape::String => "string".to_string(),
            FieldShape::Bool => "boolean".to_string(),
            FieldShape::Number => "number".to_string(),
            FieldShape::Integer => "integer".to_string(),
            FieldShape::Constant(value) => format!("constant `{value}`"),
            FieldShape::Enumerated(values) => format!(
                "one of {}",
                values
                    .iter()
                    .map(|value| format!("`{value}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            FieldShape::Reference(name) => format!("`{name}`"),
            FieldShape::Repeated(inner) => format!("array of {}", inner.label()),
        }
    }
}

/// Registration-time validation over the full set of declarations plus the
/// argument references that anchor reachability. Rejects duplicate names at
/// every level, empty unions, dangling references, cycles, dead types, and
/// ambiguous variant pairs.
pub(crate) fn validate_types(
    types: &BTreeMap<String, TypeDecl>,
    arg_references: &[(String, String)],
) -> Result<()> {
    // Structural checks per type.
    for decl in types.values() {
        if decl.variants.is_empty() {
            return Err(FrameworkError::Build(format!(
                "type `{}` declares no variants; an empty union is impossible to call",
                decl.name
            )));
        }
        let mut variant_names = BTreeSet::new();
        for variant in &decl.variants {
            if !variant_names.insert(variant.name.as_str()) {
                return Err(FrameworkError::Build(format!(
                    "type `{}` declares duplicate variant `{}`",
                    decl.name, variant.name
                )));
            }
            let mut field_names = BTreeSet::new();
            for field in &variant.fields {
                if !field_names.insert(field.name.as_str()) {
                    return Err(FrameworkError::Build(format!(
                        "type `{}` variant `{}` declares duplicate field `{}`",
                        decl.name, variant.name, field.name
                    )));
                }
            }
        }
    }

    // Every reference resolves.
    for (context, type_name) in arg_references {
        if !types.contains_key(type_name) {
            return Err(FrameworkError::Build(format!(
                "{context} references undeclared type `{type_name}`"
            )));
        }
    }
    for decl in types.values() {
        for variant in &decl.variants {
            for field in &variant.fields {
                if let Some(referenced) = field.shape.referenced_type()
                    && !types.contains_key(referenced)
                {
                    return Err(FrameworkError::Build(format!(
                        "type `{}` variant `{}` field `{}` references undeclared type `{referenced}`",
                        decl.name, variant.name, field.name
                    )));
                }
            }
        }
    }

    // No reference cycles.
    for name in types.keys() {
        let mut stack = vec![name.as_str()];
        check_cycles(name, types, &mut stack)?;
    }

    // No dead types: every declaration is reachable from some argument.
    let mut reachable: BTreeSet<&str> = BTreeSet::new();
    let mut frontier: Vec<&str> = arg_references
        .iter()
        .map(|(_, type_name)| type_name.as_str())
        .collect();
    while let Some(name) = frontier.pop() {
        if !reachable.insert(name) {
            continue;
        }
        if let Some(decl) = types.get(name) {
            for variant in &decl.variants {
                for field in &variant.fields {
                    if let Some(referenced) = field.shape.referenced_type() {
                        frontier.push(referenced);
                    }
                }
            }
        }
    }
    for name in types.keys() {
        if !reachable.contains(name.as_str()) {
            return Err(FrameworkError::Build(format!(
                "type `{name}` is declared but never referenced by any command argument or reachable field"
            )));
        }
    }

    // No ambiguous variant pairs.
    for decl in types.values() {
        for (index, left) in decl.variants.iter().enumerate() {
            for right in &decl.variants[index + 1..] {
                if variants_ambiguous(left, right) {
                    return Err(FrameworkError::Build(format!(
                        "type `{}` variants `{}` and `{}` are ambiguous: a value could match both. \
                         Add a constant field or make their required fields distinct",
                        decl.name, left.name, right.name
                    )));
                }
            }
        }
    }

    Ok(())
}

fn check_cycles<'a>(
    current: &'a str,
    types: &'a BTreeMap<String, TypeDecl>,
    stack: &mut Vec<&'a str>,
) -> Result<()> {
    let Some(decl) = types.get(current) else {
        return Ok(());
    };
    for variant in &decl.variants {
        for field in &variant.fields {
            let Some(referenced) = field.shape.referenced_type() else {
                continue;
            };
            if stack.contains(&referenced) {
                return Err(FrameworkError::Build(format!(
                    "type reference cycle: {} -> `{referenced}`",
                    stack
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(" -> ")
                )));
            }
            stack.push(referenced);
            check_cycles(referenced, types, stack)?;
            stack.pop();
        }
    }
    Ok(())
}

/// Two variants are ambiguous when no shared field has contradictory
/// constants and each variant's required fields are all accepted (required
/// or optional) by the other. Scalar-shape differences on shared fields are
/// conservatively ignored.
fn variants_ambiguous(left: &Variant, right: &Variant) -> bool {
    for left_field in &left.fields {
        if let Some(right_field) = right
            .fields
            .iter()
            .find(|field| field.name == left_field.name)
            && let (FieldShape::Constant(left_value), FieldShape::Constant(right_value)) =
                (&left_field.shape, &right_field.shape)
            && left_value != right_value
        {
            return false;
        }
    }
    required_fields_accepted(left, right) && required_fields_accepted(right, left)
}

fn required_fields_accepted(from: &Variant, by: &Variant) -> bool {
    from.fields
        .iter()
        .filter(|field| field.required)
        .all(|field| by.fields.iter().any(|other| other.name == field.name))
}

/// Matches `value` against the named union, returning the matched variant's
/// name. `arg_label` names the argument (with an element index for repeated
/// arguments); `path` prefixes nested failure paths.
pub(crate) fn match_named_value(
    arg_label: &str,
    path: &str,
    type_name: &str,
    value: &Value,
    types: &BTreeMap<String, TypeDecl>,
) -> Result<String> {
    let decl = types.get(type_name).ok_or_else(|| {
        FrameworkError::Build(format!("type `{type_name}` is not declared"))
    })?;

    let mut problems = Vec::new();
    for variant in &decl.variants {
        match variant_problem(variant, value, path, types) {
            None => return Ok(variant.name.clone()),
            Some(problem) => problems.push((variant.name.clone(), problem)),
        }
    }

    Err(FrameworkError::ArgumentUnionMismatch {
        argument: arg_label.to_string(),
        type_name: type_name.to_string(),
        problems,
    })
}

/// The first blocking problem preventing `value` from matching `variant`,
/// or `None` when it matches.
fn variant_problem(
    variant: &Variant,
    value: &Value,
    path: &str,
    types: &BTreeMap<String, TypeDecl>,
) -> Option<String> {
    let Value::Object(map) = value else {
        return Some("value is not an object".to_string());
    };

    for field in &variant.fields {
        match map.get(&field.name) {
            None => {
                if field.required {
                    return Some(format!("missing required field `{}`", field.name));
                }
            }
            Some(field_value) => {
                let field_path = format!("{path}.{}", field.name);
                if let Some(problem) =
                    shape_problem(&field.name, &field.shape, field_value, &field_path, types)
                {
                    return Some(problem);
                }
            }
        }
    }

    for key in map.keys() {
        if !variant.fields.iter().any(|field| &field.name == key) {
            return Some(format!("unknown field `{key}`"));
        }
    }

    None
}

fn shape_problem(
    field_name: &str,
    shape: &FieldShape,
    value: &Value,
    path: &str,
    types: &BTreeMap<String, TypeDecl>,
) -> Option<String> {
    match shape {
        FieldShape::String => (!value.is_string())
            .then(|| format!("field `{field_name}` must be a string")),
        FieldShape::Bool => (!value.is_boolean())
            .then(|| format!("field `{field_name}` must be a boolean")),
        FieldShape::Number => (!value.is_number())
            .then(|| format!("field `{field_name}` must be a number")),
        FieldShape::Integer => {
            let integral = value.as_i64().is_some()
                || value.as_u64().is_some()
                || value.as_f64().is_some_and(|number| number.fract() == 0.0);
            (!integral).then(|| format!("field `{field_name}` must be an integer"))
        }
        FieldShape::Constant(expected) => (value.as_str() != Some(expected.as_str()))
            .then(|| format!("field `{field_name}` must be the constant `{expected}`")),
        FieldShape::Enumerated(values) => {
            let matched = value
                .as_str()
                .is_some_and(|actual| values.iter().any(|candidate| candidate == actual));
            (!matched).then(|| {
                format!(
                    "field `{field_name}` must be one of {}",
                    values
                        .iter()
                        .map(|value| format!("`{value}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
        }
        FieldShape::Repeated(inner) => {
            let Value::Array(items) = value else {
                return Some(format!("field `{field_name}` must be an array"));
            };
            for (index, item) in items.iter().enumerate() {
                let item_path = format!("{path}[{index}]");
                if let Some(problem) =
                    shape_problem(field_name, inner, item, &item_path, types)
                {
                    return Some(problem);
                }
            }
            None
        }
        FieldShape::Reference(type_name) => {
            match match_named_value(field_name, path, type_name, value, types) {
                Ok(_) => None,
                Err(FrameworkError::ArgumentUnionMismatch { problems, .. }) => Some(format!(
                    "`{path}` does not match `{type_name}`: {}",
                    problems
                        .iter()
                        .map(|(variant, problem)| format!("not `{variant}`: {problem}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
                Err(error) => Some(error.to_string()),
            }
        }
    }
}

/// The inlined JSON schema for a named type: a `oneOf` of closed object
/// variant schemas. References are fully dereferenced; termination is
/// guaranteed because registration rejects cycles.
pub(crate) fn inline_type_schema(
    type_name: &str,
    types: &BTreeMap<String, TypeDecl>,
) -> Value {
    let Some(decl) = types.get(type_name) else {
        return Value::Null;
    };
    let variants: Vec<Value> = decl
        .variants
        .iter()
        .map(|variant| {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();
            for field in &variant.fields {
                let mut schema = field_shape_schema(&field.shape, types);
                if let Value::Object(object) = &mut schema
                    && !field.summary.is_empty()
                {
                    object.insert(
                        "description".to_string(),
                        Value::String(field.summary.clone()),
                    );
                }
                properties.insert(field.name.clone(), schema);
                if field.required {
                    required.push(Value::String(field.name.clone()));
                }
            }
            serde_json::json!({
                "type": "object",
                "description": variant.summary,
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            })
        })
        .collect();
    serde_json::json!({
        "description": decl.summary,
        "oneOf": variants,
    })
}

fn field_shape_schema(shape: &FieldShape, types: &BTreeMap<String, TypeDecl>) -> Value {
    match shape {
        FieldShape::String => serde_json::json!({ "type": "string" }),
        FieldShape::Bool => serde_json::json!({ "type": "boolean" }),
        FieldShape::Number => serde_json::json!({ "type": "number" }),
        FieldShape::Integer => serde_json::json!({ "type": "integer" }),
        FieldShape::Constant(value) => serde_json::json!({ "const": value }),
        FieldShape::Enumerated(values) => serde_json::json!({
            "type": "string",
            "enum": values,
        }),
        FieldShape::Reference(type_name) => inline_type_schema(type_name, types),
        FieldShape::Repeated(inner) => serde_json::json!({
            "type": "array",
            "items": field_shape_schema(inner, types),
        }),
    }
}
