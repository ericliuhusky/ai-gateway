use crate::{
    adapters::responses::{request_with_model, responses_to_chat_completions},
    auth::OAuthClient,
    config::Config,
    models::{
        AccountRecord, ApiProviderRecord, ApiProviderSummary, ClientProtocol, CodexConfigStatus,
        CreateApiProviderRequest, GatewayLogDetail, GatewayLogDetailResponse,
        GatewayLogListResponse, GatewayLogSettings, GatewayLogSettingsResponse, GatewayLogSummary,
        ModelListItem, ModelListResponse, PROVIDER_OPENAI_PROXY, ProviderAuthMode,
        ProviderQuotaCredits, ProviderQuotaResponse, ProviderQuotaSnapshot, ProviderQuotaSummary,
        ProviderQuotaWindow, QuotaSource, QuotaSupportStatus, ResponsesRequest, SelectedRoute,
        UpdateGatewayLogSettingsRequest, UpdateSelectedModelRequest, UpdateSelectedProviderRequest,
        UpstreamProtocol, UpstreamRateLimitStatusDetails, UpstreamRateLimitStatusPayload,
        UpstreamRateLimitWindowSnapshot,
    },
    store::{
        AccountStore, LogEvent, LogStage, LogStore, ModelStore, ProviderStore, RouteStore,
        log_store::extract_model_output_from_body,
    },
    support::time::now_unix,
    upstream::{
        OPENAI_CODEX_BASE_URL, OpenAiEndpoint, PrivateOpenAiRequestBuilder,
        PublicOpenAiRequestBuilder, UpstreamClient, chat_completions_api_url, responses_api_url,
    },
};
use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{Form, Path as AxumPath, Query, State},
    http::{HeaderValue, StatusCode},
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use chrono::{Local, TimeZone};
use debug_web::{
    DebugLogDetail as DebugWebLogDetail, DebugLogSummary as DebugWebLogSummary, DebugPageData,
};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;
use std::{fs, io::ErrorKind, path::Path, sync::Arc};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub _client: Client,
    pub _config: Arc<Config>,
    pub oauth: OAuthClient,
    pub accounts: AccountStore,
    pub providers: ProviderStore,
    pub routes: RouteStore,
    pub models: ModelStore,
    pub upstream: UpstreamClient,
    pub logs: LogStore,
}

#[derive(Debug, Deserialize)]
pub struct ListModelsQuery {
    #[serde(default)]
    pub force: bool,
}

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn auth_openai_start(State(state): State<AppState>) -> Result<Redirect, AppError> {
    let url = state
        .oauth
        .create_openai_auth_url()
        .await
        .map_err(AppError::bad_request)?;
    Ok(Redirect::temporary(&url))
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

fn auth_success_html(provider_name: &str, email: &str) -> Html<String> {
    Html(format!(
        "<html lang='zh-CN'><head><meta charset='utf-8'></head><body style='font-family:sans-serif;padding:32px'><h1>{provider_name} 登录成功</h1><p>账号 <strong>{email}</strong> 已加入代理池。</p><p>你现在可以关闭此页面，并调用 <code>{responses_path}</code>。</p></body></html>",
        responses_path = Config::responses_path()
    ))
}

pub async fn auth_openai_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<Html<String>, AppError> {
    if let Some(error) = query.error {
        return Err(AppError::bad_request(format!(
            "openai oauth error: {error}"
        )));
    }

    let code = query
        .code
        .ok_or_else(|| AppError::bad_request("missing oauth code"))?;
    let state_token = query
        .state
        .ok_or_else(|| AppError::bad_request("missing oauth state"))?;
    let code_verifier = state
        .oauth
        .consume_openai_code_verifier(&state_token)
        .await
        .map_err(AppError::bad_request)?;
    let token = state
        .oauth
        .exchange_openai_code(&code, &code_verifier)
        .await
        .map_err(AppError::bad_request)?;
    let imported = state
        .oauth
        .openai_auth_from_token_response(token)
        .map_err(AppError::bad_request)?;
    let email = imported.email.clone();
    let account = state
        .accounts
        .add_openai_account(imported)
        .await
        .map_err(AppError::bad_request)?;
    state
        .providers
        .add_account_provider(PROVIDER_OPENAI_PROXY, &account.id)
        .await
        .map_err(AppError::bad_request)?;

    Ok(auth_success_html("OpenAI", &email))
}

#[derive(Debug, Deserialize)]
struct CodexAuthTokensFile {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    tokens: Option<CodexAuthTokensFile>,
}

#[derive(Debug, Serialize)]
pub struct ImportOpenAiFromLocalResponse {
    imported: bool,
    email: String,
    account_id: String,
    has_responses_write: bool,
    source_path: String,
}

/// Import an OpenAI account from Codex's local `~/.codex/auth.json` without running OAuth.
///
/// This is useful when Codex is already logged in on the same machine, and we just want the
/// gateway to reuse that session for the `openai-proxy` account provider.
pub async fn import_openai_from_local_codex_auth(
    State(state): State<AppState>,
) -> Result<Json<ImportOpenAiFromLocalResponse>, AppError> {
    let config = state._config.as_ref();
    let auth_path = config.codex_auth_path();
    let content = fs::read_to_string(&auth_path).map_err(|err| {
        AppError::bad_request(format!(
            "failed to read Codex auth.json at {}: {err}",
            auth_path.display()
        ))
    })?;

    let auth_file: CodexAuthFile = serde_json::from_str(&content).map_err(|err| {
        AppError::bad_request(format!(
            "failed to parse Codex auth.json at {}: {err}",
            auth_path.display()
        ))
    })?;
    let tokens = auth_file.tokens.ok_or_else(|| {
        AppError::bad_request("Codex auth.json is missing `tokens` (please login in Codex first)")
    })?;
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        AppError::bad_request("Codex auth.json tokens is missing `refresh_token`")
    })?;

    let imported = state
        .oauth
        .openai_auth_from_local_tokens(
            tokens.access_token,
            refresh_token,
            tokens.id_token,
            tokens.account_id,
        )
        .map_err(AppError::bad_request)?;
    let has_responses_write = imported
        .scopes
        .iter()
        .any(|scope| scope == "api.responses.write");
    let email = imported.email.clone();

    let account = state
        .accounts
        .add_openai_account(imported)
        .await
        .map_err(AppError::bad_request)?;
    state
        .providers
        .add_account_provider(PROVIDER_OPENAI_PROXY, &account.id)
        .await
        .map_err(AppError::bad_request)?;

    Ok(Json(ImportOpenAiFromLocalResponse {
        imported: true,
        email,
        account_id: account.id,
        has_responses_write,
        source_path: auth_path.display().to_string(),
    }))
}

pub async fn list_providers(State(state): State<AppState>) -> Json<Value> {
    let providers = hydrated_provider_summaries(&state).await;
    Json(json!({ "providers": providers }))
}

pub async fn get_provider_quota(
    State(state): State<AppState>,
    AxumPath(provider_id): AxumPath<String>,
) -> Result<Json<ProviderQuotaResponse>, AppError> {
    let provider = resolve_provider_by_id(&state, &provider_id).await?;
    let provider_summary = provider_summary_for_resolved(&state, &provider).await?;

    let quota = if provider.auth_mode == ProviderAuthMode::Account {
        let account = resolve_account_for_provider(&state, &provider).await?;
        let private_usage = PrivateOpenAiRequestBuilder {
            base_url: OPENAI_CODEX_BASE_URL,
            access_token: account.access_token(),
            account_id: account.upstream_account_id(),
            client_version: None,
        };
        let upstream = state
            .upstream
            .openai_send(&private_usage, OpenAiEndpoint::Usage)
            .await
            .map_err(AppError::upstream_message)?;
        let raw: Value = upstream.json().await.map_err(AppError::upstream)?;
        let payload: UpstreamRateLimitStatusPayload = serde_json::from_value(raw)
            .map_err(|err| AppError::upstream_message(err.to_string()))?;
        quota_from_openai_usage(payload)
    } else {
        unsupported_quota_summary(format!("missing provider record for `{}`", provider.name))
    };

    Ok(Json(ProviderQuotaResponse {
        provider: provider_summary,
        quota,
    }))
}

pub async fn list_models(
    State(state): State<AppState>,
    Query(query): Query<ListModelsQuery>,
) -> Result<Json<ModelListResponse>, AppError> {
    let provider = resolve_selected_provider(&state).await?;
    let response = load_provider_models(&state, &provider, query.force).await?;

    Ok(Json(response))
}

pub async fn add_provider(
    State(state): State<AppState>,
    Json(request): Json<CreateApiProviderRequest>,
) -> Result<Json<Value>, AppError> {
    let provider = state
        .providers
        .upsert(request)
        .await
        .map_err(AppError::bad_request)?;

    Ok(Json(json!({
        "provider": {
            "id": provider.id,
            "name": provider.name,
            "auth_mode": provider.auth_mode,
            "base_url": provider.base_url,
            "api_key": provider.api_key,
            "account_id": provider.account_id,
            "uses_chat_completions": provider.uses_chat_completions,
            "billing_mode": provider.billing_mode,
        }
    })))
}

