use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const PROVIDER_OPENAI_PROXY: &str = "openai-proxy";
pub const OPENAI_ACCOUNT_PROVIDER_NAME: &str = "GPT账户";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMode {
    #[default]
    ApiKey,
    Account,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateApiProviderRequest {
    #[serde(alias = "provider_name")]
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
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
    #[serde(skip_serializing)]
    pub owner_user_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiProviderSummary {
    pub id: String,
    pub name: String,
    pub auth_mode: ProviderAuthMode,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SelectedRoute {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub selected_model: Option<String>,
    #[serde(default)]
    pub selected_reasoning_effort: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSelectedProviderRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSelectedModelRequest {
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSelectedReasoningEffortRequest {
    pub effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayIssue {
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub upstream_url: String,
    pub failure_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    pub error_message: String,
    pub upstream_response: String,
    pub upstream_response_truncated: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct GatewayIssueRecord {
    pub id: String,
    pub owner_user_id: Option<i64>,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub upstream_url: String,
    pub failure_kind: String,
    pub status_code: Option<u16>,
    pub error_message: String,
    pub upstream_response: String,
    pub upstream_response_truncated: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelListItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListItem {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    ChatgptCodexUsageApi,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSupportStatus {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderQuotaWindow {
    pub used_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderQuotaCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderQuotaSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<ProviderQuotaWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<ProviderQuotaWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<ProviderQuotaCredits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderQuotaSummary {
    pub source: QuotaSource,
    pub status: QuotaSupportStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<ProviderQuotaSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_snapshots: Vec<ProviderQuotaSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderQuotaResponse {
    pub provider: ApiProviderSummary,
    pub quota: ProviderQuotaSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatGPTAuthRecord {
    #[serde(default)]
    pub id: String,
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "upstream_account_id")]
    pub account_id: Option<String>,
}

impl ChatGPTAuthRecord {
    pub fn new(
        email: String,
        access_token: String,
        refresh_token: String,
        expiry_timestamp: i64,
        client_id: Option<String>,
        account_id: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            email,
            access_token,
            refresh_token,
            expiry_timestamp,
            client_id,
            account_id,
        }
    }

    pub fn provider(&self) -> &str {
        PROVIDER_OPENAI_PROXY
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

    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}
