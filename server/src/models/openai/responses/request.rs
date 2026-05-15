use super::{response_item::ResponseItem, tool_spec::ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    pub input: Vec<ResponseItem>,
    pub tools: Vec<ToolSpec>,
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
/// Used by unit tests. HTTP handlers decode the body as-is.
#[cfg(test)]
pub(crate) fn merge_strict_responses_request_defaults(value: Value) -> Value {
    let Value::Object(mut obj) = value else {
        return value;
    };
    let defaults: serde_json::Map<String, Value> = serde_json::from_value(serde_json::json!({
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

#[cfg(test)]
mod tests {
    use super::{ResponsesRequest, merge_strict_responses_request_defaults};
    use serde_json::json;

    #[test]
    fn omits_absent_tool_function_when_serializing() {
        let request: ResponsesRequest =
            serde_json::from_value(merge_strict_responses_request_defaults(json!({
                "model": "gpt-5.4",
                "input": [],
                "tools": [{
                    "type": "local_shell"
                }]
            })))
            .expect("request should parse");

        let body = serde_json::to_value(&request).expect("request should serialize");
        let tool = &body["tools"][0];

        assert!(tool.get("function").is_none());
        assert_eq!(tool["type"], "local_shell");
    }
}
