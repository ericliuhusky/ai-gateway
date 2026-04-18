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
    let Some(object) = body.as_object_mut() else {
        return;
    };
    if let Some(input) = object.get_mut("input") {
        normalize_input(input, provider_name);
    }
    if let Some(tools) = object.get_mut("tools") {
        normalize_native_tools(tools);
    }
    if let Some(tool_choice) = object.get_mut("tool_choice") {
        normalize_native_tool_choice(tool_choice);
    }
}

fn normalize_input(input: &mut Value, provider_name: &str) {
    let Some(items) = input.as_array_mut() else {
        rewrite_input_value_types(input);
        return;
    };
    for item in &mut *items {
        normalize_input_item(item, provider_name);
    }
    items.retain(|item| !item.is_null());
}

fn normalize_input_item(item: &mut Value, _provider_name: &str) {
    let Some(object) = item.as_object_mut() else {
        rewrite_input_value_types(item);
        return;
    };

    let item_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match item_type {
        "reasoning" => {
            *item = Value::Null;
            return;
        }
        "message" => {
            object.remove("phase");
        }
        "custom_tool_call_output" => {
            *item = json!({
                "type": "function_call_output",
                "call_id": object.get("call_id").cloned().unwrap_or(Value::Null),
                "output": stringify_output(object.get("output").cloned()),
            });
            return;
        }
        "custom_tool_call" => {
            let call_id = object
                .get("call_id")
                .cloned()
                .or_else(|| object.get("id").cloned())
                .unwrap_or_else(|| Value::String(format!("call_{}", Uuid::new_v4().simple())));
            let name = object
                .get("name")
                .cloned()
                .unwrap_or_else(|| Value::String("custom_tool".to_string()));
            let arguments = stringify_custom_tool_arguments(
                name.as_str().unwrap_or_default(),
                object.get("input").cloned(),
            );
            *item = json!({
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": arguments,
            });
            return;
        }
        "local_shell_call" => {
            let call_id = object
                .get("call_id")
                .cloned()
                .or_else(|| object.get("id").cloned())
                .unwrap_or_else(|| Value::String(format!("call_{}", Uuid::new_v4().simple())));
            *item = json!({
                "type": "function_call",
                "call_id": call_id,
                "name": "shell",
                "arguments": Value::Object(build_shell_call_arguments(object.get("action"))).to_string(),
            });
            return;
        }
        "web_search_call" => {
            let call_id = object
                .get("call_id")
                .cloned()
                .or_else(|| object.get("id").cloned())
                .unwrap_or_else(|| Value::String(format!("call_{}", Uuid::new_v4().simple())));
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
    rewrite_input_value_types(item);
}

fn build_shell_call_arguments(action: Option<&Value>) -> serde_json::Map<String, Value> {
    let mut args = serde_json::Map::new();
    let Some(exec) = action.and_then(|value| value.get("exec")) else {
        return args;
    };
    if let Some(command) = exec.get("command") {
        let command_value = if command.is_string() {
            json!([command])
        } else {
            command.clone()
        };
        args.insert("command".to_string(), command_value);
    }
    if let Some(workdir) = exec
        .get("working_directory")
        .or_else(|| exec.get("workdir"))
    {
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

fn normalize_native_tools(tools: &mut Value) {
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
        let normalized_name =
            normalized_tool_name(tool_type, name.as_ref().and_then(Value::as_str));
        let generated_parameters = generated_tool_schema(tool_type);
        normalized.push(json!({
            "type": "function",
            "name": normalized_name,
            "description": description.unwrap_or_else(|| generated_tool_description(tool_type)),
            "parameters": parameters
                .or(generated_parameters)
                .unwrap_or_else(|| json!({"type":"object","properties":{}})),
            "strict": strict.unwrap_or(Value::Null),
        }));
    }
    *tool_items = normalized;
}

fn normalize_native_tool_choice(tool_choice: &mut Value) {
    let Some(tool_choice_obj) = tool_choice.as_object() else {
        return;
    };
    let choice_type = tool_choice_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if choice_type != "tool" && choice_type != "function" {
        return;
    }
    if let Some(name) = tool_choice_obj
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            tool_choice_obj
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
        })
    {
        *tool_choice = json!({
            "type": "function",
            "name": normalized_tool_name(choice_type, Some(name)),
        });
    }
}

fn normalized_tool_name(tool_type: &str, fallback_name: Option<&str>) -> String {
    match tool_type {
        "local_shell" | "shell_command" => "shell".to_string(),
        "web_search" => "google_search".to_string(),
        other => fallback_name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(other)
            .to_string(),
    }
}

fn generated_tool_description(tool_type: &str) -> Value {
    let description = match tool_type {
        "local_shell" | "shell_command" => "Execute a local shell command.",
        "web_search" => "Search the web for current information.",
        "apply_patch" => "Apply a unified patch to local files.",
        "view_image" => "Inspect a local image file.",
        _ => "Execute a tool call.",
    };
    Value::String(description.to_string())
}

fn generated_tool_schema(tool_type: &str) -> Option<Value> {
    match tool_type {
        "local_shell" | "shell_command" => Some(json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "workdir": { "type": "string" }
            },
            "required": ["command"]
        })),
        "web_search" => Some(json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        })),
        "apply_patch" => Some(json!({
            "type": "object",
            "properties": {
                "patch": { "type": "string" }
            },
            "required": ["patch"]
        })),
        "view_image" => Some(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        })),
        _ => None,
    }
}

