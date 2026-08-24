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
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use axum::{
    Router,
    http::{Method, header::ACCEPT},
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HyperServerBuilder,
    service::TowerToHyperService,
};
use serde_json::Value;
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

/// Client for the daemon's private Unix-domain-socket control API.
///
/// This client is owned by the desktop process and is reached through Tauri
/// `invoke`; the WebView never receives a TCP management endpoint.
#[derive(Clone)]
pub struct GatewayDaemonClient {
    control_socket: std::path::PathBuf,
    client: Client,
}

impl GatewayDaemonClient {
    pub fn local() -> Result<Self, String> {
        let config = Config::local()?;
        let control_socket = config.control_socket_path();
        let client = Client::builder()
            .unix_socket(control_socket.clone())
            .build()
            .map_err(|error| format!("创建本机控制连接失败：{error}"))?;
        Ok(Self {
            control_socket,
            client,
        })
    }

    pub fn control_socket_path(&self) -> &std::path::Path {
        &self.control_socket
    }

    pub async fn request(
        &self,
        method: String,
        path: String,
        body: Option<Value>,
    ) -> Result<Value, String> {
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

        let url = format!("http://localhost{path}");
        let mut request = self
            .client
            .request(method, url)
            .header(ACCEPT, "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("连接本机 Gateway 控制服务失败：{error}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("读取本机 Gateway 控制响应失败：{error}"))?;
        let value = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        if status.is_success() {
            Ok(value)
        } else {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| value.as_str().map(str::to_string))
                .unwrap_or_else(|| format!("HTTP {status}"));
            Err(message)
        }
    }

    pub async fn is_ready(&self) -> bool {
        self.request("GET".to_string(), "/control/status".to_string(), None)
            .await
            .is_ok()
    }
}

const SERVICE_LABEL: &str = "com.ai-gateway.server";

/// Installs (or updates) a per-user LaunchAgent that runs the supplied program
/// as the persistent gateway daemon. The program may be the standalone server
/// binary or the desktop executable in daemon mode.
pub fn install_gateway_daemon(program: &Path) -> Result<(), String> {
    if !program.is_file() {
        return Err(format!("Gateway 服务程序不存在：{}", program.display()));
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "未设置 HOME 环境变量".to_string())?;
    let root = home.join(".ai-gateway");
    let log_dir = root.join("log");
    let plist_path = home
        .join("Library/LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist"));
    let plist_dir = plist_path
        .parent()
        .ok_or_else(|| "LaunchAgent 路径无效".to_string())?;
    fs::create_dir_all(&log_dir).map_err(|error| format!("创建 Gateway 日志目录失败：{error}"))?;
    fs::create_dir_all(plist_dir).map_err(|error| format!("创建 LaunchAgent 目录失败：{error}"))?;
    fs::write(
        &plist_path,
        gateway_launchd_plist(program, &root, &home, &log_dir),
    )
    .map_err(|error| format!("写入 Gateway LaunchAgent 配置失败：{error}"))?;

    let target = format!("gui/{}/{}", current_uid(), SERVICE_LABEL);
    run_launchctl(&["bootout", &target], true)?;
    run_launchctl(
        &[
            "bootstrap",
            &format!("gui/{}", current_uid()),
            path_str(&plist_path)?,
        ],
        false,
    )?;
    run_launchctl(&["enable", &target], true)?;
    run_launchctl(&["kickstart", "-k", &target], false)?;
    Ok(())
}

fn gateway_launchd_plist(program: &Path, root: &Path, home: &Path, log_dir: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string></array>
  <key>WorkingDirectory</key><string>{}</string>
  <key>EnvironmentVariables</key><dict>
    <key>HOME</key><string>{}</string>
    <key>AI_GATEWAY_DAEMON</key><string>1</string>
  </dict>
  <key>StandardOutPath</key><string>{}/service.out.log</string>
  <key>StandardErrorPath</key><string>{}/service.err.log</string>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>"#,
        xml_escape(&program.display().to_string()),
        xml_escape(&root.display().to_string()),
        xml_escape(&home.display().to_string()),
        xml_escape(&log_dir.display().to_string()),
        xml_escape(&log_dir.display().to_string()),
    )
}

fn current_uid() -> u32 {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn run_launchctl(arguments: &[&str], allow_failure: bool) -> Result<String, String> {
    let output = Command::new("/bin/launchctl")
        .args(arguments)
        .output()
        .map_err(|error| format!("执行 launchctl 失败：{error}"))?;
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .trim()
    .to_string();
    if !allow_failure && !output.status.success() {
        return Err(format!("launchctl {} 失败：{message}", arguments.join(" ")));
    }
    Ok(message)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "路径不是有效 UTF-8".to_string())
}

pub async fn local_gateway_is_healthy() -> bool {
    Client::new()
        .get(format!("{LOCAL_API_ROOT}/healthz"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
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

/// Runs the persistent local daemon. The OpenAI-compatible data plane is
/// bound to loopback TCP, while management is served only over a user-owned
/// Unix domain socket.
pub async fn serve_gateway() -> Result<(), String> {
    let (state, _) = initialize_local_gateway().await?;
    let listen_addr = gateway_listen_addr();
    let tcp_listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .map_err(|error| format!("无法启动 Gateway 服务 ({listen_addr})：{error}"))?;
    let control_socket = control_socket_path()?;
    let unix_listener = bind_control_socket(&control_socket).await?;

    let gateway = axum::serve(tcp_listener, build_router(state.clone()));
    let management = serve_control_socket(unix_listener, build_management_router(state));
    let result = tokio::try_join!(
        async {
            gateway
                .await
                .map_err(|error| format!("Gateway 服务已停止：{error}"))
        },
        management,
    );
    let _ = fs::remove_file(&control_socket);
    result.map(|_| ())
}

fn gateway_listen_addr() -> String {
    env::var("AI_GATEWAY_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:4242".to_string())
}

pub fn control_socket_path() -> Result<PathBuf, String> {
    Ok(Config::local()?.control_socket_path())
}

async fn serve_control_socket(
    listener: tokio::net::UnixListener,
    router: Router,
) -> Result<(), String> {
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|error| format!("接受本机 Gateway 控制连接失败：{error}"))?;
        let service = TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            let result = HyperServerBuilder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await;
            if let Err(error) = result {
                eprintln!("本机 Gateway 控制连接已停止：{error}");
            }
        });
    }
}

async fn bind_control_socket(path: &std::path::Path) -> Result<tokio::net::UnixListener, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "本机控制 Socket 路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建本机控制目录失败：{error}"))?;
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("清理旧本机控制 Socket 失败：{error}")),
    }
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|error| format!("无法启动本机 Gateway 控制服务：{error}"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("设置本机控制 Socket 权限失败：{error}"))?;
    Ok(listener)
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

#[cfg(test)]
mod daemon_tests {
    use super::gateway_launchd_plist;
    use std::path::Path;

    #[test]
    fn launch_agent_runs_the_program_in_daemon_mode() {
        let plist = gateway_launchd_plist(
            Path::new("/Applications/AI Gateway.app/Contents/MacOS/AI Gateway"),
            Path::new("/Users/test/.ai-gateway"),
            Path::new("/Users/test"),
            Path::new("/Users/test/.ai-gateway/log"),
        );
        assert!(plist.contains("AI_GATEWAY_DAEMON</key><string>1"));
        assert!(plist.contains("/Applications/AI Gateway.app/Contents/MacOS/AI Gateway"));
    }
}
