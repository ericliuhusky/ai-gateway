use reqwest::{Client, Proxy, StatusCode, Url};
use std::{collections::HashMap, process::Command};
pub fn build_http_client() -> Client {
    let mut builder = Client::builder();

    if cfg!(target_os = "macos") {
        if let Some(config) = load_macos_system_proxy() {
            let http_proxy_for_proxy = config.http_proxy.clone();
            let https_proxy_for_proxy = config.https_proxy.clone();
            let proxy = Proxy::custom(move |url| {
                select_proxy(url, &http_proxy_for_proxy, &https_proxy_for_proxy)
            });
            builder = builder.proxy(proxy);
        }
    }

    builder.build().unwrap_or_else(|_| Client::new())
}

pub fn proxy_url_for(url: &Url) -> Option<String> {
    if cfg!(target_os = "macos") {
        load_macos_system_proxy()
            .and_then(|config| select_proxy(url, &config.http_proxy, &config.https_proxy))
    } else {
        None
    }
}

pub fn should_try_next_endpoint(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::NOT_FOUND
        || status.is_server_error()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProxyConfig {
    http_proxy: Option<String>,
    https_proxy: Option<String>,
}

fn select_proxy(
    url: &Url,
    http_proxy: &Option<String>,
    https_proxy: &Option<String>,
) -> Option<String> {
    if should_bypass_proxy(url) {
        return None;
    }

    match url.scheme() {
        "http" | "ws" => http_proxy.clone(),
        "https" | "wss" => https_proxy.clone().or_else(|| http_proxy.clone()),
        _ => None,
    }
}

fn should_bypass_proxy(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1")
    )
}

fn load_macos_system_proxy() -> Option<ProxyConfig> {
    let output = Command::new("scutil").arg("--proxy").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let config = parse_scutil_proxy_output(&stdout);
    if config.http_proxy.is_some() || config.https_proxy.is_some() {
        Some(config)
    } else {
        None
    }
}

fn parse_scutil_proxy_output(output: &str) -> ProxyConfig {
    let values = output
        .lines()
        .filter_map(parse_scutil_line)
        .collect::<HashMap<_, _>>();

    ProxyConfig {
        http_proxy: build_proxy_url(&values, "HTTP"),
        https_proxy: build_proxy_url(&values, "HTTPS"),
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

#[cfg(test)]
mod tests {
    use super::{parse_scutil_proxy_output, select_proxy};
    use reqwest::Url;

    #[test]
    fn parses_http_and_https_system_proxy() {
        let config = parse_scutil_proxy_output(
            r#"<dictionary> {
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7890
  HTTPSProxy : 127.0.0.1
}"#,
        );

        assert_eq!(config.http_proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(config.https_proxy.as_deref(), Some("http://127.0.0.1:7890"));
    }

    #[test]
    fn bypasses_localhost_and_loopback() {
        let http_proxy = Some("http://127.0.0.1:7890".to_string());
        let https_proxy = Some("http://127.0.0.1:7890".to_string());

        assert_eq!(
            select_proxy(
                &Url::parse("http://127.0.0.1:10100/openai/v1/responses").expect("valid url"),
                &http_proxy,
                &https_proxy
            ),
            None
        );
        assert_eq!(
            select_proxy(
                &Url::parse("https://chatgpt.com/backend-api/codex/responses").expect("valid url"),
                &http_proxy,
                &https_proxy
            ),
            Some("http://127.0.0.1:7890".to_string())
        );
        assert_eq!(
            select_proxy(
                &Url::parse("wss://chatgpt.com/backend-api/codex/responses").expect("valid url"),
                &http_proxy,
                &https_proxy
            ),
            Some("http://127.0.0.1:7890".to_string())
        );
    }
}
