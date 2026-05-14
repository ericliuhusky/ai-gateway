use super::{
    ChatRequest, Content, ContentBlock, ImageUrl, Message, ToolCall, ToolFunction, ToolSpec,
    ToolSpecFunction,
};
use super::super::responses::{ResponseTool, ResponsesRequest};
use crate::models::{
    ContentItem, FunctionCallOutputPayload, LocalShellAction, LocalShellExecAction, ResponseItem,
    WebSearchAction,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

impl ChatRequest {
    pub fn normalize_messages_for_chat_completions(messages: &mut Vec<Message>) {
        for message in &mut *messages {
            if message.role == "developer" {
                message.role = "system".to_string();
            }
        }
        reorder_tool_messages_for_chat_completions(messages);
    }
}

impl TryFrom<&ResponsesRequest> for ChatRequest {
    type Error = String;

    fn try_from(request: &ResponsesRequest) -> Result<Self, Self::Error> {
        let mut messages = build_messages(request)?;
        Self::normalize_messages_for_chat_completions(&mut messages);

        Ok(Self {
            messages,
            model: request.model.clone(),
            tool_choice: (!request.tool_choice.trim().is_empty())
                .then_some(request.tool_choice.clone()),
            tools: request
                .tools
                .iter()
                .filter_map(ToolSpec::from_response_tool)
                .collect(),
        })
    }
}

impl ToolSpec {
    pub fn from_response_tool(tool: &ResponseTool) -> Option<Self> {
        let name = normalized_tool_name(&tool.r#type, tool.name.as_deref());
        if name.is_empty() {
            return None;
        }

        let description = tool
            .description
            .clone()
            .or_else(|| generated_tool_description(&tool.r#type));
        let parameters = tool
            .parameters
            .clone()
            .or_else(|| generated_tool_schema(&tool.r#type))
            .map(|mut schema| {
                clean_tool_schema(&mut schema);
                schema
            })
            .or_else(|| Some(json!({"type":"object","properties":{},"required":[]})));

        Some(Self {
            r#type: "function".to_string(),
            function: ToolSpecFunction {
                name,
                description,
                parameters,
            },
        })
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

fn generated_tool_description(tool_type: &str) -> Option<String> {
    let description = match tool_type {
        "local_shell" | "shell_command" => "Execute a local shell command.",
        "web_search" => "Search the web for current information.",
        "apply_patch" => "Apply a unified patch to local files.",
        "view_image" => "Inspect a local image file.",
        "function" => return None,
        _ => "Execute a tool call.",
    };
    Some(description.to_string())
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

            if !map.contains_key("type") && looks_like_schema {
                map.insert("type".to_string(), Value::String("object".to_string()));
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

pub fn build_messages(request: &ResponsesRequest) -> Result<Vec<Message>, String> {
    let mut messages = Vec::new();

    if !request.instructions.trim().is_empty() {
        messages.push(Message {
            role: "system".to_string(),
            content: Some(Content::String(request.instructions.clone())),
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
            } => messages.push(Message {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
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
            } => messages.push(Message {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
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

fn reorder_tool_messages_for_chat_completions(messages: &mut Vec<Message>) {
    let original = std::mem::take(messages);
    if original.is_empty() {
        return;
    }

    let mut tool_call_to_assistant_idx: HashMap<String, usize> = HashMap::new();
    for (idx, message) in original.iter().enumerate() {
        if message.role != "assistant" {
            continue;
        }
        let Some(tool_calls) = &message.tool_calls else {
            continue;
        };
        for tool_call in tool_calls {
            if !tool_call.id.trim().is_empty() {
                tool_call_to_assistant_idx.insert(tool_call.id.clone(), idx);
            }
        }
    }

    let mut tool_outputs_by_assistant_idx: HashMap<usize, Vec<Message>> = HashMap::new();
    let mut rewritten: Vec<Message> = Vec::with_capacity(original.len());
    let mut seen_assistants = HashSet::new();

    for (idx, mut message) in original.iter().cloned().enumerate() {
        if message.role == "tool" {
            message.name = None;

            let call_id = message
                .tool_call_id
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string();

            if call_id.is_empty() {
                message.role = "user".to_string();
                message.tool_call_id = None;
                message.tool_calls = None;
                rewritten.push(message);
                continue;
            }

            if let Some(&assistant_idx) = tool_call_to_assistant_idx.get(&call_id) {
                if seen_assistants.contains(&assistant_idx) {
                    rewritten.push(message);
                } else {
                    tool_outputs_by_assistant_idx
                        .entry(assistant_idx)
                        .or_default()
                        .push(message);
                }
                continue;
            }

            message.role = "user".to_string();
            message.tool_call_id = None;
            message.tool_calls = None;
            rewritten.push(message);
            continue;
        }

        let is_assistant = message.role == "assistant";
        rewritten.push(message);

        if is_assistant {
            seen_assistants.insert(idx);
            if let Some(tool_outputs) = tool_outputs_by_assistant_idx.remove(&idx) {
                rewritten.extend(tool_outputs);
            }
        }
    }

    if !tool_outputs_by_assistant_idx.is_empty() {
        let mut leftovers = tool_outputs_by_assistant_idx
            .into_values()
            .flatten()
            .collect::<Vec<_>>();
        for leftover in &mut leftovers {
            leftover.role = "user".to_string();
            leftover.tool_call_id = None;
            leftover.tool_calls = None;
        }
        rewritten.extend(leftovers);
    }

    *messages = rewritten;
}

fn response_message_to_openai(role: &str, content: &[ContentItem]) -> Message {
    Message {
        role: role.to_string(),
        content: content_items_to_openai(content),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    }
}

fn content_items_to_openai(items: &[ContentItem]) -> Option<Content> {
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
                blocks.push(ContentBlock::Text { text: text.clone() });
            }
            ContentItem::InputImage { image_url, .. } => {
                if let Some(url) = image_url {
                    has_image = true;
                    blocks.push(ContentBlock::ImageUrl {
                        image_url: ImageUrl { url: url.clone() },
                    });
                }
            }
            ContentItem::InputFile { .. } => {}
        }
    }

    if has_image {
        (!blocks.is_empty()).then_some(Content::Array(blocks))
    } else if !text_parts.is_empty() {
        Some(Content::String(text_parts.join("\n")))
    } else {
        None
    }
}

fn local_shell_call_to_message(
    call_id: Option<&String>,
    action: &LocalShellAction,
) -> Message {
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
    Message {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: call_id,
            r#type: "function".to_string(),
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
) -> Message {
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
    Message {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: call_id,
            r#type: "function".to_string(),
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
) -> Message {
    Message {
        role: "tool".to_string(),
        content: Some(Content::String(
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
) -> Message {
    Message {
        role: "tool".to_string(),
        content: Some(Content::String(
            output.body.to_text().unwrap_or_else(|| output.to_string()),
        )),
        tool_calls: None,
        tool_call_id: Some(call_id.to_string()),
        name: name.cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ChatRequest, ToolSpec};
    use crate::models::request::{ResponsesRequest, merge_strict_responses_request_defaults};
    use serde_json::json;

    #[test]
    fn maps_response_function_tool_to_chat_tool_spec() {
        let request: ResponsesRequest =
            serde_json::from_value(merge_strict_responses_request_defaults(json!({
                "model": "gpt-5.4",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }],
                "tools": [{
                    "type": "function",
                    "name": "shell",
                    "description": "Run a command",
                    "parameters": {
                        "properties": {
                            "command": { "type": "array" },
                            "workdir": { "type": "string", "format": "uri-reference" }
                        }
                    }
                }]
            })))
            .expect("request should parse");

        let tool = ToolSpec::from_response_tool(&request.tools[0]).expect("tool should map");
        let body = serde_json::to_value(&tool).expect("tool should serialize");

        assert_eq!(body["type"], "function");
        assert_eq!(body["function"]["name"], "shell");
        assert_eq!(body["function"]["description"], "Run a command");
        assert_eq!(body["function"]["parameters"]["type"], "object");
        assert!(body["function"]["parameters"]["properties"]["workdir"]
            .get("format")
            .is_none());
    }

    #[test]
    fn maps_native_response_tools_to_chat_function_specs() {
        let request: ResponsesRequest =
            serde_json::from_value(merge_strict_responses_request_defaults(json!({
                "model": "gpt-5.4",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }],
                "tools": [
                    { "type": "local_shell" },
                    { "type": "web_search" }
                ]
            })))
            .expect("request should parse");

        let body = serde_json::to_value(
            request
                .tools
                .iter()
                .filter_map(ToolSpec::from_response_tool)
                .collect::<Vec<_>>(),
        )
        .expect("tools should serialize");

        assert_eq!(body[0]["function"]["name"], "shell");
        assert_eq!(body[0]["function"]["parameters"]["required"][0], "command");
        assert_eq!(body[1]["function"]["name"], "google_search");
        assert_eq!(body[1]["function"]["parameters"]["required"][0], "query");
    }

    #[test]
    fn builds_chat_request_from_responses_request() {
        let request: ResponsesRequest =
            serde_json::from_value(merge_strict_responses_request_defaults(json!({
                "model": "gpt-5.4",
                "instructions": "be careful",
                "input": [
                    { "type": "function_call_output", "call_id": "call_1", "output": "ok", "name": "shell" },
                    { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{\"command\":[\"pwd\"]}" }
                ],
                "tools": [{ "type": "apply_patch" }],
                "tool_choice": "auto"
            })))
            .expect("request should parse");

        let chat = ChatRequest::try_from(&request).expect("chat request should map");
        let body = serde_json::to_value(chat).expect("chat should serialize");

        assert_eq!(body["model"], "gpt-5.4");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["tools"][0]["function"]["name"], "apply_patch");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["messages"][2]["role"], "tool");
        assert_eq!(body["messages"][2]["tool_call_id"], "call_1");
    }
}
