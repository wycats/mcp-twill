use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    future::Future,
    marker::PhantomData,
    sync::Arc,
};

use async_trait::async_trait;
use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::{Number, Value};

use crate::{
    ArgSpec, ArgType, CommandContext, CommandHandler, CommandOutput, FrameworkError,
    resource::{ContextAndArgs, ResourceOutput, ResourceParams, ResourceUse, WithResourcesAndArgs},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ArgumentSchemaUse {
    Named { name: String },
    Inline { schema: Value },
}

impl ArgumentSchemaUse {
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named { name: name.into() }
    }

    pub fn inline(schema: impl Into<Value>) -> Self {
        Self::Inline {
            schema: schema.into(),
        }
    }
}

impl From<Value> for ArgumentSchemaUse {
    fn from(schema: Value) -> Self {
        Self::inline(schema)
    }
}

impl From<Schema> for ArgumentSchemaUse {
    fn from(schema: Schema) -> Self {
        Self::inline(Value::from(schema))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArgumentSchemaDecl {
    pub name: String,
    pub summary: String,
    pub schema: Value,
}

impl ArgumentSchemaDecl {
    pub fn new(
        name: impl Into<String>,
        summary: impl Into<String>,
        schema: impl Into<Value>,
    ) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            schema: schema.into(),
        }
    }
}

/// An exact mathematical integer carried by `serde_json::Number`.
///
/// Construction is checked; the representation cannot be bypassed by an
/// external crate.
///
/// ```compile_fail
/// use mcp_twill::JsonInteger;
///
/// let _unchecked = JsonInteger(serde_json::Number::from(1));
/// ```
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct JsonInteger(Number);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonIntegerError {
    NotInteger,
}

impl fmt::Display for JsonIntegerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON number is not an integer")
    }
}

impl Error for JsonIntegerError {}

impl JsonInteger {
    pub fn try_from_number(number: Number) -> Result<Self, JsonIntegerError> {
        if Self::number_is_integer(&number) {
            Ok(Self(number))
        } else {
            Err(JsonIntegerError::NotInteger)
        }
    }

    pub(crate) fn number_is_integer(number: &Number) -> bool {
        number.is_i64()
            || number.is_u64()
            || number
                .as_f64()
                .is_some_and(|value| value.is_finite() && value.fract() == 0.0)
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.0.as_i64().or_else(|| {
            let value = self.0.as_f64()?;
            (value >= i64::MIN as f64 && value < 9_223_372_036_854_775_808.0)
                .then_some(value as i64)
                .filter(|converted| *converted as f64 == value)
        })
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.0.as_u64().or_else(|| {
            let value = self.0.as_f64()?;
            (0.0..18_446_744_073_709_551_616.0)
                .contains(&value)
                .then_some(value as u64)
                .filter(|converted| *converted as f64 == value)
        })
    }

    pub fn into_number(self) -> Number {
        self.0
    }
}

impl TryFrom<Number> for JsonInteger {
    type Error = JsonIntegerError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        Self::try_from_number(number)
    }
}

