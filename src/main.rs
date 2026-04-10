mod account_pool;
mod auth;
mod config;
mod handlers;
mod mapper;
mod models;
mod provider_store;
mod route_store;
mod upstream;

use account_pool::AccountPool;
use auth::OAuthClient;
use axum::{
    Router,
    routing::{get, post},
};
use config::Config;
use handlers::{
    AppState, add_provider, auth_google_callback, auth_google_start, auth_openai_callback,
    auth_openai_start, get_route, healthz, list_accounts, list_providers, responses, set_route,
};
use provider_store::ProviderStore;
use reqwest::Client;
use route_store::RouteStore;
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
    let providers = ProviderStore::new(config.clone())?;
    let provider_count = providers.load().await?;
    let routes = RouteStore::new(config.clone())?;
    let route = routes.load().await?;
    let oauth = OAuthClient::new(config.clone());
    let upstream = UpstreamClient::new();

    tracing::info!(
        "loaded {} account(s), {} provider(s), current route {:?} from {:?}",
        loaded,
        provider_count,
        route.provider,
        config.data_dir()
    );

    let state = AppState {
        _client: Client::new(),
        _config: config.clone(),
        oauth,
        accounts,
        providers,
        routes,
        upstream,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/auth/google/start", get(auth_google_start))
        .route("/auth/google/callback", get(auth_google_callback))
        .route("/auth/openai/start", get(auth_openai_start))
        .route("/auth/callback", get(auth_openai_callback))
        .route("/auth/openai/callback", get(auth_openai_callback))
        .route("/v1/accounts", get(list_accounts))
        .route("/v1/providers", get(list_providers).post(add_provider))
        .route("/v1/route", get(get_route).post(set_route))
        .route("/v1/responses", post(responses))
        .with_state(state);

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
