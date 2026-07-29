use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ProviderUpstreamProtocol {
    #[default]
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
    #[serde(rename = "openai_chat_completions")]
    OpenAiChatCompletions,
}

impl ProviderUpstreamProtocol {
    pub fn uses_chat_completions(&self) -> bool {
        matches!(self, Self::OpenAiChatCompletions)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ProviderCompatibilityProfile {
    #[serde(rename = "official_openai")]
    OfficialOpenAi,
    #[default]
    #[serde(rename = "generic_openai")]
    GenericOpenAi,
    #[serde(rename = "openai_codex")]
    OpenAiCodex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    Openai,
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
    #[serde(default)]
    pub upstream_protocol: Option<ProviderUpstreamProtocol>,
    #[serde(default)]
    pub compatibility_profile: Option<ProviderCompatibilityProfile>,
    /// Legacy input kept for API compatibility. New clients should send
    /// `upstream_protocol`.
    #[serde(default)]
    pub uses_chat_completions: bool,
    #[serde(default)]
    pub billing_mode: Option<ApiProviderBillingMode>,
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
    pub upstream_protocol: ProviderUpstreamProtocol,
    #[serde(default)]
    pub compatibility_profile: ProviderCompatibilityProfile,
    #[serde(default)]
    pub billing_mode: ApiProviderBillingMode,
}

impl ApiProviderRecord {
    pub fn uses_chat_completions(&self) -> bool {
        self.upstream_protocol.uses_chat_completions()
    }
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
    pub upstream_protocol: ProviderUpstreamProtocol,
    pub compatibility_profile: ProviderCompatibilityProfile,
    /// Legacy output kept while older Web clients are still in use.
    pub uses_chat_completions: bool,
    pub billing_mode: ApiProviderBillingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SelectedRoute {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub selected_model: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none", alias = "account_id")]
    pub upstream_account_id: Option<String>,
}

impl AccountRecord {
    pub fn provider(&self) -> &str {
        match self.account_type {
            AccountType::Openai => PROVIDER_OPENAI_PROXY,
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

    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    pub fn upstream_account_id(&self) -> Option<&str> {
        self.upstream_account_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderCompatibilityProfile, ProviderUpstreamProtocol};

    #[test]
    fn provider_protocol_and_profile_use_stable_api_names() {
        assert_eq!(
            serde_json::to_value(ProviderUpstreamProtocol::OpenAiResponses).unwrap(),
            "openai_responses"
        );
        assert_eq!(
            serde_json::to_value(ProviderUpstreamProtocol::OpenAiChatCompletions).unwrap(),
            "openai_chat_completions"
        );
        assert_eq!(
            serde_json::to_value(ProviderCompatibilityProfile::OfficialOpenAi).unwrap(),
            "official_openai"
        );
        assert_eq!(
            serde_json::to_value(ProviderCompatibilityProfile::GenericOpenAi).unwrap(),
            "generic_openai"
        );
        assert_eq!(
            serde_json::to_value(ProviderCompatibilityProfile::OpenAiCodex).unwrap(),
            "openai_codex"
        );
    }
}
