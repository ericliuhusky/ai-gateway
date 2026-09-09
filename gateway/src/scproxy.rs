// 获取系统代理

use reqwest::{ClientBuilder, Proxy, Url};
use std::{collections::HashMap, process::Command};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SysConfigProxy {
    http: Option<String>,
    https: Option<String>,
}

impl SysConfigProxy {
    pub(crate) fn apply(builder: ClientBuilder) -> ClientBuilder {
        if cfg!(target_os = "macos")
            && let Some(config) = Self::load_macos_system_proxy()
        {
            let proxy = Proxy::custom(move |url| config.select_proxy(url));
            return builder.proxy(proxy);
        }

        builder
    }

    fn select_proxy(&self, url: &Url) -> Option<String> {
        if Self::should_bypass_proxy(url) {
            return None;
        }

        match url.scheme() {
            "http" | "ws" => self.http.clone(),
            "https" | "wss" => self.https.clone().or_else(|| self.http.clone()),
            _ => None,
        }
    }

    fn should_bypass_proxy(url: &Url) -> bool {
        matches!(
            url.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("::1")
        )
    }

    fn load_macos_system_proxy() -> Option<Self> {
        let output = Command::new("scutil").arg("--proxy").output().ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let config = Self::parse_scutil_proxy_output(&stdout);
        if config.http.is_some() || config.https.is_some() {
            Some(config)
        } else {
            None
        }
    }

    fn parse_scutil_proxy_output(output: &str) -> Self {
        let values = output
            .lines()
            .filter_map(Self::parse_scutil_line)
            .collect::<HashMap<_, _>>();

        Self {
            http: Self::build_proxy_url(&values, "HTTP"),
            https: Self::build_proxy_url(&values, "HTTPS"),
        }
    }

    fn parse_scutil_line(line: &str) -> Option<(String, String)> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('<') || trimmed == "}" {
            return None;
        }

        let (key, value) = trimmed.split_once(':')?;
        Some((key.trim().to_string(), value.trim().to_string()))
    }

    fn build_proxy_url(values: &HashMap<String, String>, prefix: &str) -> Option<String> {
        let enabled = values.get(&format!("{prefix}Enable"))?;
        if enabled != "1" {
            return None;
        }

        let host = values.get(&format!("{prefix}Proxy"))?;
        let port = values.get(&format!("{prefix}Port"))?;
        Some(format!("http://{host}:{port}"))
    }
}

#[cfg(test)]
mod tests {
    use super::SysConfigProxy;
    use reqwest::Url;

    #[test]
    fn parses_http_and_https_system_proxy() {
        let config = SysConfigProxy::parse_scutil_proxy_output(
            r#"<dictionary> {
  HTTPEnable : 1
  HTTPPort : 7897
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7897
  HTTPSProxy : 127.0.0.1
}"#,
        );

        assert_eq!(config.http.as_deref(), Some("http://127.0.0.1:7897"));
        assert_eq!(config.https.as_deref(), Some("http://127.0.0.1:7897"));
    }

    #[test]
    fn bypasses_localhost_and_loopback() {
        let config = SysConfigProxy {
            http: Some("http://127.0.0.1:7897".to_string()),
            https: Some("http://127.0.0.1:7897".to_string()),
        };

        assert_eq!(
            config.select_proxy(
                &Url::parse("http://127.0.0.1:42401/v1/responses").expect("valid url")
            ),
            None
        );
        assert_eq!(
            config.select_proxy(
                &Url::parse("https://chatgpt.com/backend-api/codex/responses").expect("valid url")
            ),
            Some("http://127.0.0.1:7897".to_string())
        );
        assert_eq!(
            config.select_proxy(
                &Url::parse("wss://chatgpt.com/backend-api/codex/responses").expect("valid url")
            ),
            Some("http://127.0.0.1:7897".to_string())
        );
    }
}
