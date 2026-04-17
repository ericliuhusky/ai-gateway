use serde::{Deserialize, Serialize};

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
#[serde(deny_unknown_fields)]
pub struct CreateApiProviderRequest {
    #[serde(alias = "provider_name")]
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
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
    pub uses_chat_completions: bool,
    #[serde(default)]
    pub billing_mode: ApiProviderBillingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderExtensionRecord {
    pub provider_id: String,
    pub extension_type: String,
    pub user_id: String,
    pub access_token: String,
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
    pub uses_chat_completions: bool,
    pub billing_mode: ApiProviderBillingMode,
    pub api_key_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SelectedProvider {
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

#[derive(Debug, Clone, Serialize)]
pub struct CodexConfigStatus {
    pub target_path: String,
    pub auth_path: String,
    pub config_backup_exists: bool,
    pub auth_backup_exists: bool,
    pub restore_available: bool,
    pub target_exists: bool,
    pub auth_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListItem {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    ChatgptCodexUsageApi,
    NewApiExtension,
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

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamRateLimitStatusPayload {
    pub plan_type: String,
    #[serde(default)]
    pub rate_limit: Option<UpstreamRateLimitStatusDetails>,
    #[serde(default)]
    pub credits: Option<UpstreamCreditStatusDetails>,
    #[serde(default)]
    pub additional_rate_limits: Option<Vec<UpstreamAdditionalRateLimitDetails>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamRateLimitStatusDetails {
    #[allow(dead_code)]
    pub allowed: bool,
    #[allow(dead_code)]
    pub limit_reached: bool,
    #[serde(default)]
    pub primary_window: Option<UpstreamRateLimitWindowSnapshot>,
    #[serde(default)]
    pub secondary_window: Option<UpstreamRateLimitWindowSnapshot>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamRateLimitWindowSnapshot {
    pub used_percent: i32,
    pub limit_window_seconds: i32,
    #[allow(dead_code)]
    pub reset_after_seconds: i32,
    pub reset_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamCreditStatusDetails {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(default)]
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamAdditionalRateLimitDetails {
    pub limit_name: String,
    pub metered_feature: String,
    #[serde(default)]
    pub rate_limit: Option<UpstreamRateLimitStatusDetails>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewApiUserSelfEnvelope {
    pub data: NewApiUserSelf,
    pub success: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewApiUserSelf {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub quota: i64,
    #[serde(default)]
    pub used_quota: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewApiSubscriptionEnvelope {
    pub data: NewApiSubscriptionData,
    pub success: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewApiSubscriptionData {
    #[serde(default)]
    pub subscriptions: Vec<NewApiSubscriptionWrapper>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewApiSubscriptionWrapper {
    pub subscription: NewApiSubscription,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewApiSubscription {
    pub id: i64,
    pub user_id: i64,
    pub amount_total: i64,
    pub amount_used: i64,
    #[serde(default)]
    pub purchase_price_amount: f64,
    #[serde(default)]
    pub purchase_currency: String,
    pub start_time: i64,
    pub end_time: i64,
    pub status: String,
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

#[derive(Debug, Clone, Serialize)]
pub struct GatewayLogSummary {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    pub has_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_output: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayLogDetail {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_request_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress_request_body: Option<String>,
    pub ingress_request_body_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_request_body: Option<String>,
    pub egress_request_body_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress_response_status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingress_response_body: Option<String>,
    pub ingress_response_body_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_response_status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_response_body: Option<String>,
    pub egress_response_body_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub error_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_input_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayLogListResponse {
    pub logs: Vec<GatewayLogSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayLogDetailResponse {
    pub log: GatewayLogDetail,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayLogSettings {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayLogSettingsResponse {
    pub logging: GatewayLogSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateGatewayLogSettingsRequest {
    pub enabled: bool,
}
