use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatCompletionCreateParams {
    pub messages: Vec<ChatCompletionMessageParam>,
    pub model: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ChatCompletionTool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionMessageParam {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChatCompletionMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionMessageToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ChatCompletionMessageContent {
    String(String),
    Array(Vec<ChatCompletionContentPart>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ChatCompletionContentPart {
    #[serde(rename = "text", alias = "input_text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl {
        image_url: ChatCompletionContentPartImageUrl,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionContentPartImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionMessageToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ChatCompletionMessageToolCallFunction,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionMessageToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionTool {
    pub r#type: String,
    pub function: ChatCompletionFunctionToolFunction,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionFunctionToolFunction {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatCompletionTool>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct ChatCompletionChunk {
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
pub(crate) struct ChatCompletionChunkChoice {
    #[serde(default)]
    pub index: u64,
    #[serde(default)]
    pub delta: ChatCompletionChunkDelta,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub(crate) struct ChatCompletionChunkDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub tool_calls: Vec<ChatCompletionToolCallDelta>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct ChatCompletionToolCallDelta {
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<ChatCompletionToolFunctionDelta>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct ChatCompletionToolFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct ChatCompletionUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub prompt_tokens_details: Option<CompletionUsagePromptTokensDetails>,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionUsageCompletionTokensDetails>,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub(crate) struct CompletionUsagePromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub(crate) struct CompletionUsageCompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: u64,
}

fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
