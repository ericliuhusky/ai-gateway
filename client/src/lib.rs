use reqwest::Client;
use std::{process::Command, time::Duration};

const LOCAL_API_ROOT: &str = "http://127.0.0.1:4242";
const SERVER_BINARY: &str = "ai-gateway-server";

pub fn run() {
    tauri::async_runtime::block_on(ensure_gateway_server())
        .unwrap_or_else(|error| panic!("启动 Gateway Server 失败：{error}"));
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| panic!("运行 Tauri 客户端失败：{error}"));
}

async fn ensure_gateway_server() -> Result<(), String> {
    if gateway_is_healthy().await {
        return Ok(());
    }
    let status = Command::new(SERVER_BINARY)
        .arg("start")
        .status()
        .map_err(|error| format!("无法执行 {SERVER_BINARY} start：{error}"))?;
    if !status.success() {
        return Err("Gateway Server 启动命令执行失败；请先安装 Gateway Server".to_string());
    }
    for _ in 0..20 {
        if gateway_is_healthy().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err("Gateway Server 未在 5 秒内通过健康检查".to_string())
}

async fn gateway_is_healthy() -> bool {
    Client::new()
        .get(format!("{LOCAL_API_ROOT}/healthz"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}
