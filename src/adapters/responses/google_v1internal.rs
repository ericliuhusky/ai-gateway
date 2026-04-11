use crate::adapters::responses::shared::{build_messages, clean_tool_schema};
use crate::models::{
    GeminiContent, GeminiGenerateRequest, GenerationConfig, OpenAIContent, OpenAIContentBlock,
    OpenAIMessage, ResponseOutputContent, ResponseOutputItem, ResponseTool, ResponsesRequest,
    ResponsesResponse, ResponsesUsage,
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
            system_parts.extend(build_gemini_message_parts(message, &mut tool_id_to_name, needs_tool_thought_signature)?);
            continue;
        }
        let role = if message.role == "assistant" { "model" } else { "user" };
        let parts = build_gemini_message_parts(message, &mut tool_id_to_name, needs_tool_thought_signature)?;
        if !parts.is_empty() {
            contents.push(GeminiContent { role: role.to_string(), parts });
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

pub fn wrap_v1internal(body: Value, project_id: &str, model: &str, account_id: &str) -> Value {
    let session_hint = &account_id[..account_id.len().min(8)];
    let mut request = body;
    if let Some(object) = request.as_object_mut() {
        object.insert("sessionId".to_string(), Value::String(format!("rustproxy-{account_id}")));
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
        input_tokens: usage.get("promptTokenCount").and_then(Value::as_u64).unwrap_or(0) as u32,
        output_tokens: usage.get("candidatesTokenCount").and_then(Value::as_u64).unwrap_or(0) as u32,
        total_tokens: usage.get("totalTokenCount").and_then(Value::as_u64).unwrap_or(0) as u32,
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

fn build_tools(tools: &Option<Vec<ResponseTool>>, tool_choice: Option<&Value>) -> (Option<Vec<Value>>, Option<Value>) {
    let Some(tools) = tools else { return (None, None) };
    let function_declarations: Vec<Value> = tools.iter().filter_map(|tool| {
        if tool.tool_type != "function" { return None; }
        let function = tool.function.as_ref();
        let name = tool.name.as_deref().or_else(|| function.and_then(|f| f.get("name")).and_then(Value::as_str)).unwrap_or("").trim();
        if name.is_empty() { return None; }
        let description = tool.description.clone().or_else(|| function.and_then(|f| f.get("description")).and_then(Value::as_str).map(ToOwned::to_owned)).unwrap_or_default();
        let mut parameters = tool.parameters.clone().or_else(|| function.and_then(|f| f.get("parameters")).cloned()).unwrap_or_else(|| json!({"type":"object","properties":{},"required":[]}));
        clean_tool_schema(&mut parameters);
        Some(json!({ "name": name, "description": description, "parameters": parameters }))
    }).collect();
    if function_declarations.is_empty() {
        (None, None)
    } else {
        (
            Some(vec![json!({ "functionDeclarations": function_declarations })]),
            Some(json!({ "functionCallingConfig": map_tool_choice(tool_choice) })),
        )
    }
}

fn map_tool_choice(tool_choice: Option<&Value>) -> Value {
    match tool_choice {
        None => json!({ "mode": "VALIDATED" }),
        Some(Value::String(choice)) => match choice.as_str() {
            "none" => json!({ "mode": "NONE" }),
            "required" => json!({ "mode": "ANY" }),
            _ => json!({ "mode": "VALIDATED" }),
        },
        Some(Value::Object(map)) => {
            let choice_type = map.get("type").and_then(Value::as_str).unwrap_or("");
            if choice_type == "function" {
                let name = map.get("name").or_else(|| map.get("function").and_then(|f| f.get("name"))).and_then(Value::as_str).unwrap_or("");
                if !name.is_empty() {
                    return json!({ "mode": "ANY", "allowedFunctionNames": [name] });
                }
            }
            json!({ "mode": "VALIDATED" })
        }
        _ => json!({ "mode": "VALIDATED" }),
    }
}

fn requires_tool_thought_signature(model: &str) -> bool {
    model.to_ascii_lowercase().contains("gemini")
}

fn extract_output_items(raw: &Value) -> Vec<ResponseOutputItem> {
    let mut output = Vec::new();
    if let Some(parts) = raw.get("candidates").and_then(Value::as_array).and_then(|c| c.first()).and_then(|c| c.get("content")).and_then(|c| c.get("parts")).and_then(Value::as_array) {
        let text = parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect::<String>();
        if !text.is_empty() {
            output.push(ResponseOutputItem {
                id: format!("msg_{}", Uuid::new_v4()),
                item_type: "message".to_string(),
                role: Some("assistant".to_string()),
                content: Some(vec![ResponseOutputContent { content_type: "output_text".to_string(), text }]),
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
                    call_id: Some(function_call.get("id").and_then(Value::as_str).map(ToOwned::to_owned).unwrap_or_else(|| format!("call_{}", Uuid::new_v4()))),
                    name: function_call.get("name").and_then(Value::as_str).map(ToOwned::to_owned),
                    arguments: Some(function_call.get("args").map(Value::to_string).unwrap_or_else(|| "{}".to_string())),
                });
            }
        }
    }
    output
}

fn response_id(raw: &Value) -> String {
    raw.get("responseId").and_then(Value::as_str).map(ToOwned::to_owned).unwrap_or_else(|| format!("resp_{}", Uuid::new_v4()))
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn build_gemini_message_parts(
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
                            OpenAIContentBlock::Text { text } => parts.push(json!({ "text": text })),
                            OpenAIContentBlock::ImageUrl { image_url } => parts.push(map_image_part(&image_url.url)?),
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

fn map_image_part(url: &str) -> Result<Value, String> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest
            .split_once(',')
            .ok_or_else(|| "invalid data URL image".to_string())?;
        let mime_type = meta.split(';').next().unwrap_or("image/png");
        Ok(json!({ "inlineData": { "mimeType": mime_type, "data": data } }))
    } else {
        Ok(json!({ "fileData": { "fileUri": url, "mimeType": "image/*" } }))
    }
}
