use crate::upstream::{
    google_v1internal::GoogleV1InternalClient,
    openai::{OpenAiChatClient, OpenAiPrivateClient, OpenAiResponsesClient},
    shared::build_http_client,
};
use reqwest::Response;
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct UpstreamClient {
    google_v1internal: GoogleV1InternalClient,
    openai_private: OpenAiPrivateClient,
    openai_responses: OpenAiResponsesClient,
    openai_chat: OpenAiChatClient,
}

impl UpstreamClient {
    pub fn new() -> Self {
        let http = build_http_client();
        Self {
            google_v1internal: GoogleV1InternalClient::new(http.clone()),
            openai_private: OpenAiPrivateClient::new(http.clone()),
            openai_responses: OpenAiResponsesClient::new(http.clone()),
            openai_chat: OpenAiChatClient::new(http),
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

    pub async fn call_openai_responses(
        &self,
        id: &str,
        access_token: &str,
        account_id: Option<&str>,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        self.openai_private
            .call_responses(id, access_token, account_id, body, stream)
            .await
    }

    pub async fn fetch_openai_models(
        &self,
        id: &str,
        access_token: &str,
        account_id: Option<&str>,
        client_version: &str,
    ) -> Result<Value, String> {
        self.openai_private
            .fetch_models(id, access_token, account_id, client_version)
            .await
    }

    pub async fn fetch_openai_usage(
        &self,
        id: &str,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<Value, String> {
        self.openai_private
            .fetch_usage(id, access_token, account_id)
            .await
    }

    pub fn stream_openai_responses_websocket_blocking(
        &self,
        access_token: String,
        account_id: Option<String>,
        request_id: String,
        request_text: String,
        tx: mpsc::UnboundedSender<Result<String, String>>,
    ) -> Result<(), String> {
        self.openai_private.stream_responses_websocket_blocking(
            access_token,
            account_id,
            request_id,
            request_text,
            tx,
        )
    }

    pub async fn call_openai_responses_upstream(
        &self,
        id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        self.openai_responses
            .request(id, base_url, api_key, body, stream)
            .await
    }

    pub async fn call_openai_chat_upstream(
        &self,
        id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
    ) -> Result<Response, String> {
        self.openai_chat.request(id, base_url, api_key, body).await
    }

    pub async fn fetch_openai_models_upstream(
        &self,
        id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<Value, String> {
        self.openai_responses
            .fetch_models(id, base_url, api_key)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::upstream::openai::OPENAI_MODELS_URL;
    #[test]
    fn openai_models_endpoint_uses_codex_backend() {
        assert_eq!(
            OPENAI_MODELS_URL,
            "https://chatgpt.com/backend-api/codex/models"
        );
    }
}
