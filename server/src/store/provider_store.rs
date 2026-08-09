use crate::{
    config::Config,
    models::{
        ApiProviderRecord, ApiProviderSummary, CreateApiProviderRequest, ProviderAuthMode,
        ProviderCompatibilityProfile, ProviderUpstreamProtocol,
    },
    store::sqlite::SqliteStore,
};
use reqwest::Url;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ProviderStore {
    sqlite: SqliteStore,
    providers: Arc<Mutex<Vec<ApiProviderRecord>>>,
}

impl ProviderStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
            providers: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub async fn load(&self) -> Result<(), String> {
        *self.providers.lock().await = self.sqlite.load_providers()?;
        Ok(())
    }

    pub async fn list_for_owner(&self, owner_user_id: i64) -> Vec<ApiProviderSummary> {
        self.providers
            .lock()
            .await
            .iter()
            .filter(|provider| provider.owner_user_id == owner_user_id)
            .map(|provider| provider_summary(provider, false))
            .collect()
    }

    pub async fn list_visible_for_user(&self, user_id: i64) -> Vec<ApiProviderSummary> {
        let shared_ids = self
            .sqlite
            .shared_provider_ids_for_user(user_id)
            .unwrap_or_default();
        self.providers
            .lock()
            .await
            .iter()
            .filter(|provider| {
                provider.owner_user_id == user_id || shared_ids.contains(&provider.id)
            })
            .map(|provider| provider_summary(provider, provider.owner_user_id != user_id))
            .collect()
    }

    pub async fn find_shared_by_id_for_user(
        &self,
        user_id: i64,
        id: &str,
    ) -> Option<ApiProviderRecord> {
        let shared_ids: HashSet<String> = self
            .sqlite
            .shared_provider_ids_for_user(user_id)
            .unwrap_or_default();
        if !shared_ids.contains(id) {
            return None;
        }
        self.providers
            .lock()
            .await
            .iter()
            .find(|provider| provider.id == id && provider.owner_user_id != user_id)
            .cloned()
    }

    pub async fn upsert_for_owner(
        &self,
        owner_user_id: i64,
        request: CreateApiProviderRequest,
    ) -> Result<ApiProviderRecord, String> {
        self.save_for_owner(owner_user_id, None, request).await
    }

    pub async fn update_for_owner(
        &self,
        owner_user_id: i64,
        id: &str,
        request: CreateApiProviderRequest,
    ) -> Result<ApiProviderRecord, String> {
        self.save_for_owner(owner_user_id, Some(id), request).await
    }

    async fn save_for_owner(
        &self,
        owner_user_id: i64,
        existing_id: Option<&str>,
        request: CreateApiProviderRequest,
    ) -> Result<ApiProviderRecord, String> {
        let name = request.name.trim().to_string();
        let base_url = request.base_url.unwrap_or_default().trim().to_string();
        let api_key = request.api_key.unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return Err("供应商名称不能为空".to_string());
        }
        if base_url.is_empty() {
            return Err("base_url 不能为空".to_string());
        }
        if api_key.is_empty() {
            return Err("api_key 不能为空".to_string());
        }
        let compatibility_profile = request
            .compatibility_profile
            .unwrap_or_else(|| compatibility_profile_for_base_url(&base_url));

        let mut providers = self.providers.lock().await;
        let existing_index = existing_id
            .map(|id| {
                providers
                    .iter()
                    .position(|provider| {
                        provider.id == id && provider.owner_user_id == owner_user_id
                    })
                    .ok_or_else(|| format!("unknown provider_id: {id}"))
            })
            .transpose()?;
        if providers.iter().enumerate().any(|(index, provider)| {
            Some(index) != existing_index
                && provider.owner_user_id == owner_user_id
                && provider.name == name
        }) {
            return Err(format!("供应商名称已存在: {name}"));
        }
        let provider = ApiProviderRecord {
            id: existing_id
                .map(ToString::to_string)
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            name,
            auth_mode: ProviderAuthMode::ApiKey,
            base_url,
            api_key,
            upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
            compatibility_profile,
            owner_user_id,
        };
        self.sqlite.upsert_provider(&provider)?;
        if let Some(index) = existing_index {
            providers[index] = provider.clone();
        } else {
            providers.push(provider.clone());
        }
        Ok(provider)
    }

    pub async fn delete_for_owner(
        &self,
        owner_user_id: i64,
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
}

fn provider_summary(provider: &ApiProviderRecord, shared: bool) -> ApiProviderSummary {
    ApiProviderSummary {
        id: provider.id.clone(),
        name: provider.name.clone(),
        auth_mode: provider.auth_mode.clone(),
        base_url: provider.base_url.clone(),
        upstream_protocol: provider.upstream_protocol.clone(),
        compatibility_profile: provider.compatibility_profile.clone(),
        shared,
    }
}

fn compatibility_profile_for_base_url(base_url: &str) -> ProviderCompatibilityProfile {
    let official_openai = Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"));
    if official_openai {
        ProviderCompatibilityProfile::OfficialOpenAi
    } else {
        ProviderCompatibilityProfile::GenericOpenAi
    }
}
