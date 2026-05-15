use crate::upstream::{
    openai::{OpenAiClient, OpenAiEndpoint, OpenAiRequestBuilder},
    shared::build_http_client,
};
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
