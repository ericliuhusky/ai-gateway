use super::url::chat_completions_api_url;
use reqwest::{Client, Response};
use serde_json::Value;
#[derive(Clone, Debug)]
pub struct OpenAiChatClient {
    http: Client,
}

impl OpenAiChatClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn request(
        &self,
        _id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
    ) -> Result<Response, String> {
        let url = chat_completions_api_url(base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(api_key)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|err| format!("openai chat request failed: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "openai chat provider returned {status}: {response_body}"
            ))
        }
    }
}

