mod adapters;
mod api;
mod auth;
mod config;
mod models;
mod store;
mod upstream;

use api::{AppState, build_router};
use auth::OAuthClient;
use config::Config;
use reqwest::Client;
use std::sync::Arc;
use store::{AccountPool, LogStore, ProviderStore, RouteStore};
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
    let providers = ProviderStore::new(config.clone())?;
    let provider_count = providers.load().await?;
    let routes = RouteStore::new(config.clone())?;
    let route = routes.load().await?;
    let oauth = OAuthClient::new(config.clone());
    let upstream = UpstreamClient::new();
    let logs = LogStore::new(config.clone())?;

    tracing::info!(
        "loaded {} account(s), {} provider(s), current route {:?} from {:?}, logs at {:?} (max {} rows)",
        loaded,
        provider_count,
        route.provider_id,
        config.sqlite_path(),
        logs.db_path(),
        logs.max_rows()
    );

    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        oauth,
        accounts,
        providers,
        routes,
        upstream,
        logs,
    };

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    let openai_callback_listener =
        tokio::net::TcpListener::bind(config.openai_callback_addr()).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    tracing::info!(
        "listening on {} for OpenAI OAuth callback",
        config.openai_callback_url()
    );
    let callback_app = app.clone();
    let primary = axum::serve(listener, app);
    let callback = axum::serve(openai_callback_listener, callback_app);
    tokio::try_join!(primary, callback)?;

    Ok(())
}
