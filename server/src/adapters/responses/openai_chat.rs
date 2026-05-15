use crate::models::{
    ChatRequest, ResponseOutputContent, ResponseOutputItem, ResponsesRequest, ResponsesResponse,
    ResponsesUsage,
};
use crate::support::time::now_unix;
use serde_json::Value;
use uuid::Uuid;

pub fn responses_to_chat_completions(
    request: &ResponsesRequest,
    model: &str,
) -> Result<Value, String> {
    let mut body = ChatRequest::try_from(request)?;
    body.model = model.to_string();
    serde_json::to_value(body).map_err(|err| err.to_string())
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
                    r#type: "function_call".to_string(),
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
                r#type: "message".to_string(),
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

#[cfg(test)]
mod tests {
    use super::{chat_completions_to_responses, responses_to_chat_completions};
    use crate::models::ResponsesRequest;
    use crate::models::request::merge_strict_responses_request_defaults;
    use serde_json::json;

    #[test]
    fn preserves_lowercase_json_schema_for_chat_tools() {
        let request: ResponsesRequest =
            serde_json::from_value(merge_strict_responses_request_defaults(json!({
                "model": "gpt-5.4",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }],
                "tools": [{
                    "type": "function",
                    "name": "shell",
                    "description": "Run a command",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "command": { "type": "array" },
                            "workdir": { "type": "string", "format": "uri-reference" }
                        }
                    }
                }]
            })))
            .expect("request should parse");

        let body = responses_to_chat_completions(&request, "chat-compatible-latest")
            .expect("request should convert");
        let parameters = &body["tools"][0]["function"]["parameters"];

        assert_eq!(parameters["type"], "object");
        assert_eq!(parameters["properties"]["command"]["type"], "array");
        assert_eq!(parameters["properties"]["workdir"]["type"], "string");
        assert_eq!(
            parameters["properties"]["workdir"]["format"],
            "uri-reference"
        );
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
        assert_eq!(response.output[0].r#type, "function_call");
        assert_eq!(response.output[0].name.as_deref(), Some("shell"));
        assert_eq!(response.output[1].r#type, "message");
    }

    #[test]
    fn reorders_tool_outputs_to_follow_assistant_tool_calls_for_chat_completions() {
        let request: ResponsesRequest = serde_json::from_value(merge_strict_responses_request_defaults(json!({
            "model": "gpt-5.4",
            "input": [
                { "type": "function_call_output", "call_id": "call_1", "output": "ok", "name": "shell" },
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{\"command\":[\"pwd\"]}" }
            ]
        })))
        .expect("request should parse");

        let body = responses_to_chat_completions(&request, "chat-compatible-latest")
            .expect("request should convert");
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert!(messages[0].get("tool_calls").is_some());
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
    }

    #[test]
    fn demotes_unmatched_tool_outputs_to_user_messages_for_chat_completions() {
        let request: ResponsesRequest = serde_json::from_value(merge_strict_responses_request_defaults(json!({
            "model": "gpt-5.4",
            "input": [
                { "type": "function_call_output", "call_id": "call_orphan", "output": "orphaned", "name": "shell" }
            ]
        })))
        .expect("request should parse");

        let body = responses_to_chat_completions(&request, "chat-compatible-latest")
            .expect("request should convert");
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert!(messages[0].get("tool_call_id").is_none());
    }

    #[test]
    fn preserves_already_ordered_tool_outputs_for_chat_completions() {
        let request: ResponsesRequest = serde_json::from_value(merge_strict_responses_request_defaults(json!({
            "model": "gpt-5.4",
            "input": [
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{\"command\":[\"pwd\"]}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "ok", "name": "shell" }
            ]
        })))
        .expect("request should parse");

        let body = responses_to_chat_completions(&request, "chat-compatible-latest")
            .expect("request should convert");
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
    }
}
