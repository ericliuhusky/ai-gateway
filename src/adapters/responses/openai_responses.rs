use crate::models::ResponsesRequest;
use serde_json::{Value, json};
use uuid::Uuid;

pub fn request_with_model(
    request: &ResponsesRequest,
    model: &str,
    provider_name: &str,
) -> Result<Value, serde_json::Error> {
    let mut body = serde_json::to_value(request)?;
    if let Some(object) = body.as_object_mut() {
        object.insert("model".to_string(), Value::String(model.to_string()));
    }
    strip_null_fields(&mut body);
    normalize_request(&mut body, provider_name);
    Ok(body)
}

fn strip_null_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, nested| {
                strip_null_fields(nested);
                !nested.is_null()
            });
        }
        Value::Array(items) => {
            for item in items {
                strip_null_fields(item);
            }
        }
        _ => {}
    }
}

fn normalize_request(body: &mut Value, provider_name: &str) {
    let Some(object) = body.as_object_mut() else { return };
    if let Some(input) = object.get_mut("input") {
        normalize_input(input, provider_name);
    }
    if provider_name == "bytedance" {
        if let Some(tools) = object.get_mut("tools") {
            normalize_bytedance_tools(tools);
        }
        if let Some(tool_choice) = object.get_mut("tool_choice") {
            normalize_bytedance_tool_choice(tool_choice);
        }
    }
}

fn normalize_input(input: &mut Value, provider_name: &str) {
    let Some(items) = input.as_array_mut() else {
        rewrite_input_value_types(input);
        return;
    };
    for item in items {
        normalize_input_item(item, provider_name);
    }
}

fn normalize_input_item(item: &mut Value, provider_name: &str) {
    let Some(object) = item.as_object_mut() else {
        rewrite_input_value_types(item);
        return;
    };

    if provider_name == "bytedance" {
        let item_type = object.get("type").and_then(Value::as_str).unwrap_or_default();
        match item_type {
            "custom_tool_call_output" => {
                *item = json!({
                    "type": "function_call_output",
                    "call_id": object.get("call_id").cloned().unwrap_or(Value::Null),
                    "output": stringify_output(object.get("output").cloned()),
                });
                return;
            }
            "local_shell_call" => {
                let call_id = object.get("call_id").cloned().or_else(|| object.get("id").cloned()).unwrap_or_else(|| Value::String(format!("call_{}", Uuid::new_v4().simple())));
                *item = json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": "shell",
                    "arguments": Value::Object(build_shell_call_arguments(object.get("action"))).to_string(),
                });
                return;
            }
            "web_search_call" => {
                let call_id = object.get("call_id").cloned().or_else(|| object.get("id").cloned()).unwrap_or_else(|| Value::String(format!("call_{}", Uuid::new_v4().simple())));
                *item = json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": "google_search",
                    "arguments": Value::Object(build_web_search_arguments(object.get("action"))).to_string(),
                });
                return;
            }
            _ => {}
        }
    }
    rewrite_input_value_types(item);
}

fn build_shell_call_arguments(action: Option<&Value>) -> serde_json::Map<String, Value> {
    let mut args = serde_json::Map::new();
    let Some(exec) = action.and_then(|value| value.get("exec")) else { return args };
    if let Some(command) = exec.get("command") {
        let command_value = if command.is_string() { json!([command]) } else { command.clone() };
        args.insert("command".to_string(), command_value);
    }
    if let Some(workdir) = exec.get("working_directory").or_else(|| exec.get("workdir")) {
        args.insert("workdir".to_string(), workdir.clone());
    }
    args
}

fn build_web_search_arguments(action: Option<&Value>) -> serde_json::Map<String, Value> {
    let mut args = serde_json::Map::new();
    if let Some(query) = action.and_then(|value| value.get("query")) {
        args.insert("query".to_string(), query.clone());
    }
    args
}

fn normalize_bytedance_tools(tools: &mut Value) {
    let Some(tool_items) = tools.as_array_mut() else { return };
    let mut normalized = Vec::with_capacity(tool_items.len());
    for tool in tool_items.drain(..) {
        let Some(tool_obj) = tool.as_object() else { continue };
        let function_obj = tool_obj.get("function").and_then(Value::as_object);
        let name = tool_obj.get("name").cloned().or_else(|| function_obj.and_then(|f| f.get("name").cloned()));
        let description = tool_obj.get("description").cloned().or_else(|| function_obj.and_then(|f| f.get("description").cloned()));
        let parameters = tool_obj.get("parameters").cloned().or_else(|| function_obj.and_then(|f| f.get("parameters").cloned()));
        let strict = function_obj.and_then(|f| f.get("strict").cloned());
        if name.as_ref().and_then(Value::as_str).is_none() { continue; }
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

fn normalize_bytedance_tool_choice(tool_choice: &mut Value) {
    let Some(tool_choice_obj) = tool_choice.as_object() else { return };
    let choice_type = tool_choice_obj.get("type").and_then(Value::as_str).unwrap_or_default();
    if choice_type != "tool" && choice_type != "function" { return; }
    if let Some(name) = tool_choice_obj.get("name").cloned().or_else(|| tool_choice_obj.get("function").and_then(|function| function.get("name")).cloned()) {
        *tool_choice = json!({ "type": "function", "name": name });
    }
}

fn rewrite_input_value_types(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(item_type) = map.get_mut("type") {
                if let Some(type_name) = item_type.as_str() {
                    let rewritten = match type_name {
                        "text" => Some("input_text"),
                        "image_url" => Some("input_image"),
                        "custom_tool_call_output" => Some("function_call_output"),
                        _ => None,
                    };
                    if let Some(next) = rewritten {
                        *item_type = Value::String(next.to_string());
                    }
                }
            }
            for nested in map.values_mut() {
                rewrite_input_value_types(nested);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_input_value_types(item);
            }
        }
        _ => {}
    }
}

fn stringify_output(value: Option<Value>) -> String {
    match value {
        Some(Value::String(text)) => text,
        Some(other) => other.to_string(),
        None => String::new(),
    }
}
