use super::super::responses::{ResponsesRequest, ToolSpec as ResponsesToolSpec};
use super::{
    ChatRequest, Content, ContentBlock, ImageUrl, Message, ToolCall, ToolFunction,
    ToolSpec as ChatToolSpec, ToolSpecFunction,
};
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
                .map(ChatToolSpec::from_responses_tool_spec)
                .collect(),
        })
    }
}

impl ChatToolSpec {
    pub fn from_responses_tool_spec(tool: &ResponsesToolSpec) -> Self {
        match tool {
            ResponsesToolSpec::Function(tool) => ChatToolSpec {
                r#type: "function".to_string(),
                function: ToolSpecFunction {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: Some(tool.parameters.clone()),
                    tools: None,
                },
            },
            ResponsesToolSpec::Namespace(namespace) => ChatToolSpec {
                r#type: "function".to_string(),
                function: ToolSpecFunction {
                    name: tool.name().to_string(),
                    description: None,
                    parameters: None,
                    tools: Some(
                        namespace
                            .tools
                            .iter()
                            .map(|tool| match tool {
                                super::super::responses::ResponsesApiNamespaceTool::Function(
                                    tool,
                                ) => ChatToolSpec {
                                    r#type: "function".to_string(),
                                    function: ToolSpecFunction {
                                        name: tool.name.clone(),
                                        description: Some(tool.description.clone()),
                                        parameters: Some(tool.parameters.clone()),
                                        tools: None,
                                    },
                                },
                            })
                            .collect(),
                    ),
                },
            },
            ResponsesToolSpec::ToolSearch {
                description,
                parameters,
                ..
            } => ChatToolSpec {
                r#type: "function".to_string(),
                function: ToolSpecFunction {
                    name: tool.name().to_string(),
                    description: Some(description.clone()),
                    parameters: Some(parameters.clone()),
                    tools: None,
                },
            },
            ResponsesToolSpec::LocalShell {} => ChatToolSpec {
                r#type: "function".to_string(),
                function: ToolSpecFunction {
                    name: tool.name().to_string(),
                    description: None,
                    parameters: None,
                    tools: None,
                },
            },
            ResponsesToolSpec::ImageGeneration { .. } => ChatToolSpec {
                r#type: "function".to_string(),
                function: ToolSpecFunction {
                    name: tool.name().to_string(),
                    description: None,
                    parameters: None,
                    tools: None,
                },
            },
            ResponsesToolSpec::WebSearch { .. } => ChatToolSpec {
                r#type: "function".to_string(),
                function: ToolSpecFunction {
                    name: tool.name().to_string(),
                    description: None,
                    parameters: None,
                    tools: None,
                },
            },
            ResponsesToolSpec::Freeform(tool) => ChatToolSpec {
                r#type: "function".to_string(),
                function: ToolSpecFunction {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: None,
                    tools: None,
                },
            },
        }
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
                if let Some(message) = response_message_to_openai(role, content) {
                    messages.push(message);
                }
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

fn strip_think_tags(text: &str) -> String {
    const START_TAG: &str = "<think>";
    const END_TAG: &str = "</think>";

    let mut remaining = text;
    let mut sanitized = String::new();

    loop {
        let Some(start) = remaining.find(START_TAG) else {
            sanitized.push_str(remaining);
            break;
        };

        sanitized.push_str(&remaining[..start]);
        let after_start = &remaining[start + START_TAG.len()..];

        let Some(end) = after_start.find(END_TAG) else {
            break;
        };

        remaining = &after_start[end + END_TAG.len()..];
    }

    sanitized
}

fn sanitize_message_content_for_context(role: &str, content: &[ContentItem]) -> Vec<ContentItem> {
    if role != "assistant" {
        return content.to_vec();
    }

    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } => {
                let sanitized = strip_think_tags(text);
                (!sanitized.trim().is_empty()).then_some(ContentItem::InputText { text: sanitized })
            }
            ContentItem::OutputText { text } => {
                let sanitized = strip_think_tags(text);
                (!sanitized.trim().is_empty())
                    .then_some(ContentItem::OutputText { text: sanitized })
            }
            ContentItem::InputImage { .. } | ContentItem::InputFile { .. } => Some(item.clone()),
        })
        .collect()
}

