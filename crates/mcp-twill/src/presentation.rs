use std::{collections::BTreeMap, fmt::Write as _};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ArgSpec, ArgType, CommandSpec, FrameworkError};

const TITLE_LIMIT: usize = 80;
const INTERPOLATION_LIMIT: usize = 256;
const BODY_LIMIT: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationPresentation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<ConfirmationPresentation>,
}

impl OperationPresentation {
    pub(crate) fn is_empty(&self) -> bool {
        self.invocation_message.is_none() && self.confirmation.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationPresentation {
    pub default: ConfirmationMessage,
    pub cases: Vec<ConfirmationCase>,
}

impl ConfirmationPresentation {
    pub fn new(default: ConfirmationMessage) -> Self {
        Self {
            default,
            cases: Vec::new(),
        }
    }

    pub fn case(mut self, when: ConfirmationPredicate, message: ConfirmationMessage) -> Self {
        self.cases.push(ConfirmationCase { when, message });
        self
    }

    pub(crate) fn prepare_validated(
        &self,
        operation_id: &str,
        arguments: &BTreeMap<String, Value>,
    ) -> PreparedConfirmation {
        self.prepare(operation_id, arguments)
    }

    pub(crate) fn prepare_unvalidated(
        &self,
        operation_id: &str,
        arguments: &BTreeMap<String, Value>,
    ) -> PreparedConfirmation {
        self.prepare(operation_id, arguments)
    }

    fn prepare(
        &self,
        operation_id: &str,
        arguments: &BTreeMap<String, Value>,
    ) -> PreparedConfirmation {
        let selected = self
            .cases
            .iter()
            .find(|case| predicate_matches(&case.when, arguments));
        let (branch, message) =
            selected.map_or((ConfirmationBranch::Default, &self.default), |case| {
                (
                    ConfirmationBranch::Case {
                        predicate: case.when.clone(),
                    },
                    &case.message,
                )
            });
        PreparedConfirmation {
            operation_id: operation_id.to_string(),
            branch,
            title: message.title.clone(),
            message: render_message(message, arguments),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationCase {
    pub when: ConfirmationPredicate,
    pub message: ConfirmationMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfirmationPredicate {
    ArgumentPresent { argument: String },
    ArgumentEquals { argument: String, value: Value },
}

impl ConfirmationPredicate {
    pub fn argument_present(argument: impl Into<String>) -> Self {
        Self::ArgumentPresent {
            argument: argument.into(),
        }
    }

    pub fn argument_equals(argument: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::ArgumentEquals {
            argument: argument.into(),
            value: value.into(),
        }
    }

    fn argument(&self) -> &str {
        match self {
            Self::ArgumentPresent { argument } | Self::ArgumentEquals { argument, .. } => argument,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationMessage {
    pub title: String,
    pub body: Vec<ConfirmationSegment>,
}

impl ConfirmationMessage {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: Vec::new(),
        }
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.body.push(ConfirmationSegment::Text(text.into()));
        self
    }

    pub fn argument(
        mut self,
        argument: impl Into<String>,
        rendering: ArgumentRendering,
        fallback: impl Into<String>,
    ) -> Self {
        self.body.push(ConfirmationSegment::Argument {
            argument: argument.into(),
            rendering,
            fallback: fallback.into(),
        });
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfirmationSegment {
    Text(String),
    Argument {
        argument: String,
        rendering: ArgumentRendering,
        fallback: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ArgumentRendering {
    Plain,
    JsonString,
    TrimmedJsonString,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreparedInvocationPresentation {
    pub invocation_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<PreparedConfirmation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfacePresentationDefaults {
    invocation_message: String,
    confirmation_title: String,
    confirmation_message: String,
}

impl SurfacePresentationDefaults {
    pub(crate) fn new(
        invocation_message: impl Into<String>,
        confirmation_title: impl Into<String>,
        confirmation_message: impl Into<String>,
    ) -> crate::Result<Self> {
        let defaults = Self {
            invocation_message: invocation_message.into(),
            confirmation_title: confirmation_title.into(),
            confirmation_message: confirmation_message.into(),
        };
        validate_static_text(
            &defaults.invocation_message,
            "surface invocation message",
            TITLE_LIMIT,
        )?;
        validate_static_text(
            &defaults.confirmation_title,
            "surface confirmation title",
            TITLE_LIMIT,
        )?;
        validate_static_text(
            &defaults.confirmation_message,
            "surface confirmation message",
            BODY_LIMIT,
        )?;
        Ok(defaults)
    }

    pub fn invocation_message(&self) -> &str {
        &self.invocation_message
    }

    pub fn confirmation_title(&self) -> &str {
        &self.confirmation_title
    }

    pub fn confirmation_message(&self) -> &str {
        &self.confirmation_message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmationPresentationRequest {
    Omit,
    DeclaredOnly,
    DeclaredOrSurfaceDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreparedConfirmation {
    pub operation_id: String,
    pub branch: ConfirmationBranch,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConfirmationBranch {
    SurfaceDefault,
    Default,
    Case { predicate: ConfirmationPredicate },
}

impl CommandSpec {
    pub(crate) fn prepare_validated_presentation(
        &self,
        defaults: &SurfacePresentationDefaults,
        operation_id: &str,
        arguments: &BTreeMap<String, Value>,
        confirmation: ConfirmationPresentationRequest,
    ) -> PreparedInvocationPresentation {
        self.prepare_presentation(defaults, operation_id, arguments, confirmation, true)
    }

    pub(crate) fn prepare_unvalidated_presentation(
        &self,
        defaults: &SurfacePresentationDefaults,
        operation_id: &str,
        arguments: &BTreeMap<String, Value>,
        confirmation: ConfirmationPresentationRequest,
    ) -> PreparedInvocationPresentation {
        self.prepare_presentation(defaults, operation_id, arguments, confirmation, false)
    }

    fn prepare_presentation(
        &self,
        defaults: &SurfacePresentationDefaults,
        operation_id: &str,
        arguments: &BTreeMap<String, Value>,
        confirmation: ConfirmationPresentationRequest,
        validated: bool,
    ) -> PreparedInvocationPresentation {
        let confirmation = match (confirmation, &self.confirmation) {
            (ConfirmationPresentationRequest::Omit, _) => None,
            (ConfirmationPresentationRequest::DeclaredOnly, None) => None,
            (_, Some(declaration)) => Some(if validated {
                declaration.prepare_validated(operation_id, arguments)
            } else {
                declaration.prepare_unvalidated(operation_id, arguments)
            }),
            (ConfirmationPresentationRequest::DeclaredOrSurfaceDefault, None) => {
                Some(PreparedConfirmation {
                    operation_id: operation_id.to_string(),
                    branch: ConfirmationBranch::SurfaceDefault,
                    title: defaults.confirmation_title.clone(),
                    message: defaults.confirmation_message.clone(),
                })
            }
        };
        PreparedInvocationPresentation {
            invocation_message: self
                .invocation_message
                .clone()
                .unwrap_or_else(|| defaults.invocation_message.clone()),
            confirmation,
        }
    }
}

pub(crate) fn operation_presentation_is_absent(
    presentation: &Option<OperationPresentation>,
) -> bool {
    presentation
        .as_ref()
        .is_none_or(OperationPresentation::is_empty)
}

pub(crate) fn validate_command_presentation(
    spec: &CommandSpec,
    declarations: &crate::argument_schemas::SchemaDecls,
) -> crate::Result<()> {
    let command = spec.name();
    if let Some(message) = &spec.invocation_message {
        validate_static_text(
            message,
            &format!("command `{command}` invocation message"),
            TITLE_LIMIT,
        )?;
    }
    let Some(confirmation) = &spec.confirmation else {
        return Ok(());
    };
    validate_message(spec, declarations, &confirmation.default, "default")?;
    for (index, case) in confirmation.cases.iter().enumerate() {
        validate_predicate(spec, declarations, &case.when, index)?;
        validate_message(spec, declarations, &case.message, &format!("case {index}"))?;
    }
    for left in 0..confirmation.cases.len() {
        for right in left + 1..confirmation.cases.len() {
            validate_disjoint_predicates(
                &command,
                left,
                &confirmation.cases[left].when,
                right,
                &confirmation.cases[right].when,
            )?;
        }
    }
    Ok(())
}

fn validate_predicate(
    spec: &CommandSpec,
    declarations: &crate::argument_schemas::SchemaDecls,
    predicate: &ConfirmationPredicate,
    index: usize,
) -> crate::Result<()> {
    let command = spec.name();
    let arg = spec.arg(predicate.argument()).ok_or_else(|| {
        build_error(format!(
            "command `{command}` confirmation case {index} references unknown argument `{}`",
            predicate.argument()
        ))
    })?;
    match predicate {
        ConfirmationPredicate::ArgumentPresent { .. } => {
            if arg.required {
                return Err(build_error(format!(
                    "command `{command}` confirmation case {index} is tautological because argument `{}` is required",
                    arg.name
                )));
            }
        }
        ConfirmationPredicate::ArgumentEquals { value, .. } => {
            if !matches!(value, Value::String(_) | Value::Bool(_) | Value::Null) {
                return Err(build_error(format!(
                    "command `{command}` confirmation case {index} equality must use a string, boolean, or null constant"
                )));
            }
            let compiled = crate::argument_schemas::compile_argument_schema(arg, declarations)?;
            if !argument_accepts_value(arg, compiled.as_ref(), value) {
                return Err(build_error(format!(
                    "command `{command}` confirmation case {index} constant is not accepted by argument `{}`",
                    arg.name
                )));
            }
            if arg.required
                && crate::argument_schemas::presentation_singleton_value(compiled.as_ref())
                    .as_ref()
                    .is_some_and(|only| only == value)
            {
                return Err(build_error(format!(
                    "command `{command}` confirmation case {index} is tautological for argument `{}`",
                    arg.name
                )));
            }
        }
    }
    Ok(())
}

fn validate_disjoint_predicates(
    command: &str,
    left_index: usize,
    left: &ConfirmationPredicate,
    right_index: usize,
    right: &ConfirmationPredicate,
) -> crate::Result<()> {
    if left.argument() != right.argument() {
        return Err(build_error(format!(
            "command `{command}` confirmation cases {left_index} and {right_index} use different arguments and are not provably disjoint"
        )));
    }
    match (left, right) {
        (
            ConfirmationPredicate::ArgumentEquals { value: left, .. },
            ConfirmationPredicate::ArgumentEquals { value: right, .. },
        ) if left != right => Ok(()),
        _ => Err(build_error(format!(
            "command `{command}` confirmation cases {left_index} and {right_index} overlap"
        ))),
    }
}

fn validate_message(
    spec: &CommandSpec,
    declarations: &crate::argument_schemas::SchemaDecls,
    message: &ConfirmationMessage,
    location: &str,
) -> crate::Result<()> {
    let command = spec.name();
    validate_static_text(
        &message.title,
        &format!("command `{command}` confirmation {location} title"),
        TITLE_LIMIT,
    )?;
    if message.body.is_empty() {
        return Err(build_error(format!(
            "command `{command}` confirmation {location} body is empty"
        )));
    }
    let mut maximum = 0usize;
    for (index, segment) in message.body.iter().enumerate() {
        match segment {
            ConfirmationSegment::Text(text) => {
                validate_static_text(
                    text,
                    &format!("command `{command}` confirmation {location} segment {index}"),
                    BODY_LIMIT,
                )?;
                maximum += text.chars().count();
            }
            ConfirmationSegment::Argument {
                argument,
                rendering,
                fallback,
            } => {
                validate_static_text(
                    fallback,
                    &format!(
                        "command `{command}` confirmation {location} segment {index} fallback"
                    ),
                    INTERPOLATION_LIMIT,
                )?;
                let arg = spec.arg(argument).ok_or_else(|| {
                    build_error(format!(
                        "command `{command}` confirmation {location} segment {index} references unknown argument `{argument}`"
                    ))
                })?;
                let compiled = crate::argument_schemas::compile_argument_schema(arg, declarations)?;
                let domain = argument_domain(arg, compiled.as_ref());
                let value_width = match (rendering, domain) {
                    (ArgumentRendering::Plain, PresentationDomain::Boolean) => 5,
                    (ArgumentRendering::Plain, PresentationDomain::String)
                    | (ArgumentRendering::Plain, PresentationDomain::StringOrBoolean)
                    | (ArgumentRendering::JsonString, PresentationDomain::String)
                    | (ArgumentRendering::TrimmedJsonString, PresentationDomain::String) => {
                        INTERPOLATION_LIMIT
                    }
                    _ => {
                        return Err(build_error(format!(
                            "command `{command}` confirmation {location} segment {index} rendering is incompatible with argument `{argument}`"
                        )));
                    }
                };
                maximum += value_width.max(fallback.chars().count());
            }
        }
    }
    if maximum > BODY_LIMIT {
        return Err(build_error(format!(
            "command `{command}` confirmation {location} body can render to {maximum} scalars, exceeding {BODY_LIMIT}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentationDomain {
    String,
    Boolean,
    StringOrBoolean,
    Other,
}

fn argument_domain(
    arg: &ArgSpec,
    compiled: Option<&crate::argument_schemas::CompiledArgumentSchema>,
) -> PresentationDomain {
    if arg.repeated || matches!(arg.value_type, ArgType::ResourceRef(_) | ArgType::Named(_)) {
        return PresentationDomain::Other;
    }
    if let Some(domain) = crate::argument_schemas::presentation_domain(compiled) {
        return match domain {
            crate::argument_schemas::PresentationDomain::String => PresentationDomain::String,
            crate::argument_schemas::PresentationDomain::Boolean => PresentationDomain::Boolean,
            crate::argument_schemas::PresentationDomain::StringOrBoolean => {
                PresentationDomain::StringOrBoolean
            }
        };
    }
    match arg.value_type {
        ArgType::String | ArgType::Path => PresentationDomain::String,
        ArgType::Bool => PresentationDomain::Boolean,
        _ => PresentationDomain::Other,
    }
}

fn argument_accepts_value(
    arg: &ArgSpec,
    compiled: Option<&crate::argument_schemas::CompiledArgumentSchema>,
    value: &Value,
) -> bool {
    if arg.repeated || matches!(arg.value_type, ArgType::ResourceRef(_) | ArgType::Named(_)) {
        return false;
    }
    if let Some(compiled) = compiled {
        return crate::argument_schemas::validate_value(compiled, value).is_ok();
    }
    matches!(
        (&arg.value_type, value),
        (ArgType::String | ArgType::Path, Value::String(_))
            | (ArgType::Bool, Value::Bool(_))
            | (ArgType::Json, _)
    )
}

fn validate_static_text(text: &str, subject: &str, limit: usize) -> crate::Result<()> {
    if text.is_empty() {
        return Err(build_error(format!("{subject} is empty")));
    }
    if let Some(scalar) = text
        .chars()
        .find(|scalar| presentation_scalar_is_unsafe(*scalar))
    {
        return Err(build_error(format!(
            "{subject} contains presentation-unsafe scalar U+{:04X}",
            scalar as u32
        )));
    }
    let count = text.chars().count();
    if count > limit {
        return Err(build_error(format!(
            "{subject} has {count} scalars, exceeding {limit}"
        )));
    }
    Ok(())
}

fn presentation_scalar_is_unsafe(scalar: char) -> bool {
    matches!(scalar,
        '\u{0000}'..='\u{001F}'
        | '\u{007F}'..='\u{009F}'
        | '\u{061C}'
        | '\u{200E}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
    )
}

fn predicate_matches(
    predicate: &ConfirmationPredicate,
    arguments: &BTreeMap<String, Value>,
) -> bool {
    match predicate {
        ConfirmationPredicate::ArgumentPresent { argument } => arguments.contains_key(argument),
        ConfirmationPredicate::ArgumentEquals { argument, value } => {
            arguments.get(argument) == Some(value)
        }
    }
}

fn render_message(message: &ConfirmationMessage, arguments: &BTreeMap<String, Value>) -> String {
    let mut rendered = String::new();
    for segment in &message.body {
        match segment {
            ConfirmationSegment::Text(text) => rendered.push_str(text),
            ConfirmationSegment::Argument {
                argument,
                rendering,
                fallback,
            } => rendered.push_str(&render_argument(
                arguments.get(argument),
                *rendering,
                fallback,
            )),
        }
    }
    rendered
}

fn render_argument(value: Option<&Value>, rendering: ArgumentRendering, fallback: &str) -> String {
    match (rendering, value) {
        (ArgumentRendering::Plain, Some(Value::String(value))) if !value.is_empty() => {
            encode_presentation_string(value, false)
        }
        (ArgumentRendering::Plain, Some(Value::Bool(value))) => value.to_string(),
        (ArgumentRendering::JsonString, Some(Value::String(value))) if !value.is_empty() => {
            encode_presentation_string(value, true)
        }
        (ArgumentRendering::TrimmedJsonString, Some(Value::String(value))) => {
            let value = value.trim_matches(ecmascript_trim_scalar);
            if value.is_empty() {
                fallback.to_string()
            } else {
                encode_presentation_string(value, true)
            }
        }
        _ => fallback.to_string(),
    }
}

fn encode_presentation_string(value: &str, quoted: bool) -> String {
    let fixed_width = usize::from(quoted) * 2;
    let fits = value
        .chars()
        .try_fold(fixed_width, |width, scalar| {
            let width = width + escaped_scalar_width(scalar);
            (width <= INTERPOLATION_LIMIT).then_some(width)
        })
        .is_some();
    let mut output = String::new();
    if quoted {
        output.push('"');
    }
    let limit = if fits {
        INTERPOLATION_LIMIT - fixed_width
    } else if quoted {
        253
    } else {
        255
    };
    let mut width = 0usize;
    for scalar in value.chars() {
        let scalar_width = escaped_scalar_width(scalar);
        if width + scalar_width > limit {
            break;
        }
        push_escaped_scalar(&mut output, scalar);
        width += scalar_width;
    }
    if !fits {
        output.push('…');
    }
    if quoted {
        output.push('"');
    }
    output
}

fn escaped_scalar_width(scalar: char) -> usize {
    match scalar {
        '"' | '\\' | '\u{0008}' | '\u{000C}' | '\n' | '\r' | '\t' => 2,
        scalar if presentation_scalar_uses_escape(scalar) => 6,
        _ => 1,
    }
}

fn push_escaped_scalar(output: &mut String, scalar: char) {
    match scalar {
        '"' => output.push_str("\\\""),
        '\\' => output.push_str("\\\\"),
        '\u{0008}' => output.push_str("\\b"),
        '\u{000C}' => output.push_str("\\f"),
        '\n' => output.push_str("\\n"),
        '\r' => output.push_str("\\r"),
        '\t' => output.push_str("\\t"),
        scalar if presentation_scalar_uses_escape(scalar) => {
            write!(output, "\\u{:04X}", scalar as u32)
                .expect("writing presentation escape to String cannot fail");
        }
        scalar => output.push(scalar),
    }
}

fn presentation_scalar_uses_escape(scalar: char) -> bool {
    matches!(scalar,
        '\u{0000}'..='\u{001F}'
        | '\u{007F}'..='\u{009F}'
        | '\u{061C}'
        | '\u{200E}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
    )
}

fn ecmascript_trim_scalar(scalar: char) -> bool {
    matches!(
        scalar,
        '\u{0009}' | '\u{000B}' | '\u{000C}' | '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
                | '\u{000A}'
                | '\u{000D}'
                | '\u{2028}'
                | '\u{2029}'
    )
}

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArgSpec, PermissionSpec};
    use serde_json::json;

    #[test]
    fn string_encoder_preserves_complete_escape_boundaries() {
        let value = format!("{}\n", "a".repeat(255));
        let rendered = encode_presentation_string(&value, false);
        assert_eq!(rendered, format!("{}…", "a".repeat(255)));
        assert_eq!(rendered.chars().count(), 256);
    }

    #[test]
    fn trimmed_json_uses_the_fixed_ecmascript_table() {
        let value = Value::String("\u{0085}  visible \u{3000}".to_string());
        assert_eq!(
            render_argument(
                Some(&value),
                ArgumentRendering::TrimmedJsonString,
                "missing"
            ),
            "\"\\u0085  visible\""
        );
    }

    #[test]
    fn interpolation_bounds_match_the_public_contract() {
        let plain = "x".repeat(256);
        assert_eq!(encode_presentation_string(&plain, false), plain);
        assert_eq!(
            encode_presentation_string(&"x".repeat(257), false),
            format!("{}…", "x".repeat(255))
        );
        assert_eq!(
            encode_presentation_string(&"x".repeat(254), true),
            format!("\"{}\"", "x".repeat(254))
        );
        assert_eq!(
            encode_presentation_string(&"x".repeat(255), true),
            format!("\"{}…\"", "x".repeat(253))
        );
        assert_eq!(
            encode_presentation_string(&"x".repeat(1_000_000), false),
            format!("{}…", "x".repeat(255))
        );
    }

    #[test]
    fn every_unsafe_escape_class_is_complete_and_uppercase() {
        let scalars = [
            '\u{0000}', '\u{001F}', '\u{007F}', '\u{0085}', '\u{009F}', '\u{061C}', '\u{200E}',
            '\u{2028}', '\u{202E}', '\u{2060}', '\u{206F}', '\u{FEFF}',
        ];
        for scalar in scalars {
            let encoded = encode_presentation_string(&scalar.to_string(), false);
            assert_eq!(encoded, format!("\\u{:04X}", scalar as u32));
            assert_eq!(encoded.chars().count(), 6);
        }
        assert_eq!(
            encode_presentation_string("\"\\\u{0008}\u{000C}\n\r\t", false),
            "\\\"\\\\\\b\\f\\n\\r\\t"
        );
    }

    #[test]
    fn isolated_utf16_surrogates_cannot_enter_rust_presentation_values() {
        assert!(serde_json::from_str::<Value>(r#""\uD800""#).is_err());
        assert!(serde_json::from_str::<Value>(r#""\uDC00""#).is_err());
        assert_eq!(
            serde_json::from_str::<Value>(r#""\uD83D\uDE00""#).unwrap(),
            Value::String("😀".to_string())
        );
    }

    #[test]
    fn every_ecmascript_edge_scalar_trims_but_c1_does_not() {
        let trim = [
            '\u{0009}', '\u{000B}', '\u{000C}', '\u{0020}', '\u{00A0}', '\u{1680}', '\u{2000}',
            '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
            '\u{2008}', '\u{2009}', '\u{200A}', '\u{202F}', '\u{205F}', '\u{3000}', '\u{FEFF}',
            '\u{000A}', '\u{000D}', '\u{2028}', '\u{2029}',
        ];
        for scalar in trim {
            let value = Value::String(format!("{scalar}kept{scalar}"));
            assert_eq!(
                render_argument(
                    Some(&value),
                    ArgumentRendering::TrimmedJsonString,
                    "fallback"
                ),
                "\"kept\""
            );
        }
        let c1 = Value::String("\u{0085}".to_string());
        assert_eq!(
            render_argument(Some(&c1), ArgumentRendering::TrimmedJsonString, "fallback"),
            "\"\\u0085\""
        );
    }

    fn message_with_two_arguments(static_width: usize) -> ConfirmationMessage {
        ConfirmationMessage::new("Confirm?")
            .text("x".repeat(static_width))
            .argument("first", ArgumentRendering::Plain, "missing")
            .argument("second", ArgumentRendering::Plain, "missing")
    }

    fn two_argument_spec(message: ConfirmationMessage) -> CommandSpec {
        let mut spec = CommandSpec::new(["bounded"], "Bounded", "Bounded presentation")
            .with_arg(ArgSpec::string("first", "First value"))
            .with_arg(ArgSpec::string("second", "Second value"))
            .with_permission(PermissionSpec::write("test", "Test"));
        spec.confirmation = Some(ConfirmationPresentation::new(message));
        spec
    }

    #[test]
    fn worst_case_body_accepts_1024_and_rejects_1025() {
        assert!(
            validate_command_presentation(
                &two_argument_spec(message_with_two_arguments(512)),
                &BTreeMap::new()
            )
            .is_ok()
        );
        assert!(
            validate_command_presentation(
                &two_argument_spec(message_with_two_arguments(513)),
                &BTreeMap::new()
            )
            .unwrap_err()
            .to_string()
            .contains("1025")
        );
    }

    #[test]
    fn predicates_are_satisfiable_non_tautological_and_disjoint() {
        let mut flag =
            ArgSpec::inline_schema("flag", json!({ "type": "boolean" }), "Optional flag");
        flag.required = false;
        let mut spec =
            CommandSpec::new(["conditional"], "Conditional", "Conditional").with_arg(flag);
        spec.confirmation = Some(
            ConfirmationPresentation::new(ConfirmationMessage::new("Default?").text("Default."))
                .case(
                    ConfirmationPredicate::argument_equals("flag", true),
                    ConfirmationMessage::new("True?").text("True."),
                )
                .case(
                    ConfirmationPredicate::argument_equals("flag", false),
                    ConfirmationMessage::new("False?").text("False."),
                ),
        );
        validate_command_presentation(&spec, &BTreeMap::new()).unwrap();

        let mut required = spec.clone();
        required.args[0].required = true;
        required.confirmation = Some(
            ConfirmationPresentation::new(ConfirmationMessage::new("Default?").text("Default."))
                .case(
                    ConfirmationPredicate::argument_present("flag"),
                    ConfirmationMessage::new("Present?").text("Present."),
                ),
        );
        assert!(
            validate_command_presentation(&required, &BTreeMap::new())
                .unwrap_err()
                .to_string()
                .contains("tautological")
        );
    }

    #[test]
    fn resource_references_and_repeated_values_never_enter_presentation() {
        for arg in [
            ArgSpec {
                name: "value".to_string(),
                value_type: ArgType::ResourceRef("tab".to_string()),
                required: true,
                summary: "Tab reference".to_string(),
                workspace: None,
                repeated: false,
                schema: None,
                requires_arguments: Vec::new(),
            },
            ArgSpec::named("value", "private-record", "Named record"),
            ArgSpec::string("value", "Repeated values").repeated(),
        ] {
            let mut spec = CommandSpec::new(["private"], "Private", "Private value").with_arg(arg);
            spec.confirmation = Some(ConfirmationPresentation::new(
                ConfirmationMessage::new("Confirm?").argument(
                    "value",
                    ArgumentRendering::Plain,
                    "(unavailable)",
                ),
            ));
            assert!(
                validate_command_presentation(&spec, &BTreeMap::new())
                    .unwrap_err()
                    .to_string()
                    .contains("incompatible")
            );
        }
    }

    #[test]
    fn nullable_primitive_domains_are_rejected_for_interpolation() {
        let mut spec = CommandSpec::new(["nullable"], "Nullable", "Nullable value").with_arg(
            ArgSpec::inline_schema(
                "value",
                json!({ "type": ["string", "null"] }),
                "Nullable string",
            ),
        );
        spec.confirmation = Some(ConfirmationPresentation::new(
            ConfirmationMessage::new("Confirm?").argument(
                "value",
                ArgumentRendering::JsonString,
                "(missing)",
            ),
        ));
        assert!(
            validate_command_presentation(&spec, &BTreeMap::new())
                .unwrap_err()
                .to_string()
                .contains("incompatible")
        );
    }

    #[test]
    fn ref_siblings_narrow_the_complete_rendering_domain() {
        let arg = ArgSpec::inline_schema(
            "value",
            json!({
                "$defs": { "text": { "type": "string" } },
                "$ref": "#/$defs/text",
                "oneOf": [
                    { "type": "string" },
                    { "type": "boolean" }
                ]
            }),
            "Narrowed string",
        );
        let mut spec = CommandSpec::new(["narrowed"], "Narrowed", "Narrowed value").with_arg(arg);
        spec.confirmation = Some(ConfirmationPresentation::new(
            ConfirmationMessage::new("Confirm?").argument(
                "value",
                ArgumentRendering::JsonString,
                "(missing)",
            ),
        ));
        validate_command_presentation(&spec, &BTreeMap::new()).unwrap();
    }

    #[test]
    fn case_selection_is_order_independent_for_disjoint_cases() {
        let first =
            ConfirmationPresentation::new(ConfirmationMessage::new("Default?").text("Default."))
                .case(
                    ConfirmationPredicate::argument_equals("flag", true),
                    ConfirmationMessage::new("True?").text("True."),
                )
                .case(
                    ConfirmationPredicate::argument_equals("flag", false),
                    ConfirmationMessage::new("False?").text("False."),
                );
        let second = ConfirmationPresentation {
            default: first.default.clone(),
            cases: first.cases.iter().cloned().rev().collect(),
        };
        let arguments = BTreeMap::from([("flag".to_string(), Value::Bool(true))]);
        assert_eq!(
            first.prepare_unvalidated("conditional", &arguments),
            second.prepare_unvalidated("conditional", &arguments)
        );
        assert_ne!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(second).unwrap()
        );
    }

    #[test]
    fn surface_defaults_use_the_exact_version_one_templates() {
        let defaults = SurfacePresentationDefaults::new(
            "Running Console",
            "Confirmation required",
            "Run Console?",
        )
        .unwrap();
        let mut spec = CommandSpec::new(["console"], "Console", "Console diagnostics");
        spec.confirmation = Some(ConfirmationPresentation::new(
            ConfirmationMessage::new("Run console?").text("Run console."),
        ));
        let omitted = spec.prepare_unvalidated_presentation(
            &defaults,
            "console",
            &BTreeMap::new(),
            ConfirmationPresentationRequest::Omit,
        );
        assert!(omitted.confirmation.is_none());
        let declared_only = spec.prepare_unvalidated_presentation(
            &defaults,
            "console",
            &BTreeMap::new(),
            ConfirmationPresentationRequest::DeclaredOnly,
        );
        assert_eq!(declared_only.invocation_message, "Running Console");
        assert_eq!(
            declared_only.confirmation.unwrap().branch,
            ConfirmationBranch::Default
        );

        spec.confirmation = None;
        let effect_default = spec.prepare_unvalidated_presentation(
            &defaults,
            "console",
            &BTreeMap::new(),
            ConfirmationPresentationRequest::DeclaredOrSurfaceDefault,
        );
        let confirmation = effect_default.confirmation.unwrap();
        assert_eq!(confirmation.branch, ConfirmationBranch::SurfaceDefault);
        assert_eq!(confirmation.title, "Confirmation required");
        assert_eq!(confirmation.message, "Run Console?");
        assert_eq!(defaults.invocation_message(), "Running Console");
        assert_eq!(defaults.confirmation_title(), "Confirmation required");
        assert_eq!(defaults.confirmation_message(), "Run Console?");
    }
}