impl<'de> Deserialize<'de> for JsonInteger {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = Number::deserialize(deserializer)?;
        Self::try_from_number(number).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for JsonInteger {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "JsonInteger".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({ "type": "integer" })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArgSchemaMatch {
    pub selections: Vec<SchemaBranchSelection>,
}

impl ArgSchemaMatch {
    pub(crate) fn is_empty(&self) -> bool {
        self.selections.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchemaBranchSelection {
    pub schema: String,
    pub instance_pointer: String,
    pub one_of_pointer: String,
    pub branch_pointer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchemaBranchProblem {
    pub pointer: String,
    pub path: String,
    pub keyword: ArgumentSchemaKeyword,
    pub expected: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

impl fmt::Display for ArgumentSchemaKeyword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Type => "type",
            Self::Const => "const",
            Self::Enum => "enum",
            Self::Minimum => "minimum",
            Self::Maximum => "maximum",
            Self::MultipleOf => "multiple_of",
            Self::MinLength => "min_length",
            Self::Items => "items",
            Self::MinItems => "min_items",
            Self::Required => "required",
            Self::DependentRequired => "dependent_required",
            Self::AdditionalProperties => "additional_properties",
            Self::OneOf => "one_of",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentContractReason {
    DerivedSchemaDrift,
    TypedDeserializationFailed,
}

#[doc(hidden)]
pub struct ConstrainedHandlerRegistration {
    pub(crate) handler: Arc<dyn CommandHandler>,
    pub(crate) argument_schema: Value,
    pub(crate) resource_uses: Vec<ResourceUse>,
    pub(crate) granted: Vec<&'static str>,
    pub(crate) enumerated: Vec<&'static str>,
}

mod private {
    pub trait Sealed<M> {}
}

/// A legacy-output handler whose typed argument object is checked against
/// the command's canonical argument schema before the registry is published.
///
/// Implementations are supplied by Twill's public RFC 0012 marker-keyed
/// blanket implementations; external crates cannot add an overlapping
/// dialect by implementing the sealed trait themselves.
///
/// ```compile_fail
/// use mcp_twill::{ConstrainedCommandDialect, ConstrainedHandlerRegistration};
///
/// struct ForeignMarker;
/// struct ForeignHandler;
///
/// impl ConstrainedCommandDialect<ForeignMarker> for ForeignHandler {
///     fn into_constrained_registration(self) -> ConstrainedHandlerRegistration {
///         unimplemented!()
///     }
/// }
/// ```
pub trait ConstrainedCommandDialect<M>: private::Sealed<M> + Send + Sync + 'static {
    #[doc(hidden)]
    fn into_constrained_registration(self) -> ConstrainedHandlerRegistration;
}

struct ConstrainedHandler<M, H> {
    handler: H,
    _marker: PhantomData<fn() -> M>,
}

impl<H, A, Fut, O> private::Sealed<ContextAndArgs<A, O>> for H
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = crate::Result<O>> + Send,
    O: ResourceOutput,
{
}

impl<H, A, Fut, O> ConstrainedCommandDialect<ContextAndArgs<A, O>> for H
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = crate::Result<O>> + Send,
    O: ResourceOutput,
{
    fn into_constrained_registration(self) -> ConstrainedHandlerRegistration {
        ConstrainedHandlerRegistration {
            handler: Arc::new(ConstrainedHandler::<ContextAndArgs<A, O>, H> {
                handler: self,
                _marker: PhantomData,
            }),
            argument_schema: derived_argument_schema::<A>(),
            resource_uses: Vec::new(),
            granted: O::granted(),
            enumerated: O::enumerated(),
        }
    }
}

#[async_trait]
impl<H, A, Fut, O> CommandHandler for ConstrainedHandler<ContextAndArgs<A, O>, H>
where
    H: Fn(CommandContext, A) -> Fut + Send + Sync + 'static,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = crate::Result<O>> + Send,
    O: ResourceOutput,
{
    async fn call(&self, context: CommandContext) -> crate::Result<CommandOutput> {
        let args = extract_checked::<A>(&context)?;
        let output = (self.handler)(context, args)
            .await
            .map_err(reject_handler_argument_contract)?;
        Ok(output.into_command_output())
    }
}

impl<H, P, A, Fut, O> private::Sealed<WithResourcesAndArgs<P, A, O>> for H
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = crate::Result<O>> + Send,
    O: ResourceOutput,
{
}

impl<H, P, A, Fut, O> ConstrainedCommandDialect<WithResourcesAndArgs<P, A, O>> for H
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = crate::Result<O>> + Send,
    O: ResourceOutput,
{
    fn into_constrained_registration(self) -> ConstrainedHandlerRegistration {
        ConstrainedHandlerRegistration {
            handler: Arc::new(ConstrainedHandler::<WithResourcesAndArgs<P, A, O>, H> {
                handler: self,
                _marker: PhantomData,
            }),
            argument_schema: derived_argument_schema::<A>(),
            resource_uses: P::resource_uses(),
            granted: O::granted(),
            enumerated: O::enumerated(),
        }
    }
}

#[async_trait]
impl<H, P, A, Fut, O> CommandHandler for ConstrainedHandler<WithResourcesAndArgs<P, A, O>, H>
where
    H: Fn(P, CommandContext, A) -> Fut + Send + Sync + 'static,
    P: ResourceParams,
    A: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    Fut: Future<Output = crate::Result<O>> + Send,
    O: ResourceOutput,
{
    async fn call(&self, context: CommandContext) -> crate::Result<CommandOutput> {
        let resources = P::extract(&context)?;
        let args = extract_checked::<A>(&context)?;
        let output = (self.handler)(resources, context, args)
            .await
            .map_err(reject_handler_argument_contract)?;
        Ok(output.into_command_output())
    }
}

fn reject_handler_argument_contract(error: FrameworkError) -> FrameworkError {
    if matches!(error, FrameworkError::ArgumentContractViolation { .. }) {
        FrameworkError::Handler("handler returned invalid argument contract violation".to_string())
    } else {
        error
    }
}

pub(crate) fn derived_argument_schema<T: JsonSchema>() -> Value {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let schema = generator.into_root_schema_for::<T>();
    serde_json::to_value(schema).expect("Schemars argument schema serializes")
}

pub(crate) fn extract_checked<T: DeserializeOwned>(context: &CommandContext) -> crate::Result<T> {
    let values = context
        .plan
        .bound_args
        .iter()
        .filter(|(_, arg)| !matches!(arg.value_type, ArgType::ResourceRef(_)))
        .map(|(name, arg)| (name.clone(), arg.value.clone()))
        .collect::<serde_json::Map<_, _>>();
    serde_json::from_value(Value::Object(values)).map_err(|_| {
        FrameworkError::ArgumentContractViolation {
            operation_id: context.plan.operation_id.clone(),
            argument: None,
            reason: ArgumentContractReason::TypedDeserializationFailed,
        }
    })
}

pub(crate) fn validate_derived_argument_schema(
    operation_id: &str,
    mut authored: Value,
    derived: &Value,
    excluded_properties: &BTreeSet<String>,
) -> crate::Result<()> {
    let mut derived = derived.clone();
    if derived.as_object().is_some_and(|root| {
        root.contains_key("dependentRequired") || root.contains_key("dependencies")
    }) {
        return Err(derived_schema_drift(operation_id));
    }

    if let Some(root) = authored.as_object_mut() {
        root.remove("dependentRequired");
        if let Some(properties) = root.get_mut("properties").and_then(Value::as_object_mut) {
            for carrier in excluded_properties {
                properties.remove(carrier);
            }
        }
        if let Some(required) = root.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| {
                name.as_str()
                    .is_none_or(|name| !excluded_properties.contains(name))
            });
        }
    }
    prune_unreachable_definitions(&mut authored);
    if let Some(root) = derived.as_object_mut() {
        root.remove("$schema");
    }
    let derived_root = derived.clone();
    normalize_derived_nullable(&mut derived, &derived_root);
    strip_annotations(&mut authored);
    strip_annotations(&mut derived);
    normalize_implied_const_types(&mut authored);
    normalize_implied_const_types(&mut derived);
    normalize_comparison_sets(&mut authored);
    normalize_comparison_sets(&mut derived);
    elide_derived_optional_nulls(&authored, &mut derived);
    normalize_comparison_sets(&mut derived);

    if authored == derived {
        Ok(())
    } else {
        Err(derived_schema_drift(operation_id))
    }
}

fn derived_schema_drift(operation_id: &str) -> FrameworkError {
    FrameworkError::ArgumentContractViolation {
        operation_id: operation_id.to_string(),
        argument: None,
        reason: ArgumentContractReason::DerivedSchemaDrift,
    }
}

fn strip_annotations(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.remove("title");
    object.remove("description");
    for_each_child_schema_mut(object, strip_annotations);
}

fn normalize_comparison_sets(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
        required.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    }
    if object
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        object.remove("required");
    }
    if let Some(kinds) = object.get_mut("type").and_then(Value::as_array_mut)
        && kinds.len() == 2
        && kinds.iter().any(|kind| kind == "null")
    {
        kinds.sort_by_key(|kind| kind == "null");
    }
    for_each_child_schema_mut(object, normalize_comparison_sets);
}

fn normalize_implied_const_types(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    // Schemars emits the primitive `type` alongside a `const`, while the
    // accepted guide writes the same singleton domain using only `const`.
    // The type assertion is wholly implied by that literal, so remove it only
    // from typed-schema comparison; canonical authored projection retains
    // every declared byte.
    if object.contains_key("const") {
        object.remove("type");
    }
    for_each_child_schema_mut(object, normalize_implied_const_types);
}

fn normalize_derived_nullable(value: &mut Value, root: &Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(branches) = object.remove("anyOf") {
        let branches = branches.as_array().cloned().unwrap_or_default();
        let null_index = branches.iter().position(is_null_schema);
        if branches.len() == 2
            && let Some(null_index) = null_index
        {
            let other = branches[1 - null_index].clone();
            if schema_rejects_null(&other, root) {
                object.insert(
                    "oneOf".to_string(),
                    Value::Array(vec![other, serde_json::json!({ "type": "null" })]),
                );
            } else {
                object.insert("anyOf".to_string(), Value::Array(branches));
            }
        } else {
            object.insert("anyOf".to_string(), Value::Array(branches));
        }
    }
    for_each_child_schema_mut(object, |nested| {
        normalize_derived_nullable(nested, root);
    });
}

fn for_each_child_schema_mut(
    object: &mut serde_json::Map<String, Value>,
    mut visit: impl FnMut(&mut Value),
) {
    if let Some(items) = object.get_mut("items") {
        visit(items);
    }
    if let Some(additional) = object.get_mut("additionalProperties")
        && additional.is_object()
    {
        visit(additional);
    }
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        for property in properties.values_mut() {
            visit(property);
        }
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = object.get_mut(keyword).and_then(Value::as_array_mut) {
            for branch in branches {
                visit(branch);
            }
        }
    }
    if let Some(definitions) = object.get_mut("$defs").and_then(Value::as_object_mut) {
        for definition in definitions.values_mut() {
            visit(definition);
        }
    }
}

