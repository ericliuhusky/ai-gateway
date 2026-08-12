use super::{OpenAiEndpoint, OpenAiRequestBuilder};
use reqwest::{Client, Response};

#[derive(Clone, Debug)]
pub struct OpenAiClient {
    http: Client,
}

impl OpenAiClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn send_passthrough<B>(
        &self,
        builder: &B,
        endpoint: OpenAiEndpoint,
    ) -> Result<Response, String>
    where
        B: OpenAiRequestBuilder + ?Sized,
    {
        builder
            .build(&self.http, endpoint)
            .send()
            .await
            .map_err(|err| format!("OpenAI 请求失败: {err}"))
    }

    pub async fn send<B>(&self, builder: &B, endpoint: OpenAiEndpoint) -> Result<Response, String>
    where
        B: OpenAiRequestBuilder + ?Sized,
    {
        let response = self.send_passthrough(builder, endpoint).await?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!("OpenAI 上游返回状态码 {status}: {response_body}"))
        }
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
