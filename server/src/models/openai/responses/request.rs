use super::response_item::ResponseItem;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    pub input: Vec<ResponseItem>,
    pub tools: Vec<Value>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
    pub reasoning: Option<Value>,
    pub store: bool,
    pub stream: bool,
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<HashMap<String, String>>,
}

/// Fills missing keys so a partial JSON object matches [`ResponsesRequest`] (strict decode).
/// Used by WebSocket `response.create` ingestion and unit tests. HTTP handlers decode the body as-is.
pub(crate) fn merge_strict_responses_request_defaults(value: Value) -> Value {
    let Value::Object(mut obj) = value else {
        return value;
    };
    let defaults: Map<String, Value> = serde_json::from_value(json!({
        "client_metadata": null,
        "include": [],
        "instructions": "",
        "parallel_tool_calls": true,
        "prompt_cache_key": null,
        "reasoning": null,
        "store": false,
        "stream": false,
        "text": null,
        "tool_choice": "auto",
        "tools": [],
    }))
    .expect("strict default map is valid json");
    for (k, v) in defaults {
        obj.entry(k).or_insert(v);
    }
    Value::Object(obj)
}

/// Wraps [`ResponsesRequest::tool_choice`] as [`Value::String`] for adapters that expect [`Value`].
pub(crate) fn tool_choice_as_value(s: &str) -> Value {
    Value::String(s.to_owned())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<Value>,
    #[serde(default)]
    pub function: Option<Value>,
    #[serde(default)]
    pub tools: Option<Vec<ResponseTool>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub(crate) fn response_tool_from_value(value: &Value) -> Option<ResponseTool> {
    serde_json::from_value(value.clone()).ok()
}