fn elide_derived_optional_nulls(authored: &Value, derived: &mut Value) {
    let Some(authored_properties) = authored.get("properties").and_then(Value::as_object) else {
        return;
    };
    let Some(derived_properties) = derived.get_mut("properties").and_then(Value::as_object_mut)
    else {
        return;
    };
    for (name, authored_property) in authored_properties {
        if !schema_rejects_null(authored_property, authored) {
            continue;
        }
        let Some(derived_property) = derived_properties.get_mut(name) else {
            continue;
        };
        if let Some(kinds) = derived_property.get("type").and_then(Value::as_array)
            && kinds.len() == 2
            && kinds.iter().any(|kind| kind == "null")
        {
            let mut non_null = derived_property.clone();
            let non_null_kind = kinds
                .iter()
                .find(|kind| *kind != "null")
                .expect("two-member nullable type has a non-null member")
                .clone();
            non_null
                .as_object_mut()
                .expect("derived property is an object")
                .insert("type".to_string(), non_null_kind);
            let mut expected = authored_property.clone();
            normalize_comparison_sets(&mut non_null);
            normalize_comparison_sets(&mut expected);
            if non_null == expected {
                *derived_property = non_null;
                continue;
            }
        }
        let Some(branches) = derived_property.get("oneOf").and_then(Value::as_array) else {
            continue;
        };
        if branches.len() != 2 {
            continue;
        }
        let Some(null_index) = branches.iter().position(is_null_schema) else {
            continue;
        };
        let mut non_null = branches[1 - null_index].clone();
        let mut expected = authored_property.clone();
        normalize_comparison_sets(&mut non_null);
        normalize_comparison_sets(&mut expected);
        if non_null == expected {
            *derived_property = non_null;
        }
    }
}

fn is_null_schema(schema: &Value) -> bool {
    schema
        .get("type")
        .is_some_and(|kind| kind == &Value::String("null".to_string()))
}

fn schema_rejects_null(schema: &Value, root: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str)
        && resolve_ref(root, reference).is_some_and(|target| schema_rejects_null(target, root))
    {
        return true;
    }
    if let Some(kind) = object.get("type") {
        return !type_contains(kind, "null");
    }
    if let Some(constant) = object.get("const") {
        return !constant.is_null();
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return !values.iter().any(Value::is_null);
    }
    object
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .all(|branch| schema_rejects_null(branch, root))
        })
}

pub(crate) const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

pub(crate) type SchemaDecls = BTreeMap<String, ArgumentSchemaDecl>;

