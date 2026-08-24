#[tokio::main]
async fn main() {
    gateway::serve_gateway()
        .await
        .unwrap_or_else(|error| panic!("运行 Gateway 后台服务失败：{error}"));
}
