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
