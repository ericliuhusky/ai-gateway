use super::AppState;
use super::responses_handler::responses;
use crate::api::handlers::{
    add_provider, cancel_openai_device_login, delete_provider, get_provider_quota, get_route,
    healthz, import_openai_token, list_models, list_providers, poll_openai_device_login,
    refresh_openai_provider, set_route, start_openai_device_login,
};
use axum::{
    Router,
    extract::DefaultBodyLimit,
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
        .route("/route", get(get_route).put(set_route))
        .with_state(state);
    Router::new().nest("/management", management_routes)
}

fn gateway_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/responses", post(responses))
        .layer(DefaultBodyLimit::max(RESPONSES_REQUEST_BODY_LIMIT))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::{RESPONSES_REQUEST_BODY_LIMIT, build_router};
    use crate::{
        api::AppState,
        api::dto::CreateProviderReq,
        config::Config,
        openai::{OpenAiClient, OpenAiDeviceLoginService, OpenAiTokenService, build_http_client},
        store::{ProviderStore, RouteStore},
    };
    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::{DefaultBodyLimit, State},
        http::{HeaderMap, Method, Request, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
    };
    use http_body_util::BodyExt;
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
        accept: Option<String>,
        idempotency_key: Option<String>,
        cookie: Option<String>,
        raw_body: Bytes,
        body: Option<Value>,
    }

    #[tokio::test]
    async fn local_gateway_accepts_large_responses_requests_without_recording_an_issue() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedUpstreamRequest>::new()));
        let upstream_addr = start_mock_responses_server(captured.clone()).await;

        let data_dir = unique_test_data_dir("local-routes");
        let (state, providers, routes) = test_state(data_dir.clone()).await;
        let provider = providers
            .upsert(CreateProviderReq {
                name: "Mock Provider".to_string(),
                base_url: format!("http://{upstream_addr}/v1"),
                api_key: "sk-local-only".to_string(),
            })
            .await
            .expect("add local provider");
        routes
            .update(Some(provider.id().to_string()), None, None, true)
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

        let request_body = json!({
            "model": "mock-model",
            "input": "x".repeat(2 * 1024 * 1024),
            "stream": false
        })
        .to_string();
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("accept", "text/event-stream")
                    .header("authorization", "Bearer caller-token")
                    .header("cookie", "caller-session=secret")
                    .header("idempotency-key", "request-123")
                    .body(Body::from(request_body.clone()))
                    .expect("valid request"),
            )
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
            assert_eq!(requests[0].accept.as_deref(), Some("text/event-stream"));
            assert!(requests[0].idempotency_key.is_none());
            assert!(requests[0].cookie.is_none());
            assert_eq!(requests[0].raw_body, Bytes::from(request_body));
            assert_eq!(
                requests[0]
                    .body
                    .as_ref()
                    .expect("valid JSON body")
                    .get("input")
                    .and_then(Value::as_str)
                    .map(str::len),
                Some(2 * 1024 * 1024)
            );
        }
        let _ = fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn forwards_raw_bodies_without_route_overrides() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedUpstreamRequest>::new()));
        let upstream_addr = start_mock_responses_server(captured.clone()).await;
        let data_dir = unique_test_data_dir("raw-responses");
        let (state, providers, routes) = test_state(data_dir.clone()).await;
        let provider = providers
            .upsert(CreateProviderReq {
                name: "Mock Provider".to_string(),
                base_url: format!("http://{upstream_addr}/v1"),
                api_key: "sk-local-only".to_string(),
            })
            .await
            .expect("add local provider");
        routes
            .update(Some(provider.id().to_string()), None, None, true)
            .expect("select local provider");

        let raw_body = Bytes::from_static(b"this is intentionally not JSON");
        let response = build_router(state)
            .oneshot(request(
                Method::POST,
                "/v1/responses",
                Body::from(raw_body.clone()),
            ))
            .await
            .expect("gateway response");

        assert_eq!(response.status(), StatusCode::OK);
        let requests = captured.lock().expect("capture lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].raw_body, raw_body);
        assert!(requests[0].body.is_none());

        let _ = fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn applies_route_overrides_before_forwarding() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedUpstreamRequest>::new()));
        let upstream_addr = start_mock_responses_server(captured.clone()).await;
        let data_dir = unique_test_data_dir("route-overrides");
        let (state, providers, routes) = test_state(data_dir.clone()).await;
        let provider = providers
            .upsert(CreateProviderReq {
                name: "Mock Provider".to_string(),
                base_url: format!("http://{upstream_addr}/v1"),
                api_key: "sk-local-only".to_string(),
            })
            .await
            .expect("add local provider");
        routes
            .update(
                Some(provider.id().to_string()),
                Some("route-model".to_string()),
                Some("high".to_string()),
                false,
            )
            .expect("select route overrides");

        let response = build_router(state)
            .oneshot(request(
                Method::POST,
                "/v1/responses",
                Body::from(r#"{"model":"caller-model","input":"test"}"#),
            ))
            .await
            .expect("gateway response");

        assert_eq!(response.status(), StatusCode::OK);
        let requests = captured.lock().expect("capture lock");
        let body = requests[0].body.as_ref().expect("valid JSON body");
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some("route-model")
        );
        assert_eq!(
            body.get("reasoning")
                .and_then(Value::as_object)
                .and_then(|reasoning| reasoning.get("effort"))
                .and_then(Value::as_str),
            Some("high")
        );

        let _ = fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn transparently_returns_upstream_http_errors() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedUpstreamRequest>::new()));
        let upstream_addr = start_mock_responses_server(captured).await;
        let data_dir = unique_test_data_dir("upstream-http-error");
        let (state, providers, routes) = test_state(data_dir.clone()).await;
        let provider = providers
            .upsert(CreateProviderReq {
                name: "Mock Provider".to_string(),
                base_url: format!("http://{upstream_addr}/v1"),
                api_key: "sk-local-only".to_string(),
            })
            .await
            .expect("add local provider");
        routes
            .update(Some(provider.id().to_string()), None, None, true)
            .expect("select local provider");

        let response = build_router(state)
            .oneshot(request(
                Method::POST,
                "/v1/responses",
                Body::from(r#"{"input":"return-upstream-error"}"#),
            ))
            .await
            .expect("gateway response");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect response")
            .to_bytes();
        assert_eq!(
            body,
            Bytes::from_static(br#"{"error":{"message":"rate limited"}}"#)
        );

        let _ = fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn upstream_connection_failures_are_not_recorded() {
        let dead_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve local address");
        let upstream_addr = dead_listener.local_addr().expect("read local address");
        drop(dead_listener);

        let data_dir = unique_test_data_dir("connection-failure-issues");
        let (state, providers, routes) = test_state(data_dir.clone()).await;
        let provider = providers
            .upsert(CreateProviderReq {
                name: "Unavailable Provider".to_string(),
                base_url: format!("http://{upstream_addr}/v1"),
                api_key: "sk-local-only".to_string(),
            })
            .await
            .expect("add unavailable provider");
        routes
            .update(Some(provider.id().to_string()), None, None, true)
            .expect("select unavailable provider");
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
        let _ = fs::remove_dir_all(data_dir);
    }

    async fn mock_responses(
        State(captured): State<Arc<Mutex<Vec<CapturedUpstreamRequest>>>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let parsed_body = serde_json::from_slice(&body).ok();
        captured
            .lock()
            .expect("capture lock")
            .push(CapturedUpstreamRequest {
                authorization: headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(ToString::to_string),
                accept: headers
                    .get("accept")
                    .and_then(|value| value.to_str().ok())
                    .map(ToString::to_string),
                idempotency_key: headers
                    .get("idempotency-key")
                    .and_then(|value| value.to_str().ok())
                    .map(ToString::to_string),
                cookie: headers
                    .get("cookie")
                    .and_then(|value| value.to_str().ok())
                    .map(ToString::to_string),
                raw_body: body,
                body: parsed_body.clone(),
            });
        if parsed_body
            .as_ref()
            .and_then(|body| body.get("input"))
            .and_then(Value::as_str)
            == Some("return-upstream-error")
        {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": { "message": "rate limited" } })),
            )
                .into_response();
        }
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
        .into_response()
    }

    async fn start_mock_responses_server(
        captured: Arc<Mutex<Vec<CapturedUpstreamRequest>>>,
    ) -> std::net::SocketAddr {
        let mock = Router::new()
            .route("/v1/responses", post(mock_responses))
            .layer(DefaultBodyLimit::max(RESPONSES_REQUEST_BODY_LIMIT))
            .with_state(captured);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let upstream_addr = listener.local_addr().expect("mock address");
        tokio::spawn(async move {
            axum::serve(listener, mock)
                .await
                .expect("serve mock upstream");
        });
        upstream_addr
    }

    async fn test_state(data_dir: PathBuf) -> (AppState, ProviderStore, RouteStore) {
        let config = Arc::new(Config::for_test(data_dir.clone()));
        let providers = ProviderStore::new(config.clone()).expect("create providers");
        let routes = RouteStore::new(config.clone()).expect("create routes");
        let state = AppState {
            openai_tokens: OpenAiTokenService::new(),
            openai_device_login: OpenAiDeviceLoginService::new(),
            providers: providers.clone(),
            routes: routes.clone(),
            upstream: OpenAiClient::new(build_http_client()),
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
