use crate::upstream::{
    google_v1internal::GoogleV1InternalClient,
    openai::{OpenAiClient, OpenAiEndpoint, OpenAiRequestBuilder},
    shared::build_http_client,
};
use reqwest::Response;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct UpstreamClient {
    google_v1internal: GoogleV1InternalClient,
    openai: OpenAiClient,
}

impl UpstreamClient {
    pub fn new() -> Self {
        let http = build_http_client();
        Self {
            google_v1internal: GoogleV1InternalClient::new(http.clone()),
            openai: OpenAiClient::new(http),
        }
    }

    pub async fn fetch_project_id(&self, access_token: &str) -> Result<String, String> {
        self.google_v1internal.fetch_project_id(access_token).await
    }

    pub async fn fetch_google_available_models(
        &self,
        access_token: &str,
        project_id: Option<&str>,
    ) -> Result<Value, String> {
        self.google_v1internal
            .fetch_available_models(access_token, project_id)
            .await
    }

    pub async fn call_v1internal(
        &self,
        method: &str,
        id: &str,
        access_token: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        self.google_v1internal
            .call(method, id, access_token, body, stream)
            .await
    }

    pub async fn openai_send<B>(
        &self,
        builder: &B,
        endpoint: OpenAiEndpoint,
    ) -> Result<Response, String>
    where
        B: OpenAiRequestBuilder + ?Sized,
    {
        self.openai.send(builder, endpoint).await
    }
}