fn response_message_to_openai(role: &str, content: &[ContentItem]) -> Option<Message> {
    let sanitized_content = sanitize_message_content_for_context(role, content);
    if role == "assistant" && sanitized_content.is_empty() {
        return None;
    }

    Some(Message {
        role: role.to_string(),
        content: content_items_to_openai(&sanitized_content),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    })
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

fn local_shell_call_to_message(call_id: Option<&String>, action: &LocalShellAction) -> Message {
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

fn function_call_output_to_message(call_id: &str, output: &FunctionCallOutputPayload) -> Message {
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
    use super::{
        ChatRequest, ChatToolSpec, ContentItem, sanitize_message_content_for_context,
        strip_think_tags,
    };
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

        let tool = ChatToolSpec::from_responses_tool_spec(&request.tools[0]);
        let body = serde_json::to_value(&tool).expect("tool should serialize");

        assert_eq!(body["type"], "function");
        assert_eq!(body["function"]["name"], "shell");
        assert_eq!(body["function"]["description"], "Run a command");
        assert_eq!(
            body["function"]["parameters"]["properties"]["workdir"]["format"],
            "uri-reference"
        );
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
                .map(ChatToolSpec::from_responses_tool_spec)
                .collect::<Vec<_>>(),
        )
        .expect("tools should serialize");

        assert_eq!(body[0]["function"]["name"], "local_shell");
        assert!(body[0]["function"].get("parameters").is_none());
        assert_eq!(body[1]["function"]["name"], "web_search");
        assert!(body[1]["function"].get("parameters").is_none());
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
                "tools": [{ "type": "local_shell" }],
                "tool_choice": "auto"
            })))
            .expect("request should parse");

        let chat = ChatRequest::try_from(&request).expect("chat request should map");
        let body = serde_json::to_value(chat).expect("chat should serialize");

        assert_eq!(body["model"], "gpt-5.4");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["tools"][0]["function"]["name"], "local_shell");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["messages"][2]["role"], "tool");
        assert_eq!(body["messages"][2]["tool_call_id"], "call_1");
    }

    #[test]
    fn strips_multiple_think_blocks_from_text() {
        let sanitized =
            strip_think_tags("before<think>hidden</think>middle<think>more</think>after");
        assert_eq!(sanitized, "beforemiddleafter");
    }

    #[test]
    fn drops_assistant_text_items_that_only_contain_think_blocks() {
        let sanitized = sanitize_message_content_for_context(
            "assistant",
            &[
                ContentItem::OutputText {
                    text: "<think>hidden</think>".to_string(),
                },
                ContentItem::OutputText {
                    text: "visible<think>secret</think>".to_string(),
                },
            ],
        );

        assert_eq!(sanitized.len(), 1);
        assert_eq!(
            sanitized[0],
            ContentItem::OutputText {
                text: "visible".to_string(),
            }
        );
    }

    #[test]
    fn drops_think_only_assistant_messages_before_tool_outputs() {
        let request: ResponsesRequest =
            serde_json::from_value(merge_strict_responses_request_defaults(json!({
                "model": "gpt-5.4",
                "input": [
                    {
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "shell",
                        "arguments": "{\"command\":[\"pwd\"]}"
                    },
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "<think>running command</think>" }]
                    },
                    {
                        "type": "function_call_output",
                        "call_id": "call_1",
                        "output": "ok"
                    }
                ]
            })))
            .expect("request should parse");

        let chat = ChatRequest::try_from(&request).expect("chat request should map");
        let body = serde_json::to_value(chat).expect("chat should serialize");

        assert_eq!(body["messages"].as_array().map(Vec::len), Some(2));
        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["tool_call_id"], "call_1");
    }
}
