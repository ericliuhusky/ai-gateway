use crate::api::handlers::{
    add_provider, clear_codex_client_version, clear_selected_model,
    clear_selected_reasoning_effort, delete_provider, get_auto_routing_settings,
    get_codex_client_version, get_provider_quota, get_route, get_selected_model,
    get_selected_reasoning_effort, healthz, import_openai_token, list_models, list_providers,
    list_turn_logs, responses, set_auto_routing_settings, set_codex_client_version, set_route,
    set_selected_model, set_selected_reasoning_effort,
};
use crate::codex_scripts;
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
        .route("/codex/setup.sh", get(codex_scripts::setup_script))
        .route("/codex/restore.sh", get(codex_scripts::restore_script))
        .route("/codex/instances.sh", get(codex_scripts::instances_script))
        .route("/accounts/openai/import-token", post(import_openai_token))
        .route("/providers", get(list_providers).post(add_provider))
        .route("/providers/:provider_id", delete(delete_provider))
        .route("/providers/:provider_id/quota", get(get_provider_quota))
        .route(
            "/settings/codex-client-version",
            get(get_codex_client_version)
                .put(set_codex_client_version)
                .delete(clear_codex_client_version),
        )
        .route(
            "/settings/automatic-routing",
            get(get_auto_routing_settings).put(set_auto_routing_settings),
        )
        .route("/routing/turns", get(list_turn_logs))
        .route("/selected-provider", get(get_route).put(set_route))
        .route(
            "/selected-model",
            get(get_selected_model)
                .put(set_selected_model)
                .delete(clear_selected_model),
        )
        .route(
            "/selected-reasoning-effort",
            get(get_selected_reasoning_effort)
                .put(set_selected_reasoning_effort)
                .delete(clear_selected_reasoning_effort),
        )
        .route("/openai/v1/models", get(list_models))
        .route("/openai/v1/responses", post(responses))
        .nest_service("/assets", assets)
        .with_state(state)
        .fallback_service(ServeFile::new(index))
}
