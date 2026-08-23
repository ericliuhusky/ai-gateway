mod adapters;
mod api;
mod config;
mod control;
mod control_plane;
mod control_store;
mod crypto;
mod models;
mod openai_device_login;
mod openai_tokens;
mod routing;
mod shared_leases;
mod store;
mod support;
mod upstream;

use api::{AppState, build_router, build_router_with_web};
use config::Config;
use control_plane::spawn_periodic_shared_sync;
use openai_device_login::OpenAiDeviceLoginService;
use openai_tokens::OpenAiTokenService;
use reqwest::Client;
use shared_leases::{SharedLeaseStore, local_shared_provider_id};
use std::{path::PathBuf, sync::Arc};
use store::{
    AccountStore, IssueStore, ModelStore, ProviderStore, RouteStore, SettingsStore, TurnLogStore,
    UsageStore,
};
use upstream::UpstreamClient;

pub use control::GatewayRuntime;
pub use control_plane::{
    ControlLoginInput, ControlLoginResult, ControlRequestInput, SharedSyncStatus,
    login_control_plane, publish_shared_connection, request_control_plane, sync_shared_providers,
};
pub use control_store::{LocalGatewayStatus, LocalStore};
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

pub(crate) fn local_gateway_handle(
    providers: ProviderStore,
    routes: RouteStore,
    models: ModelStore,
    settings: SettingsStore,
    shared_leases: SharedLeaseStore,
) -> LocalGatewayHandle {
    LocalGatewayHandle {
        providers,
        routes,
        models,
        settings,
        shared_leases,
    }
}

pub async fn start_local_gateway() -> Result<LocalGatewayHandle, String> {
    let (state, handle) = initialize_local_gateway().await?;
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

pub async fn serve_gateway(web_dir: Option<PathBuf>) -> Result<(), String> {
    let (state, _) = initialize_local_gateway().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4242")
        .await
        .map_err(|error| format!("无法启动 Gateway 服务 (127.0.0.1:4242)：{error}"))?;
    axum::serve(listener, build_router_with_web(state, web_dir))
        .await
        .map_err(|error| format!("Gateway 服务已停止：{error}"))
}

async fn initialize_local_gateway() -> Result<(AppState, LocalGatewayHandle), String> {
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
    let control_store = LocalStore::open()?;
    let gateway_runtime = GatewayRuntime::new(true);
    let handle = local_gateway_handle(
        providers.clone(),
        routes.clone(),
        models.clone(),
        settings.clone(),
        shared_leases.clone(),
    );
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
        control_store: control_store.clone(),
        gateway: handle.clone(),
        gateway_runtime,
    };
    spawn_periodic_shared_sync(control_store, handle.clone());
    Ok((state, handle))
}
