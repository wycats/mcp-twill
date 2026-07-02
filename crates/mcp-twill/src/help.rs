use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum HelpTopic {
    Server,
    Usage,
    Arguments,
    Permissions,
    Examples,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum HelpDetail {
    Summary,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelpRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<HelpTopic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<HelpDetail>,
}

impl Default for HelpRequest {
    fn default() -> Self {
        Self {
            command: None,
            topic: Some(HelpTopic::Server),
            detail: Some(HelpDetail::Summary),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelpResult {
    pub title: String,
    pub text: String,
    pub structured: Value,
}
