use crate::models::{
    ContentItem, FunctionCallOutputPayload, LocalShellAction, LocalShellExecAction, OpenAIContent,
    OpenAIContentBlock, OpenAIImageUrl, OpenAIMessage, ResponseItem, ResponsesRequest, ToolCall,
    ToolFunction, WebSearchAction,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn build_messages(request: &ResponsesRequest) -> Result<Vec<OpenAIMessage>, String> {
    let mut messages = Vec::new();

    if !request.instructions.trim().is_empty() {
        messages.push(OpenAIMessage {
            role: "system".to_string(),
            content: Some(OpenAIContent::String(request.instructions.clone())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }

    for item in &request.input {
        match item {
            ResponseItem::Other => {}
            ResponseItem::Message { role, content, .. } => {
                messages.push(response_message_to_openai(role, content))
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => messages.push(OpenAIMessage {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: call_id.clone(),
                    tool_type: "function".to_string(),
                    function: ToolFunction {
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                }]),
                tool_call_id: None,
                name: None,
            }),
            ResponseItem::LocalShellCall {
                call_id, action, ..
            } => messages.push(local_shell_call_to_message(call_id.as_ref(), action)),
            ResponseItem::WebSearchCall { id, action, .. } => {
                messages.push(web_search_call_to_message(id.as_ref(), action))
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => messages.push(function_call_output_to_message(call_id, output)),
            ResponseItem::CustomToolCallOutput {
                call_id,
                name,
                output,
            } => messages.push(custom_tool_call_output_to_message(
                call_id,
                name.as_ref(),
                output,
            )),
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => messages.push(OpenAIMessage {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: call_id.clone(),
                    tool_type: "function".to_string(),
                    function: ToolFunction {
                        name: name.clone(),
                        arguments: input.clone(),
                    },
                }]),
                tool_call_id: None,
                name: None,
            }),
            ResponseItem::Reasoning { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::ImageGenerationCall { .. } => {}
        }
    }

    if messages.is_empty() {
        return Err("input cannot be empty".to_string());
    }

    Ok(messages)
}

pub fn clean_tool_schema(value: &mut Value) {
    clean_tool_schema_with_case(value, false);
}

pub fn clean_tool_schema_for_gemini(value: &mut Value) {
    clean_tool_schema_with_case(value, true);
}

fn clean_tool_schema_with_case(value: &mut Value, uppercase_types: bool) {
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
                    if uppercase_types {
                        *type_name = type_name.to_uppercase();
                    }
                }
            } else if looks_like_schema {
                map.insert(
                    "type".to_string(),
                    Value::String(if uppercase_types { "OBJECT" } else { "object" }.to_string()),
                );
            }

            if let Some(properties) = map.get_mut("properties") {
                if let Value::Object(properties_map) = properties {
                    for value in properties_map.values_mut() {
                        clean_tool_schema_with_case(value, uppercase_types);
                    }
                }
            } else {
                for value in map.values_mut() {
                    clean_tool_schema_with_case(value, uppercase_types);
                }
            }

            if let Some(items) = map.get_mut("items") {
                clean_tool_schema_with_case(items, uppercase_types);
            }
        }
        Value::Array(values) => {
            for value in values {
                clean_tool_schema_with_case(value, uppercase_types);
            }
        }
        _ => {}
    }
}

fn response_message_to_openai(role: &str, content: &[ContentItem]) -> OpenAIMessage {
    OpenAIMessage {
        role: role.to_string(),
        content: content_items_to_openai(content),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    }
}

fn content_items_to_openai(items: &[ContentItem]) -> Option<OpenAIContent> {
    if items.is_empty() {
        return None;
    }
    let mut text_parts = Vec::new();
    let mut blocks = Vec::new();
    let mut has_image = false;

    for item in items {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                text_parts.push(text.clone());
                blocks.push(OpenAIContentBlock::Text { text: text.clone() });
            }
            ContentItem::InputImage { image_url, .. } => {
                if let Some(url) = image_url {
                    has_image = true;
                    blocks.push(OpenAIContentBlock::ImageUrl {
                        image_url: OpenAIImageUrl { url: url.clone() },
                    });
                }
            }
            ContentItem::InputFile { .. } => {}
        }
    }

    if has_image {
        (!blocks.is_empty()).then_some(OpenAIContent::Array(blocks))
    } else if !text_parts.is_empty() {
        Some(OpenAIContent::String(text_parts.join("\n")))
    } else {
        None
    }
}

fn local_shell_call_to_message(
    call_id: Option<&String>,
    action: &LocalShellAction,
) -> OpenAIMessage {
    let call_id = call_id
        .cloned()
        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
    let mut args = serde_json::Map::new();
    let LocalShellAction::Exec(LocalShellExecAction {
        command,
        working_directory,
        ..
    }) = action;
    args.insert(
        "command".to_string(),
        serde_json::to_value(command).unwrap_or_else(|_| json!([])),
    );
    if let Some(wd) = working_directory {
        args.insert("workdir".to_string(), Value::String(wd.clone()));
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

fn web_search_call_to_message(
    fallback_id: Option<&String>,
    action: &Option<WebSearchAction>,
) -> OpenAIMessage {
    let call_id = fallback_id
        .cloned()
        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
    let mut args = serde_json::Map::new();
    if let Some(WebSearchAction::Search { query, queries, .. }) = action {
        if let Some(q) = query {
            args.insert("query".to_string(), Value::String(q.clone()));
        } else if let Some(qs) = queries {
            if let Some(first) = qs.first() {
                args.insert("query".to_string(), Value::String(first.clone()));
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
                name: "google_search".to_string(),
                arguments: Value::Object(args).to_string(),
            },
        }]),
        tool_call_id: None,
        name: None,
    }
}

fn function_call_output_to_message(
    call_id: &str,
    output: &FunctionCallOutputPayload,
) -> OpenAIMessage {
    OpenAIMessage {
        role: "tool".to_string(),
        content: Some(OpenAIContent::String(
            output.body.to_text().unwrap_or_else(|| output.to_string()),
        )),
        tool_calls: None,
        tool_call_id: Some(call_id.to_string()),
        name: None,
    }
}

fn custom_tool_call_output_to_message(
    call_id: &str,
    name: Option<&String>,
    output: &FunctionCallOutputPayload,
) -> OpenAIMessage {
    OpenAIMessage {
        role: "tool".to_string(),
        content: Some(OpenAIContent::String(
            output.body.to_text().unwrap_or_else(|| output.to_string()),
        )),
        tool_calls: None,
        tool_call_id: Some(call_id.to_string()),
        name: name.cloned(),
    }
}
