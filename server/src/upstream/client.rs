use crate::upstream::{
    google_v1internal::GoogleV1InternalClient,
    openai::{OpenAiClient, OpenAiEndpoint, OpenAiRequestBuilder},
    shared::build_http_client,
};
use reqwest::Response;
use serde_json::Value;
use tokio::sync::mpsc;

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

    pub fn stream_openai_responses_websocket_blocking(
        &self,
        access_token: String,
        account_id: Option<String>,
        request_id: String,
        request_text: String,
        tx: mpsc::UnboundedSender<Result<String, String>>,
    ) -> Result<(), String> {
        self.openai.stream_responses_websocket_blocking(
            access_token,
            account_id,
            request_id,
            request_text,
            tx,
        )
    }
}
