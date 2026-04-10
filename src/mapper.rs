use crate::models::{
    GeminiContent, GeminiGenerateRequest, GenerationConfig, OpenAIContent, OpenAIContentBlock,
    OpenAIImageUrl, OpenAIMessage, ResponseCustomToolCallOutputItem, ResponseFunctionCallItem,
    ResponseFunctionCallOutputItem, ResponseFunctionToolCall, ResponseLocalShellCallItem,
    ResponseMessageInput, ResponseOutputContent, ResponseOutputItem, ResponseTool,
    ResponseWebSearchCallItem, ResponsesInput, ResponsesInputBlock, ResponsesInputItem,
    ResponsesRequest, ResponsesResponse, ResponsesUsage, ToolCall, ToolFunction,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub fn responses_to_gemini(request: &ResponsesRequest) -> Result<GeminiGenerateRequest, String> {
    let messages = build_messages(request)?;
    let needs_tool_thought_signature = requires_tool_thought_signature(&request.model);

    let mut system_parts = Vec::new();
    let mut contents = Vec::new();
    let mut tool_id_to_name = HashMap::new();

    for message in &messages {
        if matches!(message.role.as_str(), "system" | "developer") {
            system_parts.extend(message_parts(
                message,
                &mut tool_id_to_name,
                needs_tool_thought_signature,
            )?);
            continue;
        }

        let role = if message.role == "assistant" {
            "model"
        } else {
            "user"
        };

        let parts = message_parts(message, &mut tool_id_to_name, needs_tool_thought_signature)?;
        if !parts.is_empty() {
            contents.push(GeminiContent {
                role: role.to_string(),
                parts,
            });
        }
    }

    if contents.is_empty() {
        return Err("no usable input could be mapped".to_string());
    }

    let generation_config = Some(GenerationConfig {
        max_output_tokens: request.max_output_tokens,
        temperature: request.temperature.or(Some(1.0)),
        top_p: request.top_p.or(Some(1.0)),
        top_k: Some(40),
    });

    let (tools, tool_config) = build_tools(&request.tools, request.tool_choice.as_ref());

    Ok(GeminiGenerateRequest {
        system_instruction: (!system_parts.is_empty()).then_some(GeminiContent {
            role: "user".to_string(),
            parts: system_parts,
        }),
        contents,
        generation_config,
        tools,
        tool_config,
    })
}

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

pub fn wrap_v1internal(body: Value, project_id: &str, model: &str, account_id: &str) -> Value {
    let session_hint = &account_id[..account_id.len().min(8)];
    let mut request = body;
    if let Some(object) = request.as_object_mut() {
        object.insert(
            "sessionId".to_string(),
            Value::String(format!("rustproxy-{account_id}")),
        );
    }

    json!({
        "project": project_id,
        "requestId": format!("agent/rustproxy/{session_hint}/{}", Uuid::new_v4()),
        "request": request,
        "model": model,
        "userAgent": "antigravity",
        "requestType": "agent"
    })
}

pub fn gemini_to_responses(model: &str, gemini: &Value) -> ResponsesResponse {
    let raw = gemini.get("response").unwrap_or(gemini);
    let usage = raw.get("usageMetadata").map(|usage| ResponsesUsage {
        input_tokens: usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output_tokens: usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        total_tokens: usage
            .get("totalTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    });

    ResponsesResponse {
        id: response_id(raw),
        object: "response".to_string(),
        created_at: now_unix(),
        status: "completed".to_string(),
        model: model.to_string(),
        output: extract_output_items(raw),
        usage,
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

fn build_messages(request: &ResponsesRequest) -> Result<Vec<OpenAIMessage>, String> {
    let mut messages = Vec::new();

    if let Some(instructions) = &request.instructions {
        messages.push(OpenAIMessage {
            role: "system".to_string(),
            content: Some(OpenAIContent::String(instructions.clone())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }

    match &request.input {
        Some(ResponsesInput::String(text)) => messages.push(OpenAIMessage {
            role: "user".to_string(),
            content: Some(OpenAIContent::String(text.clone())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }),
        Some(ResponsesInput::Array(items)) => {
            for item in items {
                match item {
                    ResponsesInputItem::Message(message) => {
                        messages.push(response_message_to_openai(message))
                    }
                    ResponsesInputItem::Block(block) => messages.push(OpenAIMessage {
                        role: "user".to_string(),
                        content: Some(match block {
                            ResponsesInputBlock::InputText { text } => {
                                OpenAIContent::String(text.clone())
                            }
                            ResponsesInputBlock::InputImage { image_url } => {
                                OpenAIContent::Array(vec![OpenAIContentBlock::ImageUrl {
                                    image_url: OpenAIImageUrl {
                                        url: image_url.clone(),
                                    },
                                }])
                            }
                        }),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    }),
                    ResponsesInputItem::FunctionCall(item) => {
                        messages.push(function_call_item_to_message(item))
                    }
                    ResponsesInputItem::LocalShellCall(item) => {
                        messages.push(local_shell_call_item_to_message(item))
                    }
                    ResponsesInputItem::WebSearchCall(item) => {
                        messages.push(web_search_call_item_to_message(item))
                    }
                    ResponsesInputItem::FunctionCallOutput(item) => {
                        messages.push(function_call_output_item_to_message(item))
                    }
                    ResponsesInputItem::CustomToolCallOutput(item) => {
                        messages.push(custom_tool_call_output_item_to_message(item))
                    }
                }
            }
        }
        None => return Err("input cannot be empty".to_string()),
    }

    if messages.is_empty() {
        return Err("input cannot be empty".to_string());
    }

    Ok(messages)
}

fn response_message_to_openai(message: &ResponseMessageInput) -> OpenAIMessage {
    OpenAIMessage {
        role: message.role.clone(),
        content: message.content.clone(),
        tool_calls: message.tool_calls.as_ref().map(|tool_calls| {
            tool_calls
                .iter()
                .map(response_tool_call_to_openai)
                .collect::<Vec<_>>()
        }),
        tool_call_id: None,
        name: None,
    }
}

fn response_tool_call_to_openai(tool_call: &ResponseFunctionToolCall) -> ToolCall {
    ToolCall {
        id: tool_call.call_id.clone(),
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
        },
    }
}

fn function_call_item_to_message(item: &ResponseFunctionCallItem) -> OpenAIMessage {
    OpenAIMessage {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: item.call_id.clone(),
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: item.name.clone(),
                arguments: item.arguments.clone(),
            },
        }]),
        tool_call_id: None,
        name: None,
    }
}

fn function_call_output_item_to_message(item: &ResponseFunctionCallOutputItem) -> OpenAIMessage {
    OpenAIMessage {
        role: "tool".to_string(),
        content: Some(OpenAIContent::String(match &item.output {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })),
        tool_calls: None,
        tool_call_id: Some(item.call_id.clone()),
        name: item.name.clone(),
    }
}

fn custom_tool_call_output_item_to_message(
    item: &ResponseCustomToolCallOutputItem,
) -> OpenAIMessage {
    OpenAIMessage {
        role: "tool".to_string(),
        content: Some(OpenAIContent::String(match &item.output {
            Value::String(text) => text.clone(),
            Value::Object(object) => object
                .get("content")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| item.output.to_string()),
            other => other.to_string(),
        })),
        tool_calls: None,
        tool_call_id: Some(item.call_id.clone()),
        name: item.name.clone(),
    }
}

fn local_shell_call_item_to_message(item: &ResponseLocalShellCallItem) -> OpenAIMessage {
    let call_id = item
        .call_id
        .clone()
        .or_else(|| item.id.clone())
        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));

    let mut args = serde_json::Map::new();
    if let Some(action) = &item.action {
        if let Some(exec) = action.get("exec") {
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
        }
    }

    OpenAIMessage {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: call_id,
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: "shell".to_string(),
                arguments: Value::Object(args).to_string(),
            },
        }]),
        tool_call_id: None,
        name: None,
    }
}

