use crate::{
    api::control::{
        ControlState, add_group_member, client_me, create_group, create_shared_connection,
        create_user, delete_group_member, delete_shared_connection, delete_user, get_group,
        healthz, issue_client_shared_provider_lease, list_client_shared_providers, list_groups,
        list_shared_connections, list_users, search_users, share_group_connection,
        unshare_group_connection, update_shared_connection,
    },
    auth::{self, require_auth},
};
use axum::{
    Router, middleware,
    routing::{delete, get, post},
};

pub fn build_router(state: ControlState) -> Router {
    let authenticated_control = Router::new()
        .route(
            "/shared-connections",
            get(list_shared_connections).post(create_shared_connection),
        )
        .route(
            "/shared-connections/:provider_id",
            axum::routing::put(update_shared_connection).delete(delete_shared_connection),
        )
        .route("/groups", get(list_groups).post(create_group))
        .route("/groups/:group_id", get(get_group))
        .route("/groups/:group_id/members", post(add_group_member))
        .route(
            "/groups/:group_id/members/:user_id",
            delete(delete_group_member),
        )
        .route("/groups/:group_id/providers", post(share_group_connection))
        .route(
            "/groups/:group_id/providers/:provider_id",
            delete(unshare_group_connection),
        )
        .route("/users/search", get(search_users))
        .route("/users", get(list_users).post(create_user))
        .route("/users/:user_id", delete(delete_user))
        .route_layer(middleware::from_fn_with_state(
            state.auth.clone(),
            auth::require_gateway_auth,
        ))
        .with_state(state.clone());

    let auth_routes = Router::new()
        .route("/auth/status", get(auth::auth_status))
        .route("/auth/feishu/authorize", get(auth::feishu_authorize))
        .route("/auth/feishu/callback", get(auth::feishu_callback))
        .route("/auth/login", post(auth::email_login))
        .route("/auth/me", get(auth::me))
        .route("/auth/logout", post(auth::logout))
        .with_state(state.auth.clone());

    let token_routes = Router::new()
        .route(
            "/auth/access-tokens",
            get(auth::gateway_access_token).post(auth::regenerate_gateway_access_token),
        )
        .route_layer(middleware::from_fn_with_state(
            state.auth.clone(),
            require_auth,
        ))
        .with_state(state.auth.clone());

    // These endpoints are the only API called by a local data-plane gateway.
    // They never receive prompt or response bodies.
    let local_client_routes = Router::new()
        .route("/client/v1/me", get(client_me))
        .route(
            "/client/v1/shared-providers",
            get(list_client_shared_providers),
        )
        .route(
            "/client/v1/shared-providers/:provider_id/lease",
            post(issue_client_shared_provider_lease),
        )
        .route_layer(middleware::from_fn_with_state(
            state.auth.clone(),
            auth::require_gateway_auth,
        ))
        .with_state(state);

    Router::new()
        .route("/healthz", get(healthz))
        .merge(auth_routes)
        .merge(token_routes)
        .merge(authenticated_control)
        .merge(local_client_routes)
}

