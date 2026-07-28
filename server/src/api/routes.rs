use crate::api::handlers::{
    add_provider, clear_logs, clear_selected_model, debug_clear_logs, debug_dashboard,
    debug_set_log_settings, delete_provider, get_log_detail, get_log_settings, get_logs,
    get_provider_quota, get_route, get_selected_model, healthz, import_openai_token, list_models,
    list_providers, responses, set_log_settings, set_route, set_selected_model,
};
use axum::{
    Router,
    routing::{delete, get, post},
};
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

use super::AppState;

pub fn build_router(state: AppState, web_dir: PathBuf) -> Router {
    let index = web_dir.join("index.html");
    let assets = ServeDir::new(web_dir.join("assets"));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/accounts/openai/import-token", post(import_openai_token))
        .route("/providers", get(list_providers).post(add_provider))
        .route("/providers/:provider_id", delete(delete_provider))
        .route("/providers/:provider_id/quota", get(get_provider_quota))
        .route("/selected-provider", get(get_route).put(set_route))
        .route(
            "/selected-model",
            get(get_selected_model)
                .put(set_selected_model)
                .delete(clear_selected_model),
        )
        .route("/logs", get(get_logs).delete(clear_logs))
        .route(
            "/logs/settings",
            get(get_log_settings).put(set_log_settings),
        )
        .route("/logs/:id", get(get_log_detail))
        .route("/debug", get(debug_dashboard))
        .route("/debug/logging", post(debug_set_log_settings))
        .route("/debug/clear", post(debug_clear_logs))
        .route("/openai/v1/models", get(list_models))
        .route("/openai/v1/responses", post(responses))
        .nest_service("/assets", assets)
        .with_state(state)
        .fallback_service(ServeFile::new(index))
}
