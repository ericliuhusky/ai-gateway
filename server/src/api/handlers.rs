use crate::{
    adapters::responses::{
        chat_completions_to_responses, gemini_to_responses, request_with_model,
        responses_to_chat_completions, responses_to_gemini, responses_to_openai_private,
        wrap_v1internal,
    },
    auth::OAuthClient,
    config::Config,
    models::{
        AccountRecord, ApiProviderRecord, ApiProviderSummary, CodexConfigStatus,
        CreateApiProviderRequest, EgressProtocol, GatewayLogDetail, GatewayLogDetailResponse,
        GatewayLogListResponse, GatewayLogSettings, GatewayLogSettingsResponse, GatewayLogSummary,
        IngressProtocol, ModelListItem, ModelListResponse, PROVIDER_GOOGLE_PROXY,
        PROVIDER_OPENAI_PROXY, ProviderAuthMode, ProviderQuotaCredits, ProviderQuotaResponse,
        ProviderQuotaSnapshot, ProviderQuotaSummary, ProviderQuotaWindow, QuotaSource,
        QuotaSupportStatus, ResponsesRequest, ResponsesResponse, SelectedProvider,
        UpdateGatewayLogSettingsRequest, UpdateSelectedModelRequest, UpdateSelectedProviderRequest,
        UpstreamRateLimitStatusDetails, UpstreamRateLimitStatusPayload,
        UpstreamRateLimitWindowSnapshot,
    },
    store::{
        AccountPool, LogEvent, LogStage, LogStore, ModelStore, ProviderStore, RouteStore,
        log_store::extract_model_output_from_body,
    },
    upstream::{
        GOOGLE_PROJECT_ID_FALLBACK, UpstreamClient, chat_completions_api_url, responses_api_url,
    },
};
use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{Form, Host, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
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
use std::time::Instant;
use std::{fs, io::ErrorKind, path::Path, sync::Arc};
use tracing::{info, warn};
use uuid::Uuid;

const BUNDLED_CODEX_CONFIG: &str = include_str!("../../../assets/codex-config.toml");
const MISSING_FILE_SENTINEL: &str = "__AI_GATEWAY_MISSING__";
const RESPONSES_PATH: &str = "/openai/v1/responses";
const OPENAI_PRIVATE_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const DEFAULT_OPENAI_CLIENT_VERSION: &str = "0.122.0";

#[derive(Clone)]
pub struct AppState {
    pub _client: Client,
    pub _config: Arc<Config>,
    pub oauth: OAuthClient,
    pub accounts: AccountPool,
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

pub async fn auth_google_start(
    State(state): State<AppState>,
    Host(host): Host,
    headers: HeaderMap,
) -> Result<Redirect, AppError> {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let callback_url = format!("{scheme}://{host}/auth/google/callback");
    let url = state
        .oauth
        .create_auth_url(callback_url)
        .await
        .map_err(AppError::bad_request)?;
    Ok(Redirect::temporary(&url))
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
        "<html lang='zh-CN'><head><meta charset='utf-8'></head><body style='font-family:sans-serif;padding:32px'><h1>{provider_name} 登录成功</h1><p>账号 <strong>{email}</strong> 已加入代理池。</p><p>你现在可以关闭此页面，并调用 <code>{RESPONSES_PATH}</code>。</p></body></html>"
    ))
}

pub async fn auth_google_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<Html<String>, AppError> {
    if let Some(error) = query.error {
        return Err(AppError::bad_request(format!(
            "google oauth error: {error}"
        )));
    }

    let code = query
        .code
        .ok_or_else(|| AppError::bad_request("missing oauth code"))?;
    let state_token = query
        .state
        .ok_or_else(|| AppError::bad_request("missing oauth state"))?;

    let redirect_uri = state
        .oauth
        .consume_redirect_uri(&state_token)
        .await
        .map_err(AppError::bad_request)?;

    let token = state
        .oauth
        .exchange_code(&code, &redirect_uri)
        .await
        .map_err(AppError::bad_request)?;
    let user = state
        .oauth
        .get_user_info(&token.access_token)
        .await
        .map_err(AppError::bad_request)?;
    let project_id = match state.upstream.fetch_project_id(&token.access_token).await {
        Ok(project_id) => Some(project_id),
        Err(err) => {
            warn!(
                email = %user.email,
                error = %err,
                "Google login continuing without cloudaicompanionProject"
            );
            None
        }
    };
    let account = state
        .accounts
        .add_oauth_account(user, token, project_id)
        .await
        .map_err(AppError::bad_request)?;
    state
        .providers
        .bind_account_provider(PROVIDER_GOOGLE_PROXY, &account.id)
        .await
        .map_err(AppError::bad_request)?;

    Ok(auth_success_html("Google", &account.email))
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
        .bind_account_provider(PROVIDER_OPENAI_PROXY, &account.id)
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
        .bind_account_provider(PROVIDER_OPENAI_PROXY, &account.id)
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
        if provider_uses_openai_account(&provider) {
            let account = resolve_account_for_provider(&state, &provider).await?;
            let raw = state
                .upstream
                .fetch_openai_usage(
                    &format!("quota_{}", Uuid::new_v4().simple()),
                    account.access_token(),
                    account.upstream_account_id(),
                )
                .await
                .map_err(AppError::upstream_message)?;
            let payload: UpstreamRateLimitStatusPayload = serde_json::from_value(raw)
                .map_err(|err| AppError::upstream_message(err.to_string()))?;
            quota_from_openai_usage(payload)
        } else {
            unsupported_quota_summary(format!(
                "official quota snapshot is not supported yet for account provider `{}`",
                provider.name
            ))
        }
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
        backup_or_mark_missing(&target_path, &backup_path, "CodeX config")?;
    }

    if !auth_backup_path.exists() {
        backup_or_mark_missing(&auth_path, &auth_backup_path, "CodeX auth")?;
    }

    fs::write(&target_path, BUNDLED_CODEX_CONFIG)
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

