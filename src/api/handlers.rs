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
        CreateApiProviderRequest, EgressProtocol, IngressProtocol, ModelListItem,
        ModelListResponse, PROVIDER_GOOGLE_PROXY, PROVIDER_OPENAI_PROXY, ProviderAuthMode,
        ProviderQuotaCredits, ProviderQuotaResponse, ProviderQuotaSnapshot, ProviderQuotaSummary,
        ProviderQuotaWindow, QuotaSource, QuotaSupportStatus, ResponsesRequest, ResponsesResponse,
        SelectedProvider, UpdateSelectedProviderRequest, UpstreamRateLimitStatusDetails,
        UpstreamRateLimitStatusPayload, UpstreamRateLimitWindowSnapshot,
    },
    store::{AccountPool, ProviderStore, RouteStore},
    upstream::UpstreamClient,
};
use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{Host, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Instant;
use std::{fs, path::Path, sync::Arc};
use tracing::{info, warn};
use uuid::Uuid;

const BUNDLED_CODEX_CONFIG: &str = include_str!("../../assets/codex-config.toml");
const MISSING_FILE_SENTINEL: &str = "__AI_GATEWAY_MISSING__";

#[derive(Clone)]
pub struct AppState {
    pub _client: Client,
    pub _config: Arc<Config>,
    pub oauth: OAuthClient,
    pub accounts: AccountPool,
    pub providers: ProviderStore,
    pub routes: RouteStore,
    pub upstream: UpstreamClient,
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
    let project_id = state
        .upstream
        .fetch_project_id(&token.access_token)
        .await
        .map_err(AppError::bad_request)?;
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

    Ok(Html(format!(
        "<html><body style='font-family:sans-serif;padding:32px'><h1>Login successful</h1><p>Account <strong>{}</strong> is now in the proxy pool.</p><p>You can close this page and call <code>/openai/v1/responses</code>.</p></body></html>",
        account.email
    )))
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
    let scope_hint = if has_responses_write {
        "<p>Detected <code>api.responses.write</code>, so this account should be able to call the ChatGPT Codex responses endpoint directly.</p>"
    } else {
        "<p><strong>Warning:</strong> this ChatGPT/Codex session does not appear to include <code>api.responses.write</code>. OAuth login succeeds, but calls to the ChatGPT Codex responses endpoint may still return 401.</p>"
    };

    Ok(Html(format!(
        "<html><body style='font-family:sans-serif;padding:32px'><h1>OpenAI login successful</h1><p>Account <strong>{}</strong> is now in the proxy pool.</p>{}<p>Stored account id: <code>{}</code></p><p>You can close this page and call <code>/openai/v1/responses</code>.</p></body></html>",
        email, scope_hint, account.id
    )))
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
        if provider.name == PROVIDER_OPENAI_PROXY {
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
        unsupported_quota_summary(format!(
            "official quota snapshot is not supported yet for api_key provider `{}`",
            provider.name
        ))
    };

    Ok(Json(ProviderQuotaResponse {
        provider: provider_summary,
        quota,
    }))
}

pub async fn list_models(
    State(state): State<AppState>,
) -> Result<Json<ModelListResponse>, AppError> {
    let provider = resolve_selected_provider(&state).await?;
    let response = if provider.auth_mode == ProviderAuthMode::Account {
        let account = resolve_account_for_provider(&state, &provider).await?;
        if provider.name == PROVIDER_GOOGLE_PROXY {
            let raw = state
                .upstream
                .fetch_google_available_models(account.access_token(), account.project_id())
                .await
                .map_err(AppError::upstream_message)?;
            google_models_response(&provider.name, &raw)?
        } else if provider.name == PROVIDER_OPENAI_PROXY {
            let raw = state
                .upstream
                .fetch_openai_models(
                    &format!("models_{}", Uuid::new_v4().simple()),
                    account.access_token(),
                    account.upstream_account_id(),
                )
                .await
                .map_err(AppError::upstream_message)?;
            openai_models_response(&provider.name, &raw)?
        } else {
            return Err(AppError::bad_request(format!(
                "account auth provider is not supported yet: {}",
                provider.name
            )));
        }
    } else {
        let native_provider = provider
            .record
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
        native_models_response(&provider.name, &raw)?
    };

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
    validate_selected_provider(&state, &provider_id).await?;

    let route = state
        .routes
        .set(Some(provider_id))
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(json!({ "selected_provider": route_payload(route) })))
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

pub async fn responses(
    State(state): State<AppState>,
    Json(request): Json<ResponsesRequest>,
) -> Result<Response, AppError> {
    let request_id = format!("req_{}", Uuid::new_v4().simple());
    let started_at = Instant::now();
    let provider = resolve_selected_provider(&state).await?;
    info!(
        request_id = %request_id,
        ingress = IngressProtocol::OpenAiResponses.as_str(),
        provider = %provider.name,
        body = %json_for_log(&request),
        "received /openai/v1/responses request"
    );

    if provider.auth_mode == ProviderAuthMode::Account && provider.name == PROVIDER_OPENAI_PROXY {
        let account = resolve_account_for_provider(&state, &provider).await?;
        let request_body = responses_to_openai_private(&request)
            .map_err(|err| AppError::internal(err.to_string()))?;
        let upstream = state
            .upstream
            .call_openai_responses(
                &request_id,
                account.access_token(),
                account.upstream_account_id(),
                request_body,
                request.stream,
            )
            .await
            .map_err(AppError::upstream_message)?;

        if !request.stream {
            let response_body: Value = upstream.json().await.map_err(AppError::upstream)?;
            info!(
                request_id = %request_id,
                elapsed_ms = started_at.elapsed().as_millis(),
                email = %account.email,
                provider = %provider.name,
                egress = %EgressProtocol::OpenAiPrivateResponses.as_str(),
                response_body = %json_value_for_log(&response_body),
                "returning OpenAI /openai/v1/responses body"
            );
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

        let output = upstream
            .bytes_stream()
            .map(|result| result.map(Bytes::from).map_err(std::io::Error::other));

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
        let project_id = account
            .project_id()
            .map(str::to_string)
            .ok_or_else(|| AppError::bad_request("selected account has no project_id"))?;
        let request_body = wrap_v1internal(
            serde_json::to_value(&gemini_request)
                .map_err(|err| AppError::internal(err.to_string()))?,
            &project_id,
            &request.model,
            &account.id,
        );

        info!(
            request_id = %request_id,
            model = %request.model,
            stream = request.stream,
            email = %account.email,
            project_id = %project_id,
            egress = %EgressProtocol::GoogleV1Internal.as_str(),
            gemini_request = %json_for_log(&gemini_request),
            upstream_request = %json_value_for_log(&request_body),
            "proxying request to Gemini upstream"
        );

        let upstream = state
            .upstream
            .call_v1internal(
                if request.stream {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                },
                &request_id,
                account.access_token(),
                request_body,
                request.stream,
            )
            .await
            .map_err(AppError::upstream_message)?;

        if !request.stream {
            let gemini_body: Value = upstream.json().await.map_err(AppError::upstream)?;
            info!(
                request_id = %request_id,
                elapsed_ms = started_at.elapsed().as_millis(),
                upstream_response = %json_value_for_log(&gemini_body),
                "received non-stream Gemini response"
            );
            let response = gemini_to_responses(&request.model, &gemini_body);
            info!(
                request_id = %request_id,
                elapsed_ms = started_at.elapsed().as_millis(),
                output_items = response.output.len(),
                response_body = %json_for_log(&response),
                "returning non-stream /openai/v1/responses body"
            );
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
        let stream = upstream.bytes_stream();
        let request_id_for_stream = request_id.clone();
        let output = stream! {
        let encode_event = |value: &Value| -> Result<String, std::io::Error> {
            serde_json::to_string(value)
                .map(|body| format!("data: {body}\n\n"))
                .map_err(std::io::Error::other)
        };
        let mut buffer = String::new();
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
                emitted_event_count += 1;
                yield Ok::<String, std::io::Error>(event)
            },
            Err(err) => {
                yield Err(err);
                return;
            }
        }

        for await chunk in stream {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) => {
                    yield Err(std::io::Error::other(err));
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));

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
                    request_id = %request_id_for_stream,
                    chunk_index = upstream_chunk_count,
                    upstream_chunk = %truncate_for_log(payload, 3000),
                    "received upstream SSE chunk"
                );

                let gemini_event: Value = match serde_json::from_str(payload) {
                    Ok(value) => value,
                    Err(err) => {
                        warn!(
                            request_id = %request_id_for_stream,
                            chunk_index = upstream_chunk_count,
                            error = %err,
                            "failed to parse upstream SSE chunk"
                        );
                        continue;
                    }
                };

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
                                            emitted_event_count += 1;
                                            yield Ok(event)
                                        },
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
                                            emitted_event_count += 1;
                                            yield Ok(event)
                                        },
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
                                        emitted_event_count += 1;
                                        yield Ok(event)
                                    },
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
                                        emitted_event_count += 1;
                                        yield Ok(event)
                                    },
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
                                        emitted_event_count += 1;
                                        yield Ok(event)
                                    },
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
                    emitted_event_count += 1;
                    yield Ok(event)
                },
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
                    emitted_event_count += 1;
                    yield Ok(event)
                },
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
                    emitted_event_count += 1;
                    yield Ok(event)
                },
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
        match encode_event(&completed) {
            Ok(event) => {
                emitted_event_count += 1;
                yield Ok(event)
            },
            Err(err) => {
                yield Err(err);
                return;
            }
        }

        info!(
            request_id = %request_id_for_stream,
            elapsed_ms = started_at.elapsed().as_millis(),
            upstream_chunks = upstream_chunk_count,
            emitted_events = emitted_event_count,
            output_items = completed_output_items.len(),
            text_len = accumulated_text.len(),
            "completed streaming /openai/v1/responses request"
        );

        yield Ok("data: [DONE]\n\n".to_string());
        }
        .map(|result| result.map(Bytes::from));

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
        let upstream = state
            .upstream
            .call_openai_chat_upstream(
                &request_id,
                &native_provider.base_url,
                &native_provider.api_key,
                request_body,
            )
            .await
            .map_err(AppError::upstream_message)?;
        let chat_body: Value = upstream.json().await.map_err(AppError::upstream)?;
        let response = chat_completions_to_responses(&request.model, &chat_body);

        if !request.stream {
            info!(
                request_id = %request_id,
                elapsed_ms = started_at.elapsed().as_millis(),
                provider = %provider.name,
                egress = %native_target.egress.as_str(),
                upstream_model = %native_target.upstream_model,
                response_body = %json_for_log(&response),
                "returning chat-completions-adapted /openai/v1/responses body"
            );
            return Ok((
                StatusCode::OK,
                [("x-provider", provider.name.as_str())],
                Json(response),
            )
                .into_response());
        }

        let output = synthesized_responses_stream(response).map(|result| result.map(Bytes::from));
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
    let upstream = state
        .upstream
        .call_openai_responses_upstream(
            &request_id,
            &native_provider.base_url,
            &native_provider.api_key,
            request_body,
            request.stream,
        )
        .await
        .map_err(AppError::upstream_message)?;

    if !request.stream {
        let response_body: Value = upstream.json().await.map_err(AppError::upstream)?;
        info!(
            request_id = %request_id,
            elapsed_ms = started_at.elapsed().as_millis(),
            provider = %provider.name,
            egress = %native_target.egress.as_str(),
            upstream_model = %native_target.upstream_model,
            response_body = %json_value_for_log(&response_body),
            "returning native provider /openai/v1/responses body"
        );
        return Ok((
            StatusCode::OK,
            [("x-provider", provider.name.as_str())],
            Json(response_body),
        )
            .into_response());
    }

    let output = upstream
        .bytes_stream()
        .map(|result| result.map(Bytes::from).map_err(std::io::Error::other));

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
        "updated_at": route.updated_at,
    })
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

