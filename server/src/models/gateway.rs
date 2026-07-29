#[derive(Debug, Clone)]
pub struct CachedProviderModels {
    pub provider_id: String,
    pub models_json: String,
    pub updated_at: i64,
}
