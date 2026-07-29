mod adapters;
mod api;
mod codex_scripts;
mod config;
mod crypto;
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
use store::{AccountStore, ModelStore, ProviderStore, RouteStore, SettingsStore};
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
    let settings = SettingsStore::new(config.clone())?;
    let openai_tokens = OpenAiTokenService::new();
    let upstream = UpstreamClient::new();
    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        openai_tokens,
        accounts,
        providers,
        routes,
        models,
        settings,
        upstream,
    };

    let app = build_router(state, config.web_dir());

    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
