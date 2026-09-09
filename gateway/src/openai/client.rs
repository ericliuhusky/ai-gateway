use super::OPENAI_CODEX_BASE_URL;
use super::url::{models_api_url, responses_api_url};
use axum::{body::Bytes, http::HeaderMap};
use reqwest::{Client, RequestBuilder, Response};

#[derive(Clone, Debug)]
pub struct OpenAiClient {
    http: Client,
}

impl OpenAiClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn api_responses_passthrough(
        &self,
        base_url: &str,
        api_key: &str,
        body: Bytes,
        headers: &HeaderMap,
    ) -> Result<Response, String> {
        let request = apply_request_accept(self.http.post(responses_api_url(base_url)), headers)
            .bearer_auth(api_key)
            .header("content-type", "application/json")
            .body(body);
        self.send(request).await
    }

    pub async fn account_responses_passthrough(
        &self,
        access_token: &str,
        body: Bytes,
        headers: &HeaderMap,
    ) -> Result<Response, String> {
        let request = apply_request_accept(
            self.account_request(
                self.http.post(format!("{OPENAI_CODEX_BASE_URL}/responses")),
                access_token,
            ),
            headers,
        )
        .header("content-type", "application/json")
        .body(body);
        self.send(request).await
    }

    pub async fn api_models(&self, base_url: &str, api_key: &str) -> Result<Response, String> {
        self.send(
            self.http
                .get(models_api_url(base_url))
                .bearer_auth(api_key)
                .header("accept", "application/json"),
        )
        .await
    }

    pub async fn account_models(
        &self,
        access_token: &str,
        client_version: Option<&str>,
    ) -> Result<Response, String> {
        let mut request = self
            .account_request(
                self.http.get(format!("{OPENAI_CODEX_BASE_URL}/models")),
                access_token,
            )
            .header("accept", "application/json");
        if let Some(client_version) = client_version {
            request = request.query(&[("client_version", client_version)]);
        }
        self.send(request).await
    }

    pub async fn account_usage(&self, access_token: &str) -> Result<Response, String> {
        let request = self.account_request(
            self.http.get(format!(
                "{}/wham/usage",
                OPENAI_CODEX_BASE_URL.trim_end_matches("/codex")
            )),
            access_token,
        );
        self.send(request).await
    }

    async fn send(&self, request: RequestBuilder) -> Result<Response, String> {
        request
            .send()
            .await
            .map_err(|err| format!("[AI网关] 向供应商发送http请求失败: {err}"))
    }

    fn account_request(&self, request: RequestBuilder, access_token: &str) -> RequestBuilder {
        request.bearer_auth(access_token)
    }
}

fn apply_request_accept(mut request: RequestBuilder, headers: &HeaderMap) -> RequestBuilder {
    if let Some(accept) = headers.get("accept") {
        request = request.header("accept", accept);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::{OPENAI_CODEX_BASE_URL, OpenAiClient};

    #[tokio::test]
    async fn account_models_use_codex_backend() {
        let client = OpenAiClient::new(reqwest::Client::new());
        let request = client
            .account_request(
                client.http.get(format!("{OPENAI_CODEX_BASE_URL}/models")),
                "token",
            )
            .query(&[("client_version", "test")])
            .build()
            .expect("valid request");

        assert_eq!(
            request.url().as_str(),
            "https://chatgpt.com/backend-api/codex/models?client_version=test"
        );
    }
}
