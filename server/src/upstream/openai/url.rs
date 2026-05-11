pub const OPENAI_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

fn base_api_url(base_url: &str, endpoint: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if has_api_prefix(trimmed) {
        format!("{trimmed}/{endpoint}")
    } else {
        format!("{trimmed}/v1/{endpoint}")
    }
}

fn has_api_prefix(base_url: &str) -> bool {
    base_url.ends_with("/v1") || base_url.contains("/api") || base_url.contains("/backend-api")
}

pub fn chat_completions_api_url(base_url: &str) -> String {
    base_api_url(base_url, "chat/completions")
}

pub fn responses_api_url(base_url: &str) -> String {
    base_api_url(base_url, "responses")
}

pub fn models_api_url(base_url: &str) -> String {
    base_api_url(base_url, "models")
}

pub fn usage_api_url(base_url: &str) -> String {
    base_api_url(
        base_url
            .trim_end_matches('/')
            .strip_suffix("/codex")
            .unwrap_or(base_url),
        "wham/usage",
    )
}

#[cfg(test)]
mod tests {
    use super::{base_api_url, usage_api_url};

    #[test]
    fn base_api_url_test() {
        assert_eq!(
            base_api_url("https://example.com", "endpoint"),
            "https://example.com/v1/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/", "endpoint"),
            "https://example.com/v1/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/v1", "endpoint"),
            "https://example.com/v1/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/v1/", "endpoint"),
            "https://example.com/v1/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/api", "endpoint"),
            "https://example.com/api/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/api/", "endpoint"),
            "https://example.com/api/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/api/v3/coding", "endpoint"),
            "https://example.com/api/v3/coding/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/api/v3/coding/", "endpoint"),
            "https://example.com/api/v3/coding/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/backend-api", "endpoint"),
            "https://example.com/backend-api/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/backend-api/", "endpoint"),
            "https://example.com/backend-api/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/backend-api/codex", "endpoint"),
            "https://example.com/backend-api/codex/endpoint"
        );

        assert_eq!(
            base_api_url("https://example.com/backend-api/codex/", "endpoint"),
            "https://example.com/backend-api/codex/endpoint"
        );

        assert_eq!(
            usage_api_url("https://example.com/backend-api/codex"),
            "https://example.com/backend-api/wham/usage"
        );

        assert_eq!(
            usage_api_url("https://example.com/backend-api/codex/"),
            "https://example.com/backend-api/wham/usage"
        );
    }
}