pub async fn delete_provider(
    State(state): State<AppState>,
    AxumPath(provider_id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let deleted = state
        .providers
        .delete(&provider_id)
        .await
        .map_err(AppError::bad_request)?;

    let route = state.routes.get().await;
    if route.provider_id.as_deref() == Some(provider_id.as_str()) {
        state
            .routes
            .set_provider(None)
            .await
            .map_err(AppError::bad_request)?;
    }

    Ok(Json(json!({
        "deleted_provider": {
            "id": deleted.id,
            "name": deleted.name,
        }
    })))
}

pub async fn get_route(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "selected_provider": route_payload(state.routes.get().await) }))
}

pub async fn set_route(
    State(state): State<AppState>,
    Json(request): Json<UpdateSelectedProviderRequest>,
) -> Result<Json<Value>, AppError> {
    let provider_id = normalize_selected_provider_id(request.provider_id)?;
    let _provider = resolve_provider_by_id(&state, &provider_id).await?;

    let route = state
        .routes
        .set_provider(Some(provider_id))
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(json!({
        "selected_provider": route_payload(route),
    })))
}

pub async fn get_selected_model(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "selected_model": route_payload(state.routes.get().await) }))
}

pub async fn set_selected_model(
    State(state): State<AppState>,
    Json(request): Json<UpdateSelectedModelRequest>,
) -> Result<Json<Value>, AppError> {
    let model = normalize_selected_model(request.model)?;
    let provider = resolve_selected_provider(&state).await?;
    let models = load_provider_models(&state, &provider, false).await?;
    if !models.data.iter().any(|item| item.id == model) {
        return Err(AppError::bad_request(format!(
            "model `{model}` is not available for selected provider `{}`",
            provider.name
        )));
    }

    let route = state
        .routes
        .set_model(Some(model))
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(json!({ "selected_model": route_payload(route) })))
}

pub async fn clear_selected_model(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let route = state
        .routes
        .set_model(None)
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(json!({ "selected_model": route_payload(route) })))
}

pub async fn get_codex_config_status(
    State(state): State<AppState>,
) -> Result<Json<Value>, AppError> {
    let status = codex_config_status(&state)?;
    Ok(Json(json!({ "codex_config": status })))
}

pub async fn apply_codex_config(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let config = state._config.as_ref();
    let target_path = config.codex_config_path();
    let backup_path = config.codex_config_backup_path();
    let auth_path = config.codex_auth_path();
    let auth_backup_path = config.codex_auth_backup_path();

    fs::create_dir_all(config.data_dir())
        .map_err(|err| AppError::bad_request(format!("failed to create data dir: {err}")))?;
    fs::create_dir_all(config.codex_dir())
        .map_err(|err| AppError::bad_request(format!("failed to create CodeX dir: {err}")))?;

    if !backup_path.exists() {
        backup_if_exists(&target_path, &backup_path, "CodeX config")?;
    }

    if !auth_backup_path.exists() {
        backup_if_exists(&auth_path, &auth_backup_path, "CodeX auth")?;
    }

    fs::write(&target_path, config.bundled_codex_config())
        .map_err(|err| AppError::bad_request(format!("failed to write CodeX config: {err}")))?;
    remove_file_if_exists(&auth_path, "CodeX auth")?;

    Ok(Json(
        json!({ "codex_config": codex_config_status(&state)? }),
    ))
}

