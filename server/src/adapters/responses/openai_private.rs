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
            "custom_tool_call" => {
                let mut tool_call = serde_json::Map::new();
                tool_call.insert(
                    "type".to_string(),
                    Value::String("custom_tool_call".to_string()),
                );
                tool_call.insert(
                    "call_id".to_string(),
                    item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                );
                tool_call.insert(
                    "name".to_string(),
                    item_obj.get("name").cloned().unwrap_or(Value::Null),
                );
                tool_call.insert(
                    "input".to_string(),
                    item_obj
                        .get("input")
                        .cloned()
                        .unwrap_or(Value::String(String::new())),
                );
                copy_optional_fields(item_obj, &mut tool_call, &["id", "status"]);
                rewritten.push(Value::Object(tool_call));
                continue;
            }
            "function_call" => {
                let mut tool_call = serde_json::Map::new();
                tool_call.insert(
                    "type".to_string(),
                    Value::String("function_call".to_string()),
                );
                tool_call.insert(
                    "call_id".to_string(),
                    item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                );
                tool_call.insert(
                    "name".to_string(),
                    item_obj.get("name").cloned().unwrap_or(Value::Null),
                );
                tool_call.insert(
                    "arguments".to_string(),
                    item_obj
                        .get("arguments")
                        .cloned()
                        .unwrap_or(Value::String("{}".to_string())),
                );
                copy_optional_fields(item_obj, &mut tool_call, &["id", "status"]);
                rewritten.push(Value::Object(tool_call));
                continue;
            }
            "function_call_output" => {
                let mut output = serde_json::Map::new();
                output.insert(
                    "type".to_string(),
                    Value::String("function_call_output".to_string()),
                );
                output.insert(
                    "call_id".to_string(),
                    item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                );
                output.insert(
                    "output".to_string(),
                    Value::String(stringify_output(item_obj.get("output").cloned())),
                );
                copy_optional_fields(item_obj, &mut output, &["name", "status"]);
                rewritten.push(Value::Object(output));
                continue;
            }
            "custom_tool_call_output" => {
                let mut output = serde_json::Map::new();
                output.insert(
                    "type".to_string(),
                    Value::String("custom_tool_call_output".to_string()),
                );
                output.insert(
                    "call_id".to_string(),
                    item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                );
                output.insert(
                    "output".to_string(),
                    Value::String(stringify_output(item_obj.get("output").cloned())),
                );
                copy_optional_fields(item_obj, &mut output, &["name", "status"]);
                rewritten.push(Value::Object(output));
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
        if !item_type.is_empty() && item_type != "message" {
            rewritten.push(item);
            continue;
        }
        let role = item_obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        let content = normalize_message_content(item_obj.get("content").cloned(), &role);
        if let Some(content) = content {
            let mut message = item_obj.clone();
            message.insert("type".to_string(), Value::String("message".to_string()));
            message.insert("role".to_string(), Value::String(role.clone()));
            message.insert("content".to_string(), content);
            message.remove("tool_calls");
            rewritten.push(Value::Object(message));
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

fn copy_optional_fields(
    source: &serde_json::Map<String, Value>,
    target: &mut serde_json::Map<String, Value>,
    field_names: &[&str],
) {
    for field_name in field_names {
        if let Some(value) = source.get(*field_name) {
            target.insert((*field_name).to_string(), value.clone());
        }
    }
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
            let mut image = serde_json::Map::new();
            image.insert("type".to_string(), Value::String("input_image".to_string()));
            image.insert("image_url".to_string(), Value::String(url.to_string()));
            if let Some(detail) = part_obj.get("detail") {
                image.insert("detail".to_string(), detail.clone());
            }
            return Value::Object(image);
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
                if key == "description"
                    && server_executed_tool_disallows_description(tool_type, tool_obj)
                {
                    continue;
                }
                if key == "parameters"
                    && server_executed_tool_disallows_parameters(tool_type, tool_obj)
                {
                    continue;
                }
                let mut value = value.clone();
                if key == "tools" {
                    normalize_openai_codex_tools(&mut value);
                }
                preserved.insert(key.clone(), value);
            }
            maybe_add_client_tool_search_defaults(tool_type, tool_obj, &mut preserved);
            if !preserved.is_empty() {
                normalized.push(Value::Object(preserved));
            }
            continue;
        }
        let function_obj = tool_obj.get("function").and_then(Value::as_object);
        let name = tool_obj
            .get("name")
            .filter(|value| !value.is_null())
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("name").cloned()));
        let description = tool_obj
            .get("description")
            .filter(|value| !value.is_null())
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("description").cloned()));
        let parameters = tool_obj
            .get("parameters")
            .filter(|value| !value.is_null())
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("parameters").cloned()));
        let strict = function_obj.and_then(|f| f.get("strict").cloned());
        if name.as_ref().and_then(Value::as_str).is_none() {
            continue;
        }
        let normalized_name = name.unwrap_or(Value::String(String::new()));
        let normalized_name_str = normalized_name.as_str().unwrap_or_default().to_string();
        let mut normalized_tool = serde_json::Map::new();
        for (key, value) in tool_obj {
            if matches!(
                key.as_str(),
                "type" | "name" | "description" | "parameters" | "function"
            ) || value.is_null()
            {
                continue;
            }
            let mut value = value.clone();
            if key == "tools" {
                normalize_openai_codex_tools(&mut value);
            }
            normalized_tool.insert(key.clone(), value);
        }
        normalized_tool.insert("type".to_string(), Value::String("function".to_string()));
        normalized_tool.insert("name".to_string(), normalized_name);
        if !server_executed_tool_disallows_description(&normalized_name_str, tool_obj) {
            if let Some(description) = description {
                normalized_tool.insert("description".to_string(), description);
            }
        }
        if !server_executed_tool_disallows_parameters(&normalized_name_str, tool_obj) {
            normalized_tool.insert(
                "parameters".to_string(),
                parameters.unwrap_or_else(|| json!({"type":"object","properties":{}})),
            );
        }
        if let Some(strict) = strict.clone() {
            normalized_tool.insert("strict".to_string(), strict);
        }
        if let Some(function_obj) = function_obj {
            for (key, value) in function_obj {
                if matches!(
                    key.as_str(),
                    "name" | "description" | "parameters" | "strict"
                ) || value.is_null()
                {
                    continue;
                }
                normalized_tool
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
        }
        normalized.push(Value::Object(normalized_tool));
    }
    *tool_items = normalized;
}

