use codex_adapter::{
    CodexConfigurationResult, DefaultCodexStatus, default_codex_status,
    delete_codex_instance as remove_codex_instance_files,
    start_codex_gateway as patch_codex_gateway, start_codex_instance as launch_codex_instance,
    stop_codex_gateway as restore_codex_gateway,
};
use serde_json::Value;
use tauri::State;

const LOCAL_API_ROOT: &str = "http://127.0.0.1:4242";
const LOCAL_GATEWAY_URL: &str = "http://127.0.0.1:4242/openai/v1";

pub fn run() {
    let desktop_gateway = tauri::async_runtime::block_on(server::start_desktop_gateway())
        .unwrap_or_else(|error| panic!("启动本机 Gateway 失败：{error}"));
    match patch_codex_gateway(LOCAL_GATEWAY_URL) {
        Ok(result) => {
            for warning in result.warnings {
                eprintln!("{warning}");
            }
        }
        Err(error) => eprintln!("自动配置本机 Codex 失败：{error}"),
    }
    tauri::Builder::default()
        .manage(desktop_gateway)
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

/// Dispatches desktop-only management calls in Rust. It never issues an HTTP
/// request to the local gateway, so these routes are not reachable over TCP.
#[tauri::command]
async fn gateway_request(
    gateway: State<'_, server::DesktopGateway>,
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