pub async fn responses(
    State(state): State<AppState>,
    Json(mut request): Json<ResponsesRequest>,
) -> Result<Response, AppError> {
    let id = Uuid::new_v4().simple().to_string();
    let started_at = Instant::now();
    apply_selected_model_override(&state, &mut request).await;
    log_http_event(
        &state.logs,
        &id,
        LogStage::IngressRequest,
        None,
        Some(IngressProtocol::OpenAiResponses.as_str()),
        None,
        None,
        None,
        None,
        Some(&request.model),
        request.stream,
        Some("POST"),
        Some(RESPONSES_PATH),
        None,
        Some(json_for_storage(&request)),
        None,
        None,
    )
    .await;

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
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                None,
                None,
                None,
                Some(&model),
                stream,
                Some("POST"),
                Some(RESPONSES_PATH),
                None,
                Some(json_value_for_storage(&error_body)),
                Some(err.message.clone()),
                Some(elapsed),
            )
            .await;
            log_http_event(
                &state.logs,
                &id,
                LogStage::EgressResponse,
                Some(err.status),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                None,
                None,
                None,
                Some(&model),
                stream,
                Some("POST"),
                Some(RESPONSES_PATH),
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
        ingress_protocol: log.ingress_protocol,
        egress_protocol: log.egress_protocol,
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
        ingress_protocol: log.ingress_protocol,
        egress_protocol: log.egress_protocol,
        method: log.method,
        path: log.path,
        egress_request_url: log.egress_request_url,
        ingress_request_body: log.ingress_request_body,
        ingress_request_body_truncated: log.ingress_request_body_truncated,
        egress_request_body: log.egress_request_body,
        egress_request_body_truncated: log.egress_request_body_truncated,
        ingress_response_status_code: log.ingress_response_status_code,
        ingress_response_body: log.ingress_response_body,
        ingress_response_body_truncated: log.ingress_response_body_truncated,
        egress_response_status_code: log.egress_response_status_code,
        egress_response_body: log.egress_response_body,
        egress_response_body_truncated: log.egress_response_body_truncated,
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

async fn responses_inner(
    state: AppState,
    request: ResponsesRequest,
    id: String,
    started_at: Instant,
) -> Result<Response, AppError> {
    let provider = resolve_selected_provider(&state).await?;
    info!(
        id = %id,
        ingress = IngressProtocol::OpenAiResponses.as_str(),
        provider = %provider.name,
        body = %json_for_log(&request),
        "received /openai/v1/responses request"
    );

    if provider.auth_mode == ProviderAuthMode::Account && provider_uses_openai_account(&provider) {
        let account = resolve_account_for_provider(&state, &provider).await?;
        let request_body = responses_to_openai_private(&request)
            .map_err(|err| AppError::internal(err.to_string()))?;

        log_http_event(
            &state.logs,
            &id,
            LogStage::EgressRequest,
            None,
            Some(IngressProtocol::OpenAiResponses.as_str()),
            Some(EgressProtocol::OpenAiPrivateResponses.as_str()),
            Some(&provider.name),
            Some(&account.id),
            Some(&account.email),
            Some(&request.model),
            request.stream,
            Some("POST"),
            None,
            Some(OPENAI_PRIVATE_RESPONSES_URL),
            Some(json_value_for_storage(&request_body)),
            None,
            None,
        )
        .await;

        let upstream = state
            .upstream
            .call_openai_responses(
                &id,
                account.access_token(),
                account.upstream_account_id(),
                request_body,
                request.stream,
            )
            .await
            .map_err(AppError::upstream_message)?;
        let upstream_status = upstream.status();

        if !request.stream {
            let response_body: Value = upstream.json().await.map_err(AppError::upstream)?;
            let elapsed = elapsed_ms(started_at);
            let stored_body = json_value_for_storage(&response_body);
            info!(
                id = %id,
                elapsed_ms = elapsed,
                email = %account.email,
                provider = %provider.name,
                egress = %EgressProtocol::OpenAiPrivateResponses.as_str(),
                response_body = %json_value_for_log(&response_body),
                "returning OpenAI /openai/v1/responses body"
            );
            log_http_event(
                &state.logs,
                &id,
                LogStage::IngressResponse,
                Some(upstream_status),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                Some(EgressProtocol::OpenAiPrivateResponses.as_str()),
                Some(&provider.name),
                Some(&account.id),
                Some(&account.email),
                Some(&request.model),
                false,
                Some("POST"),
                None,
                Some(OPENAI_PRIVATE_RESPONSES_URL),
                Some(stored_body.clone()),
                None,
                Some(elapsed),
            )
            .await;
            log_http_event(
                &state.logs,
                &id,
                LogStage::EgressResponse,
                Some(StatusCode::OK),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider.name),
                Some(&account.id),
                Some(&account.email),
                Some(&request.model),
                false,
                Some("POST"),
                Some(RESPONSES_PATH),
                None,
                Some(stored_body),
                None,
                Some(elapsed),
            )
            .await;
            return Ok((
                StatusCode::OK,
                [
                    ("x-account-email", account.email.as_str()),
                    ("x-provider", provider.name.as_str()),
                ],
                Json(response_body),
            )
                .into_response());
        }

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
                            Some(IngressProtocol::OpenAiResponses.as_str()),
                            Some(EgressProtocol::OpenAiPrivateResponses.as_str()),
                            Some(&provider_name),
                            Some(&account_id),
                            Some(&account_email),
                            Some(&model),
                            true,
                            Some("POST"),
                            Some(RESPONSES_PATH),
                            Some(OPENAI_PRIVATE_RESPONSES_URL),
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
                LogStage::IngressResponse,
                Some(upstream_status),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                Some(EgressProtocol::OpenAiPrivateResponses.as_str()),
                Some(&provider_name),
                Some(&account_id),
                Some(&account_email),
                Some(&model),
                true,
                Some("POST"),
                None,
                Some(OPENAI_PRIVATE_RESPONSES_URL),
                Some(logged_response_body.clone()),
                None,
                Some(elapsed),
            )
            .await;
            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::EgressResponse,
                Some(StatusCode::OK),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider_name),
                Some(&account_id),
                Some(&account_email),
                Some(&model),
                true,
                Some("POST"),
                Some(RESPONSES_PATH),
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

    if provider.auth_mode == ProviderAuthMode::Account && provider.name == PROVIDER_GOOGLE_PROXY {
        let gemini_request = responses_to_gemini(&request).map_err(AppError::bad_request)?;
        let account = resolve_account_for_provider(&state, &provider).await?;
        let project_id = google_project_id_for_request(&account).to_string();
        if account.project_id().is_none() {
            warn!(
                account_id = %account.id,
                email = %account.email,
                fallback_project_id = %project_id,
                "selected Google account has no cloudaicompanionProject; using fallback project"
            );
        }
        let request_body = wrap_v1internal(
            serde_json::to_value(&gemini_request)
                .map_err(|err| AppError::internal(err.to_string()))?,
            &project_id,
            &request.model,
            &account.id,
        );
        let method = if request.stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let upstream_url = google_v1internal_url_label(method, request.stream);

        info!(
            id = %id,
            model = %request.model,
            stream = request.stream,
            email = %account.email,
            project_id = %project_id,
            egress = %EgressProtocol::GoogleV1Internal.as_str(),
            gemini_request = %json_for_log(&gemini_request),
            upstream_request = %json_value_for_log(&request_body),
            "proxying request to Gemini upstream"
        );

        log_http_event(
            &state.logs,
            &id,
            LogStage::EgressRequest,
            None,
            Some(IngressProtocol::OpenAiResponses.as_str()),
            Some(EgressProtocol::GoogleV1Internal.as_str()),
            Some(&provider.name),
            Some(&account.id),
            Some(&account.email),
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

        let upstream = state
            .upstream
            .call_v1internal(
                method,
                &id,
                account.access_token(),
                request_body,
                request.stream,
            )
            .await
            .map_err(AppError::upstream_message)?;
        let upstream_status = upstream.status();

        if !request.stream {
            let gemini_body: Value = upstream.json().await.map_err(AppError::upstream)?;
            let response = gemini_to_responses(&request.model, &gemini_body);
            let elapsed = elapsed_ms(started_at);
            let upstream_body = json_value_for_storage(&gemini_body);
            let response_body = json_for_storage(&response);
            info!(
                id = %id,
                elapsed_ms = elapsed,
                upstream_response = %json_value_for_log(&gemini_body),
                "received non-stream Gemini response"
            );
            info!(
                id = %id,
                elapsed_ms = elapsed,
                output_items = response.output.len(),
                response_body = %json_for_log(&response),
                "returning non-stream /openai/v1/responses body"
            );
            log_http_event(
                &state.logs,
                &id,
                LogStage::IngressResponse,
                Some(upstream_status),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                Some(EgressProtocol::GoogleV1Internal.as_str()),
                Some(&provider.name),
                Some(&account.id),
                Some(&account.email),
                Some(&request.model),
                false,
                Some("POST"),
                None,
                Some(&upstream_url),
                Some(upstream_body),
                None,
                Some(elapsed),
            )
            .await;
            log_http_event(
                &state.logs,
                &id,
                LogStage::EgressResponse,
                Some(StatusCode::OK),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider.name),
                Some(&account.id),
                Some(&account.email),
                Some(&request.model),
                false,
                Some("POST"),
                Some(RESPONSES_PATH),
                None,
                Some(response_body),
                None,
                Some(elapsed),
            )
            .await;
            return Ok((
                StatusCode::OK,
                [
                    ("x-account-email", account.email.as_str()),
                    ("x-provider", provider.name.as_str()),
                ],
                Json(response),
            )
                .into_response());
        }

        let model = request.model.clone();
        let id_for_stream = id.clone();
        let provider_name = provider.name.clone();
        let account_id = account.id.clone();
        let account_email = account.email.clone();
        let logs = state.logs.clone();
        let output = stream! {
            let encode_event = |value: &Value| -> Result<String, std::io::Error> {
                serde_json::to_string(value)
                    .map(|body| format!("data: {body}\n\n"))
                    .map_err(std::io::Error::other)
            };
            let mut stream = upstream.bytes_stream();
            let mut buffer = String::new();
            let mut upstream_body = String::new();
            let mut final_upstream_body: Option<String> = None;
            let mut client_body = String::new();
            let response_id = format!("resp_{}", Uuid::new_v4().simple());
            let message_item_id = format!("msg_{}", Uuid::new_v4().simple());
            let mut message_item_started = false;
            let mut accumulated_text = String::new();
            let mut completed_output_items: Vec<Value> = Vec::new();
            let mut emitted_tool_calls = std::collections::HashSet::new();
            let mut upstream_chunk_count = 0usize;
            let mut emitted_event_count = 0usize;

            let created = json!({
                "type": "response.created",
                "response": {
                    "id": &response_id,
                    "object": "response",
                    "status": "in_progress",
                    "output": []
                }
            });
            match encode_event(&created) {
                Ok(event) => {
                    append_to_log_buffer(&mut client_body, &event);
                    emitted_event_count += 1;
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                }
                Err(err) => {
                    let error_message = err.to_string();
                    log_http_event(
                        &logs,
                        &id_for_stream,
                        LogStage::Error,
                        Some(StatusCode::INTERNAL_SERVER_ERROR),
                        Some(IngressProtocol::OpenAiResponses.as_str()),
                        Some(EgressProtocol::GoogleV1Internal.as_str()),
                        Some(&provider_name),
                        Some(&account_id),
                        Some(&account_email),
                        Some(&model),
                        true,
                        Some("POST"),
                        Some(RESPONSES_PATH),
                        Some(&upstream_url),
                        Some(client_body.clone()),
                        Some(error_message),
                        Some(elapsed_ms(started_at)),
                    )
                    .await;
                    yield Err(err);
                    return;
                }
            }

            while let Some(result) = stream.next().await {
                let chunk = match result {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        let error_message = err.to_string();
                        log_http_event(
                            &logs,
                            &id_for_stream,
                            LogStage::Error,
                            Some(StatusCode::BAD_GATEWAY),
                            Some(IngressProtocol::OpenAiResponses.as_str()),
                            Some(EgressProtocol::GoogleV1Internal.as_str()),
                            Some(&provider_name),
                            Some(&account_id),
                            Some(&account_email),
                            Some(&model),
                            true,
                            Some("POST"),
                            Some(RESPONSES_PATH),
                            Some(&upstream_url),
                            Some(upstream_body.clone()),
                            Some(error_message),
                            Some(elapsed_ms(started_at)),
                        )
                        .await;
                        yield Err(std::io::Error::other(err));
                        return;
                    }
                };

                let chunk_text = String::from_utf8_lossy(&chunk);
                append_to_log_buffer(&mut upstream_body, &chunk_text);
                buffer.push_str(&chunk_text);

                while let Some(line_end) = buffer.find('\n') {
                    let line: String = buffer.drain(..=line_end).collect();
                    let line = line.trim();
                    if !line.starts_with("data: ") {
                        continue;
                    }

                    let payload = &line[6..];
                    if payload == "[DONE]" {
                        continue;
                    }

                    upstream_chunk_count += 1;
                    info!(
                        id = %id_for_stream,
                        chunk_index = upstream_chunk_count,
                        upstream_chunk = %truncate_for_log(payload, 3000),
                        "received upstream SSE chunk"
                    );

                    let gemini_event: Value = match serde_json::from_str(payload) {
                        Ok(value) => value,
                        Err(err) => {
                            warn!(
                                id = %id_for_stream,
                                chunk_index = upstream_chunk_count,
                                error = %err,
                                "failed to parse upstream SSE chunk"
                            );
                            continue;
                        }
                    };
                    final_upstream_body = Some(json_value_for_storage(&gemini_event));

                    let raw = gemini_event.get("response").unwrap_or(&gemini_event);
                    let candidate = raw
                        .get("candidates")
                        .and_then(Value::as_array)
                        .and_then(|candidates| candidates.first());

                    if let Some(parts) = candidate
                        .and_then(|candidate| candidate.get("content"))
                        .and_then(|content| content.get("parts"))
                        .and_then(Value::as_array)
                    {
                        for part in parts {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                if !text.is_empty() {
                                    if !message_item_started {
                                        let output_item_added = json!({
                                            "type": "response.output_item.added",
                                            "output_index": 0,
                                            "item": {
                                                "id": &message_item_id,
                                                "type": "message",
                                                "role": "assistant",
                                                "status": "in_progress",
                                                "content": []
                                            }
                                        });
                                        match encode_event(&output_item_added) {
                                            Ok(event) => {
                                                append_to_log_buffer(&mut client_body, &event);
                                                emitted_event_count += 1;
                                                yield Ok(Bytes::from(event));
                                            }
                                            Err(err) => {
                                                yield Err(err);
                                                return;
                                            }
                                        }

                                        let content_part_added = json!({
                                            "type": "response.content_part.added",
                                            "item_id": &message_item_id,
                                            "output_index": 0,
                                            "content_index": 0,
                                            "part": {
                                                "type": "output_text",
                                                "text": ""
                                            }
                                        });
                                        match encode_event(&content_part_added) {
                                            Ok(event) => {
                                                append_to_log_buffer(&mut client_body, &event);
                                                emitted_event_count += 1;
                                                yield Ok(Bytes::from(event));
                                            }
                                            Err(err) => {
                                                yield Err(err);
                                                return;
                                            }
                                        }
                                        message_item_started = true;
                                    }

                                    accumulated_text.push_str(text);
                                    let delta = json!({
                                        "type": "response.output_text.delta",
                                        "item_id": &message_item_id,
                                        "output_index": 0,
                                        "content_index": 0,
                                        "delta": text
                                    });
                                    match encode_event(&delta) {
                                        Ok(event) => {
                                            append_to_log_buffer(&mut client_body, &event);
                                            emitted_event_count += 1;
                                            yield Ok(Bytes::from(event));
                                        }
                                        Err(err) => {
                                            yield Err(err);
                                            return;
                                        }
                                    }
                                }
                            }

                            if let Some(function_call) = part.get("functionCall") {
                                let call_key = function_call
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned)
                                    .unwrap_or_else(|| function_call.to_string());

                                if emitted_tool_calls.insert(call_key.clone()) {
                                    let call_id = function_call
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .map(ToOwned::to_owned)
                                        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4().simple()));
                                    let tool_item = json!({
                                        "id": format!("fc_{}", Uuid::new_v4().simple()),
                                        "type": "function_call",
                                        "call_id": call_id,
                                        "name": function_call
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .unwrap_or("unknown"),
                                        "arguments": function_call
                                            .get("args")
                                            .map(Value::to_string)
                                            .unwrap_or_else(|| "{}".to_string()),
                                        "status": "completed"
                                    });
                                    let added = json!({
                                        "type": "response.output_item.added",
                                        "output_index": completed_output_items.len(),
                                        "item": tool_item
                                    });
                                    match encode_event(&added) {
                                        Ok(event) => {
                                            append_to_log_buffer(&mut client_body, &event);
                                            emitted_event_count += 1;
                                            yield Ok(Bytes::from(event));
                                        }
                                        Err(err) => {
                                            yield Err(err);
                                            return;
                                        }
                                    }

                                    let done = json!({
                                        "type": "response.output_item.done",
                                        "output_index": completed_output_items.len(),
                                        "item": added["item"].clone()
                                    });
                                    match encode_event(&done) {
                                        Ok(event) => {
                                            append_to_log_buffer(&mut client_body, &event);
                                            emitted_event_count += 1;
                                            yield Ok(Bytes::from(event));
                                        }
                                        Err(err) => {
                                            yield Err(err);
                                            return;
                                        }
                                    }
                                    completed_output_items.push(added["item"].clone());
                                }
                            }
                        }
                    }
                }
            }

            if message_item_started {
                let text_done = json!({
                    "type": "response.output_text.done",
                    "item_id": &message_item_id,
                    "output_index": 0,
                    "content_index": 0,
                    "text": &accumulated_text
                });
                match encode_event(&text_done) {
                    Ok(event) => {
                        append_to_log_buffer(&mut client_body, &event);
                        emitted_event_count += 1;
                        yield Ok(Bytes::from(event));
                    }
                    Err(err) => {
                        yield Err(err);
                        return;
                    }
                }

                let content_part_done = json!({
                    "type": "response.content_part.done",
                    "item_id": &message_item_id,
                    "output_index": 0,
                    "content_index": 0,
                    "part": {
                        "type": "output_text",
                        "text": &accumulated_text
                    }
                });
                match encode_event(&content_part_done) {
                    Ok(event) => {
                        append_to_log_buffer(&mut client_body, &event);
                        emitted_event_count += 1;
                        yield Ok(Bytes::from(event));
                    }
                    Err(err) => {
                        yield Err(err);
                        return;
                    }
                }

                let message_item = json!({
                    "id": &message_item_id,
                    "type": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "text": &accumulated_text
                    }]
                });
                let output_index = completed_output_items.len();
                let output_item_done = json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": message_item
                });
                match encode_event(&output_item_done) {
                    Ok(event) => {
                        append_to_log_buffer(&mut client_body, &event);
                        emitted_event_count += 1;
                        yield Ok(Bytes::from(event));
                    }
                    Err(err) => {
                        yield Err(err);
                        return;
                    }
                }
                completed_output_items.push(message_item);
            }

            let completed = json!({
                "type": "response.completed",
                "response": {
                    "id": &response_id,
                    "object": "response",
                    "status": "completed",
                    "model": &model,
                    "output": completed_output_items
                }
            });
            let final_client_body = completed
                .get("response")
                .map(json_value_for_storage)
                .unwrap_or_else(|| client_body.clone());
            match encode_event(&completed) {
                Ok(event) => {
                    append_to_log_buffer(&mut client_body, &event);
                    emitted_event_count += 1;
                    yield Ok(Bytes::from(event));
                }
                Err(err) => {
                    yield Err(err);
                    return;
                }
            }

            info!(
                id = %id_for_stream,
                elapsed_ms = started_at.elapsed().as_millis(),
                upstream_chunks = upstream_chunk_count,
                emitted_events = emitted_event_count,
                output_items = completed_output_items.len(),
                text_len = accumulated_text.len(),
                "completed streaming /openai/v1/responses request"
            );

            let done = "data: [DONE]\n\n".to_string();
            append_to_log_buffer(&mut client_body, &done);
            let elapsed = elapsed_ms(started_at);
            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::IngressResponse,
                Some(upstream_status),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                Some(EgressProtocol::GoogleV1Internal.as_str()),
                Some(&provider_name),
                Some(&account_id),
                Some(&account_email),
                Some(&model),
                true,
                Some("POST"),
                None,
                Some(&upstream_url),
                Some(final_upstream_body.unwrap_or(upstream_body)),
                None,
                Some(elapsed),
            )
            .await;
            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::EgressResponse,
                Some(StatusCode::OK),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider_name),
                Some(&account_id),
                Some(&account_email),
                Some(&model),
                true,
                Some("POST"),
                Some(RESPONSES_PATH),
                None,
                Some(final_client_body),
                None,
                Some(elapsed),
            )
            .await;
            yield Ok(Bytes::from(done));
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
            LogStage::EgressRequest,
            None,
            Some(IngressProtocol::OpenAiResponses.as_str()),
            Some(native_target.egress.as_str()),
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

        let upstream = state
            .upstream
            .call_openai_chat_upstream(
                &id,
                &native_provider.base_url,
                &native_provider.api_key,
                request_body,
            )
            .await
            .map_err(AppError::upstream_message)?;
        let upstream_status = upstream.status();
        let chat_body: Value = upstream.json().await.map_err(AppError::upstream)?;
        let response = chat_completions_to_responses(&request.model, &chat_body);
        let elapsed = elapsed_ms(started_at);
        let upstream_body = json_value_for_storage(&chat_body);

        log_http_event(
            &state.logs,
            &id,
            LogStage::IngressResponse,
            Some(upstream_status),
            Some(IngressProtocol::OpenAiResponses.as_str()),
            Some(native_target.egress.as_str()),
            Some(&provider.name),
            None,
            None,
            Some(&request.model),
            request.stream,
            Some("POST"),
            None,
            Some(&upstream_url),
            Some(upstream_body),
            None,
            Some(elapsed),
        )
        .await;

        if !request.stream {
            let response_body = json_for_storage(&response);
            info!(
                id = %id,
                elapsed_ms = elapsed,
                provider = %provider.name,
                egress = %native_target.egress.as_str(),
                upstream_model = %native_target.upstream_model,
                response_body = %json_for_log(&response),
                "returning chat-completions-adapted /openai/v1/responses body"
            );
            log_http_event(
                &state.logs,
                &id,
                LogStage::EgressResponse,
                Some(StatusCode::OK),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider.name),
                None,
                None,
                Some(&request.model),
                false,
                Some("POST"),
                Some(RESPONSES_PATH),
                None,
                Some(response_body),
                None,
                Some(elapsed),
            )
            .await;
            return Ok((
                StatusCode::OK,
                [("x-provider", provider.name.as_str())],
                Json(response),
            )
                .into_response());
        }

        let logs = state.logs.clone();
        let id_for_stream = id.clone();
        let provider_name = provider.name.clone();
        let model = request.model.clone();
        let egress_protocol = native_target.egress.as_str().to_string();
        let upstream_url_for_stream = upstream_url.clone();
        let final_response_body = json_for_storage(&response);
        let output = stream! {
            let stream = synthesized_responses_stream(response);
            futures_util::pin_mut!(stream);
            let mut response_body = String::new();

            while let Some(result) = stream.next().await {
                match result {
                    Ok(event) => {
                        append_to_log_buffer(&mut response_body, &event);
                        yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                    }
                    Err(err) => {
                        log_http_event(
                            &logs,
                            &id_for_stream,
                            LogStage::Error,
                            Some(StatusCode::INTERNAL_SERVER_ERROR),
                            Some(IngressProtocol::OpenAiResponses.as_str()),
                            Some(&egress_protocol),
                            Some(&provider_name),
                            None,
                            None,
                            Some(&model),
                            true,
                            Some("POST"),
                            Some(RESPONSES_PATH),
                            Some(&upstream_url_for_stream),
                            Some(response_body.clone()),
                            Some(err.to_string()),
                            Some(elapsed_ms(started_at)),
                        )
                        .await;
                        yield Err(err);
                        return;
                    }
                }
            }

            log_http_event(
                &logs,
                &id_for_stream,
                LogStage::EgressResponse,
                Some(StatusCode::OK),
                Some(IngressProtocol::OpenAiResponses.as_str()),
                None,
                Some(&provider_name),
                None,
                None,
                Some(&model),
                true,
                Some("POST"),
                Some(RESPONSES_PATH),
                None,
                Some(final_response_body),
                None,
                Some(elapsed_ms(started_at)),
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
        LogStage::EgressRequest,
        None,
        Some(IngressProtocol::OpenAiResponses.as_str()),
        Some(native_target.egress.as_str()),
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

    let upstream = state
        .upstream
        .call_openai_responses_upstream(
            &id,
            &native_provider.base_url,
            &native_provider.api_key,
            request_body,
            request.stream,
        )
        .await
        .map_err(AppError::upstream_message)?;
    let upstream_status = upstream.status();

    if !request.stream {
        let response_body: Value = upstream.json().await.map_err(AppError::upstream)?;
        let elapsed = elapsed_ms(started_at);
        let stored_body = json_value_for_storage(&response_body);
        info!(
            id = %id,
            elapsed_ms = elapsed,
            provider = %provider.name,
            egress = %native_target.egress.as_str(),
            upstream_model = %native_target.upstream_model,
            response_body = %json_value_for_log(&response_body),
            "returning native provider /openai/v1/responses body"
        );
        log_http_event(
            &state.logs,
            &id,
            LogStage::IngressResponse,
            Some(upstream_status),
            Some(IngressProtocol::OpenAiResponses.as_str()),
            Some(native_target.egress.as_str()),
            Some(&provider.name),
            None,
            None,
            Some(&request.model),
            false,
            Some("POST"),
            None,
            Some(&upstream_url),
            Some(stored_body.clone()),
            None,
            Some(elapsed),
        )
        .await;
        log_http_event(
            &state.logs,
            &id,
            LogStage::EgressResponse,
            Some(StatusCode::OK),
            Some(IngressProtocol::OpenAiResponses.as_str()),
            None,
            Some(&provider.name),
            None,
            None,
            Some(&request.model),
            false,
            Some("POST"),
            Some(RESPONSES_PATH),
            None,
            Some(stored_body),
            None,
            Some(elapsed),
        )
        .await;
        return Ok((
            StatusCode::OK,
            [("x-provider", provider.name.as_str())],
            Json(response_body),
        )
            .into_response());
    }

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
                        Some(IngressProtocol::OpenAiResponses.as_str()),
                        Some(native_target.egress.as_str()),
                        Some(&provider_name),
                        None,
                        None,
                        Some(&model),
                        true,
                        Some("POST"),
                        Some(RESPONSES_PATH),
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
            LogStage::IngressResponse,
            Some(upstream_status),
            Some(IngressProtocol::OpenAiResponses.as_str()),
            Some(native_target.egress.as_str()),
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
            LogStage::EgressResponse,
            Some(StatusCode::OK),
            Some(IngressProtocol::OpenAiResponses.as_str()),
            None,
            Some(&provider_name),
            None,
            None,
            Some(&model),
            true,
            Some("POST"),
            Some(RESPONSES_PATH),
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

async fn resolve_selected_provider(state: &AppState) -> Result<ResolvedProvider, AppError> {
    let route = state.routes.get().await;
    if let Some(provider_id) = route.provider_id {
        return resolve_provider_by_id(state, &provider_id).await;
    }

    Err(AppError::bad_request(
        "no provider selected; call PUT /selected-provider first",
    ))
}

fn route_payload(route: SelectedProvider) -> Value {
    json!({
        "provider_id": route.provider_id,
        "selected_model": route.selected_model,
        "updated_at": route.updated_at,
    })
}

async fn apply_selected_model_override(state: &AppState, request: &mut ResponsesRequest) {
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
        if provider.name == PROVIDER_GOOGLE_PROXY {
            let raw = state
                .upstream
                .fetch_google_available_models(account.access_token(), account.project_id())
                .await
                .map_err(AppError::upstream_message)?;
            return google_models_response(&provider.name, &raw);
        }
        if provider.name == PROVIDER_OPENAI_PROXY {
            let client_version = openai_client_version(state);
            let raw = state
                .upstream
                .fetch_openai_models(
                    &format!("models_{}", Uuid::new_v4().simple()),
                    account.access_token(),
                    account.upstream_account_id(),
                    &client_version,
                )
                .await
                .map_err(AppError::upstream_message)?;
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
    let raw = state
        .upstream
        .fetch_openai_models_upstream(
            &format!("models_{}", Uuid::new_v4().simple()),
            &native_provider.base_url,
            &native_provider.api_key,
        )
        .await
        .map_err(AppError::upstream_message)?;
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

fn backup_or_mark_missing(source: &Path, backup: &Path, label: &str) -> Result<(), AppError> {
    if source.exists() {
        fs::copy(source, backup)
            .map_err(|err| AppError::bad_request(format!("failed to back up {label}: {err}")))?;
    } else {
        fs::write(backup, MISSING_FILE_SENTINEL)
            .map_err(|err| AppError::bad_request(format!("failed to back up {label}: {err}")))?;
    }

    Ok(())
}

fn restore_or_remove_backup(backup: &Path, target: &Path, label: &str) -> Result<(), AppError> {
    let backup_contents = fs::read(backup)
        .map_err(|err| AppError::bad_request(format!("failed to read {label} backup: {err}")))?;

    if backup_contents == MISSING_FILE_SENTINEL.as_bytes() {
        if target.exists() {
            fs::remove_file(target).map_err(|err| {
                AppError::bad_request(format!("failed to remove {label} file: {err}"))
            })?;
        }
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                AppError::bad_request(format!("failed to create {label} directory: {err}"))
            })?;
        }
        fs::write(target, backup_contents)
            .map_err(|err| AppError::bad_request(format!("failed to restore {label}: {err}")))?;
    }

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

fn google_models_response(_provider: &str, raw: &Value) -> Result<ModelListResponse, AppError> {
    let models = raw
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::upstream_message("google models payload missing `models`"))?;

    let mut data = Vec::with_capacity(models.len());
    for (id, meta) in models {
        let _ = meta;
        data.push(ModelListItem { id: id.clone() });
    }
    data.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(ModelListResponse {
        object: "list".to_string(),
        data,
    })
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

#[derive(Debug, Deserialize)]
struct CodexModelsCacheVersion {
    client_version: Option<String>,
}

fn openai_client_version(state: &AppState) -> String {
    let cache_path = state._config.codex_models_cache_path();
    let parsed = fs::read_to_string(&cache_path)
        .ok()
        .and_then(|content| serde_json::from_str::<CodexModelsCacheVersion>(&content).ok())
        .and_then(|cache| cache.client_version)
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty());

    parsed.unwrap_or_else(|| DEFAULT_OPENAI_CLIENT_VERSION.to_string())
}

fn native_model_id(entry: &Value) -> Option<&str> {
    entry
        .get("id")
        .or_else(|| entry.get("model"))
        .or_else(|| entry.get("name"))
        .and_then(Value::as_str)
}

#[derive(Clone, Debug)]
struct ResolvedProvider {
    name: String,
    auth_mode: ProviderAuthMode,
    account_id: Option<String>,
    record: Option<ApiProviderRecord>,
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

async fn resolve_account_for_provider(
    state: &AppState,
    provider: &ResolvedProvider,
) -> Result<AccountRecord, AppError> {
    if let Some(account_id) = provider.account_id.as_deref() {
        return state
            .accounts
            .acquire_by_id(&state.oauth, &state.upstream, account_id)
            .await
            .map_err(AppError::bad_request);
    }

    state
        .accounts
        .acquire_for_provider(&state.oauth, &state.upstream, &provider.name)
        .await
        .map_err(AppError::bad_request)
}

fn provider_uses_openai_account(provider: &ResolvedProvider) -> bool {
    provider
        .record
        .as_ref()
        .and_then(|record| record.account_id.as_ref())
        .is_some()
        && provider.name != PROVIDER_GOOGLE_PROXY
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
        api_key_preview: if record.api_key.is_empty() {
            "********".to_string()
        } else {
            let prefix = &record.api_key[..record.api_key.len().min(4)];
            let suffix_start = record.api_key.len().saturating_sub(4);
            let suffix = &record.api_key[suffix_start..];
            if record.api_key.len() <= 8 {
                "********".to_string()
            } else {
                format!("{prefix}...{suffix}")
            }
        },
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
    egress: EgressProtocol,
    uses_chat_completions: bool,
}

fn resolve_native_target(provider: &ApiProviderRecord, requested_model: &str) -> NativeTarget {
    if provider.uses_chat_completions {
        return NativeTarget {
            upstream_model: requested_model.to_string(),
            egress: EgressProtocol::NativeChatCompletions,
            uses_chat_completions: true,
        };
    }

    NativeTarget {
        upstream_model: requested_model.to_string(),
        egress: EgressProtocol::NativeResponses,
        uses_chat_completions: false,
    }
}

fn synthesized_responses_stream(
    response: ResponsesResponse,
) -> impl futures_util::Stream<Item = Result<String, std::io::Error>> {
    stream! {
        let response_value = match serde_json::to_value(&response) {
            Ok(value) => value,
            Err(err) => {
                yield Err(std::io::Error::other(err));
                return;
            }
        };
        yield Ok(format!("data: {}\n\n", json!({
            "type": "response.created",
            "response": {
                "id": &response.id,
                "object": "response",
                "status": "in_progress",
                "output": []
            }
        })));

        for (index, item) in response.output.iter().enumerate() {
            yield Ok(format!("data: {}\n\n", json!({
                "type": "response.output_item.added",
                "output_index": index,
                "item": item.clone()
            })));

            if item.item_type == "message" {
                if let Some(content) = &item.content {
                    for (content_index, part) in content.iter().enumerate() {
                        yield Ok(format!("data: {}\n\n", json!({
                            "type": "response.content_part.added",
                            "item_id": &item.id,
                            "output_index": index,
                            "content_index": content_index,
                            "part": part.clone()
                        })));
                        yield Ok(format!("data: {}\n\n", json!({
                            "type": "response.output_text.delta",
                            "item_id": &item.id,
                            "output_index": index,
                            "content_index": content_index,
                            "delta": &part.text
                        })));
                        yield Ok(format!("data: {}\n\n", json!({
                            "type": "response.output_text.done",
                            "item_id": &item.id,
                            "output_index": index,
                            "content_index": content_index,
                            "text": &part.text
                        })));
                        yield Ok(format!("data: {}\n\n", json!({
                            "type": "response.content_part.done",
                            "item_id": &item.id,
                            "output_index": index,
                            "content_index": content_index,
                            "part": part.clone()
                        })));
                    }
                }
            }

            yield Ok(format!("data: {}\n\n", json!({
                "type": "response.output_item.done",
                "output_index": index,
                "item": item.clone()
            })));
        }

        yield Ok(format!("data: {}\n\n", json!({
            "type": "response.completed",
            "response": response_value
        })));
        yield Ok("data: [DONE]\n\n".to_string());
    }
}

async fn log_http_event(
    logs: &LogStore,
    id: &str,
    stage: LogStage,
    status_code: Option<StatusCode>,
    ingress_protocol: Option<&str>,
    egress_protocol: Option<&str>,
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
    let stage_name = stage.as_str().to_string();
    if let Err(err) = logs
        .record(LogEvent {
            id: id.to_string(),
            stage,
            status_code: status_code.map(|status| status.as_u16()),
            ingress_protocol: ingress_protocol.map(ToOwned::to_owned),
            egress_protocol: egress_protocol.map(ToOwned::to_owned),
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
    {
        warn!(
            id = %id,
            stage = %stage_name,
            error = %err,
            "failed to persist gateway log"
        );
    }
}

fn gateway_error_payload(message: &str) -> Value {
    json!({
        "error": {
            "message": message,
            "type": "proxy_error"
        }
    })
}

fn json_for_storage<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(body) => body,
        Err(err) => format!("<serialize error: {err}>"),
    }
}

fn json_value_for_storage(value: &Value) -> String {
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

fn logged_stream_response_body(final_response_body: Option<&str>, response_body: &str) -> String {
    if let Some(final_body) = final_response_body {
        if extract_model_output_from_body(final_body).is_some() {
            return final_body.to_string();
        }
    }

    response_body.to_string()
}

fn google_v1internal_url_label(method: &str, stream: bool) -> String {
    if stream {
        format!("google-v1internal:{method}?alt=sse")
    } else {
        format!("google-v1internal:{method}")
    }
}

fn google_project_id_for_request(account: &AccountRecord) -> &str {
    account
        .project_id()
        .filter(|project_id| !project_id.trim().is_empty())
        .unwrap_or(GOOGLE_PROJECT_ID_FALLBACK)
}

fn elapsed_ms(started_at: Instant) -> i64 {
    started_at.elapsed().as_millis().min(i64::MAX as u128) as i64
}

fn append_to_log_buffer(buffer: &mut String, chunk: &str) {
    buffer.push_str(chunk);
}

fn json_for_log<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(body) => truncate_for_log(&body, 8_000),
        Err(err) => format!("<serialize error: {err}>"),
    }
}

fn json_value_for_log(value: &Value) -> String {
    truncate_for_log(&value.to_string(), 8_000)
}

fn truncate_for_log(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{truncated}...<truncated>")
    } else {
        truncated
    }
}

#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    message: String,
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
        ResolvedProvider, capture_final_response_from_sse_chunk, logged_stream_response_body,
        openai_models_response, provider_uses_openai_account, quota_from_openai_usage,
        resolve_native_target,
    };
    use crate::models::{
        ApiProviderBillingMode, ApiProviderRecord, EgressProtocol, ProviderAuthMode,
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

        assert_eq!(target.egress, EgressProtocol::NativeResponses);
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

        assert_eq!(target.egress, EgressProtocol::NativeChatCompletions);
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
