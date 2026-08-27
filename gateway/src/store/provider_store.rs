use crate::{
    config::Config,
    models::{
        ApiProviderRecord, ApiProviderSummary, CreateApiProviderRequest,
        OPENAI_ACCOUNT_PROVIDER_NAME, PROVIDER_OPENAI_PROXY, ProviderAuthMode,
        ProviderCompatibilityProfile, ProviderUpstreamProtocol,
    },
    store::sqlite::SqliteStore,
};
use reqwest::Url;
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

    pub async fn load(&self) -> Result<(), String> {
        let mut loaded = self.sqlite.load_providers()?;
        let removed_shared_ids = loaded
            .iter()
            .filter(|provider| provider.id.starts_with("shared_"))
            .map(|provider| provider.id.clone())
            .collect::<Vec<_>>();
        for provider_id in &removed_shared_ids {
            self.sqlite.delete_provider(provider_id)?;
        }
        loaded.retain(|provider| !provider.id.starts_with("shared_"));
        for provider in &mut loaded {
            if provider.auth_mode == ProviderAuthMode::Account
                && provider.name == PROVIDER_OPENAI_PROXY
            {
                provider.name = OPENAI_ACCOUNT_PROVIDER_NAME.to_string();
                self.persist_provider(provider)?;
            }
        }
        *self.providers.lock().await = loaded;
        Ok(())
    }

    pub async fn list_for_owner(&self, owner_user_id: Option<i64>) -> Vec<ApiProviderSummary> {
        self.providers
            .lock()
            .await
            .iter()
            .filter(|provider| provider.owner_user_id == owner_user_id)
            .map(|provider| ApiProviderSummary {
                id: provider.id.clone(),
                usage_id: provider.id.clone(),
                name: provider.name.clone(),
                auth_mode: provider.auth_mode.clone(),
                base_url: provider.base_url.clone(),
                account_id: provider.account_id.clone(),
                account_email: None,
                upstream_protocol: provider.upstream_protocol.clone(),
                compatibility_profile: provider.compatibility_profile.clone(),
            })
            .collect()
    }

    pub async fn upsert_for_owner(
        &self,
        owner_user_id: Option<i64>,
        request: CreateApiProviderRequest,
    ) -> Result<ApiProviderRecord, String> {
        let name = request.name.trim().to_string();
        if name.is_empty() {
            return Err("供应商名称不能为空".to_string());
        }

        let base_url = request.base_url.unwrap_or_default().trim().to_string();
        let api_key = request.api_key.unwrap_or_default().trim().to_string();
        if api_key.is_empty() {
            return Err("api_key 不能为空".to_string());
        }
        if base_url.is_empty() {
            return Err("base_url 不能为空".to_string());
        }
        let compatibility_profile = request
            .compatibility_profile
            .unwrap_or_else(|| compatibility_profile_for_base_url(&base_url));
        if compatibility_profile == ProviderCompatibilityProfile::OpenAiCodex {
            return Err("兼容性配置 `openai_codex` 仅用于导入的账户供应商".to_string());
        }

        let mut providers = self.providers.lock().await;
        if providers
            .iter()
            .any(|provider| provider.owner_user_id == owner_user_id && provider.name == name)
        {
            return Err(format!("供应商名称已存在: {name}"));
        }

        let provider = ApiProviderRecord {
            id: Uuid::new_v4().to_string(),
            name,
            auth_mode: ProviderAuthMode::ApiKey,
            base_url,
            api_key,
            account_id: None,
            upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
            compatibility_profile,
            owner_user_id,
        };
        self.persist_provider(&provider)?;
        providers.push(provider.clone());
        Ok(provider)
    }

    pub async fn find_by_id_for_owner(
        &self,
        owner_user_id: Option<i64>,
        id: &str,
    ) -> Option<ApiProviderRecord> {
        self.providers
            .lock()
            .await
            .iter()
            .find(|provider| provider.id == id && provider.owner_user_id == owner_user_id)
            .cloned()
    }

    pub async fn has_account_provider_for_owner(
        &self,
        owner_user_id: Option<i64>,
        account_id: &str,
    ) -> bool {
        self.providers.lock().await.iter().any(|provider| {
            provider.owner_user_id == owner_user_id
                && provider.account_id.as_deref() == Some(account_id)
        })
    }

    pub async fn add_account_provider_for_owner(
        &self,
        owner_user_id: Option<i64>,
        name: &str,
        account_id: &str,
    ) -> Result<ApiProviderRecord, String> {
        let mut providers = self.providers.lock().await;
        if providers.iter().any(|provider| {
            provider.owner_user_id == owner_user_id
                && provider.account_id.as_deref() == Some(account_id)
        }) {
            return Err(format!("账户已经绑定供应商: {account_id}"));
        }
        let provider = ApiProviderRecord {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            auth_mode: ProviderAuthMode::Account,
            base_url: String::new(),
            api_key: String::new(),
            account_id: Some(account_id.to_string()),
            upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
            compatibility_profile: ProviderCompatibilityProfile::OpenAiCodex,
            owner_user_id,
        };
        self.persist_provider(&provider)?;
        providers.push(provider.clone());
        Ok(provider)
    }

    pub async fn delete_for_owner(
        &self,
        owner_user_id: Option<i64>,
        id: &str,
    ) -> Result<ApiProviderRecord, String> {
        let mut providers = self.providers.lock().await;
        let index = providers
            .iter()
            .position(|provider| provider.id == id && provider.owner_user_id == owner_user_id)
            .ok_or_else(|| format!("unknown provider_id: {id}"))?;
        let provider = providers.remove(index);
        self.sqlite.delete_provider(id)?;
        Ok(provider)
    }

    fn persist_provider(&self, provider: &ApiProviderRecord) -> Result<(), String> {
        self.sqlite.upsert_provider(provider)
    }
}

