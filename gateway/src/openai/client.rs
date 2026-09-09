use super::OPENAI_CODEX_BASE_URL;
use super::url::{models_api_url, responses_api_url};
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
        body: String,
        stream: bool,
    ) -> Result<Response, String> {
        self.send_passthrough(
            self.http
                .post(responses_api_url(base_url))
                .bearer_auth(api_key)
                .header("content-type", "application/json")
                .header(
                    "accept",
                    if stream {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                )
                .body(body),
        )
        .await
    }

    pub async fn account_responses_passthrough(
        &self,
        access_token: &str,
        upstream_account_id: Option<&str>,
        body: String,
        stream: bool,
    ) -> Result<Response, String> {
        let request = self
            .account_request(
                self.http.post(format!("{OPENAI_CODEX_BASE_URL}/responses")),
                access_token,
                upstream_account_id,
            )
            .header("content-type", "application/json")
            .header(
                "accept",
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .body(body);
        self.send_passthrough(request).await
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
        upstream_account_id: Option<&str>,
        client_version: Option<&str>,
    ) -> Result<Response, String> {
        let mut request = self
            .account_request(
                self.http.get(format!("{OPENAI_CODEX_BASE_URL}/models")),
                access_token,
                upstream_account_id,
            )
            .header("accept", "application/json");
        if let Some(client_version) = client_version {
            request = request.query(&[("client_version", client_version)]);
        }
        self.send(request).await
    }

    pub async fn account_usage(
        &self,
        access_token: &str,
        upstream_account_id: Option<&str>,
    ) -> Result<Response, String> {
        let request = self.account_request(
            self.http.get(format!(
                "{}/wham/usage",
                OPENAI_CODEX_BASE_URL.trim_end_matches("/codex")
            )),
            access_token,
            upstream_account_id,
        );
        self.send(request).await
    }

    async fn send_passthrough(&self, request: RequestBuilder) -> Result<Response, String> {
        request
            .send()
            .await
            .map_err(|err| format!("OpenAI 请求失败: {err}"))
    }

    async fn send(&self, request: RequestBuilder) -> Result<Response, String> {
        let response = self.send_passthrough(request).await?;

        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!("OpenAI 上游返回状态码 {status}: {response_body}"))
        }
    }

    fn account_request(
        &self,
        request: RequestBuilder,
        access_token: &str,
        upstream_account_id: Option<&str>,
    ) -> RequestBuilder {
        let request = request
            .bearer_auth(access_token)
            .header("user-agent", "CodexBar");
        if let Some(upstream_account_id) = upstream_account_id.filter(|value| !value.is_empty()) {
            request.header("ChatGPT-Account-Id", upstream_account_id)
        } else {
            request
        }
    }
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
                None,
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