fn native_model_id(entry: &Value) -> Option<&str> {
    entry
        .get("id")
        .or_else(|| entry.get("model"))
        .or_else(|| entry.get("name"))
        .and_then(Value::as_str)
}

async fn validate_selected_provider(state: &AppState, provider_id: &str) -> Result<(), AppError> {
    resolve_provider_by_id(state, provider_id).await.map(|_| ())
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
    let name = provider.name.as_str();
    let base_url = provider.base_url.as_str();

    if name == "bytedance-coding-plan" || base_url.contains("/api/coding/v3") {
        return NativeTarget {
            upstream_model: map_bytedance_coding_model(requested_model),
            egress: EgressProtocol::NativeChatCompletions,
            uses_chat_completions: true,
        };
    }

    if name == "bytedance" || base_url.contains("volces.com/api/v3") {
        return NativeTarget {
            upstream_model: map_bytedance_model(requested_model),
            egress: EgressProtocol::NativeResponses,
            uses_chat_completions: false,
        };
    }

    NativeTarget {
        upstream_model: requested_model.to_string(),
        egress: EgressProtocol::NativeResponses,
        uses_chat_completions: false,
    }
}

fn map_bytedance_model(requested_model: &str) -> String {
    if is_codex_style_model(requested_model) {
        "doubao-seed-2-0-lite-260215".to_string()
    } else {
        requested_model.to_string()
    }
}

fn map_bytedance_coding_model(requested_model: &str) -> String {
    if is_codex_style_model(requested_model) {
        "ark-code-latest".to_string()
    } else {
        requested_model.to_string()
    }
}

fn is_codex_style_model(model: &str) -> bool {
    model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("codex-")
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
    use super::{openai_models_response, quota_from_openai_usage};
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
}
