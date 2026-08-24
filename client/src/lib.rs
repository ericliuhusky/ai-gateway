use codex_adapter::{
    CodexConfigurationResult, DefaultCodexStatus, default_codex_status,
    delete_codex_instance as remove_codex_instance_files,
    start_codex_gateway as patch_codex_gateway, start_codex_instance as launch_codex_instance,
    stop_codex_gateway as restore_codex_gateway,
};
use serde_json::Value;
use std::{env, thread, time::Duration};
use tauri::State;

const LOCAL_API_ROOT: &str = "http://127.0.0.1:4242";
const LOCAL_GATEWAY_URL: &str = "http://127.0.0.1:4242/openai/v1";
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

async fn ensure_gateway_daemon() -> Result<server::GatewayDaemonClient, String> {
    let gateway = server::GatewayDaemonClient::local()?;
    if gateway.is_ready().await && server::local_gateway_is_healthy().await {
        return Ok(gateway);
    }

    install_and_start_gateway_daemon()?;
    let attempts =
        (DAEMON_READY_TIMEOUT.as_millis() / DAEMON_READY_POLL_INTERVAL.as_millis()) as u32;
    for _ in 0..attempts {
        if gateway.is_ready().await && server::local_gateway_is_healthy().await {
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

fn install_and_start_gateway_daemon() -> Result<(), String> {
    let executable = env::current_exe().map_err(|error| format!("读取客户端路径失败：{error}"))?;
    server::install_gateway_daemon(&executable)
}

/// The WebView invokes this command. Rust then talks to the persistent daemon
/// through its private Unix socket; no management HTTP endpoint is exposed.
#[tauri::command]
async fn gateway_request(
    gateway: State<'_, server::GatewayDaemonClient>,
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
