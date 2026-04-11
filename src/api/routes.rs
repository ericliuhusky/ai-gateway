use crate::api::handlers::{
    add_provider, apply_codex_config, auth_google_callback, auth_google_start,
    auth_openai_callback, auth_openai_start, get_codex_config_status, get_route, healthz,
    list_models, list_providers, responses, restore_codex_config, set_route,
};
use axum::{
    Router,
    routing::{get, post},
};

use super::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/auth/google/start", get(auth_google_start))
        .route("/auth/google/callback", get(auth_google_callback))
        .route("/auth/openai/start", get(auth_openai_start))
        .route("/auth/callback", get(auth_openai_callback))
        .route("/auth/openai/callback", get(auth_openai_callback))
        .route("/providers", get(list_providers).post(add_provider))
        .route("/selected-provider", get(get_route).put(set_route))
        .route(
            "/codex-config",
            get(get_codex_config_status)
                .put(apply_codex_config)
                .delete(restore_codex_config),
        )
        .route("/openai/v1/models", get(list_models))
        .route("/openai/v1/responses", post(responses))
        .with_state(state)
}
