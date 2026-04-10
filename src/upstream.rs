use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use std::sync::OnceLock;
use tracing::{info, warn};
use uuid::Uuid;

const V1_INTERNAL_BASE_URLS: [&str; 3] = [
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal",
    "https://daily-cloudcode-pa.googleapis.com/v1internal",
    "https://cloudcode-pa.googleapis.com/v1internal",
];
const CLIENT_VERSION: &str = "4.1.31";
const USER_AGENT: &str =
    "Antigravity/4.1.31 (Macintosh; Intel Mac OS X 10_15_7) Chrome/132.0.6834.160 Electron/39.2.3";
const OPENAI_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const OPENAI_MODELS_URL: &str =
    "https://chatgpt.com/backend-api/codex/models?client_version=4.1.31";

fn session_id() -> &'static str {
    static SESSION_ID: OnceLock<String> = OnceLock::new();
    SESSION_ID.get_or_init(|| Uuid::new_v4().to_string())
}

fn machine_id() -> &'static str {
    static MACHINE_ID: OnceLock<String> = OnceLock::new();
    MACHINE_ID.get_or_init(|| Uuid::new_v4().to_string())
}

#[derive(Clone, Debug)]
pub struct UpstreamClient {
    http: Client,
}

impl UpstreamClient {
    pub fn new() -> Self {
        Self {
            http: Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    pub async fn fetch_project_id(&self, access_token: &str) -> Result<String, String> {
        let body = json!({
            "metadata": {
                "ideType": "ANTIGRAVITY"
            }
        });

        for base in V1_INTERNAL_BASE_URLS {
            let url = format!("{base}:loadCodeAssist");
            info!(url = %url, request = %truncate_for_log(&body.to_string(), 2_000), "requesting project_id from upstream");
            let response = self
                .http
                .post(&url)
                .bearer_auth(access_token)
                .header("content-type", "application/json")
                .header("x-client-name", "antigravity")
                .header("x-client-version", CLIENT_VERSION)
                .header("x-machine-id", machine_id())
                .header("x-vscode-sessionid", session_id())
                .header("user-agent", USER_AGENT)
                .json(&body)
                .send()
                .await
                .map_err(|err| format!("loadCodeAssist failed: {err}"))?;

            if response.status().is_success() {
                let value: Value = response
                    .json()
                    .await
                    .map_err(|err| format!("project response parse failed: {err}"))?;
                info!(url = %url, response = %truncate_for_log(&value.to_string(), 2_000), "received project_id response");
                if let Some(project_id) =
                    value.get("cloudaicompanionProject").and_then(Value::as_str)
                {
                    return Ok(project_id.to_string());
                }
            } else {
                warn!(url = %url, status = %response.status(), "loadCodeAssist request failed");
            }
        }

        Err("failed to fetch cloudaicompanionProject".to_string())
    }

    pub async fn fetch_google_available_models(
        &self,
        access_token: &str,
        project_id: Option<&str>,
    ) -> Result<Value, String> {
        let payload = project_id
            .filter(|value| !value.is_empty())
            .map(|project| json!({ "project": project }))
            .unwrap_or_else(|| json!({}));
        let mut errors = Vec::new();

        for base in V1_INTERNAL_BASE_URLS {
            let url = format!("{base}:fetchAvailableModels");
            info!(
                url = %url,
                request = %truncate_for_log(&payload.to_string(), 2_000),
                "requesting Google available models from upstream"
            );

            let response = self
                .http
                .post(&url)
                .bearer_auth(access_token)
                .header("content-type", "application/json")
                .header("x-client-name", "antigravity")
                .header("x-client-version", CLIENT_VERSION)
                .header("x-machine-id", machine_id())
                .header("x-vscode-sessionid", session_id())
                .header("user-agent", USER_AGENT)
                .json(&payload)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let value: Value = resp
                        .json()
                        .await
                        .map_err(|err| format!("google models parse failed: {err}"))?;
                    info!(
                        url = %url,
                        response = %truncate_for_log(&value.to_string(), 4_000),
                        "received Google available models response"
                    );
                    return Ok(value);
                }
                Ok(resp) if should_try_next_endpoint(resp.status()) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    warn!(
                        url = %url,
                        status = %status,
                        response = %truncate_for_log(&body, 4_000),
                        "Google models endpoint failed, trying next fallback"
                    );
                    errors.push(format!("{url} -> {status}: {body}"));
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!("google models upstream returned {status}: {body}"));
                }
                Err(err) => {
                    errors.push(format!("{url} -> request failed: {err}"));
                }
            }
        }

        if errors.is_empty() {
            Err("all Google models endpoints failed".to_string())
        } else {
            Err(errors.join(" | "))
        }
    }

    pub async fn call_v1internal(
        &self,
        method: &str,
        request_id: &str,
        access_token: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        let mut errors = Vec::new();

        for base in V1_INTERNAL_BASE_URLS {
            let url = if stream {
                format!("{base}:{method}?alt=sse")
            } else {
                format!("{base}:{method}")
            };
            info!(
                request_id = %request_id,
                method = %method,
                stream = stream,
                url = %url,
                request = %truncate_for_log(&body.to_string(), 4_000),
                "sending upstream request"
            );

            let response = self
                .http
                .post(&url)
                .bearer_auth(access_token)
                .header("content-type", "application/json")
                .header("x-client-name", "antigravity")
                .header("x-client-version", CLIENT_VERSION)
                .header("x-machine-id", machine_id())
                .header("x-vscode-sessionid", session_id())
                .header("user-agent", USER_AGENT)
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    info!(
                        request_id = %request_id,
                        method = %method,
                        url = %url,
                        status = %resp.status(),
                        "upstream request succeeded"
                    );
                    return Ok(resp);
                }
                Ok(resp) if should_try_next_endpoint(resp.status()) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    warn!(
                        request_id = %request_id,
                        method = %method,
                        url = %url,
                        status = %status,
                        response = %truncate_for_log(&text, 4_000),
                        "upstream endpoint failed, trying next fallback"
                    );
                    errors.push(format!("{url} -> {status}: {text}"));
                    continue;
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    warn!(
                        request_id = %request_id,
                        method = %method,
                        url = %url,
                        status = %status,
                        response = %truncate_for_log(&body, 4_000),
                        "upstream request failed without fallback"
                    );
                    return Err(format!("upstream returned {status}: {body}"));
                }
                Err(err) => {
                    warn!(
                        request_id = %request_id,
                        method = %method,
                        url = %url,
                        error = %err,
                        "upstream transport error"
                    );
                    errors.push(format!("{url} -> request failed: {err}"));
                }
            }
        }

        if errors.is_empty() {
            Err("all v1internal endpoints failed".to_string())
        } else {
            Err(errors.join(" | "))
        }
    }

    pub async fn call_openai_responses(
        &self,
        request_id: &str,
        access_token: &str,
        account_id: Option<&str>,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        info!(
            request_id = %request_id,
            stream = stream,
            url = %OPENAI_RESPONSES_URL,
            request = %truncate_for_log(&body.to_string(), 4_000),
            "sending upstream request to OpenAI"
        );

        let mut request = self
            .http
            .post(OPENAI_RESPONSES_URL)
            .bearer_auth(access_token)
            .header("content-type", "application/json")
            .header(
                "accept",
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .header("user-agent", "CodexBar");

        if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
            request = request.header("ChatGPT-Account-Id", account_id);
        }

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|err| format!("openai request failed: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("openai upstream returned {status}: {body}"))
        }
    }

    pub async fn fetch_openai_models(
        &self,
        request_id: &str,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<Value, String> {
        info!(
            request_id = %request_id,
            url = %OPENAI_MODELS_URL,
            "sending upstream request to OpenAI models"
        );

        let mut request = self
            .http
            .get(OPENAI_MODELS_URL)
            .bearer_auth(access_token)
            .header("accept", "application/json")
            .header("user-agent", USER_AGENT);

        if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
            request = request.header("ChatGPT-Account-Id", account_id);
        }

        let response = request
            .send()
            .await
            .map_err(|err| format!("openai models request failed: {err}"))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|err| format!("openai models parse failed: {err}"))
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("openai models upstream returned {status}: {body}"))
        }
    }

    pub async fn call_native_responses(
        &self,
        request_id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        let url = responses_api_url(base_url);
        info!(
            request_id = %request_id,
            stream = stream,
            url = %url,
            request = %truncate_for_log(&body.to_string(), 4_000),
            "sending upstream request to native provider"
        );

        let response = self
            .http
            .post(&url)
            .bearer_auth(api_key)
            .header("content-type", "application/json")
            .header(
                "accept",
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .json(&body)
            .send()
            .await
            .map_err(|err| format!("native provider request failed: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("native provider returned {status}: {body}"))
        }
    }

    pub async fn call_native_chat_completions(
        &self,
        request_id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
    ) -> Result<Response, String> {
        let url = chat_completions_api_url(base_url);
        info!(
            request_id = %request_id,
            url = %url,
            request = %truncate_for_log(&body.to_string(), 4_000),
            "sending upstream request to native chat completions provider"
        );

        let response = self
            .http
            .post(&url)
            .bearer_auth(api_key)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|err| format!("native chat completions request failed: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("native chat completions returned {status}: {body}"))
        }
    }

    pub async fn fetch_native_models(
        &self,
        request_id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<Value, String> {
        let url = models_api_url(base_url);
        info!(
            request_id = %request_id,
            url = %url,
            "sending upstream request to native provider /models"
        );

        let response = self
            .http
            .get(&url)
            .bearer_auth(api_key)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|err| format!("native models request failed: {err}"))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|err| format!("native models parse failed: {err}"))
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("native models upstream returned {status}: {body}"))
        }
    }
}

