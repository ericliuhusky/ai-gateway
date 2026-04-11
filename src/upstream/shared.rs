use reqwest::{Client, StatusCode};

pub fn build_http_client() -> Client {
    Client::builder().build().unwrap_or_else(|_| Client::new())
}

pub fn should_try_next_endpoint(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::NOT_FOUND
        || status.is_server_error()
}

pub fn truncate_for_log(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{truncated}...<truncated>")
    } else {
        truncated
    }
}

pub fn has_api_prefix(base_url: &str) -> bool {
    base_url.ends_with("/v1") || base_url.contains("/api/") || base_url.ends_with("/api")
}
