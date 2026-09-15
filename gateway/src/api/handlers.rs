use super::dto::{CreateProviderReq, ProviderSummaryResp, UpdateRouteRequest};
use crate::{
    config::DEFAULT_CODEX_CLIENT_VERSION,
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
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const GATEWAY_ERROR_PREFIX: &str = "AI网关错误：";
const UPSTREAM_ERROR_PREFIX: &str = "上游服务错误：";

#[derive(Clone)]
pub struct AppState {
    pub openai_tokens: OpenAiTokenService,
    pub openai_device_login: OpenAiDeviceLoginService,
    pub providers: ProviderStore,
    pub routes: RouteStore,
    pub upstream: OpenAiClient,
    pub raw_provider_traffic: RawProviderTrafficState,
}

const RAW_PROVIDER_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// In-memory inspection state for the most recent provider exchange.
///
/// This is deliberately opt-in and never persisted: provider payloads can
/// contain private conversations, tool arguments, or other sensitive data.
#[derive(Clone, Default)]
pub struct RawProviderTrafficState {
    enabled: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<Arc<Mutex<RawProviderTrafficRecord>>>>>,
}

struct RawProviderTrafficRecord {
    provider_id: String,
    provider_name: String,
    request: Value,
    response_status: Option<u16>,
    response_raw: Vec<u8>,
    response_buffer: Vec<u8>,
    response_output_text: String,
    items: Vec<RawProviderTrafficItem>,
    response_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RawProviderTrafficItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub direction: String,
    pub item: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct RawProviderTrafficView {
    pub provider_id: String,
    pub provider_name: String,
    pub request: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_output_text: Option<String>,
    pub items: Vec<RawProviderTrafficItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<u16>,
    pub response_truncated: bool,
}

impl RawProviderTrafficState {
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn begin(
        &self,
        provider_id: &str,
        provider_name: &str,
        request_body: &[u8],
    ) -> Option<RawProviderTrafficHandle> {
        if !self.enabled() {
            return None;
        }
        let request = serde_json::from_slice(request_body).ok()?;
        let record = Arc::new(Mutex::new(RawProviderTrafficRecord {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            request: sanitized_value(&request),
            response_status: None,
            response_raw: Vec::new(),
            response_buffer: Vec::new(),
            response_output_text: String::new(),
            items: request_items(&request),
            response_truncated: false,
        }));
        if let Ok(mut latest) = self.latest.lock() {
            *latest = Some(record.clone());
            Some(RawProviderTrafficHandle { record })
        } else {
            None
        }
    }

    pub fn view(&self) -> Option<RawProviderTrafficView> {
        let record = self.latest.lock().ok()?.as_ref()?.clone();
        let record = record.lock().ok()?;
        let response_raw = if record.response_raw.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&record.response_raw).into_owned())
        };
        let response = response_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        let response_output_text = if record.response_output_text.is_empty() {
            None
        } else {
            Some(record.response_output_text.clone())
        };
        Some(RawProviderTrafficView {
            provider_id: record.provider_id.clone(),
            provider_name: record.provider_name.clone(),
            request: record.request.clone(),
            response,
            response_raw,
            response_output_text,
            items: record.items.clone(),
            response_status: record.response_status,
            response_truncated: record.response_truncated,
        })
    }
}

pub struct RawProviderTrafficHandle {
    record: Arc<Mutex<RawProviderTrafficRecord>>,
}

impl RawProviderTrafficHandle {
    pub fn set_response_status(&self, status: u16) {
        if let Ok(mut record) = self.record.lock() {
            record.response_status = Some(status);
        }
    }

    pub fn append_response(&self, chunk: &[u8]) {
        if let Ok(mut record) = self.record.lock() {
            record.response_buffer.extend_from_slice(chunk);
            if record.response_buffer.len() > RAW_PROVIDER_RESPONSE_MAX_BYTES {
                record.response_buffer.clear();
                record.response_truncated = true;
                return;
            }

            while let Some(newline) = record
                .response_buffer
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let line = record.response_buffer.drain(..=newline).collect::<Vec<_>>();
                if let Some(payload) = line.strip_prefix(b"data:") {
                    if let Ok(value) = serde_json::from_slice::<Value>(payload) {
                        process_response_event(&mut record, value);
                    }
                }
            }

            if record.response_buffer.first() == Some(&b'{')
                && let Ok(value) = serde_json::from_slice::<Value>(&record.response_buffer)
            {
                record.response_buffer.clear();
                process_response_event(&mut record, value);
            }
        }
    }
}

