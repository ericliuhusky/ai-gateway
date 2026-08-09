use crate::{
    api::RequestScope,
    api::handlers::{
        add_provider, cancel_openai_device_login, clear_codex_client_version, clear_gateway_issues,
        clear_selected_model, clear_selected_reasoning_effort, delete_instance, delete_provider,
        get_auto_routing_settings, get_codex_client_version, get_gateway_issue_repair_prompt,
        get_instance_routing_config, get_provider_quota, get_route, get_selected_model,
        get_selected_reasoning_effort, healthz, import_openai_token, list_daily_usage,
        list_gateway_issues, list_instance_routing_configs, list_models, list_models_for_instance,
        list_providers, list_turn_logs, list_usage_summary, poll_openai_device_login, responses,
        responses_for_instance, run_model_benchmark, set_auto_routing_settings,
        set_codex_client_version, set_instance_routing_config, set_route, set_selected_model,
        set_selected_reasoning_effort, start_openai_device_login,
    },
    codex_scripts,
};
use axum::{
    Router,
    extract::Request,
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, post},
};
use tower_http::cors::{Any, CorsLayer};

use super::AppState;

pub fn build_router(state: AppState) -> Router {
    let management = Router::new()
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
        .route("/benchmarks/models", post(run_model_benchmark))
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
        .route(
            "/gateway/issues",
            get(list_gateway_issues).delete(clear_gateway_issues),
        )
        .route(
            "/gateway/issues/:issue_id/repair-prompt",
            get(get_gateway_issue_repair_prompt),
        )
        .route("/usage/summary", get(list_usage_summary))
        .route("/usage/daily", get(list_daily_usage))
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
        .route_layer(middleware::from_fn(local_scope))
        .with_state(state.clone());

    let gateway = Router::new()
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
        .route_layer(middleware::from_fn(local_scope))
        .with_state(state);

    Router::new()
        .route("/healthz", get(healthz))
        .route("/codex/setup.sh", get(codex_scripts::setup_script))
        .route("/codex/restore.sh", get(codex_scripts::restore_script))
        .route("/codex/instances.sh", get(codex_scripts::instances_script))
        .merge(management)
        .merge(gateway)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any),
        )
}

