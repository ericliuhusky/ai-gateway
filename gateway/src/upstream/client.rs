use crate::upstream::{openai::OpenAiClient, shared::build_http_client};
use reqwest::Response;

#[derive(Clone, Debug)]
pub struct UpstreamClient {
    openai: OpenAiClient,
}

impl UpstreamClient {
    pub fn new() -> Self {
        let http = build_http_client();
        Self {
            openai: OpenAiClient::new(http),
        }
    }

    pub async fn api_responses_passthrough(
        &self,
        base_url: &str,
        api_key: &str,
        body: String,
        stream: bool,
    ) -> Result<Response, String> {
        self.openai
            .send_api_responses_passthrough(base_url, api_key, body, stream)
            .await
    }

    pub async fn account_responses_passthrough(
        &self,
        access_token: &str,
        upstream_account_id: Option<&str>,
        body: String,
        stream: bool,
    ) -> Result<Response, String> {
        self.openai
            .send_account_responses_passthrough(access_token, upstream_account_id, body, stream)
            .await
    }

    pub async fn api_models(&self, base_url: &str, api_key: &str) -> Result<Response, String> {
        self.openai.send_api_models(base_url, api_key).await
    }

    pub async fn account_models(
        &self,
        access_token: &str,
        upstream_account_id: Option<&str>,
        client_version: Option<&str>,
    ) -> Result<Response, String> {
        self.openai
            .send_account_models(access_token, upstream_account_id, client_version)
            .await
    }

    pub async fn account_usage(
        &self,
        access_token: &str,
        upstream_account_id: Option<&str>,
    ) -> Result<Response, String> {
        self.openai
            .send_account_usage(access_token, upstream_account_id)
            .await
    }
}
