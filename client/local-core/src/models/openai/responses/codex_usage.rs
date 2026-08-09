use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CodexUsageResponse {
    pub plan_type: String,
    #[serde(default)]
    pub rate_limit: Option<CodexUsageRateLimit>,
    #[serde(default)]
    pub credits: Option<CodexUsageCredits>,
    #[serde(default)]
    pub additional_rate_limits: Option<Vec<CodexUsageAdditionalRateLimit>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodexUsageRateLimit {
    #[allow(dead_code)]
    pub allowed: bool,
    #[allow(dead_code)]
    pub limit_reached: bool,
    #[serde(default)]
    pub primary_window: Option<CodexUsageRateLimitWindow>,
    #[serde(default)]
    pub secondary_window: Option<CodexUsageRateLimitWindow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodexUsageRateLimitWindow {
    pub used_percent: i32,
    pub limit_window_seconds: i32,
    #[allow(dead_code)]
    pub reset_after_seconds: i32,
    pub reset_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodexUsageCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(default)]
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodexUsageAdditionalRateLimit {
    pub limit_name: String,
    pub metered_feature: String,
    #[serde(default)]
    pub rate_limit: Option<CodexUsageRateLimit>,
}