pub async fn restore_codex_config(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let config = state._config.as_ref();
    let target_path = config.codex_config_path();
    let backup_path = config.codex_config_backup_path();
    let auth_path = config.codex_auth_path();
    let auth_backup_path = config.codex_auth_backup_path();

    if !backup_path.exists() && !auth_backup_path.exists() {
        return Err(AppError::bad_request("no CodeX config backup available"));
    }

    if backup_path.exists() {
        restore_or_remove_backup(&backup_path, &target_path, "CodeX config")?;
        let _ = fs::remove_file(&backup_path);
    }

    if auth_backup_path.exists() {
        restore_or_remove_backup(&auth_backup_path, &auth_path, "CodeX auth")?;
        let _ = fs::remove_file(&auth_backup_path);
    }

    Ok(Json(
        json!({ "codex_config": codex_config_status(&state)? }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DebugDashboardQuery {
    pub id: Option<String>,
    pub limit: Option<usize>,
    pub notice: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DebugLogSettingsForm {
    pub enabled: bool,
    pub id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DebugClearLogsForm {
    pub limit: Option<usize>,
}

pub async fn get_logs(
    State(state): State<AppState>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<GatewayLogListResponse>, AppError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let logs = state
        .logs
        .list_request_summaries(limit)
        .map_err(AppError::internal)?;
    Ok(Json(GatewayLogListResponse { logs }))
}

pub async fn get_log_detail(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<GatewayLogDetailResponse>, AppError> {
    let detail = state.logs.load_request(&id).map_err(AppError::internal)?;
    let Some(log) = detail else {
        return Err(AppError::bad_request(format!("log id not found: {id}")));
    };
    Ok(Json(GatewayLogDetailResponse { log }))
}

pub async fn get_log_settings(State(state): State<AppState>) -> Json<GatewayLogSettingsResponse> {
    Json(GatewayLogSettingsResponse {
        logging: GatewayLogSettings {
            enabled: state.logs.is_enabled(),
        },
    })
}

pub async fn set_log_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateGatewayLogSettingsRequest>,
) -> Result<Json<GatewayLogSettingsResponse>, AppError> {
    let enabled = state
        .logs
        .set_enabled(request.enabled)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(GatewayLogSettingsResponse {
        logging: GatewayLogSettings { enabled },
    }))
}

pub async fn clear_logs(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    state.logs.clear().await.map_err(AppError::internal)?;
    Ok(Json(json!({ "cleared": true })))
}

pub async fn debug_dashboard(
    State(state): State<AppState>,
    Query(query): Query<DebugDashboardQuery>,
) -> Result<Html<String>, AppError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let logs = state
        .logs
        .list_request_summaries(limit)
        .map_err(AppError::internal)?;
    let selected_id = query
        .id
        .clone()
        .or_else(|| logs.first().map(|log| log.id.clone()));

    let selected_detail = if let Some(id) = selected_id.as_ref() {
        state
            .logs
            .load_request(id)
            .map_err(AppError::internal)?
            .map(map_debug_log_detail)
    } else {
        None
    };

    let invalid_selection_error = if query.id.is_some() && selected_detail.is_none() {
        Some("指定的 id 不存在或已经被清空。".to_string())
    } else {
        None
    };

    let document = debug_web::render_debug_page(DebugPageData {
        logging_enabled: state.logs.is_enabled(),
        logs: logs.into_iter().map(map_debug_log_summary).collect(),
        selected_id,
        selected_detail,
        limit,
        notice: query.notice,
        error: query.error.or(invalid_selection_error),
    });
    Ok(Html(document))
}

pub async fn debug_set_log_settings(
    State(state): State<AppState>,
    Form(form): Form<DebugLogSettingsForm>,
) -> Result<Redirect, AppError> {
    let enabled = state
        .logs
        .set_enabled(form.enabled)
        .await
        .map_err(AppError::internal)?;
    let notice = if enabled {
        "日志记录已开启"
    } else {
        "日志记录已暂停"
    };
    Ok(Redirect::to(&build_debug_redirect_url(
        form.limit.unwrap_or(100),
        form.id.as_deref(),
        Some(notice),
        None,
    )))
}

pub async fn debug_clear_logs(
    State(state): State<AppState>,
    Form(form): Form<DebugClearLogsForm>,
) -> Result<Redirect, AppError> {
    state.logs.clear().await.map_err(AppError::internal)?;
    Ok(Redirect::to(&build_debug_redirect_url(
        form.limit.unwrap_or(100),
        None,
        Some("日志已清空"),
        None,
    )))
}

pub async fn responses(State(state): State<AppState>, body: Bytes) -> Result<Response, AppError> {
    let raw_body = std::str::from_utf8(&body)
        .map_err(|_| AppError::bad_request("request body must be valid UTF-8"))?
        .to_owned();
    let mut request: ResponsesRequest = serde_json::from_str(&raw_body)
        .map_err(|err| AppError::bad_request(format!("invalid request JSON: {err}")))?;

    let id = Uuid::new_v4().simple().to_string();
    let started_at = Instant::now();
    log_http_event(
        &state.logs,
        &id,
        LogStage::ClientRequest,
        None,
        Some(ClientProtocol::OpenAiResponses.as_str()),
        None,
        None,
        None,
        None,
        Some(&request.model),
        request.stream,
        Some("POST"),
        Some(Config::responses_path()),
        None,
        Some(raw_body),
        None,
        None,
    )
    .await;

    apply_selected_model_override(&state, &mut request).await;

    let model = request.model.clone();
    let stream = request.stream;
    match responses_inner(state.clone(), request, id.clone(), started_at).await {
        Ok(response) => Ok(response),
        Err(err) => {
            let error_body = gateway_error_payload(&err.message);
            let elapsed = elapsed_ms(started_at);
            log_http_event(
                &state.logs,
                &id,
                LogStage::Error,
                Some(err.status),
                Some(ClientProtocol::OpenAiResponses.as_str()),
                None,
                None,
                None,
                None,
                Some(&model),
                stream,
                Some("POST"),
                Some(Config::responses_path()),
                None,
                Some(json_value_for_storage(&error_body)),
                Some(err.message.clone()),
                Some(elapsed),
            )
            .await;
            log_http_event(
                &state.logs,
                &id,
                LogStage::UpstreamResponse,
                Some(err.status),
                Some(ClientProtocol::OpenAiResponses.as_str()),
                None,
                None,
                None,
                None,
                Some(&model),
                stream,
                Some("POST"),
                Some(Config::responses_path()),
                None,
                Some(json_value_for_storage(&error_body)),
                Some(err.message.clone()),
                Some(elapsed),
            )
            .await;
            Err(err)
        }
    }
}

fn map_debug_log_summary(log: GatewayLogSummary) -> DebugWebLogSummary {
    DebugWebLogSummary {
        id: log.id,
        updated_at_label: format_timestamp(log.updated_at),
        provider_name: log.provider_name,
        account_email: log.account_email,
        model: log.model,
        stream: log.stream,
        status_code: log.status_code,
        has_error: log.has_error,
        error_message: log.error_message,
        client_protocol: log.client_protocol,
        upstream_protocol: log.upstream_protocol,
        method: log.method,
        path: log.path,
        upstream_request_url: log.upstream_request_url,
        user_input: log.user_input,
        model_output: log.model_output,
    }
}

fn map_debug_log_detail(log: GatewayLogDetail) -> DebugWebLogDetail {
    DebugWebLogDetail {
        id: log.id,
        created_at_label: format_timestamp(log.created_at),
        updated_at_label: format_timestamp(log.updated_at),
        provider_name: log.provider_name,
        account_id: log.account_id,
        account_email: log.account_email,
        model: log.model,
        stream: log.stream,
        client_protocol: log.client_protocol,
        upstream_protocol: log.upstream_protocol,
        method: log.method,
        path: log.path,
        upstream_request_url: log.upstream_request_url,
        client_request_body: log.client_request_body,
        client_request_body_truncated: log.client_request_body_truncated,
        upstream_request_body: log.upstream_request_body,
        upstream_request_body_truncated: log.upstream_request_body_truncated,
        client_response_status_code: log.client_response_status_code,
        client_response_body: log.client_response_body,
        client_response_body_truncated: log.client_response_body_truncated,
        upstream_response_status_code: log.upstream_response_status_code,
        upstream_response_body: log.upstream_response_body,
        upstream_response_body_truncated: log.upstream_response_body_truncated,
        error_message: log.error_message,
        error_truncated: log.error_truncated,
        elapsed_ms: log.elapsed_ms,
        user_input: log.user_input,
        user_input_path: log.user_input_path,
        model_output: log.model_output,
        model_output_path: log.model_output_path,
    }
}

fn format_timestamp(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.format("%Y年%-m月%-d日 %H:%M").to_string())
        .unwrap_or_else(|| timestamp.to_string())
}

fn build_debug_redirect_url(
    limit: usize,
    id: Option<&str>,
    notice: Option<&str>,
    error: Option<&str>,
) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("limit", &limit.clamp(1, 500).to_string());
    if let Some(id) = id {
        serializer.append_pair("id", id);
    }
    if let Some(notice) = notice {
        serializer.append_pair("notice", notice);
    }
    if let Some(error) = error {
        serializer.append_pair("error", error);
    }
    format!("/debug?{}", serializer.finish())
}

pub(super) async fn responses_inner(
    state: AppState,
    request: ResponsesRequest,
    id: String,
    started_at: Instant,
) -> Result<Response, AppError> {
    let provider = resolve_selected_provider(&state).await?;
    if !request.stream {
        return Err(AppError::bad_request(
            "responses 接口请求必须使用流式 (\"stream\": true)".to_string(),
        ));
    }
    if provider.auth_mode == ProviderAuthMode::Account && provider_uses_openai_account(&provider) {
        let account = resolve_account_for_provider(&state, &provider).await?;
        let request_body =
            serde_json::to_value(&request).map_err(|err| AppError::internal(err.to_string()))?;

        log_http_event(
            &state.logs,
            &id,
            LogStage::UpstreamRequest,
            None,
            Some(ClientProtocol::OpenAiResponses.as_str()),
            Some(UpstreamProtocol::OpenAiPrivateResponses.as_str()),
            Some(&provider.name),
            Some(&account.id),
            Some(&account.email),
            Some(&request.model),
            request.stream,
            Some("POST"),
            None,
            Some(Config::openai_private_responses_url()),
            Some(json_value_for_storage(&request_body)),
            None,
            None,
        )
        .await;

        let private_responses = PrivateOpenAiRequestBuilder {
            base_url: OPENAI_CODEX_BASE_URL,
            access_token: account.access_token(),
            account_id: account.upstream_account_id(),
            client_version: None,
        };
        let upstream = state
            .upstream
            .openai_send(
                &private_responses,
                OpenAiEndpoint::Responses {
                    body: request_body,
                    stream: true,
                },
            )
            .await
            .map_err(AppError::upstream_message)?;
        let upstream_status = upstream.status();

        let logs = state.logs.clone();
        let id_for_stream = id.clone();
        let provider_name = provider.name.clone();
        let account_id = account.id.clone();
        let account_email = account.email.clone();
        let model = request.model.clone();
        let output = stream! {
            let mut stream = upstream.bytes_stream();
            let mut response_body = String::new();
            let mut final_response_sse_buffer = String::new();
            let mut final_response_body: Option<String> = None;

            while let Some(result) = stream.next().await {
                match result {
                    Ok(chunk) => {
                        let chunk_text = String::from_utf8_lossy(&chunk);
                        append_to_log_buffer(&mut response_body, &chunk_text);
                        capture_final_response_from_sse_chunk(
                            &mut final_response_sse_buffer,
                            &chunk_text,
                            &mut final_response_body,
                        );
                        yield Ok::<Bytes, std::io::Error>(chunk);
                    }
                    Err(err) => {
                        let error_message = err.to_string();
                        log_http_event(
                            &logs,
                            &id_for_stream,
                            LogStage::Error,
                            Some(StatusCode::BAD_GATEWAY),
                            Some(ClientProtocol::OpenAiResponses.as_str()),
                            Some(UpstreamProtocol::OpenAiPrivateResponses.as_str()),
                            Some(&provider_name),
                            Some(&account_id),
                            Some(&account_email),
                            Some(&model),
                            true,
                            Some("POST"),
                            Some(Config::responses_path()),
                            Some(Config::openai_private_responses_url()),
                            Some(response_body.clone()),
                            Some(error_message.clone()),
                            Some(elapsed_ms(started_at)),
                        )
                        .await;
                        yield Err(std::io::Error::other(err));
                        return;
                    }
                }
            }

            let elapsed = elapsed_ms(started_at);
            let logged_response_body =
                logged_stream_response_body(final_response_body.as_deref(), &response_body);
            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::ClientResponse,
                Some(upstream_status),
                Some(ClientProtocol::OpenAiResponses.as_str()),
                Some(UpstreamProtocol::OpenAiPrivateResponses.as_str()),
                Some(&provider_name),
                Some(&account_id),
                Some(&account_email),
                Some(&model),
                true,
                Some("POST"),
                None,
                Some(Config::openai_private_responses_url()),
                Some(logged_response_body.clone()),
                None,
                Some(elapsed),
            )
            .await;
            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::UpstreamResponse,
                Some(StatusCode::OK),
                Some(ClientProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider_name),
                Some(&account_id),
                Some(&account_email),
                Some(&model),
                true,
                Some("POST"),
                Some(Config::responses_path()),
                None,
                Some(logged_response_body),
                None,
                Some(elapsed),
            )
            .await;
        };

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(
                "content-type",
                HeaderValue::from_static("text/event-stream"),
            )
            .header("cache-control", HeaderValue::from_static("no-cache"))
            .header("connection", HeaderValue::from_static("keep-alive"))
            .header(
                "x-account-email",
                HeaderValue::from_str(&account.email)
                    .map_err(|err| AppError::internal(err.to_string()))?,
            )
            .header(
                "x-provider",
                HeaderValue::from_str(&provider.name)
                    .map_err(|err| AppError::internal(err.to_string()))?,
            )
            .body(Body::from_stream(output))
            .map_err(|err| AppError::internal(err.to_string()))?);
    }

    if provider.auth_mode == ProviderAuthMode::Account {
        return Err(AppError::bad_request(format!(
            "account auth provider is not supported yet: {}",
            provider.name
        )));
    }

    let native_provider = provider
        .record
        .clone()
        .ok_or_else(|| AppError::bad_request(format!("unknown provider: {}", provider.name)))?;

    let native_target = resolve_native_target(&native_provider, &request.model);
    if native_target.uses_chat_completions {
        let request_body = responses_to_chat_completions(&request, &native_target.upstream_model)
            .map_err(AppError::bad_request)?;
        let upstream_url = chat_completions_api_url(&native_provider.base_url);
        log_http_event(
            &state.logs,
            &id,
            LogStage::UpstreamRequest,
            None,
            Some(ClientProtocol::OpenAiResponses.as_str()),
            Some(native_target.upstream.as_str()),
            Some(&provider.name),
            None,
            None,
            Some(&request.model),
            request.stream,
            Some("POST"),
            None,
            Some(&upstream_url),
            Some(json_value_for_storage(&request_body)),
            None,
            None,
        )
        .await;

        let public_chat = PublicOpenAiRequestBuilder {
            base_url: native_provider.base_url.as_str(),
            api_key: native_provider.api_key.as_str(),
        };
        let upstream = state
            .upstream
            .openai_send(
                &public_chat,
                OpenAiEndpoint::ChatCompletions { body: request_body },
            )
            .await
            .map_err(AppError::upstream_message)?;
        let upstream_status = upstream.status();

        let logs = state.logs.clone();
        let id_for_stream = id.clone();
        let provider_name = provider.name.clone();
        let model = request.model.clone();
        let upstream_protocol = native_target.upstream.as_str().to_string();
        let upstream_url_for_stream = upstream_url.clone();
        let output = stream! {
            let mut stream = upstream.bytes_stream();
            let mut chat_sse_buffer = String::new();
            let mut response_body = String::new();
            let mut final_response_sse_buffer = String::new();
            let mut final_response_body: Option<String> = None;
            let mut response_stream =
                ChatCompletionsResponsesStream::new(model.clone(), now_unix());

            while let Some(result) = stream.next().await {
                match result {
                    Ok(chunk) => {
                        let chunk_text = String::from_utf8_lossy(&chunk);
                        let payloads = drain_sse_payloads(&mut chat_sse_buffer, &chunk_text);
                        for payload in payloads {
                            match response_stream.push_chat_payload(&payload) {
                                Ok(events) => {
                                    for event in events {
                                        append_to_log_buffer(&mut response_body, &event);
                                        capture_final_response_from_sse_chunk(
                                            &mut final_response_sse_buffer,
                                            &event,
                                            &mut final_response_body,
                                        );
                                        yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                                    }
                                }
                                Err(err) => {
                                    log_http_event(
                                        &logs,
                                        &id_for_stream,
                                        LogStage::Error,
                                        Some(StatusCode::BAD_GATEWAY),
                                        Some(ClientProtocol::OpenAiResponses.as_str()),
                                        Some(&upstream_protocol),
                                        Some(&provider_name),
                                        None,
                                        None,
                                        Some(&model),
                                        true,
                                        Some("POST"),
                                        Some(Config::responses_path()),
                                        Some(&upstream_url_for_stream),
                                        Some(response_body.clone()),
                                        Some(err),
                                        Some(elapsed_ms(started_at)),
                                    )
                                    .await;
                                    yield Err(std::io::Error::other("failed to parse chat completions stream"));
                                    return;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        log_http_event(
                            &logs,
                            &id_for_stream,
                            LogStage::Error,
                            Some(StatusCode::INTERNAL_SERVER_ERROR),
                            Some(ClientProtocol::OpenAiResponses.as_str()),
                            Some(&upstream_protocol),
                            Some(&provider_name),
                            None,
                            None,
                            Some(&model),
                            true,
                            Some("POST"),
                            Some(Config::responses_path()),
                            Some(&upstream_url_for_stream),
                            Some(response_body.clone()),
                            Some(err.to_string()),
                            Some(elapsed_ms(started_at)),
                        )
                        .await;
                        yield Err(std::io::Error::other(err));
                        return;
                    }
                }
            }

            if !chat_sse_buffer.trim().is_empty() {
                let payloads = drain_sse_payloads(&mut chat_sse_buffer, "\n\n");
                for payload in payloads {
                    match response_stream.push_chat_payload(&payload) {
                        Ok(events) => {
                            for event in events {
                                append_to_log_buffer(&mut response_body, &event);
                                capture_final_response_from_sse_chunk(
                                    &mut final_response_sse_buffer,
                                    &event,
                                    &mut final_response_body,
                                );
                                yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                            }
                        }
                        Err(err) => {
                            log_http_event(
                                &logs,
                                &id_for_stream,
                                LogStage::Error,
                                Some(StatusCode::BAD_GATEWAY),
                                Some(ClientProtocol::OpenAiResponses.as_str()),
                                Some(&upstream_protocol),
                                Some(&provider_name),
                                None,
                                None,
                                Some(&model),
                                true,
                                Some("POST"),
                                Some(Config::responses_path()),
                                Some(&upstream_url_for_stream),
                                Some(response_body.clone()),
                                Some(err),
                                Some(elapsed_ms(started_at)),
                            )
                            .await;
                            yield Err(std::io::Error::other("failed to parse chat completions stream"));
                            return;
                        }
                    }
                }
            }

            match response_stream.finish() {
                Ok(events) => {
                    for event in events {
                        append_to_log_buffer(&mut response_body, &event);
                        capture_final_response_from_sse_chunk(
                            &mut final_response_sse_buffer,
                            &event,
                            &mut final_response_body,
                        );
                        yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                    }
                }
                Err(err) => {
                    log_http_event(
                        &logs,
                        &id_for_stream,
                        LogStage::Error,
                        Some(StatusCode::INTERNAL_SERVER_ERROR),
                        Some(ClientProtocol::OpenAiResponses.as_str()),
                        Some(&upstream_protocol),
                        Some(&provider_name),
                        None,
                        None,
                        Some(&model),
                        true,
                        Some("POST"),
                        Some(Config::responses_path()),
                        Some(&upstream_url_for_stream),
                        Some(response_body.clone()),
                        Some(err),
                        Some(elapsed_ms(started_at)),
                    )
                    .await;
                    yield Err(std::io::Error::other("failed to finish chat completions stream"));
                    return;
                }
            }

            let elapsed = elapsed_ms(started_at);
            let logged_response_body =
                logged_stream_response_body(final_response_body.as_deref(), &response_body);
            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::ClientResponse,
                Some(upstream_status),
                Some(ClientProtocol::OpenAiResponses.as_str()),
                Some(&upstream_protocol),
                Some(&provider_name),
                None,
                None,
                Some(&model),
                true,
                Some("POST"),
                None,
                Some(&upstream_url_for_stream),
                Some(logged_response_body.clone()),
                None,
                Some(elapsed),
            )
            .await;
            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::UpstreamResponse,
                Some(StatusCode::OK),
                Some(ClientProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider_name),
                None,
                None,
                Some(&model),
                true,
                Some("POST"),
                Some(Config::responses_path()),
                None,
                Some(logged_response_body),
                None,
                Some(elapsed),
            )
            .await;
        };

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(
                "content-type",
                HeaderValue::from_static("text/event-stream"),
            )
            .header("cache-control", HeaderValue::from_static("no-cache"))
            .header("connection", HeaderValue::from_static("keep-alive"))
            .header(
                "x-provider",
                HeaderValue::from_str(&provider.name)
                    .map_err(|err| AppError::internal(err.to_string()))?,
            )
            .body(Body::from_stream(output))
            .map_err(|err| AppError::internal(err.to_string()))?);
    }

    let request_body = request_with_model(
        &request,
        &native_target.upstream_model,
        &native_provider.name,
    )
    .map_err(|err| AppError::internal(err.to_string()))?;
    let upstream_url = responses_api_url(&native_provider.base_url);
    log_http_event(
        &state.logs,
        &id,
        LogStage::UpstreamRequest,
        None,
        Some(ClientProtocol::OpenAiResponses.as_str()),
        Some(native_target.upstream.as_str()),
        Some(&provider.name),
        None,
        None,
        Some(&request.model),
        request.stream,
        Some("POST"),
        None,
        Some(&upstream_url),
        Some(json_value_for_storage(&request_body)),
        None,
        None,
    )
    .await;

    let public_responses = PublicOpenAiRequestBuilder {
        base_url: native_provider.base_url.as_str(),
        api_key: native_provider.api_key.as_str(),
    };
    let upstream = state
        .upstream
        .openai_send(
            &public_responses,
            OpenAiEndpoint::Responses {
                body: request_body,
                stream: true,
            },
        )
        .await
        .map_err(AppError::upstream_message)?;
    let upstream_status = upstream.status();

    let logs = state.logs.clone();
    let id_for_stream = id.clone();
    let provider_name = provider.name.clone();
    let model = request.model.clone();
    let output = stream! {
        let mut stream = upstream.bytes_stream();
        let mut response_body = String::new();
        let mut final_response_sse_buffer = String::new();
        let mut final_response_body: Option<String> = None;

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    let chunk_text = String::from_utf8_lossy(&chunk);
                    append_to_log_buffer(&mut response_body, &chunk_text);
                    capture_final_response_from_sse_chunk(
                        &mut final_response_sse_buffer,
                        &chunk_text,
                        &mut final_response_body,
                    );
                    yield Ok::<Bytes, std::io::Error>(chunk);
                }
                Err(err) => {
                    log_http_event(
                        &logs,
                        &id_for_stream,
                        LogStage::Error,
                        Some(StatusCode::BAD_GATEWAY),
                        Some(ClientProtocol::OpenAiResponses.as_str()),
                        Some(native_target.upstream.as_str()),
                        Some(&provider_name),
                        None,
                        None,
                        Some(&model),
                        true,
                        Some("POST"),
                        Some(Config::responses_path()),
                        Some(&upstream_url),
                        Some(response_body.clone()),
                        Some(err.to_string()),
                        Some(elapsed_ms(started_at)),
                    )
                    .await;
                    yield Err(std::io::Error::other(err));
                    return;
                }
            }
        }

        let elapsed = elapsed_ms(started_at);
        let logged_response_body =
            logged_stream_response_body(final_response_body.as_deref(), &response_body);
        log_http_event(
            &logs,
            &id_for_stream,
            LogStage::ClientResponse,
            Some(upstream_status),
            Some(ClientProtocol::OpenAiResponses.as_str()),
            Some(native_target.upstream.as_str()),
            Some(&provider_name),
            None,
            None,
            Some(&model),
            true,
            Some("POST"),
            None,
            Some(&upstream_url),
            Some(logged_response_body.clone()),
            None,
            Some(elapsed),
        )
        .await;
        log_http_event(
            &logs,
            &id_for_stream,
            LogStage::UpstreamResponse,
            Some(StatusCode::OK),
            Some(ClientProtocol::OpenAiResponses.as_str()),
            None,
            Some(&provider_name),
            None,
            None,
            Some(&model),
            true,
            Some("POST"),
            Some(Config::responses_path()),
            None,
            Some(logged_response_body),
            None,
            Some(elapsed),
        )
        .await;
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            "content-type",
            HeaderValue::from_static("text/event-stream"),
        )
        .header("cache-control", HeaderValue::from_static("no-cache"))
        .header("connection", HeaderValue::from_static("keep-alive"))
        .header(
            "x-provider",
            HeaderValue::from_str(&provider.name)
                .map_err(|err| AppError::internal(err.to_string()))?,
        )
        .body(Body::from_stream(output))
        .map_err(|err| AppError::internal(err.to_string()))?)
}

