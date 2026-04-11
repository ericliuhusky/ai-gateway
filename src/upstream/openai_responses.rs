use crate::upstream::shared::{has_api_prefix, truncate_for_log};
use reqwest::{Client, Response};
use serde_json::Value;
use tracing::info;

#[derive(Clone, Debug)]
pub struct OpenAiResponsesClient {
    http: Client,
}

impl OpenAiResponsesClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn call(
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
            "sending upstream request to OpenAI responses provider"
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
        request_id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<Value, String> {
        let url = models_api_url(base_url);
        info!(
            request_id = %request_id,
            url = %url,
            "sending upstream request to OpenAI provider /models"
        );

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

pub fn responses_api_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else if has_api_prefix(trimmed) {
        format!("{trimmed}/responses")
    } else {
        format!("{trimmed}/v1/responses")
    }
}

pub fn models_api_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/models") {
        trimmed.to_string()
    } else if has_api_prefix(trimmed) {
        format!("{trimmed}/models")
    } else {
        format!("{trimmed}/v1/models")
    }
}

#[cfg(test)]
mod tests {
    use super::{models_api_url, responses_api_url};

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
    fn preserves_existing_responses_and_models_paths() {
        assert_eq!(
            responses_api_url("https://example.com/v1/responses"),
            "https://example.com/v1/responses"
        );
        assert_eq!(
            models_api_url("https://example.com/v1/models"),
            "https://example.com/v1/models"
        );
    }
}
