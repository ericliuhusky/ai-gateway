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
}

fn responses_api_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/responses")
    }
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
