use crate::upstream::shared::truncate_for_log;
use reqwest::{Client, Response};
use serde_json::Value;
use tracing::info;

pub const OPENAI_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub const OPENAI_MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";

#[derive(Clone, Debug)]
pub struct OpenAiPrivateClient {
    http: Client,
}

impl OpenAiPrivateClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn call_responses(
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
            let response_body = response.text().await.unwrap_or_default();
            Err(format!("openai upstream returned {status}: {response_body}"))
        }
    }

    pub async fn fetch_models(
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
            .header("accept", "application/json");

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
            let response_body = response.text().await.unwrap_or_default();
            Err(format!("openai models upstream returned {status}: {response_body}"))
        }
    }
}
