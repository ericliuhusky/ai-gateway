use serde::Deserialize;
use std::{env, fs, net::SocketAddr, path::PathBuf};

// 服务监听配置。
const BIND_ADDR: &str = "127.0.0.1:10100";
const OPENAI_CALLBACK_ADDR: &str = "127.0.0.1:1455";
const OPENAI_CALLBACK_URL: &str = "http://localhost:1455/auth/callback";

// OpenAI / Codex 上游配置。
const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.130.0";
const RESPONSES_PATH: &str = "/openai/v1/responses";
const OPENAI_PRIVATE_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

// 内置 Codex 资源。
const BUNDLED_CODEX_CONFIG: &str = include_str!("../../assets/codex-config.toml");

// Google OAuth 凭据。
const GOOGLE_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GOOGLE_CLIENT_SECRET: &str = "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";

#[derive(Debug, Deserialize)]
struct CodexVersion {
    latest_version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    home_dir: PathBuf,
}

impl Config {
    // 环境初始化。
    pub fn from_env() -> Result<Self, String> {
        let home_dir = env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "没有$HOME环境变量".to_string())?;
        Ok(Self { home_dir })
    }

    // 服务监听配置。
    pub fn bind_addr(&self) -> SocketAddr {
        BIND_ADDR
            .parse()
            .expect("hardcoded bind address must be valid")
    }

    pub fn openai_callback_addr(&self) -> SocketAddr {
        OPENAI_CALLBACK_ADDR
            .parse()
            .expect("hardcoded openai callback address must be valid")
    }

    pub fn openai_callback_url(&self) -> &'static str {
        OPENAI_CALLBACK_URL
    }

    // OpenAI / Codex 上游配置。
    pub fn responses_path() -> &'static str {
        RESPONSES_PATH
    }

    pub fn openai_private_responses_url() -> &'static str {
        OPENAI_PRIVATE_RESPONSES_URL
    }

    // 网关数据文件。
    pub fn data_dir(&self) -> PathBuf {
        self.home_dir.join(".ai-gateway")
    }

    pub fn sqlite_path(&self) -> PathBuf {
        self.data_dir().join("db.sqlite")
    }

    pub fn log_sqlite_path(&self) -> PathBuf {
        self.data_dir().join("log.db")
    }

    // Codex 文件和默认值。
    pub fn bundled_codex_config(&self) -> &'static str {
        BUNDLED_CODEX_CONFIG
    }

    pub fn codex_dir(&self) -> PathBuf {
        self.home_dir.join(".codex")
    }

    pub fn codex_config_path(&self) -> PathBuf {
        self.codex_dir().join("config.toml")
    }

    pub fn codex_config_backup_path(&self) -> PathBuf {
        self.data_dir().join("codex-config.backup.toml")
    }

    pub fn codex_auth_path(&self) -> PathBuf {
        self.codex_dir().join("auth.json")
    }

    pub fn codex_auth_backup_path(&self) -> PathBuf {
        self.data_dir().join("codex-auth.backup.json")
    }

    fn codex_version_path(&self) -> PathBuf {
        self.codex_dir().join("version.json")
    }

    pub fn codex_client_version(&self) -> String {
        let parsed = fs::read_to_string(self.codex_version_path())
            .ok()
            .and_then(|content| serde_json::from_str::<CodexVersion>(&content).ok())
            .and_then(|version| version.latest_version)
            .map(|version| version.trim().to_string())
            .filter(|version| !version.is_empty());

        parsed.unwrap_or_else(|| DEFAULT_CODEX_CLIENT_VERSION.to_string())
    }

    // Google OAuth 凭据。
    pub fn google_client_id(&self) -> &'static str {
        GOOGLE_CLIENT_ID
    }

    pub fn google_client_secret(&self) -> &'static str {
        GOOGLE_CLIENT_SECRET
    }
}
