mod gateway;
mod store;

use ai_gateway_local_core::{LOCAL_GATEWAY_URL, LocalGatewayHandle, start_local_gateway};
use gateway::{
    ControlLoginInput, ControlLoginResult, ControlRequestInput, SharedSyncStatus,
    login_control_plane, publish_shared_connection, request_control_plane,
    spawn_periodic_shared_sync, sync_shared_providers,
};
use serde_json::Value;
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
    spawn_periodic_shared_sync(store.clone(), gateway.clone());

    tauri::Builder::default()
        .manage(ClientState {
            store,
            gateway,
            local_gateway_url,
        })
        .invoke_handler(tauri::generate_handler![
            gateway_status,
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
