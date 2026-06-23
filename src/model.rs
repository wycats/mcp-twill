use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{FrameworkError, Result};

fn default_output_format() -> OutputFormat {
    OutputFormat::Structured
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ArgType {
    String,
    Path,
    Json,
    Bool,
    Number,
}

impl ArgType {
    pub fn expected_name(&self) -> &'static str {
        match self {
            ArgType::String => "a string",
            ArgType::Path => "a path string",
            ArgType::Json => "JSON",
            ArgType::Bool => "a boolean",
            ArgType::Number => "a number",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArgSpec {
    pub name: String,
    pub value_type: ArgType,
    pub required: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default)]
    pub repeated: bool,
}

impl ArgSpec {
    pub fn string(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::String,
            required: true,
            summary: summary.into(),
            workspace: None,
            repeated: false,
        }
    }

    pub fn path(
        name: impl Into<String>,
        summary: impl Into<String>,
        workspace: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value_type: ArgType::Path,
            required: true,
            summary: summary.into(),
            workspace: Some(workspace.into()),
            repeated: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PermissionEffect {
    Read,
    Write,
    Delete,
    Exec,
    Network,
    Custom(String),
}

impl PermissionEffect {
    pub fn as_label(&self) -> String {
        match self {
            PermissionEffect::Read => "read".to_string(),
            PermissionEffect::Write => "write".to_string(),
            PermissionEffect::Delete => "delete".to_string(),
            PermissionEffect::Exec => "exec".to_string(),
            PermissionEffect::Network => "network".to_string(),
            PermissionEffect::Custom(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSpec {
    pub effect: PermissionEffect,
    pub scope: String,
    pub description: String,
}

impl PermissionSpec {
    pub fn new(
        effect: PermissionEffect,
        scope: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            effect,
            scope: scope.into(),
            description: description.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDecl {
    pub name: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl WorkspaceDecl {
    pub fn new(name: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            uri: uri.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn contains_path_value(&self, value: &str) -> bool {
        if self.uri.starts_with("file://") || value.starts_with("file://") {
            let root = normalize_file_uri(&self.uri);
            let candidate = normalize_file_uri(value);
            return path_has_prefix(&candidate, &root);
        }

        path_has_prefix(&normalize_path(value), &normalize_path(&self.uri))
    }
}

fn normalize_file_uri(value: &str) -> String {
    let stripped = value
        .strip_prefix("file:///")
        .or_else(|| value.strip_prefix("file://"));
    match stripped {
        Some(path) => normalize_path(path),
        None => normalize_path(value),
    }
}

fn normalize_path(value: &str) -> String {
    let replaced = value.replace('\\', "/");
    let absolute = replaced.starts_with('/');
    let (prefix, rest) = if replaced.len() >= 2 && replaced.as_bytes()[1] == b':' {
        (replaced[..2].to_string(), &replaced[2..])
    } else {
        (String::new(), replaced.as_str())
    };

    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }

    let mut normalized = String::new();
    if !prefix.is_empty() {
        normalized.push_str(&prefix);
        if !parts.is_empty() {
            normalized.push('/');
        }
    } else if absolute {
        normalized.push('/');
    }
    normalized.push_str(&parts.join("/"));
    normalized.trim_end_matches('/').to_ascii_lowercase()
}

fn path_has_prefix(candidate: &str, root: &str) -> bool {
    candidate == root || candidate.starts_with(&format!("{root}/"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandExample {
    pub command: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Value>,
}

impl CommandExample {
    pub fn new(command: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            summary: summary.into(),
            args: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub path: Vec<String>,
    pub summary: String,
    pub description: String,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
    #[serde(default)]
    pub permissions: Vec<PermissionSpec>,
    #[serde(default)]
    pub examples: Vec<CommandExample>,
}

impl CommandSpec {
    pub fn new(
        path: impl IntoIterator<Item = impl Into<String>>,
        summary: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
            summary: summary.into(),
            description: description.into(),
            args: Vec::new(),
            permissions: Vec::new(),
            examples: Vec::new(),
        }
    }

    pub fn name(&self) -> String {
        self.path.join(" ")
    }

    pub fn with_arg(mut self, arg: ArgSpec) -> Self {
        self.args.push(arg);
        self
    }

    pub fn with_permission(mut self, permission: PermissionSpec) -> Self {
        self.permissions.push(permission);
        self
    }

    pub fn with_example(mut self, example: CommandExample) -> Self {
        self.examples.push(example);
        self
    }

    pub fn arg(&self, name: &str) -> Option<&ArgSpec> {
        self.args.iter().find(|arg| arg.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    Structured,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResponseProfile {
    Text,
    Structured,
    CompactStructured,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputSpec {
    #[serde(default = "default_output_format")]
    pub format: OutputFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ResponseProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, rename = "maxBytes", skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            format: OutputFormat::Structured,
            profile: None,
            limit: None,
            fields: None,
            cursor: None,
            max_bytes: Some(32 * 1024),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StdinSpec {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub command: String,
    #[serde(default)]
    pub args: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<StdinSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputSpec>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundArg {
    pub name: String,
    pub value_type: ArgType,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InvocationToken {
    Literal { value: String },
    Placeholder { name: String, value: Value },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvocationPlan {
    pub operation_id: String,
    pub command_path: Vec<String>,
    pub raw_command: String,
    pub catalog_hash: String,
    pub effect: crate::EffectSpec,
    pub lane: crate::EffectLane,
    pub tokens: Vec<InvocationToken>,
    pub bound_args: BTreeMap<String, BoundArg>,
    pub permissions: Vec<PermissionSpec>,
    pub workspaces: Vec<WorkspaceDecl>,
    pub output: OutputSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stderr: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl CommandOutput {
    pub fn structured(value: Value) -> Self {
        Self {
            text: None,
            structured: Some(value),
            stderr: Vec::new(),
            next_cursor: None,
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self {
            text: Some(value.into()),
            structured: None,
            stderr: Vec::new(),
            next_cursor: None,
        }
    }

    pub fn apply_output_spec(mut self, spec: &OutputSpec) -> Self {
        if let Some(value) = self.structured.take() {
            self.structured = Some(limit_structured(
                shape_structured(value, spec),
                spec.max_bytes,
            ));
        }

        if let Some(text) = self.text.take() {
            self.text = Some(limit_text(text, spec.max_bytes));
        }

        self
    }
}

fn shape_structured(value: Value, spec: &OutputSpec) -> Value {
    let limited = match (value, spec.limit) {
        (Value::Array(items), Some(limit)) => Value::Array(items.into_iter().take(limit).collect()),
        (value, _) => value,
    };

    match spec.fields.as_ref() {
        Some(fields) => select_fields(limited, fields),
        None => limited,
    }
}

fn select_fields(value: Value, fields: &[String]) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| select_fields(item, fields))
                .collect(),
        ),
        Value::Object(map) => {
            let mut selected = serde_json::Map::new();
            for field in fields {
                if let Some(value) = map.get(field) {
                    selected.insert(field.clone(), value.clone());
                }
            }
            Value::Object(selected)
        }
        other => other,
    }
}

fn limit_structured(value: Value, max_bytes: Option<usize>) -> Value {
    let Some(max_bytes) = max_bytes else {
        return value;
    };
    let Ok(bytes) = serde_json::to_vec(&value) else {
        return value;
    };
    if bytes.len() <= max_bytes {
        return value;
    }

    let preview = String::from_utf8_lossy(&bytes).to_string();
    json!({
        "truncated": true,
        "maxBytes": max_bytes,
        "actualBytes": bytes.len(),
        "preview": limit_text(preview, Some(max_bytes)),
    })
}

fn limit_text(text: String, max_bytes: Option<usize>) -> String {
    let Some(max_bytes) = max_bytes else {
        return text;
    };
    if text.len() <= max_bytes {
        return text;
    }

    let marker = "...[truncated]";
    let target = max_bytes.saturating_sub(marker.len());
    let mut end = if target == 0 { max_bytes } else { target };
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if target == 0 {
        text[..end].to_string()
    } else {
        format!("{}{}", &text[..end], marker)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunResponse {
    pub plan: InvocationPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandContext {
    pub plan: InvocationPlan,
    pub stdin: Option<StdinSpec>,
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    allowed: BTreeSet<PermissionEffect>,
}

impl PermissionPolicy {
    pub fn allow_all() -> Self {
        Self {
            allowed: [
                PermissionEffect::Read,
                PermissionEffect::Write,
                PermissionEffect::Delete,
                PermissionEffect::Exec,
                PermissionEffect::Network,
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn read_only() -> Self {
        Self {
            allowed: [PermissionEffect::Read].into_iter().collect(),
        }
    }

    pub fn allows(&self, permission: &PermissionSpec) -> bool {
        self.allowed.contains(&permission.effect)
            || matches!(
                &permission.effect,
                PermissionEffect::Custom(effect) if self.allowed.contains(&PermissionEffect::Custom(effect.clone()))
            )
    }

    pub fn check(&self, permissions: &[PermissionSpec]) -> Result<()> {
        for permission in permissions {
            if !self.allows(permission) {
                return Err(FrameworkError::PermissionDenied {
                    effect: permission.effect.as_label(),
                    scope: permission.scope.clone(),
                });
            }
        }
        Ok(())
    }
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::allow_all()
    }
}

pub fn value_matches_type(name: &str, value: &Value, value_type: &ArgType) -> Result<()> {
    let valid = match value_type {
        ArgType::String | ArgType::Path => value.is_string(),
        ArgType::Json => true,
        ArgType::Bool => value.is_boolean(),
        ArgType::Number => value.is_number(),
    };
    if valid {
        Ok(())
    } else {
        Err(FrameworkError::InvalidArgumentType(
            name.to_string(),
            value_type.expected_name(),
        ))
    }
}

pub fn structured_error(error: &FrameworkError) -> Value {
    json!({
        "error": error.to_string()
    })
}