fn responses_api_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else if has_api_prefix(trimmed) {
        format!("{trimmed}/responses")
    } else {
        format!("{trimmed}/v1/responses")
    }
}

fn chat_completions_api_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if has_api_prefix(trimmed) {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

fn models_api_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/models") {
        trimmed.to_string()
    } else if has_api_prefix(trimmed) {
        format!("{trimmed}/models")
    } else {
        format!("{trimmed}/v1/models")
    }
}

fn has_api_prefix(base_url: &str) -> bool {
    base_url.ends_with("/v1") || base_url.contains("/api/") || base_url.ends_with("/api")
}

fn should_try_next_endpoint(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::NOT_FOUND
        || status.is_server_error()
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

#[cfg(test)]
mod tests {
    use super::{OPENAI_MODELS_URL, chat_completions_api_url, models_api_url, responses_api_url};

    #[test]
    fn appends_models_to_v1_base_url() {
        assert_eq!(
            models_api_url("https://ark.cn-beijing.volces.com/api/v3"),
            "https://ark.cn-beijing.volces.com/api/v3/models"
        );
    }

    #[test]
    fn appends_models_to_plain_base_url() {
        assert_eq!(
            models_api_url("https://api.xcode.best"),
            "https://api.xcode.best/v1/models"
        );
    }

    #[test]
    fn preserves_existing_endpoint_paths() {
        assert_eq!(
            responses_api_url("https://example.com/v1/responses"),
            "https://example.com/v1/responses"
        );
        assert_eq!(
            chat_completions_api_url("https://example.com/v1/chat/completions"),
            "https://example.com/v1/chat/completions"
        );
        assert_eq!(
            models_api_url("https://example.com/v1/models"),
            "https://example.com/v1/models"
        );
    }

    #[test]
    fn openai_models_endpoint_uses_codex_backend() {
        assert_eq!(
            OPENAI_MODELS_URL,
            "https://chatgpt.com/backend-api/codex/models?client_version=4.1.31"
        );
    }
}