fn compatibility_profile_for_base_url(base_url: &str) -> ProviderCompatibilityProfile {
    let is_official_openai = Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"));
    if is_official_openai {
        ProviderCompatibilityProfile::OfficialOpenAi
    } else {
        ProviderCompatibilityProfile::GenericOpenAi
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderStore;
    use crate::{
        models::{
            AccountRecord, AccountType, CreateApiProviderRequest, OPENAI_ACCOUNT_PROVIDER_NAME,
            ProviderCompatibilityProfile, ProviderUpstreamProtocol,
        },
        store::sqlite::SqliteStore,
    };
    use std::{
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn add_account_provider_creates_one_provider_per_account() {
        let sqlite = test_sqlite_store("multi-account-providers");
        for id in ["account_1", "account_2"] {
            sqlite
                .upsert_account(&AccountRecord {
                    id: id.to_string(),
                    account_type: AccountType::Openai,
                    email: format!("{id}@example.com"),
                    access_token: "access".to_string(),
                    refresh_token: "refresh".to_string(),
                    expiry_timestamp: 0,
                    client_id: None,
                    upstream_account_id: None,
                    owner_user_id: None,
                })
                .expect("save account");
        }
        let store = ProviderStore {
            sqlite,
            providers: Arc::new(Mutex::new(Vec::new())),
        };

        let first = store
            .add_account_provider_for_owner(None, OPENAI_ACCOUNT_PROVIDER_NAME, "account_1")
            .await
            .expect("bind first account");
        let second = store
            .add_account_provider_for_owner(None, OPENAI_ACCOUNT_PROVIDER_NAME, "account_2")
            .await
            .expect("bind second account");

        assert_ne!(first.id, second.id);
        assert_eq!(first.name, OPENAI_ACCOUNT_PROVIDER_NAME);
        assert_eq!(second.name, OPENAI_ACCOUNT_PROVIDER_NAME);
        assert_eq!(first.account_id.as_deref(), Some("account_1"));
        assert_eq!(second.account_id.as_deref(), Some("account_2"));
        assert_eq!(
            first.compatibility_profile,
            ProviderCompatibilityProfile::OpenAiCodex
        );
        assert_eq!(
            first.upstream_protocol,
            ProviderUpstreamProtocol::OpenAiResponses
        );

        let providers = store.list_for_owner(None).await;
        assert_eq!(providers.len(), 2);
        assert_eq!(
            providers
                .iter()
                .filter(|provider| provider.name == OPENAI_ACCOUNT_PROVIDER_NAME)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn creates_api_key_provider_with_responses_protocol() {
        let sqlite = test_sqlite_store("legacy-create-provider");
        let store = ProviderStore {
            sqlite,
            providers: Arc::new(Mutex::new(Vec::new())),
        };

        let provider = store
            .upsert_for_owner(
                None,
                CreateApiProviderRequest {
                    name: "official".to_string(),
                    base_url: Some("https://api.openai.com/v1".to_string()),
                    api_key: Some("sk-test".to_string()),
                    compatibility_profile: None,
                },
            )
            .await
            .expect("create legacy provider");

        assert_eq!(
            provider.upstream_protocol,
            ProviderUpstreamProtocol::OpenAiResponses
        );
        assert_eq!(
            provider.compatibility_profile,
            ProviderCompatibilityProfile::OfficialOpenAi
        );
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
