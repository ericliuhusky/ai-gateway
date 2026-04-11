use super::openai_chat::OpenAIContent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(default)]
    pub input: Option<ResponsesInput>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(rename = "max_output_tokens", alias = "max_tokens")]
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f64>,
    #[serde(rename = "top_p")]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub tools: Option<Vec<ResponseTool>>,
    #[serde(default)]
    #[allow(dead_code)]
    pub tool_choice: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    String(String),
    Array(Vec<ResponsesInputItem>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponsesInputItem {
    Message(ResponseMessageInput),
    Block(ResponsesInputBlock),
    FunctionCall(ResponseFunctionCallItem),
    LocalShellCall(ResponseLocalShellCallItem),
    WebSearchCall(ResponseWebSearchCallItem),
    FunctionCallOutput(ResponseFunctionCallOutputItem),
    CustomToolCallOutput(ResponseCustomToolCallOutputItem),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseMessageInput {
    pub role: String,
    #[serde(default)]
    pub content: Option<OpenAIContent>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ResponseFunctionToolCall>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ResponsesInputBlock {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "input_image")]
    InputImage { image_url: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFunctionToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFunctionCallItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseLocalShellCallItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub action: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseWebSearchCallItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub action: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFunctionCallOutputItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    pub call_id: String,
    pub output: Value,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseCustomToolCallOutputItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    pub call_id: String,
    pub output: Value,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<Value>,
    #[serde(default)]
    pub function: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponsesResponse {
    pub id: String,
    pub object: String,
    pub created_at: u64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponsesUsage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseOutputItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ResponseOutputContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseOutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponsesUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}