const EXCLUDED_ITEM_TYPES: &[&str] = &["reasoning", "handoff"];

fn classify_item(value: &Value) -> Option<&'static str> {
    let item_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    match item_type {
        "message" => match value.get("role").and_then(Value::as_str) {
            Some("user") => Some("user_message"),
            Some("assistant") => Some("assistant_message"),
            _ => None,
        },
        "function_call" => Some("function_call"),
        "tool_call" => Some("tool_call"),
        "custom_tool_call" => Some("custom_tool_call"),
        "mcp_call" => Some("mcp_call"),
        "code_interpreter_call" => Some("code_interpreter_call"),
        "local_shell_call" | "shell_call" => Some("shell_call"),
        "image_generation_call" => Some("image_generation_call"),
        "apply_patch_call" => Some("apply_patch_call"),
        "function_call_output" => Some("function_call_output"),
        "tool_result" => Some("tool_result"),
        "approval_request" | "mcp_approval_request" => Some("approval_request"),
        "approval_response" | "mcp_approval_response" => Some("approval_response"),
        "file_search" | "file_search_call" => Some("file_search"),
        "web_search" | "web_search_call" => Some("web_search"),
        "computer_action" | "computer_call" | "computer_call_output" => Some("computer_action"),
        "error" => Some("error"),
        _ if EXCLUDED_ITEM_TYPES.contains(&item_type) => None,
        _ => None,
    }
}

fn sanitized_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .iter()
                .filter_map(|value| {
                    if value
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|item_type| EXCLUDED_ITEM_TYPES.contains(&item_type))
                    {
                        None
                    } else {
                        Some(sanitized_value(value))
                    }
                })
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "usage" | "metadata" | "reasoning"))
                .map(|(key, value)| (key.clone(), sanitized_value(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn push_item(
    record: &mut RawProviderTrafficRecord,
    direction: &str,
    item_type: &str,
    mut item: Value,
) {
    let item_id = item.get("id").and_then(Value::as_str);
    if let Some(index) = record.items.iter().position(|existing| {
        existing.direction == direction
            && existing.item_type == item_type
            && item_id.is_some()
            && existing.item.get("id").and_then(Value::as_str) == item_id
    }) {
        merge_tool_item_fields(&record.items[index].item, &mut item);
        record.items[index].item = item;
        return;
    }

    if record.items.iter().any(|existing| {
        existing.direction == direction && existing.item_type == item_type && existing.item == item
    }) {
        return;
    }

    record.items.push(RawProviderTrafficItem {
        item_type: item_type.to_string(),
        direction: direction.to_string(),
        item,
    });
}

fn merge_tool_item_fields(existing: &Value, incoming: &mut Value) {
    let is_tool_item = existing
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|item_type| item_type.contains("call"));
    if !is_tool_item {
        return;
    }
    let Some(existing_object) = existing.as_object() else {
        return;
    };
    let Some(incoming_object) = incoming.as_object_mut() else {
        return;
    };
    for key in ["arguments", "input"] {
        let incoming_value = incoming_object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default();
        let existing_value = existing_object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default();
        if incoming_value.is_empty() && !existing_value.is_empty() {
            incoming_object.insert(key.to_string(), Value::String(existing_value.to_string()));
        }
    }
}

fn append_classified_item(record: &mut RawProviderTrafficRecord, direction: &str, item: &Value) {
    if let Some(item_type) = classify_item(item) {
        push_item(record, direction, item_type, sanitized_value(item));
    }
}

fn request_items(request: &Value) -> Vec<RawProviderTrafficItem> {
    let mut record = RawProviderTrafficRecord {
        provider_id: String::new(),
        provider_name: String::new(),
        request: Value::Null,
        response_status: None,
        response_raw: Vec::new(),
        response_buffer: Vec::new(),
        response_output_text: String::new(),
        items: Vec::new(),
        response_truncated: false,
    };
    let input = request.get("input").or_else(|| request.get("messages"));
    match input {
        Some(Value::Array(items)) => {
            for item in items {
                append_classified_item(&mut record, "request", item);
            }
        }
        Some(Value::String(text)) if !text.is_empty() => {
            push_item(
                &mut record,
                "request",
                "user_message",
                Value::String(text.clone()),
            );
        }
        Some(value) if value.is_object() => append_classified_item(&mut record, "request", value),
        _ => {}
    }
    record.items
}

fn process_response_event(record: &mut RawProviderTrafficRecord, value: Value) {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                record.response_output_text.push_str(delta);
            }
        }
        "response.function_call_arguments.delta"
        | "response.custom_tool_call_input.delta"
        | "response.mcp_call_arguments.delta" => {
            append_tool_call_delta(record, event_type, &value);
        }
        "response.function_call_arguments.done" => {
            update_tool_call_value(record, &value, "arguments");
        }
        "response.custom_tool_call_input.done" | "response.mcp_call_arguments.done" => {
            update_tool_call_value(record, &value, "input");
        }
        "response.output_item.added" | "response.output_item.done" => {
            if let Some(item) = value.get("item") {
                append_classified_item(record, "response", item);
            }
        }
        "response.completed" | "response" => {
            if let Some(response) = value.get("response") {
                if let Some(output) = response.get("output").and_then(Value::as_array) {
                    for item in output {
                        append_classified_item(record, "response", item);
                    }
                }
                append_response_output_text(record, response);
                push_item(
                    record,
                    "response",
                    "final_response",
                    sanitized_value(response),
                );
                record.response_raw =
                    serde_json::to_vec(&sanitized_value(&value)).unwrap_or_default();
            } else if event_type == "response" {
                if let Some(output) = value.get("output").and_then(Value::as_array) {
                    for item in output {
                        append_classified_item(record, "response", item);
                    }
                }
                append_response_output_text(record, &value);
                push_item(
                    record,
                    "response",
                    "final_response",
                    sanitized_value(&value),
                );
                record.response_raw =
                    serde_json::to_vec(&sanitized_value(&value)).unwrap_or_default();
            }
        }
        "response.error" | "response.failed" | "error" => {
            push_item(record, "response", "error", sanitized_value(&value));
        }
        _ => {
            if event_type.ends_with(".error") {
                push_item(record, "response", "error", sanitized_value(&value));
            }
        }
    }

    if event_type.is_empty() && value.get("output").is_some() {
        if let Some(output) = value.get("output").and_then(Value::as_array) {
            for item in output {
                append_classified_item(record, "response", item);
            }
        }
        append_response_output_text(record, &value);
        push_item(
            record,
            "response",
            "final_response",
            sanitized_value(&value),
        );
        record.response_raw = serde_json::to_vec(&sanitized_value(&value)).unwrap_or_default();
    } else if event_type.is_empty() && value.get("error").is_some() {
        push_item(record, "response", "error", sanitized_value(&value));
    }
}

