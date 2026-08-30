use codex_adapter::{
    CodexConfigurationResult, DefaultCodexStatus, default_codex_status,
    delete_codex_instance as remove_codex_instance_files,
    start_codex_gateway as patch_codex_gateway, start_codex_instance as launch_codex_instance,
    stop_codex_gateway as restore_codex_gateway,
};
use serde_json::Value;
use std::{
    env,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use tauri::State;

const LOCAL_API_ROOT: &str = "http://127.0.0.1:42401";
const LOCAL_GATEWAY_URL: &str = "http://127.0.0.1:42401/openai/v1";
const GATEWAY_DAEMON_BINARY: &str = "ai-gateway-daemon";
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(8);
const DAEMON_READY_POLL_INTERVAL: Duration = Duration::from_millis(200);

pub fn run() {
    let gateway = tauri::async_runtime::block_on(ensure_gateway_daemon())
        .unwrap_or_else(|error| panic!("启动本机 Gateway 服务失败：{error}"));
    match patch_codex_gateway(LOCAL_GATEWAY_URL) {
        Ok(result) => {
            for warning in result.warnings {
                eprintln!("{warning}");
            }
        }
        Err(error) => eprintln!("自动配置本机 Codex 失败：{error}"),
    }
    tauri::Builder::default()
        .manage(gateway)
        .invoke_handler(tauri::generate_handler![
            gateway_request,
            get_codex_gateway_status,
            start_codex_gateway,
            stop_codex_gateway,
            start_codex_instance,
            delete_codex_instance
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| panic!("运行 Tauri 客户端失败：{error}"));
}

async fn ensure_gateway_daemon() -> Result<gateway::GatewayDaemonClient, String> {
    let gateway = gateway::GatewayDaemonClient::local()?;
    let daemon = gateway_daemon_path()?;
    if gateway.is_ready().await
        && gateway::local_gateway_is_healthy().await
        && gateway::gateway_daemon_is_installed(&daemon)?
    {
        return Ok(gateway);
    }

    install_and_start_gateway_daemon(&daemon)?;
    let attempts =
        (DAEMON_READY_TIMEOUT.as_millis() / DAEMON_READY_POLL_INTERVAL.as_millis()) as u32;
    for _ in 0..attempts {
        if gateway.is_ready().await && gateway::local_gateway_is_healthy().await {
            return Ok(gateway);
        }
        thread::sleep(DAEMON_READY_POLL_INTERVAL);
    }
    Err(format!(
        "Gateway 服务未在 {} 秒内就绪；控制 Socket：{}",
        DAEMON_READY_TIMEOUT.as_secs(),
        gateway.control_socket_path().display()
    ))
}

fn install_and_start_gateway_daemon(daemon: &Path) -> Result<(), String> {
    gateway::install_gateway_daemon(daemon)
}

/// Tauri copies `bundle.externalBin` next to the desktop executable both in
/// Cargo's development output and in the macOS application bundle.
fn gateway_daemon_path() -> Result<PathBuf, String> {
    let desktop = env::current_exe().map_err(|error| format!("读取客户端路径失败：{error}"))?;
    gateway_daemon_path_from_desktop(&desktop)
}

fn gateway_daemon_path_from_desktop(desktop: &Path) -> Result<PathBuf, String> {
    let directory = desktop
        .parent()
        .ok_or_else(|| format!("客户端路径无效：{}", desktop.display()))?;
    let daemon = directory.join(GATEWAY_DAEMON_BINARY);
    if daemon.is_file() {
        Ok(daemon)
    } else {
        Err(format!("内置 Gateway 服务程序不存在：{}", daemon.display()))
    }
}

/// The WebView invokes this command. Rust then talks to the persistent daemon
/// through its private Unix socket; no management HTTP endpoint is exposed.
#[tauri::command]
async fn gateway_request(
    gateway: State<'_, gateway::GatewayDaemonClient>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, String> {
    gateway.request(method, path, body).await
}

#[tauri::command]
fn get_codex_gateway_status() -> Result<DefaultCodexStatus, String> {
    default_codex_status()
}

#[tauri::command]
fn start_codex_gateway() -> Result<CodexConfigurationResult, String> {
    patch_codex_gateway(LOCAL_GATEWAY_URL)
}

#[tauri::command]
fn stop_codex_gateway() -> Result<CodexConfigurationResult, String> {
    restore_codex_gateway()
}

#[tauri::command]
fn start_codex_instance(instance_id: String) -> Result<String, String> {
    launch_codex_instance(&instance_id, LOCAL_API_ROOT)?;
    Ok(instance_id)
}

#[tauri::command]
fn delete_codex_instance(instance_id: String) -> Result<bool, String> {
    remove_codex_instance_files(&instance_id)
}

#[cfg(test)]
mod tests {
    use super::{GATEWAY_DAEMON_BINARY, gateway_daemon_path_from_desktop};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn finds_the_sidecar_next_to_the_desktop_executable() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ai-gateway-desktop-test-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let desktop = root.join("ai-gateway-desktop");
        let daemon = root.join(GATEWAY_DAEMON_BINARY);
        fs::write(&desktop, []).unwrap();
        fs::write(&daemon, []).unwrap();

        assert_eq!(gateway_daemon_path_from_desktop(&desktop).unwrap(), daemon);

        fs::remove_dir_all(root).unwrap();
    }
}