fn maybe_add_client_tool_search_defaults(
    tool_name: &str,
    tool_obj: &serde_json::Map<String, Value>,
    normalized_tool: &mut serde_json::Map<String, Value>,
) {
    if !is_client_executed_tool_search(tool_name, tool_obj) {
        return;
    }
    normalized_tool
        .entry("description".to_string())
        .or_insert_with(client_tool_search_description);
    normalized_tool
        .entry("parameters".to_string())
        .or_insert_with(client_tool_search_parameters);
}

fn is_client_executed_tool_search(
    tool_name: &str,
    tool_obj: &serde_json::Map<String, Value>,
) -> bool {
    tool_name == "tool_search"
        && tool_obj.get("execution").and_then(Value::as_str) == Some("client")
}

fn client_tool_search_description() -> Value {
    Value::String(
        "Search over deferred tool metadata with BM25 and expose matching tools for the next model call."
            .to_string(),
    )
}

fn client_tool_search_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type": "string",
                "description": "Search query for deferred tools."
            },
            "limit": {
                "type": "number",
                "description": "Maximum number of tools to return."
            }
        },
        "required": ["query"]
    })
}

fn server_executed_tool_disallows_description(
    tool_name: &str,
    tool_obj: &serde_json::Map<String, Value>,
) -> bool {
    matches!(tool_name, "tool_search") && !is_client_executed_tool_search(tool_name, tool_obj)
}

