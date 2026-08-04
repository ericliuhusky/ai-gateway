mod adapters;
mod api;
mod auth;
mod codex_scripts;
mod config;
mod crypto;
mod models;
mod openai_device_login;
mod openai_tokens;
mod routing;
mod store;
mod support;
mod upstream;

use api::{AppState, build_router};
use auth::AuthService;
use config::Config;
use openai_device_login::OpenAiDeviceLoginService;
use openai_tokens::OpenAiTokenService;
use reqwest::Client;
use std::sync::Arc;
use store::{
    AccountStore, GroupStore, IssueStore, ModelStore, ProviderStore, RouteStore, SettingsStore,
    TurnLogStore, UsageStore,
};
use upstream::UpstreamClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::from_env()?);
    let auth = AuthService::new(config.clone())?;
    auth.initialize()?;
    let accounts = AccountStore::new(config.clone())?;
    accounts.load().await?;
    let groups = GroupStore::new(config.clone())?;
    let providers = ProviderStore::new(config.clone())?;
    providers.load().await?;
    let routes = RouteStore::new(config.clone())?;
    routes.load().await?;
    let models = ModelStore::new(config.clone())?;
    let settings = SettingsStore::new(config.clone())?;
    let turn_logs = TurnLogStore::new(config.clone())?;
    let issues = IssueStore::new(config.clone())?;
    let usage = UsageStore::new(config.clone())?;
    let openai_tokens = OpenAiTokenService::new();
    let openai_device_login = OpenAiDeviceLoginService::new();
    let upstream = UpstreamClient::new();
    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        auth,
        openai_tokens,
        openai_device_login,
        accounts,
        groups,
        providers,
        routes,
        models,
        settings,
        turn_logs,
        issues,
        usage,
        upstream,
    };

    let app = build_router(state, config.web_dir());

    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
