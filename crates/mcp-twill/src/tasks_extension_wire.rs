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
struct TaskIdParams {
    #[serde(rename = "_meta")]
    meta: Value,
    task_id: String,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskUpdateParams {
    #[serde(rename = "_meta")]
    meta: Value,
    task_id: String,
    input_responses: Map<String, Value>,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

pub(crate) fn validate(method: &str, params: &str) -> serde_json::Result<()> {
    match method {
        "tools/call" => {
            let parsed: CallToolParams = serde_json::from_str(params)?;
            let _ = (parsed.meta, parsed.name, parsed.arguments, parsed.unknown);
        }
        "tasks/get" | "tasks/cancel" => {
            let parsed: TaskIdParams = serde_json::from_str(params)?;
            let _ = (parsed.meta, parsed.task_id, parsed.unknown);
        }
        "tasks/update" => {
            let parsed: TaskUpdateParams = serde_json::from_str(params)?;
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
