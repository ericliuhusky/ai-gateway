mod codex_config;
mod codex_history;
mod config;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, sync::Arc};
use url::Url;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
}

#[derive(Debug, Deserialize)]
struct ApplyCodexConfigRequest {
    gateway_base_url: String,
}

#[derive(Debug, Serialize)]
struct AgentErrorBody {
    error: AgentErrorMessage,
}

#[derive(Debug, Serialize)]
struct AgentErrorMessage {
    message: String,
}

#[derive(Debug)]
struct AgentError {
    status: StatusCode,
    message: String,
}

impl AgentError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl IntoResponse for AgentError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(AgentErrorBody {
                error: AgentErrorMessage {
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::from_env()?);
    let state = AppState {
        config: config.clone(),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/codex-config",
            get(get_codex_config_status)
                .put(apply_codex_config)
                .delete(restore_codex_config),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr()).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn get_codex_config_status(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "codex_config": codex_config_status(&state.config) }))
}

async fn apply_codex_config(
    State(state): State<AppState>,
    Json(request): Json<ApplyCodexConfigRequest>,
) -> Result<Json<Value>, AgentError> {
    let gateway_base_url = normalize_gateway_base_url(&request.gateway_base_url)?;
    let config = state.config.as_ref();

    fs::create_dir_all(config.data_dir())
        .map_err(|err| AgentError::bad_request(format!("failed to create data dir: {err}")))?;
    fs::create_dir_all(config.codex_dir())
        .map_err(|err| AgentError::bad_request(format!("failed to create Codex dir: {err}")))?;

    let takeover = codex_config::apply_takeover(
        &config.codex_config_path(),
        &config.codex_config_patch_path(),
        &gateway_base_url,
    )
    .map_err(AgentError::bad_request)?;

    let (history_aliases, history_warning) = match codex_history::sync_openai_history_aliases(
        &config.codex_dir(),
        &config.codex_state_path(),
        &config.codex_session_alias_patch_path(),
    ) {
        Ok(summary) => (serde_json::to_value(summary).ok(), None),
        Err(error) => (None, Some(error)),
    };

    Ok(Json(json!({
        "codex_config": codex_config_status(config),
        "takeover": takeover,
        "history_aliases": history_aliases,
        "history_warning": history_warning,
    })))
}

async fn restore_codex_config(State(state): State<AppState>) -> Result<Json<Value>, AgentError> {
    let config = state.config.as_ref();
    let restore = codex_config::restore_takeover(
        &config.codex_config_path(),
        &config.codex_config_patch_path(),
    )
    .map_err(AgentError::bad_request)?;

    Ok(Json(json!({
        "codex_config": codex_config_status(config),
        "restore": restore,
    })))
}

fn normalize_gateway_base_url(value: &str) -> Result<String, AgentError> {
    let trimmed = value.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed)
        .map_err(|err| AgentError::bad_request(format!("invalid gateway_base_url: {err}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AgentError::bad_request(
            "gateway_base_url must use http or https",
        ));
    }
    if parsed.host_str().is_none() {
        return Err(AgentError::bad_request(
            "gateway_base_url must include a host",
        ));
    }
    Ok(trimmed.to_string())
}

fn codex_config_status(config: &Config) -> Value {
    let patch_exists = codex_config::patch_exists(&config.codex_config_patch_path());
    json!({
        "target_path": config.codex_config_path().display().to_string(),
        "config_patch_exists": patch_exists,
        "restore_available": patch_exists,
        "target_exists": config.codex_config_path().exists(),
        "state_exists": config.codex_state_path().exists(),
        "auth_exists": config.codex_auth_path().exists(),
        "version_exists": config.codex_version_path().exists(),
        "home_path": config.home_dir().display().to_string(),
    })
}