#[derive(Debug, Clone)]
pub(crate) struct CompiledArgumentSchema {
    pub identity: String,
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaFailure {
    pub path: String,
    pub pointer: String,
    pub keyword: ArgumentSchemaKeyword,
    pub expected: String,
    pub branches: Vec<SchemaBranchProblem>,
}

impl SchemaFailure {
    pub(crate) fn branch_problem(&self) -> SchemaBranchProblem {
        SchemaBranchProblem {
            pointer: self.pointer.clone(),
            path: self.path.clone(),
            keyword: self.keyword,
            expected: self.expected.clone(),
        }
    }
}

pub(crate) fn compile_argument_schema(
    arg: &ArgSpec,
    declarations: &SchemaDecls,
) -> crate::Result<Option<CompiledArgumentSchema>> {
    if matches!(arg.value_type, ArgType::Named(_)) && arg.schema.is_some() {
        return Err(build_error(format!(
            "command argument `{}` uses an RFC 0008 named type and cannot declare an argument schema override",
            arg.name
        )));
    }
    let (identity, mut schema) = match &arg.schema {
        None if matches!(arg.value_type, ArgType::Integer) => (
            format!("@integer:{}", arg.name),
            serde_json::json!({
                "type": "integer",
                "description": arg.summary,
            }),
        ),
        None => return Ok(None),
        Some(schema_use) => match schema_use {
            ArgumentSchemaUse::Named { name } => {
                let declaration = declarations.get(name).ok_or_else(|| {
                    build_error(format!(
                        "command argument `{}` references undeclared argument schema `{name}`",
                        arg.name
                    ))
                })?;
                (name.clone(), declaration.schema.clone())
            }
            ArgumentSchemaUse::Inline { schema } => {
                (format!("@inline:{}", arg.name), schema.clone())
            }
        },
    };
    canonicalize_schema(&mut schema)?;
    if let Some(description) = schema.get("description")
        && description.as_str() != Some(arg.summary.as_str())
    {
        return Err(build_error(format!(
            "command argument `{}` schema description must equal its summary",
            arg.name
        )));
    }
    ensure_coarse_compatibility(arg, &schema)?;
    if arg.repeated {
        if schema_implies_type(&schema, &schema, "array") {
            return Err(build_error(format!(
                "command argument `{}` combines `repeated` with an array-root schema",
                arg.name
            )));
        }
        let description = schema
            .as_object_mut()
            .and_then(|object| object.remove("description"));
        let definitions = schema
            .as_object_mut()
            .and_then(|object| object.remove("$defs"));
        schema = serde_json::json!({
            "type": "array",
            "items": schema,
        });
        if let Some(description) = description {
            schema
                .as_object_mut()
                .expect("array schema is an object")
                .insert("description".to_string(), description);
        }
        if let Some(definitions) = definitions {
            schema
                .as_object_mut()
                .expect("array schema is an object")
                .insert("$defs".to_string(), definitions);
        }
    }
    Ok(Some(CompiledArgumentSchema { identity, schema }))
}

pub(crate) fn canonicalize_schema(schema: &mut Value) -> crate::Result<()> {
    let root = schema.as_object_mut().ok_or_else(|| {
        build_error("argument schema root must be an object; boolean schemas are unsupported")
    })?;
    if let Some(marker) = root.remove("$schema")
        && marker.as_str() != Some(DIALECT)
    {
        return Err(build_error(
            "argument schema `$schema` must name JSON Schema draft 2020-12",
        ));
    }
    canonicalize_nullable_types(schema);
    validate_schema_node(schema, schema, true)?;
    // Every numeric token in the supported schema document is public
    // canonical data, not only values nested under `const` or `enum`.
    // Keep bounds, divisors, and size assertions inside the same exact
    // RFC 8785/I-JSON domain before hashing or projection.
    validate_schema_literal(schema)?;
    canonicalize_schema_numbers(schema);
    validate_local_definitions(schema)?;
    validate_static_domain(schema, schema)?;
    normalize_comparison_sets(schema);
    Ok(())
}

fn validate_schema_node(schema: &Value, root_schema: &Value, root: bool) -> crate::Result<()> {
    let object = schema.as_object().ok_or_else(|| {
        build_error("argument schemas must use object schemas; boolean schemas are unsupported")
    })?;
    const ALLOWED: &[&str] = &[
        "$schema",
        "$defs",
        "$ref",
        "title",
        "description",
        "type",
        "const",
        "enum",
        "minimum",
        "maximum",
        "multipleOf",
        "minLength",
        "items",
        "minItems",
        "properties",
        "required",
        "additionalProperties",
        "oneOf",
    ];
    for key in object.keys() {
        if !ALLOWED.contains(&key.as_str()) {
            return Err(build_error(format!(
                "unsupported argument schema keyword `{key}`"
            )));
        }
    }
    if !root && object.contains_key("$defs") {
        return Err(build_error(
            "argument schema `$defs` is supported only at the schema root",
        ));
    }
    if object.contains_key("$schema") {
        return Err(build_error(
            "argument schema `$schema` is supported only at the schema root",
        ));
    }
    for annotation in ["title", "description"] {
        if object
            .get(annotation)
            .is_some_and(|value| !value.is_string())
        {
            return Err(build_error(format!(
                "argument schema `{annotation}` must be a string"
            )));
        }
    }
    if let Some(reference) = object.get("$ref") {
        let reference = reference
            .as_str()
            .ok_or_else(|| build_error("argument schema `$ref` must be a string"))?;
        if !reference.starts_with("#/$defs/") || reference[8..].contains('/') {
            return Err(build_error(format!(
                "argument schema reference `{reference}` is not a local definition reference"
            )));
        }
    }
    if let Some(kind) = object.get("type") {
        validate_schema_type(kind)?;
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .ok_or_else(|| build_error("argument schema `enum` must be an array"))?;
        if values.is_empty() {
            return Err(build_error("argument schema `enum` cannot be empty"));
        }
        let mut seen = Vec::new();
        let mut kind = None;
        for value in values {
            validate_schema_literal(value)?;
            let current = literal_kind(value);
            if kind.get_or_insert(current) != &current {
                return Err(build_error("argument schema `enum` must be homogeneous"));
            }
            if seen
                .iter()
                .any(|existing| schema_values_equal(existing, value))
            {
                return Err(build_error("argument schema `enum` entries must be unique"));
            }
            seen.push(value.clone());
        }
    }
    if let Some(value) = object.get("const") {
        validate_schema_literal(value)?;
    }
    for keyword in ["minimum", "maximum", "multipleOf"] {
        if let Some(value) = object.get(keyword) {
            if !value
                .as_number()
                .is_some_and(JsonInteger::number_is_integer)
            {
                return Err(build_error(format!(
                    "argument schema `{keyword}` must be an exact integer"
                )));
            }
            if keyword == "multipleOf" && !number_is_positive(value) {
                return Err(build_error(
                    "argument schema `multipleOf` must be a positive integer",
                ));
            }
            if !object
                .get("type")
                .is_some_and(|kind| type_contains(kind, "integer"))
                && !object.contains_key("$ref")
            {
                return Err(build_error(format!(
                    "argument schema `{keyword}` is supported only for integer schemas"
                )));
            }
        }
    }
    for keyword in ["minLength", "minItems"] {
        if let Some(value) = object.get(keyword) {
            let valid = integer_as_i128(value)
                .is_some_and(|value| value >= 0 && value <= i128::from(u64::MAX));
            if !valid {
                return Err(build_error(format!(
                    "argument schema `{keyword}` must be a non-negative integer"
                )));
            }
        }
    }
    if object.contains_key("minLength")
        && !object
            .get("type")
            .is_some_and(|kind| type_contains(kind, "string"))
        && !object.contains_key("$ref")
    {
        return Err(build_error(
            "argument schema `minLength` is supported only for string schemas",
        ));
    }
    if (object.contains_key("items") || object.contains_key("minItems"))
        && !object
            .get("type")
            .is_some_and(|kind| type_contains(kind, "array"))
        && !object.contains_key("$ref")
    {
        return Err(build_error(
            "argument schema array keywords require an array schema",
        ));
    }
    if ["properties", "required", "additionalProperties"]
        .iter()
        .any(|keyword| object.contains_key(*keyword))
        && !object
            .get("type")
            .is_some_and(|kind| type_contains(kind, "object"))
        && !object.contains_key("$ref")
    {
        return Err(build_error(
            "argument schema object keywords require an object schema",
        ));
    }
    if let Some(items) = object.get("items") {
        validate_schema_node(items, root_schema, false)?;
    }
    if let Some(properties) = object.get("properties") {
        for nested in properties
            .as_object()
            .ok_or_else(|| build_error("argument schema `properties` must be an object"))?
            .values()
        {
            validate_schema_node(nested, root_schema, false)?;
        }
    }
    if let Some(required) = object.get("required") {
        let required = required
            .as_array()
            .ok_or_else(|| build_error("argument schema `required` must be an array"))?;
        let mut seen = BTreeSet::new();
        for name in required {
            let name = name
                .as_str()
                .ok_or_else(|| build_error("argument schema `required` entries must be strings"))?;
            if !seen.insert(name) {
                return Err(build_error(
                    "argument schema `required` entries must be unique",
                ));
            }
        }
    }
    if let Some(additional) = object.get("additionalProperties")
        && !additional.is_boolean()
    {
        validate_schema_node(additional, root_schema, false)?;
    }
    if object
        .get("type")
        .is_some_and(|kind| type_contains(kind, "object"))
        && !object.contains_key("additionalProperties")
    {
        return Err(build_error(
            "argument schema object must declare `additionalProperties`",
        ));
    }
    if let Some(branches) = object.get("oneOf") {
        let branches = branches
            .as_array()
            .ok_or_else(|| build_error("argument schema `oneOf` must be an array"))?;
        if branches.is_empty() {
            return Err(build_error("argument schema `oneOf` cannot be empty"));
        }
        for branch in branches {
            validate_schema_node(branch, root_schema, false)?;
        }
        for left in 0..branches.len() {
            for right in left + 1..branches.len() {
                if !schemas_provably_disjoint(&branches[left], &branches[right], root_schema) {
                    return Err(build_error(format!(
                        "argument schema `oneOf` branches {left} and {right} are not provably disjoint"
                    )));
                }
            }
        }
    }
    if let Some(definitions) = object.get("$defs") {
        for nested in definitions
            .as_object()
            .ok_or_else(|| build_error("argument schema `$defs` must be an object"))?
            .values()
        {
            validate_schema_node(nested, root_schema, false)?;
        }
    }
    Ok(())
}

fn validate_schema_type(value: &Value) -> crate::Result<()> {
    const TYPES: &[&str] = &[
        "null", "boolean", "object", "array", "number", "integer", "string",
    ];
    match value {
        Value::String(kind) if TYPES.contains(&kind.as_str()) => Ok(()),
        Value::Array(kinds) => {
            if kinds.len() != 2 {
                return Err(build_error(
                    "argument schema type array must contain one primitive and null",
                ));
            }
            let kinds = kinds
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| build_error("argument schema type array must contain strings"))?;
            if kinds[1] != "null" || kinds[0] == "null" || !TYPES.contains(&kinds[0]) {
                return Err(build_error(
                    "argument schema type array must contain one primitive and null",
                ));
            }
            Ok(())
        }
        _ => Err(build_error(
            "argument schema has an unsupported `type` value",
        )),
    }
}

fn canonicalize_nullable_types(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(kinds)) = object.get_mut("type")
        && kinds.len() == 2
        && kinds.iter().any(|kind| kind == "null")
    {
        kinds.sort_by_key(|kind| kind == "null");
    }
    for_each_child_schema_mut(object, canonicalize_nullable_types);
}