pub(super) async fn resolve_selected_provider(
    state: &AppState,
) -> Result<ResolvedProvider, AppError> {
    let route = state.routes.get().await;
    if let Some(provider_id) = route.provider_id {
        return resolve_provider_by_id(state, &provider_id).await;
    }

    Err(AppError::bad_request(
        "no provider selected; call PUT /selected-provider first",
    ))
}

fn route_payload(route: SelectedRoute) -> Value {
    json!({
        "provider_id": route.provider_id,
        "selected_model": route.selected_model,
        "updated_at": route.updated_at,
    })
}

pub(super) async fn apply_selected_model_override(
    state: &AppState,
    request: &mut ResponsesRequest,
) {
    if let Some(model) = state.routes.get().await.selected_model {
        request.model = model;
    }
}

async fn fetch_provider_models(
    state: &AppState,
    provider: &ResolvedProvider,
) -> Result<ModelListResponse, AppError> {
    if provider.auth_mode == ProviderAuthMode::Account {
        let account = resolve_account_for_provider(state, provider).await?;
        if provider.name == PROVIDER_OPENAI_PROXY {
            let client_version = state._config.codex_client_version();
            let private_models = PrivateOpenAiRequestBuilder {
                base_url: OPENAI_CODEX_BASE_URL,
                access_token: account.access_token(),
                account_id: account.upstream_account_id(),
                client_version: Some(client_version.as_str()),
            };
            let upstream = state
                .upstream
                .openai_send(&private_models, OpenAiEndpoint::Models)
                .await
                .map_err(AppError::upstream_message)?;
            let raw: Value = upstream.json().await.map_err(AppError::upstream)?;
            return openai_models_response(&provider.name, &raw);
        }

        return Err(AppError::bad_request(format!(
            "account auth provider is not supported yet: {}",
            provider.name
        )));
    }

    let native_provider = provider
        .record
        .as_ref()
        .ok_or_else(|| AppError::bad_request(format!("unknown provider: {}", provider.name)))?;
    let public_models = PublicOpenAiRequestBuilder {
        base_url: native_provider.base_url.as_str(),
        api_key: native_provider.api_key.as_str(),
    };
    let upstream = state
        .upstream
        .openai_send(&public_models, OpenAiEndpoint::Models)
        .await
        .map_err(AppError::upstream_message)?;
    let raw: Value = upstream.json().await.map_err(AppError::upstream)?;
    native_models_response(&provider.name, &raw)
}