fn rewrite_input_value_types(value: &mut Value) {
    rewrite_input_value_types_for_role(value, None);
}

fn rewrite_input_value_types_for_role(value: &mut Value, role: Option<&str>) {
    match value {
        Value::Object(map) => {
            let next_role = map
                .get("role")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| role.map(str::to_owned));

            if let Some(item_type) = map.get_mut("type") {
                if let Some(type_name) = item_type.as_str() {
                    let rewritten = match type_name {
                        "text" => Some(match next_role.as_deref() {
                            Some("assistant") => "output_text",
                            _ => "input_text",
                        }),
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
                rewrite_input_value_types_for_role(nested, next_role.as_deref());
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_input_value_types_for_role(item, role);
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

fn stringify_custom_tool_arguments(name: &str, input: Option<Value>) -> String {
    let key = match name {
        "apply_patch" => "patch",
        "view_image" => "path",
        _ => "input",
    };
    let mut args = serde_json::Map::new();
    args.insert(
        key.to_string(),
        input.unwrap_or_else(|| Value::String(String::new())),
    );
    Value::Object(args).to_string()
}

#[cfg(test)]
mod tests {
    use super::request_with_model;
    use crate::models::ResponsesRequest;
    use serde_json::json;

    #[test]
    fn rewrites_local_shell_tools_to_function_tools() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": "pwd",
            "tools": [{
                "type": "local_shell"
            }]
        }))
        .expect("request should parse");

        let body = request_with_model(&request, "gpt-5.4", "xcode-best")
            .expect("request should normalize");

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "shell");
        assert_eq!(
            body["tools"][0]["parameters"]["properties"]["command"]["type"],
            "array"
        );
    }

    #[test]
    fn rewrites_local_shell_call_items_to_function_call_items() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": [{
                "type": "local_shell_call",
                "call_id": "call_123",
                "action": {
                    "exec": {
                        "command": ["pwd"],
                        "working_directory": "/tmp"
                    }
                }
            }]
        }))
        .expect("request should parse");

        let body = request_with_model(&request, "gpt-5.4", "xcode-best")
            .expect("request should normalize");

        assert_eq!(body["input"][0]["type"], "function_call");
        assert_eq!(body["input"][0]["name"], "shell");
        assert_eq!(
            body["input"][0]["arguments"],
            "{\"command\":[\"pwd\"],\"workdir\":\"/tmp\"}"
        );
    }

    #[test]
    fn drops_reasoning_items_and_strips_message_phase() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": [
                {
                    "type": "reasoning",
                    "summary": [],
                    "content": null,
                    "encrypted_content": "abc"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [{
                        "type": "output_text",
                        "text": "checking"
                    }]
                }
            ]
        }))
        .expect("request should parse");

        let body = request_with_model(&request, "gpt-5.4", "xcode-best")
            .expect("request should normalize");

        assert_eq!(body["input"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["role"], "assistant");
        assert!(body["input"][0].get("phase").is_none());
        assert_eq!(body["input"][0]["content"][0]["type"], "output_text");
    }

    #[test]
    fn rewrites_text_parts_by_message_role() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": "question"
                    }]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "answer"
                    }]
                }
            ]
        }))
        .expect("request should parse");

        let body = request_with_model(&request, "gpt-5.4", "xcode-best")
            .expect("request should normalize");

        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
    }

    #[test]
    fn rewrites_custom_tool_call_items_to_function_call_items() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": [
                {
                    "type": "custom_tool_call",
                    "call_id": "call_123",
                    "name": "apply_patch",
                    "input": "*** Begin Patch\n*** End Patch\n"
                }
            ]
        }))
        .expect("request should parse");

        let body = request_with_model(&request, "gpt-5.4", "xcode-best")
            .expect("request should normalize");

        assert_eq!(body["input"][0]["type"], "function_call");
        assert_eq!(body["input"][0]["call_id"], "call_123");
        assert_eq!(body["input"][0]["name"], "apply_patch");
        assert_eq!(
            body["input"][0]["arguments"],
            "{\"patch\":\"*** Begin Patch\\n*** End Patch\\n\"}"
        );
    }

    #[test]
    fn rewrites_custom_tool_call_outputs_to_function_call_outputs() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": [
                {
                    "type": "custom_tool_call",
                    "call_id": "call_123",
                    "name": "apply_patch",
                    "input": "*** Begin Patch\n*** End Patch\n"
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_123",
                    "output": "ok"
                }
            ]
        }))
        .expect("request should parse");

        let body = request_with_model(&request, "gpt-5.4", "xcode-best")
            .expect("request should normalize");

        assert_eq!(body["input"].as_array().map(Vec::len), Some(2));
        assert_eq!(body["input"][0]["type"], "function_call");
        assert_eq!(body["input"][1]["type"], "function_call_output");
        assert_eq!(body["input"][1]["call_id"], "call_123");
    }
}