fn response_item_id(value: &Value) -> Option<String> {
    ["item_id", "output_item_id", "id"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_string))
}

fn append_tool_call_delta(record: &mut RawProviderTrafficRecord, event_type: &str, value: &Value) {
    let Some(delta) = value.get("delta").and_then(Value::as_str) else {
        return;
    };
    let item_id = response_item_id(value);
    let item_type = if event_type.contains("function_call") {
        "function_call"
    } else if event_type.contains("mcp_call") {
        "mcp_call"
    } else {
        "custom_tool_call"
    };
    let field = if event_type.contains("function_call") {
        "arguments"
    } else {
        "input"
    };

    let existing_index = item_id.as_deref().and_then(|id| {
        record.items.iter().position(|item| {
            item.direction == "response" && item.item.get("id").and_then(Value::as_str) == Some(id)
        })
    });
    if let Some(index) = existing_index {
        if let Some(object) = record.items[index].item.as_object_mut() {
            let current = object
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            object.insert(
                field.to_string(),
                Value::String(format!("{current}{delta}")),
            );
        }
        return;
    }

    let mut item = json!({ "type": item_type });
    let object = item.as_object_mut().expect("tool call item is an object");
    object.insert(field.to_string(), Value::String(delta.to_string()));
    if let Some(id) = item_id {
        object.insert("id".to_string(), Value::String(id));
    }
    push_item(record, "response", item_type, item);
}

fn update_tool_call_value(record: &mut RawProviderTrafficRecord, value: &Value, field: &str) {
    let Some(item_id) = response_item_id(value) else {
        return;
    };
    let Some(argument) = value.get(field).and_then(Value::as_str) else {
        return;
    };
    if let Some(item) = record.items.iter_mut().find(|item| {
        item.direction == "response"
            && item.item.get("id").and_then(Value::as_str) == Some(item_id.as_str())
    }) {
        if let Some(object) = item.item.as_object_mut() {
            object.insert(field.to_string(), Value::String(argument.to_string()));
        }
    }
}

