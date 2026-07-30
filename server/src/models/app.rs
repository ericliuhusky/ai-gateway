use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROVIDER_OPENAI_PROXY: &str = "openai-proxy";

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
    pub compatibility_profile: Option<ProviderCompatibilityProfile>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstanceRoutingConfig {
    pub instance_id: String,
    #[serde(flatten)]
    pub route: SelectedRoute,
    pub automatic_routing: AutoRoutingSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateInstanceRoutingConfigRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub selected_model: Option<String>,
    #[serde(default)]
    pub selected_reasoning_effort: Option<String>,
    #[serde(default)]
    pub automatic_routing: Option<AutoRoutingSettings>,
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
pub struct CodexClientVersionSetting {
    pub default_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_version: Option<String>,
    pub effective_version: String,
    pub is_overridden: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCodexClientVersionRequest {
    pub version: String,
}

fn default_low_confidence_threshold() -> f64 {
    0.7
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingModelTarget {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoRoutingSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier: Option<RoutingModelTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light: Option<RoutingModelTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standard: Option<RoutingModelTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pro: Option<RoutingModelTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<RoutingModelTarget>,
    #[serde(default = "default_low_confidence_threshold")]
    pub low_confidence_threshold: f64,
}

impl Default for AutoRoutingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            classifier: None,
            light: None,
            standard: None,
            pro: None,
            max: None,
            low_confidence_threshold: default_low_confidence_threshold(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAutoRoutingSettingsRequest {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub classifier: Option<RoutingModelTarget>,
    #[serde(default)]
    pub light: Option<RoutingModelTarget>,
    #[serde(default)]
    pub standard: Option<RoutingModelTarget>,
    #[serde(default)]
    pub pro: Option<RoutingModelTarget>,
    #[serde(default)]
    pub max: Option<RoutingModelTarget>,
    #[serde(default = "default_low_confidence_threshold")]
    pub low_confidence_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRouteLog {
    /// Opaque, hashed identifier. Raw client IDs and request contents are never stored.
    pub turn_id: String,
    pub provider_id: String,
    pub model: String,
    pub routing_mode: String,
    pub routing_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_input_preview: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    pub request_count: i64,
    pub tool_round_count: i64,
}

#[derive(Debug, Clone)]
pub struct TurnRouteLogUpdate {
    pub turn_id: String,
    pub provider_id: String,
    pub model: String,
    pub routing_mode: String,
    pub routing_reason: String,
    pub routing_detail: Option<String>,
    pub routing_tier: Option<String>,
    pub classifier_confidence: Option<f64>,
    pub classifier_output: Option<String>,
    pub reasoning_effort: Option<String>,
    pub user_input_preview: Option<String>,
    pub is_tool_round: bool,
    pub timestamp: i64,
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
    use super::{CreateApiProviderRequest, ProviderCompatibilityProfile, ProviderUpstreamProtocol};
    use serde_json::json;

    #[test]
    fn provider_protocol_and_profile_use_stable_api_names() {
        assert_eq!(
            serde_json::to_value(ProviderUpstreamProtocol::OpenAiResponses).unwrap(),
            "openai_responses"
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

    #[test]
    fn create_provider_rejects_upstream_protocol_selection() {
        let request = json!({
            "name": "legacy-chat-provider",
            "base_url": "https://example.com/v1",
            "api_key": "sk-test",
            "upstream_protocol": "openai_chat_completions"
        });

        assert!(serde_json::from_value::<CreateApiProviderRequest>(request).is_err());
    }
}
