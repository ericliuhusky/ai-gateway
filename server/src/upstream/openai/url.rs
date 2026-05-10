fn base_api_url(base_url: &str, endpoint: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if has_api_prefix(trimmed) {
        format!("{trimmed}/{endpoint}")
    } else {
        format!("{trimmed}/v1/{endpoint}")
    }
}

fn has_api_prefix(base_url: &str) -> bool {
    base_url.ends_with("/v1") || base_url.contains("/api/") || base_url.ends_with("/api")
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

#[cfg(test)]
mod tests {
    use super::{chat_completions_api_url, models_api_url, responses_api_url};

    #[test]
    fn appends_chat_completions_to_plain_base_url() {
        assert_eq!(
            chat_completions_api_url("https://example.com"),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn appends_chat_completions_to_versioned_base_url() {
        assert_eq!(
            chat_completions_api_url("https://example.com/v1"),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn appends_models_to_v1_base_url() {
        assert_eq!(
            models_api_url("https://example.com/api/v3"),
            "https://example.com/api/v3/models"
        );
    }

    #[test]
    fn appends_models_to_plain_base_url() {
        assert_eq!(
            models_api_url("https://api.xcode.best"),
            "https://api.xcode.best/v1/models"
        );
    }

    #[test]
    fn appends_models_to_explicit_v1_base_url() {
        assert_eq!(
            models_api_url("https://api.xcode.best/v1"),
            "https://api.xcode.best/v1/models"
        );
        assert_eq!(
            responses_api_url("https://api.xcode.best/v1"),
            "https://api.xcode.best/v1/responses"
        );
    }

    #[test]
    fn appends_responses_to_plain_base_url() {
        assert_eq!(
            responses_api_url("https://example.com"),
            "https://example.com/v1/responses"
        );
    }

    #[test]
    fn appends_responses_to_versioned_base_url() {
        assert_eq!(
            responses_api_url("https://example.com/v1"),
            "https://example.com/v1/responses"
        );
    }

    #[test]
    fn trims_trailing_slash_before_appending_endpoint() {
        assert_eq!(
            models_api_url("https://example.com/v1/"),
            "https://example.com/v1/models"
        );
    }
}
