use crate::{
    config::{Config, DEFAULT_CODEX_CLIENT_VERSION},
    models::{
        CreateProviderRequest, ProviderAuthMode, ProviderRecord, ProviderSummary, SelectedRoute,
        UpdateSelectedModelRequest, UpdateSelectedProviderRequest,
        UpdateSelectedReasoningEffortRequest,
    },
    openai::OpenAiClient,
    openai_device_login::{
        DeviceLoginCompletion, DeviceLoginPoll, DeviceLoginStart, OpenAiDeviceLoginService,
    },
    openai_tokens::OpenAiTokenService,
    store::{ProviderStore, RouteStore},
};
use axum::{
    body::{Body, Bytes},
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderName, StatusCode},
    response::{IntoResponse, Json, Response},
};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

const GATEWAY_ERROR_PREFIX: &str = "AI网关错误：";
const UPSTREAM_ERROR_PREFIX: &str = "上游服务错误：";

#[derive(Clone)]
pub struct AppState {
    pub _client: Client,
    pub _config: Arc<Config>,
    pub openai_tokens: OpenAiTokenService,
    pub openai_device_login: OpenAiDeviceLoginService,
    pub providers: ProviderStore,
    pub routes: RouteStore,
    pub upstream: OpenAiClient,
}

#[derive(Debug, Deserialize)]
pub struct ListModelsQuery {
    #[serde(default)]
    pub provider_id: Option<String>,
}

pub async fn healthz() -> &'static str {
    "ok"
}

