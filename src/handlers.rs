use crate::{
    account_pool::AccountPool,
    auth::OAuthClient,
    config::Config,
    mapper::{gemini_to_responses, responses_to_gemini, wrap_v1internal},
    models::{
        CreateApiProviderRequest, PROVIDER_GOOGLE_PROXY, PROVIDER_OPENAI_PROXY, ResponsesRequest,
        RouteSelection, UpdateRouteRequest,
    },
    provider_store::ProviderStore,
    route_store::RouteStore,
    upstream::UpstreamClient,
};
use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{Host, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};
use uuid::Uuid;

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

    Ok(Html(format!(
        "<html><body style='font-family:sans-serif;padding:32px'><h1>Login successful</h1><p>Account <strong>{}</strong> is now in the proxy pool.</p><p>You can close this page and call <code>/v1/responses</code>.</p></body></html>",
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
    let scope_hint = if has_responses_write {
        "<p>Detected <code>api.responses.write</code>, so this account should be able to call the ChatGPT Codex responses endpoint directly.</p>"
    } else {
        "<p><strong>Warning:</strong> this ChatGPT/Codex session does not appear to include <code>api.responses.write</code>. OAuth login succeeds, but calls to the ChatGPT Codex responses endpoint may still return 401.</p>"
    };

    Ok(Html(format!(
        "<html><body style='font-family:sans-serif;padding:32px'><h1>OpenAI login successful</h1><p>Account <strong>{}</strong> is now in the proxy pool.</p>{}<p>Stored account id: <code>{}</code></p><p>You can close this page and call <code>/v1/responses</code>.</p></body></html>",
        email, scope_hint, account.id
    )))
}

pub async fn list_accounts(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "accounts": state.accounts.list().await }))
}

pub async fn list_providers(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "providers": state.providers.list().await }))
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
            "base_url": provider.base_url,
            "billing_mode": provider.billing_mode,
            "disabled": provider.disabled,
            "updated_at": provider.updated_at,
        }
    })))
}

pub async fn get_route(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "route": route_payload(state.routes.get().await) }))
}

pub async fn set_route(
    State(state): State<AppState>,
    Json(request): Json<UpdateRouteRequest>,
) -> Result<Json<Value>, AppError> {
    let provider = normalize_route_provider(request.provider);
    if let Some(provider_name) = provider.as_deref() {
        validate_selected_provider(&state, provider_name).await?;
    }

    let route = state
        .routes
        .set(provider)
        .await
        .map_err(AppError::bad_request)?;
    Ok(Json(json!({ "route": route_payload(route) })))
}

const OPENAI_CODEX_DEFAULT_INSTRUCTIONS: &str = "You are Codex.";

fn sanitize_openai_codex_request_body(body: &mut Value) {
    let Some(object) = body.as_object_mut() else {
        return;
    };

    if object
        .get("instructions")
        .is_none_or(|value| value.is_null() || value.as_str().is_some_and(str::is_empty))
    {
        object.insert(
            "instructions".to_string(),
            Value::String(OPENAI_CODEX_DEFAULT_INSTRUCTIONS.to_string()),
        );
    }

    object.insert("store".to_string(), Value::Bool(false));

    // ChatGPT Codex backend-api accepts a narrower payload than the public
    // Responses API, so strip fields that it rejects instead of surfacing
    // avoidable 400s to callers.
    for key in ["max_output_tokens", "max_tokens", "temperature", "top_p"] {
        object.remove(key);
    }

    if let Some(tools) = object.get_mut("tools") {
        normalize_openai_codex_tools(tools);
    }
    if let Some(tool_choice) = object.get_mut("tool_choice") {
        normalize_openai_codex_tool_choice(tool_choice);
    }

    if let Some(input) = object.get_mut("input") {
        normalize_openai_codex_input(input);
    }
}