fn append_response_output_text(record: &mut RawProviderTrafficRecord, response: &Value) {
    if !record.response_output_text.is_empty() {
        return;
    }
    let Some(output) = response.get("output").and_then(Value::as_array) else {
        return;
    };
    let mut text = String::new();
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for part in content {
            if matches!(
                part.get("type").and_then(Value::as_str),
                Some("output_text") | Some("text")
            ) {
                if let Some(value) = part.get("text").and_then(Value::as_str) {
                    text.push_str(value);
                }
            }
        }
    }
    if !text.is_empty() {
        record.response_output_text = text;
    }
}

#[derive(Debug, Deserialize)]
pub struct RawProviderTrafficSettings {
    pub enabled: Option<bool>,
}

pub async fn get_raw_provider_traffic(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "enabled": state.raw_provider_traffic.enabled(),
        "traffic": state.raw_provider_traffic.view(),
    }))
}

pub async fn set_raw_provider_traffic(
    State(state): State<AppState>,
    Json(settings): Json<RawProviderTrafficSettings>,
) -> Json<Value> {
    if let Some(enabled) = settings.enabled {
        state.raw_provider_traffic.set_enabled(enabled);
    }
    Json(json!({
        "enabled": state.raw_provider_traffic.enabled(),
        "traffic": state.raw_provider_traffic.view(),
    }))
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
    refresh_token: String,
}

fn import_tokens_from_value(value: Value) -> Result<CodexAuthTokensFile, String> {
    let entries = value
        .as_array()
        .ok_or_else(|| "导入内容必须是包含一个账号的 JSON 数组".to_string())?;
    let entry = entries
        .first()
        .ok_or_else(|| "导入 JSON 不包含账号".to_string())?;
    if entries.len() != 1 {
        return Err("只支持导入一个账号，请确保 JSON 数组只包含一项".to_string());
    }
    serde_json::from_value(entry.clone())
        .map_err(|error| format!("导入 JSON 格式无效，需要 access_token 和 refresh_token: {error}"))
}

#[derive(Debug, Serialize)]
pub struct ImportOpenAiFromLocalResponse {
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

/// Import one OpenAI account from a one-item Cockpit Tools export.
pub async fn import_openai_token(
    State(state): State<AppState>,
    Query(query): Query<ReplaceProviderQuery>,
    Json(payload): Json<Value>,
) -> Result<Json<ImportOpenAiFromLocalResponse>, AppError> {
    let tokens = import_tokens_from_value(payload).map_err(AppError::bad_request)?;
    if tokens.access_token.trim().is_empty() || tokens.refresh_token.trim().is_empty() {
        return Err(AppError::bad_request(
            "access_token 和 refresh_token 不能为空",
        ));
    }
    let imported = state
        .openai_tokens
        .import_codex_tokens(tokens.access_token, tokens.refresh_token)
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

    Ok(Json(ImportOpenAiFromLocalResponse {
        email: provider.email().unwrap_or_default().to_string(),
        provider_id: provider.id().to_string(),
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
                if !query.replace
                    && state
                        .providers
                        .account_exists(&email)
                        .map_err(AppError::internal)?
                {
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

pub async fn list_providers(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let providers = state.providers.list().map_err(AppError::internal)?;
    Ok(Json(json!({ "providers": providers })))
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
) -> Result<(StatusCode, Json<ProviderSummaryResp>), AppError> {
    let provider = state
        .providers
        .upsert(request)
        .await
        .map_err(AppError::bad_request)?;

    Ok((
        StatusCode::CREATED,
        Json(ProviderSummaryResp::from(&provider)),
    ))
}

pub async fn delete_provider(
    State(state): State<AppState>,
    AxumPath(provider_id): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let deleted = state
        .providers
        .delete(&provider_id)
        .map_err(AppError::bad_request)?;

    Ok(Json(json!({
        "deleted_provider": {
            "id": deleted.id,
            "name": deleted.name(),
        }
    })))
}

pub async fn get_route(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "route": selected_route(&state).await? })))
}

pub async fn set_route(
    State(state): State<AppState>,
    Json(request): Json<UpdateRouteRequest>,
) -> Result<Json<Value>, AppError> {
    let provider_id = normalize_selected_provider_id(request.provider_id)?;
    resolve_provider_by_id(&state, &provider_id).await?;
    let model = request.model.map(normalize_model).transpose()?;
    let reasoning_effort = request
        .reasoning_effort
        .map(normalize_reasoning_effort)
        .transpose()?;
    let route = state
        .routes
        .update(
            Some(provider_id),
            model,
            reasoning_effort,
            request.use_saved_preferences,
        )
        .map_err(AppError::internal)?;
    Ok(Json(json!({ "route": route })))
}

async fn resolve_selected_provider(state: &AppState) -> Result<Provider, AppError> {
    let route = selected_route(state).await?;
    if let Some(provider_id) = route.provider_id {
        return resolve_provider_by_id(state, &provider_id).await;
    }
    Err(no_provider_selected_error())
}

pub(super) fn no_provider_selected_error() -> AppError {
    AppError::bad_request("尚未选择供应商；请先调用 PUT /management/route")
}

pub(super) async fn selected_route(state: &AppState) -> Result<SelectedRoute, AppError> {
    state.routes.get().map_err(AppError::internal)
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

fn normalize_selected_provider_id(provider_id: String) -> Result<String, AppError> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("provider_id 不能为空"));
    }
    Ok(trimmed.to_string())
}

