use super::super::responses::{
    ResponseContentPart, ResponseCreateParams, ResponseItemStatus, ResponseObject,
    ResponseOutputItem, ResponseStatus, ResponseStreamEvent, ResponseStreamFrame,
    ToolSpec as ResponsesToolSpec,
};
use super::types::{
    ChatCompletionChunk, ChatCompletionContentPart, ChatCompletionContentPartImageUrl,
    ChatCompletionCreateParams, ChatCompletionFunctionToolFunction, ChatCompletionMessageContent,
    ChatCompletionMessageParam, ChatCompletionMessageToolCall,
    ChatCompletionMessageToolCallFunction, ChatCompletionTool as ChatToolSpec,
    ChatCompletionToolCallDelta, ChatCompletionUsage,
};
use crate::models::{
    ContentItem, FunctionCallOutputPayload, LocalShellAction, LocalShellExecAction, MessagePhase,
    ResponseItem, ResponseUsage, ResponseUsageInputTokensDetails, ResponseUsageOutputTokensDetails,
    WebSearchAction,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

pub(crate) fn responses_to_chat_completions(
    request: &ResponseCreateParams,
    model: &str,
) -> Result<ChatCompletionCreateParams, String> {
    let mut body = ChatCompletionCreateParams::try_from(request)?;
    body.model = model.to_string();
    Ok(body)
}

impl ChatCompletionCreateParams {
    fn normalize_messages_for_chat_completions(messages: &mut Vec<ChatCompletionMessageParam>) {
        for message in &mut *messages {
            if message.role == "developer" {
                message.role = "system".to_string();
            }
        }
        reorder_tool_messages_for_chat_completions(messages);
    }
}

impl TryFrom<&ResponseCreateParams> for ChatCompletionCreateParams {
    type Error = String;

    fn try_from(request: &ResponseCreateParams) -> Result<Self, Self::Error> {
        let mut messages: Vec<ChatCompletionMessageParam> = request
            .input
            .iter()
            .filter_map(ChatCompletionMessageParam::from_responses_item)
            .collect();
        if let Some(instructions) = request
            .instructions
            .as_ref()
            .filter(|instructions| !instructions.trim().is_empty())
        {
            messages.insert(
                0,
                ChatCompletionMessageParam {
                    role: "system".to_string(),
                    content: Some(ChatCompletionMessageContent::String(
                        instructions.clone(),
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            );
        }
        Self::normalize_messages_for_chat_completions(&mut messages);

        Ok(Self {
            messages,
            model: request.model.clone(),
            stream: request.stream,
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
    fn from_responses_tool_spec(tool: &ResponsesToolSpec) -> Self {
        match tool {
            ResponsesToolSpec::Function(tool) => ChatToolSpec {
                r#type: "function".to_string(),
                function: ChatCompletionFunctionToolFunction {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: Some(tool.parameters.clone()),
                    tools: None,
                },
            },
            ResponsesToolSpec::Namespace(namespace) => ChatToolSpec {
                r#type: "function".to_string(),
                function: ChatCompletionFunctionToolFunction {
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
                                    function: ChatCompletionFunctionToolFunction {
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
                function: ChatCompletionFunctionToolFunction {
                    name: tool.name().to_string(),
                    description: Some(description.clone()),
                    parameters: Some(parameters.clone()),
                    tools: None,
                },
            },
            ResponsesToolSpec::LocalShell {} => ChatToolSpec {
                r#type: "function".to_string(),
                function: ChatCompletionFunctionToolFunction {
                    name: tool.name().to_string(),
                    description: None,
                    parameters: None,
                    tools: None,
                },
            },
            ResponsesToolSpec::ImageGeneration { .. } => ChatToolSpec {
                r#type: "function".to_string(),
                function: ChatCompletionFunctionToolFunction {
                    name: tool.name().to_string(),
                    description: None,
                    parameters: None,
                    tools: None,
                },
            },
            ResponsesToolSpec::WebSearch { .. } => ChatToolSpec {
                r#type: "function".to_string(),
                function: ChatCompletionFunctionToolFunction {
                    name: tool.name().to_string(),
                    description: None,
                    parameters: None,
                    tools: None,
                },
            },
            ResponsesToolSpec::Freeform(tool) => ChatToolSpec {
                r#type: "function".to_string(),
                function: ChatCompletionFunctionToolFunction {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: None,
                    tools: None,
                },
            },
        }
    }
}

fn reorder_tool_messages_for_chat_completions(messages: &mut Vec<ChatCompletionMessageParam>) {
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

    let mut tool_outputs_by_assistant_idx: HashMap<usize, Vec<ChatCompletionMessageParam>> =
        HashMap::new();
    let mut rewritten: Vec<ChatCompletionMessageParam> = Vec::with_capacity(original.len());
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

fn assistant_content_has_think_markup(content: &[ContentItem]) -> bool {
    content.iter().any(|item| match item {
        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
            text.contains("<think>") || text.contains("</think>")
        }
        ContentItem::InputImage { .. } | ContentItem::InputFile { .. } => false,
    })
}

fn local_shell_call_to_message(
    call_id: Option<&String>,
    action: &LocalShellAction,
) -> ChatCompletionMessageParam {
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
    ChatCompletionMessageParam {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ChatCompletionMessageToolCall {
            id: call_id,
            r#type: "function".to_string(),
            function: ChatCompletionMessageToolCallFunction {
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
) -> ChatCompletionMessageParam {
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
    ChatCompletionMessageParam {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ChatCompletionMessageToolCall {
            id: call_id,
            r#type: "function".to_string(),
            function: ChatCompletionMessageToolCallFunction {
                name: "web_search".to_string(),
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
) -> ChatCompletionMessageParam {
    ChatCompletionMessageParam {
        role: "tool".to_string(),
        content: Some(ChatCompletionMessageContent::String(
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
) -> ChatCompletionMessageParam {
    ChatCompletionMessageParam {
        role: "tool".to_string(),
        content: Some(ChatCompletionMessageContent::String(
            output.body.to_text().unwrap_or_else(|| output.to_string()),
        )),
        tool_calls: None,
        tool_call_id: Some(call_id.to_string()),
        name: name.cloned(),
    }
}

impl ChatCompletionContentPart {
    fn from_content_item(item: &ContentItem) -> Self {
        match item {
            ContentItem::InputText { text } => Self::Text { text: text.clone() },
            ContentItem::OutputText { text } => Self::Text { text: text.clone() },
            ContentItem::InputImage { image_url, .. } => Self::ImageUrl {
                image_url: ChatCompletionContentPartImageUrl {
                    url: image_url.clone().unwrap_or(String::new()),
                },
            },
            ContentItem::InputFile { .. } => Self::Text {
                text: "".to_string(),
            },
        }
    }
}

impl ChatCompletionMessageParam {
    fn from_responses_item(item: &ResponseItem) -> Option<Self> {
        match item {
            ResponseItem::Message { role, content, .. } => {
                if role == "assistant" && assistant_content_has_think_markup(content) {
                    return None;
                }

                Some(ChatCompletionMessageParam {
                    role: role.to_string(),
                    content: Some(ChatCompletionMessageContent::Array(
                        content
                            .iter()
                            .map(ChatCompletionContentPart::from_content_item)
                            .collect(),
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                })
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => Some(Self {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![ChatCompletionMessageToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
                    function: ChatCompletionMessageToolCallFunction {
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                }]),
                tool_call_id: None,
                name: None,
            }),
            ResponseItem::LocalShellCall {
                call_id, action, ..
            } => Some(local_shell_call_to_message(call_id.as_ref(), action)),
            ResponseItem::WebSearchCall { id, action, .. } => {
                Some(web_search_call_to_message(id.as_ref(), action))
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => Some(function_call_output_to_message(call_id, output)),
            ResponseItem::CustomToolCallOutput {
                call_id,
                name,
                output,
                ..
            } => Some(custom_tool_call_output_to_message(
                call_id,
                name.as_ref(),
                output,
            )),
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => Some(ChatCompletionMessageParam {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![ChatCompletionMessageToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
                    function: ChatCompletionMessageToolCallFunction {
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
            | ResponseItem::ImageGenerationCall { .. } => None,
            ResponseItem::Other => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ChatCompletionChunkResponses {
    response_id: Option<String>,
    response_model: Option<String>,
    created_at: Option<u64>,
    text_deltas: Vec<String>,
    tool_call_deltas: Vec<ChatCompletionToolCallDelta>,
    usage: Option<ResponseUsage>,
}

impl ChatCompletionChunk {
    fn to_responses(&self) -> ChatCompletionChunkResponses {
        let mut text_deltas = Vec::new();
        let mut tool_call_deltas = Vec::new();

        for choice in self.choices.iter().filter(|choice| choice.index == 0) {
            if let Some(content) = choice
                .delta
                .content
                .as_ref()
                .filter(|content| !content.is_empty())
            {
                text_deltas.push(content.clone());
            }
            tool_call_deltas.extend(choice.delta.tool_calls.iter().cloned());
        }

        ChatCompletionChunkResponses {
            response_id: self.id.clone(),
            response_model: self.model.clone(),
            created_at: self.created,
            text_deltas,
            tool_call_deltas,
            usage: self.usage.as_ref().map(ChatCompletionUsage::to_responses),
        }
    }
}

impl ChatCompletionUsage {
    fn to_responses(&self) -> ResponseUsage {
        ResponseUsage {
            input_tokens: self.prompt_tokens as i64,
            input_tokens_details: ResponseUsageInputTokensDetails {
                cached_tokens: self
                    .prompt_tokens_details
                    .as_ref()
                    .map(|details| details.cached_tokens)
                    .unwrap_or(0) as i64,
            },
            output_tokens: self.completion_tokens as i64,
            output_tokens_details: ResponseUsageOutputTokensDetails {
                reasoning_tokens: self
                    .completion_tokens_details
                    .as_ref()
                    .map(|details| details.reasoning_tokens)
                    .unwrap_or(0) as i64,
            },
            total_tokens: self.total_tokens as i64,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ChatCompletionsResponsesStream {
    requested_model: String,
    created_at: u64,
    response_id: Option<String>,
    response_model: Option<String>,
    created_emitted: bool,
    message_started: bool,
    finished: bool,
    message_item_id: String,
    text: String,
    tool_calls: BTreeMap<usize, StreamedChatToolCall>,
    usage: Option<ResponseUsage>,
    sequence_number: u64,
}

#[derive(Debug)]
struct StreamedChatToolCall {
    id: String,
    item_id: String,
    name: String,
    arguments: String,
}

impl ChatCompletionsResponsesStream {
    pub(crate) fn new(requested_model: String, created_at: u64) -> Self {
        Self {
            requested_model,
            created_at,
            response_id: None,
            response_model: None,
            created_emitted: false,
            message_started: false,
            finished: false,
            message_item_id: format!("msg_{}", Uuid::new_v4().simple()),
            text: String::new(),
            tool_calls: BTreeMap::new(),
            usage: None,
            sequence_number: 1,
        }
    }

    pub(crate) fn push_chat_payload(
        &mut self,
        payload: &str,
    ) -> Result<Vec<ResponseStreamFrame>, String> {
        if payload == "[DONE]" {
            return self.finish();
        }

        let chunk: ChatCompletionChunk =
            serde_json::from_str(payload).map_err(|err| err.to_string())?;
        let response_chunk = chunk.to_responses();
        if let Some(id) = response_chunk.response_id {
            self.response_id.get_or_insert(id);
        }
        if let Some(model) = response_chunk.response_model {
            self.response_model.get_or_insert(model);
        }
        if let Some(created_at) = response_chunk.created_at {
            self.created_at = created_at;
        }
        if let Some(usage) = response_chunk.usage {
            self.usage = Some(usage);
        }

        let mut events = self.ensure_created().map_err(|err| err.to_string())?;
        for delta in response_chunk.text_deltas {
            events.extend(
                self.push_text_delta(&delta)
                    .map_err(|err| err.to_string())?,
            );
        }
        self.push_tool_call_deltas(&response_chunk.tool_call_deltas);

        Ok(events)
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<ResponseStreamFrame>, String> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;

        let mut events = self.ensure_created().map_err(|err| err.to_string())?;
        if self.message_started {
            let message = self.message_item();
            events.push(self.event(ResponseStreamEvent::OutputTextDone {
                item_id: self.message_item_id.clone(),
                output_index: 0,
                content_index: 0,
                text: self.text.clone(),
                sequence_number: 0,
            }));
            events.push(self.event(ResponseStreamEvent::ContentPartDone {
                item_id: self.message_item_id.clone(),
                output_index: 0,
                content_index: 0,
                part: ResponseContentPart::OutputText {
                    text: self.text.clone(),
                    annotations: Vec::new(),
                },
                sequence_number: 0,
            }));
            events.push(self.event(ResponseStreamEvent::OutputItemDone {
                output_index: 0,
                item: message,
                sequence_number: 0,
            }));
        }

        let mut next_output_index = usize::from(self.message_started);
        let tool_calls = self
            .tool_calls
            .values()
            .map(StreamedChatToolCall::response_item)
            .collect::<Vec<_>>();
        for item in tool_calls {
            events.push(self.event(ResponseStreamEvent::OutputItemAdded {
                output_index: next_output_index,
                item: item.clone(),
                sequence_number: 0,
            }));
            events.push(self.event(ResponseStreamEvent::OutputItemDone {
                output_index: next_output_index,
                item,
                sequence_number: 0,
            }));
            next_output_index += 1;
        }

        let response = self.completed_response();
        events.push(self.event(ResponseStreamEvent::Completed {
            response,
            sequence_number: 0,
        }));
        events.push(ResponseStreamFrame::Done);

        Ok(events)
    }

    fn ensure_created(&mut self) -> Result<Vec<ResponseStreamFrame>, serde_json::Error> {
        if self.created_emitted {
            return Ok(Vec::new());
        }
        self.created_emitted = true;
        let response_id = self.response_id();
        let response_model = self.response_model().to_string();
        let response = ResponseObject {
            id: response_id,
            object: "response",
            created_at: self.created_at,
            status: ResponseStatus::InProgress,
            model: response_model,
            output: Vec::new(),
            usage: None,
            end_turn: None,
        };
        Ok(vec![self.event(ResponseStreamEvent::Created {
            response,
            sequence_number: 0,
        })])
    }

    fn push_text_delta(
        &mut self,
        delta: &str,
    ) -> Result<Vec<ResponseStreamFrame>, serde_json::Error> {
        let mut events = Vec::new();
        if !self.message_started {
            self.message_started = true;
            events.push(self.event(ResponseStreamEvent::OutputItemAdded {
                output_index: 0,
                item: ResponseOutputItem::Message {
                    id: self.message_item_id.clone(),
                    role: "assistant".to_string(),
                    content: Vec::new(),
                    status: ResponseItemStatus::InProgress,
                    phase: None,
                },
                sequence_number: 0,
            }));
            events.push(self.event(ResponseStreamEvent::ContentPartAdded {
                item_id: self.message_item_id.clone(),
                output_index: 0,
                content_index: 0,
                part: ResponseContentPart::OutputText {
                    text: String::new(),
                    annotations: Vec::new(),
                },
                sequence_number: 0,
            }));
        }

        self.text.push_str(delta);
        events.push(self.event(ResponseStreamEvent::OutputTextDelta {
            item_id: self.message_item_id.clone(),
            output_index: 0,
            content_index: 0,
            delta: delta.to_string(),
            sequence_number: 0,
        }));

        Ok(events)
    }

    fn push_tool_call_deltas(&mut self, tool_call_deltas: &[ChatCompletionToolCallDelta]) {
        for tool_call_delta in tool_call_deltas {
            let index = tool_call_delta
                .index
                .unwrap_or(self.tool_calls.len() as u64) as usize;
            let tool_call = self
                .tool_calls
                .entry(index)
                .or_insert_with(|| StreamedChatToolCall {
                    id: format!("call_{}", Uuid::new_v4().simple()),
                    item_id: format!("fc_{}", Uuid::new_v4().simple()),
                    name: "unknown".to_string(),
                    arguments: String::new(),
                });

            if let Some(id) = &tool_call_delta.id {
                tool_call.id = id.to_string();
            }
            let Some(function) = &tool_call_delta.function else {
                continue;
            };
            if let Some(name) = function.name.as_ref().filter(|name| !name.is_empty()) {
                tool_call.name = name.to_string();
            }
            if let Some(arguments) = &function.arguments {
                tool_call.arguments.push_str(arguments);
            }
        }
    }

    fn completed_response(&mut self) -> ResponseObject {
        let mut output = Vec::new();
        if self.message_started {
            output.push(self.message_item());
        }
        output.extend(
            self.tool_calls
                .values()
                .map(StreamedChatToolCall::response_item),
        );

        ResponseObject {
            id: self.response_id(),
            object: "response",
            created_at: self.created_at,
            status: ResponseStatus::Completed,
            model: self.response_model().to_string(),
            output,
            usage: self.usage.clone(),
            end_turn: Some(true),
        }
    }

    fn message_item(&self) -> ResponseOutputItem {
        ResponseOutputItem::Message {
            id: self.message_item_id.clone(),
            role: "assistant".to_string(),
            content: vec![ResponseContentPart::OutputText {
                text: self.text.clone(),
                annotations: Vec::new(),
            }],
            status: ResponseItemStatus::Completed,
            phase: Some(MessagePhase::FinalAnswer),
        }
    }

    fn response_id(&mut self) -> String {
        self.response_id
            .get_or_insert_with(|| format!("resp_{}", Uuid::new_v4().simple()))
            .clone()
    }

    fn response_model(&self) -> &str {
        self.response_model
            .as_deref()
            .unwrap_or(&self.requested_model)
    }

    fn event(&mut self, event: ResponseStreamEvent) -> ResponseStreamFrame {
        ResponseStreamFrame::Event(event.with_sequence_number(self.next_sequence_number()))
    }

    fn next_sequence_number(&mut self) -> u64 {
        let sequence_number = self.sequence_number;
        self.sequence_number += 1;
        sequence_number
    }
}

impl StreamedChatToolCall {
    fn response_item(&self) -> ResponseOutputItem {
        ResponseOutputItem::FunctionCall {
            id: self.item_id.clone(),
            name: self.name.clone(),
            namespace: None,
            arguments: self.arguments.clone(),
            call_id: self.id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatCompletionChunk, ChatCompletionCreateParams, ChatCompletionMessageParam, ChatToolSpec,
        ContentItem, responses_to_chat_completions,
    };
    use crate::models::ResponseItem;
    use crate::models::openai::responses::{
        ResponseCreateParams, merge_strict_responses_request_defaults,
    };
    use serde_json::json;

    fn chat_body(request: &ResponseCreateParams, model: &str) -> serde_json::Value {
        serde_json::to_value(
            responses_to_chat_completions(request, model).expect("request should convert"),
        )
        .expect("chat request should serialize")
    }

    #[test]
    fn maps_response_function_tool_to_chat_tool_spec() {
        let request: ResponseCreateParams =
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
        let request: ResponseCreateParams =
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
        let request: ResponseCreateParams =
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

        let chat = ChatCompletionCreateParams::try_from(&request).expect("chat request should map");
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
    fn omits_assistant_message_when_any_text_has_think_markup() {
        let msg = ChatCompletionMessageParam::from_responses_item(&ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![
                ContentItem::OutputText {
                    text: "<think>hidden</think>".to_string(),
                },
                ContentItem::OutputText {
                    text: "visible<think>secret</think>".to_string(),
                },
            ],
            phase: None,
        });

        assert!(msg.is_none());
    }

    #[test]
    fn omits_assistant_message_when_mixed_clean_and_think_markup_text() {
        let msg = ChatCompletionMessageParam::from_responses_item(&ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![
                ContentItem::OutputText {
                    text: "looks clean".to_string(),
                },
                ContentItem::OutputText {
                    text: "<think>x</think>".to_string(),
                },
            ],
            phase: None,
        });

        assert!(msg.is_none());
    }

    #[test]
    fn drops_think_only_assistant_messages_before_tool_outputs() {
        let request: ResponseCreateParams =
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

        let chat = ChatCompletionCreateParams::try_from(&request).expect("chat request should map");
        let body = serde_json::to_value(chat).expect("chat should serialize");

        assert_eq!(body["messages"].as_array().map(Vec::len), Some(2));
        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["tool_call_id"], "call_1");
    }

    #[test]
    fn preserves_lowercase_json_schema_for_chat_tools() {
        let request: ResponseCreateParams =
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
                        "type": "object",
                        "properties": {
                            "command": { "type": "array" },
                            "workdir": { "type": "string", "format": "uri-reference" }
                        }
                    }
                }]
            })))
            .expect("request should parse");

        let body = chat_body(&request, "chat-compatible-latest");
        let parameters = &body["tools"][0]["function"]["parameters"];

        assert_eq!(parameters["type"], "object");
        assert_eq!(parameters["properties"]["command"]["type"], "array");
        assert_eq!(parameters["properties"]["workdir"]["type"], "string");
        assert_eq!(
            parameters["properties"]["workdir"]["format"],
            "uri-reference"
        );
    }

    #[test]
    fn forwards_responses_stream_flag_to_chat_completions() {
        let request: ResponseCreateParams =
            serde_json::from_value(merge_strict_responses_request_defaults(json!({
                "model": "gpt-5.4",
                "stream": true,
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }]
            })))
            .expect("request should parse");

        let body = chat_body(&request, "chat-compatible-latest");

        assert_eq!(body["stream"], true);
    }

    #[test]
    fn reorders_tool_outputs_to_follow_assistant_tool_calls_for_chat_completions() {
        let request: ResponseCreateParams = serde_json::from_value(merge_strict_responses_request_defaults(json!({
            "model": "gpt-5.4",
            "input": [
                { "type": "function_call_output", "call_id": "call_1", "output": "ok", "name": "shell" },
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{\"command\":[\"pwd\"]}" }
            ]
        })))
        .expect("request should parse");

        let body = chat_body(&request, "chat-compatible-latest");
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert!(messages[0].get("tool_calls").is_some());
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
    }

    #[test]
    fn demotes_unmatched_tool_outputs_to_user_messages_for_chat_completions() {
        let request: ResponseCreateParams = serde_json::from_value(merge_strict_responses_request_defaults(json!({
            "model": "gpt-5.4",
            "input": [
                { "type": "function_call_output", "call_id": "call_orphan", "output": "orphaned", "name": "shell" }
            ]
        })))
        .expect("request should parse");

        let body = chat_body(&request, "chat-compatible-latest");
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert!(messages[0].get("tool_call_id").is_none());
    }

    #[test]
    fn maps_chat_completion_chunk_text_and_usage_to_responses() {
        let chunk: ChatCompletionChunk = serde_json::from_str(
            r#"{
                "id": "chatcmpl_1",
                "created": 1700000000,
                "model": "chat-model",
                "choices": [
                    { "index": 0, "delta": { "content": "hi" } },
                    { "index": 1, "delta": { "content": "ignored" } }
                ],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 2,
                    "total_tokens": 5
                }
            }"#,
        )
        .expect("chunk should parse");

        let responses = chunk.to_responses();

        assert_eq!(responses.response_id.as_deref(), Some("chatcmpl_1"));
        assert_eq!(responses.response_model.as_deref(), Some("chat-model"));
        assert_eq!(responses.created_at, Some(1700000000));
        assert_eq!(responses.text_deltas, vec!["hi"]);
        assert_eq!(
            responses.usage.as_ref().map(|usage| usage.input_tokens),
            Some(3)
        );
        assert_eq!(
            responses.usage.as_ref().map(|usage| usage.output_tokens),
            Some(2)
        );
    }

    #[test]
    fn maps_chat_completion_tool_call_delta_to_responses() {
        let chunk: ChatCompletionChunk = serde_json::from_str(
            r#"{
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_1",
                            "function": {
                                "name": "lookup",
                                "arguments": "{\"q\":\"test\"}"
                            }
                        }]
                    }
                }]
            }"#,
        )
        .expect("chunk should parse");

        let responses = chunk.to_responses();
        let tool_call = responses
            .tool_call_deltas
            .first()
            .expect("tool call delta should be present");

        assert_eq!(tool_call.index, Some(0));
        assert_eq!(tool_call.id.as_deref(), Some("call_1"));
        assert_eq!(
            tool_call
                .function
                .as_ref()
                .and_then(|function| function.name.as_deref()),
            Some("lookup")
        );
    }
}
