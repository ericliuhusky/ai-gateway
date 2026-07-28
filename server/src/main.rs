mod adapters;
mod api;
mod config;
mod models;
mod openai_tokens;
mod store;
mod support;
mod upstream;

use api::{AppState, build_router};
use config::Config;
use openai_tokens::OpenAiTokenService;
use reqwest::Client;
use std::sync::Arc;
use store::{AccountStore, LogStore, ModelStore, ProviderStore, RouteStore};
use upstream::UpstreamClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::from_env()?);
    let accounts = AccountStore::new(config.clone())?;
    accounts.load().await?;
    let providers = ProviderStore::new(config.clone())?;
    providers.load().await?;
    let routes = RouteStore::new(config.clone())?;
    routes.load().await?;
    let models = ModelStore::new(config.clone())?;
    let openai_tokens = OpenAiTokenService::new();
    let upstream = UpstreamClient::new();
    let logs = LogStore::new(config.clone())?;

    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        openai_tokens,
        accounts,
        providers,
        routes,
        models,
        upstream,
        logs,
    };

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
