use super::{ResponseItem, ResponsesRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseCreateWsRequest {
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
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
    pub generate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<HashMap<String, String>>,
}

impl From<ResponseCreateWsRequest> for ResponsesRequest {
    fn from(request: ResponseCreateWsRequest) -> Self {
        Self {
            model: request.model,
            instructions: request.instructions,
            input: request.input,
            tools: request.tools,
            tool_choice: request.tool_choice,
            parallel_tool_calls: request.parallel_tool_calls,
            reasoning: request.reasoning,
            store: request.store,
            stream: request.stream,
            include: request.include,
            service_tier: request.service_tier,
            prompt_cache_key: request.prompt_cache_key,
            text: request.text,
            client_metadata: request.client_metadata,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct ResponseProcessedWsRequest {
    pub response_id: String,
}

/// Fills missing keys so a partial WS `response.create` payload matches
/// [`ResponseCreateWsRequest`] (strict decode).
pub(crate) fn merge_strict_response_create_ws_request_defaults(value: Value) -> Value {
    let Value::Object(mut obj) = value else {
        return value;
    };
    let defaults: Map<String, Value> = serde_json::from_value(json!({
        "client_metadata": null,
        "generate": true,
        "include": [],
        "instructions": "",
        "parallel_tool_calls": true,
        "previous_response_id": null,
        "prompt_cache_key": null,
        "reasoning": null,
        "store": false,
        "stream": false,
        "text": null,
        "tool_choice": "auto",
        "tools": [],
    }))
    .expect("strict websocket default map is valid json");
    for (k, v) in defaults {
        obj.entry(k).or_insert(v);
    }
    Value::Object(obj)
}
