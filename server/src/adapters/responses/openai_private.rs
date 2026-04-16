use crate::models::ResponsesRequest;
use serde_json::{Value, json};

const OPENAI_CODEX_DEFAULT_INSTRUCTIONS: &str = "You are Codex.";

pub fn responses_to_openai_private(request: &ResponsesRequest) -> Result<Value, serde_json::Error> {
    let mut body = serde_json::to_value(request)?;
    sanitize_openai_codex_request_body(&mut body);
    Ok(body)
}

fn sanitize_openai_codex_request_body(body: &mut Value) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    if object
        .get("instructions")
        .is_none_or(|value| value.is_null() || value.as_str().is_some_and(str::is_empty))
    {
        object.insert(
            "instructions".to_string(),
            Value::String(OPENAI_CODEX_DEFAULT_INSTRUCTIONS.to_string()),
        );
    }
    object.insert("store".to_string(), Value::Bool(false));
    for key in ["max_output_tokens", "max_tokens", "temperature", "top_p"] {
        object.remove(key);
    }
    if let Some(tools) = object.get_mut("tools") {
        normalize_openai_codex_tools(tools);
    }
    if let Some(tool_choice) = object.get_mut("tool_choice") {
        normalize_openai_codex_tool_choice(tool_choice);
    }
    if let Some(input) = object.get_mut("input") {
        normalize_openai_codex_input(input);
    }
}

fn normalize_openai_codex_input(input: &mut Value) {
    let Some(items) = input.as_array_mut() else {
        return;
    };
    let mut rewritten = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        let Some(item_obj) = item.as_object() else {
            rewritten.push(item);
            continue;
        };
        let item_type = item_obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match item_type {
            "function_call" => {
                rewritten.push(json!({
                    "type": "function_call",
                    "call_id": item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                    "name": item_obj.get("name").cloned().unwrap_or(Value::Null),
                    "arguments": item_obj.get("arguments").cloned().unwrap_or(Value::String("{}".to_string())),
                }));
                continue;
            }
            "function_call_output" | "custom_tool_call_output" => {
                rewritten.push(json!({
                    "type": "function_call_output",
                    "call_id": item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                    "output": stringify_output(item_obj.get("output").cloned()),
                }));
                continue;
            }
            "input_text" | "input_image" => {
                rewritten.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [normalize_content_part(item.clone(), "user")],
                }));
                continue;
            }
            _ => {}
        }
        let role = item_obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        let content = normalize_message_content(item_obj.get("content").cloned(), &role);
        if let Some(content) = content {
            rewritten.push(json!({ "type": "message", "role": role, "content": content }));
        }
        if let Some(tool_calls) = item_obj.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                if let Some(tool_call_obj) = tool_call.as_object() {
                    rewritten.push(json!({
                        "type": "function_call",
                        "call_id": tool_call_obj.get("call_id").cloned().unwrap_or(Value::Null),
                        "name": tool_call_obj.get("name").cloned().unwrap_or(Value::Null),
                        "arguments": tool_call_obj.get("arguments").cloned().unwrap_or(Value::String("{}".to_string())),
                    }));
                }
            }
        }
    }
    *items = rewritten;
}

fn normalize_message_content(content: Option<Value>, role: &str) -> Option<Value> {
    let content = content?;
    if let Some(text) = content.as_str() {
        return Some(json!([normalize_text_part(text, role)]));
    }
    let Some(parts) = content.as_array() else {
        return None;
    };
    let normalized = parts
        .iter()
        .map(|part| normalize_content_part(part.clone(), role))
        .collect::<Vec<_>>();
    (!normalized.is_empty()).then_some(Value::Array(normalized))
}

fn normalize_content_part(part: Value, role: &str) -> Value {
    let Some(part_obj) = part.as_object() else {
        return normalize_text_part(&part.to_string(), role);
    };
    let part_type = part_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if role == "assistant" && part_type == "refusal" {
        return json!({
            "type": "refusal",
            "refusal": part_obj
                .get("refusal")
                .cloned()
                .or_else(|| part_obj.get("text").cloned())
                .unwrap_or(Value::String(String::new())),
        });
    }
    let is_text_like = matches!(
        part_type,
        "text" | "input_text" | "output_text" | "summary_text"
    ) || part_obj.get("text").is_some();
    if is_text_like {
        return normalize_text_value(
            part_obj
                .get("text")
                .cloned()
                .unwrap_or(Value::String(String::new())),
            role,
        );
    }
    if part_type == "input_image" {
        if let Some(url) = part_obj.get("image_url").and_then(Value::as_str) {
            return json!({ "type": "input_image", "source": { "type": "url", "url": url } });
        }
    }
    part
}

fn normalize_text_part(text: &str, role: &str) -> Value {
    normalize_text_value(Value::String(text.to_string()), role)
}

fn normalize_text_value(text: Value, role: &str) -> Value {
    let text_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    json!({
        "type": text_type,
        "text": text,
    })
}

fn stringify_output(value: Option<Value>) -> String {
    match value {
        Some(Value::String(text)) => text,
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn normalize_openai_codex_tools(tools: &mut Value) {
    let Some(tool_items) = tools.as_array_mut() else {
        return;
    };
    let mut normalized = Vec::with_capacity(tool_items.len());
    for tool in tool_items.drain(..) {
        let Some(tool_obj) = tool.as_object() else {
            continue;
        };
        let tool_type = tool_obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if tool_type != "function" {
            let mut preserved = serde_json::Map::new();
            for (key, value) in tool_obj {
                if key == "function" || value.is_null() {
                    continue;
                }
                preserved.insert(key.clone(), value.clone());
            }
            if !preserved.is_empty() {
                normalized.push(Value::Object(preserved));
            }
            continue;
        }
        let function_obj = tool_obj.get("function").and_then(Value::as_object);
        let name = tool_obj
            .get("name")
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("name").cloned()));
        let description = tool_obj
            .get("description")
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("description").cloned()));
        let parameters = tool_obj
            .get("parameters")
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("parameters").cloned()));
        let strict = function_obj.and_then(|f| f.get("strict").cloned());
        if name.as_ref().and_then(Value::as_str).is_none() {
            continue;
        }
        normalized.push(json!({
            "type": "function",
            "name": name.unwrap_or(Value::String(String::new())),
            "description": description.unwrap_or(Value::Null),
            "parameters": parameters.unwrap_or_else(|| json!({"type":"object","properties":{}})),
            "strict": strict.unwrap_or(Value::Null),
        }));
    }
    *tool_items = normalized;
}

fn normalize_openai_codex_tool_choice(tool_choice: &mut Value) {
    let Some(tool_choice_obj) = tool_choice.as_object() else {
        return;
    };
    if tool_choice_obj.get("type").and_then(Value::as_str) == Some("tool") {
        if let Some(name) = tool_choice_obj.get("name").cloned() {
            *tool_choice = json!({ "type": "function", "function": { "name": name } });
        }
    }
}
