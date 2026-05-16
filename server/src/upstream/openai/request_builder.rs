use super::url::{chat_completions_api_url, models_api_url, responses_api_url, usage_api_url};
use reqwest::{Client, RequestBuilder};
use serde_json::Value;

pub trait OpenAiRequestBuilder {
    fn base_url(&self) -> &str;

    fn auth_token(&self) -> &str;

    fn customize_request(&self, request: RequestBuilder) -> RequestBuilder;

    fn client_version(&self) -> Option<&str> {
        None
    }

    fn build(&self, http: &Client, endpoint: OpenAiEndpoint) -> RequestBuilder {
        let request = match endpoint {
            OpenAiEndpoint::Responses { body, stream } => {
                let request = http
                    .post(responses_api_url(self.base_url()))
                    .bearer_auth(self.auth_token())
                    .header("content-type", "application/json")
                    .header(
                        "accept",
                        if stream {
                            "text/event-stream"
                        } else {
                            "application/json"
                        },
                    );

                match body {
                    OpenAiRequestBody::Json(body) => request.json(&body),
                    OpenAiRequestBody::Raw(body) => request.body(body),
                }
            }
            OpenAiEndpoint::ChatCompletions { body } => http
                .post(chat_completions_api_url(self.base_url()))
                .bearer_auth(self.auth_token())
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .json(&body),
            OpenAiEndpoint::Models => {
                let mut request = http
                    .get(models_api_url(self.base_url()))
                    .bearer_auth(self.auth_token())
                    .header("accept", "application/json");

                if let Some(client_version) = self.client_version() {
                    request = request.query(&[("client_version", client_version)]);
                }

                request
            }
            OpenAiEndpoint::Usage => http
                .get(usage_api_url(self.base_url()))
                .bearer_auth(self.auth_token())
                .header("accept", "application/json"),
        };

        self.customize_request(request)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PublicOpenAiRequestBuilder<'a> {
    pub base_url: &'a str,
    pub api_key: &'a str,
}

impl OpenAiRequestBuilder for PublicOpenAiRequestBuilder<'_> {
    fn base_url(&self) -> &str {
        self.base_url
    }

    fn auth_token(&self) -> &str {
        self.api_key
    }

    fn customize_request(&self, request: RequestBuilder) -> RequestBuilder {
        request
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PrivateOpenAiRequestBuilder<'a> {
    pub base_url: &'a str,
    pub access_token: &'a str,
    pub account_id: Option<&'a str>,
    pub client_version: Option<&'a str>,
}

impl OpenAiRequestBuilder for PrivateOpenAiRequestBuilder<'_> {
    fn base_url(&self) -> &str {
        self.base_url
    }

    fn auth_token(&self) -> &str {
        self.access_token
    }

    fn customize_request(&self, request: RequestBuilder) -> RequestBuilder {
        let request = request.header("user-agent", "CodexBar");
        if let Some(account_id) = self.account_id.filter(|value| !value.is_empty()) {
            request.header("ChatGPT-Account-Id", account_id)
        } else {
            request
        }
    }

    fn client_version(&self) -> Option<&str> {
        self.client_version
    }
}

#[derive(Clone, Debug)]
pub enum OpenAiRequestBody {
    Json(Value),
    Raw(String),
}

#[derive(Clone, Debug)]
pub enum OpenAiEndpoint {
    Responses {
        body: OpenAiRequestBody,
        stream: bool,
    },
    ChatCompletions {
        body: Value,
    },
    Models,
    Usage,
}