/// Local daemon readiness probe exposed under the HTTP management namespace.
///
/// It remains available even when the Gateway data plane is stopped.
pub async fn gateway_status() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAuthTokensFile {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    #[serde(alias = "account_id")]
    upstream_account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexAuthFile {
    #[serde(default)]
    tokens: Option<CodexAuthTokensFile>,
}

fn import_tokens_from_value(value: Value) -> Result<Vec<CodexAuthTokensFile>, String> {
    let entries = match value {
        Value::Array(entries) => {
            if entries.is_empty() {
                return Err("导入 JSON 不包含任何账号".to_string());
            }
            entries
        }
        entry @ Value::Object(_) => vec![entry],
        _ => {
            return Err(
                "导入内容必须是 Codex auth.json，或 Cockpit Tools 导出的账号对象/数组".to_string(),
            );
        }
    };

    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let label = if index == 0 {
                "导入 JSON"
            } else {
                "导入 JSON 账号"
            };
            let object = entry
                .as_object()
                .ok_or_else(|| format!("{label} 必须是对象"))?;

            if object.contains_key("tokens") {
                let auth_file = serde_json::from_value::<CodexAuthFile>(entry)
                    .map_err(|error| format!("{label} 格式无效: {error}"))?;
                return auth_file
                    .tokens
                    .ok_or_else(|| format!("{label} 缺少 `tokens`"));
            }

            // Cockpit Tools exports portable Codex accounts as flat objects. A single
            // export is still wrapped in an array, and includes fields such as
            // `type`, `email`, `last_refresh`, and `expired` in addition to tokens.
            serde_json::from_value::<CodexAuthTokensFile>(entry).map_err(|error| {
                format!("{label} 不是有效的 Codex 或 Cockpit Tools Token: {error}")
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct ImportOpenAiFromLocalResponse {
    imported: bool,
    imported_count: usize,
    email: String,
    provider_id: String,
}

#[derive(Debug, Serialize)]
pub struct RefreshOpenAiProviderResponse {
    provider_id: String,
    email: String,
    expiry_timestamp: i64,
}

#[derive(Debug, Serialize)]
pub struct OpenAiDeviceLoginStartResponse {
    login_id: String,
    user_code: String,
    verification_uri: String,
    interval_seconds: u64,
    expires_in: i64,
}

#[derive(Debug, Serialize)]
pub struct OpenAiDeviceLoginStatusResponse {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interval_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Import OpenAI accounts from a pasted Codex `auth.json` or Cockpit Tools export.
pub async fn import_openai_token(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Json<ImportOpenAiFromLocalResponse>, AppError> {
    let tokens = import_tokens_from_value(payload).map_err(AppError::bad_request)?;
    let imported_count = tokens.len();
    let mut first_imported = None;

    for (index, tokens) in tokens.into_iter().enumerate() {
        let refresh_token = tokens
            .refresh_token
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| {
                AppError::bad_request(format!(
                    "第 {} 个账号缺少 `refresh_token`，无法导入可自动刷新的 OpenAI 账号",
                    index + 1
                ))
            })?;
        let imported = state
            .openai_tokens
            .import_codex_tokens(
                tokens.access_token,
                refresh_token,
                tokens.upstream_account_id,
            )
            .map_err(AppError::bad_request)?;
        let provider = state
            .providers
            .import_openai_provider(imported)
            .await
            .map_err(AppError::bad_request)?;

        if first_imported.is_none() {
            first_imported = Some((provider.email.clone().unwrap_or_default(), provider.id));
        }
    }

    let (email, provider_id) =
        first_imported.ok_or_else(|| AppError::bad_request("导入 JSON 不包含任何账号"))?;

    Ok(Json(ImportOpenAiFromLocalResponse {
        imported: true,
        imported_count,
        email,
        provider_id,
    }))
}

pub async fn refresh_openai_provider(
    State(state): State<AppState>,
    AxumPath(provider_id): AxumPath<String>,
) -> Result<Json<RefreshOpenAiProviderResponse>, AppError> {
    let provider = state
        .providers
        .refresh(&state.openai_tokens, &provider_id)
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(RefreshOpenAiProviderResponse {
        provider_id: provider.id,
        email: provider.email.unwrap_or_default(),
        expiry_timestamp: provider.expiry_timestamp.unwrap_or_default(),
    }))
}

/// Starts the official OpenAI device authorization flow used by Codex.
pub async fn start_openai_device_login(
    State(state): State<AppState>,
) -> Result<Json<OpenAiDeviceLoginStartResponse>, AppError> {
    let start = state
        .openai_device_login
        .start()
        .await
        .map_err(AppError::upstream_message)?;
    Ok(Json(device_login_start_response(start)))
}

/// Polls a device authorization session and persists the account when OpenAI approves it.
pub async fn poll_openai_device_login(
    State(state): State<AppState>,
    AxumPath(login_id): AxumPath<String>,
) -> Result<Json<OpenAiDeviceLoginStatusResponse>, AppError> {
    let poll = state
        .openai_device_login
        .poll(&login_id)
        .await
        .map_err(AppError::bad_request)?;

    match poll {
        DeviceLoginPoll::Pending(start) => Ok(Json(device_login_pending_response(start))),
        DeviceLoginPoll::Finalizing => Ok(Json(device_login_finalizing_response())),
        DeviceLoginPoll::Completed(completion) => {
            Ok(Json(device_login_completed_response(completion)))
        }
        DeviceLoginPoll::Failed(error) => Ok(Json(device_login_failed_response(error))),
        DeviceLoginPoll::Ready => {
            let authorization = match state
                .openai_device_login
                .begin_finalization(&login_id)
                .await
                .map_err(AppError::bad_request)?
            {
                Some(authorization) => authorization,
                None => return Ok(Json(device_login_finalizing_response())),
            };

            let completion = async {
                let imported = state
                    .openai_device_login
                    .exchange_authorization(&authorization, &state.openai_tokens)
                    .await
                    .map_err(AppError::upstream_message)?;
                let provider = state
                    .providers
                    .import_openai_provider(imported)
                    .await
                    .map_err(AppError::bad_request)?;
                Ok::<_, AppError>(DeviceLoginCompletion {
                    email: provider.email.clone().unwrap_or_default(),
                    provider_id: provider.id,
                })
            }
            .await;

            match completion {
                Ok(completion) => {
                    state
                        .openai_device_login
                        .complete(&login_id, completion.clone())
                        .await;
                    Ok(Json(device_login_completed_response(completion)))
                }
                Err(error) => {
                    let message = error.message;
                    state
                        .openai_device_login
                        .fail(&login_id, message.clone())
                        .await;
                    Ok(Json(device_login_failed_response(message)))
                }
            }
        }
    }
}

pub async fn cancel_openai_device_login(
    State(state): State<AppState>,
    AxumPath(login_id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    state
        .openai_device_login
        .cancel(&login_id)
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(json!({ "cancelled": true })))
}

fn device_login_start_response(start: DeviceLoginStart) -> OpenAiDeviceLoginStartResponse {
    OpenAiDeviceLoginStartResponse {
        login_id: start.login_id,
        user_code: start.user_code,
        verification_uri: start.verification_uri,
        interval_seconds: start.interval_seconds,
        expires_in: start.expires_in,
    }
}

fn device_login_pending_response(start: DeviceLoginStart) -> OpenAiDeviceLoginStatusResponse {
    OpenAiDeviceLoginStatusResponse {
        status: "pending".to_string(),
        login_id: Some(start.login_id),
        user_code: Some(start.user_code),
        verification_uri: Some(start.verification_uri),
        interval_seconds: Some(start.interval_seconds),
        expires_in: Some(start.expires_in),
        email: None,
        provider_id: None,
        error: None,
    }
}

fn device_login_finalizing_response() -> OpenAiDeviceLoginStatusResponse {
    OpenAiDeviceLoginStatusResponse {
        status: "finalizing".to_string(),
        login_id: None,
        user_code: None,
        verification_uri: None,
        interval_seconds: None,
        expires_in: None,
        email: None,
        provider_id: None,
        error: None,
    }
}

fn device_login_completed_response(
    completion: DeviceLoginCompletion,
) -> OpenAiDeviceLoginStatusResponse {
    OpenAiDeviceLoginStatusResponse {
        status: "completed".to_string(),
        login_id: None,
        user_code: None,
        verification_uri: None,
        interval_seconds: None,
        expires_in: None,
        email: Some(completion.email),
        provider_id: Some(completion.provider_id),
        error: None,
    }
}

fn device_login_failed_response(error: String) -> OpenAiDeviceLoginStatusResponse {
    OpenAiDeviceLoginStatusResponse {
        status: "failed".to_string(),
        login_id: None,
        user_code: None,
        verification_uri: None,
        interval_seconds: None,
        expires_in: None,
        email: None,
        provider_id: None,
        error: Some(format!("{UPSTREAM_ERROR_PREFIX}{error}")),
    }
}

pub async fn list_providers(State(state): State<AppState>) -> Json<Value> {
    let providers = hydrated_provider_summaries(&state).await;
    Json(json!({ "providers": providers }))
}

pub async fn get_provider_quota(
    State(state): State<AppState>,
    AxumPath(provider_id): AxumPath<String>,
) -> Result<Response, AppError> {
    let provider = resolve_provider_by_id(&state, &provider_id).await?;
    if provider.auth_mode != ProviderAuthMode::Account {
        return Err(AppError::bad_request(format!(
            "供应商 `{}` 不支持账户额度查询",
            provider.name
        )));
    }

    let provider_record = resolve_provider_record_for_use(&state, &provider).await?;
    let access_token = provider_record.access_token().ok_or_else(|| {
        AppError::bad_request(format!(
            "账户认证供应商 `{}` 缺少 access token",
            provider.name
        ))
    })?;
    let upstream = state
        .upstream
        .account_usage(access_token)
        .await
        .map_err(AppError::upstream_message)?;
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let body = upstream.bytes().await.map_err(AppError::upstream)?;

    build_passthrough_response(status, &headers, Body::from(body))
}

pub async fn list_models(
    State(state): State<AppState>,
    Query(query): Query<ListModelsQuery>,
) -> Result<Response, AppError> {
    let provider = match query.provider_id.as_deref().map(str::trim) {
        Some(provider_id) if !provider_id.is_empty() => {
            resolve_provider_by_id(&state, provider_id).await?
        }
        _ => resolve_selected_provider(&state).await?,
    };
    let upstream = fetch_provider_models(&state, &provider).await?;
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let body = upstream.bytes().await.map_err(AppError::upstream)?;
    build_passthrough_response(status, &headers, Body::from(body))
}

pub async fn add_provider(
    State(state): State<AppState>,
    Json(request): Json<CreateProviderRequest>,
) -> Result<Json<Value>, AppError> {
    let provider = state
        .providers
        .upsert(request)
        .await
        .map_err(AppError::bad_request)?;

    Ok(Json(json!({
        "provider": {
            "id": provider.id,
            "name": provider.name(),
            "auth_mode": provider.auth_mode,
            "base_url": provider.base_url,
            "api_key": provider.api_key,
        }
    })))
}

pub async fn delete_provider(
    State(state): State<AppState>,
    AxumPath(provider_id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let _provider = state
        .providers
        .find_by_id(&provider_id)
        .await
        .ok_or_else(|| AppError::bad_request(format!("未知的 provider_id: {provider_id}")))?;
    let deleted = state
        .providers
        .delete(&provider_id)
        .await
        .map_err(AppError::bad_request)?;

    let route = selected_route(&state).await?;
    if route.provider_id.as_deref() == Some(provider_id.as_str()) {
        let _ = update_route(&state, None, None, None, false).await?;
    }

    Ok(Json(json!({
        "deleted_provider": {
            "id": deleted.id,
            "name": deleted.name(),
        }
    })))
}

pub async fn get_route(State(state): State<AppState>) -> Json<Value> {
    let route = selected_route(&state).await.unwrap_or_default();
    Json(json!({ "selected_provider": route_payload(route) }))
}

pub async fn set_route(
    State(state): State<AppState>,
    Json(request): Json<UpdateSelectedProviderRequest>,
) -> Result<Json<Value>, AppError> {
    let provider_id = normalize_selected_provider_id(request.provider_id)?;
    let _provider = resolve_provider_by_id(&state, &provider_id).await?;
    let route = update_route(&state, Some(provider_id), None, None, true).await?;
    Ok(Json(json!({
        "selected_provider": route_payload(route),
    })))
}

pub async fn get_selected_model(State(state): State<AppState>) -> Json<Value> {
    let route = selected_route(&state).await.unwrap_or_default();
    Json(json!({ "selected_model": route_payload(route) }))
}

pub async fn set_selected_model(
    State(state): State<AppState>,
    Json(request): Json<UpdateSelectedModelRequest>,
) -> Result<Json<Value>, AppError> {
    let model = normalize_selected_model(request.model)?;
    let existing = selected_route(&state).await?;
    let route = update_route(
        &state,
        existing.provider_id,
        Some(model),
        existing.selected_reasoning_effort,
        false,
    )
    .await?;
    Ok(Json(json!({ "selected_model": route_payload(route) })))
}

pub async fn clear_selected_model(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let existing = selected_route(&state).await?;
    let route = update_route(
        &state,
        existing.provider_id,
        None,
        existing.selected_reasoning_effort,
        false,
    )
    .await?;
    Ok(Json(json!({ "selected_model": route_payload(route) })))
}

pub async fn get_selected_reasoning_effort(State(state): State<AppState>) -> Json<Value> {
    let route = selected_route(&state).await.unwrap_or_default();
    Json(json!({
        "selected_reasoning_effort": route_payload(route)
    }))
}

pub async fn set_selected_reasoning_effort(
    State(state): State<AppState>,
    Json(request): Json<UpdateSelectedReasoningEffortRequest>,
) -> Result<Json<Value>, AppError> {
    resolve_selected_provider(&state).await?;
    let effort = normalize_selected_reasoning_effort(request.effort)?;
    let existing = selected_route(&state).await?;
    let route = update_route(
        &state,
        existing.provider_id,
        existing.selected_model,
        Some(effort),
        false,
    )
    .await?;
    Ok(Json(json!({
        "selected_reasoning_effort": route_payload(route)
    })))
}

pub async fn clear_selected_reasoning_effort(
    State(state): State<AppState>,
) -> Result<Json<Value>, AppError> {
    let existing = selected_route(&state).await?;
    let route = update_route(
        &state,
        existing.provider_id,
        existing.selected_model,
        None,
        false,
    )
    .await?;
    Ok(Json(json!({
        "selected_reasoning_effort": route_payload(route)
    })))
}

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
        apply_responses_route_overrides(&mut request_json, &route)?;
        Bytes::from(request_json.to_string())
    } else {
        body
    };

    let upstream_result = if routed_provider.auth_mode == ProviderAuthMode::Account {
        if !provider_uses_openai_account(&routed_provider) {
            return Err(AppError::bad_request(format!(
                "账户认证供应商 `{}` 暂不支持",
                routed_provider.name
            )));
        }
        let provider_record = resolve_provider_record_for_use(&state, &routed_provider).await?;
        let access_token = provider_record.access_token().ok_or_else(|| {
            AppError::bad_request(format!(
                "账户认证供应商 `{}` 缺少 access token",
                routed_provider.name
            ))
        })?;
        state
            .upstream
            .account_responses_passthrough(access_token, request_body, &headers)
            .await
    } else {
        let native_provider = routed_provider.record.as_ref().ok_or_else(|| {
            AppError::bad_request(format!("未知供应商: {}", routed_provider.name))
        })?;
        state
            .upstream
            .api_responses_passthrough(
                native_provider.base_url.as_str(),
                native_provider.api_key.as_str(),
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

async fn resolve_selected_provider(state: &AppState) -> Result<ResolvedProvider, AppError> {
    let route = selected_route(state).await?;
    if let Some(provider_id) = route.provider_id {
        return resolve_provider_by_id(state, &provider_id).await;
    }
    Err(no_provider_selected_error())
}

fn no_provider_selected_error() -> AppError {
    AppError::bad_request("尚未选择供应商；请先调用 PUT /selected-provider")
}

async fn selected_route(state: &AppState) -> Result<SelectedRoute, AppError> {
    Ok(state.routes.get().await)
}

fn route_payload(route: SelectedRoute) -> Value {
    json!({
        "provider_id": route.provider_id,
        "selected_model": route.selected_model,
        "selected_reasoning_effort": route.selected_reasoning_effort,
        "updated_at": route.updated_at,
    })
}

fn apply_responses_route_overrides(
    request: &mut Value,
    route: &SelectedRoute,
) -> Result<(), AppError> {
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

async fn update_route(
    state: &AppState,
    provider_id: Option<String>,
    selected_model: Option<String>,
    selected_reasoning_effort: Option<String>,
    load_provider_preferences: bool,
) -> Result<SelectedRoute, AppError> {
    state
        .routes
        .update(
            provider_id,
            selected_model,
            selected_reasoning_effort,
            load_provider_preferences,
        )
        .await
        .map_err(AppError::internal)
}

fn should_skip_passthrough_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str().to_ascii_lowercase().as_str(),
        "connection"
            | "content-length"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn build_passthrough_response(
    status: StatusCode,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, AppError> {
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        if should_skip_passthrough_header(name) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder
        .body(body)
        .map_err(|err| AppError::internal(err.to_string()))
}

async fn fetch_provider_models(
    state: &AppState,
    provider: &ResolvedProvider,
) -> Result<reqwest::Response, AppError> {
    if provider.auth_mode == ProviderAuthMode::Account {
        let provider_record = resolve_provider_record_for_use(state, provider).await?;
        let access_token = provider_record.access_token().ok_or_else(|| {
            AppError::bad_request(format!(
                "账户认证供应商 `{}` 缺少 access token",
                provider.name
            ))
        })?;
        let client_version = DEFAULT_CODEX_CLIENT_VERSION;
        let upstream = state
            .upstream
            .account_models(access_token, Some(client_version))
            .await
            .map_err(AppError::upstream_message)?;
        return Ok(upstream);
    }

    let native_provider = provider
        .record
        .as_ref()
        .ok_or_else(|| AppError::bad_request(format!("未知供应商: {}", provider.name)))?;
    let upstream = state
        .upstream
        .api_models(
            native_provider.base_url.as_str(),
            native_provider.api_key.as_str(),
        )
        .await
        .map_err(AppError::upstream_message)?;
    Ok(upstream)
}

fn normalize_selected_provider_id(provider_id: Option<String>) -> Result<String, AppError> {
    let provider_id = provider_id.ok_or_else(|| AppError::bad_request("必须提供 provider_id"))?;
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("provider_id 不能为空"));
    }
    Ok(trimmed.to_string())
}

fn normalize_selected_model(model: String) -> Result<String, AppError> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("模型不能为空"));
    }
    Ok(trimmed.to_string())
}

fn normalize_selected_reasoning_effort(effort: String) -> Result<String, AppError> {
    let effort = effort.trim();
    if matches!(effort, "low" | "medium" | "high" | "xhigh") {
        return Ok(effort.to_string());
    }
    Err(AppError::bad_request(
        "推理强度必须是以下值之一：low、medium、high、xhigh",
    ))
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedProvider {
    pub(super) name: String,
    pub(super) auth_mode: ProviderAuthMode,
    pub(super) record: Option<ProviderRecord>,
}

async fn resolve_provider_by_id(
    state: &AppState,
    provider_id: &str,
) -> Result<ResolvedProvider, AppError> {
    let record = state
        .providers
        .find_by_id(provider_id)
        .await
        .ok_or_else(|| AppError::bad_request(format!("未知的 provider_id: {provider_id}")))?;
    Ok(resolved_provider_from_record(record))
}

fn resolved_provider_from_record(record: ProviderRecord) -> ResolvedProvider {
    ResolvedProvider {
        name: record.name().to_string(),
        auth_mode: record.auth_mode.clone(),
        record: Some(record),
    }
}

async fn resolve_provider_record_for_use(
    state: &AppState,
    provider: &ResolvedProvider,
) -> Result<ProviderRecord, AppError> {
    let provider_id = provider
        .record
        .as_ref()
        .map(|record| record.id.as_str())
        .ok_or_else(|| AppError::bad_request(format!("未知供应商: {}", provider.name)))?;
    state
        .providers
        .acquire_by_id(&state.openai_tokens, provider_id)
        .await
        .map_err(AppError::bad_request)
}

pub(super) fn provider_uses_openai_account(provider: &ResolvedProvider) -> bool {
    provider
        .record
        .as_ref()
        .map(|record| record.auth_mode == ProviderAuthMode::Account)
        .unwrap_or(false)
}

async fn hydrated_provider_summaries(state: &AppState) -> Vec<ProviderSummary> {
    state.providers.list().await
}

#[derive(Debug)]
pub struct AppError {
    pub(super) status: StatusCode,
    pub(super) message: String,
    source: AppErrorSource,
}

#[derive(Debug, Clone, Copy)]
enum AppErrorSource {
    Gateway,
    Upstream,
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            source: AppErrorSource::Gateway,
        }
    }

    fn upstream(error: reqwest::Error) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            source: AppErrorSource::Upstream,
        }
    }

    fn upstream_message(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
            source: AppErrorSource::Upstream,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            source: AppErrorSource::Gateway,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "message": match self.source {
                        AppErrorSource::Gateway => format!("{GATEWAY_ERROR_PREFIX}{}", self.message),
                        AppErrorSource::Upstream => format!("{UPSTREAM_ERROR_PREFIX}{}", self.message),
                    },
                    "type": "proxy_error"
                }
            })),
        )
            .into_response()
    }
}
