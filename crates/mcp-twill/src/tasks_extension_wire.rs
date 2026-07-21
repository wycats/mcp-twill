use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
struct CallToolParams {
    #[serde(rename = "_meta")]
    meta: Value,
    name: String,
    #[serde(default)]
    arguments: Map<String, Value>,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskParams {
    #[serde(rename = "_meta")]
    meta: Value,
    task_id: String,
    #[serde(default)]
    input_responses: Option<Map<String, Value>>,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

pub(crate) fn validate(method: &str, params: &str) -> serde_json::Result<()> {
    match method {
        "tools/call" => {
            let parsed: CallToolParams = serde_json::from_str(params)?;
            let _ = (parsed.meta, parsed.name, parsed.arguments, parsed.unknown);
        }
        "tasks/get" | "tasks/update" | "tasks/cancel" => {
            let parsed: TaskParams = serde_json::from_str(params)?;
            let _ = (
                parsed.meta,
                parsed.task_id,
                parsed.input_responses,
                parsed.unknown,
            );
        }
        _ => {}
    }
    Ok(())
}
