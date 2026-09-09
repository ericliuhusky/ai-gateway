use super::AppState;
use crate::api::handlers::{
    add_provider, cancel_openai_device_login, clear_gateway_issues, clear_selected_model,
    clear_selected_reasoning_effort, delete_provider, gateway_status,
    get_gateway_issue_repair_prompt, get_provider_quota, get_route, get_selected_model,
    get_selected_reasoning_effort, healthz, import_openai_token, list_gateway_issues, list_models,
    list_providers, poll_openai_device_login, refresh_openai_provider, responses, set_route,
    set_selected_model, set_selected_reasoning_effort, start_openai_device_login,
};
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};

// Codex Responses requests can include lengthy conversation history and tool
// definitions, which exceed Axum's 2 MiB default body limit.
const RESPONSES_REQUEST_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// HTTP routes exposed by the local Gateway.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(gateway_router(state.clone()))
        .merge(build_management_router(state))
}

/// Management routes exposed under the `/management` namespace.
pub fn build_management_router(state: AppState) -> Router {
    let management_routes = Router::new()
        .route("/healthz", get(healthz))
        .route("/control/status", get(gateway_status))
        .route("/providers/openai/import-token", post(import_openai_token))
        .route(
            "/providers/:provider_id/refresh",
            post(refresh_openai_provider),
        )
        .route(
            "/providers/openai/login/device",
            post(start_openai_device_login),
        )
        .route(
            "/providers/openai/login/device/:login_id",
            get(poll_openai_device_login).delete(cancel_openai_device_login),
        )
        .route("/providers", get(list_providers).post(add_provider))
        .route("/providers/:provider_id", delete(delete_provider))
        .route("/providers/:provider_id/quota", get(get_provider_quota))
        .route(
            "/gateway/issues",
            get(list_gateway_issues).delete(clear_gateway_issues),
        )
        .route(
            "/gateway/issues/:issue_id/repair-prompt",
            get(get_gateway_issue_repair_prompt),
        )
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
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            management_runtime_scope,
        ))
        .with_state(state);
    Router::new().nest("/management", management_routes)
}

fn gateway_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/responses", post(responses))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            gateway_runtime_scope,
        ))
        .layer(DefaultBodyLimit::max(RESPONSES_REQUEST_BODY_LIMIT))
        .with_state(state)
}

async fn management_runtime_scope(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let allowed_while_stopped =
        matches!(path, "/management/healthz" | "/management/control/status");
    if state.gateway_runtime.enabled() || allowed_while_stopped {
        next.run(request).await
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "AI Gateway 服务未启动").into_response()
    }
}

async fn gateway_runtime_scope(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.gateway_runtime.enabled() {
        next.run(request).await
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "AI Gateway 服务未启动").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{RESPONSES_REQUEST_BODY_LIMIT, build_router};
    use crate::{
        api::AppState,
        config::Config,
        models::CreateProviderRequest,
        openai::{OpenAiClient, build_http_client},
        openai_device_login::OpenAiDeviceLoginService,
        openai_tokens::OpenAiTokenService,
        store::{IssueStore, ModelStore, ProviderStore, RouteStore},
    };
    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::{DefaultBodyLimit, State},
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
    async fn local_gateway_accepts_large_responses_requests_without_recording_an_issue() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedUpstreamRequest>::new()));
        let mock = Router::new()
            .route("/v1/responses", post(mock_responses))
            .layer(DefaultBodyLimit::max(RESPONSES_REQUEST_BODY_LIMIT))
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
        let (state, providers, routes) = test_state(data_dir.clone()).await;
        let issues = state.issues.clone();
        let provider = providers
            .upsert(CreateProviderRequest {
                name: "Mock Provider".to_string(),
                base_url: Some(format!("http://{upstream_addr}/v1")),
                api_key: Some("sk-local-only".to_string()),
            })
            .await
            .expect("add local provider");
        routes
            .update(Some(provider.id.clone()), None, None, true)
            .await
            .expect("select local provider");
        let router = build_router(state);

        let externally_visible_management_response = router
            .clone()
            .oneshot(request(Method::GET, "/providers", Body::empty()))
            .await
            .expect("external management response");
        assert_eq!(
            externally_visible_management_response.status(),
            StatusCode::NOT_FOUND
        );

        let providers_response = router
            .clone()
            .oneshot(request(Method::GET, "/management/providers", Body::empty()))
            .await
            .expect("list providers over management HTTP route");
        assert_eq!(providers_response.status(), StatusCode::OK);

        let readiness_response = router
            .clone()
            .oneshot(request(
                Method::GET,
                "/management/control/status",
                Body::empty(),
            ))
            .await
            .expect("daemon readiness response over management HTTP route");
        assert_eq!(readiness_response.status(), StatusCode::OK);

        let response = router
            .clone()
            .oneshot(request(
                Method::POST,
                "/v1/responses",
                Body::from(
                    json!({
                        "model": "mock-model",
                        "input": "x".repeat(2 * 1024 * 1024),
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
                requests[0]
                    .body
                    .get("input")
                    .and_then(Value::as_str)
                    .map(str::len),
                Some(2 * 1024 * 1024)
            );
        }
        assert!(
            issues.list(50).expect("list gateway issues").is_empty(),
            "successful requests must not be recorded as gateway issues"
        );

        let _ = fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn records_upstream_connection_failures() {
        let dead_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve local address");
        let upstream_addr = dead_listener.local_addr().expect("read local address");
        drop(dead_listener);

        let data_dir = unique_test_data_dir("connection-failure-issues");
        let (state, providers, routes) = test_state(data_dir.clone()).await;
        let provider = providers
            .upsert(CreateProviderRequest {
                name: "Unavailable Provider".to_string(),
                base_url: Some(format!("http://{upstream_addr}/v1")),
                api_key: Some("sk-local-only".to_string()),
            })
            .await
            .expect("add unavailable provider");
        routes
            .update(Some(provider.id.clone()), None, None, true)
            .await
            .expect("select unavailable provider");
        let issues = state.issues.clone();
        let router = build_router(state);

        let response = router
            .oneshot(request(
                Method::POST,
                "/v1/responses",
                Body::from(
                    json!({
                        "model": "test-model",
                        "input": "connection-failure-sentinel",
                        "stream": false
                    })
                    .to_string(),
                ),
            ))
            .await
            .expect("gateway response");

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let recorded = issues.list(50).expect("list gateway issues");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].failure_kind, "upstream_connect_error");
        assert_eq!(recorded[0].provider_id, provider.id);
        assert!(recorded[0].status_code.is_none());
        assert!(recorded[0].upstream_response.is_empty());

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

    async fn test_state(data_dir: PathBuf) -> (AppState, ProviderStore, RouteStore) {
        let config = Arc::new(Config::for_test(data_dir.clone()));
        let providers = ProviderStore::new(config.clone()).expect("create providers");
        providers.load().await.expect("load providers");
        let routes = RouteStore::new(config.clone()).expect("create routes");
        routes.load().await.expect("load routes");
        let models = ModelStore::new(config.clone()).expect("create models");
        let state = AppState {
            _client: Client::new(),
            _config: config.clone(),
            openai_tokens: OpenAiTokenService::new(),
            openai_device_login: OpenAiDeviceLoginService::new(),
            providers: providers.clone(),
            routes: routes.clone(),
            models,
            issues: IssueStore::new(config.clone()).expect("create issues"),
            upstream: OpenAiClient::new(build_http_client()),
            gateway_runtime: crate::GatewayRuntime::new(true),
        };
        (state, providers, routes)
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
