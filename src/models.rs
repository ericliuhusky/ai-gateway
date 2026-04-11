use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROVIDER_GOOGLE_PROXY: &str = "google-proxy";
pub const PROVIDER_OPENAI_PROXY: &str = "openai-proxy";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApiProviderBillingMode {
    Metered,
    Subscription,
}

impl Default for ApiProviderBillingMode {
    fn default() -> Self {
        Self::Metered
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMode {
    #[default]
    ApiKey,
    Account,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    Openai,
    Google,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiProviderRequest {
    #[serde(alias = "provider_name")]
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub billing_mode: Option<ApiProviderBillingMode>,
    #[serde(default)]
    pub auth_mode: Option<ProviderAuthMode>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiProviderRecord {
    #[serde(default)]
    pub id: String,
    #[serde(alias = "provider_name")]
    pub name: String,
    #[serde(default)]
    pub auth_mode: ProviderAuthMode,
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub billing_mode: ApiProviderBillingMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiProviderSummary {
    pub id: String,
    pub name: String,
    pub auth_mode: ProviderAuthMode,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub billing_mode: ApiProviderBillingMode,
    pub api_key_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouteSelection {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRouteRequest {
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelListItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelListItem {
    pub id: String,
}

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
pub struct OpenAIMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OpenAIContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OpenAIContent {
    String(String),
    Array(Vec<OpenAIContentBlock>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum OpenAIContentBlock {
    #[serde(rename = "text", alias = "input_text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OpenAIImageUrl },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAIImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
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

#[derive(Debug, Serialize)]
pub struct GeminiGenerateRequest {
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiContent>,
    pub contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct GeminiContent {
    pub role: String,
    pub parts: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct GenerationConfig {
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(rename = "topK", skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", alias = "account_type", alias = "kind")]
    pub account_type: AccountType,
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "account_id")]
    pub upstream_account_id: Option<String>,
}

impl AccountRecord {
    pub fn with_id(mut self, id: String) -> Self {
        self.id = id;
        self
    }

    pub fn provider(&self) -> &str {
        match self.account_type {
            AccountType::Openai => PROVIDER_OPENAI_PROXY,
            AccountType::Google => PROVIDER_GOOGLE_PROXY,
        }
    }

    pub fn access_token(&self) -> &str {
        self.access_token.as_str()
    }

    pub fn refresh_token(&self) -> &str {
        self.refresh_token.as_str()
    }

    pub fn refresh_token_mut(&mut self) -> &mut String {
        &mut self.refresh_token
    }

    pub fn access_token_mut(&mut self) -> &mut String {
        &mut self.access_token
    }

    pub fn set_expiry_timestamp(&mut self, expiry_timestamp: i64) {
        self.expiry_timestamp = expiry_timestamp;
    }

    pub fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }

    pub fn set_project_id(&mut self, project_id: String) {
        self.project_id = Some(project_id);
    }

    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    pub fn upstream_account_id(&self) -> Option<&str> {
        self.upstream_account_id.as_deref()
    }
}
