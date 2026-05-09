use crate::upstream::shared::has_api_prefix;
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

    pub async fn call(
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

pub fn chat_completions_api_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if has_api_prefix(trimmed) {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

#[cfg(test)]
mod tests {
    use super::chat_completions_api_url;

    #[test]
    fn preserves_existing_chat_completions_path() {
        assert_eq!(
            chat_completions_api_url("https://example.com/v1/chat/completions"),
            "https://example.com/v1/chat/completions"
        );
    }
}
