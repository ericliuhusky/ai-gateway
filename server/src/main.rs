mod adapters;
mod api;
mod auth;
mod codex_config;
mod codex_history;
mod config;
mod models;
mod store;
mod support;
mod upstream;

use api::{AppState, build_router};
use auth::OAuthClient;
use config::Config;
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
    let oauth = OAuthClient::new(config.clone());
    let upstream = UpstreamClient::new();
    let logs = LogStore::new(config.clone())?;

    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        oauth,
        accounts,
        providers,
        routes,
        models,
        upstream,
        logs,
    };

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    let openai_callback_listener =
        tokio::net::TcpListener::bind(config.openai_callback_addr()).await?;
    let callback_app = app.clone();
    let primary = axum::serve(listener, app);
    let callback = axum::serve(openai_callback_listener, callback_app);
    tokio::try_join!(primary, callback)?;

    Ok(())
}
