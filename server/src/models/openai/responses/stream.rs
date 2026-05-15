use super::TokenUsage;
use crate::models::MessagePhase;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum ResponsesStreamEvent {
    #[serde(rename = "response.created")]
    Created {
        response: ResponsesStreamResponse,
        sequence_number: u64,
    },
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        output_index: usize,
        item: ResponsesStreamOutputItem,
        sequence_number: u64,
    },
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        item_id: String,
        output_index: usize,
        content_index: usize,
        part: ResponsesStreamContentPart,
        sequence_number: u64,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        item_id: String,
        output_index: usize,
        content_index: usize,
        delta: String,
        sequence_number: u64,
    },
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        item_id: String,
        output_index: usize,
        content_index: usize,
        text: String,
        sequence_number: u64,
    },
    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        item_id: String,
        output_index: usize,
        content_index: usize,
        part: ResponsesStreamContentPart,
        sequence_number: u64,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        output_index: usize,
        item: ResponsesStreamOutputItem,
        sequence_number: u64,
    },
    #[serde(rename = "response.completed")]
    Completed {
        response: ResponsesStreamResponse,
        sequence_number: u64,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResponsesStreamResponse {
    pub id: String,
    pub object: &'static str,
    pub created_at: u64,
    pub status: ResponsesStreamResponseStatus,
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<ResponsesStreamOutputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_turn: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesStreamResponseStatus {
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesStreamOutputItem {
    Message {
        id: String,
        role: String,
        content: Vec<ResponsesStreamContentPart>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<MessagePhase>,
    },
    FunctionCall {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        arguments: String,
        call_id: String,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesStreamContentPart {
    OutputText { text: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponsesStreamFrame {
    Event(ResponsesStreamEvent),
    Done,
}

impl ResponsesStreamEvent {
    pub fn with_sequence_number(self, sequence_number: u64) -> Self {
        match self {
            Self::Created { response, .. } => Self::Created {
                response,
                sequence_number,
            },
            Self::OutputItemAdded {
                output_index, item, ..
            } => Self::OutputItemAdded {
                output_index,
                item,
                sequence_number,
            },
            Self::ContentPartAdded {
                item_id,
                output_index,
                content_index,
                part,
                ..
            } => Self::ContentPartAdded {
                item_id,
                output_index,
                content_index,
                part,
                sequence_number,
            },
            Self::OutputTextDelta {
                item_id,
                output_index,
                content_index,
                delta,
                ..
            } => Self::OutputTextDelta {
                item_id,
                output_index,
                content_index,
                delta,
                sequence_number,
            },
            Self::OutputTextDone {
                item_id,
                output_index,
                content_index,
                text,
                ..
            } => Self::OutputTextDone {
                item_id,
                output_index,
                content_index,
                text,
                sequence_number,
            },
            Self::ContentPartDone {
                item_id,
                output_index,
                content_index,
                part,
                ..
            } => Self::ContentPartDone {
                item_id,
                output_index,
                content_index,
                part,
                sequence_number,
            },
            Self::OutputItemDone {
                output_index, item, ..
            } => Self::OutputItemDone {
                output_index,
                item,
                sequence_number,
            },
            Self::Completed { response, .. } => Self::Completed {
                response,
                sequence_number,
            },
        }
    }

    fn event_name(&self) -> &'static str {
        match self {
            Self::Created { .. } => "response.created",
            Self::OutputItemAdded { .. } => "response.output_item.added",
            Self::ContentPartAdded { .. } => "response.content_part.added",
            Self::OutputTextDelta { .. } => "response.output_text.delta",
            Self::OutputTextDone { .. } => "response.output_text.done",
            Self::ContentPartDone { .. } => "response.content_part.done",
            Self::OutputItemDone { .. } => "response.output_item.done",
            Self::Completed { .. } => "response.completed",
        }
    }
}

impl ResponsesStreamFrame {
    pub fn encode_sse(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Event(event) => serde_json::to_string(event)
                .map(|body| format!("event: {}\ndata: {body}\n\n", event.event_name())),
            Self::Done => Ok("data: [DONE]\n\n".to_string()),
        }
    }
}
