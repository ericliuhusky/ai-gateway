fn main() {
    if std::env::var_os("AI_GATEWAY_DAEMON").is_some() {
        tauri::async_runtime::block_on(gateway::serve_gateway())
            .unwrap_or_else(|error| panic!("运行 Gateway 后台服务失败：{error}"));
        return;
    }
    ai_gateway_desktop_lib::run();
}
