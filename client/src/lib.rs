use codex_adapter::{
    CodexConfigurationResult, DefaultCodexStatus, default_codex_status,
    delete_codex_instance as remove_codex_instance_files,
    start_codex_gateway as patch_codex_gateway, start_codex_instance as launch_codex_instance,
    stop_codex_gateway as restore_codex_gateway,
};

const LOCAL_API_ROOT: &str = "http://127.0.0.1:4242";
const LOCAL_GATEWAY_URL: &str = "http://127.0.0.1:4242/openai/v1";

pub fn run() {
    tauri::async_runtime::block_on(ensure_local_gateway())
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
        .invoke_handler(tauri::generate_handler![
            get_codex_gateway_status,
            start_codex_gateway,
            stop_codex_gateway,
            start_codex_instance,
            delete_codex_instance
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| panic!("运行 Tauri 客户端失败：{error}"));
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

async fn ensure_local_gateway() -> Result<(), String> {
    if local_gateway_is_healthy().await {
        return Ok(());
    }
    server::start_local_gateway().await.map(|_| ())
}

async fn local_gateway_is_healthy() -> bool {
    reqwest::Client::new()
        .get(format!("{LOCAL_API_ROOT}/healthz"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}