fn normalize_openai_codex_input(input: &mut Value) {
    let Some(items) = input.as_array_mut() else {
        return;
    };

    let mut rewritten = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        let Some(item_obj) = item.as_object() else {
            rewritten.push(item);
            continue;
        };

        let item_type = item_obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();

        match item_type {
            "function_call" => {
                rewritten.push(json!({
                    "type": "function_call",
                    "call_id": item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                    "name": item_obj.get("name").cloned().unwrap_or(Value::Null),
                    "arguments": item_obj.get("arguments").cloned().unwrap_or(Value::String("{}".to_string())),
                }));
                continue;
            }
            "function_call_output" | "custom_tool_call_output" => {
                rewritten.push(json!({
                    "type": "function_call_output",
                    "call_id": item_obj.get("call_id").cloned().unwrap_or(Value::Null),
                    "output": stringify_openai_codex_output(item_obj.get("output").cloned()),
                }));
                continue;
            }
            "input_text" | "input_image" => {
                rewritten.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [normalize_openai_codex_content_part(item.clone())],
                }));
                continue;
            }
            _ => {}
        }

        let role = item_obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        let content = normalize_openai_codex_message_content(item_obj.get("content").cloned());
        if let Some(content) = content {
            rewritten.push(json!({
                "type": "message",
                "role": role,
                "content": content,
            }));
        }

        if let Some(tool_calls) = item_obj.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                if let Some(tool_call_obj) = tool_call.as_object() {
                    rewritten.push(json!({
                        "type": "function_call",
                        "call_id": tool_call_obj.get("call_id").cloned().unwrap_or(Value::Null),
                        "name": tool_call_obj.get("name").cloned().unwrap_or(Value::Null),
                        "arguments": tool_call_obj.get("arguments").cloned().unwrap_or(Value::String("{}".to_string())),
                    }));
                }
            }
        }
    }

    *items = rewritten;
}

fn normalize_openai_codex_message_content(content: Option<Value>) -> Option<Value> {
    let content = content?;

    if let Some(text) = content.as_str() {
        return Some(json!([{ "type": "input_text", "text": text }]));
    }

    let Some(parts) = content.as_array() else {
        return None;
    };

    let mut normalized = Vec::with_capacity(parts.len());
    for part in parts {
        normalized.push(normalize_openai_codex_content_part(part.clone()));
    }

    if normalized.is_empty() {
        None
    } else {
        Some(Value::Array(normalized))
    }
}

fn normalize_openai_codex_content_part(part: Value) -> Value {
    let Some(part_obj) = part.as_object() else {
        return json!({
            "type": "input_text",
            "text": part,
        });
    };

    let part_type = part_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let is_text_like = matches!(
        part_type,
        "text" | "input_text" | "output_text" | "summary_text"
    ) || part_obj.get("text").is_some();

    if is_text_like {
        return json!({
            "type": "input_text",
            "text": part_obj.get("text").cloned().unwrap_or(Value::String(String::new())),
        });
    }

    if part_type == "input_image" {
        if let Some(url) = part_obj.get("image_url").and_then(Value::as_str) {
            return json!({
                "type": "input_image",
                "source": {
                    "type": "url",
                    "url": url,
                }
            });
        }
    }

    part
}

fn stringify_openai_codex_output(value: Option<Value>) -> String {
    match value {
        Some(Value::String(text)) => text,
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn normalize_openai_codex_tools(tools: &mut Value) {
    let Some(tool_items) = tools.as_array_mut() else {
        return;
    };

    let mut normalized = Vec::with_capacity(tool_items.len());
    for tool in tool_items.drain(..) {
        let Some(tool_obj) = tool.as_object() else {
            continue;
        };

        let tool_type = tool_obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();

        if tool_type != "function" {
            let mut preserved = serde_json::Map::new();
            for (key, value) in tool_obj {
                if key == "function" || value.is_null() {
                    continue;
                }
                preserved.insert(key.clone(), value.clone());
            }
            if !preserved.is_empty() {
                normalized.push(Value::Object(preserved));
            }
            continue;
        }

        let function_obj = tool_obj.get("function").and_then(Value::as_object);
        let name = tool_obj
            .get("name")
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("name").cloned()));
        let description = tool_obj
            .get("description")
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("description").cloned()));
        let parameters = tool_obj
            .get("parameters")
            .cloned()
            .or_else(|| function_obj.and_then(|f| f.get("parameters").cloned()));
        let strict = function_obj.and_then(|f| f.get("strict").cloned());

        if name.as_ref().and_then(Value::as_str).is_none() {
            continue;
        }

        normalized.push(json!({
            "type": "function",
            "name": name.unwrap_or(Value::String(String::new())),
            "description": description.unwrap_or(Value::Null),
            "parameters": parameters.unwrap_or_else(|| json!({"type":"object","properties":{}})),
            "strict": strict.unwrap_or(Value::Null),
        }));
    }

    *tool_items = normalized;
}

fn normalize_openai_codex_tool_choice(tool_choice: &mut Value) {
    let Some(tool_choice_obj) = tool_choice.as_object() else {
        return;
    };

    if tool_choice_obj.get("type").and_then(Value::as_str) == Some("tool") {
        if let Some(name) = tool_choice_obj.get("name").cloned() {
            *tool_choice = json!({
                "type": "function",
                "function": { "name": name }
            });
        }
    }
}

