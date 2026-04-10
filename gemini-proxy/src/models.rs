use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct ResponseOutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
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
pub struct GoogleTokenData {
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_timestamp: i64,
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAITokenData {
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_timestamp: i64,
    pub client_id: String,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AccountToken {
    Google(GoogleTokenData),
    OpenAI(OpenAITokenData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider")]
enum AccountTokenWire {
    #[serde(rename = "google")]
    Google {
        access_token: String,
        refresh_token: String,
        expiry_timestamp: i64,
        project_id: String,
    },
    #[serde(rename = "openai")]
    OpenAI {
        access_token: String,
        refresh_token: String,
        expiry_timestamp: i64,
        client_id: String,
        #[serde(default)]
        account_id: Option<String>,
    },
}

impl AccountToken {
    pub fn google(
        access_token: String,
        refresh_token: String,
        expiry_timestamp: i64,
        project_id: String,
    ) -> Self {
        Self::Google(GoogleTokenData {
            access_token,
            refresh_token,
            expiry_timestamp,
            project_id,
        })
    }

    pub fn openai(
        access_token: String,
        refresh_token: String,
        expiry_timestamp: i64,
        client_id: String,
        account_id: Option<String>,
    ) -> Self {
        Self::OpenAI(OpenAITokenData {
            access_token,
            refresh_token,
            expiry_timestamp,
            client_id,
            account_id,
        })
    }

    pub fn provider(&self) -> &'static str {
        match self {
            Self::Google(_) => "google",
            Self::OpenAI(_) => "openai",
        }
    }

    pub fn access_token(&self) -> &str {
        match self {
            Self::Google(token) => &token.access_token,
            Self::OpenAI(token) => &token.access_token,
        }
    }

    pub fn refresh_token(&self) -> &str {
        match self {
            Self::Google(token) => &token.refresh_token,
            Self::OpenAI(token) => &token.refresh_token,
        }
    }

    pub fn refresh_token_mut(&mut self) -> &mut String {
        match self {
            Self::Google(token) => &mut token.refresh_token,
            Self::OpenAI(token) => &mut token.refresh_token,
        }
    }

    pub fn access_token_mut(&mut self) -> &mut String {
        match self {
            Self::Google(token) => &mut token.access_token,
            Self::OpenAI(token) => &mut token.access_token,
        }
    }

    pub fn expiry_timestamp(&self) -> i64 {
        match self {
            Self::Google(token) => token.expiry_timestamp,
            Self::OpenAI(token) => token.expiry_timestamp,
        }
    }

    pub fn set_expiry_timestamp(&mut self, expiry_timestamp: i64) {
        match self {
            Self::Google(token) => token.expiry_timestamp = expiry_timestamp,
            Self::OpenAI(token) => token.expiry_timestamp = expiry_timestamp,
        }
    }

    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Google(token) => Some(&token.project_id),
            Self::OpenAI(_) => None,
        }
    }

    pub fn set_project_id(&mut self, project_id: String) {
        if let Self::Google(token) = self {
            token.project_id = project_id;
        }
    }

    pub fn client_id(&self) -> Option<&str> {
        match self {
            Self::OpenAI(token) => Some(&token.client_id),
            Self::Google(_) => None,
        }
    }

    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::OpenAI(token) => token.account_id.as_deref(),
            Self::Google(_) => None,
        }
    }
}

impl Serialize for AccountToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::Google(token) => AccountTokenWire::Google {
                access_token: token.access_token.clone(),
                refresh_token: token.refresh_token.clone(),
                expiry_timestamp: token.expiry_timestamp,
                project_id: token.project_id.clone(),
            },
            Self::OpenAI(token) => AccountTokenWire::OpenAI {
                access_token: token.access_token.clone(),
                refresh_token: token.refresh_token.clone(),
                expiry_timestamp: token.expiry_timestamp,
                client_id: token.client_id.clone(),
                account_id: token.account_id.clone(),
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AccountToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AccountTokenWire::deserialize(deserializer)?;
        Ok(match wire {
            AccountTokenWire::Google {
                access_token,
                refresh_token,
                expiry_timestamp,
                project_id,
            } => Self::google(access_token, refresh_token, expiry_timestamp, project_id),
            AccountTokenWire::OpenAI {
                access_token,
                refresh_token,
                expiry_timestamp,
                client_id,
                account_id,
            } => Self::openai(
                access_token,
                refresh_token,
                expiry_timestamp,
                client_id,
                account_id,
            ),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    #[serde(skip_serializing, default)]
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub token: AccountToken,
    pub created_at: i64,
    pub last_used: i64,
    #[serde(default)]
    pub disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl AccountRecord {
    pub fn with_id(mut self, id: String) -> Self {
        self.id = id;
        self
    }

    pub fn provider(&self) -> &'static str {
        self.token.provider()
    }

    pub fn has_project_id(&self) -> bool {
        self.token
            .project_id()
            .is_some_and(|project_id| !project_id.is_empty())
    }
}

#[derive(Debug, Serialize)]
pub struct AccountSummary {
    pub id: String,
    pub email: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub has_project_id: bool,
    pub disabled: bool,
    pub last_used: i64,
}