fn normalize_model(model: String) -> Result<String, AppError> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("模型不能为空"));
    }
    Ok(trimmed.to_string())
}

fn normalize_reasoning_effort(effort: String) -> Result<String, AppError> {
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
        .map_err(AppError::internal)?
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

#[cfg(test)]
mod tests {
    use super::{RawProviderTrafficState, import_tokens_from_value};
    use serde_json::{Value, json};

    #[test]
    fn token_import_accepts_exactly_one_flat_array_entry() {
        let tokens = import_tokens_from_value(json!([{
            "access_token": "access",
            "refresh_token": "refresh",
            "type": "codex"
        }]))
        .unwrap();
        assert_eq!(tokens.access_token, "access");
        assert_eq!(tokens.refresh_token, "refresh");
    }

    #[test]
    fn token_import_rejects_old_and_batch_formats() {
        assert!(
            import_tokens_from_value(json!({
                "tokens": { "access_token": "access", "refresh_token": "refresh" }
            }))
            .is_err()
        );
        assert!(
            import_tokens_from_value(json!([
                { "access_token": "one", "refresh_token": "one" },
                { "access_token": "two", "refresh_token": "two" }
            ]))
            .is_err()
        );
    }

    #[test]
    fn raw_provider_traffic_keeps_selected_items_and_total_text() {
        let state = RawProviderTrafficState::default();
        state.set_enabled(true);
        let traffic = state
            .begin("provider-1", "Mock Provider", br#"{"input":"hello"}"#)
            .expect("debug recording is enabled");

        assert_eq!(
            state
                .view()
                .expect("traffic view")
                .items
                .first()
                .map(|item| item.item_type.as_str()),
            Some("user_message")
        );

        traffic.append_response(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
        );
        let partial = state.view().expect("traffic view");
        assert!(partial.response.is_none());
        assert_eq!(partial.response_output_text.as_deref(), Some("hello"));

        traffic.append_response(
            b"data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"msg-1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
        );
        traffic.append_response(
            b"data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"call-1\",\"type\":\"function_call\",\"name\":\"lookup_weather\",\"arguments\":\"\"}}\n\n",
        );
        traffic.append_response(
            b"data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"call-1\",\"delta\":\"{\\\"city\\\":\\\"Beijing\\\"}\"}\n\n",
        );
        traffic.append_response(
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-1\",\"metadata\":{\"secret\":true},\"usage\":{\"total_tokens\":1},\"output\":[{\"id\":\"msg-1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]},{\"type\":\"reasoning\",\"encrypted_content\":\"secret\"}]}}\n\n",
        );
        let view = state.view().expect("traffic view");
        assert_eq!(view.response_output_text.as_deref(), Some("hello"));
        let function_call = view
            .items
            .iter()
            .find(|item| item.item_type == "function_call")
            .expect("function call");
        assert_eq!(
            function_call.item.get("arguments").and_then(Value::as_str),
            Some("{\"city\":\"Beijing\"}")
        );
        assert_eq!(
            view.items
                .iter()
                .filter(|item| item.item_type == "assistant_message")
                .count(),
            1
        );
        assert!(view.items.iter().all(|item| item.item_type != "reasoning"));
        assert!(view.items.iter().all(|item| item.item_type != "handoff"));
        assert!(
            view.items
                .iter()
                .any(|item| item.item_type == "final_response")
        );
        let final_response = view
            .items
            .iter()
            .find(|item| item.item_type == "final_response")
            .expect("final response");
        assert!(final_response.item.get("metadata").is_none());
        assert!(final_response.item.get("usage").is_none());
        assert!(final_response.item.get("reasoning").is_none());

        state.set_enabled(false);
        assert!(state.view().is_some());
    }
}
