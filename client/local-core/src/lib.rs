mod adapters;
mod api;
mod codex_config;
mod config;
mod crypto;
mod models;
mod openai_device_login;
mod openai_tokens;
mod routing;
mod shared_leases;
mod store;
mod support;
mod upstream;

use api::{AppState, build_router};
use config::Config;
use openai_device_login::OpenAiDeviceLoginService;
use openai_tokens::OpenAiTokenService;
use reqwest::Client;
use shared_leases::{SharedLeaseStore, local_shared_provider_id};
use std::sync::Arc;
use store::{
    AccountStore, IssueStore, ModelStore, ProviderStore, RouteStore, SettingsStore, TurnLogStore,
    UsageStore,
};
use upstream::UpstreamClient;

pub use codex_config::{
    CodexConfigurationResult, CodexInstancePaths, DefaultCodexStatus, default_codex_status,
    delete_codex_instance, prepare_codex_instance, start_default_codex, stop_default_codex,
};
pub use models::{ProviderCompatibilityProfile, ProviderUpstreamProtocol};

pub const LOCAL_GATEWAY_URL: &str = "http://127.0.0.1:4242/openai/v1";
pub const LOCAL_API_ROOT: &str = "http://127.0.0.1:4242";

pub struct SharedProviderLeaseInput {
    pub central_provider_id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub upstream_protocol: ProviderUpstreamProtocol,
    pub compatibility_profile: ProviderCompatibilityProfile,
    pub expires_at: i64,
}

pub struct ShareableProviderSource {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub compatibility_profile: ProviderCompatibilityProfile,
}

#[derive(Clone)]
pub struct LocalGatewayHandle {
    providers: ProviderStore,
    routes: RouteStore,
    models: ModelStore,
    settings: SettingsStore,
    shared_leases: SharedLeaseStore,
}

impl LocalGatewayHandle {
    pub async fn shareable_provider_source(
        &self,
        provider_id: &str,
    ) -> Result<ShareableProviderSource, String> {
        let provider = self
            .providers
            .find_by_id_for_owner(None, provider_id)
            .await
            .ok_or_else(|| format!("本地供应商不存在: {provider_id}"))?;
        if provider
            .id
            .starts_with(shared_leases::SHARED_PROVIDER_PREFIX)
        {
            return Err("不能再次共享从群组同步到本机的供应商".to_string());
        }
        if provider.auth_mode != models::ProviderAuthMode::ApiKey {
            return Err("群组共享目前只支持 API Key 供应商".to_string());
        }
        if provider.compatibility_profile == ProviderCompatibilityProfile::OpenAiCodex {
            return Err("账户专用供应商不能共享到群组".to_string());
        }
        Ok(ShareableProviderSource {
            name: provider.name,
            base_url: provider.base_url,
            api_key: provider.api_key,
            compatibility_profile: provider.compatibility_profile,
        })
    }

    pub async fn upsert_shared_provider(
        &self,
        lease: SharedProviderLeaseInput,
    ) -> Result<String, String> {
        let local_id = local_shared_provider_id(&lease.central_provider_id);
        self.providers
            .upsert_shared_lease(
                &local_id,
                &lease.name,
                &lease.base_url,
                &lease.api_key,
                lease.upstream_protocol,
                lease.compatibility_profile,
            )
            .await?;
        self.shared_leases.authorize(&local_id, lease.expires_at)?;
        Ok(local_id)
    }

    pub async fn revoke_shared_provider(&self, central_provider_id: &str) -> Result<(), String> {
        let local_id = local_shared_provider_id(central_provider_id);
        self.shared_leases.revoke(&local_id)?;
        self.routes.clear_instance_provider(&local_id)?;
        if self.routes.get().await.provider_id.as_deref() == Some(local_id.as_str()) {
            self.routes.set_provider(None).await?;
        }
        self.settings.clear_auto_routing_provider(&local_id)?;
        self.settings
            .clear_instance_auto_routing_provider(&local_id)?;
        self.models.delete(&local_id)?;
        self.providers.delete_shared_lease(&local_id).await
    }

    pub async fn shared_provider_ids(&self) -> Vec<String> {
        self.providers
            .list_shared_lease_ids()
            .await
            .into_iter()
            .filter_map(|id| {
                id.strip_prefix(shared_leases::SHARED_PROVIDER_PREFIX)
                    .map(str::to_string)
            })
            .collect()
    }
}

pub async fn start_local_gateway() -> Result<LocalGatewayHandle, String> {
    let config = Arc::new(Config::local()?);
    let accounts = AccountStore::new(config.clone())?;
    accounts.load().await?;
    let providers = ProviderStore::new(config.clone())?;
    providers.load().await?;
    let routes = RouteStore::new(config.clone())?;
    routes.load().await?;
    let models = ModelStore::new(config.clone())?;
    let settings = SettingsStore::new(config.clone())?;
    let shared_leases = SharedLeaseStore::default();
    let handle = LocalGatewayHandle {
        providers: providers.clone(),
        routes: routes.clone(),
        models: models.clone(),
        settings: settings.clone(),
        shared_leases: shared_leases.clone(),
    };
    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        openai_tokens: OpenAiTokenService::new(),
        openai_device_login: OpenAiDeviceLoginService::new(),
        accounts,
        providers,
        routes,
        models,
        settings,
        turn_logs: TurnLogStore::new(config.clone())?,
        issues: IssueStore::new(config.clone())?,
        usage: UsageStore::new(config)?,
        upstream: UpstreamClient::new(),
        shared_leases,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4242")
        .await
        .map_err(|error| format!("无法启动本地 Gateway (127.0.0.1:4242)：{error}"))?;
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, build_router(state)).await {
            eprintln!("本地 Gateway 已停止：{error}");
        }
    });
    Ok(handle)
}
