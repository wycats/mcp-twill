use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ArgSpec, CommandExample, CommandSpec, PermissionEffect, PermissionSpec, WorkspaceDecl,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Stability {
    Draft,
    Stable,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectValue {
    pub summary: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerSpec {
    pub name: String,
    pub summary: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub stability: Stability,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<ProjectValue>,
}

impl ServerSpec {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        let name = name.into();
        let description = description.into();
        Self {
            summary: description.clone(),
            name,
            description,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            stability: Stability::Draft,
            values: vec![
                ProjectValue {
                    summary: "Command strings are typed templates".to_string(),
                    description: "The command string selects an operation and binds typed values from structured arguments.".to_string(),
                },
                ProjectValue {
                    summary: "Shell syntax is modeled as framework features".to_string(),
                    description: "Composition, filtering, redirection, and expansion-like behavior are represented as typed framework features when supported.".to_string(),
                },
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceSpec {
    pub path: Vec<String>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub stability: Stability,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum EffectSpec {
    Pure,
    Read,
    Write,
    Delete,
    Exec,
    Network,
    Composite(Vec<EffectSpec>),
    Custom(String),
}

impl EffectSpec {
    pub fn from_permissions(permissions: &[PermissionSpec]) -> Self {
        if permissions.is_empty() {
            return Self::Pure;
        }

        let mut effects = permissions
            .iter()
            .map(|permission| match &permission.effect {
                PermissionEffect::Read => Self::Read,
                PermissionEffect::Write => Self::Write,
                PermissionEffect::Delete => Self::Delete,
                PermissionEffect::Exec => Self::Exec,
                PermissionEffect::Network => Self::Network,
                PermissionEffect::Custom(value) => Self::Custom(value.clone()),
            })
            .collect::<Vec<_>>();
        effects.sort();
        effects.dedup();

        if effects.len() == 1 {
            effects.remove(0)
        } else {
            Self::Composite(effects)
        }
    }

    pub fn lane(&self) -> EffectLane {
        match self {
            EffectSpec::Pure | EffectSpec::Read => EffectLane::Primary,
            EffectSpec::Write => EffectLane::Write,
            EffectSpec::Delete => EffectLane::Delete,
            // Custom effects are rejected at server construction. If one gets
            // here anyway, treat it as the most restrictive lane.
            EffectSpec::Exec | EffectSpec::Custom(_) => EffectLane::Exec,
            EffectSpec::Network => EffectLane::Network,
            EffectSpec::Composite(effects) => effects
                .iter()
                .map(EffectSpec::lane)
                .max_by_key(EffectLane::rank)
                .unwrap_or(EffectLane::Primary),
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum EffectLane {
    Primary,
    Write,
    Delete,
    Exec,
    Network,
}

impl EffectLane {
    pub fn rank(&self) -> u8 {
        match self {
            EffectLane::Primary => 0,
            EffectLane::Write => 1,
            EffectLane::Delete => 2,
            EffectLane::Exec => 3,
            EffectLane::Network => 4,
        }
    }

    pub fn suffix(&self) -> Option<&'static str> {
        match self {
            EffectLane::Primary => None,
            EffectLane::Write => Some("write"),
            EffectLane::Delete => Some("delete"),
            EffectLane::Exec => Some("exec"),
            EffectLane::Network => Some("network"),
        }
    }

    pub fn tool_name(&self, primary: &str) -> String {
        match self.suffix() {
            Some(suffix) => format!("{primary}-{suffix}"),
            None => primary.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StdinContract {
    pub mime_type: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputContract {
    pub format: crate::OutputFormat,
    pub summary: String,
}

impl Default for OutputContract {
    fn default() -> Self {
        Self {
            format: crate::OutputFormat::Structured,
            summary: "Command handler output shaped by the requested output spec.".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPhaseSpec {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TaskSupportSpec {
    Forbidden,
    Optional,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationSpec {
    pub id: String,
    pub path: Vec<String>,
    pub summary: String,
    pub description: String,
    pub effect: EffectSpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<ArgSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<StdinContract>,
    pub output: OutputContract,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermissionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<CommandExample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress: Vec<ProgressPhaseSpec>,
    pub task_support: TaskSupportSpec,
    pub stability: Stability,
}

impl OperationSpec {
    pub fn from_command_spec(spec: &CommandSpec) -> Self {
        Self {
            id: spec.path.join("."),
            path: spec.path.clone(),
            summary: spec.summary.clone(),
            description: spec.description.clone(),
            effect: EffectSpec::from_permissions(&spec.permissions),
            args: spec.args.clone(),
            stdin: None,
            output: spec.output.clone().unwrap_or_default(),
            permissions: spec.permissions.clone(),
            examples: spec.examples.clone(),
            progress: Vec::new(),
            task_support: TaskSupportSpec::Optional,
            stability: Stability::Draft,
        }
    }

    pub fn name(&self) -> String {
        self.path.join(" ")
    }

    pub fn lane(&self) -> EffectLane {
        self.effect.lane()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogIdentity {
    pub catalog_hash: String,
    pub run_schema_hash: String,
    pub help_schema_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandCatalog {
    pub server: ServerSpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub namespaces: Vec<NamespaceSpec>,
    pub operations: Vec<OperationSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<WorkspaceDecl>,
    pub identity: CatalogIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceKind {
    RunCommand,
    HumanAction,
    ExternalShell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandGuidance {
    pub id: String,
    pub surface: String,
    pub text: String,
    pub kind: GuidanceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolLaneSpec {
    pub tool_name: String,
    pub lane: EffectLane,
    pub allowed_effects: Vec<EffectSpec>,
    pub description: String,
}

pub(crate) fn stable_hash_value(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    stable_hash_bytes(&bytes)
}

pub(crate) fn stable_hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn group_namespaces(operations: &[OperationSpec]) -> Vec<NamespaceSpec> {
    let mut namespaces = BTreeMap::<Vec<String>, NamespaceSpec>::new();
    for operation in operations {
        if operation.path.len() < 2 {
            continue;
        }
        for end in 1..operation.path.len() {
            let path = operation.path[..end].to_vec();
            namespaces
                .entry(path.clone())
                .or_insert_with(|| NamespaceSpec {
                    summary: path.join(" "),
                    path,
                    description: None,
                    stability: Stability::Draft,
                });
        }
    }
    namespaces.into_values().collect()
}