fn validate_schema_literal(value: &Value) -> crate::Result<()> {
    match value {
        Value::Number(number) => {
            const MAX_EXACT_INTEGER: i128 = 9_007_199_254_740_991;
            let exact = number
                .as_i64()
                .map(|value| i128::from(value).abs() <= MAX_EXACT_INTEGER)
                .or_else(|| {
                    number
                        .as_u64()
                        .map(|value| i128::from(value) <= MAX_EXACT_INTEGER)
                })
                .unwrap_or_else(|| {
                    number.as_f64().is_some_and(|value| {
                        value.is_finite()
                            && (value.fract() != 0.0 || value.abs() <= MAX_EXACT_INTEGER as f64)
                    })
                });
            if !exact {
                return Err(build_error(
                    "argument schema numeric literal is outside the exact I-JSON domain",
                ));
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_schema_literal(value)?;
            }
        }
        Value::Object(object) => {
            for value in object.values() {
                validate_schema_literal(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn canonicalize_schema_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if !number.is_i64() && !number.is_u64() => {
            let Some(float) = number.as_f64() else {
                return;
            };
            if !float.is_finite() || float.fract() != 0.0 {
                return;
            }
            if float < 0.0 {
                if float >= i64::MIN as f64 {
                    *number = Number::from(float as i64);
                }
            } else if float <= u64::MAX as f64 {
                *number = Number::from(float as u64);
            }
        }
        Value::Array(values) => {
            for value in values {
                canonicalize_schema_numbers(value);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                canonicalize_schema_numbers(value);
            }
        }
        _ => {}
    }
}

fn validate_local_definitions(schema: &Value) -> crate::Result<()> {
    let definitions = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut reachable = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    visit_schema_refs(schema, &definitions, &mut reachable, &mut visiting)?;
    for name in definitions.keys() {
        if !reachable.contains(name) {
            return Err(build_error(format!(
                "argument schema definition `{name}` is unreachable"
            )));
        }
    }
    Ok(())
}

fn prune_unreachable_definitions(schema: &mut Value) {
    let definitions = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if definitions.is_empty() {
        return;
    }
    let mut reachable = BTreeSet::new();
    visit_schema_refs(schema, &definitions, &mut reachable, &mut BTreeSet::new())
        .expect("canonical schema references remain valid after removing properties");
    let Some(root) = schema.as_object_mut() else {
        return;
    };
    let Some(definitions) = root.get_mut("$defs").and_then(Value::as_object_mut) else {
        return;
    };
    definitions.retain(|name, _| reachable.contains(name));
    if definitions.is_empty() {
        root.remove("$defs");
    }
}

fn visit_schema_refs(
    schema: &Value,
    definitions: &serde_json::Map<String, Value>,
    reachable: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
) -> crate::Result<()> {
    let Some(object) = schema.as_object() else {
        return Ok(());
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let name = reference.trim_start_matches("#/$defs/");
        let target = definitions.get(name).ok_or_else(|| {
            build_error(format!(
                "argument schema reference `{reference}` is dangling"
            ))
        })?;
        if !visiting.insert(name.to_string()) {
            return Err(build_error(format!(
                "argument schema reference cycle through `{name}`"
            )));
        }
        reachable.insert(name.to_string());
        visit_schema_refs(target, definitions, reachable, visiting)?;
        visiting.remove(name);
    }
    for nested in child_schemas(object) {
        visit_schema_refs(nested, definitions, reachable, visiting)?;
    }
    Ok(())
}

fn validate_static_domain(schema: &Value, root: &Value) -> crate::Result<()> {
    let object = schema.as_object().expect("validated schema object");
    if let Some(kind) = object.get("type") {
        if let Some(constant) = object.get("const")
            && !matches_type(constant, kind)
        {
            return Err(build_error(
                "argument schema `const` contradicts its primitive type",
            ));
        }
        if let Some(values) = object.get("enum").and_then(Value::as_array)
            && !values.iter().any(|value| matches_type(value, kind))
        {
            return Err(build_error(
                "argument schema `enum` contradicts its primitive type",
            ));
        }
    }
    if let (Some(minimum), Some(maximum)) = (object.get("minimum"), object.get("maximum")) {
        let minimum = integer_as_i128(minimum).ok_or_else(|| {
            build_error("argument schema minimum is outside the supported integer domain")
        })?;
        let maximum = integer_as_i128(maximum).ok_or_else(|| {
            build_error("argument schema maximum is outside the supported integer domain")
        })?;
        if minimum > maximum {
            return Err(build_error("argument schema minimum cannot exceed maximum"));
        }
        if let Some(divisor) = object.get("multipleOf").and_then(integer_as_i128) {
            let first = ceil_multiple(minimum, divisor);
            if first > maximum {
                return Err(build_error(
                    "argument schema bounded divisible integer domain is empty",
                ));
            }
        }
    }
    if object
        .keys()
        .any(|key| matches!(key.as_str(), "minimum" | "maximum" | "multipleOf"))
        && let Some(values) = finite_values(object)
        && values
            .iter()
            .all(|value| !finite_value_satisfies_numeric_assertions(value, object))
    {
        let message = if object.contains_key("const") {
            "argument schema `const` contradicts its numeric assertions"
        } else {
            "argument schema `enum` has no value satisfying its numeric assertions"
        };
        return Err(build_error(message));
    }
    if let (Some(constant), Some(values)) = (
        object.get("const"),
        object.get("enum").and_then(Value::as_array),
    ) && !values
        .iter()
        .any(|value| schema_values_equal(value, constant))
    {
        return Err(build_error(
            "argument schema `const` contradicts its `enum`",
        ));
    }
    if let Some(minimum) = object.get("minLength").and_then(Value::as_u64) {
        if let Some(constant) = object.get("const").and_then(Value::as_str)
            && constant.chars().count() < minimum as usize
        {
            return Err(build_error(
                "argument schema `const` contradicts its `minLength`",
            ));
        }
        if let Some(values) = object.get("enum").and_then(Value::as_array)
            && values
                .iter()
                .filter_map(Value::as_str)
                .all(|value| value.chars().count() < minimum as usize)
        {
            return Err(build_error(
                "argument schema `enum` has no value satisfying `minLength`",
            ));
        }
    }
    if let Some(minimum) = object.get("minItems").and_then(Value::as_u64) {
        if let Some(constant) = object.get("const").and_then(Value::as_array)
            && constant.len() < minimum as usize
        {
            return Err(build_error(
                "argument schema `const` contradicts its `minItems`",
            ));
        }
        if let Some(values) = object.get("enum").and_then(Value::as_array)
            && values
                .iter()
                .filter_map(Value::as_array)
                .all(|value| value.len() < minimum as usize)
        {
            return Err(build_error(
                "argument schema `enum` has no value satisfying `minItems`",
            ));
        }
    }
    if let Some(required) = object.get("required").and_then(Value::as_array)
        && object.get("additionalProperties") == Some(&Value::Bool(false))
    {
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(name) = required
            .iter()
            .filter_map(Value::as_str)
            .find(|name| !properties.contains_key(*name))
        {
            return Err(build_error(format!(
                "argument schema requires undeclared closed-object property `{name}`"
            )));
        }
    }
    for nested in child_schemas(object) {
        validate_static_domain(nested, root)?;
    }
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let target = resolve_ref(root, reference).expect("references validated");
        let sibling_types = [
            (
                object
                    .keys()
                    .any(|key| matches!(key.as_str(), "minimum" | "maximum" | "multipleOf")),
                "integer",
            ),
            (object.contains_key("minLength"), "string"),
            (
                object
                    .keys()
                    .any(|key| matches!(key.as_str(), "items" | "minItems")),
                "array",
            ),
            (
                object.keys().any(|key| {
                    matches!(
                        key.as_str(),
                        "properties" | "required" | "additionalProperties"
                    )
                }),
                "object",
            ),
        ];
        for (_, expected) in sibling_types.into_iter().filter(|(present, _)| *present) {
            if !schema_implies_type(target, root, expected) {
                return Err(build_error(format!(
                    "argument schema reference `{reference}` has sibling assertions incompatible with `{expected}`"
                )));
            }
        }
        if schema_intersection_is_empty(schema, target, root) {
            return Err(build_error(format!(
                "argument schema reference `{reference}` has contradictory sibling assertions"
            )));
        }
        validate_static_domain(target, root)?;
    }
    if let Some(values) = finite_schema_values(schema, root)
        && values
            .iter()
            .all(|value| evaluate_node(value, schema, root, "", "", "@static").is_err())
    {
        return Err(build_error(
            "argument schema finite domain is statically empty",
        ));
    }
    Ok(())
}

pub(crate) fn validate_resource_reference_domain(
    resource: &crate::ResourceDecl,
    compiled: &CompiledArgumentSchema,
) -> crate::Result<()> {
    let Some(values) = finite_string_domain(&compiled.schema, &compiled.schema) else {
        return Ok(());
    };
    if values
        .iter()
        .any(|value| resource.parse_uri(value).is_some() || resource.mint_uri(value).is_ok())
    {
        Ok(())
    } else {
        Err(build_error(format!(
            "resource `{}` reference schema has no syntactically valid id or URI",
            resource.name
        )))
    }
}

fn finite_string_domain(schema: &Value, root: &Value) -> Option<Vec<String>> {
    let object = schema.as_object()?;
    let local = if let Some(value) = object.get("const") {
        Some(value.as_str().map(ToOwned::to_owned).into_iter().collect())
    } else if let Some(values) = object.get("enum").and_then(Value::as_array) {
        Some(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
        )
    } else if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
        let mut values = Vec::new();
        for branch in branches {
            values.extend(finite_string_domain(branch, root)?);
        }
        Some(values)
    } else {
        None
    };
    let referenced = object
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| resolve_ref(root, reference))
        .and_then(|target| finite_string_domain(target, root));
    match (local, referenced) {
        (Some(local), Some(referenced)) => Some(
            local
                .into_iter()
                .filter(|value| referenced.contains(value))
                .collect(),
        ),
        (Some(local), None) => Some(local),
        (None, Some(referenced)) => Some(referenced),
        (None, None) => None,
    }
}

