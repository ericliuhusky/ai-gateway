use crate::{
    config::Config,
    models::{
        ApiProviderBillingMode, ApiProviderRecord, ApiProviderSummary, CreateApiProviderRequest,
        ProviderAuthMode,
    },
    store::sqlite::SqliteStore,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ProviderStore {
    sqlite: SqliteStore,
    providers: Arc<Mutex<Vec<ApiProviderRecord>>>,
}

impl ProviderStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let store = Self {
            sqlite: SqliteStore::new(config.clone())?,
            providers: Arc::new(Mutex::new(Vec::new())),
        };
        Ok(store)
    }

    pub async fn load(&self) -> Result<usize, String> {
        let loaded = self.sqlite.load_providers()?;
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
                auth_mode: provider.auth_mode.clone(),
                base_url: provider.base_url.clone(),
                account_id: provider.account_id.clone(),
                account_email: None,
                uses_chat_completions: provider.uses_chat_completions,
                billing_mode: provider.billing_mode.clone(),
                api_key_preview: mask_api_key(&provider.api_key),
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

        let base_url = request.base_url.unwrap_or_default().trim().to_string();
        let api_key = request.api_key.unwrap_or_default().trim().to_string();
        if api_key.is_empty() {
            return Err("api_key cannot be empty".to_string());
        }
        if base_url.is_empty() {
            return Err("base_url cannot be empty".to_string());
        }

        let mut providers = self.providers.lock().await;
        let provider =
            if let Some(existing) = providers.iter_mut().find(|provider| provider.name == name) {
                existing.auth_mode = ProviderAuthMode::ApiKey;
                existing.base_url = base_url;
                existing.api_key = api_key;
                existing.account_id = None;
                existing.uses_chat_completions = request.uses_chat_completions;
                existing.billing_mode = request
                    .billing_mode
                    .unwrap_or_else(|| existing.billing_mode.clone());
                existing.clone()
            } else {
                let provider = ApiProviderRecord {
                    id: Uuid::new_v4().to_string(),
                    name,
                    auth_mode: ProviderAuthMode::ApiKey,
                    base_url,
                    api_key,
                    account_id: None,
                    uses_chat_completions: request.uses_chat_completions,
                    billing_mode: request
                        .billing_mode
                        .unwrap_or(ApiProviderBillingMode::Metered),
                };
                providers.push(provider.clone());
                provider
            };

        self.persist_provider(&provider)?;
        Ok(provider)
    }

    pub async fn find_by_id(&self, id: &str) -> Option<ApiProviderRecord> {
        self.providers
            .lock()
            .await
            .iter()
            .find(|provider| provider.id == id)
            .cloned()
    }

    pub async fn bind_account_provider(
        &self,
        name: &str,
        account_id: &str,
    ) -> Result<ApiProviderRecord, String> {
        let mut providers = self.providers.lock().await;
        let provider = if let Some(existing) = providers
            .iter_mut()
            .find(|provider| provider.account_id.as_deref() == Some(account_id))
        {
            existing.name = name.to_string();
            existing.auth_mode = ProviderAuthMode::Account;
            existing.base_url.clear();
            existing.api_key.clear();
            existing.account_id = Some(account_id.to_string());
            existing.uses_chat_completions = false;
            existing.clone()
        } else {
            let provider = ApiProviderRecord {
                id: Uuid::new_v4().to_string(),
                name: name.to_string(),
                auth_mode: ProviderAuthMode::Account,
                base_url: String::new(),
                api_key: String::new(),
                account_id: Some(account_id.to_string()),
                uses_chat_completions: false,
                billing_mode: ApiProviderBillingMode::Metered,
            };
            providers.push(provider.clone());
            provider
        };

        self.persist_provider(&provider)?;
        Ok(provider)
    }
    fn persist_provider(&self, provider: &ApiProviderRecord) -> Result<(), String> {
        self.sqlite.upsert_provider(provider)
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

#[cfg(test)]
mod tests {
    use super::ProviderStore;
    use crate::{
        models::{PROVIDER_GOOGLE_PROXY, PROVIDER_OPENAI_PROXY, ProviderAuthMode},
        store::sqlite::SqliteStore,
    };
    use std::{
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn bind_account_provider_creates_one_provider_per_account() {
        let sqlite = test_sqlite_store("multi-account-providers");
        let store = ProviderStore {
            sqlite,
            providers: Arc::new(Mutex::new(Vec::new())),
        };

        let first = store
            .bind_account_provider(PROVIDER_OPENAI_PROXY, "account_1")
            .await
            .expect("bind first account");
        let second = store
            .bind_account_provider(PROVIDER_OPENAI_PROXY, "account_2")
            .await
            .expect("bind second account");

        assert_ne!(first.id, second.id);
        assert_eq!(first.name, PROVIDER_OPENAI_PROXY);
        assert_eq!(second.name, PROVIDER_OPENAI_PROXY);
        assert_eq!(first.account_id.as_deref(), Some("account_1"));
        assert_eq!(second.account_id.as_deref(), Some("account_2"));

        let providers = store.list().await;
        assert_eq!(providers.len(), 2);
        assert_eq!(
            providers
                .iter()
                .filter(|provider| provider.name == PROVIDER_OPENAI_PROXY)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn bind_account_provider_reuses_existing_provider_for_same_account() {
        let sqlite = test_sqlite_store("same-account-provider");
        let store = ProviderStore {
            sqlite,
            providers: Arc::new(Mutex::new(Vec::new())),
        };

        let first = store
            .bind_account_provider(PROVIDER_GOOGLE_PROXY, "account_1")
            .await
            .expect("bind first time");
        let second = store
            .bind_account_provider(PROVIDER_GOOGLE_PROXY, "account_1")
            .await
            .expect("bind second time");

        assert_eq!(first.id, second.id);
        assert_eq!(second.account_id.as_deref(), Some("account_1"));
        assert_eq!(second.auth_mode, ProviderAuthMode::Account);
        assert_eq!(store.list().await.len(), 1);
    }

    fn test_sqlite_store(prefix: &str) -> SqliteStore {
        let db_path = unique_test_db_path(prefix);
        SqliteStore::for_test(db_path).expect("create sqlite store")
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
