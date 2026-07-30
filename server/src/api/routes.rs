use crate::{
    api::handlers::{
        add_provider, cancel_openai_device_login, clear_codex_client_version, clear_selected_model,
        clear_selected_reasoning_effort, delete_instance, delete_provider,
        get_auto_routing_settings, get_codex_client_version, get_instance_routing_config,
        get_provider_quota, get_route, get_selected_model, get_selected_reasoning_effort, healthz,
        import_openai_token, list_instance_routing_configs, list_models, list_models_for_instance,
        list_providers, list_turn_logs, poll_openai_device_login, responses,
        responses_for_instance, set_auto_routing_settings, set_codex_client_version,
        set_instance_routing_config, set_route, set_selected_model, set_selected_reasoning_effort,
        start_openai_device_login,
    },
    auth::{self, require_auth},
    codex_scripts,
};
use axum::{
    Router, middleware,
    routing::{delete, get, post},
};
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

use super::AppState;

pub fn build_router(state: AppState, web_dir: PathBuf) -> Router {
    let index = web_dir.join("index.html");
    let assets = ServeDir::new(web_dir.join("assets"));

    let protected = Router::new()
        .route("/accounts/openai/import-token", post(import_openai_token))
        .route(
            "/accounts/openai/login/device",
            post(start_openai_device_login),
        )
        .route(
            "/accounts/openai/login/device/:login_id",
            get(poll_openai_device_login).delete(cancel_openai_device_login),
        )
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
        .route("/instances", get(list_instance_routing_configs))
        .route("/instances/:instance_id", delete(delete_instance))
        .route(
            "/instances/:instance_id/config",
            get(get_instance_routing_config).put(set_instance_routing_config),
        )
        .route_layer(middleware::from_fn_with_state(
            state.auth.clone(),
            require_auth,
        ))
        .with_state(state.clone());

    let auth_routes = Router::new()
        .route("/auth/status", get(auth::auth_status))
        .route("/auth/feishu/authorize", get(auth::feishu_authorize))
        .route("/auth/feishu/callback", get(auth::feishu_callback))
        .route("/auth/me", get(auth::me))
        .route("/auth/logout", post(auth::logout))
        .with_state(state.auth.clone());

    let gateway_routes = Router::new()
        // Gateway request endpoints remain token-free for backward-compatible Codex access.
        .route("/openai/v1/models", get(list_models))
        .route("/openai/v1/responses", post(responses))
        .route(
            "/instances/:instance_id/openai/v1/models",
            get(list_models_for_instance),
        )
        .route(
            "/instances/:instance_id/openai/v1/responses",
            post(responses_for_instance),
        )
        .with_state(state);

    Router::new()
        .route("/healthz", get(healthz))
        .route("/codex/setup.sh", get(codex_scripts::setup_script))
        .route("/codex/restore.sh", get(codex_scripts::restore_script))
        .route("/codex/instances.sh", get(codex_scripts::instances_script))
        .merge(auth_routes)
        .merge(protected)
        .merge(gateway_routes)
        .nest_service("/assets", assets)
        .fallback_service(ServeFile::new(index))
}