fn child_schemas(object: &serde_json::Map<String, Value>) -> Vec<&Value> {
    let mut children = Vec::new();
    if let Some(items) = object.get("items") {
        children.push(items);
    }
    if let Some(additional) = object.get("additionalProperties")
        && additional.is_object()
    {
        children.push(additional);
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        children.extend(properties.values());
    }
    if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
        children.extend(branches);
    }
    children
}

fn ensure_coarse_compatibility(arg: &ArgSpec, schema: &Value) -> crate::Result<()> {
    let expected = match arg.value_type {
        ArgType::String | ArgType::Path | ArgType::ResourceRef(_) => Some("string"),
        ArgType::Bool => Some("boolean"),
        ArgType::Number => Some("number"),
        ArgType::Integer => Some("integer"),
        ArgType::Json => None,
        ArgType::Named(_) => return Ok(()),
    };
    if let Some(expected) = expected
        && !schema_implies_type(schema, schema, expected)
    {
        return Err(build_error(format!(
            "command argument `{}` schema does not imply `{expected}`",
            arg.name
        )));
    }
    Ok(())
}

fn schema_implies_type(schema: &Value, root: &Value, expected: &str) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if let Some(kind) = object.get("type") {
        return (type_contains(kind, expected)
            || (expected == "number" && type_contains(kind, "integer")))
            && (!type_contains(kind, "null") || expected == "null");
    }
    let expected_type = Value::String(expected.to_string());
    if let Some(constant) = object.get("const") {
        return matches_type(constant, &expected_type);
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return !values.is_empty()
            && values
                .iter()
                .all(|value| matches_type(value, &expected_type));
    }
    if object
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            !branches.is_empty()
                && branches
                    .iter()
                    .all(|branch| schema_implies_type(branch, root, expected))
        })
    {
        return true;
    }
    object
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| resolve_ref(root, reference))
        .is_some_and(|target| schema_implies_type(target, root, expected))
}

fn schemas_provably_disjoint(left: &Value, right: &Value, root: &Value) -> bool {
    let (Some(left), Some(right)) = (left.as_object(), right.as_object()) else {
        return false;
    };
    if let Some(target) = left
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| resolve_ref(root, reference))
        && schemas_provably_disjoint(target, &Value::Object(right.clone()), root)
    {
        return true;
    }
    if let Some(target) = right
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| resolve_ref(root, reference))
        && schemas_provably_disjoint(&Value::Object(left.clone()), target, root)
    {
        return true;
    }
    if let (Some(left_type), Some(right_type)) = (left.get("type"), right.get("type")) {
        let left_types = primitive_types(left_type);
        let right_types = primitive_types(right_type);
        let numeric_overlap = (left_types.contains("integer") && right_types.contains("number"))
            || (left_types.contains("number") && right_types.contains("integer"));
        if left_types.is_disjoint(&right_types) && !numeric_overlap {
            return true;
        }
    }
    let left_values = finite_values(left);
    let right_values = finite_values(right);
    if let (Some(left_values), Some(right_values)) = (left_values, right_values)
        && !left_values.iter().any(|left| {
            right_values
                .iter()
                .any(|right| schema_values_equal(left, right))
        })
    {
        return true;
    }
    let left_required = string_set(left.get("required"));
    let right_required = string_set(right.get("required"));
    let left_properties = left
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let right_properties = right
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if left.get("additionalProperties") == Some(&Value::Bool(false))
        && right_required
            .iter()
            .any(|name| !left_properties.contains_key(name))
    {
        return true;
    }
    if right.get("additionalProperties") == Some(&Value::Bool(false))
        && left_required
            .iter()
            .any(|name| !right_properties.contains_key(name))
    {
        return true;
    }
    left_required.intersection(&right_required).any(|name| {
        left_properties
            .get(name)
            .zip(right_properties.get(name))
            .is_some_and(|(left, right)| schemas_provably_disjoint(left, right, root))
    })
}

fn schema_intersection_is_empty(left: &Value, right: &Value, root: &Value) -> bool {
    if schemas_provably_disjoint(left, right, root) {
        return true;
    }
    if let Some(values) = finite_schema_values(left, root)
        && values
            .iter()
            .all(|value| evaluate_node(value, right, root, "", "", "@static").is_err())
    {
        return true;
    }
    if let Some(values) = finite_schema_values(right, root)
        && values
            .iter()
            .all(|value| evaluate_node(value, left, root, "", "", "@static").is_err())
    {
        return true;
    }
    let left = left.as_object().expect("validated schema object");
    let right = right.as_object().expect("validated schema object");
    let minimum = [left.get("minimum"), right.get("minimum")]
        .into_iter()
        .flatten()
        .filter_map(integer_as_i128)
        .max();
    let maximum = [left.get("maximum"), right.get("maximum")]
        .into_iter()
        .flatten()
        .filter_map(integer_as_i128)
        .min();
    minimum.zip(maximum).is_some_and(|(minimum, maximum)| {
        if minimum > maximum {
            return true;
        }
        let divisor = [left.get("multipleOf"), right.get("multipleOf")]
            .into_iter()
            .flatten()
            .filter_map(integer_as_i128)
            .reduce(integer_lcm);
        divisor.is_some_and(|divisor| ceil_multiple(minimum, divisor) > maximum)
    })
}

