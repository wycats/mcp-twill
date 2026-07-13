use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;

use crate::{FrameworkError, Result};

/// Canonical MCP metadata key for host-supplied conversation identity.
pub const CONVERSATION_IDENTITY_META_KEY: &str = "io.github.wycats.mcp-twill/conversation-identity";

/// A validated host conversation identity. The complete tuple is the
/// identity; ids from different issuers never alias.
#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ConversationIdentity {
    version: u32,
    issuer: String,
    id: String,
}

impl ConversationIdentity {
    pub const VERSION: u32 = 1;

    pub fn new(issuer: impl Into<String>, id: impl Into<String>) -> Result<Self> {
        let issuer = issuer.into();
        let id = id.into();
        validate_issuer(&issuer).map_err(invalid_canonical)?;
        if id.is_empty() {
            return Err(invalid_canonical(IdentityProblem::new(
                Some("id"),
                "empty_id",
                Some("a non-empty opaque string"),
            )));
        }
        Ok(Self {
            version: Self::VERSION,
            issuer,
            id,
        })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Debug for ConversationIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConversationIdentity")
            .field("version", &self.version)
            .field("issuer", &"<redacted>")
            .field("id", &"<redacted>")
            .finish()
    }
}

impl<'de> Deserialize<'de> for ConversationIdentity {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        parse_canonical_identity(&value).map_err(de::Error::custom)
    }
}

/// Private, non-serializing facts supplied by a host for one invocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvocationContext {
    conversation_identity: Option<ConversationIdentity>,
    host_workspace_roots: Option<mcp_workspace_resolver::HostWorkspaceRootsObservation>,
}

impl InvocationContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_conversation_identity(mut self, identity: ConversationIdentity) -> Self {
        self.conversation_identity = Some(identity);
        self
    }

    pub fn conversation_identity(&self) -> Option<&ConversationIdentity> {
        self.conversation_identity.as_ref()
    }

    pub fn with_host_workspace_roots(
        mut self,
        roots: mcp_workspace_resolver::HostWorkspaceRootsObservation,
    ) -> Self {
        self.host_workspace_roots = Some(roots);
        self
    }

    pub(crate) fn host_workspace_roots(
        &self,
    ) -> Option<&mcp_workspace_resolver::HostWorkspaceRootsObservation> {
        self.host_workspace_roots.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentityProblem {
    field: Option<String>,
    reason: &'static str,
    expected: Option<String>,
}

impl IdentityProblem {
    fn new(
        field: Option<impl Into<String>>,
        reason: &'static str,
        expected: Option<impl Into<String>>,
    ) -> Self {
        Self {
            field: field.map(Into::into),
            reason,
            expected: expected.map(Into::into),
        }
    }
}

fn invalid_canonical(problem: IdentityProblem) -> FrameworkError {
    FrameworkError::InvalidConversationIdentity {
        observation_source: "canonical".to_string(),
        key: CONVERSATION_IDENTITY_META_KEY.to_string(),
        field: problem.field,
        reason: problem.reason.to_string(),
        expected: problem.expected,
    }
}

pub(crate) fn parse_canonical_identity(value: &Value) -> Result<ConversationIdentity> {
    parse_identity_value(value).map_err(invalid_canonical)
}

pub(crate) fn codex_thread_identity(value: &Value) -> Result<ConversationIdentity> {
    let id = value.as_str().filter(|id| !id.is_empty()).ok_or_else(|| {
        FrameworkError::InvalidConversationIdentity {
            observation_source: "codexThreadId".to_string(),
            key: "threadId".to_string(),
            field: None,
            reason: "expected_non_empty_string".to_string(),
            expected: Some("a non-empty string".to_string()),
        }
    })?;
    ConversationIdentity::new("com.openai.codex", id)
}

fn parse_identity_value(
    value: &Value,
) -> std::result::Result<ConversationIdentity, IdentityProblem> {
    let object = value.as_object().ok_or_else(|| {
        IdentityProblem::new(
            None::<String>,
            "expected_object",
            Some("an object with version, issuer, and id"),
        )
    })?;

    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "version" | "issuer" | "id"))
    {
        return Err(IdentityProblem::new(
            None::<String>,
            "unknown_field",
            Some("version, issuer, or id"),
        ));
    }

    for field in ["version", "issuer", "id"] {
        if !object.contains_key(field) {
            return Err(IdentityProblem::new(
                Some(field),
                "missing_field",
                Some(format!("the `{field}` field")),
            ));
        }
    }

    let version = object["version"]
        .as_u64()
        .ok_or_else(|| IdentityProblem::new(Some("version"), "unsupported_version", Some("1")))?;
    if version != u64::from(ConversationIdentity::VERSION) {
        return Err(IdentityProblem::new(
            Some("version"),
            "unsupported_version",
            Some("1"),
        ));
    }

    let issuer = object["issuer"].as_str().ok_or_else(|| {
        IdentityProblem::new(
            Some("issuer"),
            "invalid_issuer",
            Some("a lowercase reverse-DNS name"),
        )
    })?;
    validate_issuer(issuer)?;

    let id = object["id"].as_str().ok_or_else(|| {
        IdentityProblem::new(
            Some("id"),
            "expected_non_empty_string",
            Some("a non-empty opaque string"),
        )
    })?;
    if id.is_empty() {
        return Err(IdentityProblem::new(
            Some("id"),
            "empty_id",
            Some("a non-empty opaque string"),
        ));
    }

    Ok(ConversationIdentity {
        version: ConversationIdentity::VERSION,
        issuer: issuer.to_string(),
        id: id.to_string(),
    })
}

fn validate_issuer(issuer: &str) -> std::result::Result<(), IdentityProblem> {
    let labels = issuer.split('.').collect::<Vec<_>>();
    let valid = labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        });
    if valid {
        Ok(())
    } else {
        Err(IdentityProblem::new(
            Some("issuer"),
            "invalid_issuer",
            Some("a lowercase reverse-DNS name with at least two labels"),
        ))
    }
}
