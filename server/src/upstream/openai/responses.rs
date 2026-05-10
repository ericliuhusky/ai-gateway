use super::url::{chat_completions_api_url, models_api_url, responses_api_url};
use reqwest::{Client, RequestBuilder};
use serde_json::Value;

pub fn responses_request(
    http: &Client,
    base_url: &str,
    api_key: &str,
    body: Value,
    stream: bool,
    is_private: bool,
    account_id: Option<&str>,
    is_chat: bool,
) -> RequestBuilder {
    let url = if is_chat {
        chat_completions_api_url(base_url)
    } else {
        responses_api_url(base_url)
    };
    let request = http
        .post(&url)
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
        .json(&body);

    with_private_headers(request, is_private, account_id)
}

pub fn models_request(
    http: &Client,
    base_url: &str,
    api_key: &str,
    is_private: bool,
    account_id: Option<&str>,
    client_version: Option<&str>,
) -> RequestBuilder {
    let url = models_api_url(base_url);
    let mut request = http
        .get(&url)
        .bearer_auth(api_key)
        .header("accept", "application/json");

    if is_private && let Some(client_version) = client_version {
        request = request.query(&[("client_version", client_version)]);
    }

    with_private_headers(request, is_private, account_id)
}

pub fn usage_request(
    http: &Client,
    url: &str,
    api_key: &str,
    is_private: bool,
    account_id: Option<&str>,
) -> RequestBuilder {
    let request = http
        .get(url)
        .bearer_auth(api_key)
        .header("accept", "application/json");

    with_private_headers(request, is_private, account_id)
}

fn with_private_headers(
    request: RequestBuilder,
    is_private: bool,
    account_id: Option<&str>,
) -> RequestBuilder {
    if is_private {
        let request = request.header("user-agent", "CodexBar");
        if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
            request.header("ChatGPT-Account-Id", account_id)
        } else {
            request
        }
    } else {
        request
    }
}
