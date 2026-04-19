use crate::upstream::shared::{should_try_next_endpoint, truncate_for_log};
use reqwest::{Client, Response};
use serde_json::{Value, json};
use std::sync::OnceLock;
use tracing::{info, warn};
use uuid::Uuid;

const V1_INTERNAL_BASE_URLS: [&str; 3] = [
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal",
    "https://daily-cloudcode-pa.googleapis.com/v1internal",
    "https://cloudcode-pa.googleapis.com/v1internal",
];
const ANTIGRAVITY_CLIENT_VERSION: &str = "4.1.31";
const ANTIGRAVITY_USER_AGENT: &str =
    "Antigravity/4.1.31 (Macintosh; Intel Mac OS X 10_15_7) Chrome/132.0.6834.160 Electron/39.2.3";
pub const GOOGLE_PROJECT_ID_FALLBACK: &str = "bamboo-precept-lgxtn";

fn session_id() -> &'static str {
    static SESSION_ID: OnceLock<String> = OnceLock::new();
    SESSION_ID.get_or_init(|| Uuid::new_v4().to_string())
}

fn machine_id() -> &'static str {
    static MACHINE_ID: OnceLock<String> = OnceLock::new();
    MACHINE_ID.get_or_init(|| Uuid::new_v4().to_string())
}

#[derive(Clone, Debug)]
pub struct GoogleV1InternalClient {
    http: Client,
}

impl GoogleV1InternalClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn fetch_project_id(&self, access_token: &str) -> Result<String, String> {
        let body = json!({
            "metadata": {
                "ideType": "ANTIGRAVITY"
            }
        });

        for base in V1_INTERNAL_BASE_URLS {
            let url = format!("{base}:loadCodeAssist");
            info!(
                url = %url,
                request = %truncate_for_log(&body.to_string(), 2_000),
                "requesting project_id from upstream"
            );
            let response = self
                .http
                .post(&url)
                .bearer_auth(access_token)
                .header("content-type", "application/json")
                .header("x-client-name", "antigravity")
                .header("x-client-version", ANTIGRAVITY_CLIENT_VERSION)
                .header("x-machine-id", machine_id())
                .header("x-vscode-sessionid", session_id())
                .header("user-agent", ANTIGRAVITY_USER_AGENT)
                .json(&body)
                .send()
                .await
                .map_err(|err| format!("loadCodeAssist failed: {err}"))?;

            if response.status().is_success() {
                let value: Value = response
                    .json()
                    .await
                    .map_err(|err| format!("project response parse failed: {err}"))?;
                info!(
                    url = %url,
                    response = %truncate_for_log(&value.to_string(), 2_000),
                    "received project_id response"
                );
                if let Some(project_id) =
                    value.get("cloudaicompanionProject").and_then(Value::as_str)
                {
                    return Ok(project_id.to_string());
                }
            } else {
                warn!(url = %url, status = %response.status(), "loadCodeAssist request failed");
            }
        }

        warn!(
            fallback_project_id = GOOGLE_PROJECT_ID_FALLBACK,
            "failed to fetch cloudaicompanionProject; using fallback project"
        );
        Ok(GOOGLE_PROJECT_ID_FALLBACK.to_string())
    }

    pub async fn fetch_available_models(
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
                .header("x-client-version", ANTIGRAVITY_CLIENT_VERSION)
                .header("x-machine-id", machine_id())
                .header("x-vscode-sessionid", session_id())
                .header("user-agent", ANTIGRAVITY_USER_AGENT)
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
                Err(err) => errors.push(format!("{url} -> request failed: {err}")),
            }
        }

        if errors.is_empty() {
            Err("all Google models endpoints failed".to_string())
        } else {
            Err(errors.join(" | "))
        }
    }

    pub async fn call(
        &self,
        method: &str,
        id: &str,
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
                id = %id,
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
                .header("x-client-version", ANTIGRAVITY_CLIENT_VERSION)
                .header("x-machine-id", machine_id())
                .header("x-vscode-sessionid", session_id())
                .header("user-agent", ANTIGRAVITY_USER_AGENT)
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    info!(
                        id = %id,
                        method = %method,
                        url = %url,
                        status = %resp.status(),
                        "upstream request succeeded"
                    );
                    return Ok(resp);
                }
                Ok(resp) if should_try_next_endpoint(resp.status()) => {
                    let status = resp.status();
                    let response_body = resp.text().await.unwrap_or_default();
                    warn!(
                        id = %id,
                        method = %method,
                        url = %url,
                        status = %status,
                        response = %truncate_for_log(&response_body, 4_000),
                        "upstream endpoint failed, trying next fallback"
                    );
                    errors.push(format!("{url} -> {status}: {response_body}"));
                }
                Ok(resp) => {
                    let status = resp.status();
                    let response_body = resp.text().await.unwrap_or_default();
                    warn!(
                        id = %id,
                        method = %method,
                        url = %url,
                        status = %status,
                        response = %truncate_for_log(&response_body, 4_000),
                        "upstream request failed without fallback"
                    );
                    return Err(format!("upstream returned {status}: {response_body}"));
                }
                Err(err) => {
                    warn!(
                        id = %id,
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
}