async fn load_provider_models(
    state: &AppState,
    provider: &ResolvedProvider,
    force_refresh: bool,
) -> Result<ModelListResponse, AppError> {
    let provider_id = provider
        .record
        .as_ref()
        .map(|record| record.id.as_str())
        .or(provider.account_id.as_deref())
        .ok_or_else(|| {
            AppError::bad_request(format!("provider cache key missing: {}", provider.name))
        })?;

    if !force_refresh {
        if let Some(cached) = state.models.load(provider_id).map_err(AppError::internal)? {
            return Ok(cached);
        }
    }

    let models = fetch_provider_models(state, provider).await?;
    state
        .models
        .save(provider_id, &models)
        .map_err(AppError::internal)?;
    Ok(models)
}

fn codex_config_status(state: &AppState) -> Result<CodexConfigStatus, AppError> {
    let config = state._config.as_ref();
    let config_backup_exists = config.codex_config_backup_path().exists();
    let auth_backup_exists = config.codex_auth_backup_path().exists();

    Ok(CodexConfigStatus {
        target_path: config.codex_config_path().display().to_string(),
        auth_path: config.codex_auth_path().display().to_string(),
        config_backup_exists,
        auth_backup_exists,
        restore_available: config_backup_exists || auth_backup_exists,
        target_exists: config.codex_config_path().exists(),
        auth_exists: config.codex_auth_path().exists(),
    })
}

fn backup_if_exists(source: &Path, backup: &Path, label: &str) -> Result<(), AppError> {
    if source.exists() {
        fs::copy(source, backup)
            .map_err(|err| AppError::bad_request(format!("failed to back up {label}: {err}")))?;
    }

    Ok(())
}

fn restore_or_remove_backup(backup: &Path, target: &Path, label: &str) -> Result<(), AppError> {
    let backup_contents = fs::read(backup)
        .map_err(|err| AppError::bad_request(format!("failed to read {label} backup: {err}")))?;

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            AppError::bad_request(format!("failed to create {label} directory: {err}"))
        })?;
    }
    fs::write(target, backup_contents)
        .map_err(|err| AppError::bad_request(format!("failed to restore {label}: {err}")))?;

    Ok(())
}

fn remove_file_if_exists(target: &Path, label: &str) -> Result<(), AppError> {
    match fs::remove_file(target) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::bad_request(format!(
            "failed to remove {label} file: {err}"
        ))),
    }
}

fn normalize_selected_provider_id(provider_id: Option<String>) -> Result<String, AppError> {
    let provider_id = provider_id.ok_or_else(|| {
        AppError::bad_request("provider_id is required; automatic routing has been removed")
    })?;
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request(
            "provider_id cannot be empty; automatic routing has been removed",
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_selected_model(model: String) -> Result<String, AppError> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("model cannot be empty"));
    }
    Ok(trimmed.to_string())
}

fn native_models_response(_provider: &str, raw: &Value) -> Result<ModelListResponse, AppError> {
    let entries: Vec<&Value> = if let Some(data) = raw.get("data").and_then(Value::as_array) {
        data.iter().collect()
    } else if let Some(models) = raw.get("models").and_then(Value::as_array) {
        models.iter().collect()
    } else if let Some(array) = raw.as_array() {
        array.iter().collect()
    } else {
        return Err(AppError::upstream_message(
            "native models payload missing `data` or `models` array",
        ));
    };

    let mut data = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(id) = native_model_id(entry) {
            data.push(ModelListItem { id: id.to_string() });
        }
    }
    data.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(ModelListResponse {
        object: "list".to_string(),
        data,
    })
}

fn openai_models_response(_provider: &str, raw: &Value) -> Result<ModelListResponse, AppError> {
    let entries = raw
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::upstream_message("openai models payload missing `models`"))?;

    let mut data = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.get("supported_in_api").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let id = entry
            .get("slug")
            .or_else(|| entry.get("id"))
            .and_then(Value::as_str);
        if let Some(id) = id {
            let priority = entry
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            data.push((
                priority,
                id.to_string(),
                ModelListItem { id: id.to_string() },
            ));
        }
    }
    data.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    Ok(ModelListResponse {
        object: "list".to_string(),
        data: data.into_iter().map(|(_, _, item)| item).collect(),
    })
}

