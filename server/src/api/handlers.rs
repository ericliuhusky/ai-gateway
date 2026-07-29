use crate::{
    adapters::responses::{
        PreparedResponsesUpstream, ResponsesAdapterError, ResponsesAdapterProvider,
        prepare_responses_upstream,
    },
    config::{Config, DEFAULT_CODEX_CLIENT_VERSION},
    models::openai::responses::{
        CodexUsageCredits, CodexUsageRateLimit, CodexUsageRateLimitWindow, CodexUsageResponse,
    },
    models::{
        AccountRecord, ApiProviderRecord, ApiProviderSummary, ChatCompletionsResponsesStream,
        CodexClientVersionSetting, CreateApiProviderRequest, ModelListItem, ModelListResponse,
        PROVIDER_OPENAI_PROXY, ProviderAuthMode, ProviderQuotaCredits, ProviderQuotaResponse,
        ProviderQuotaSnapshot, ProviderQuotaSummary, ProviderQuotaWindow, QuotaSource,
        QuotaSupportStatus, ResponseStreamFrame, SelectedRoute, UpdateCodexClientVersionRequest,
        UpdateSelectedModelRequest, UpdateSelectedProviderRequest,
    },
    openai_tokens::OpenAiTokenService,
    store::{AccountStore, ModelStore, ProviderStore, RouteStore, SettingsStore},
    support::time::now_unix,
    upstream::{
        OPENAI_CODEX_BASE_URL, OpenAiEndpoint, OpenAiRequestBody, OpenAiRequestBuilder,
        PrivateOpenAiRequestBuilder, PublicOpenAiRequestBuilder, UpstreamClient,
    },
};
use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub _client: Client,
    pub _config: Arc<Config>,
    pub openai_tokens: OpenAiTokenService,
    pub accounts: AccountStore,
    pub providers: ProviderStore,
    pub routes: RouteStore,
    pub models: ModelStore,
    pub settings: SettingsStore,
    pub upstream: UpstreamClient,
}

#[derive(Debug, Deserialize)]
pub struct ListModelsQuery {
    #[serde(default)]
    pub force: bool,
}

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn get_codex_client_version(
    State(state): State<AppState>,
) -> Result<Json<CodexClientVersionSetting>, AppError> {
    Ok(Json(codex_client_version_setting(&state)?))
}

pub async fn set_codex_client_version(
    State(state): State<AppState>,
    Json(request): Json<UpdateCodexClientVersionRequest>,
) -> Result<Json<CodexClientVersionSetting>, AppError> {
    let version = normalize_codex_client_version(request.version)?;
    state
        .settings
        .set_codex_client_version(&version)
        .map_err(AppError::internal)?;
    clear_openai_model_caches(&state).await?;
    Ok(Json(codex_client_version_setting(&state)?))
}

