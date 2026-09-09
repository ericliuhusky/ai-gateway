use super::dto::{
    CreateProviderReq, ProviderSummaryResp, UpdateSelectedModelRequest,
    UpdateSelectedProviderRequest, UpdateSelectedReasoningEffortRequest,
};
use crate::{
    config::{Config, DEFAULT_CODEX_CLIENT_VERSION},
    domain::{Provider, ProviderAuthMode, SelectedRoute},
    openai::{
        DeviceLoginCompletion, DeviceLoginPoll, DeviceLoginStart, OpenAiClient,
        OpenAiDeviceLoginService, OpenAiTokenService,
    },
    store::{ProviderStore, RouteStore},
};
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderName, StatusCode},
    response::{IntoResponse, Json, Response},
};
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

#[derive(Debug, Default, Deserialize)]
pub struct ReplaceProviderQuery {
    #[serde(default)]
    pub replace: bool,
}

pub async fn healthz() -> &'static str {
    "ok"
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAuthTokensFile {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
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

fn device_login_conflict_response(email: String) -> OpenAiDeviceLoginStatusResponse {
    OpenAiDeviceLoginStatusResponse {
        status: "conflict".to_string(),
        login_id: None,
        user_code: None,
        verification_uri: None,
        interval_seconds: None,
        expires_in: None,
        email: Some(email),
        provider_id: None,
        error: None,
    }
}

/// Import OpenAI accounts from a pasted Codex `auth.json` or Cockpit Tools export.
pub async fn import_openai_token(
    State(state): State<AppState>,
    Query(query): Query<ReplaceProviderQuery>,
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
            .import_codex_tokens(tokens.access_token, refresh_token, tokens.account_id)
            .map_err(AppError::bad_request)?;
        let provider = if query.replace {
            state
                .providers
                .import_openai_provider_with_replacement(imported, true)
                .await
        } else {
            state.providers.import_openai_provider(imported).await
        }
        .map_err(AppError::bad_request)?;

        if first_imported.is_none() {
            first_imported = Some((
                provider.email().unwrap_or_default().to_string(),
                provider.id().to_string(),
            ));
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
        provider_id: provider.id().to_string(),
        email: provider.email().unwrap_or_default().to_string(),
        expiry_timestamp: provider.expiry_timestamp().unwrap_or_default(),
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
    Query(query): Query<ReplaceProviderQuery>,
) -> Result<Json<OpenAiDeviceLoginStatusResponse>, AppError> {
    let poll = state
        .openai_device_login
        .poll(&login_id, query.replace)
        .await
        .map_err(AppError::bad_request)?;

    match poll {
        DeviceLoginPoll::Pending(start) => Ok(Json(device_login_pending_response(start))),
        DeviceLoginPoll::Finalizing => Ok(Json(device_login_finalizing_response())),
        DeviceLoginPoll::Conflict(email) => Ok(Json(device_login_conflict_response(email))),
        DeviceLoginPoll::Replacement(imported) => {
            let provider = state
                .providers
                .import_openai_provider_with_replacement(imported, true)
                .await
                .map_err(AppError::bad_request)?;
            let completion = DeviceLoginCompletion {
                email: provider.email().unwrap_or_default().to_string(),
                provider_id: provider.id().to_string(),
            };
            state
                .openai_device_login
                .complete(&login_id, completion.clone())
                .await;
            Ok(Json(device_login_completed_response(completion)))
        }
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

            enum DeviceLoginFinalization {
                Completed(DeviceLoginCompletion),
                Conflict(String),
            }

            let completion = async {
                let imported = state
                    .openai_device_login
                    .exchange_authorization(&authorization, &state.openai_tokens)
                    .await
                    .map_err(AppError::upstream_message)?;
                let email = imported
                    .email()
                    .ok_or_else(|| AppError::bad_request("导入的 OpenAI 凭据缺少邮箱"))?
                    .to_string();
                if !query.replace && state.providers.account_exists(&email).await {
                    state
                        .openai_device_login
                        .mark_replacement(&login_id, imported)
                        .await
                        .map_err(AppError::bad_request)?;
                    return Ok::<_, AppError>(DeviceLoginFinalization::Conflict(email));
                }
                let provider = state
                    .providers
                    .import_openai_provider_with_replacement(imported, query.replace)
                    .await
                    .map_err(AppError::bad_request)?;
                Ok::<_, AppError>(DeviceLoginFinalization::Completed(DeviceLoginCompletion {
                    email: provider.email().unwrap_or_default().to_string(),
                    provider_id: provider.id().to_string(),
                }))
            }
            .await;

            match completion {
                Ok(DeviceLoginFinalization::Completed(completion)) => {
                    state
                        .openai_device_login
                        .complete(&login_id, completion.clone())
                        .await;
                    Ok(Json(device_login_completed_response(completion)))
                }
                Ok(DeviceLoginFinalization::Conflict(email)) => {
                    Ok(Json(device_login_conflict_response(email)))
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
    if provider.auth_mode() != ProviderAuthMode::Account {
        return Err(AppError::bad_request(format!(
            "供应商 `{}` 不支持账户额度查询",
            provider.name()
        )));
    }

    let provider_record = acquire_provider_for_use(&state, provider.id()).await?;
    let access_token = provider_record.account_access_token();
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
    Json(request): Json<CreateProviderReq>,
) -> Result<Json<Value>, AppError> {
    let provider = state
        .providers
        .upsert(request)
        .await
        .map_err(AppError::bad_request)?;

    Ok(Json(json!({
        "provider": {
            "id": provider.id(),
            "name": provider.name(),
            "auth_mode": provider.auth_mode(),
            "base_url": provider.base_url(),
            "api_key": provider.api_key(),
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

async fn resolve_selected_provider(state: &AppState) -> Result<Provider, AppError> {
    let route = selected_route(state).await?;
    if let Some(provider_id) = route.provider_id {
        return resolve_provider_by_id(state, &provider_id).await;
    }
    Err(no_provider_selected_error())
}

pub(super) fn no_provider_selected_error() -> AppError {
    AppError::bad_request("尚未选择供应商；请先调用 PUT /selected-provider")
}

pub(super) async fn selected_route(state: &AppState) -> Result<SelectedRoute, AppError> {
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

pub(super) fn build_passthrough_response(
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
    provider: &Provider,
) -> Result<reqwest::Response, AppError> {
    if provider.auth_mode() == ProviderAuthMode::Account {
        let provider_record = acquire_provider_for_use(state, provider.id()).await?;
        let access_token = provider_record.account_access_token();
        let client_version = DEFAULT_CODEX_CLIENT_VERSION;
        let upstream = state
            .upstream
            .account_models(access_token, Some(client_version))
            .await
            .map_err(AppError::upstream_message)?;
        return Ok(upstream);
    }

    let upstream = state
        .upstream
        .api_models(
            provider.base_url().ok_or_else(|| {
                AppError::bad_request(format!("供应商 `{}` 缺少 base_url", provider.name()))
            })?,
            provider.api_key().ok_or_else(|| {
                AppError::bad_request(format!("供应商 `{}` 缺少 api_key", provider.name()))
            })?,
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

pub(super) async fn resolve_provider_by_id(
    state: &AppState,
    provider_id: &str,
) -> Result<Provider, AppError> {
    let record = state
        .providers
        .find_by_id(provider_id)
        .await
        .ok_or_else(|| AppError::bad_request(format!("未知的 provider_id: {provider_id}")))?;
    Ok(record)
}

pub(super) async fn acquire_provider_for_use(
    state: &AppState,
    provider_id: &str,
) -> Result<Provider, AppError> {
    state
        .providers
        .acquire_by_id(&state.openai_tokens, provider_id)
        .await
        .map_err(AppError::bad_request)
}

async fn hydrated_provider_summaries(state: &AppState) -> Vec<ProviderSummaryResp> {
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
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            source: AppErrorSource::Gateway,
        }
    }

    pub(super) fn upstream(error: reqwest::Error) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            source: AppErrorSource::Upstream,
        }
    }

    pub(super) fn upstream_message(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
            source: AppErrorSource::Upstream,
        }
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
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
