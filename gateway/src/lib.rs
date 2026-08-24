mod adapters;
mod api;
mod config;
mod control;
mod crypto;
mod models;
mod openai_device_login;
mod openai_tokens;
mod routing;
mod store;
mod support;
mod upstream;

use api::{AppState, build_management_router, build_router};
use config::Config;
use openai_device_login::OpenAiDeviceLoginService;
use openai_tokens::OpenAiTokenService;
use reqwest::Client;
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
pub use models::{ProviderCompatibilityProfile, ProviderUpstreamProtocol};

pub const LOCAL_GATEWAY_URL: &str = "http://127.0.0.1:4242/openai/v1";
pub const LOCAL_API_ROOT: &str = "http://127.0.0.1:4242";

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

/// Returns whether the current user's LaunchAgent already points to this
/// Gateway daemon executable with the expected service configuration.
pub fn gateway_daemon_is_installed(program: &Path) -> Result<bool, String> {
    if !program.is_file() {
        return Ok(false);
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "未设置 HOME 环境变量".to_string())?;
    let root = home.join(".ai-gateway");
    let log_dir = root.join("log");
    let plist_path = home
        .join("Library/LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist"));
    let expected = gateway_launchd_plist(program, &root, &home, &log_dir);
    match fs::read_to_string(plist_path) {
        Ok(actual) => Ok(actual == expected),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("读取 Gateway LaunchAgent 配置失败：{error}")),
    }
}

/// Installs (or updates) a per-user LaunchAgent that runs the supplied
/// Gateway daemon executable.
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
    if let Err(bootstrap_error) = run_launchctl(
        &[
            "bootstrap",
            &format!("gui/{}", current_uid()),
            path_str(&plist_path)?,
        ],
        false,
    ) {
        // launchd can keep a just-booted-out job registered briefly. In that
        // case bootstrapping the same label returns EIO, while kickstart can
        // still restart the already registered job using the updated plist.
        run_launchctl(&["kickstart", "-k", &target], false).map_err(|restart_error| {
            format!(
                "启动 Gateway LaunchAgent 失败：{bootstrap_error}；尝试重启已注册服务也失败：{restart_error}"
            )
        })?;
    }
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

/// Runs the persistent local daemon. The OpenAI-compatible data plane is
/// bound to loopback TCP, while management is served only over a user-owned
/// Unix domain socket.
pub async fn serve_gateway() -> Result<(), String> {
    let state = initialize_local_gateway().await?;
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

async fn initialize_local_gateway() -> Result<AppState, String> {
    let config = Arc::new(Config::local()?);
    let accounts = AccountStore::new(config.clone())?;
    accounts.load().await?;
    let providers = ProviderStore::new(config.clone())?;
    providers.load().await?;
    let routes = RouteStore::new(config.clone())?;
    routes.load().await?;
    let models = ModelStore::new(config.clone())?;
    let settings = SettingsStore::new(config.clone())?;
    let gateway_runtime = GatewayRuntime::new(true);
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
        gateway_runtime,
    };
    Ok(state)
}

#[cfg(test)]
mod daemon_tests {
    use super::gateway_launchd_plist;
    use std::path::Path;

    #[test]
    fn launch_agent_runs_the_gateway_daemon_binary() {
        let plist = gateway_launchd_plist(
            Path::new("/Applications/AI Gateway.app/Contents/MacOS/ai-gateway-daemon"),
            Path::new("/Users/test/.ai-gateway"),
            Path::new("/Users/test"),
            Path::new("/Users/test/.ai-gateway/log"),
        );
        assert!(!plist.contains("AI_GATEWAY_DAEMON"));
        assert!(plist.contains("/Applications/AI Gateway.app/Contents/MacOS/ai-gateway-daemon"));
    }
}