fn native_model_id(entry: &Value) -> Option<&str> {
    entry
        .get("id")
        .or_else(|| entry.get("model"))
        .or_else(|| entry.get("name"))
        .and_then(Value::as_str)
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedProvider {
    pub(super) name: String,
    pub(super) auth_mode: ProviderAuthMode,
    pub(super) account_id: Option<String>,
    pub(super) record: Option<ApiProviderRecord>,
}

async fn resolve_provider_by_id(
    state: &AppState,
    provider_id: &str,
) -> Result<ResolvedProvider, AppError> {
    let record = state
        .providers
        .find_by_id(provider_id)
        .await
        .ok_or_else(|| AppError::bad_request(format!("unknown provider_id: {provider_id}")))?;

    Ok(ResolvedProvider {
        name: record.name.clone(),
        auth_mode: record.auth_mode.clone(),
        account_id: record.account_id.clone(),
        record: Some(record),
    })
}

pub(super) async fn resolve_account_for_provider(
    state: &AppState,
    provider: &ResolvedProvider,
) -> Result<AccountRecord, AppError> {
    let account_id = provider
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::bad_request(format!(
                "account auth provider `{}` is missing account_id; bind an account first",
                provider.name
            ))
        })?;

    state
        .accounts
        .acquire_by_id(&state.oauth, &state.upstream, account_id)
        .await
        .map_err(AppError::bad_request)
}

pub(super) fn provider_uses_openai_account(provider: &ResolvedProvider) -> bool {
    provider
        .record
        .as_ref()
        .and_then(|record| record.account_id.as_ref())
        .is_some()
}

async fn hydrated_provider_summaries(state: &AppState) -> Vec<ApiProviderSummary> {
    let mut providers = state.providers.list().await;
    for provider in &mut providers {
        hydrate_provider_summary(state, provider).await;
    }
    providers
}

async fn provider_summary_for_resolved(
    state: &AppState,
    provider: &ResolvedProvider,
) -> Result<ApiProviderSummary, AppError> {
    let record = provider
        .record
        .clone()
        .ok_or_else(|| AppError::bad_request(format!("unknown provider: {}", provider.name)))?;
    let mut summary = ApiProviderSummary {
        id: record.id.clone(),
        name: record.name.clone(),
        auth_mode: record.auth_mode.clone(),
        base_url: record.base_url.clone(),
        account_id: record.account_id.clone(),
        account_email: None,
        uses_chat_completions: record.uses_chat_completions,
        billing_mode: record.billing_mode.clone(),
    };
    hydrate_provider_summary(state, &mut summary).await;
    Ok(summary)
}

async fn hydrate_provider_summary(state: &AppState, provider: &mut ApiProviderSummary) {
    if provider.auth_mode == ProviderAuthMode::Account
        && let Some(account_id) = provider.account_id.as_deref()
    {
        provider.account_email = state
            .accounts
            .find_by_id(account_id)
            .await
            .map(|account| account.email);
    }
}

fn unsupported_quota_summary(message: String) -> ProviderQuotaSummary {
    ProviderQuotaSummary {
        source: QuotaSource::Unsupported,
        status: QuotaSupportStatus::Unsupported,
        snapshot: None,
        additional_snapshots: Vec::new(),
        message: Some(message),
    }
}

fn quota_from_openai_usage(payload: UpstreamRateLimitStatusPayload) -> ProviderQuotaSummary {
    ProviderQuotaSummary {
        source: QuotaSource::ChatgptCodexUsageApi,
        status: QuotaSupportStatus::Supported,
        snapshot: Some(rate_limit_snapshot_from_payload(
            Some("codex".to_string()),
            None,
            payload.rate_limit,
            payload.credits,
            Some(payload.plan_type.clone()),
        )),
        additional_snapshots: payload
            .additional_rate_limits
            .unwrap_or_default()
            .into_iter()
            .map(|details| {
                rate_limit_snapshot_from_payload(
                    Some(details.metered_feature),
                    Some(details.limit_name),
                    details.rate_limit,
                    None,
                    Some(payload.plan_type.clone()),
                )
            })
            .collect(),
        message: None,
    }
}

fn rate_limit_snapshot_from_payload(
    limit_id: Option<String>,
    limit_name: Option<String>,
    rate_limit: Option<UpstreamRateLimitStatusDetails>,
    credits: Option<crate::models::UpstreamCreditStatusDetails>,
    plan_type: Option<String>,
) -> ProviderQuotaSnapshot {
    let (primary, secondary) = match rate_limit {
        Some(details) => (
            rate_limit_window_from_payload(details.primary_window),
            rate_limit_window_from_payload(details.secondary_window),
        ),
        None => (None, None),
    };

    ProviderQuotaSnapshot {
        limit_id,
        limit_name,
        primary,
        secondary,
        credits: credits.map(|details| ProviderQuotaCredits {
            has_credits: details.has_credits,
            unlimited: details.unlimited,
            balance: details.balance,
        }),
        plan_type,
    }
}

fn rate_limit_window_from_payload(
    window: Option<UpstreamRateLimitWindowSnapshot>,
) -> Option<ProviderQuotaWindow> {
    let window = window?;
    Some(ProviderQuotaWindow {
        used_percent: f64::from(window.used_percent),
        window_minutes: Some(i64::from(window.limit_window_seconds) / 60),
        resets_at: Some(window.reset_at),
    })
}

#[derive(Clone, Debug)]
struct NativeTarget {
    upstream_model: String,
    upstream: UpstreamProtocol,
    uses_chat_completions: bool,
}

fn resolve_native_target(provider: &ApiProviderRecord, requested_model: &str) -> NativeTarget {
    if provider.uses_chat_completions {
        return NativeTarget {
            upstream_model: requested_model.to_string(),
            upstream: UpstreamProtocol::NativeChatCompletions,
            uses_chat_completions: true,
        };
    }

    NativeTarget {
        upstream_model: requested_model.to_string(),
        upstream: UpstreamProtocol::NativeResponses,
        uses_chat_completions: false,
    }
}

#[derive(Debug)]
struct ChatCompletionsResponsesStream {
    requested_model: String,
    created_at: u64,
    response_id: Option<String>,
    response_model: Option<String>,
    created_emitted: bool,
    message_started: bool,
    finished: bool,
    message_item_id: String,
    text: String,
    tool_calls: BTreeMap<usize, StreamedChatToolCall>,
    usage: Option<Value>,
}

#[derive(Debug)]
struct StreamedChatToolCall {
    id: String,
    item_id: String,
    name: String,
    arguments: String,
}

impl ChatCompletionsResponsesStream {
    fn new(requested_model: String, created_at: u64) -> Self {
        Self {
            requested_model,
            created_at,
            response_id: None,
            response_model: None,
            created_emitted: false,
            message_started: false,
            finished: false,
            message_item_id: format!("msg_{}", Uuid::new_v4().simple()),
            text: String::new(),
            tool_calls: BTreeMap::new(),
            usage: None,
        }
    }

    fn push_chat_payload(&mut self, payload: &str) -> Result<Vec<String>, String> {
        if payload == "[DONE]" {
            return self.finish();
        }

        let chunk: Value = serde_json::from_str(payload).map_err(|err| err.to_string())?;
        if let Some(id) = chunk.get("id").and_then(Value::as_str) {
            self.response_id.get_or_insert_with(|| id.to_string());
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            self.response_model.get_or_insert_with(|| model.to_string());
        }
        if let Some(created) = chunk.get("created").and_then(Value::as_u64) {
            self.created_at = created;
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(chat_usage_to_responses_usage(usage));
        }

        let mut events = self.ensure_created().map_err(|err| err.to_string())?;
        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            return Ok(events);
        };

