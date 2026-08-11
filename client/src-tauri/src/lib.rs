mod gateway;
mod store;

use ai_gateway_local_core::{
    CodexConfigurationResult, CodexInstancePaths, DefaultCodexStatus, LOCAL_API_ROOT,
    LOCAL_GATEWAY_URL, LocalGatewayHandle, default_codex_status,
    delete_codex_instance as delete_codex_instance_files, prepare_codex_instance,
    start_default_codex, start_local_gateway, stop_default_codex,
};
use gateway::{
    ControlLoginInput, ControlLoginResult, ControlRequestInput, SharedSyncStatus,
    login_control_plane, publish_shared_connection, request_control_plane,
    spawn_periodic_shared_sync, sync_shared_providers,
};
use serde_json::Value;
#[cfg(target_os = "macos")]
use std::{path::Path, process::Command, thread, time::Duration};
use store::{LocalGatewayStatus, LocalStore};
use tauri::State;

#[derive(Clone)]
struct ClientState {
    store: LocalStore,
    gateway: LocalGatewayHandle,
    local_gateway_url: String,
}

#[derive(serde::Deserialize)]
struct ConfigureControlPlaneInput {
    url: String,
    access_token: String,
}

#[tauri::command]
fn gateway_status(state: State<'_, ClientState>) -> Result<LocalGatewayStatus, String> {
    state.store.status(&state.local_gateway_url)
}

#[tauri::command]
fn codex_gateway_status() -> Result<DefaultCodexStatus, String> {
    default_codex_status()
}

#[tauri::command]
async fn start_codex_gateway() -> Result<CodexConfigurationResult, String> {
    tauri::async_runtime::spawn_blocking(|| configure_codex(true))
        .await
        .map_err(|error| format!("启动 Codex 网关任务失败：{error}"))?
}

#[tauri::command]
async fn stop_codex_gateway() -> Result<CodexConfigurationResult, String> {
    tauri::async_runtime::spawn_blocking(|| configure_codex(false))
        .await
        .map_err(|error| format!("停止 Codex 网关任务失败：{error}"))?
}

#[tauri::command]
async fn start_codex_instance(instance_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let gateway_url = format!("{LOCAL_API_ROOT}/instances/{instance_id}/openai/v1");
        let paths = prepare_codex_instance(&instance_id, &gateway_url)?;
        open_codex_instance(&paths)
    })
    .await
    .map_err(|error| format!("启动 Codex 实例任务失败：{error}"))?
}

#[tauri::command]
async fn delete_codex_instance(instance_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        delete_codex_instance_files(&instance_id)?;
        Ok(())
    })
    .await
    .map_err(|error| format!("删除 Codex 实例任务失败：{error}"))?
}

fn configure_codex(start: bool) -> Result<CodexConfigurationResult, String> {
    let mut result = if start {
        start_default_codex(LOCAL_GATEWAY_URL)?
    } else {
        stop_default_codex()?
    };
    if result.changed {
        if let Some(warning) = restart_codex() {
            result.warnings.push(warning);
        }
    }
    Ok(result)
}

fn open_codex_instance(paths: &CodexInstancePaths) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if !Path::new("/Applications/ChatGPT.app").is_dir() {
            return Err("未找到 /Applications/ChatGPT.app".to_string());
        }
        let codex_home = paths.codex_home.display().to_string();
        let electron_home = paths.electron_home.display().to_string();
        let codex_env = format!("CODEX_HOME={codex_home}");
        let electron_env = format!("CODEX_ELECTRON_USER_DATA_PATH={electron_home}");
        let user_data_dir = format!("--user-data-dir={electron_home}");
        let status = Command::new("open")
            .args([
                "-n",
                "-a",
                "ChatGPT",
                "--env",
                &codex_env,
                "--env",
                &electron_env,
                "--args",
                &user_data_dir,
            ])
            .status()
            .map_err(|error| format!("启动 Codex 实例失败：{error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("启动 Codex 实例失败".to_string())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = paths;
        Err("当前 Codex 实例启动仅支持 macOS".to_string())
    }
}

fn restart_codex() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if !Path::new("/Applications/ChatGPT.app").is_dir() {
            return Some("未找到 ChatGPT.app，请手动重新启动 Codex 以加载新配置。".to_string());
        }
        let running = Command::new("pgrep")
            .args(["-x", "ChatGPT"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if running {
            if !Command::new("osascript")
                .args(["-e", "tell application \"ChatGPT\" to quit"])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
            {
                return Some("无法自动退出 Codex，请手动完全退出后重新打开。".to_string());
            }
            for _ in 0..10 {
                if !Command::new("pgrep")
                    .args(["-x", "ChatGPT"])
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false)
                {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
            if Command::new("pgrep")
                .args(["-x", "ChatGPT"])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
            {
                return Some("Codex 未在 10 秒内完全退出，请手动完全退出后重新打开。".to_string());
            }
        }
        if !Command::new("open")
            .args(["-a", "ChatGPT"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Some("无法自动打开 Codex，请手动重新打开 ChatGPT.app。".to_string());
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some("请手动重新启动 Codex 以加载新配置。".to_string())
    }
}

#[tauri::command]
fn configure_control_plane(
    state: State<'_, ClientState>,
    input: ConfigureControlPlaneInput,
) -> Result<(), String> {
    state
        .store
        .configure_control_plane(input.url, input.access_token)
}

#[tauri::command]
async fn disconnect_control_plane(state: State<'_, ClientState>) -> Result<(), String> {
    let shared_ids = state.gateway.shared_provider_ids().await;
    for provider_id in shared_ids {
        state.gateway.revoke_shared_provider(&provider_id).await?;
    }
    state.store.clear_control_plane()
}

#[tauri::command]
async fn sync_shared_connections(
    state: State<'_, ClientState>,
) -> Result<SharedSyncStatus, String> {
    sync_shared_providers(state.store.clone(), state.gateway.clone()).await
}

#[tauri::command]
async fn login_center(
    state: State<'_, ClientState>,
    input: ControlLoginInput,
) -> Result<ControlLoginResult, String> {
    login_control_plane(state.store.clone(), input).await
}

#[tauri::command]
async fn center_request(
    state: State<'_, ClientState>,
    input: ControlRequestInput,
) -> Result<Value, String> {
    request_control_plane(state.store.clone(), input).await
}

#[tauri::command]
async fn share_local_provider(
    state: State<'_, ClientState>,
    provider_id: String,
) -> Result<String, String> {
    publish_shared_connection(state.store.clone(), state.gateway.clone(), provider_id).await
}

pub fn run() {
    let store =
        LocalStore::open().unwrap_or_else(|error| panic!("初始化本地凭据存储失败：{error}"));
    let gateway = tauri::async_runtime::block_on(start_local_gateway())
        .unwrap_or_else(|error| panic!("启动本地 Gateway 失败：{error}"));
    let local_gateway_url = LOCAL_GATEWAY_URL.to_string();
    if let Err(error) = configure_codex(true) {
        eprintln!("自动启动 Codex 网关失败：{error}");
    }
    spawn_periodic_shared_sync(store.clone(), gateway.clone());

    tauri::Builder::default()
        .manage(ClientState {
            store,
            gateway,
            local_gateway_url,
        })
        .invoke_handler(tauri::generate_handler![
            gateway_status,
            codex_gateway_status,
            start_codex_gateway,
            stop_codex_gateway,
            start_codex_instance,
            delete_codex_instance,
            configure_control_plane,
            disconnect_control_plane,
            sync_shared_connections,
            login_center,
            center_request,
            share_local_provider,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| panic!("运行 Tauri 客户端失败：{error}"));
}