async fn local_scope(mut request: Request, next: Next) -> Response {
    request.extensions_mut().insert(RequestScope {
        owner_user_id: None,
    });
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::build_router;
    use crate::{
        api::AppState,
        config::Config,
        models::CreateApiProviderRequest,
        models::{ProviderCompatibilityProfile, ProviderUpstreamProtocol},
        openai_device_login::OpenAiDeviceLoginService,
        openai_tokens::OpenAiTokenService,
        shared_leases::SharedLeaseStore,
        store::{
            AccountStore, IssueStore, ModelStore, ProviderStore, RouteStore, SettingsStore,
            TurnLogStore, UsageStore,
        },
        upstream::UpstreamClient,
    };
    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::State,
        http::{HeaderMap, Method, Request, StatusCode},
        routing::post,
    };
    use http_body_util::BodyExt;
    use reqwest::Client;
    use serde_json::{Value, json};
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };
    use tower::ServiceExt;

    #[derive(Clone, Debug)]
    struct CapturedUpstreamRequest {
        authorization: Option<String>,
        body: Value,
    }

    #[tokio::test]
    async fn local_gateway_needs_no_login_and_sends_inference_directly_upstream() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedUpstreamRequest>::new()));
        let mock = Router::new()
            .route("/v1/responses", post(mock_responses))
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let upstream_addr = listener.local_addr().expect("mock address");
        tokio::spawn(async move {
            axum::serve(listener, mock)
                .await
                .expect("serve mock upstream");
        });

        let data_dir = unique_test_data_dir("local-routes");
        let (state, providers, routes, shared_leases) = test_state(data_dir.clone()).await;
        let provider = providers
            .upsert_for_owner(
                None,
                CreateApiProviderRequest {
                    name: "Mock Provider".to_string(),
                    base_url: Some(format!("http://{upstream_addr}/v1")),
                    api_key: Some("sk-local-only".to_string()),
                    compatibility_profile: None,
                },
            )
            .await
            .expect("add local provider");
        routes
            .set_provider(Some(provider.id.clone()))
            .await
            .expect("select local provider");
        let router = build_router(state);

        let providers_response = router
            .clone()
            .oneshot(request(Method::GET, "/providers", Body::empty()))
            .await
            .expect("list local providers");
        assert_eq!(providers_response.status(), StatusCode::OK);

        let group_response = router
            .clone()
            .oneshot(request(Method::GET, "/groups", Body::empty()))
            .await
            .expect("group route response");
        assert_eq!(group_response.status(), StatusCode::NOT_FOUND);

        let response = router
            .clone()
            .oneshot(request(
                Method::POST,
                "/openai/v1/responses",
                Body::from(
                    json!({
                        "model": "mock-model",
                        "input": "local-direct-sentinel",
                        "stream": false
                    })
                    .to_string(),
                ),
            ))
            .await
            .expect("local inference response");
        let status = response.status();
        let response_body = response
            .into_body()
            .collect()
            .await
            .expect("collect response")
            .to_bytes();
        assert!(
            status.is_success(),
            "local inference failed ({status}): {}",
            String::from_utf8_lossy(&response_body)
        );

        {
            let requests = captured.lock().expect("capture lock");
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].authorization.as_deref(),
                Some("Bearer sk-local-only")
            );
            assert_eq!(
                requests[0].body.get("input").and_then(Value::as_str),
                Some("local-direct-sentinel")
            );
        }

        providers
            .upsert_shared_lease(
                "shared_center-provider",
                "Shared Provider",
                &format!("http://{upstream_addr}/v1"),
                "sk-shared",
                ProviderUpstreamProtocol::OpenAiResponses,
                ProviderCompatibilityProfile::GenericOpenAi,
            )
            .await
            .expect("persist shared provider");
        let rejected = router
            .clone()
            .oneshot(request(
                Method::PUT,
                "/selected-provider",
                Body::from(r#"{"provider_id":"shared_center-provider"}"#),
            ))
            .await
            .expect("unauthorized shared selection");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

        shared_leases
            .authorize("shared_center-provider", i64::MAX)
            .expect("authorize shared provider");
        let accepted = router
            .clone()
            .oneshot(request(
                Method::PUT,
                "/selected-provider",
                Body::from(r#"{"provider_id":"shared_center-provider"}"#),
            ))
            .await
            .expect("authorized shared selection");
        assert_eq!(accepted.status(), StatusCode::OK);

        shared_leases
            .authorize("shared_center-provider", 0)
            .expect("expire shared provider");
        let expired = router
            .oneshot(request(
                Method::POST,
                "/openai/v1/responses",
                Body::from(r#"{"model":"mock-model","input":"must-not-leave","stream":false}"#),
            ))
            .await
            .expect("expired shared response");
        assert_eq!(expired.status(), StatusCode::BAD_REQUEST);

        let _ = fs::remove_dir_all(data_dir);
    }

    async fn mock_responses(
        State(captured): State<Arc<Mutex<Vec<CapturedUpstreamRequest>>>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Json<Value> {
        let body = serde_json::from_slice(&body).expect("valid upstream JSON");
        captured
            .lock()
            .expect("capture lock")
            .push(CapturedUpstreamRequest {
                authorization: headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(ToString::to_string),
                body,
            });
        Json(json!({
            "id": "resp_mock",
            "object": "response",
            "created_at": 1,
            "status": "completed",
            "model": "mock-model",
            "output": [],
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "total_tokens": 2
            }
        }))
    }

    async fn test_state(
        data_dir: PathBuf,
    ) -> (AppState, ProviderStore, RouteStore, SharedLeaseStore) {
        let config = Arc::new(Config::for_test(data_dir));
        let accounts = AccountStore::new(config.clone()).expect("create accounts");
        accounts.load().await.expect("load accounts");
        let providers = ProviderStore::new(config.clone()).expect("create providers");
        providers.load().await.expect("load providers");
        let routes = RouteStore::new(config.clone()).expect("create routes");
        routes.load().await.expect("load routes");
        let shared_leases = SharedLeaseStore::default();
        let state = AppState {
            _client: Client::new(),
            _config: config.clone(),
            openai_tokens: OpenAiTokenService::new(),
            openai_device_login: OpenAiDeviceLoginService::new(),
            accounts,
            providers: providers.clone(),
            routes: routes.clone(),
            models: ModelStore::new(config.clone()).expect("create models"),
            settings: SettingsStore::new(config.clone()).expect("create settings"),
            turn_logs: TurnLogStore::new(config.clone()).expect("create turn logs"),
            issues: IssueStore::new(config.clone()).expect("create issues"),
            usage: UsageStore::new(config).expect("create usage"),
            upstream: UpstreamClient::new(),
            shared_leases: shared_leases.clone(),
        };
        (state, providers, routes, shared_leases)
    }

    fn request(method: Method, path: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(body)
            .expect("valid request")
    }

    fn unique_test_data_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}"))
    }
}
