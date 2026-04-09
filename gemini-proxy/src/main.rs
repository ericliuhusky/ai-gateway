mod account_pool;
mod auth;
mod config;
mod handlers;
mod mapper;
mod models;
mod upstream;

use account_pool::AccountPool;
use auth::OAuthClient;
use axum::{
    Router,
    routing::{get, post},
};
use config::Config;
use handlers::{
    AppState, auth_google_callback, auth_google_start, healthz, list_accounts, responses,
};
use reqwest::Client;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use upstream::UpstreamClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gemini_proxy=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Arc::new(Config::from_env()?);
    let accounts = AccountPool::new(config.clone())?;
    let loaded = accounts.load().await?;
    let oauth = OAuthClient::new(config.clone());
    let upstream = UpstreamClient::new();

    tracing::info!("loaded {} account(s) from {:?}", loaded, config.data_dir());

    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        oauth,
        accounts,
        upstream,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/auth/google/start", get(auth_google_start))
        .route("/auth/google/callback", get(auth_google_callback))
        .route("/v1/accounts", get(list_accounts))
        .route("/v1/responses", post(responses))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}