fn finite_schema_values(schema: &Value, root: &Value) -> Option<Vec<Value>> {
    let object = schema.as_object()?;
    if let Some(values) = finite_values(object) {
        return Some(values);
    }
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        return finite_schema_values(resolve_ref(root, reference)?, root);
    }
    let branches = object.get("oneOf")?.as_array()?;
    let mut values = Vec::new();
    for branch in branches {
        values.extend(finite_schema_values(branch, root)?);
    }
    Some(values)
}

fn integer_lcm(left: i128, right: i128) -> i128 {
    let mut a = left;
    let mut b = right;
    while b != 0 {
        (a, b) = (b, a.rem_euclid(b));
    }
    left / a * right
}

pub(crate) fn validate_value(
    compiled: &CompiledArgumentSchema,
    value: &Value,
) -> Result<ArgSchemaMatch, SchemaFailure> {
    let mut selections = evaluate_node(
        value,
        &compiled.schema,
        &compiled.schema,
        "",
        "",
        &compiled.identity,
    )?;
    selections.sort_by(|left, right| {
        (
            &left.instance_pointer,
            &left.one_of_pointer,
            &left.branch_pointer,
        )
            .cmp(&(
                &right.instance_pointer,
                &right.one_of_pointer,
                &right.branch_pointer,
            ))
    });
    Ok(ArgSchemaMatch { selections })
}

fn evaluate_node(
    value: &Value,
    schema: &Value,
    root: &Value,
    schema_pointer: &str,
    instance_pointer: &str,
    identity: &str,
) -> Result<Vec<SchemaBranchSelection>, SchemaFailure> {
    let object = schema.as_object().expect("compiled schema object");
    let mut selections = Vec::new();
    let mut failures = Vec::new();
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let target = resolve_ref(root, reference).expect("compiled reference");
        let target_pointer = reference.trim_start_matches('#');
        match evaluate_node(
            value,
            target,
            root,
            target_pointer,
            instance_pointer,
            identity,
        ) {
            Ok(nested) => selections.extend(nested),
            Err(failure) => failures.push(failure),
        }
    }
    if let Some(kind) = object.get("type")
        && !matches_type(value, kind)
    {
        failures.push(failure(
            instance_pointer,
            schema_pointer,
            ArgumentSchemaKeyword::Type,
            canonical_literal(kind),
        ));
    }
    if let Some(constant) = object.get("const")
        && !schema_values_equal(value, constant)
    {
        failures.push(failure(
            instance_pointer,
            schema_pointer,
            ArgumentSchemaKeyword::Const,
            canonical_literal(constant),
        ));
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array)
        && !values
            .iter()
            .any(|allowed| schema_values_equal(value, allowed))
    {
        failures.push(failure(
            instance_pointer,
            schema_pointer,
            ArgumentSchemaKeyword::Enum,
            values
                .iter()
                .map(canonical_literal)
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if let Some(number) = value.as_number() {
        if let Some(minimum) = object.get("minimum")
            && compare_numbers(number, minimum.as_number().expect("compiled number"))
                == std::cmp::Ordering::Less
        {
            failures.push(failure(
                instance_pointer,
                schema_pointer,
                ArgumentSchemaKeyword::Minimum,
                canonical_literal(minimum),
            ));
        }
        if let Some(maximum) = object.get("maximum")
            && compare_numbers(number, maximum.as_number().expect("compiled number"))
                == std::cmp::Ordering::Greater
        {
            failures.push(failure(
                instance_pointer,
                schema_pointer,
                ArgumentSchemaKeyword::Maximum,
                canonical_literal(maximum),
            ));
        }
        if let Some(divisor) = object.get("multipleOf")
            && !number_is_multiple(number, divisor.as_number().expect("compiled number"))
        {
            failures.push(failure(
                instance_pointer,
                schema_pointer,
                ArgumentSchemaKeyword::MultipleOf,
                canonical_literal(divisor),
            ));
        }
    }
    if let Some(minimum) = object.get("minLength").and_then(Value::as_u64)
        && value
            .as_str()
            .is_some_and(|text| text.chars().count() < minimum as usize)
    {
        failures.push(failure(
            instance_pointer,
            schema_pointer,
            ArgumentSchemaKeyword::MinLength,
            minimum.to_string(),
        ));
    }
    if let (Some(items), Some(values)) = (object.get("items"), value.as_array()) {
        for (index, value) in values.iter().enumerate() {
            let pointer = join_pointer(instance_pointer, &index.to_string());
            let schema_pointer = join_pointer(schema_pointer, "items");
            match evaluate_node(value, items, root, &schema_pointer, &pointer, identity) {
                Ok(nested) => selections.extend(nested),
                Err(failure) => failures.push(failure),
            }
        }
    }
    if let Some(minimum) = object.get("minItems").and_then(Value::as_u64)
        && value
            .as_array()
            .is_some_and(|values| values.len() < minimum as usize)
    {
        failures.push(failure(
            instance_pointer,
            schema_pointer,
            ArgumentSchemaKeyword::MinItems,
            minimum.to_string(),
        ));
    }
    if let Some(map) = value.as_object() {
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(required) = object.get("required").and_then(Value::as_array)
            && let Some(name) = required
                .iter()
                .filter_map(Value::as_str)
                .find(|name| !map.contains_key(*name))
        {
            failures.push(failure(
                instance_pointer,
                schema_pointer,
                ArgumentSchemaKeyword::Required,
                (*name).to_string(),
            ));
        }
        for (name, property_schema) in &properties {
            if let Some(property_value) = map.get(name) {
                match evaluate_node(
                    property_value,
                    property_schema,
                    root,
                    &join_pointer(&join_pointer(schema_pointer, "properties"), name),
                    &join_pointer(instance_pointer, name),
                    identity,
                ) {
                    Ok(nested) => selections.extend(nested),
                    Err(failure) => failures.push(failure),
                }
            }
        }
        for (name, property_value) in map {
            if properties.contains_key(name) {
                continue;
            }
            match object.get("additionalProperties") {
                Some(Value::Bool(false)) => {
                    failures.push(failure(
                        instance_pointer,
                        schema_pointer,
                        ArgumentSchemaKeyword::AdditionalProperties,
                        "no additional properties".to_string(),
                    ));
                    break;
                }
                Some(schema @ Value::Object(_)) => match evaluate_node(
                    property_value,
                    schema,
                    root,
                    &join_pointer(schema_pointer, "additionalProperties"),
                    &join_pointer(instance_pointer, name),
                    identity,
                ) {
                    Ok(nested) => selections.extend(nested),
                    Err(failure) => failures.push(failure),
                },
                _ => {}
            }
        }
    }
    if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
        let one_of_pointer = join_pointer(schema_pointer, "oneOf");
        let mut matched = Vec::new();
        let mut problems = Vec::with_capacity(branches.len());
        for (index, branch) in branches.iter().enumerate() {
            let branch_pointer = join_pointer(&one_of_pointer, &index.to_string());
            match evaluate_node(
                value,
                branch,
                root,
                &branch_pointer,
                instance_pointer,
                identity,
            ) {
                Ok(branch_selections) => matched.push((index, branch_selections)),
                Err(problem) => problems.push(problem.branch_problem()),
            }
        }
        if matched.len() != 1 {
            let mut mismatch = failure(
                instance_pointer,
                &one_of_pointer,
                ArgumentSchemaKeyword::OneOf,
                "exactly one branch".to_string(),
            );
            mismatch.branches = problems;
            failures.push(mismatch);
        } else {
            let (index, branch_selections) = matched.remove(0);
            selections.extend(branch_selections);
            selections.push(SchemaBranchSelection {
                schema: identity.to_string(),
                instance_pointer: instance_pointer.to_string(),
                one_of_pointer: one_of_pointer.clone(),
                branch_pointer: join_pointer(&one_of_pointer, &index.to_string()),
            });
        }
    }
    if failures.is_empty() {
        Ok(selections)
    } else {
        failures.sort_by(compare_failures);
        Err(failures.remove(0))
    }
}

pub(crate) fn compare_failures(left: &SchemaFailure, right: &SchemaFailure) -> std::cmp::Ordering {
    pointer_depth(&left.path)
        .cmp(&pointer_depth(&right.path))
        .then_with(|| keyword_rank(left.keyword).cmp(&keyword_rank(right.keyword)))
        .then_with(|| left.path.cmp(&right.path))
        .then_with(|| left.pointer.cmp(&right.pointer))
        .then_with(|| left.expected.cmp(&right.expected))
}

pub(crate) fn compare_argument_failures(
    left_argument: &str,
    left: &SchemaFailure,
    right_argument: &str,
    right: &SchemaFailure,
) -> std::cmp::Ordering {
    pointer_depth(&left.path)
        .cmp(&pointer_depth(&right.path))
        .then_with(|| keyword_rank(left.keyword).cmp(&keyword_rank(right.keyword)))
        // At the command boundary the argument is the canonical property.
        // In particular, competing presence failures order by trigger before
        // their missing targets rather than allowing `expected` to invert the
        // declaration graph's canonical order.
        .then_with(|| left_argument.cmp(right_argument))
        .then_with(|| left.path.cmp(&right.path))
        .then_with(|| left.pointer.cmp(&right.pointer))
        .then_with(|| left.expected.cmp(&right.expected))
}

fn keyword_rank(keyword: ArgumentSchemaKeyword) -> u8 {
    match keyword {
        ArgumentSchemaKeyword::Type => 0,
        ArgumentSchemaKeyword::Const => 1,
        ArgumentSchemaKeyword::Enum => 2,
        ArgumentSchemaKeyword::Minimum => 3,
        ArgumentSchemaKeyword::Maximum => 4,
        ArgumentSchemaKeyword::MultipleOf => 5,
        ArgumentSchemaKeyword::MinLength => 6,
        ArgumentSchemaKeyword::Items => 7,
        ArgumentSchemaKeyword::MinItems => 8,
        ArgumentSchemaKeyword::Required => 9,
        ArgumentSchemaKeyword::DependentRequired => 10,
        ArgumentSchemaKeyword::AdditionalProperties => 11,
        ArgumentSchemaKeyword::OneOf => 12,
    }
}

fn failure(
    path: &str,
    pointer: &str,
    keyword: ArgumentSchemaKeyword,
    expected: String,
) -> SchemaFailure {
    SchemaFailure {
        path: path.to_string(),
        pointer: pointer.to_string(),
        keyword,
        expected,
        branches: Vec::new(),
    }
}

fn matches_type(value: &Value, kind: &Value) -> bool {
    primitive_types(kind)
        .iter()
        .any(|kind| match kind.as_str() {
            "null" => value.is_null(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            "number" => value.is_number(),
            "integer" => value
                .as_number()
                .is_some_and(JsonInteger::number_is_integer),
            "string" => value.is_string(),
            _ => false,
        })
}

fn resolve_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    root.get("$defs")?
        .as_object()?
        .get(reference.strip_prefix("#/$defs/")?)
}

fn type_contains(value: &Value, expected: &str) -> bool {
    primitive_types(value).contains(expected)
}

fn primitive_types(value: &Value) -> BTreeSet<String> {
    match value {
        Value::String(kind) => [kind.clone()].into_iter().collect(),
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn finite_values(object: &serde_json::Map<String, Value>) -> Option<Vec<Value>> {
    if let Some(value) = object.get("const") {
        return Some(vec![value.clone()]);
    }
    object.get("enum").and_then(Value::as_array).cloned()
}

fn finite_value_satisfies_numeric_assertions(
    value: &Value,
    schema: &serde_json::Map<String, Value>,
) -> bool {
    let Some(number) = value.as_number() else {
        return false;
    };
    if schema
        .get("type")
        .is_some_and(|kind| !matches_type(value, kind))
    {
        return false;
    }
    if schema.get("minimum").is_some_and(|minimum| {
        compare_numbers(number, minimum.as_number().expect("validated minimum"))
            == std::cmp::Ordering::Less
    }) {
        return false;
    }
    if schema.get("maximum").is_some_and(|maximum| {
        compare_numbers(number, maximum.as_number().expect("validated maximum"))
            == std::cmp::Ordering::Greater
    }) {
        return false;
    }
    !schema.get("multipleOf").is_some_and(|divisor| {
        !number_is_multiple(number, divisor.as_number().expect("validated divisor"))
    })
}

fn schema_values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            compare_numbers(left, right) == std::cmp::Ordering::Equal
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| schema_values_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(name, left)| {
                    right
                        .get(name)
                        .is_some_and(|right| schema_values_equal(left, right))
                })
        }
        _ => left == right,
    }
}