        for choice in choices {
            if choice
                .get("index")
                .and_then(Value::as_u64)
                .is_some_and(|index| index != 0)
            {
                continue;
            }

            if let Some(delta) = choice.get("delta") {
                if let Some(content) = delta.get("content").and_then(Value::as_str) {
                    if !content.is_empty() {
                        events.extend(
                            self.push_text_delta(content)
                                .map_err(|err| err.to_string())?,
                        );
                    }
                }

                if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    self.push_tool_call_deltas(tool_calls);
                }
            }
        }

        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<String>, String> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;

        let mut events = self.ensure_created().map_err(|err| err.to_string())?;
        if self.message_started {
            let message = self.message_item();
            events.push(
                encode_response_sse_value(&json!({
                    "type": "response.output_text.done",
                    "item_id": self.message_item_id,
                    "output_index": 0,
                    "content_index": 0,
                    "text": self.text
                }))
                .map_err(|err| err.to_string())?,
            );
            events.push(
                encode_response_sse_value(&json!({
                    "type": "response.content_part.done",
                    "item_id": self.message_item_id,
                    "output_index": 0,
                    "content_index": 0,
                    "part": {
                        "type": "output_text",
                        "text": self.text
                    }
                }))
                .map_err(|err| err.to_string())?,
            );
            events.push(
                encode_response_sse_value(&json!({
                    "type": "response.output_item.done",
                    "output_index": 0,
                    "item": message
                }))
                .map_err(|err| err.to_string())?,
            );
        }

        let mut next_output_index = usize::from(self.message_started);
        for tool_call in self.tool_calls.values() {
            let item = tool_call.response_item();
            events.push(
                encode_response_sse_value(&json!({
                    "type": "response.output_item.added",
                    "output_index": next_output_index,
                    "item": item
                }))
                .map_err(|err| err.to_string())?,
            );
            events.push(
                encode_response_sse_value(&json!({
                    "type": "response.output_item.done",
                    "output_index": next_output_index,
                    "item": item
                }))
                .map_err(|err| err.to_string())?,
            );
            next_output_index += 1;
        }

        events.push(
            encode_response_sse_value(&json!({
                "type": "response.completed",
                "response": self.completed_response()
            }))
            .map_err(|err| err.to_string())?,
        );
        events.push("data: [DONE]\n\n".to_string());

        Ok(events)
    }

    fn ensure_created(&mut self) -> Result<Vec<String>, serde_json::Error> {
        if self.created_emitted {
            return Ok(Vec::new());
        }
        self.created_emitted = true;
        encode_response_sse_value(&json!({
            "type": "response.created",
            "response": {
                "id": self.response_id(),
                "object": "response",
                "created_at": self.created_at,
                "status": "in_progress",
                "model": self.response_model(),
                "output": []
            }
        }))
        .map(|event| vec![event])
    }

    fn push_text_delta(&mut self, delta: &str) -> Result<Vec<String>, serde_json::Error> {
        let mut events = Vec::new();
        if !self.message_started {
            self.message_started = true;
            events.push(encode_response_sse_value(&json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "id": self.message_item_id,
                    "type": "message",
                    "role": "assistant",
                    "content": []
                }
            }))?);
            events.push(encode_response_sse_value(&json!({
                "type": "response.content_part.added",
                "item_id": self.message_item_id,
                "output_index": 0,
                "content_index": 0,
                "part": {
                    "type": "output_text",
                    "text": ""
                }
            }))?);
        }

        self.text.push_str(delta);
        events.push(encode_response_sse_value(&json!({
            "type": "response.output_text.delta",
            "item_id": self.message_item_id,
            "output_index": 0,
            "content_index": 0,
            "delta": delta
        }))?);

        Ok(events)
    }

    fn push_tool_call_deltas(&mut self, tool_call_deltas: &[Value]) {
        for tool_call_delta in tool_call_deltas {
            let index = tool_call_delta
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or(self.tool_calls.len() as u64) as usize;
            let tool_call = self
                .tool_calls
                .entry(index)
                .or_insert_with(|| StreamedChatToolCall {
                    id: format!("call_{}", Uuid::new_v4().simple()),
                    item_id: format!("fc_{}", Uuid::new_v4().simple()),
                    name: "unknown".to_string(),
                    arguments: String::new(),
                });

            if let Some(id) = tool_call_delta.get("id").and_then(Value::as_str) {
                tool_call.id = id.to_string();
            }
            let Some(function) = tool_call_delta.get("function") else {
                continue;
            };
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if !name.is_empty() {
                    tool_call.name = name.to_string();
                }
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                tool_call.arguments.push_str(arguments);
            }
        }
    }

    fn completed_response(&mut self) -> Value {
        let mut output = Vec::new();
        if self.message_started {
            output.push(self.message_item());
        }
        output.extend(
            self.tool_calls
                .values()
                .map(StreamedChatToolCall::response_item),
        );

        json!({
            "id": self.response_id(),
            "object": "response",
            "created_at": self.created_at,
            "status": "completed",
            "model": self.response_model(),
            "output": output,
            "usage": self.usage
        })
    }

    fn message_item(&self) -> Value {
        json!({
            "id": self.message_item_id,
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": self.text
            }],
            "phase": "final_answer"
        })
    }

    fn response_id(&mut self) -> String {
        self.response_id
            .get_or_insert_with(|| format!("resp_{}", Uuid::new_v4().simple()))
            .clone()
    }

    fn response_model(&self) -> &str {
        self.response_model
            .as_deref()
            .unwrap_or(&self.requested_model)
    }
}

impl StreamedChatToolCall {
    fn response_item(&self) -> Value {
        json!({
            "id": self.item_id,
            "type": "function_call",
            "name": self.name,
            "arguments": self.arguments,
            "call_id": self.id
        })
    }
}

fn drain_sse_payloads(buffer: &mut String, chunk: &str) -> Vec<String> {
    buffer.push_str(chunk);
    let mut payloads = Vec::new();

    while let Some((boundary_start, boundary_len)) = find_sse_event_boundary(buffer) {
        let frame = buffer[..boundary_start].to_string();
        buffer.drain(..boundary_start + boundary_len);
        if let Some(payload) = sse_payload_from_frame(&frame) {
            payloads.push(payload);
        }
    }

    payloads
}

fn find_sse_event_boundary(buffer: &str) -> Option<(usize, usize)> {
    match (buffer.find("\r\n\r\n"), buffer.find("\n\n")) {
        (Some(crlf), Some(lf)) if crlf < lf => Some((crlf, 4)),
        (Some(_), Some(lf)) => Some((lf, 2)),
        (Some(crlf), None) => Some((crlf, 4)),
        (None, Some(lf)) => Some((lf, 2)),
        (None, None) => None,
    }
}

fn sse_payload_from_frame(frame: &str) -> Option<String> {
    let data_lines: Vec<&str> = frame
        .lines()
        .filter_map(|line| {
            line.trim_end_matches('\r')
                .strip_prefix("data:")
                .map(str::trim_start)
        })
        .collect();

    (!data_lines.is_empty()).then(|| data_lines.join("\n"))
}

fn encode_response_sse_value(value: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(value).map(|body| format!("data: {body}\n\n"))
}

fn chat_usage_to_responses_usage(usage: &Value) -> Value {
    json!({
        "input_tokens": usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        "output_tokens": usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        "total_tokens": usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    })
}

pub(super) async fn log_http_event(
    logs: &LogStore,
    id: &str,
    stage: LogStage,
    status_code: Option<StatusCode>,
    client_protocol: Option<&str>,
    upstream_protocol: Option<&str>,
    provider_name: Option<&str>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    model: Option<&str>,
    stream: bool,
    method: Option<&str>,
    path: Option<&str>,
    url: Option<&str>,
    body: Option<String>,
    error_message: Option<String>,
    elapsed_ms: Option<i64>,
) {
    if let Err(_) = logs
        .record(LogEvent {
            id: id.to_string(),
            stage,
            status_code: status_code.map(|status| status.as_u16()),
            client_protocol: client_protocol.map(ToOwned::to_owned),
            upstream_protocol: upstream_protocol.map(ToOwned::to_owned),
            provider_name: provider_name.map(ToOwned::to_owned),
            account_id: account_id.map(ToOwned::to_owned),
            account_email: account_email.map(ToOwned::to_owned),
            model: model.map(ToOwned::to_owned),
            stream,
            method: method.map(ToOwned::to_owned),
            path: path.map(ToOwned::to_owned),
            url: url.map(ToOwned::to_owned),
            body,
            error_message,
            elapsed_ms,
        })
        .await
    {}
}

fn gateway_error_payload(message: &str) -> Value {
    json!({
        "error": {
            "message": message,
            "type": "proxy_error"
        }
    })
}

pub(super) fn json_value_for_storage(value: &Value) -> String {
    value.to_string()
}

fn capture_final_response_from_sse_chunk(
    buffer: &mut String,
    chunk: &str,
    final_response_body: &mut Option<String>,
) {
    buffer.push_str(chunk);

    while let Some(line_end) = buffer.find('\n') {
        let line: String = buffer.drain(..=line_end).collect();
        capture_final_response_from_sse_line(line.trim_end(), final_response_body);
    }
}

fn capture_final_response_from_sse_line(line: &str, final_response_body: &mut Option<String>) {
    let Some(payload) = line.strip_prefix("data: ") else {
        return;
    };
    if payload == "[DONE]" {
        return;
    }

    let Ok(event) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    capture_final_response_from_event_value(&event, final_response_body);
}

fn capture_final_response_from_event_value(
    event: &Value,
    final_response_body: &mut Option<String>,
) {
    let Some(event_type) = event.get("type").and_then(Value::as_str) else {
        return;
    };
    if matches!(
        event_type,
        "response.completed" | "response.failed" | "response.incomplete"
    ) {
        if let Some(response) = event.get("response") {
            *final_response_body = Some(json_value_for_storage(response));
        }
    }
}

pub(super) fn logged_stream_response_body(
    final_response_body: Option<&str>,
    response_body: &str,
) -> String {
    if let Some(final_body) = final_response_body {
        if extract_model_output_from_body(final_body).is_some() {
            return final_body.to_string();
        }
    }

    response_body.to_string()
}

pub(super) fn elapsed_ms(started_at: Instant) -> i64 {
    started_at.elapsed().as_millis().min(i64::MAX as u128) as i64
}

