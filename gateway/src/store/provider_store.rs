use crate::{
    api::dto::{CreateProviderReq, ProviderSummaryResp},
    config::Config,
    domain::{Provider, ProviderAuthMode, ProviderCredentials},
    openai::OpenAiTokenService,
    store::{ProviderRecord, sqlite::SqliteStore},
    support::time::now_unix,
};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ProviderStore {
    sqlite: SqliteStore,
}

impl ProviderStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    fn load(&self) -> Result<Vec<Provider>, String> {
        self.sqlite
            .load_providers()?
            .into_iter()
            .map(Provider::try_from)
            .collect()
    }

    pub fn list(&self) -> Result<Vec<ProviderSummaryResp>, String> {
        Ok(self.load()?.iter().map(ProviderSummaryResp::from).collect())
    }

    pub async fn upsert(&self, request: CreateProviderReq) -> Result<Provider, String> {
        let name = request.name.trim().to_string();
        if name.is_empty() {
            return Err("供应商名称不能为空".to_string());
        }

        let base_url = request.base_url.trim().to_string();
        let api_key = request.api_key.trim().to_string();
        if api_key.is_empty() {
            return Err("api_key 不能为空".to_string());
        }
        if base_url.is_empty() {
            return Err("base_url 不能为空".to_string());
        }
        let providers = self.load()?;
        if providers.iter().any(|provider| provider.name() == name) {
            return Err(format!("供应商名称已存在: {name}"));
        }

        let provider = Provider {
            id: Uuid::new_v4().to_string(),
            name,
            credentials: ProviderCredentials::ApiKey { base_url, api_key },
        };
        self.persist_provider(&provider)?;
        Ok(provider)
    }

    pub async fn import_openai_provider(&self, provider: Provider) -> Result<Provider, String> {
        self.import_openai_provider_with_replacement(provider, false)
            .await
    }

    pub async fn import_openai_provider_with_replacement(
        &self,
        mut provider: Provider,
        replace: bool,
    ) -> Result<Provider, String> {
        if provider.auth_mode() != ProviderAuthMode::Account {
            return Err("导入的 OpenAI 凭据必须是账户认证供应商".to_string());
        }
        let email = provider
            .email()
            .filter(|email| !email.trim().is_empty())
            .ok_or_else(|| "导入的 OpenAI 凭据缺少邮箱".to_string())?;
        let providers = self.load()?;
        if let Some(existing) = providers.iter().find(|existing| {
            existing.auth_mode() == ProviderAuthMode::Account && existing.email() == Some(email)
        }) {
            if !replace {
                return Err(format!("OpenAI 账号已经存在: {email}"));
            }

            // Keep the existing provider ID so routes and cached UI state continue to point to
            // the same account after its credentials are replaced.
            provider.id = existing.id().to_string();
            self.persist_provider(&provider)?;
            return Ok(provider);
        }

        self.persist_provider(&provider)?;
        Ok(provider)
    }

    pub fn account_exists(&self, email: &str) -> Result<bool, String> {
        Ok(self.load()?.iter().any(|provider| {
            provider.auth_mode() == ProviderAuthMode::Account && provider.email() == Some(email)
        }))
    }

    pub async fn acquire_by_id(
        &self,
        token_service: &OpenAiTokenService,
        provider_id: &str,
    ) -> Result<Provider, String> {
        let provider = self
            .find_by_id(provider_id)?
            .ok_or_else(|| format!("供应商不存在: {provider_id}"))?;
        self.prepare_provider_for_use(provider, token_service).await
    }

    pub async fn refresh(
        &self,
        token_service: &OpenAiTokenService,
        provider_id: &str,
    ) -> Result<Provider, String> {
        let provider = self
            .find_by_id(provider_id)?
            .ok_or_else(|| format!("供应商不存在: {provider_id}"))?;
        let provider = self.refresh_provider(provider, token_service).await?;
        self.persist_provider(&provider)?;
        Ok(provider)
    }

    pub fn find_by_id(&self, id: &str) -> Result<Option<Provider>, String> {
        Ok(self
            .load()?
            .iter()
            .find(|provider| provider.id() == id)
            .cloned())
    }

    pub fn delete(&self, id: &str) -> Result<Provider, String> {
        let provider = self
            .find_by_id(id)?
            .ok_or_else(|| format!("unknown provider_id: {id}"))?;
        self.sqlite.delete_provider(id)?;
        Ok(provider)
    }

    async fn prepare_provider_for_use(
        &self,
        mut provider: Provider,
        token_service: &OpenAiTokenService,
    ) -> Result<Provider, String> {
        if provider.auth_mode() != ProviderAuthMode::Account {
            return Err(format!("供应商不是账户认证模式: {}", provider.name()));
        }
        let expiry_timestamp = provider
            .expiry_timestamp()
            .ok_or_else(|| format!("账户认证供应商 `{}` 缺少过期时间", provider.name()))?;
        if token_service.refresh_needed(expiry_timestamp) {
            provider = self.refresh_provider(provider, token_service).await?;
        }

        self.persist_provider(&provider)?;
        Ok(provider)
    }

    async fn refresh_provider(
        &self,
        mut provider: Provider,
        token_service: &OpenAiTokenService,
    ) -> Result<Provider, String> {
        let client_id = provider
            .client_id()
            .ok_or_else(|| "openai provider missing oauth client id".to_string())?;
        let refresh_token = provider
            .refresh_token()
            .ok_or_else(|| "openai provider missing refresh token".to_string())?;
        let refreshed = token_service
            .refresh_access_token(client_id, refresh_token)
            .await
            .map_err(|err| {
                format!(
                    "refresh failed for {}: {err}",
                    provider.email().unwrap_or("unknown")
                )
            })?;

        provider.set_access_token(refreshed.access_token);
        provider.set_expiry_timestamp(now_unix() as i64 + refreshed.expires_in);
        if let Some(refresh_token) = refreshed.refresh_token {
            provider.set_refresh_token(refresh_token);
        }
        Ok(provider)
    }

    fn persist_provider(&self, provider: &Provider) -> Result<(), String> {
        self.sqlite.upsert_provider(&ProviderRecord::from(provider))
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderStore;
    use crate::{
        api::dto::CreateProviderReq,
        domain::{Provider, ProviderAuthMode},
        store::sqlite::SqliteStore,
    };
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[tokio::test]
    async fn imports_one_provider_per_openai_email() {
        let sqlite = test_sqlite_store("openai-providers");
        let store = ProviderStore { sqlite };
        let provider = || {
            Provider::new_openai_account(
                "user@example.com".to_string(),
                "access".to_string(),
                "refresh".to_string(),
                1_700_000_000,
                Some("client".to_string()),
            )
        };

        let first = store
            .import_openai_provider(provider())
            .await
            .expect("import first provider");
        assert_eq!(first.auth_mode(), ProviderAuthMode::Account);
        assert_eq!(first.email(), Some("user@example.com"));
        assert!(store.import_openai_provider(provider()).await.is_err());

        let replacement = Provider::new_openai_account(
            "user@example.com".to_string(),
            "new-access".to_string(),
            "new-refresh".to_string(),
            1_800_000_000,
            Some("client".to_string()),
        );
        let updated = store
            .import_openai_provider_with_replacement(replacement, true)
            .await
            .expect("replace existing provider");
        assert_eq!(updated.id(), first.id());
        assert_eq!(updated.account_access_token(), "new-access");
    }

    #[tokio::test]
    async fn creates_api_key_provider_without_profile() {
        let sqlite = test_sqlite_store("create-provider");
        let store = ProviderStore { sqlite };

        let provider = store
            .upsert(CreateProviderReq {
                name: "official".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                api_key: "sk-test".to_string(),
            })
            .await
            .expect("create provider");
        assert_eq!(provider.base_url(), Some("https://api.openai.com/v1"));
        assert!(provider.email().is_none());
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
