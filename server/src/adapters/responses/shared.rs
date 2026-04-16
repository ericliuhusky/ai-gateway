use crate::models::{
    OpenAIContent, OpenAIContentBlock, OpenAIImageUrl, OpenAIMessage,
    ResponseCustomToolCallOutputItem, ResponseFunctionCallItem, ResponseFunctionCallOutputItem,
    ResponseFunctionToolCall, ResponseLocalShellCallItem, ResponseMessageInput,
    ResponseWebSearchCallItem, ResponsesInput, ResponsesInputBlock, ResponsesInputItem,
    ResponsesRequest, ToolCall, ToolFunction,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn build_messages(request: &ResponsesRequest) -> Result<Vec<OpenAIMessage>, String> {
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
                    ResponsesInputItem::Raw(value) => {
                        if let Some(message) = raw_input_item_to_message(value) {
                            messages.push(message);
                        }
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

fn raw_input_item_to_message(value: &Value) -> Option<OpenAIMessage> {
    let object = value.as_object()?;
    match object.get("type").and_then(Value::as_str).unwrap_or_default() {
        "reasoning" => None,
        "message" => raw_message_to_openai(object),
        "function_call" => Some(raw_function_call_to_message(object)),
        "local_shell_call" => Some(raw_local_shell_call_to_message(object)),
        "web_search_call" => Some(raw_web_search_call_to_message(object)),
        "function_call_output" => Some(raw_tool_output_to_message(object)),
        "custom_tool_call_output" => Some(raw_tool_output_to_message(object)),
        _ => None,
    }
}

fn raw_message_to_openai(object: &serde_json::Map<String, Value>) -> Option<OpenAIMessage> {
    let role = object.get("role")?.as_str()?.to_string();
    let content = object.get("content").and_then(raw_message_content_to_openai);
    let tool_calls = object
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|tool_calls| {
            tool_calls
                .iter()
                .filter_map(raw_tool_call_to_openai)
                .collect::<Vec<_>>()
        })
        .filter(|tool_calls| !tool_calls.is_empty());

    Some(OpenAIMessage {
        role,
        content,
        tool_calls,
        tool_call_id: None,
        name: None,
    })
}

fn raw_message_content_to_openai(value: &Value) -> Option<OpenAIContent> {
    match value {
        Value::String(text) => Some(OpenAIContent::String(text.clone())),
        Value::Array(parts) => {
            let mut text_parts = Vec::new();
            let mut blocks = Vec::new();
            let mut has_image = false;

            for part in parts {
                let Some(part_obj) = part.as_object() else {
                    continue;
                };
                match part_obj.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "text" | "input_text" | "output_text" => {
                        if let Some(text) = part_obj.get("text").and_then(Value::as_str) {
                            text_parts.push(text.to_string());
                            blocks.push(OpenAIContentBlock::Text {
                                text: text.to_string(),
                            });
                        }
                    }
                    "image_url" => {
                        let url = part_obj
                            .get("image_url")
                            .and_then(Value::as_object)
                            .and_then(|image| image.get("url"))
                            .and_then(Value::as_str)
                            .or_else(|| part_obj.get("image_url").and_then(Value::as_str));
                        if let Some(url) = url {
                            has_image = true;
                            blocks.push(OpenAIContentBlock::ImageUrl {
                                image_url: OpenAIImageUrl {
                                    url: url.to_string(),
                                },
                            });
                        }
                    }
                    "input_image" => {
                        if let Some(url) = part_obj.get("image_url").and_then(Value::as_str) {
                            has_image = true;
                            blocks.push(OpenAIContentBlock::ImageUrl {
                                image_url: OpenAIImageUrl {
                                    url: url.to_string(),
                                },
                            });
                        }
                    }
                    _ => {}
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
        _ => None,
    }
}

fn raw_tool_call_to_openai(value: &Value) -> Option<ToolCall> {
    let object = value.as_object()?;
    let id = object
        .get("call_id")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = object.get("name").and_then(Value::as_str).unwrap_or_default();
    let arguments = object
        .get("arguments")
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "{}".to_string());
    if id.is_empty() || name.is_empty() {
        return None;
    }
    Some(ToolCall {
        id: id.to_string(),
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: name.to_string(),
            arguments,
        },
    })
}

fn raw_function_call_to_message(object: &serde_json::Map<String, Value>) -> OpenAIMessage {
    let call_id = object
        .get("call_id")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "unknown".to_string());
    let arguments = object
        .get("arguments")
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "{}".to_string());

    OpenAIMessage {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: call_id,
            tool_type: "function".to_string(),
            function: ToolFunction { name, arguments },
        }]),
        tool_call_id: None,
        name: None,
    }
}

fn raw_local_shell_call_to_message(object: &serde_json::Map<String, Value>) -> OpenAIMessage {
    let call_id = object
        .get("call_id")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
    let mut args = serde_json::Map::new();
    if let Some(exec) = object
        .get("action")
        .and_then(|action| action.get("exec"))
        .and_then(Value::as_object)
    {
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

fn raw_web_search_call_to_message(object: &serde_json::Map<String, Value>) -> OpenAIMessage {
    let call_id = object
        .get("call_id")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
    let mut args = serde_json::Map::new();
    if let Some(query) = object
        .get("action")
        .and_then(|action| action.get("query"))
    {
        args.insert("query".to_string(), query.clone());
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

fn raw_tool_output_to_message(object: &serde_json::Map<String, Value>) -> OpenAIMessage {
    let call_id = object
        .get("call_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_default();
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let output = object.get("output").cloned().unwrap_or(Value::Null);

    OpenAIMessage {
        role: "tool".to_string(),
        content: Some(OpenAIContent::String(match output {
            Value::String(text) => text,
            Value::Object(ref map) => map
                .get("content")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| output.to_string()),
            other => other.to_string(),
        })),
        tool_calls: None,
        tool_call_id: Some(call_id),
        name,
    }
}