#[cfg(test)]
mod tests {
    use super::build_router;
    use crate::{
        api::ControlState,
        auth::AuthService,
        config::Config,
        store::{GroupStore, ProviderStore},
    };
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
        response::Response,
    };
    use serde_json::{Value, json};
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn center_is_headless_and_has_no_data_plane_routes() {
        let data_dir = unique_test_data_dir();
        let config = Arc::new(Config::for_test(data_dir.clone()));
        let auth = AuthService::new(config.clone()).expect("create auth");
        auth.initialize().expect("initialize auth");
        let groups = GroupStore::new(config.clone()).expect("create groups");
        let providers = ProviderStore::new(config).expect("create providers");
        providers.load().await.expect("load providers");
        let router = build_router(ControlState {
            auth,
            groups,
            providers,
        });

        let health = router
            .clone()
            .oneshot(request(Method::GET, "/healthz", Body::empty()))
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);

        let users = router
            .clone()
            .oneshot(request(Method::GET, "/users", Body::empty()))
            .await
            .expect("account control response");
        assert_eq!(users.status(), StatusCode::UNAUTHORIZED);

        let auth_status = router
            .clone()
            .oneshot(request(Method::GET, "/auth/status", Body::empty()))
            .await
            .expect("auth status response");
        assert_eq!(auth_status.status(), StatusCode::OK);
        assert_eq!(
            response_json(auth_status).await["mode"],
            Value::String("required".to_string())
        );

        for (method, path, body) in [
            (Method::GET, "/", Body::empty()),
            (Method::GET, "/index.html", Body::empty()),
            (Method::GET, "/providers", Body::empty()),
            (Method::GET, "/openai/v1/models", Body::empty()),
            (
                Method::POST,
                "/openai/v1/responses",
                Body::from(r#"{"input":"must-never-reach-center"}"#),
            ),
        ] {
            let response = router
                .clone()
                .oneshot(request(method, path, body))
                .await
                .expect("route response");
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "center unexpectedly exposed {path}"
            );
        }

        let _ = fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn center_issues_and_revokes_shared_provider_leases() {
        let data_dir = unique_test_data_dir();
        let config = Arc::new(Config::for_test(data_dir.clone()));
        let auth = AuthService::new(config.clone()).expect("create auth");
        auth.initialize().expect("initialize auth");
        auth.create_user("admin@example.com", "Admin", "admin", "admin-secret")
            .expect("create admin");
        let groups = GroupStore::new(config.clone()).expect("create groups");
        let providers = ProviderStore::new(config).expect("create providers");
        providers.load().await.expect("load providers");
        let router = build_router(ControlState {
            auth,
            groups,
            providers,
        });

        let unauthenticated = router
            .clone()
            .oneshot(request(Method::GET, "/groups", Body::empty()))
            .await
            .expect("unauthenticated group response");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let admin_cookie = login(&router, "admin@example.com", "admin-secret").await;
        let create_member = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                "/users",
                json!({
                    "email": "member@example.com",
                    "name": "Member",
                    "role": "user",
                    "password": "member-secret"
                }),
                Some(&admin_cookie),
                None,
            ))
            .await
            .expect("create member response");
        assert_eq!(create_member.status(), StatusCode::OK);
        let member_id = response_json(create_member).await["user"]["id"]
            .as_i64()
            .expect("member id");

        let create_provider = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                "/shared-connections",
                json!({
                    "name": "Shared Mock",
                    "base_url": "https://supplier.example/v1",
                    "api_key": "sk-center-only",
                    "compatibility_profile": "official_openai"
                }),
                Some(&admin_cookie),
                None,
            ))
            .await
            .expect("create shared connection response");
        assert_eq!(create_provider.status(), StatusCode::OK);
        let provider_id = response_json(create_provider).await["provider_id"]
            .as_str()
            .expect("provider id")
            .to_string();

        let update_provider = router
            .clone()
            .oneshot(authenticated_request(
                Method::PUT,
                &format!("/shared-connections/{provider_id}"),
                json!({
                    "name": "Shared Mock",
                    "base_url": "https://supplier.example/v1",
                    "api_key": "sk-center-updated",
                    "compatibility_profile": "official_openai"
                }),
                Some(&admin_cookie),
                None,
            ))
            .await
            .expect("update shared connection response");
        assert_eq!(update_provider.status(), StatusCode::OK);
        assert_eq!(
            response_json(update_provider).await["provider_id"],
            provider_id
        );

        let create_group = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                "/groups",
                json!({ "name": "Team" }),
                Some(&admin_cookie),
                None,
            ))
            .await
            .expect("create group response");
        assert_eq!(create_group.status(), StatusCode::OK);
        let group_id = response_json(create_group).await["id"]
            .as_i64()
            .expect("group id");

        let add_member = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                &format!("/groups/{group_id}/members"),
                json!({ "user_id": member_id }),
                Some(&admin_cookie),
                None,
            ))
            .await
            .expect("add member response");
        assert_eq!(add_member.status(), StatusCode::OK);

        let share_provider = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                &format!("/groups/{group_id}/providers"),
                json!({ "provider_id": provider_id }),
                Some(&admin_cookie),
                None,
            ))
            .await
            .expect("share provider response");
        assert_eq!(share_provider.status(), StatusCode::OK);

        let member_cookie = login(&router, "member@example.com", "member-secret").await;
        let access_token_response = router
            .clone()
            .oneshot(authenticated_request(
                Method::GET,
                "/auth/access-tokens",
                Value::Null,
                Some(&member_cookie),
                None,
            ))
            .await
            .expect("access token response");
        assert_eq!(access_token_response.status(), StatusCode::OK);
        let access_token = response_json(access_token_response).await["access_token"]
            .as_str()
            .expect("access token")
            .to_string();

        let desktop_profile = router
            .clone()
            .oneshot(authenticated_request(
                Method::GET,
                "/client/v1/me",
                Value::Null,
                None,
                Some(&access_token),
            ))
            .await
            .expect("desktop profile response");
        assert_eq!(desktop_profile.status(), StatusCode::OK);
        assert_eq!(
            response_json(desktop_profile).await["user"]["name"],
            "Member"
        );

        let groups_with_token = router
            .clone()
            .oneshot(authenticated_request(
                Method::GET,
                "/groups",
                Value::Null,
                None,
                Some(&access_token),
            ))
            .await
            .expect("desktop groups response");
        assert_eq!(groups_with_token.status(), StatusCode::OK);
        assert_eq!(
            response_json(groups_with_token).await["groups"][0]["id"],
            group_id
        );

        let visible = router
            .clone()
            .oneshot(authenticated_request(
                Method::GET,
                "/client/v1/shared-providers",
                Value::Null,
                None,
                Some(&access_token),
            ))
            .await
            .expect("visible shared providers response");
        assert_eq!(visible.status(), StatusCode::OK);
        let visible_json = response_json(visible).await;
        assert_eq!(visible_json["providers"][0]["id"], provider_id);
        assert!(
            !visible_json.to_string().contains("sk-center-updated"),
            "shared-provider listing must not expose credentials"
        );

        let lease = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                &format!("/client/v1/shared-providers/{provider_id}/lease"),
                json!({ "device_id": "member-desktop" }),
                None,
                Some(&access_token),
            ))
            .await
            .expect("lease response");
        assert_eq!(lease.status(), StatusCode::OK);
        let lease_json = response_json(lease).await;
        assert_eq!(lease_json["provider_id"], provider_id);
        assert_eq!(lease_json["api_key"], "sk-center-updated");
        assert_eq!(lease_json["upstream_protocol"], "openai_responses");
        assert_eq!(lease_json["compatibility_profile"], "official_openai");

        let remove_member = router
            .clone()
            .oneshot(authenticated_request(
                Method::DELETE,
                &format!("/groups/{group_id}/members/{member_id}"),
                Value::Null,
                Some(&admin_cookie),
                None,
            ))
            .await
            .expect("remove member response");
        assert_eq!(remove_member.status(), StatusCode::OK);

        let visible_after_revoke = router
            .clone()
            .oneshot(authenticated_request(
                Method::GET,
                "/client/v1/shared-providers",
                Value::Null,
                None,
                Some(&access_token),
            ))
            .await
            .expect("visible providers after revoke");
        assert_eq!(visible_after_revoke.status(), StatusCode::OK);
        assert_eq!(
            response_json(visible_after_revoke).await["providers"]
                .as_array()
                .expect("providers array")
                .len(),
            0
        );

        let rejected_lease = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                &format!("/client/v1/shared-providers/{provider_id}/lease"),
                json!({ "device_id": "member-desktop" }),
                None,
                Some(&access_token),
            ))
            .await
            .expect("rejected lease response");
        assert_eq!(rejected_lease.status(), StatusCode::NOT_FOUND);

        let _ = fs::remove_dir_all(data_dir);
    }

    fn request(method: Method, path: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(body)
            .expect("valid request")
    }

    fn authenticated_request(
        method: Method,
        path: &str,
        body: Value,
        cookie: Option<&str>,
        bearer: Option<&str>,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if let Some(token) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        builder.body(body).expect("valid authenticated request")
    }

    async fn login(router: &Router, email: &str, password: &str) -> String {
        let response = router
            .clone()
            .oneshot(authenticated_request(
                Method::POST,
                "/auth/login",
                json!({ "email": email, "password": password }),
                None,
                None,
            ))
            .await
            .expect("login response");
        assert_eq!(response.status(), StatusCode::OK);
        response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .expect("session cookie")
            .to_string()
    }

    async fn response_json(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("collect response body");
        serde_json::from_slice(&bytes).expect("JSON response")
    }

    fn unique_test_data_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_control_routes_{unique}"))
    }
}
