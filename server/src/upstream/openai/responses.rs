use super::url::{models_api_url, responses_api_url};
use reqwest::{Client, Response};
use serde_json::Value;
#[derive(Clone, Debug)]
pub struct OpenAiResponsesClient {
    http: Client,
}

impl OpenAiResponsesClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn request(
        &self,
        _id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        let url = responses_api_url(base_url);
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
            .map_err(|err| format!("openai responses request failed: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "openai responses provider returned {status}: {response_body}"
            ))
        }
    }

    pub async fn fetch_models(
        &self,
        _id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<Value, String> {
        let url = models_api_url(base_url);
        let response = self
            .http
            .get(&url)
            .bearer_auth(api_key)
            .header("accept", "application/json")
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
            Err(format!(
                "openai models upstream returned {status}: {response_body}"
            ))
        }
    }
}
