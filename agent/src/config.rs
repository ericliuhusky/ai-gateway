use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:10101";

#[derive(Clone, Debug)]
pub struct Config {
    bind_addr: SocketAddr,
    home_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bind_addr = env::var("AI_GATEWAY_AGENT_BIND_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string())
            .parse()
            .map_err(|err| format!("invalid AI_GATEWAY_AGENT_BIND_ADDR: {err}"))?;
        let home_dir = env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "missing HOME environment variable".to_string())?;

        Ok(Self {
            bind_addr,
            home_dir,
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn data_dir(&self) -> PathBuf {
        self.home_dir.join(".ai-gateway")
    }

    pub fn codex_dir(&self) -> PathBuf {
        self.home_dir.join(".codex")
    }

    pub fn codex_config_path(&self) -> PathBuf {
        self.codex_dir().join("config.toml")
    }

    pub fn codex_config_patch_path(&self) -> PathBuf {
        self.data_dir().join("codex-config.patch.json")
    }

    pub fn codex_session_alias_patch_path(&self) -> PathBuf {
        self.data_dir().join("codex-session-aliases.json")
    }

    pub fn codex_state_path(&self) -> PathBuf {
        self.codex_dir().join("state_5.sqlite")
    }

    pub fn codex_version_path(&self) -> PathBuf {
        self.codex_dir().join("version.json")
    }

    pub fn codex_auth_path(&self) -> PathBuf {
        self.codex_dir().join("auth.json")
    }

    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }
}