fn web_search_call_item_to_message(item: &ResponseWebSearchCallItem) -> OpenAIMessage {
    let call_id = item
        .call_id
        .clone()
        .or_else(|| item.id.clone())
        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));

    let mut args = serde_json::Map::new();
    if let Some(action) = &item.action {
        if let Some(query) = action.get("query") {
            args.insert("query".to_string(), query.clone());
        }
    }

    OpenAIMessage {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: call_id,
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: "google_search".to_string(),
                arguments: Value::Object(args).to_string(),
            },
        }]),
        tool_call_id: None,
        name: None,
    }
}

fn message_parts(
    message: &OpenAIMessage,
    tool_id_to_name: &mut HashMap<String, String>,
    needs_tool_thought_signature: bool,
) -> Result<Vec<Value>, String> {
    let mut parts = Vec::new();

    if message.role != "tool" && message.role != "function" {
        if let Some(content) = &message.content {
            match content {
                OpenAIContent::String(text) => parts.push(json!({ "text": text })),
                OpenAIContent::Array(blocks) => {
                    for block in blocks {
                        match block {
                            OpenAIContentBlock::Text { text } => {
                                parts.push(json!({ "text": text }))
                            }
                            OpenAIContentBlock::ImageUrl { image_url } => {
                                parts.push(map_image_part(&image_url.url)?)
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            let args = serde_json::from_str::<Value>(&tool_call.function.arguments)
                .unwrap_or_else(|_| json!({}));
            tool_id_to_name.insert(tool_call.id.clone(), tool_call.function.name.clone());
            let mut part = json!({
                "functionCall": {
                    "name": tool_call.function.name,
                    "args": args,
                    "id": tool_call.id
                }
            });
            if needs_tool_thought_signature {
                part["thoughtSignature"] = json!("skip_thought_signature_validator");
            }
            parts.push(part);
        }
    }

    if message.role == "tool" || message.role == "function" {
        let id = message.tool_call_id.clone().unwrap_or_default();
        let name = message
            .name
            .clone()
            .or_else(|| tool_id_to_name.get(&id).cloned())
            .unwrap_or_else(|| "unknown".to_string());
        let result = match &message.content {
            Some(OpenAIContent::String(text)) => text.clone(),
            Some(OpenAIContent::Array(blocks)) => blocks
                .iter()
                .filter_map(|block| match block {
                    OpenAIContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            None => String::new(),
        };

        parts.push(json!({
            "functionResponse": {
                "name": name,
                "response": { "result": result },
                "id": id
            }
        }));
    }

    Ok(parts)
}

fn build_tools(
    tools: &Option<Vec<ResponseTool>>,
    tool_choice: Option<&Value>,
) -> (Option<Vec<Value>>, Option<Value>) {
    let Some(tools) = tools else {
        return (None, None);
    };

    let function_declarations: Vec<Value> = tools
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
                .unwrap_or_else(|| {
                    json!({
                        "type": "object",
                        "properties": {},
                        "required": []
                    })
                });
            clean_tool_schema(&mut parameters);

            Some(json!({
                "name": name,
                "description": description,
                "parameters": parameters
            }))
        })
        .collect();

    if function_declarations.is_empty() {
        (None, None)
    } else {
        let function_calling_config = map_tool_choice(tool_choice);
        (
            Some(vec![
                json!({ "functionDeclarations": function_declarations }),
            ]),
            Some(json!({
                "functionCallingConfig": function_calling_config
            })),
        )
    }
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
                .unwrap_or_else(|| {
                    json!({
                        "type": "object",
                        "properties": {},
                        "required": []
                    })
                });
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

fn map_tool_choice(tool_choice: Option<&Value>) -> Value {
    match tool_choice {
        None => json!({ "mode": "VALIDATED" }),
        Some(Value::String(choice)) => match choice.as_str() {
            "none" => json!({ "mode": "NONE" }),
            "required" => json!({ "mode": "ANY" }),
            "auto" => json!({ "mode": "VALIDATED" }),
            _ => json!({ "mode": "VALIDATED" }),
        },
        Some(Value::Object(map)) => {
            let choice_type = map.get("type").and_then(Value::as_str).unwrap_or("");
            if choice_type == "function" {
                let name = map
                    .get("name")
                    .or_else(|| map.get("function").and_then(|f| f.get("name")))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !name.is_empty() {
                    return json!({
                        "mode": "ANY",
                        "allowedFunctionNames": [name]
                    });
                }
            }
            json!({ "mode": "VALIDATED" })
        }
        _ => json!({ "mode": "VALIDATED" }),
    }
}

fn map_image_part(url: &str) -> Result<Value, String> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest
            .split_once(',')
            .ok_or_else(|| "invalid data URL image".to_string())?;
        let mime_type = meta.split(';').next().unwrap_or("image/png");
        Ok(json!({
            "inlineData": {
                "mimeType": mime_type,
                "data": data
            }
        }))
    } else {
        Ok(json!({
            "fileData": {
                "fileUri": url,
                "mimeType": "image/*"
            }
        }))
    }
}

fn clean_tool_schema(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("$schema");
            map.remove("definitions");
            map.remove("$defs");
            map.remove("format");

            let looks_like_schema = map.contains_key("type")
                || map.contains_key("properties")
                || map.contains_key("items")
                || map.contains_key("required")
                || map.contains_key("additionalProperties")
                || map.contains_key("enum")
                || map.contains_key("description");

            if let Some(type_value) = map.get_mut("type") {
                if let Value::String(type_name) = type_value {
                    *type_name = type_name.to_uppercase();
                }
            } else if looks_like_schema {
                map.insert("type".to_string(), Value::String("OBJECT".to_string()));
            }

            if let Some(properties) = map.get_mut("properties") {
                if let Value::Object(properties_map) = properties {
                    for value in properties_map.values_mut() {
                        clean_tool_schema(value);
                    }
                }
            } else {
                for value in map.values_mut() {
                    clean_tool_schema(value);
                }
            }

            if let Some(items) = map.get_mut("items") {
                clean_tool_schema(items);
            }
        }
        Value::Array(values) => {
            for value in values {
                clean_tool_schema(value);
            }
        }
        _ => {}
    }
}

fn requires_tool_thought_signature(model: &str) -> bool {
    model.to_ascii_lowercase().contains("gemini")
}

fn extract_output_items(raw: &Value) -> Vec<ResponseOutputItem> {
    let mut output = Vec::new();

    if let Some(parts) = raw
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
    {
        let text = parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<String>();
        if !text.is_empty() {
            output.push(ResponseOutputItem {
                id: format!("msg_{}", Uuid::new_v4()),
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

        for part in parts {
            if let Some(function_call) = part.get("functionCall") {
                output.push(ResponseOutputItem {
                    id: format!("fc_{}", Uuid::new_v4()),
                    item_type: "function_call".to_string(),
                    role: None,
                    content: None,
                    call_id: Some(
                        function_call
                            .get("id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("call_{}", Uuid::new_v4())),
                    ),
                    name: function_call
                        .get("name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    arguments: Some(
                        function_call
                            .get("args")
                            .map(Value::to_string)
                            .unwrap_or_else(|| "{}".to_string()),
                    ),
                });
            }
        }
    }

    output
}

fn response_id(raw: &Value) -> String {
    raw.get("responseId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("resp_{}", Uuid::new_v4()))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_responses_request() {
        let request = ResponsesRequest {
            model: "gemini-2.5-pro".to_string(),
            input: Some(ResponsesInput::String("hello".to_string())),
            instructions: Some("be concise".to_string()),
            stream: false,
            max_output_tokens: Some(128),
            temperature: Some(0.2),
            top_p: None,
            tools: None,
            tool_choice: None,
        };

        let mapped = responses_to_gemini(&request).unwrap();
        assert_eq!(mapped.contents.len(), 1);
        assert!(mapped.system_instruction.is_some());
        assert_eq!(
            mapped.generation_config.unwrap().max_output_tokens,
            Some(128)
        );
    }

    #[test]
    fn wraps_v1internal_request() {
        let wrapped = wrap_v1internal(
            json!({"contents": []}),
            "test-project",
            "gemini-2.5-pro",
            "account-123",
        );
        assert_eq!(wrapped["project"], "test-project");
        assert_eq!(wrapped["model"], "gemini-2.5-pro");
        assert_eq!(wrapped["requestType"], "agent");
    }

    #[test]
    fn maps_tools_into_gemini() {
        let request = ResponsesRequest {
            model: "gemini-2.5-pro".to_string(),
            input: Some(ResponsesInput::String("hello".to_string())),
            instructions: None,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool {
                tool_type: "function".to_string(),
                name: Some("weather".to_string()),
                description: Some("Get weather".to_string()),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                })),
                function: None,
            }]),
            tool_choice: None,
        };

        let mapped = responses_to_gemini(&request).unwrap();
        assert!(mapped.tools.is_some());
        assert!(mapped.tool_config.is_some());
    }

    #[test]
    fn maps_tool_choice_into_gemini() {
        let request = ResponsesRequest {
            model: "gemini-2.5-pro".to_string(),
            input: Some(ResponsesInput::String("hello".to_string())),
            instructions: None,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool {
                tool_type: "function".to_string(),
                name: Some("weather".to_string()),
                description: Some("Get weather".to_string()),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    }
                })),
                function: None,
            }]),
            tool_choice: Some(json!({
                "type": "function",
                "name": "weather"
            })),
        };

        let mapped = responses_to_gemini(&request).unwrap();
        assert_eq!(
            mapped.tool_config.unwrap()["functionCallingConfig"]["allowedFunctionNames"][0],
            "weather"
        );
    }

    #[test]
    fn ignores_non_function_tools_without_name() {
        let request = ResponsesRequest {
            model: "gemini-2.5-pro".to_string(),
            input: Some(ResponsesInput::String("hello".to_string())),
            instructions: None,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![
                ResponseTool {
                    tool_type: "function".to_string(),
                    name: Some("weather".to_string()),
                    description: Some("Get weather".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": { "city": { "type": "string" } }
                    })),
                    function: None,
                },
                ResponseTool {
                    tool_type: "web_search_preview".to_string(),
                    name: None,
                    description: None,
                    parameters: None,
                    function: None,
                },
            ]),
            tool_choice: None,
        };

        let mapped = responses_to_gemini(&request).unwrap();
        let declarations = mapped.tools.unwrap();
        assert_eq!(
            declarations[0]["functionDeclarations"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            declarations[0]["functionDeclarations"][0]["name"],
            "weather"
        );
    }

    #[test]
    fn maps_shell_and_custom_tool_output_items() {
        let request = ResponsesRequest {
            model: "gemini-2.5-pro".to_string(),
            input: Some(ResponsesInput::Array(vec![
                ResponsesInputItem::LocalShellCall(ResponseLocalShellCallItem {
                    item_type: "local_shell_call".to_string(),
                    call_id: Some("call_shell_1".to_string()),
                    id: None,
                    action: Some(json!({
                        "exec": {
                            "command": "pwd",
                            "working_directory": "/tmp"
                        }
                    })),
                }),
                ResponsesInputItem::CustomToolCallOutput(ResponseCustomToolCallOutputItem {
                    item_type: "custom_tool_call_output".to_string(),
                    call_id: "call_shell_1".to_string(),
                    output: json!({"content": "/tmp"}),
                    name: Some("shell".to_string()),
                }),
            ])),
            instructions: None,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: None,
            tool_choice: None,
        };

        let mapped = responses_to_gemini(&request).unwrap();
        let assistant_parts = &mapped.contents[0].parts;
        let tool_parts = &mapped.contents[1].parts;
        assert_eq!(assistant_parts[0]["functionCall"]["name"], "shell");
        assert_eq!(
            assistant_parts[0]["functionCall"]["args"]["command"][0],
            "pwd"
        );
        assert_eq!(
            tool_parts[0]["functionResponse"]["response"]["result"],
            "/tmp"
        );
    }

    #[test]
    fn maps_nested_function_tool_schema_and_uppercases_types() {
        let request = ResponsesRequest {
            model: "gemini-2.5-pro".to_string(),
            input: Some(ResponsesInput::String("hello".to_string())),
            instructions: None,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool {
                tool_type: "function".to_string(),
                name: None,
                description: None,
                parameters: None,
                function: Some(json!({
                    "name": "apply_patch",
                    "description": "Apply a patch",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "format": "text"
                            }
                        },
                        "required": ["content"]
                    }
                })),
            }]),
            tool_choice: None,
        };

        let mapped = responses_to_gemini(&request).unwrap();
        let declaration = &mapped.tools.unwrap()[0]["functionDeclarations"][0];
        assert_eq!(declaration["name"], "apply_patch");
        assert_eq!(declaration["parameters"]["type"], "OBJECT");
        assert_eq!(
            declaration["parameters"]["properties"]["content"]["type"],
            "STRING"
        );
        assert!(declaration["parameters"]["properties"]["content"]["format"].is_null());
    }

    #[test]
    fn injects_thought_signature_for_gemini_tool_calls() {
        let request = ResponsesRequest {
            model: "gemini-2.5-pro".to_string(),
            input: Some(ResponsesInput::Array(vec![
                ResponsesInputItem::FunctionCall(ResponseFunctionCallItem {
                    item_type: "function_call".to_string(),
                    call_id: "call_exec".to_string(),
                    name: "default_api:exec_command".to_string(),
                    arguments: "{\"cmd\":\"pwd\"}".to_string(),
                }),
            ])),
            instructions: None,
            stream: false,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: None,
            tool_choice: None,
        };

        let mapped = responses_to_gemini(&request).unwrap();
        assert_eq!(
            mapped.contents[0].parts[0]["thoughtSignature"],
            "skip_thought_signature_validator"
        );
    }
}
