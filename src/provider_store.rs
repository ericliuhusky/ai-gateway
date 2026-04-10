use crate::{
    config::Config,
    models::{ApiProviderRecord, ApiProviderSummary, CreateApiProviderRequest},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ProviderStore {
    config: Arc<Config>,
    providers: Arc<Mutex<Vec<ApiProviderRecord>>>,
}

impl ProviderStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let store = Self {
            config,
            providers: Arc::new(Mutex::new(Vec::new())),
        };
        store.ensure_dirs()?;
        Ok(store)
    }

    pub async fn load(&self) -> Result<usize, String> {
        let providers_dir = self.providers_dir();
        let mut loaded = Vec::new();

        for entry in fs::read_dir(providers_dir).map_err(|err| format!("read_dir failed: {err}"))? {
            let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let content =
                fs::read_to_string(&path).map_err(|err| format!("read provider failed: {err}"))?;
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| format!("invalid provider filename: {}", path.display()))?
                .to_string();
            let provider = serde_json::from_str::<ApiProviderRecord>(&content)
                .map_err(|err| format!("parse provider failed: {err}"))?;
            loaded.push(provider.with_id(id));
        }

        *self.providers.lock().await = loaded;
        Ok(self.providers.lock().await.len())
    }

    pub async fn list(&self) -> Vec<ApiProviderSummary> {
        self.providers
            .lock()
            .await
            .iter()
            .map(|provider| ApiProviderSummary {
                id: provider.id.clone(),
                name: provider.name.clone(),
                base_url: provider.base_url.clone(),
                billing_mode: provider.billing_mode.clone(),
                disabled: provider.disabled,
                api_key_preview: mask_api_key(&provider.api_key),
                updated_at: provider.updated_at,
            })
            .collect()
    }

    pub async fn upsert(
        &self,
        request: CreateApiProviderRequest,
    ) -> Result<ApiProviderRecord, String> {
        let name = request.name.trim().to_string();
        if name.is_empty() {
            return Err("name cannot be empty".to_string());
        }

        let api_key = request.api_key.trim().to_string();
        if api_key.is_empty() {
            return Err("api_key cannot be empty".to_string());
        }

        let base_url = request.base_url.trim().to_string();
        if base_url.is_empty() {
            return Err("base_url cannot be empty".to_string());
        }

        let now = now_unix() as i64;
        let mut providers = self.providers.lock().await;
        let provider =
            if let Some(existing) = providers.iter_mut().find(|provider| provider.name == name) {
                existing.base_url = base_url;
                existing.api_key = api_key;
                existing.billing_mode = request.billing_mode;
                existing.disabled = false;
                existing.updated_at = now;
                existing.clone()
            } else {
                let provider = ApiProviderRecord {
                    id: Uuid::new_v4().to_string(),
                    name,
                    base_url,
                    api_key,
                    billing_mode: request.billing_mode,
                    created_at: now,
                    updated_at: now,
                    disabled: false,
                };
                providers.push(provider.clone());
                provider
            };

        self.persist_provider(&provider)?;
        Ok(provider)
    }

    pub async fn find_by_name(&self, name: &str) -> Option<ApiProviderRecord> {
        self.providers
            .lock()
            .await
            .iter()
            .find(|provider| provider.name == name)
            .cloned()
    }

    fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(self.providers_dir())
            .map_err(|err| format!("create providers dir failed: {err}"))
    }

    fn providers_dir(&self) -> PathBuf {
        self.config.data_dir().join("providers")
    }

    fn provider_path(&self, provider_id: &str) -> PathBuf {
        self.providers_dir().join(format!("{provider_id}.json"))
    }

    fn persist_provider(&self, provider: &ApiProviderRecord) -> Result<(), String> {
        let path = self.provider_path(&provider.id);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(provider)
            .map_err(|err| format!("serialize provider failed: {err}"))?;
        fs::write(&tmp, body).map_err(|err| format!("write temp provider failed: {err}"))?;
        rename_replace(&tmp, &path)
    }
}

impl ApiProviderRecord {
    fn with_id(mut self, id: String) -> Self {
        self.id = id;
        self
    }
}

fn mask_api_key(api_key: &str) -> String {
    if api_key.len() <= 8 {
        return "********".to_string();
    }

    let prefix = &api_key[..4];
    let suffix = &api_key[api_key.len().saturating_sub(4)..];
    format!("{prefix}...{suffix}")
}

fn rename_replace(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        fs::remove_file(dst).map_err(|err| format!("remove old file failed: {err}"))?;
    }
    fs::rename(src, dst).map_err(|err| format!("rename failed: {err}"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
