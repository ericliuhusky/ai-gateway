use super::{
    AppState,
    handlers::{
        AppError, acquire_provider_for_use, build_passthrough_response, no_provider_selected_error,
        resolve_provider_by_id, selected_route,
    },
};
use crate::domain::{ProviderAuthMode, SelectedRoute};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::HeaderMap,
    response::Response,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

/// Forwards an OpenAI Responses request to the selected provider.
pub async fn responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let route = selected_route(&state).await?;
    let provider_id = route
        .provider_id
        .as_deref()
        .ok_or_else(no_provider_selected_error)?;
    let routed_provider = resolve_provider_by_id(&state, provider_id).await?;
    let has_overrides = route.selected_model.is_some() || route.selected_reasoning_effort.is_some();
    let request_body = if has_overrides {
        let mut request_json: Value = serde_json::from_slice(&body)
            .map_err(|err| AppError::bad_request(format!("无效的请求 JSON: {err}")))?;
        apply_route_overrides(&mut request_json, &route)?;
        Bytes::from(request_json.to_string())
    } else {
        body
    };

    let upstream_result = if routed_provider.auth_mode() == ProviderAuthMode::Account {
        let provider_record = acquire_provider_for_use(&state, routed_provider.id()).await?;
        let access_token = provider_record.access_token().ok_or_else(|| {
            AppError::bad_request(format!(
                "账户认证供应商 `{}` 缺少 access token",
                routed_provider.name()
            ))
        })?;
        state
            .upstream
            .account_responses_passthrough(access_token, request_body, &headers)
            .await
    } else {
        state
            .upstream
            .api_responses_passthrough(
                routed_provider.base_url().ok_or_else(|| {
                    AppError::bad_request(format!(
                        "供应商 `{}` 缺少 base_url",
                        routed_provider.name()
                    ))
                })?,
                routed_provider.api_key().ok_or_else(|| {
                    AppError::bad_request(format!(
                        "供应商 `{}` 缺少 api_key",
                        routed_provider.name()
                    ))
                })?,
                request_body,
                &headers,
            )
            .await
    };
    responses_passthrough_inner(upstream_result)
}

fn responses_passthrough_inner(
    upstream_result: Result<reqwest::Response, String>,
) -> Result<Response, AppError> {
    let upstream = upstream_result.map_err(AppError::upstream_message)?;
    let upstream_status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let output = upstream
        .bytes_stream()
        .map(|result| result.map_err(std::io::Error::other));

    build_passthrough_response(
        upstream_status,
        &upstream_headers,
        Body::from_stream(output),
    )
}

fn apply_route_overrides(request: &mut Value, route: &SelectedRoute) -> Result<(), AppError> {
    let request = request
        .as_object_mut()
        .ok_or_else(|| AppError::bad_request("请求 JSON 必须是对象"))?;

    if let Some(model) = route.selected_model.as_ref() {
        request.insert("model".to_string(), Value::String(model.clone()));
    }
    if let Some(effort) = route.selected_reasoning_effort.as_deref() {
        let reasoning = request
            .entry("reasoning".to_string())
            .or_insert_with(|| json!({}));
        if !reasoning.is_object() {
            *reasoning = json!({});
        }
        reasoning
            .as_object_mut()
            .expect("reasoning object was just initialized")
            .insert("effort".to_string(), Value::String(effort.to_string()));
    }
    Ok(())
}
