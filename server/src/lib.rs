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

use api::{AppState, build_management_router, build_router};
use config::Config;
use control_plane::spawn_periodic_shared_sync;
use openai_device_login::OpenAiDeviceLoginService;
use openai_tokens::OpenAiTokenService;
use reqwest::Client;
use shared_leases::{SharedLeaseStore, local_shared_provider_id};
use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Method, Request,
        header::{ACCEPT, CONTENT_TYPE},
    },
};
use serde_json::Value;
use store::{
    AccountStore, IssueStore, ModelStore, ProviderStore, RouteStore, SettingsStore, TurnLogStore,
    UsageStore,
};
use tower::ServiceExt;
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

/// In-process management surface for the Tauri desktop client.
///
/// Management requests are dispatched to an Axum router without binding them
/// to a socket. The only TCP API remains the OpenAI-compatible gateway.
#[derive(Clone)]
pub struct DesktopGateway {
    management_router: Router,
}

impl DesktopGateway {
    async fn invoke(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let method = Method::from_bytes(method.trim().as_bytes())
            .map_err(|_| "不支持的本机请求方法".to_string())?;
        if !matches!(
            method,
            Method::GET | Method::POST | Method::PUT | Method::DELETE
        ) {
            return Err("本机请求只支持 GET、POST、PUT 和 DELETE".to_string());
        }
        if !path.starts_with('/') || path.starts_with("//") || path.contains("://") {
            return Err("本机请求路径无效".to_string());
        }

        let request_body = match body {
            Some(value) => serde_json::to_vec(&value)
                .map(Body::from)
                .map_err(|error| format!("序列化本机请求失败：{error}"))?,
            None => Body::empty(),
        };
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(request_body)
            .map_err(|error| format!("构造本机请求失败：{error}"))?;
        let response = self
            .management_router
            .clone()
            .oneshot(request)
            .await
            .map_err(|error| format!("执行本机请求失败：{error}"))?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .map_err(|error| format!("读取本机响应失败：{error}"))?;
        let response_value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        if status.is_success() {
            Ok(response_value)
        } else {
            let message = response_value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| response_value.as_str().map(str::to_string))
                .unwrap_or_else(|| format!("HTTP {status}"));
            Err(message)
        }
    }

    /// Invoke a private management endpoint from the desktop process.
    pub async fn request(
        &self,
        method: String,
        path: String,
        body: Option<Value>,
    ) -> Result<Value, String> {
        self.invoke(&method, &path, body).await
    }
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
    start_gateway_listener(state).await?;
    Ok(handle)
}

/// Starts the OpenAI-compatible HTTP gateway and returns the private desktop
/// management surface. This is the entry point used by the Tauri client.
pub async fn start_desktop_gateway() -> Result<DesktopGateway, String> {
    let (state, _) = initialize_local_gateway().await?;
    let desktop_gateway = DesktopGateway {
        management_router: build_management_router(state.clone()),
    };
    start_gateway_listener(state).await?;
    Ok(desktop_gateway)
}

async fn start_gateway_listener(state: AppState) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4242")
        .await
        .map_err(|error| format!("无法启动本地 Gateway (127.0.0.1:4242)：{error}"))?;
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, build_router(state)).await {
            eprintln!("本地 Gateway 已停止：{error}");
        }
    });
    Ok(())
}

/// Serves only the OpenAI-compatible HTTP gateway. Management is available
/// exclusively through the desktop client's in-process invoke bridge.
pub async fn serve_gateway() -> Result<(), String> {
    let (state, _) = initialize_local_gateway().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4242")
        .await
        .map_err(|error| format!("无法启动 Gateway 服务 (127.0.0.1:4242)：{error}"))?;
    axum::serve(listener, build_router(state))
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
