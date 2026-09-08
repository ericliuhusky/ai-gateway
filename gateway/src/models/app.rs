use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMode {
    #[default]
    ApiKey,
    Account,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProviderRequest {
    #[serde(alias = "provider_name")]
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    #[serde(default)]
    pub id: String,
    #[serde(alias = "provider_name", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub auth_mode: ProviderAuthMode,
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    #[serde(skip_serializing)]
    pub email: Option<String>,
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing)]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing)]
    pub expiry_timestamp: Option<i64>,
    #[serde(default, skip_serializing)]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing, alias = "account_id")]
    pub upstream_account_id: Option<String>,
    #[serde(skip_serializing)]
    pub owner_user_id: Option<i64>,
}

impl ProviderRecord {
    pub fn name(&self) -> &str {
        self.name.as_deref().or(self.email.as_deref()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub auth_mode: ProviderAuthMode,
    pub base_url: String,
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
    pub provider: ProviderSummary,
    pub quota: ProviderQuotaSummary,
}

impl ProviderRecord {
    pub fn new_openai_account(
        email: String,
        access_token: String,
        refresh_token: String,
        expiry_timestamp: i64,
        client_id: Option<String>,
        upstream_account_id: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: None,
            auth_mode: ProviderAuthMode::Account,
            base_url: String::new(),
            api_key: String::new(),
            email: Some(email),
            access_token: Some(access_token),
            refresh_token: Some(refresh_token),
            expiry_timestamp: Some(expiry_timestamp),
            client_id,
            upstream_account_id,
            owner_user_id: None,
        }
    }

    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref()
    }

    pub fn upstream_account_id(&self) -> Option<&str> {
        self.upstream_account_id.as_deref()
    }

    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    pub fn set_expiry_timestamp(&mut self, expiry_timestamp: i64) {
        self.expiry_timestamp = Some(expiry_timestamp);
    }
}
