use crate::{
    config::Config,
    models::{CachedProviderModels, ModelListResponse},
    store::sqlite::SqliteStore,
};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct ModelStore {
    sqlite: SqliteStore,
}

impl ModelStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    pub fn load(&self, provider_id: &str) -> Result<Option<ModelListResponse>, String> {
        let Some(cached) = self.sqlite.load_cached_models(provider_id)? else {
            return Ok(None);
        };

        let response = serde_json::from_str(&cached.models_json)
            .map_err(|err| format!("decode cached provider models failed: {err}"))?;
        Ok(Some(response))
    }

    pub fn save(&self, provider_id: &str, response: &ModelListResponse) -> Result<(), String> {
        let models_json = serde_json::to_string(response)
            .map_err(|err| format!("encode cached provider models failed: {err}"))?;
        self.sqlite.upsert_cached_models(&CachedProviderModels {
            provider_id: provider_id.to_string(),
            models_json,
            updated_at: now_unix() as i64,
        })
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
