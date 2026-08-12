use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMode {
    #[default]
    ApiKey,
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
    pub id: String,
    pub name: String,
    pub auth_mode: ProviderAuthMode,
    pub base_url: String,
    pub api_key: String,
    pub upstream_protocol: ProviderUpstreamProtocol,
    pub compatibility_profile: ProviderCompatibilityProfile,
    #[serde(skip_serializing)]
    pub owner_user_id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiProviderSummary {
    pub id: String,
    pub name: String,
    pub auth_mode: ProviderAuthMode,
    pub base_url: String,
    pub upstream_protocol: ProviderUpstreamProtocol,
    pub compatibility_profile: ProviderCompatibilityProfile,
    pub shared: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayUserSummary {
    pub id: i64,
    pub name: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayGroupSummary {
    pub id: i64,
    pub name: String,
    pub owner_user_id: i64,
    pub owner_name: String,
    pub role: String,
    pub member_count: i64,
    pub provider_count: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayGroupMember {
    pub user_id: i64,
    pub name: String,
    pub avatar_url: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayGroupProvider {
    pub provider: ApiProviderSummary,
    pub shared_by_user_id: i64,
    pub shared_by_name: String,
    pub shared_at: i64,
    pub can_remove: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayGroupDetail {
    pub group: GatewayGroupSummary,
    pub members: Vec<GatewayGroupMember>,
    pub providers: Vec<GatewayGroupProvider>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateGatewayGroupRequest {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddGatewayGroupMemberRequest {
    pub user_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShareGatewayGroupProviderRequest {
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SharedProviderDescriptor {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub upstream_protocol: ProviderUpstreamProtocol,
    pub compatibility_profile: ProviderCompatibilityProfile,
    pub shared_by_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssueSharedProviderLeaseRequest {
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SharedProviderLease {
    pub provider_id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub upstream_protocol: ProviderUpstreamProtocol,
    pub compatibility_profile: ProviderCompatibilityProfile,
    pub expires_at: i64,
}
