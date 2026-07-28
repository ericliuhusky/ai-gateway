use std::{env, net::SocketAddr, path::PathBuf};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:10100";
const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.130.0";
const RESPONSES_PATH: &str = "/openai/v1/responses";
const OPENAI_PRIVATE_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

#[derive(Clone, Debug)]
pub struct Config {
    bind_addr: SocketAddr,
    data_dir: PathBuf,
    web_dir: PathBuf,
    codex_client_version: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bind_addr = env::var("AI_GATEWAY_BIND_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string())
            .parse()
            .map_err(|err| format!("invalid AI_GATEWAY_BIND_ADDR: {err}"))?;

        let data_dir = match env::var("AI_GATEWAY_DATA_DIR") {
            Ok(path) if !path.trim().is_empty() => PathBuf::from(path),
            _ => {
                let home = env::var("HOME")
                    .map(PathBuf::from)
                    .map_err(|_| "set AI_GATEWAY_DATA_DIR or HOME".to_string())?;
                home.join(".ai-gateway")
            }
        };

        let codex_client_version = env::var("AI_GATEWAY_CODEX_CLIENT_VERSION")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CODEX_CLIENT_VERSION.to_string());

        let web_dir = match env::var("AI_GATEWAY_WEB_DIR") {
            Ok(path) if !path.trim().is_empty() => PathBuf::from(path),
            _ => data_dir.join("web"),
        };

        Ok(Self {
            bind_addr,
            data_dir,
            web_dir,
            codex_client_version,
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn responses_path() -> &'static str {
        RESPONSES_PATH
    }

    pub fn openai_private_responses_url() -> &'static str {
        OPENAI_PRIVATE_RESPONSES_URL
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    pub fn sqlite_path(&self) -> PathBuf {
        self.data_dir.join("db.sqlite")
    }

    pub fn log_sqlite_path(&self) -> PathBuf {
        self.data_dir.join("log.db")
    }

    pub fn web_dir(&self) -> PathBuf {
        self.web_dir.clone()
    }

    pub fn codex_client_version(&self) -> &str {
        &self.codex_client_version
    }
}
