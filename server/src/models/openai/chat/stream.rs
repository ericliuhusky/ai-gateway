use super::super::responses::{
    ResponsesStreamContentPart, ResponsesStreamEvent, ResponsesStreamFrame,
    ResponsesStreamOutputItem, ResponsesStreamResponse, ResponsesStreamResponseStatus, TokenUsage,
};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChatCompletionChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub choices: Vec<ChatCompletionChunkChoice>,
    #[serde(default)]
    pub usage: Option<ChatCompletionUsage>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChatCompletionChunkChoice {
    #[serde(default)]
    pub index: u64,
    #[serde(default)]
    pub delta: ChatCompletionChunkDelta,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ChatCompletionChunkDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub tool_calls: Vec<ChatCompletionToolCallDelta>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChatCompletionToolCallDelta {
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<ChatCompletionToolFunctionDelta>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChatCompletionToolFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChatCompletionUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatCompletionChunkResponses {
    pub response_id: Option<String>,
    pub response_model: Option<String>,
    pub created_at: Option<u64>,
    pub text_deltas: Vec<String>,
    pub tool_call_deltas: Vec<ChatCompletionToolCallDelta>,
    pub usage: Option<TokenUsage>,
}

impl ChatCompletionChunk {
    pub fn to_responses(&self) -> ChatCompletionChunkResponses {
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
    pub fn to_responses(&self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.prompt_tokens as i64,
            cached_input_tokens: 0,
            output_tokens: self.completion_tokens as i64,
            reasoning_output_tokens: 0,
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
    usage: Option<TokenUsage>,
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
    ) -> Result<Vec<ResponsesStreamFrame>, String> {
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

    pub(crate) fn finish(&mut self) -> Result<Vec<ResponsesStreamFrame>, String> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;

        let mut events = self.ensure_created().map_err(|err| err.to_string())?;
        if self.message_started {
            let message = self.message_item();
            events.push(self.event(ResponsesStreamEvent::OutputTextDone {
                item_id: self.message_item_id.clone(),
                output_index: 0,
                content_index: 0,
                text: self.text.clone(),
                sequence_number: 0,
            }));
            events.push(self.event(ResponsesStreamEvent::ContentPartDone {
                item_id: self.message_item_id.clone(),
                output_index: 0,
                content_index: 0,
                part: ResponsesStreamContentPart::OutputText {
                    text: self.text.clone(),
                },
                sequence_number: 0,
            }));
            events.push(self.event(ResponsesStreamEvent::OutputItemDone {
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
            events.push(self.event(ResponsesStreamEvent::OutputItemAdded {
                output_index: next_output_index,
                item: item.clone(),
                sequence_number: 0,
            }));
            events.push(self.event(ResponsesStreamEvent::OutputItemDone {
                output_index: next_output_index,
                item,
                sequence_number: 0,
            }));
            next_output_index += 1;
        }

        let response = self.completed_response();
        events.push(self.event(ResponsesStreamEvent::Completed {
            response,
            sequence_number: 0,
        }));
        events.push(ResponsesStreamFrame::Done);

        Ok(events)
    }

    fn ensure_created(&mut self) -> Result<Vec<ResponsesStreamFrame>, serde_json::Error> {
        if self.created_emitted {
            return Ok(Vec::new());
        }
        self.created_emitted = true;
        let response_id = self.response_id();
        let response_model = self.response_model().to_string();
        let response = ResponsesStreamResponse {
            id: response_id,
            object: "response",
            created_at: self.created_at,
            status: ResponsesStreamResponseStatus::InProgress,
            model: response_model,
            output: Vec::new(),
            usage: None,
            end_turn: None,
        };
        Ok(vec![self.event(ResponsesStreamEvent::Created {
            response,
            sequence_number: 0,
        })])
    }

    fn push_text_delta(
        &mut self,
        delta: &str,
    ) -> Result<Vec<ResponsesStreamFrame>, serde_json::Error> {
        let mut events = Vec::new();
        if !self.message_started {
            self.message_started = true;
            events.push(self.event(ResponsesStreamEvent::OutputItemAdded {
                output_index: 0,
                item: ResponsesStreamOutputItem::Message {
                    id: self.message_item_id.clone(),
                    role: "assistant".to_string(),
                    content: Vec::new(),
                    phase: None,
                },
                sequence_number: 0,
            }));
            events.push(self.event(ResponsesStreamEvent::ContentPartAdded {
                item_id: self.message_item_id.clone(),
                output_index: 0,
                content_index: 0,
                part: ResponsesStreamContentPart::OutputText {
                    text: String::new(),
                },
                sequence_number: 0,
            }));
        }

        self.text.push_str(delta);
        events.push(self.event(ResponsesStreamEvent::OutputTextDelta {
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

    fn completed_response(&mut self) -> ResponsesStreamResponse {
        let mut output = Vec::new();
        if self.message_started {
            output.push(self.message_item());
        }
        output.extend(
            self.tool_calls
                .values()
                .map(StreamedChatToolCall::response_item),
        );

        ResponsesStreamResponse {
            id: self.response_id(),
            object: "response",
            created_at: self.created_at,
            status: ResponsesStreamResponseStatus::Completed,
            model: self.response_model().to_string(),
            output,
            usage: self.usage.clone(),
            end_turn: Some(true),
        }
    }

    fn message_item(&self) -> ResponsesStreamOutputItem {
        ResponsesStreamOutputItem::Message {
            id: self.message_item_id.clone(),
            role: "assistant".to_string(),
            content: vec![ResponsesStreamContentPart::OutputText {
                text: self.text.clone(),
            }],
            phase: Some(crate::models::MessagePhase::FinalAnswer),
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

    fn event(&mut self, event: ResponsesStreamEvent) -> ResponsesStreamFrame {
        ResponsesStreamFrame::Event(event.with_sequence_number(self.next_sequence_number()))
    }

    fn next_sequence_number(&mut self) -> u64 {
        let sequence_number = self.sequence_number;
        self.sequence_number += 1;
        sequence_number
    }
}

impl StreamedChatToolCall {
    fn response_item(&self) -> ResponsesStreamOutputItem {
        ResponsesStreamOutputItem::FunctionCall {
            id: self.item_id.clone(),
            name: self.name.clone(),
            namespace: None,
            arguments: self.arguments.clone(),
            call_id: self.id.clone(),
        }
    }
}

fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::ChatCompletionChunk;

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
