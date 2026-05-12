use crate::api::handlers::{
    AppState, ResolvedProvider, append_to_log_buffer, apply_selected_model_override,
    capture_final_response_from_ws_event, elapsed_ms, json_value_for_storage, log_http_event,
    logged_stream_response_body, provider_uses_openai_account, resolve_account_for_provider,
    resolve_selected_provider, responses_inner,
};
use crate::{
    adapters::responses::responses_to_openai_private,
    config::Config,
    models::{AccountRecord, ClientProtocol, ProviderAuthMode, ResponsesRequest, UpstreamProtocol},
    store::LogStage,
};
use axum::{
    extract::{
        State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::time::Instant;
use uuid::Uuid;

pub async fn responses_websocket(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| responses_websocket_session(state, socket))
}

async fn responses_websocket_session(state: AppState, mut socket: WebSocket) {
    while let Some(message) = socket.next().await {
        let message = match message {
            Ok(message) => message,
            Err(_) => {
                break;
            }
        };

        match message {
            WsMessage::Text(text) => {
                if handle_responses_websocket_text(state.clone(), &mut socket, text)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            WsMessage::Close(_) => break,
            WsMessage::Ping(payload) => {
                if socket.send(WsMessage::Pong(payload)).await.is_err() {
                    break;
                }
            }
            WsMessage::Pong(_) => {}
            WsMessage::Binary(_) => {
                if socket
                    .send(WsMessage::Text(responses_ws_error_event(
                        "websocket messages must be JSON text frames",
                    )))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

async fn handle_responses_websocket_text(
    state: AppState,
    socket: &mut WebSocket,
    text: String,
) -> Result<(), String> {
    let mut ws_request = match response_create_ws_message_to_request(&text) {
        Ok(request) => request,
        Err(err) => {
            socket
                .send(WsMessage::Text(responses_ws_error_event(&err)))
                .await
                .map_err(|send_err| send_err.to_string())?;
            return Ok(());
        }
    };

    if !ws_request.generate {
        socket
            .send(WsMessage::Text(responses_ws_completed_event(
                &ws_request.request.model,
            )))
            .await
            .map_err(|send_err| send_err.to_string())?;
        return Ok(());
    }

    let id = Uuid::new_v4().simple().to_string();
    let started_at = Instant::now();
    let client_model = ws_request.request.model.clone();
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
        Some(&client_model),
        true,
        Some("WS"),
        Some(Config::responses_path()),
        None,
        Some(text),
        None,
        None,
    )
    .await;

    apply_selected_model_override(&state, &mut ws_request.request).await;

    let provider = resolve_selected_provider(&state)
        .await
        .map_err(|err| err.message.clone())?;

    if provider.auth_mode == ProviderAuthMode::Account && provider_uses_openai_account(&provider) {
        let account = resolve_account_for_provider(&state, &provider)
            .await
            .map_err(|err| err.message.clone())?;
        if let Err(err) = proxy_openai_private_websocket_request(
            state,
            socket,
            provider,
            account,
            ws_request.request,
            id,
            started_at,
        )
        .await
        {
            socket
                .send(WsMessage::Text(responses_ws_error_event(&err)))
                .await
                .map_err(|send_err| send_err.to_string())?;
        }
        return Ok(());
    }

    let response = match responses_inner(state, ws_request.request, id, started_at).await {
        Ok(response) => response,
        Err(err) => {
            socket
                .send(WsMessage::Text(responses_ws_error_event(&err.message)))
                .await
                .map_err(|send_err| send_err.to_string())?;
            return Ok(());
        }
    };

    if !response.status().is_success() {
        socket
            .send(WsMessage::Text(responses_ws_error_event(&format!(
                "responses request returned {}",
                response.status()
            ))))
            .await
            .map_err(|send_err| send_err.to_string())?;
        return Ok(());
    }

    let mut body = response.into_body();
    let mut sse_buffer = String::new();
    let mut saw_terminal_event = false;

    while let Some(frame_result) = body.frame().await {
        let frame = frame_result.map_err(|err| err.to_string())?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let chunk = String::from_utf8_lossy(&data);
        for event in sse_chunk_to_ws_json_messages(&mut sse_buffer, &chunk) {
            saw_terminal_event |= is_terminal_responses_ws_event(&event);
            socket
                .send(WsMessage::Text(event))
                .await
                .map_err(|send_err| send_err.to_string())?;
        }
    }

    if !saw_terminal_event {
        socket
            .send(WsMessage::Text(responses_ws_error_event(
                "stream closed before response.completed",
            )))
            .await
            .map_err(|send_err| send_err.to_string())?;
    }

    Ok(())
}

async fn proxy_openai_private_websocket_request(
    state: AppState,
    socket: &mut WebSocket,
    provider: ResolvedProvider,
    account: AccountRecord,
    request: ResponsesRequest,
    id: String,
    started_at: Instant,
) -> Result<(), String> {
    let request_body = responses_to_openai_private(&request).map_err(|err| err.to_string())?;
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
        true,
        Some("WS"),
        Some(Config::responses_path()),
        Some(Config::openai_private_responses_ws_url()),
        Some(json_value_for_storage(&request_body)),
        None,
        None,
    )
    .await;

    let request_text = serde_json::to_string(&openai_private_response_create_event(request_body))
        .map_err(|err| format!("serialize websocket request failed: {err}"))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let upstream_client = state.upstream.clone();
    let access_token = account.access_token().to_string();
    let upstream_account_id = account.upstream_account_id().map(str::to_string);
    let request_id = id.clone();
    let worker = tokio::task::spawn_blocking(move || {
        upstream_client.stream_openai_responses_websocket_blocking(
            access_token,
            upstream_account_id,
            request_id,
            request_text,
            tx,
        )
    });

    let mut response_body = String::new();
    let mut final_response_body: Option<String> = None;
    let mut saw_terminal_event = false;

    loop {
        let message = tokio::time::timeout(Config::openai_private_ws_idle_timeout(), rx.recv())
            .await
            .map_err(|_| "idle timeout waiting for upstream websocket event".to_string())?;
        let Some(message) = message else {
            break;
        };
        let text = message?;
        let text = normalize_openai_private_ws_event_for_client(&text);
        append_to_log_buffer(&mut response_body, &text);
        append_to_log_buffer(&mut response_body, "\n");
        capture_final_response_from_ws_event(&text, &mut final_response_body);
        saw_terminal_event |= is_terminal_responses_ws_event(&text);
        socket
            .send(WsMessage::Text(text))
            .await
            .map_err(|err| err.to_string())?;
        if saw_terminal_event {
            break;
        }
    }

    let worker_result = worker
        .await
        .map_err(|err| format!("upstream websocket worker failed: {err}"))?;
    if let Err(err) = worker_result
        && !saw_terminal_event
    {
        return Err(err);
    }

    if !saw_terminal_event {
        socket
            .send(WsMessage::Text(responses_ws_error_event(
                "stream closed before response.completed",
            )))
            .await
            .map_err(|err| err.to_string())?;
    }

    let elapsed = elapsed_ms(started_at);
    let logged_response_body = logged_stream_response_body(final_response_body.as_deref(), &response_body);
    log_http_event(
        &state.logs,
        &id,
        LogStage::ClientResponse,
        Some(StatusCode::OK),
        Some(ClientProtocol::OpenAiResponses.as_str()),
        Some(UpstreamProtocol::OpenAiPrivateResponses.as_str()),
        Some(&provider.name),
        Some(&account.id),
        Some(&account.email),
        Some(&request.model),
        true,
        Some("WS"),
        Some(Config::responses_path()),
        Some(Config::openai_private_responses_ws_url()),
        Some(logged_response_body.clone()),
        None,
        Some(elapsed),
    )
    .await;
    log_http_event(
        &state.logs,
        &id,
        LogStage::UpstreamResponse,
        Some(StatusCode::OK),
        Some(ClientProtocol::OpenAiResponses.as_str()),
        Some(UpstreamProtocol::OpenAiPrivateResponses.as_str()),
        Some(&provider.name),
        Some(&account.id),
        Some(&account.email),
        Some(&request.model),
        true,
        Some("WS"),
        Some(Config::responses_path()),
        Some(Config::openai_private_responses_ws_url()),
        Some(logged_response_body),
        None,
        Some(elapsed),
    )
    .await;

    Ok(())
}

#[derive(Debug)]
pub(crate) struct ResponseCreateWsRequest {
    pub(crate) request: ResponsesRequest,
    pub(crate) generate: bool,
}

pub(crate) fn response_create_ws_message_to_request(text: &str) -> Result<ResponseCreateWsRequest, String> {
    let mut value: Value =
        serde_json::from_str(text).map_err(|err| format!("invalid websocket JSON: {err}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "websocket message must be a JSON object".to_string())?;
    match object.get("type").and_then(Value::as_str) {
        Some("response.create") => {}
        Some(other) => return Err(format!("unsupported websocket message type `{other}`")),
        None => return Err("websocket message is missing `type`".to_string()),
    }

    object.remove("type");
    let generate = object
        .remove("generate")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    object.insert("stream".to_string(), Value::Bool(true));
    object.remove("background");

    let value = crate::models::openai_responses::merge_strict_responses_request_defaults(value);
    let request = serde_json::from_value(value)
        .map_err(|err| format!("invalid response.create payload: {err}"))?;
    Ok(ResponseCreateWsRequest { request, generate })
}

pub(crate) fn openai_private_response_create_event(mut request_body: Value) -> Value {
    let Some(object) = request_body.as_object_mut() else {
        return request_body;
    };
    object.insert(
        "type".to_string(),
        Value::String("response.create".to_string()),
    );
    request_body
}

pub(crate) fn normalize_openai_private_ws_event_for_client(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return text.to_string();
    };
    let Some(event_type) = value.get("type").and_then(Value::as_str) else {
        return text.to_string();
    };
    if event_type != "error" {
        return text.to_string();
    }

    let message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("upstream websocket error");
    responses_ws_error_event(message)
}

pub(crate) fn sse_chunk_to_ws_json_messages(buffer: &mut String, chunk: &str) -> Vec<String> {
    buffer.push_str(chunk);
    let mut messages = Vec::new();

    while let Some(block_end) = buffer.find("\n\n") {
        let block: String = buffer.drain(..block_end + 2).collect();
        let mut data_lines = Vec::new();
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.strip_prefix(' ').unwrap_or(data);
            if data == "[DONE]" {
                data_lines.clear();
                break;
            }
            data_lines.push(data);
        }

        if data_lines.is_empty() {
            continue;
        }
        let payload = data_lines.join("\n");
        if serde_json::from_str::<Value>(&payload).is_ok() {
            messages.push(payload);
        }
    }

    messages
}

pub(crate) fn is_terminal_responses_ws_event(event: &str) -> bool {
    serde_json::from_str::<Value>(event)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|event_type| {
            matches!(
                event_type.as_str(),
                "response.completed" | "response.failed" | "response.incomplete"
            )
        })
}

pub(crate) fn responses_ws_error_event(message: &str) -> String {
    json!({
        "type": "response.failed",
        "response": {
            "id": format!("resp_{}", Uuid::new_v4().simple()),
            "object": "response",
            "status": "failed",
            "error": {
                "message": message,
                "type": "proxy_error"
            }
        }
    })
    .to_string()
}

pub(crate) fn responses_ws_completed_event(model: &str) -> String {
    json!({
        "type": "response.completed",
        "response": {
            "id": format!("resp_{}", Uuid::new_v4().simple()),
            "object": "response",
            "status": "completed",
            "model": model,
            "output": []
        }
    })
    .to_string()
}