fn server_executed_tool_disallows_parameters(
    tool_name: &str,
    tool_obj: &serde_json::Map<String, Value>,
) -> bool {
    matches!(tool_name, "tool_search") && !is_client_executed_tool_search(tool_name, tool_obj)
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

#[cfg(test)]
mod tests {
    use super::responses_to_openai_private;
    use crate::models::ResponsesRequest;
    use serde_json::json;

    #[test]
    fn preserves_custom_tool_calls_for_openai_private() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": [{
                "type": "custom_tool_call",
                "call_id": "call_123",
                "name": "apply_patch",
                "input": "*** Begin Patch\n*** End Patch\n"
            }, {
                "type": "custom_tool_call_output",
                "call_id": "call_123",
                "output": "ok"
            }]
        }))
        .expect("request should parse");

        let body = responses_to_openai_private(&request).expect("request should normalize");

        assert_eq!(body["input"][0]["type"], "custom_tool_call");
        assert_eq!(body["input"][0]["call_id"], "call_123");
        assert_eq!(body["input"][0]["name"], "apply_patch");
        assert_eq!(
            body["input"][0]["input"],
            "*** Begin Patch\n*** End Patch\n"
        );
        assert_eq!(body["input"][1]["type"], "custom_tool_call_output");
        assert_eq!(body["input"][1]["call_id"], "call_123");
    }

    #[test]
    fn preserves_namespace_tools_for_openai_private() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [{
                "type": "namespace",
                "name": "mcp__computer_use__",
                "description": "Computer Use tools",
                "parameters": null,
                "function": null,
                "tools": [{
                    "type": "function",
                    "name": "get_app_state",
                    "description": "Get app state",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "app": { "type": "string" }
                        },
                        "required": ["app"]
                    },
                    "function": null
                }, {
                    "type": "function",
                    "function": {
                        "name": "click",
                        "description": "Click an element",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "app": { "type": "string" },
                                "x": { "type": "number" },
                                "y": { "type": "number" }
                            },
                            "required": ["app"]
                        },
                        "strict": true
                    }
                }]
            }]
        }))
        .expect("request should parse");

        let body = responses_to_openai_private(&request).expect("request should normalize");
        let namespace = &body["tools"][0];

        assert_eq!(namespace["type"], "namespace");
        assert_eq!(namespace["name"], "mcp__computer_use__");
        assert!(namespace.get("function").is_none());
        assert!(namespace.get("parameters").is_none());
        assert!(namespace["tools"].is_array());
        assert_eq!(namespace["tools"].as_array().expect("tools array").len(), 2);

        assert_eq!(namespace["tools"][0]["type"], "function");
        assert_eq!(namespace["tools"][0]["name"], "get_app_state");
        assert_eq!(namespace["tools"][0]["parameters"]["required"][0], "app");

        assert_eq!(namespace["tools"][1]["type"], "function");
        assert_eq!(namespace["tools"][1]["name"], "click");
        assert_eq!(namespace["tools"][1]["strict"], true);
    }

    #[test]
    fn strips_description_from_server_executed_tool_search() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [{
                "type": "tool_search",
                "description": "Search local tools"
            }]
        }))
        .expect("request should parse");

        let body = responses_to_openai_private(&request).expect("request should normalize");
        let tool = &body["tools"][0];

        assert_eq!(tool["type"], "tool_search");
        assert!(tool.get("description").is_none());
    }

    #[test]
    fn strips_parameters_from_server_executed_tool_search() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [{
                "type": "tool_search",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    }
                }
            }]
        }))
        .expect("request should parse");

        let body = responses_to_openai_private(&request).expect("request should normalize");
        let tool = &body["tools"][0];

        assert_eq!(tool["type"], "tool_search");
        assert!(tool.get("parameters").is_none());
    }

    #[test]
    fn adds_description_and_parameters_to_client_executed_tool_search() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [{
                "type": "tool_search",
                "execution": "client"
            }]
        }))
        .expect("request should parse");

        let body = responses_to_openai_private(&request).expect("request should normalize");
        let tool = &body["tools"][0];

        assert_eq!(tool["type"], "tool_search");
        assert_eq!(tool["execution"], "client");
        assert_eq!(
            tool["description"],
            "Search over deferred tool metadata with BM25 and expose matching tools for the next model call."
        );
        assert_eq!(tool["parameters"]["type"], "object");
        assert_eq!(tool["parameters"]["required"][0], "query");
        assert_eq!(tool["parameters"]["properties"]["query"]["type"], "string");
    }

    #[test]
    fn preserves_deferred_flag_for_function_tools() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [{
                "type": "tool_search"
            }, {
                "type": "function",
                "name": "tool_search_tool",
                "description": "Search over deferred tools",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                },
                "deferred": true
            }]
        }))
        .expect("request should parse");

        let body = responses_to_openai_private(&request).expect("request should normalize");
        let tool = &body["tools"][1];

        assert_eq!(tool["type"], "function");
        assert_eq!(tool["name"], "tool_search_tool");
        assert_eq!(tool["deferred"], true);
    }

    #[test]
    fn covers_codex_backend_http_request_fields_items_and_tools() {
        let request: ResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.1-codex",
            "instructions": "You are Codex, a coding agent running locally.",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "Analyze the repository." },
                        { "type": "input_image", "image_url": "data:image/png;base64,BASE64", "detail": "high" }
                    ]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "I will inspect the project." }],
                    "end_turn": false,
                    "phase": "commentary"
                },
                {
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": "Inspect, test, fix." }],
                    "content": [{ "type": "text", "text": "Internal reasoning placeholder." }],
                    "encrypted_content": "gAAAAABl"
                },
                {
                    "type": "function_call",
                    "name": "shell",
                    "arguments": "{\"command\":[\"bash\",\"-lc\",\"cargo test\"]}",
                    "call_id": "call_shell_001"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_shell_001",
                    "output": "{\"output\":\"failed\",\"metadata\":{\"exit_code\":101}}"
                },
                {
                    "type": "custom_tool_call",
                    "status": "completed",
                    "call_id": "call_custom_001",
                    "name": "apply_patch",
                    "input": "*** Begin Patch\n*** End Patch"
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_custom_001",
                    "name": "apply_patch",
                    "output": "Patch applied successfully."
                },
                {
                    "type": "local_shell_call",
                    "call_id": "call_local_shell_001",
                    "status": "completed",
                    "action": {
                        "type": "exec",
                        "command": ["bash", "-lc", "cargo test"],
                        "timeout_ms": 120000,
                        "working_directory": "/workspace/project",
                        "env": { "RUST_BACKTRACE": "1" },
                        "user": null
                    }
                },
                {
                    "type": "tool_search_call",
                    "call_id": "call_tool_search_001",
                    "status": "completed",
                    "execution": "server",
                    "arguments": { "query": "rust test failure", "limit": 10 }
                },
                {
                    "type": "tool_search_output",
                    "call_id": "call_tool_search_001",
                    "status": "completed",
                    "execution": "server",
                    "tools": [{ "name": "example_tool", "description": "placeholder" }]
                },
                {
                    "type": "web_search_call",
                    "status": "completed",
                    "action": { "type": "search", "query": "Rust cargo test failure example" }
                },
                {
                    "type": "image_generation_call",
                    "id": "ig_123",
                    "status": "completed",
                    "revised_prompt": "A diagram of the fixed architecture",
                    "result": "base64-or-image-result-placeholder"
                },
                {
                    "type": "compaction",
                    "encrypted_content": "encrypted_compaction_summary_placeholder"
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "shell",
                    "description": "Run a shell command.",
                    "strict": false,
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "command": { "type": "array", "items": { "type": "string" } },
                            "workdir": { "type": "string" },
                            "timeout_ms": { "type": "integer" },
                            "sandbox_permissions": { "type": "string", "enum": ["use_default", "require_escalated", "with_additional_permissions"] },
                            "justification": { "type": "string" }
                        },
                        "required": ["command"],
                        "additionalProperties": false
                    }
                },
                {
                    "type": "custom",
                    "name": "apply_patch",
                    "description": "Apply a patch to files.",
                    "format": { "type": "grammar", "syntax": "lark", "definition": "start: /(.|\\n)+/" }
                },
                { "type": "local_shell" },
                {
                    "type": "web_search",
                    "external_web_access": true,
                    "filters": { "allowed_domains": ["github.com", "docs.rs"] },
                    "user_location": {
                        "type": "approximate",
                        "country": "US",
                        "region": "CA",
                        "city": "Los Angeles",
                        "timezone": "America/Los_Angeles"
                    },
                    "search_context_size": "medium",
                    "search_content_types": ["text", "image"]
                },
                { "type": "image_generation", "output_format": "png" },
                {
                    "type": "tool_search",
                    "execution": "server",
                    "description": "Search available tools.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "limit": { "type": "integer" }
                        },
                        "required": ["query"],
                        "additionalProperties": false
                    }
                },
                {
                    "type": "namespace",
                    "name": "container",
                    "description": "Container-related tools.",
                    "tools": [{
                        "type": "function",
                        "name": "exec",
                        "description": "Execute a command.",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "cmd": { "type": "array", "items": { "type": "string" } },
                                "workdir": { "type": "string" },
                                "timeout": { "type": "integer" }
                            },
                            "required": ["cmd"],
                            "additionalProperties": false
                        }
                    }]
                }
            ],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": { "effort": "medium", "summary": "auto" },
            "store": true,
            "stream": true,
            "include": ["reasoning.encrypted_content"],
            "service_tier": "priority",
            "prompt_cache_key": "00000000-0000-0000-0000-000000000000",
            "text": {
                "verbosity": "medium",
                "format": {
                    "type": "json_schema",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "summary": { "type": "string" },
                            "changed_files": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["summary", "changed_files"],
                        "additionalProperties": false
                    },
                    "name": "codex_output_schema"
                }
            },
            "client_metadata": {
                "x-codex-installation-id": "installation-id-placeholder"
            }
        }))
        .expect("codex backend request should parse");

        let body = responses_to_openai_private(&request).expect("request should normalize");

        assert_eq!(body["model"], "gpt-5.1-codex");
        assert_eq!(
            body["instructions"],
            "You are Codex, a coding agent running locally."
        );
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["include"][0], "reasoning.encrypted_content");
        assert_eq!(body["service_tier"], "priority");
        assert_eq!(
            body["prompt_cache_key"],
            "00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(body["text"]["verbosity"], "medium");
        assert_eq!(body["text"]["format"]["name"], "codex_output_schema");
        assert_eq!(
            body["client_metadata"]["x-codex-installation-id"],
            "installation-id-placeholder"
        );
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert!(body.get("max_output_tokens").is_none());

        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(
            body["input"][0]["content"][1]["image_url"],
            "data:image/png;base64,BASE64"
        );
        assert_eq!(body["input"][1]["phase"], "commentary");
        assert_eq!(body["input"][2]["type"], "reasoning");
        assert_eq!(body["input"][3]["type"], "function_call");
        assert_eq!(body["input"][4]["type"], "function_call_output");
        assert_eq!(body["input"][5]["type"], "custom_tool_call");
        assert_eq!(body["input"][6]["type"], "custom_tool_call_output");
        assert_eq!(body["input"][7]["type"], "local_shell_call");
        assert_eq!(body["input"][8]["type"], "tool_search_call");
        assert_eq!(body["input"][9]["type"], "tool_search_output");
        assert_eq!(body["input"][10]["type"], "web_search_call");
        assert_eq!(body["input"][11]["type"], "image_generation_call");
        assert_eq!(body["input"][12]["type"], "compaction");

        let tool_types = body["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["type"].as_str().expect("tool type"))
            .collect::<Vec<_>>();
        for expected in [
            "function",
            "custom",
            "local_shell",
            "web_search",
            "image_generation",
            "tool_search",
            "namespace",
        ] {
            assert!(
                tool_types.contains(&expected),
                "missing tool type {expected}"
            );
        }
        assert_eq!(body["tools"][1]["format"]["syntax"], "lark");
        assert_eq!(body["tools"][5]["type"], "tool_search");
        assert!(body["tools"][5].get("description").is_none());
        assert!(body["tools"][5].get("parameters").is_none());
        assert_eq!(body["tools"][6]["tools"][0]["type"], "function");
    }
}
