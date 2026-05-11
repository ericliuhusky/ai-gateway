use crate::upstream::{
    google_v1internal::GoogleV1InternalClient,
    openai::{
        OPENAI_CODEX_BASE_URL, OpenAiEndpoint, OpenAiRequestBuilder, PrivateOpenAiRequestBuilder,
        PublicOpenAiRequestBuilder, stream_private_responses_websocket_blocking,
    },
    shared::build_http_client,
};
use reqwest::{Client, Response};
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct UpstreamClient {
    http: Client,
    google_v1internal: GoogleV1InternalClient,
}

impl UpstreamClient {
    pub fn new() -> Self {
        let http = build_http_client();
        Self {
            http: http.clone(),
            google_v1internal: GoogleV1InternalClient::new(http.clone()),
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
        _id: &str,
        access_token: &str,
        account_id: Option<&str>,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        let response = PrivateOpenAiRequestBuilder {
            base_url: OPENAI_CODEX_BASE_URL,
            access_token,
            account_id,
            client_version: None,
        }
        .build(&self.http, OpenAiEndpoint::Responses { body, stream })
        .send()
        .await
        .map_err(|err| format!("OpenAI responses 私有接口请求失败: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "OpenAI responses 私有接口上游返回状态码 {status}: {response_body}"
            ))
        }
    }

    pub async fn fetch_openai_models(
        &self,
        _id: &str,
        access_token: &str,
        account_id: Option<&str>,
        client_version: &str,
    ) -> Result<Value, String> {
        let response = PrivateOpenAiRequestBuilder {
            base_url: OPENAI_CODEX_BASE_URL,
            access_token,
            account_id,
            client_version: Some(client_version),
        }
        .build(&self.http, OpenAiEndpoint::Models)
        .send()
        .await
        .map_err(|err| format!("OpenAI 模型请求失败: {err}"))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|err| format!("OpenAI 模型解析失败: {err}"))
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "OpenAI 模型私有接口上游返回状态码 {status}: {response_body}"
            ))
        }
    }

    pub async fn fetch_openai_usage(
        &self,
        _id: &str,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<Value, String> {
        let response = PrivateOpenAiRequestBuilder {
            base_url: OPENAI_CODEX_BASE_URL,
            access_token,
            account_id,
            client_version: None,
        }
        .build(&self.http, OpenAiEndpoint::Usage)
        .send()
        .await
        .map_err(|err| format!("openai usage request failed: {err}"))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|err| format!("openai usage parse failed: {err}"))
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "openai usage upstream returned {status}: {response_body}"
            ))
        }
    }

    pub fn stream_openai_responses_websocket_blocking(
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

    pub async fn call_openai_responses_upstream(
        &self,
        _id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
        stream: bool,
    ) -> Result<Response, String> {
        let response = PublicOpenAiRequestBuilder { base_url, api_key }
            .build(&self.http, OpenAiEndpoint::Responses { body, stream })
            .send()
            .await
            .map_err(|err| format!("OpenAI responses 请求失败: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "OpenAI responses 上游返回状态码 {status}: {response_body}"
            ))
        }
    }

    pub async fn call_openai_chat_upstream(
        &self,
        _id: &str,
        base_url: &str,
        api_key: &str,
        body: Value,
    ) -> Result<Response, String> {
        let response = PublicOpenAiRequestBuilder { base_url, api_key }
            .build(&self.http, OpenAiEndpoint::ChatCompletions { body })
            .send()
            .await
            .map_err(|err| format!("OpenAI chat 请求失败: {err}"))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "OpenAI chat 上游返回状态码 {status}: {response_body}"
            ))
        }
    }

    pub async fn fetch_openai_models_upstream(
        &self,
        _id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<Value, String> {
        let response = PublicOpenAiRequestBuilder { base_url, api_key }
            .build(&self.http, OpenAiEndpoint::Models)
            .send()
            .await
            .map_err(|err| format!("OpenAI 模型请求失败: {err}"))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|err| format!("OpenAI 模型解析失败: {err}"))
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "OpenAI 模型上游返回状态码 {status}: {response_body}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::upstream::openai::{
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
