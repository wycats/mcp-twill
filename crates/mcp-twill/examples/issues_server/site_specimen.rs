use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::{fmt::Write as _, result::Result as StdResult};

use mcp_twill::{
    ApplicationResultContract, ApplicationSuccess, ArgType, ArgumentRendering, ArgumentSchemaUse,
    CommandContext, CommandRegistry, ConfirmationMessage, ConfirmationPresentation,
    ConfirmationSegment, DynamicApplicationResult, OperationSpec, PermissionEffect, Result,
    TaskSupportSpec, arg,
};
use serde_json::{Map, Value, json};

const INLINE_JSON_OBJECT_WIDTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleRule {
    Unconstrained,
    NonEmpty,
}

impl TitleRule {
    pub fn id(self) -> &'static str {
        match self {
            Self::Unconstrained => "unconstrained",
            Self::NonEmpty => "nonEmpty",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Local,
    Remote,
}

impl Destination {
    pub fn id(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationKind {
    Generic,
    TitleInterpolated,
}

impl ConfirmationKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::TitleInterpolated => "titleInterpolated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateContext {
    None,
    ConversationIdentity,
}

impl PrivateContext {
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ConversationIdentity => "conversationIdentity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecimenConfig {
    pub title_rule: TitleRule,
    pub destination: Destination,
    pub confirmation: ConfirmationKind,
    pub private_context: PrivateContext,
}

impl Default for SpecimenConfig {
    fn default() -> Self {
        Self {
            title_rule: TitleRule::Unconstrained,
            destination: Destination::Local,
            confirmation: ConfirmationKind::TitleInterpolated,
            private_context: PrivateContext::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationCodeRange {
    pub fact_id: &'static str,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedDeclaration {
    pub text: String,
    pub fact_ranges: Vec<DeclarationCodeRange>,
}

#[derive(Debug, Default)]
pub struct HandlerObservation {
    identity_observed: AtomicBool,
    plan: Mutex<Option<Value>>,
}

impl HandlerObservation {
    pub fn identity_observed(&self) -> bool {
        self.identity_observed.load(Ordering::SeqCst)
    }

    pub fn plan(&self) -> Option<Value> {
        self.plan.lock().expect("site specimen plan").clone()
    }
}

async fn create_issue(context: CommandContext) -> DynamicApplicationResult {
    let title = context.plan.bound_args["title"].value.clone();
    let body = context.plan.bound_args["body"].value.clone();
    Ok(ApplicationSuccess::value(json!({
        "id": 1,
        "title": title,
        "body": body,
        "status": "open"
    })))
}

pub fn registry(
    config: SpecimenConfig,
    observation: Arc<HandlerObservation>,
) -> Result<CommandRegistry> {
    CommandRegistry::build(
        "issues",
        "A deterministic issue tracker specimen for A Command, Woven.",
        |server| {
            server.preamble(
                "The command catalog is authoritative; generated surfaces must agree with it.",
            );
            server.command("issues create", |command| {
                command
                    .summary("Create an issue")
                    .description("Creates a new issue from typed title and body arguments.")
                    .use_when("reporting a single new problem")
                    .arg({
                        let title = arg::string("title").summary("Issue title");
                        match config.title_rule {
                            TitleRule::Unconstrained => title,
                            TitleRule::NonEmpty => title.with_inline_schema(json!({
                                "type": "string",
                                "minLength": 1
                            })),
                        }
                    })
                    .arg(arg::string("body").summary("Issue body"))
                    .write("issues", "Creates a new issue record");
                if matches!(config.destination, Destination::Remote) {
                    command.network("tracker.example", "Sends the issue to the remote tracker");
                }
                command
                    .invocation_message("Creating an issue")
                    .confirmation(match config.confirmation {
                        ConfirmationKind::Generic => ConfirmationPresentation::new(
                            ConfirmationMessage::new("Create issue?")
                                .text("Create this issue?"),
                        ),
                        ConfirmationKind::TitleInterpolated => ConfirmationPresentation::new(
                            ConfirmationMessage::new("Create issue?")
                                .text("Create issue ")
                                .argument(
                                    "title",
                                    ArgumentRendering::JsonString,
                                    "(missing title)",
                                )
                                .text("?"),
                        ),
                    })
                    .idempotent()
                    .task_support(TaskSupportSpec::Optional);
                if matches!(
                    config.private_context,
                    PrivateContext::ConversationIdentity
                ) {
                    command.uses_conversation_identity();
                }
                command
                    .example_with_args(
                        "issues create --title $args.title --body $args.body",
                        "Create an issue with typed title and body values",
                        json!({
                            "title": "Crash on launch",
                            "body": "The app exits after the splash screen."
                        }),
                    )
                    .result_contract(ApplicationResultContract::new(json!({
                        "type": "object",
                        "properties": {
                            "id": { "type": "integer" },
                            "title": { "type": "string" },
                            "body": { "type": "string" },
                            "status": { "type": "string" }
                        },
                        "required": ["id", "title", "body", "status"],
                        "additionalProperties": false
                    })))
                    .handle_dynamic(move |context: CommandContext| {
                        let observation = observation.clone();
                        async move {
                            observation.identity_observed.store(
                                context.conversation_identity().is_some(),
                                Ordering::SeqCst,
                            );
                            *observation.plan.lock().expect("site specimen plan") =
                                Some(serde_json::to_value(&context.plan).expect("plan serializes"));
                            create_issue(context).await
                        }
                    });
            });
        },
    )
}

pub fn declaration(operation: &OperationSpec) -> StdResult<RenderedDeclaration, String> {
    let use_when = operation
        .use_when
        .as_deref()
        .ok_or_else(|| "site specimen operation is missing use_when guidance".to_string())?;
    let presentation = operation
        .presentation
        .as_ref()
        .ok_or_else(|| "site specimen operation is missing presentation".to_string())?;
    let invocation_message = presentation
        .invocation_message
        .as_deref()
        .ok_or_else(|| "site specimen operation is missing invocation_message".to_string())?;
    let confirmation = presentation
        .confirmation
        .as_ref()
        .ok_or_else(|| "site specimen operation is missing confirmation".to_string())?;
    if !confirmation.cases.is_empty() {
        return Err("site specimen declaration renderer does not support confirmation cases".into());
    }
    let application = operation
        .output
        .application
        .as_ref()
        .ok_or_else(|| "site specimen operation is missing its application result contract".to_string())?;
    if !application.errors.is_empty() {
        return Err("site specimen declaration renderer requires an explicit error renderer".into());
    }

    let command_name = operation.path.join(" ");
    let mut output = String::new();
    let mut fact_ranges = Vec::new();
    writeln!(
        output,
        "server.command({}, |command| {{",
        rust_string(&command_name)
    )
    .expect("writing to String cannot fail");
    writeln!(output, "    command").expect("writing to String cannot fail");
    writeln!(
        output,
        "        .summary({})",
        rust_string(&operation.summary)
    )
    .expect("writing to String cannot fail");
    writeln!(
        output,
        "        .description({})",
        rust_string(&operation.description)
    )
    .expect("writing to String cannot fail");
    writeln!(output, "        .use_when({})", rust_string(use_when))
        .expect("writing to String cannot fail");

    for argument in &operation.args {
        if argument.value_type != ArgType::String
            || !argument.required
            || argument.repeated
            || argument.workspace.is_some()
            || !argument.requires_arguments.is_empty()
        {
            return Err(format!(
                "site specimen declaration renderer cannot faithfully render argument `{}`",
                argument.name
            ));
        }
        let fact_start = (argument.name == "title").then(|| next_line(&output));
        writeln!(output, "        .arg(").expect("writing to String cannot fail");
        writeln!(
            output,
            "            arg::string({})",
            rust_string(&argument.name)
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "                .summary({})",
            rust_string(&argument.summary)
        )
        .expect("writing to String cannot fail");
        match &argument.schema {
            None => {}
            Some(ArgumentSchemaUse::Inline { schema }) => {
                writeln!(
                    output,
                    "                .with_inline_schema(json!("
                )
                .expect("writing to String cannot fail");
                write_json(&mut output, schema, 20)?;
                writeln!(output, "                ))").expect("writing to String cannot fail");
            }
            Some(ArgumentSchemaUse::Named { .. }) => {
                return Err(format!(
                    "site specimen declaration renderer cannot faithfully render named schema argument `{}`",
                    argument.name
                ));
            }
        }
        writeln!(output, "        )").expect("writing to String cannot fail");
        if let Some(start_line) = fact_start {
            fact_ranges.push(DeclarationCodeRange {
                fact_id: "fact.titleRule",
                start_line,
                end_line: current_line(&output),
            });
        }
    }

    let effect_start = next_line(&output);
    for permission in &operation.permissions {
        let method = match permission.effect {
            PermissionEffect::Write => "write",
            PermissionEffect::Network => "network",
            _ => {
                return Err(format!(
                    "site specimen declaration renderer cannot faithfully render `{}` permission",
                    permission.effect.as_label()
                ));
            }
        };
        writeln!(
            output,
            "        .{}({}, {})",
            method,
            rust_string(&permission.scope),
            rust_string(&permission.description)
        )
        .expect("writing to String cannot fail");
    }
    if current_line(&output) >= effect_start {
        fact_ranges.push(DeclarationCodeRange {
            fact_id: "fact.destination",
            start_line: effect_start,
            end_line: current_line(&output),
        });
    }

    writeln!(
        output,
        "        .invocation_message({})",
        rust_string(invocation_message)
    )
    .expect("writing to String cannot fail");
    let confirmation_start = next_line(&output);
    writeln!(
        output,
        "        .confirmation(ConfirmationPresentation::new("
    )
    .expect("writing to String cannot fail");
    writeln!(
        output,
        "            ConfirmationMessage::new({})",
        rust_string(&confirmation.default.title)
    )
    .expect("writing to String cannot fail");
    for segment in &confirmation.default.body {
        match segment {
            ConfirmationSegment::Text(text) => {
                writeln!(output, "                .text({})", rust_string(text))
                    .expect("writing to String cannot fail");
            }
            ConfirmationSegment::Argument {
                argument,
                rendering,
                fallback,
            } => {
                writeln!(
                    output,
                    "                .argument({}, {}, {})",
                    rust_string(argument),
                    argument_rendering_source(*rendering),
                    rust_string(fallback)
                )
                .expect("writing to String cannot fail");
            }
        }
    }
    writeln!(output, "        ))").expect("writing to String cannot fail");
    fact_ranges.push(DeclarationCodeRange {
        fact_id: "fact.confirmation",
        start_line: confirmation_start,
        end_line: current_line(&output),
    });

    if operation.uses_conversation_identity {
        let start_line = next_line(&output);
        writeln!(output, "        .uses_conversation_identity()")
            .expect("writing to String cannot fail");
        fact_ranges.push(DeclarationCodeRange {
            fact_id: "fact.privateContext",
            start_line,
            end_line: current_line(&output),
        });
    }
    if operation.idempotent {
        writeln!(output, "        .idempotent()").expect("writing to String cannot fail");
    }
    writeln!(
        output,
        "        .task_support({})",
        task_support_source(&operation.task_support)
    )
    .expect("writing to String cannot fail");

    for example in &operation.examples {
        writeln!(output, "        .example_with_args(")
            .expect("writing to String cannot fail");
        writeln!(
            output,
            "            {},",
            rust_string(&example.command)
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "            {},",
            rust_string(&example.summary)
        )
        .expect("writing to String cannot fail");
        writeln!(output, "            json!(").expect("writing to String cannot fail");
        write_json(
            &mut output,
            &serde_json::to_value(&example.args).map_err(|error| error.to_string())?,
            16,
        )?;
        writeln!(output, "            ),").expect("writing to String cannot fail");
        writeln!(output, "        )").expect("writing to String cannot fail");
    }

    writeln!(output, "        // Application errors: none.")
        .expect("writing to String cannot fail");
    writeln!(
        output,
        "        .result_contract(ApplicationResultContract::new(json!("
    )
    .expect("writing to String cannot fail");
    write_json(&mut output, &application.success_schema, 12)?;
    writeln!(output, "        )))").expect("writing to String cannot fail");
    writeln!(output, "        .handle_dynamic(create_issue);")
        .expect("writing to String cannot fail");
    write!(output, "}});").expect("writing to String cannot fail");
    Ok(RenderedDeclaration {
        text: output,
        fact_ranges,
    })
}

fn next_line(output: &str) -> u32 {
    current_line(output) + 1
}

fn current_line(output: &str) -> u32 {
    output.lines().count() as u32
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn argument_rendering_source(rendering: ArgumentRendering) -> &'static str {
    match rendering {
        ArgumentRendering::Plain => "ArgumentRendering::Plain",
        ArgumentRendering::JsonString => "ArgumentRendering::JsonString",
        ArgumentRendering::TrimmedJsonString => "ArgumentRendering::TrimmedJsonString",
    }
}

fn task_support_source(task_support: &TaskSupportSpec) -> &'static str {
    match task_support {
        TaskSupportSpec::Forbidden => "TaskSupportSpec::Forbidden",
        TaskSupportSpec::Optional => "TaskSupportSpec::Optional",
        TaskSupportSpec::Required => "TaskSupportSpec::Required",
    }
}

fn write_json(output: &mut String, value: &Value, indentation: usize) -> StdResult<(), String> {
    let rendered = render_json(value, indentation)?;
    for line in rendered.lines() {
        writeln!(output, "{line}").expect("writing to String cannot fail");
    }
    Ok(())
}

/// Renders JSON embedded in the site-only Rust declaration.
///
/// This intentionally favors the way a reader scans a schema over serde_json's
/// generic pretty-printer: primitive arrays and small leaf objects stay on one
/// line, while structural objects remain multiline. JSON Schema vocabulary is
/// presented in semantic order without changing the underlying value.
pub fn render_json(value: &Value, indentation: usize) -> StdResult<String, String> {
    Ok(render_json_lines(value, indentation, None)?.join("\n"))
}

fn render_json_lines(
    value: &Value,
    indentation: usize,
    preferred_keys: Option<&[String]>,
) -> StdResult<Vec<String>, String> {
    if let Some(compact) = compact_json(value, preferred_keys)? {
        return Ok(vec![format!("{}{compact}", " ".repeat(indentation))]);
    }

    let padding = " ".repeat(indentation);
    let child_indentation = indentation + 2;
    match value {
        Value::Array(values) => {
            let mut lines = vec![format!("{padding}[")];
            for (index, value) in values.iter().enumerate() {
                let mut child = render_json_lines(value, child_indentation, None)?;
                if index + 1 != values.len() {
                    child
                        .last_mut()
                        .expect("a rendered JSON value always has a line")
                        .push(',');
                }
                lines.extend(child);
            }
            lines.push(format!("{padding}]"));
            Ok(lines)
        }
        Value::Object(object) => {
            let keys = ordered_keys(object, preferred_keys);
            let property_order = required_property_order(object);
            let mut lines = vec![format!("{padding}{{")];
            for (index, key) in keys.iter().enumerate() {
                let child_preference =
                    (key.as_str() == "properties").then_some(property_order.as_slice());
                let mut child = render_json_lines(
                    object
                        .get(*key)
                        .expect("ordered JSON key must remain in its object"),
                    child_indentation,
                    child_preference,
                )?;
                let child_padding = " ".repeat(child_indentation);
                let first = child
                    .first_mut()
                    .expect("a rendered JSON value always has a line");
                let value_head = first
                    .strip_prefix(&child_padding)
                    .expect("child JSON indentation is deterministic")
                    .to_string();
                *first = format!(
                    "{child_padding}{}: {value_head}",
                    serde_json::to_string(*key).map_err(|error| error.to_string())?
                );
                if index + 1 != keys.len() {
                    child
                        .last_mut()
                        .expect("a rendered JSON value always has a line")
                        .push(',');
                }
                lines.extend(child);
            }
            lines.push(format!("{padding}}}"));
            Ok(lines)
        }
        _ => unreachable!("non-container JSON values always have a compact rendering"),
    }
}

fn compact_json(
    value: &Value,
    preferred_keys: Option<&[String]>,
) -> StdResult<Option<String>, String> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_string(value)
                .map(Some)
                .map_err(|error| error.to_string())
        }
        Value::Array(values) if values.iter().all(is_json_primitive) => {
            let values = values
                .iter()
                .map(serde_json::to_string)
                .collect::<StdResult<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            Ok(Some(format!("[{}]", values.join(", "))))
        }
        Value::Object(object) if object.is_empty() => Ok(Some("{}".to_string())),
        Value::Object(object) if object.values().all(is_json_primitive) => {
            let entries = ordered_keys(object, preferred_keys)
                .into_iter()
                .map(|key| {
                    Ok(format!(
                        "{}: {}",
                        serde_json::to_string(key).map_err(|error| error.to_string())?,
                        serde_json::to_string(
                            object
                                .get(key)
                                .expect("ordered JSON key must remain in its object")
                        )
                        .map_err(|error| error.to_string())?
                    ))
                })
                .collect::<StdResult<Vec<_>, String>>()?;
            let rendered = format!("{{ {} }}", entries.join(", "));
            Ok((rendered.len() <= INLINE_JSON_OBJECT_WIDTH).then_some(rendered))
        }
        _ => Ok(None),
    }
}

fn is_json_primitive(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn required_property_order(object: &Map<String, Value>) -> Vec<String> {
    object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn ordered_keys<'a>(
    object: &'a Map<String, Value>,
    preferred_keys: Option<&[String]>,
) -> Vec<&'a String> {
    let mut keys = object.keys().collect::<Vec<_>>();
    keys.sort_by_key(|key| {
        let preferred_position = preferred_keys
            .and_then(|preferred| preferred.iter().position(|candidate| candidate == *key));
        (
            preferred_position.is_none(),
            preferred_position.unwrap_or(usize::MAX),
            json_schema_key_rank(key),
            key.as_str(),
        )
    });
    keys
}

fn json_schema_key_rank(key: &str) -> u8 {
    match key {
        "$schema" => 0,
        "$id" => 1,
        "$ref" => 2,
        "type" => 3,
        "required" => 4,
        "properties" => 5,
        "items" => 6,
        "enum" | "const" => 7,
        "format" => 8,
        "minLength" | "minimum" | "minItems" => 9,
        "maxLength" | "maximum" | "maxItems" => 10,
        "additionalProperties" => 100,
        _ => 50,
    }
}
