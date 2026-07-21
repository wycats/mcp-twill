use serde::Deserialize;
use serde_json::{Map, Value, value::RawValue};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct MetaParams {
    #[serde(rename = "_meta")]
    meta: Value,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

#[derive(Deserialize)]
struct NamedParams {
    #[serde(rename = "_meta")]
    meta: Value,
    name: String,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

#[derive(Deserialize)]
struct ResourceParams {
    #[serde(rename = "_meta")]
    meta: Value,
    uri: String,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

pub(crate) struct Request {
    pub(crate) id: Value,
    pub(crate) has_id: bool,
    pub(crate) method: String,
    pub(crate) params: Map<String, Value>,
    pub(crate) known_method: bool,
}

pub(crate) enum WireError {
    Parse,
    InvalidRequest,
    InvalidParams,
}

pub(crate) fn parse(bytes: &[u8], tasks_extension_enabled: bool) -> Result<Request, WireError> {
    let raw = serde_json::from_slice::<Value>(bytes).map_err(|_| WireError::Parse)?;
    let object = raw.as_object().ok_or(WireError::InvalidRequest)?;
    let has_id = object.contains_key("id");
    if has_id && !object.get("id").is_some_and(valid_request_id) {
        return Err(WireError::InvalidRequest);
    }
    let request: RawRequest =
        serde_json::from_slice(bytes).map_err(|_| WireError::InvalidRequest)?;
    if request.jsonrpc != "2.0" {
        return Err(WireError::InvalidRequest);
    }
    let known_method = known_method(&request.method, tasks_extension_enabled);
    if !known_method {
        return Ok(Request {
            id: request.id,
            has_id,
            method: request.method,
            params: Map::new(),
            known_method,
        });
    }
    let params = request.params.as_deref().map(RawValue::get).unwrap_or("{}");
    if request.method == "tools/call"
        || (tasks_extension_enabled
            && matches!(
                request.method.as_str(),
                "tasks/get" | "tasks/update" | "tasks/cancel"
            ))
    {
        crate::tasks_extension_wire::validate(&request.method, params)
            .map_err(|_| WireError::InvalidParams)?;
    } else {
        validate_base_params(&request.method, params).map_err(|_| WireError::InvalidParams)?;
    }
    let params =
        serde_json::from_str::<Map<String, Value>>(params).map_err(|_| WireError::InvalidParams)?;
    Ok(Request {
        id: request.id,
        has_id,
        method: request.method,
        params,
        known_method,
    })
}

pub(crate) fn valid_request_id(id: &Value) -> bool {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    match id {
        Value::String(_) => true,
        Value::Number(number) => {
            number
                .as_i64()
                .is_some_and(|value| value.unsigned_abs() <= MAX_SAFE_INTEGER)
                || number
                    .as_u64()
                    .is_some_and(|value| value <= MAX_SAFE_INTEGER)
        }
        _ => false,
    }
}

fn known_method(method: &str, tasks_extension_enabled: bool) -> bool {
    matches!(
        method,
        "server/discover"
            | "tools/list"
            | "resources/list"
            | "prompts/list"
            | "tools/call"
            | "resources/read"
            | "prompts/get"
    ) || (tasks_extension_enabled
        && matches!(method, "tasks/get" | "tasks/update" | "tasks/cancel"))
}

fn validate_base_params(method: &str, params: &str) -> serde_json::Result<()> {
    match method {
        "prompts/get" => {
            let parsed: NamedParams = serde_json::from_str(params)?;
            let _ = (parsed.meta, parsed.name, parsed.unknown);
        }
        "resources/read" => {
            let parsed: ResourceParams = serde_json::from_str(params)?;
            let _ = (parsed.meta, parsed.uri, parsed.unknown);
        }
        _ => {
            let parsed: MetaParams = serde_json::from_str(params)?;
            let _ = (parsed.meta, parsed.unknown);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_methods_reject_duplicate_known_fields() {
        let duplicate_uri = br#"{
            "jsonrpc":"2.0",
            "id":1,
            "method":"resources/read",
            "params":{"_meta":{},"uri":"one","uri":"two"}
        }"#;
        assert!(matches!(
            parse(duplicate_uri, true),
            Err(WireError::InvalidParams)
        ));

        let duplicate_meta = br#"{
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/list",
            "params":{"_meta":{},"_meta":{}}
        }"#;
        assert!(matches!(
            parse(duplicate_meta, true),
            Err(WireError::InvalidParams)
        ));
    }

    #[test]
    fn request_ids_are_limited_to_json_rpc_scalar_forms() {
        let body = br#"{
            "jsonrpc":"2.0",
            "id":{"private":"value"},
            "method":"tools/list",
            "params":{"_meta":{}}
        }"#;
        assert!(matches!(parse(body, true), Err(WireError::InvalidRequest)));

        let fractional = br#"{
            "jsonrpc":"2.0",
            "id":1.5,
            "method":"tools/list",
            "params":{"_meta":{}}
        }"#;
        assert!(matches!(
            parse(fractional, true),
            Err(WireError::InvalidRequest)
        ));

        let null = br#"{
            "jsonrpc":"2.0",
            "id":null,
            "method":"tools/list",
            "params":{"_meta":{}}
        }"#;
        assert!(matches!(parse(null, true), Err(WireError::InvalidRequest)));

        for id in ["9007199254740992", "-9007199254740992"] {
            let body = format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list","params":{{"_meta":{{}}}}}}"#
            );
            assert!(matches!(
                parse(body.as_bytes(), true),
                Err(WireError::InvalidRequest)
            ));
        }
    }

    #[test]
    fn request_envelopes_are_closed_but_unknown_methods_route_before_params() {
        let extra = br#"{
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/list",
            "params":{"_meta":{}},
            "extra":true
        }"#;
        assert!(matches!(parse(extra, true), Err(WireError::InvalidRequest)));

        let unknown = br#"{
            "jsonrpc":"2.0",
            "id":1,
            "method":"unknown/method",
            "params":false
        }"#;
        let Ok(parsed) = parse(unknown, true) else {
            panic!("unknown methods route without validating their params");
        };
        assert!(!parsed.known_method);
        assert!(parsed.params.is_empty());
    }
}