fn string_set(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn literal_kind(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

fn canonical_literal(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn number_is_positive(value: &Value) -> bool {
    value
        .as_number()
        .is_some_and(|number| number.as_f64().is_some_and(|number| number > 0.0))
}

fn integer_as_i128(value: &Value) -> Option<i128> {
    let number = value.as_number()?;
    if let Some(value) = number.as_i64() {
        return Some(i128::from(value));
    }
    if let Some(value) = number.as_u64() {
        return Some(i128::from(value));
    }
    let value = number.as_f64()?;
    (value.is_finite()
        && value.fract() == 0.0
        && value >= i128::MIN as f64
        && value < i128::MAX as f64)
        .then_some(value as i128)
}

fn ceil_multiple(value: i128, divisor: i128) -> i128 {
    let remainder = value.rem_euclid(divisor);
    if remainder == 0 {
        value
    } else {
        value + (divisor - remainder)
    }
}

fn compare_numbers(left: &Number, right: &Number) -> std::cmp::Ordering {
    match (
        integer_as_i128(&Value::Number(left.clone())),
        integer_as_i128(&Value::Number(right.clone())),
    ) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left
            .as_f64()
            .partial_cmp(&right.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
    }
}

fn number_is_multiple(value: &Number, divisor: &Number) -> bool {
    match (
        integer_as_i128(&Value::Number(value.clone())),
        integer_as_i128(&Value::Number(divisor.clone())),
    ) {
        (Some(value), Some(divisor)) => value.rem_euclid(divisor) == 0,
        _ => {
            let (Some(value), Some(divisor)) = (value.as_f64(), divisor.as_f64()) else {
                return false;
            };
            value % divisor == 0.0
        }
    }
}

fn join_pointer(base: &str, segment: &str) -> String {
    format!("{base}/{}", segment.replace('~', "~0").replace('/', "~1"))
}

fn pointer_depth(pointer: &str) -> usize {
    pointer.bytes().filter(|byte| *byte == b'/').count()
}

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}
