use super::{OpenAiEndpoint, OpenAiRequestBuilder, stream_private_responses_websocket_blocking};
use reqwest::{Client, Response};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct OpenAiClient {
    http: Client,
}

impl OpenAiClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn send<B>(&self, builder: &B, endpoint: OpenAiEndpoint) -> Result<Response, String>
    where
        B: OpenAiRequestBuilder + ?Sized,
    {
        let response = builder
            .build(&self.http, endpoint)
            .send()
            .await
            .map_err(|err| format!("OpenAI 请求失败: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!("OpenAI 上游返回状态码 {status}: {response_body}"))
        }
    }

    pub fn stream_responses_websocket_blocking(
        &self,
        access_token: String,
        account_id: Option<String>,
        request_id: String,
        request_text: String,
        tx: mpsc::UnboundedSender<Result<String, String>>,
    ) -> Result<(), String> {
        stream_private_responses_websocket_blocking(
            access_token,
            account_id,
            request_id,
            request_text,
            tx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        OPENAI_CODEX_BASE_URL, OpenAiEndpoint, OpenAiRequestBuilder, PrivateOpenAiRequestBuilder,
    };

    #[test]
    fn openai_models_endpoint_uses_codex_backend() {
        let request = PrivateOpenAiRequestBuilder {
            base_url: OPENAI_CODEX_BASE_URL,
            access_token: "token",
            account_id: None,
            client_version: Some("test"),
        }
        .build(&reqwest::Client::new(), OpenAiEndpoint::Models)
        .build()
        .expect("valid request");

        assert_eq!(
            request.url().as_str(),
            "https://chatgpt.com/backend-api/codex/models?client_version=test"
        );
    }
}