pub async fn clear_codex_client_version(
    State(state): State<AppState>,
) -> Result<Json<CodexClientVersionSetting>, AppError> {
    state
        .settings
        .clear_codex_client_version()
        .map_err(AppError::internal)?;
    clear_openai_model_caches(&state).await?;
    Ok(Json(codex_client_version_setting(&state)?))
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
pub(crate) struct CodexAuthFile {
    #[serde(default)]
    tokens: Option<CodexAuthTokensFile>,
}

#[derive(Debug, Serialize)]
pub struct ImportOpenAiFromLocalResponse {
    imported: bool,
    email: String,
    account_id: String,
    has_responses_write: bool,
}

/// Import an OpenAI account from a pasted Codex `auth.json` token payload.
pub async fn import_openai_token(
    State(state): State<AppState>,
    Json(auth_file): Json<CodexAuthFile>,
) -> Result<Json<ImportOpenAiFromLocalResponse>, AppError> {
    let tokens = auth_file
        .tokens
        .ok_or_else(|| AppError::bad_request("Codex auth JSON is missing `tokens`"))?;
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        AppError::bad_request("Codex auth JSON tokens is missing `refresh_token`")
    })?;

    let imported = state
        .openai_tokens
        .import_codex_tokens(
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
        let payload: CodexUsageResponse = serde_json::from_value(raw)
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
    let mut response = load_provider_models(&state, &provider, query.force).await?;
    ensure_codex_model_infos(&mut response);

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
            "upstream_protocol": provider.upstream_protocol,
            "compatibility_profile": provider.compatibility_profile,
            "uses_chat_completions": provider.uses_chat_completions(),
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

pub async fn responses(State(state): State<AppState>, body: Bytes) -> Result<Response, AppError> {
    let raw_body = std::str::from_utf8(&body)
        .map_err(|_| AppError::bad_request("request body must be valid UTF-8"))?
        .to_owned();
    responses_inner(state, raw_body).await
}

pub(super) async fn responses_inner(
    state: AppState,
    raw_body: String,
) -> Result<Response, AppError> {
    let provider = resolve_selected_provider(&state).await?;
    let mut request_json: Value = serde_json::from_str(&raw_body)
        .map_err(|err| AppError::bad_request(format!("invalid request JSON: {err}")))?;
    let request_stream = responses_request_stream(&request_json);
    let model_overridden =
        apply_selected_model_override_to_raw_request(&state, &mut request_json).await;
    let request_body = if model_overridden {
        request_json.to_string()
    } else {
        raw_body
    };
    let requested_model = responses_request_model(&request_json)
        .unwrap_or_default()
        .to_string();

    let prepared = prepare_responses_upstream(
        ResponsesAdapterProvider {
            name: provider.name.clone(),
            auth_mode: provider.auth_mode.clone(),
            record: provider.record.clone(),
            uses_openai_account: provider_uses_openai_account(&provider),
        },
        request_json,
        request_body,
        requested_model,
        request_stream,
    )
    .map_err(adapter_error_to_app_error)?;

    match prepared {
        PreparedResponsesUpstream::OpenAiAccountResponsesPassthrough(prepared) => {
            let account = resolve_account_for_provider(&state, &provider).await?;
            let private_responses = PrivateOpenAiRequestBuilder {
                base_url: OPENAI_CODEX_BASE_URL,
                access_token: account.access_token(),
                account_id: account.upstream_account_id(),
                client_version: None,
            };
            responses_passthrough_inner(
                state,
                private_responses,
                prepared.request_stream,
                prepared.request_body,
            )
            .await
        }
        PreparedResponsesUpstream::ApiChatCompletions(prepared) => {
            let public_chat = PublicOpenAiRequestBuilder {
                base_url: prepared.provider.base_url.as_str(),
                api_key: prepared.provider.api_key.as_str(),
            };
            let upstream = state
                .upstream
                .openai_send(
                    &public_chat,
                    OpenAiEndpoint::ChatCompletions {
                        body: prepared.request_body,
                    },
                )
                .await
                .map_err(AppError::upstream_message)?;

            let response_provider_name = prepared.provider_name;
            let model = prepared.model;
            let output = stream! {
                let mut stream = upstream.bytes_stream();
                let mut chat_sse_buffer = String::new();
                let mut response_stream =
                    ChatCompletionsResponsesStream::new(model, now_unix());

                while let Some(result) = stream.next().await {
                    match result {
                        Ok(chunk) => {
                            let chunk_text = String::from_utf8_lossy(&chunk);
                            let payloads = drain_sse_payloads(&mut chat_sse_buffer, &chunk_text);
                            for payload in payloads {
                                match response_stream.push_chat_payload(&payload) {
                                    Ok(frames) => {
                                        for frame in frames {
                                            let event = match encode_response_frame(frame) {
                                                Ok(event) => event,
                                                Err(err) => {
                                                    yield Err(err);
                                                    return;
                                                }
                                            };
                                            yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                                        }
                                    }
                                    Err(_) => {
                                        yield Err(std::io::Error::other("failed to parse chat completions stream"));
                                        return;
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            yield Err(std::io::Error::other(err));
                            return;
                        }
                    }
                }

                if !chat_sse_buffer.trim().is_empty() {
                    let payloads = drain_sse_payloads(&mut chat_sse_buffer, "\n\n");
                    for payload in payloads {
                        match response_stream.push_chat_payload(&payload) {
                            Ok(frames) => {
                                for frame in frames {
                                    let event = match encode_response_frame(frame) {
                                        Ok(event) => event,
                                        Err(err) => {
                                            yield Err(err);
                                            return;
                                        }
                                    };
                                    yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                                }
                            }
                            Err(_) => {
                                yield Err(std::io::Error::other("failed to parse chat completions stream"));
                                return;
                            }
                        }
                    }
                }

                match response_stream.finish() {
                    Ok(frames) => {
                        for frame in frames {
                            let event = match encode_response_frame(frame) {
                                Ok(event) => event,
                                Err(err) => {
                                    yield Err(err);
                                    return;
                                }
                            };
                            yield Ok::<Bytes, std::io::Error>(Bytes::from(event));
                        }
                    }
                    Err(_) => {
                        yield Err(std::io::Error::other("failed to finish chat completions stream"));
                    }
                }
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
                    HeaderValue::from_str(&response_provider_name)
                        .map_err(|err| AppError::internal(err.to_string()))?,
                )
                .body(Body::from_stream(output))
                .map_err(|err| AppError::internal(err.to_string()))?)
        }
        PreparedResponsesUpstream::ApiResponsesPassthrough(prepared) => {
            let public_responses = PublicOpenAiRequestBuilder {
                base_url: prepared.provider.base_url.as_str(),
                api_key: prepared.provider.api_key.as_str(),
            };
            responses_passthrough_inner(
                state,
                public_responses,
                prepared.request_stream,
                prepared.request_body,
            )
            .await
        }
    }
}

async fn responses_passthrough_inner<B>(
    state: AppState,
    builder: B,
    request_stream: bool,
    request_body: String,
) -> Result<Response, AppError>
where
    B: OpenAiRequestBuilder,
{
    let upstream = state
        .upstream
        .openai_send_passthrough(
            &builder,
            OpenAiEndpoint::Responses {
                body: OpenAiRequestBody::Raw(request_body),
                stream: request_stream,
            },
        )
        .await
        .map_err(AppError::upstream_message)?;
    let upstream_status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let response_is_stream = is_event_stream_response(&upstream_headers);

    if !response_is_stream {
        let response_bytes = upstream.bytes().await.map_err(AppError::upstream)?;
        return build_passthrough_response(
            upstream_status,
            &upstream_headers,
            Body::from(response_bytes),
        );
    }

    let output = stream! {
        let mut stream = upstream.bytes_stream();
        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => yield Ok::<Bytes, std::io::Error>(chunk),
                Err(err) => {
                    yield Err(std::io::Error::other(err));
                    return;
                }
            }
        }
    };

    build_passthrough_response(
        upstream_status,
        &upstream_headers,
        Body::from_stream(output),
    )
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

fn responses_request_model(request: &Value) -> Option<&str> {
    request.get("model").and_then(Value::as_str)
}

fn responses_request_stream(request: &Value) -> bool {
    request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

async fn apply_selected_model_override_to_raw_request(
    state: &AppState,
    request: &mut Value,
) -> bool {
    let Some(model) = state.routes.get().await.selected_model else {
        return false;
    };
    let Some(object) = request.as_object_mut() else {
        return false;
    };
    object.insert("model".to_string(), Value::String(model));
    true
}

fn is_event_stream_response(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false)
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
) -> Result<ModelListResponse, AppError> {
    if provider.auth_mode == ProviderAuthMode::Account {
        let account = resolve_account_for_provider(state, provider).await?;
        if provider.name == PROVIDER_OPENAI_PROXY {
            let client_version = effective_codex_client_version(state)?;
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

fn effective_codex_client_version(state: &AppState) -> Result<String, AppError> {
    Ok(state
        .settings
        .codex_client_version_override()
        .map_err(AppError::internal)?
        .unwrap_or_else(|| DEFAULT_CODEX_CLIENT_VERSION.to_string()))
}

fn codex_client_version_setting(state: &AppState) -> Result<CodexClientVersionSetting, AppError> {
    let override_version = state
        .settings
        .codex_client_version_override()
        .map_err(AppError::internal)?;
    let effective_version = override_version
        .clone()
        .unwrap_or_else(|| DEFAULT_CODEX_CLIENT_VERSION.to_string());
    Ok(CodexClientVersionSetting {
        default_version: DEFAULT_CODEX_CLIENT_VERSION.to_string(),
        is_overridden: override_version.is_some(),
        override_version,
        effective_version,
    })
}

fn normalize_codex_client_version(version: String) -> Result<String, AppError> {
    let version = version.trim();
    if version.is_empty() {
        return Err(AppError::bad_request("client version cannot be empty"));
    }
    if version.len() > 64
        || !version
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+_".contains(character))
    {
        return Err(AppError::bad_request(
            "client version may only contain letters, numbers, `.`, `-`, `+`, and `_`",
        ));
    }
    Ok(version.to_string())
}

async fn clear_openai_model_caches(state: &AppState) -> Result<(), AppError> {
    for provider in state.providers.list().await {
        if provider.name == PROVIDER_OPENAI_PROXY {
            state
                .models
                .delete(&provider.id)
                .map_err(AppError::internal)?;
        }
    }
    Ok(())
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
            data.push((
                id.to_string(),
                ModelListItem { id: id.to_string() },
                codex_model_info(id, Some(entry)),
            ));
        }
    }
    data.sort_by(|left, right| left.0.cmp(&right.0));

    Ok(ModelListResponse {
        object: "list".to_string(),
        data: data.iter().map(|(_, item, _)| item.clone()).collect(),
        models: data.into_iter().map(|(_, _, model)| model).collect(),
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
                codex_model_info(id, Some(entry)),
            ));
        }
    }
    data.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    Ok(ModelListResponse {
        object: "list".to_string(),
        data: data.iter().map(|(_, _, item, _)| item.clone()).collect(),
        models: data.into_iter().map(|(_, _, _, model)| model).collect(),
    })
}

fn ensure_codex_model_infos(response: &mut ModelListResponse) {
    if response.models.is_empty() {
        response.models = response
            .data
            .iter()
            .map(|item| codex_model_info(item.id.as_str(), None))
            .collect();
    }
}

fn codex_model_info(id: &str, entry: Option<&Value>) -> Value {
    let context_window = entry
        .and_then(|entry| entry.get("context_window").and_then(Value::as_i64))
        .or_else(|| entry.and_then(|entry| entry.get("max_context_window").and_then(Value::as_i64)))
        .unwrap_or(272_000);
    let max_context_window = entry
        .and_then(|entry| entry.get("max_context_window").and_then(Value::as_i64))
        .unwrap_or(context_window);
    let auto_compact_token_limit = entry
        .and_then(|entry| entry.get("auto_compact_token_limit"))
        .filter(|value| value.is_number())
        .cloned()
        .unwrap_or(Value::Null);
    let display_name = entry
        .and_then(|entry| entry.get("display_name").and_then(Value::as_str))
        .unwrap_or(id);
    let description = entry
        .and_then(|entry| entry.get("description").and_then(Value::as_str))
        .map(Value::from)
        .unwrap_or(Value::Null);

    json!({
        "slug": id,
        "display_name": display_name,
        "description": description,
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            { "effort": "low", "description": "Fast responses with lighter reasoning" },
            { "effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks" },
            { "effort": "high", "description": "Greater reasoning depth for complex problems" },
            { "effort": "xhigh", "description": "Extra high reasoning depth for complex problems" }
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": entry.and_then(|entry| entry.get("priority").and_then(Value::as_i64)).unwrap_or(0),
        "availability_nux": null,
        "upgrade": null,
        "base_instructions": "",
        "model_messages": null,
        "supports_reasoning_summaries": true,
        "default_reasoning_summary": "none",
        "support_verbosity": true,
        "default_verbosity": "low",
        "apply_patch_tool_type": "freeform",
        "web_search_tool_type": "text_and_image",
        "truncation_policy": { "mode": "tokens", "limit": 10000 },
        "supports_parallel_tool_calls": true,
        "supports_image_detail_original": true,
        "context_window": context_window,
        "max_context_window": max_context_window,
        "auto_compact_token_limit": auto_compact_token_limit,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": true
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
        .acquire_by_id(&state.openai_tokens, &state.upstream, account_id)
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
        upstream_protocol: record.upstream_protocol.clone(),
        compatibility_profile: record.compatibility_profile.clone(),
        uses_chat_completions: record.uses_chat_completions(),
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

fn quota_from_openai_usage(payload: CodexUsageResponse) -> ProviderQuotaSummary {
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
    rate_limit: Option<CodexUsageRateLimit>,
    credits: Option<CodexUsageCredits>,
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
    window: Option<CodexUsageRateLimitWindow>,
) -> Option<ProviderQuotaWindow> {
    let window = window?;
    Some(ProviderQuotaWindow {
        used_percent: f64::from(window.used_percent),
        window_minutes: Some(i64::from(window.limit_window_seconds) / 60),
        resets_at: Some(window.reset_at),
    })
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

fn encode_response_frame(frame: ResponseStreamFrame) -> Result<String, std::io::Error> {
    frame
        .encode_sse()
        .map_err(|err| std::io::Error::other(err.to_string()))
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

fn adapter_error_to_app_error(error: ResponsesAdapterError) -> AppError {
    match error {
        ResponsesAdapterError::BadRequest(message) => AppError::bad_request(message),
        ResponsesAdapterError::Internal(message) => AppError::internal(message),
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
    use super::CodexAuthFile;
    use super::{
        ChatCompletionsResponsesStream, ResolvedProvider, drain_sse_payloads,
        openai_models_response, provider_uses_openai_account, quota_from_openai_usage,
    };
    use crate::models::{
        ApiProviderBillingMode, ApiProviderRecord, ProviderAuthMode, ProviderCompatibilityProfile,
        ProviderUpstreamProtocol, ResponseStreamFrame,
    };
    use serde_json::json;

    #[test]
    fn validates_codex_client_version_override() {
        assert_eq!(
            super::normalize_codex_client_version(" 0.147.0-beta.1 ".to_string()).unwrap(),
            "0.147.0-beta.1"
        );
        assert!(super::normalize_codex_client_version("0.147.0 ?".to_string()).is_err());
    }

    #[test]
    fn parses_minimal_pasted_codex_auth_json() {
        let auth: CodexAuthFile = serde_json::from_value(json!({
            "tokens": {
                "access_token": "access-token",
                "refresh_token": "refresh-token"
            }
        }))
        .expect("minimal Codex auth JSON should parse");

        let tokens = auth.tokens.expect("tokens should be present");
        assert_eq!(tokens.access_token, "access-token");
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh-token"));
        assert!(tokens.id_token.is_none());
        assert!(tokens.account_id.is_none());
    }

    #[test]
    fn ignores_optional_official_codex_auth_fields() {
        let auth: CodexAuthFile = serde_json::from_value(json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": "id-token",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "account_id": "account-id"
            },
            "last_refresh": "2026-07-19T06:25:55Z"
        }))
        .expect("full Codex auth JSON should parse");

        let tokens = auth.tokens.expect("tokens should be present");
        assert_eq!(tokens.id_token.as_deref(), Some("id-token"));
        assert_eq!(tokens.account_id.as_deref(), Some("account-id"));
    }

    fn encode_frames(frames: Vec<ResponseStreamFrame>) -> String {
        frames
            .into_iter()
            .map(|frame| frame.encode_sse().expect("frame should encode"))
            .collect::<String>()
    }

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
        let joined = encode_frames(events);

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
        let joined = encode_frames(events);

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
                upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
                compatibility_profile: ProviderCompatibilityProfile::OpenAiCodex,
                billing_mode: ApiProviderBillingMode::Subscription,
            }),
        };

        assert!(provider_uses_openai_account(&provider));
    }
}
