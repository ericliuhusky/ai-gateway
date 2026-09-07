use serde_json::Value;
use std::collections::HashSet;

pub fn sanitize_codex_request_body(request_body: String) -> Result<String, String> {
    let mut body = parse_request_body(&request_body)?;
    if !sanitize_codex_history(&mut body) {
        return Ok(request_body);
    }
    serialize_request_body(&body)
}

fn parse_request_body(request_body: &str) -> Result<Value, String> {
    serde_json::from_str(request_body).map_err(|err| format!("无效的请求 JSON: {err}"))
}

fn serialize_request_body(body: &Value) -> Result<String, String> {
    serde_json::to_string(body).map_err(|err| err.to_string())
}

fn sanitize_codex_history(body: &mut Value) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    let invalid_call_ids = input
        .iter()
        .filter(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && !has_non_empty_string(item, "name")
        })
        .filter_map(|item| item.get("call_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();

    input.retain(|item| {
        let item_type = item.get("type").and_then(Value::as_str);
        let remove = match item_type {
            Some("function_call") => !has_non_empty_string(item, "name"),
            Some("function_call_output") => item
                .get("call_id")
                .and_then(Value::as_str)
                .is_some_and(|call_id| invalid_call_ids.contains(call_id)),
            Some("reasoning") => !has_non_empty_string(item, "encrypted_content"),
            _ => false,
        };
        changed |= remove;
        !remove
    });

    for item in input {
        if item.get("type").and_then(Value::as_str) != Some("reasoning") {
            continue;
        }
        let Some(content) = item.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        if !content.is_empty() {
            content.clear();
            changed = true;
        }
    }
    changed
}

fn has_non_empty_string(item: &Value, key: &str) -> bool {
    item.get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::sanitize_codex_request_body;
    use serde_json::json;

    #[test]
    fn codex_history_only_empties_reasoning_content() {
        let body = json!({
            "input": [
                {
                    "type": "reasoning",
                    "content": [{"type": "reasoning_text", "text": "do not forward"}],
                    "encrypted_content": "opaque",
                    "summary": [{"type": "summary_text", "text": "summary"}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "visible answer"}]
                }
            ]
        });
        let transformed = sanitize_codex_request_body(body.to_string()).unwrap();
        let adapted: serde_json::Value = serde_json::from_str(&transformed).unwrap();

        assert_eq!(adapted["input"][0]["content"], json!([]));
        assert_eq!(adapted["input"][0]["encrypted_content"], "opaque");
        assert_eq!(adapted["input"][0]["summary"], body["input"][0]["summary"]);
        assert_eq!(adapted["input"][1], body["input"][1]);
    }

    #[test]
    fn codex_history_removes_reasoning_without_encrypted_content() {
        let body = json!({
            "input": [
                {
                    "type": "reasoning",
                    "id": "rs_not_persisted",
                    "content": [],
                    "encrypted_content": null,
                    "summary": [{"type": "summary_text", "text": "summary"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "continue"}]
                }
            ]
        });
        let transformed = sanitize_codex_request_body(body.to_string()).unwrap();
        let adapted: serde_json::Value = serde_json::from_str(&transformed).unwrap();

        assert_eq!(adapted["input"].as_array().unwrap().len(), 1);
        assert_eq!(adapted["input"][0]["type"], "message");
    }

    #[test]
    fn codex_history_removes_invalid_function_call_and_matching_output() {
        let body = json!({
            "input": [
                {
                    "type": "function_call",
                    "call_id": "valid_call",
                    "name": "exec_command",
                    "arguments": "{}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "valid_call",
                    "output": "ok"
                },
                {
                    "type": "function_call",
                    "call_id": "invalid_call",
                    "name": " ",
                    "arguments": "}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "invalid_call",
                    "output": "discarded"
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "continue"}]
                }
            ]
        });

        let transformed = sanitize_codex_request_body(body.to_string()).unwrap();
        let adapted: serde_json::Value = serde_json::from_str(&transformed).unwrap();
        let input = adapted["input"].as_array().unwrap();

        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["call_id"], "valid_call");
        assert_eq!(input[1]["call_id"], "valid_call");
        assert_eq!(input[2]["type"], "message");
    }
}
