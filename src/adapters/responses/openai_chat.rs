use crate::adapters::responses::shared::{build_messages, clean_tool_schema};
use crate::models::{
    ResponseOutputContent, ResponseOutputItem, ResponseTool, ResponsesRequest, ResponsesResponse,
    ResponsesUsage,
};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub fn responses_to_chat_completions(
    request: &ResponsesRequest,
    model: &str,
) -> Result<Value, String> {
    let mut messages = build_messages(request)?;
    for message in &mut messages {
        if message.role == "developer" {
            message.role = "system".to_string();
        }
    }
    let mut body = serde_json::Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert(
        "messages".to_string(),
        serde_json::to_value(&messages).map_err(|err| err.to_string())?,
    );
    if let Some(tools) = build_openai_tools(&request.tools) {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(tool_choice) = map_openai_tool_choice(request.tool_choice.as_ref()) {
        body.insert("tool_choice".to_string(), tool_choice);
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        body.insert("max_tokens".to_string(), json!(max_output_tokens));
    }
    if let Some(temperature) = request.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        body.insert("top_p".to_string(), json!(top_p));
    }
    Ok(Value::Object(body))
}

fn build_openai_tools(tools: &Option<Vec<ResponseTool>>) -> Option<Vec<Value>> {
    let tools = tools.as_ref()?;
    let mapped: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            if tool.tool_type != "function" {
                return None;
            }

            let function = tool.function.as_ref();
            let name = tool
                .name
                .as_deref()
                .or_else(|| function.and_then(|f| f.get("name")).and_then(Value::as_str))
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                return None;
            }

            let description = tool
                .description
                .clone()
                .or_else(|| {
                    function
                        .and_then(|f| f.get("description"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_default();
            let mut parameters = tool
                .parameters
                .clone()
                .or_else(|| function.and_then(|f| f.get("parameters")).cloned())
                .unwrap_or_else(|| json!({"type":"object","properties":{},"required":[]}));
            clean_tool_schema(&mut parameters);

            Some(json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": parameters,
                }
            }))
        })
        .collect();

    (!mapped.is_empty()).then_some(mapped)
}

fn map_openai_tool_choice(tool_choice: Option<&Value>) -> Option<Value> {
    match tool_choice {
        None => None,
        Some(Value::String(choice)) => Some(Value::String(choice.clone())),
        Some(Value::Object(map)) => {
            let choice_type = map.get("type").and_then(Value::as_str).unwrap_or("");
            if choice_type == "function" {
                let name = map
                    .get("name")
                    .or_else(|| {
                        map.get("function")
                            .and_then(|function| function.get("name"))
                    })
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !name.is_empty() {
                    return Some(json!({
                        "type": "function",
                        "function": { "name": name }
                    }));
                }
            }
            Some(Value::Object(map.clone()))
        }
        Some(other) => Some(other.clone()),
    }
}

pub fn chat_completions_to_responses(model: &str, chat: &Value) -> ResponsesResponse {
    let usage = chat.get("usage").map(|usage| ResponsesUsage {
        input_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    });

    let mut output = Vec::new();
    if let Some(message) = chat
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
    {
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let call_id = tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("call_{}", Uuid::new_v4().simple()));
                let name = tool_call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| "unknown".to_string());
                let arguments = tool_call
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .map(|value| match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_else(|| "{}".to_string());
                output.push(ResponseOutputItem {
                    id: format!("fc_{}", Uuid::new_v4().simple()),
                    item_type: "function_call".to_string(),
                    role: None,
                    content: None,
                    call_id: Some(call_id),
                    name: Some(name),
                    arguments: Some(arguments),
                });
            }
        }
        let text = message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !text.is_empty() {
            output.push(ResponseOutputItem {
                id: format!("msg_{}", Uuid::new_v4().simple()),
                item_type: "message".to_string(),
                role: Some("assistant".to_string()),
                content: Some(vec![ResponseOutputContent {
                    content_type: "output_text".to_string(),
                    text,
                }]),
                call_id: None,
                name: None,
                arguments: None,
            });
        }
    }

    ResponsesResponse {
        id: chat
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("resp_{}", Uuid::new_v4().simple())),
        object: "response".to_string(),
        created_at: chat
            .get("created")
            .and_then(Value::as_u64)
            .unwrap_or_else(now_unix),
        status: "completed".to_string(),
        model: model.to_string(),
        output,
        usage,
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{chat_completions_to_responses, responses_to_chat_completions};
    use crate::models::ResponsesRequest;
    use serde_json::json;

    #[test]
    fn preserves_lowercase_json_schema_for_chat_tools() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [{
                "type": "function",
                "name": "shell",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "array" },
                        "workdir": { "type": "string", "format": "uri-reference" }
                    }
                }
            }]
        }))
        .expect("request should parse");

        let body = responses_to_chat_completions(&request, "ark-code-latest")
            .expect("request should convert");
        let parameters = &body["tools"][0]["function"]["parameters"];

        assert_eq!(parameters["type"], "object");
        assert_eq!(parameters["properties"]["command"]["type"], "array");
        assert_eq!(parameters["properties"]["workdir"]["type"], "string");
        assert!(parameters["properties"]["workdir"].get("format").is_none());
    }

    #[test]
    fn maps_chat_completions_tool_calls_and_text_back_to_responses() {
        let chat = json!({
            "id": "chatcmpl_123",
            "created": 1_700_000_000u64,
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 6,
                "total_tokens": 16
            },
            "choices": [{
                "message": {
                    "content": "done",
                    "tool_calls": [{
                        "id": "call_123",
                        "function": {
                            "name": "shell",
                            "arguments": "{\"command\":[\"pwd\"]}"
                        }
                    }]
                }
            }]
        });

        let response = chat_completions_to_responses("gpt-5.4", &chat);

        assert_eq!(response.output.len(), 2);
        assert_eq!(response.output[0].item_type, "function_call");
        assert_eq!(response.output[0].name.as_deref(), Some("shell"));
        assert_eq!(response.output[1].item_type, "message");
    }
}
