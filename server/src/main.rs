mod api;
mod auth;
mod config;
mod crypto;
mod models;
mod store;
mod support;

use api::{ControlState, build_router};
use auth::AuthService;
use config::Config;
use std::sync::Arc;
use store::{GroupStore, ProviderStore};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::from_env()?);
    let bind_addr = config.bind_addr();
    let auth = AuthService::new(config.clone())?;
    auth.initialize()?;
    let groups = GroupStore::new(config.clone())?;
    let providers = ProviderStore::new(config)?;
    providers.load().await?;

    let app = build_router(ControlState {
        auth,
        groups,
        providers,
    });
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