pub async fn responses(
    State(state): State<AppState>,
    Json(request): Json<ResponsesRequest>,
) -> Result<Response, AppError> {
    let request_id = format!("req_{}", Uuid::new_v4().simple());
    let started_at = Instant::now();
    let route = state.routes.get().await;
    let provider = route
        .provider
        .clone()
        .unwrap_or_else(|| provider_for_model(&request.model).to_string());
    let route_mode = if route.provider.is_some() {
        "manual"
    } else {
        "auto"
    };
    info!(
        request_id = %request_id,
        provider = %provider,
        route_mode = %route_mode,
        body = %json_for_log(&request),
        "received /v1/responses request"
    );

    if provider == PROVIDER_OPENAI_PROXY {
        let account = state
            .accounts
            .acquire_for_provider(&state.oauth, &state.upstream, PROVIDER_OPENAI_PROXY)
            .await
            .map_err(AppError::bad_request)?;
        let mut request_body =
            serde_json::to_value(&request).map_err(|err| AppError::internal(err.to_string()))?;
        sanitize_openai_codex_request_body(&mut request_body);
        let upstream = state
            .upstream
            .call_openai_responses(
                &request_id,
                account.token.access_token(),
                account.token.account_id(),
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
                provider = PROVIDER_OPENAI_PROXY,
                response_body = %json_value_for_log(&response_body),
                "returning OpenAI /v1/responses body"
            );
            return Ok((
                StatusCode::OK,
                [
                    ("x-account-email", account.email.as_str()),
                    ("x-provider", PROVIDER_OPENAI_PROXY),
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
                HeaderValue::from_static(PROVIDER_OPENAI_PROXY),
            )
            .body(Body::from_stream(output))
            .map_err(|err| AppError::internal(err.to_string()))?);
    }

    if provider == PROVIDER_GOOGLE_PROXY {
        let gemini_request = responses_to_gemini(&request).map_err(AppError::bad_request)?;
        let account = state
            .accounts
            .acquire_for_provider(&state.oauth, &state.upstream, PROVIDER_GOOGLE_PROXY)
            .await
            .map_err(AppError::bad_request)?;
        let project_id = account
            .token
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
                account.token.access_token(),
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
                "returning non-stream /v1/responses body"
            );
            return Ok((
                StatusCode::OK,
                [
                    ("x-account-email", account.email.as_str()),
                    ("x-provider", PROVIDER_GOOGLE_PROXY),
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
            "completed streaming /v1/responses request"
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
                HeaderValue::from_static(PROVIDER_GOOGLE_PROXY),
            )
            .body(Body::from_stream(output))
            .map_err(|err| AppError::internal(err.to_string()))?);
    }

    let native_provider = state
        .providers
        .find_by_name(&provider)
        .await
        .ok_or_else(|| AppError::bad_request(format!("unknown provider: {provider}")))?;
    if native_provider.disabled {
        return Err(AppError::bad_request(format!(
            "provider {provider} is disabled"
        )));
    }

    let upstream = state
        .upstream
        .call_native_responses(
            &request_id,
            &native_provider.base_url,
            &native_provider.api_key,
            serde_json::to_value(&request).map_err(|err| AppError::internal(err.to_string()))?,
            request.stream,
        )
        .await
        .map_err(AppError::upstream_message)?;

    if !request.stream {
        let response_body: Value = upstream.json().await.map_err(AppError::upstream)?;
        info!(
            request_id = %request_id,
            elapsed_ms = started_at.elapsed().as_millis(),
            provider = %provider,
            response_body = %json_value_for_log(&response_body),
            "returning native provider /v1/responses body"
        );
        return Ok((
            StatusCode::OK,
            [("x-provider", provider.as_str())],
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
            HeaderValue::from_str(&provider).map_err(|err| AppError::internal(err.to_string()))?,
        )
        .body(Body::from_stream(output))
        .map_err(|err| AppError::internal(err.to_string()))?)
}

fn provider_for_model(model: &str) -> &'static str {
    if model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("codex-")
    {
        PROVIDER_OPENAI_PROXY
    } else {
        PROVIDER_GOOGLE_PROXY
    }
}

fn route_payload(route: RouteSelection) -> Value {
    json!({
        "mode": if route.provider.is_some() { "manual" } else { "auto" },
        "provider": route.provider,
        "updated_at": route.updated_at,
    })
}

fn normalize_route_provider(provider: Option<String>) -> Option<String> {
    provider.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn validate_selected_provider(state: &AppState, provider: &str) -> Result<(), AppError> {
    if provider == PROVIDER_OPENAI_PROXY || provider == PROVIDER_GOOGLE_PROXY {
        return Ok(());
    }

    let native_provider = state
        .providers
        .find_by_name(provider)
        .await
        .ok_or_else(|| AppError::bad_request(format!("unknown provider: {provider}")))?;
    if native_provider.disabled {
        return Err(AppError::bad_request(format!(
            "provider {provider} is disabled"
        )));
    }
    Ok(())
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