pub(super) fn append_to_log_buffer(buffer: &mut String, chunk: &str) {
    buffer.push_str(chunk);
}

#[derive(Debug)]
pub struct AppError {
    pub(super) status: StatusCode,
    pub(super) message: String,
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn upstream(error: reqwest::Error) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        }
    }

    fn upstream_message(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "message": self.message,
                    "type": "proxy_error"
                }
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatCompletionsResponsesStream, ResolvedProvider, capture_final_response_from_sse_chunk,
        drain_sse_payloads, logged_stream_response_body, openai_models_response,
        provider_uses_openai_account, quota_from_openai_usage, resolve_native_target,
    };
    use crate::models::{
        ApiProviderBillingMode, ApiProviderRecord, ProviderAuthMode, UpstreamProtocol,
    };
    use serde_json::json;

    #[test]
    fn parses_openai_codex_models_payload() {
        let raw = json!({
            "models": [
                {
                    "slug": "gpt-5.4",
                    "display_name": "GPT-5.4"
                }
            ]
        });

        let response = openai_models_response("openai-proxy", &raw).expect("parse response");

        assert_eq!(response.object, "list");
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].id, "gpt-5.4");
    }

    #[test]
    fn maps_openai_usage_payload_to_gateway_quota_snapshot() {
        let payload = serde_json::from_value(json!({
            "plan_type": "pro",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "used_percent": 42,
                    "limit_window_seconds": 18000,
                    "reset_after_seconds": 120,
                    "reset_at": 1735689720
                },
                "secondary_window": {
                    "used_percent": 5,
                    "limit_window_seconds": 604800,
                    "reset_after_seconds": 3600,
                    "reset_at": 1736294400
                }
            },
            "credits": {
                "has_credits": true,
                "unlimited": false,
                "balance": "9.99"
            },
            "additional_rate_limits": [{
                "limit_name": "codex_other",
                "metered_feature": "codex_other",
                "rate_limit": {
                    "allowed": true,
                    "limit_reached": false,
                    "primary_window": {
                        "used_percent": 88,
                        "limit_window_seconds": 1800,
                        "reset_after_seconds": 600,
                        "reset_at": 1735693200
                    }
                }
            }]
        }))
        .expect("payload should parse");

        let quota = quota_from_openai_usage(payload);

        assert_eq!(
            quota.source,
            crate::models::QuotaSource::ChatgptCodexUsageApi
        );
        assert_eq!(quota.status, crate::models::QuotaSupportStatus::Supported);
        assert_eq!(
            quota
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.limit_id.as_deref()),
            Some("codex")
        );
        assert_eq!(
            quota
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.primary.as_ref())
                .and_then(|window| window.window_minutes),
            Some(300)
        );
        assert_eq!(
            quota
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.secondary.as_ref())
                .and_then(|window| window.window_minutes),
            Some(10080)
        );
        assert_eq!(
            quota
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.credits.as_ref())
                .and_then(|credits| credits.balance.as_deref()),
            Some("9.99")
        );
        assert_eq!(quota.additional_snapshots.len(), 1);
        assert_eq!(
            quota.additional_snapshots[0].limit_id.as_deref(),
            Some("codex_other")
        );
    }

    #[test]
    fn streams_split_chat_completion_sse_as_responses_delta() {
        let mut buffer = String::new();
        let frame = concat!(
            "data: {\"id\":\"chatcmpl_1\",\"created\":1700000000,\"model\":\"chat-model\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hi\"},",
            "\"finish_reason\":null}]}\n\n"
        );
        let split_at = frame.find("\"content\"").expect("split marker exists");

        assert!(drain_sse_payloads(&mut buffer, &frame[..split_at]).is_empty());
        let payloads = drain_sse_payloads(&mut buffer, &frame[split_at..]);
        assert_eq!(payloads.len(), 1);

        let mut stream = ChatCompletionsResponsesStream::new("requested-model".to_string(), 1);
        let events = stream
            .push_chat_payload(&payloads[0])
            .expect("payload should convert");
        let joined = events.join("");

        assert!(joined.contains("\"type\":\"response.output_text.delta\""));
        assert!(joined.contains("\"delta\":\"hi\""));
    }

    #[test]
    fn finishes_chat_completion_sse_with_completed_response() {
        let mut stream = ChatCompletionsResponsesStream::new("requested-model".to_string(), 1);
        stream
            .push_chat_payload(concat!(
                "{\"id\":\"chatcmpl_1\",\"created\":1700000000,\"model\":\"chat-model\",",
                "\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"}}],",
                "\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}"
            ))
            .expect("payload should convert");

        let events = stream.finish().expect("stream should finish");
        let joined = events.join("");

        assert!(joined.contains("\"type\":\"response.completed\""));
        assert!(joined.contains("\"text\":\"done\""));
        assert!(joined.contains("\"input_tokens\":3"));
        assert!(joined.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn treats_named_openai_account_alias_as_openai_provider() {
        let provider = ResolvedProvider {
            name: "xcode-best".to_string(),
            auth_mode: ProviderAuthMode::Account,
            account_id: Some("account-123".to_string()),
            record: Some(ApiProviderRecord {
                id: "provider-123".to_string(),
                name: "xcode-best".to_string(),
                auth_mode: ProviderAuthMode::Account,
                base_url: String::new(),
                api_key: String::new(),
                account_id: Some("account-123".to_string()),
                uses_chat_completions: false,
                billing_mode: ApiProviderBillingMode::Subscription,
            }),
        };

        assert!(provider_uses_openai_account(&provider));
    }

    #[test]
    fn api_provider_uses_responses_by_default_even_for_compatible_provider_name() {
        let provider = ApiProviderRecord {
            id: "provider-123".to_string(),
            name: "compatible-provider".to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: "https://example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            account_id: None,
            uses_chat_completions: false,
            billing_mode: ApiProviderBillingMode::Metered,
        };

        let target = resolve_native_target(&provider, "gpt-5.4");

        assert_eq!(target.upstream, UpstreamProtocol::NativeResponses);
        assert!(!target.uses_chat_completions);
    }

    #[test]
    fn api_provider_uses_chat_completions_only_when_enabled() {
        let provider = ApiProviderRecord {
            id: "provider-123".to_string(),
            name: "custom-compatible".to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: "https://example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            account_id: None,
            uses_chat_completions: true,
            billing_mode: ApiProviderBillingMode::Metered,
        };

        let target = resolve_native_target(&provider, "qwen3-32b");

        assert_eq!(target.upstream, UpstreamProtocol::NativeChatCompletions);
        assert!(target.uses_chat_completions);
        assert_eq!(target.upstream_model, "qwen3-32b");
    }

    #[test]
    fn captures_completed_response_from_sse_for_log_storage() {
        let mut buffer = String::new();
        let mut final_response = None;

        capture_final_response_from_sse_chunk(
            &mut buffer,
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
            &mut final_response,
        );
        assert!(final_response.is_none());

        capture_final_response_from_sse_chunk(
            &mut buffer,
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"output\":[]}}\n\ndata: [DONE]\n\n",
            &mut final_response,
        );

        assert_eq!(
            final_response.as_deref(),
            Some("{\"id\":\"resp_1\",\"output\":[],\"status\":\"completed\"}")
        );
    }

    #[test]
    fn captures_failed_response_from_sse_for_log_storage() {
        let mut buffer = String::new();
        let mut final_response = None;

        capture_final_response_from_sse_chunk(
            &mut buffer,
            "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp_1\",\"status\":\"failed\",\"error\":{\"message\":\"boom\"}}}\n\n",
            &mut final_response,
        );

        assert_eq!(
            final_response.as_deref(),
            Some("{\"error\":{\"message\":\"boom\"},\"id\":\"resp_1\",\"status\":\"failed\"}")
        );
    }

    #[test]
    fn captures_completed_response_from_split_sse_chunk_for_log_storage() {
        let mut buffer = String::new();
        let mut final_response = None;

        capture_final_response_from_sse_chunk(
            &mut buffer,
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_",
            &mut final_response,
        );
        assert!(final_response.is_none());

        capture_final_response_from_sse_chunk(
            &mut buffer,
            "split\",\"status\":\"completed\",\"output\":[]}}\n\n",
            &mut final_response,
        );

        assert_eq!(
            final_response.as_deref(),
            Some("{\"id\":\"resp_split\",\"output\":[],\"status\":\"completed\"}")
        );
    }

    #[test]
    fn keeps_sse_body_when_completed_response_has_no_output_text() {
        let final_response = r#"{"id":"resp_1","status":"completed","output":[]}"#;
        let sse_body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"output\":[]}}\n\n",
            "data: [DONE]\n\n"
        );

        assert_eq!(
            logged_stream_response_body(Some(final_response), sse_body),
            sse_body
        );
    }

    #[test]
    fn keeps_compact_completed_response_when_it_has_output_text() {
        let final_response = r#"{"id":"resp_1","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}]}"#;
        let sse_body = "data: ignored\n\n";

        assert_eq!(
            logged_stream_response_body(Some(final_response), sse_body),
            final_response
        );
    }
}
