use crate::upstream::{
    google_v1internal::GoogleV1InternalClient, openai_chat::OpenAiChatClient,
    new_api_site::NewApiSiteClient,
    openai_private::OpenAiPrivateClient, openai_responses::OpenAiResponsesClient,
    shared::build_http_client,
};
use reqwest::Response;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct UpstreamClient {
    google_v1internal: GoogleV1InternalClient,
    new_api_site: NewApiSiteClient,
    openai_private: OpenAiPrivateClient,
    openai_responses: OpenAiResponsesClient,
    openai_chat: OpenAiChatClient,
}

impl UpstreamClient {
    pub fn new() -> Self {
        let http = build_http_client();
        Self {
            google_v1internal: GoogleV1InternalClient::new(http.clone()),
            new_api_site: NewApiSiteClient::new(http.clone()),
            openai_private: OpenAiPrivateClient::new(http.clone()),
            openai_responses: OpenAiResponsesClient::new(http.clone()),
            openai_chat: OpenAiChatClient::new(http),
        }
    }

    pub async fn fetch_new_api_user_self(
        &self,
        request_id: &str,
        token: &str,
        user_id: &str,
    ) -> Result<Value, String> {
        self.new_api_site
            .fetch_user_self(request_id, token, user_id)
            .await
    }

    pub async fn fetch_new_api_subscription_self(
        &self,
        request_id: &str,
        token: &str,
        user_id: &str,
    ) -> Result<Value, String> {
        self.new_api_site
            .fetch_subscription_self(request_id, token, user_id)
            .await
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
        request_id: &str,
        access_token: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        self.google_v1internal
            .call(method, request_id, access_token, body, stream)
            .await
    }

    pub async fn call_openai_responses(
        &self,
        request_id: &str,
        access_token: &str,
        account_id: Option<&str>,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        self.openai_private
            .call_responses(request_id, access_token, account_id, body, stream)
            .await
    }

    pub async fn fetch_openai_models(
        &self,
        request_id: &str,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<Value, String> {
        self.openai_private
            .fetch_models(request_id, access_token, account_id)
            .await
    }

    pub async fn fetch_openai_usage(
        &self,
        request_id: &str,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<Value, String> {
        self.openai_private
            .fetch_usage(request_id, access_token, account_id)
            .await
    }

    pub async fn call_openai_responses_upstream(
        &self,
        request_id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        self.openai_responses
            .call(request_id, base_url, api_key, body, stream)
            .await
    }

    pub async fn call_openai_chat_upstream(
        &self,
        request_id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
    ) -> Result<Response, String> {
        self.openai_chat
            .call(request_id, base_url, api_key, body)
            .await
    }

    pub async fn fetch_openai_models_upstream(
        &self,
        request_id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<Value, String> {
        self.openai_responses
            .fetch_models(request_id, base_url, api_key)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::upstream::openai_private::OPENAI_MODELS_URL;
    #[test]
    fn openai_models_endpoint_uses_codex_backend() {
        assert_eq!(
            OPENAI_MODELS_URL,
            "https://chatgpt.com/backend-api/codex/models"
        );
    }
}
