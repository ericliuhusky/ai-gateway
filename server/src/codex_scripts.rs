use axum::{
    http::{
        HeaderValue,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};

const SETUP_SCRIPT: &str = include_str!("../scripts/codex-setup.sh");
const RESTORE_SCRIPT: &str = include_str!("../scripts/codex-restore.sh");

pub async fn setup_script() -> Response {
    shell_script_response(SETUP_SCRIPT, "setup.sh")
}

pub async fn restore_script() -> Response {
    shell_script_response(RESTORE_SCRIPT, "restore.sh")
}

fn shell_script_response(script: &'static str, filename: &'static str) -> Response {
    let disposition = HeaderValue::from_str(&format!("inline; filename=\"{filename}\""))
        .expect("static script filename must be a valid header value");

    (
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("text/x-shellscript; charset=utf-8"),
            ),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (CONTENT_DISPOSITION, disposition),
        ],
        script,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_script_requires_a_gateway_url_and_writes_responses_provider() {
        assert!(SETUP_SCRIPT.contains("gateway_base_url=${1:-}"));
        assert!(SETUP_SCRIPT.contains("[model_providers.ai-gateway]"));
        assert!(SETUP_SCRIPT.contains("wire_api = \"responses\""));
        assert!(!SETUP_SCRIPT.contains("state_5.sqlite"));
    }

    #[test]
    fn restore_script_uses_the_original_config_backup() {
        assert!(RESTORE_SCRIPT.contains("codex-config.before-ai-gateway.toml"));
        assert!(RESTORE_SCRIPT.contains("codex-config.before-restore."));
        assert!(!RESTORE_SCRIPT.contains("state_5.sqlite"));
    }
}
