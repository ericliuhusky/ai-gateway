use crate::api::handlers::{
    add_provider, apply_codex_config, auth_google_callback, auth_google_start,
    auth_openai_callback, auth_openai_start, clear_logs, clear_selected_model, debug_clear_logs,
    debug_dashboard, debug_set_log_settings, get_codex_config_status, get_log_detail,
    get_log_settings, get_logs, get_provider_quota, get_route, get_selected_model, healthz,
    import_openai_from_local_codex_auth, list_models, list_providers, responses,
    restore_codex_config, set_log_settings, set_route, set_selected_model,
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
        .route(
            "/auth/openai/import-local",
            post(import_openai_from_local_codex_auth),
        )
        .route("/providers", get(list_providers).post(add_provider))
        .route("/providers/:provider_id/quota", get(get_provider_quota))
        .route("/selected-provider", get(get_route).put(set_route))
        .route(
            "/selected-model",
            get(get_selected_model)
                .put(set_selected_model)
                .delete(clear_selected_model),
        )
        .route(
            "/codex-config",
            get(get_codex_config_status)
                .put(apply_codex_config)
                .delete(restore_codex_config),
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
        .with_state(state)
}
