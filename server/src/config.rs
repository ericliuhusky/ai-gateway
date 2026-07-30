use crate::crypto::FieldEncryptor;
use std::{env, net::SocketAddr, path::PathBuf};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:4242";
pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.146.0";
const ENCRYPTION_KEY_ENV: &str = "AI_GATEWAY_ENCRYPTION_KEY";

#[derive(Clone)]
pub struct Config {
    bind_addr: SocketAddr,
    data_dir: PathBuf,
    web_dir: PathBuf,
    encryption: FieldEncryptor,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bind_addr = env::var("AI_GATEWAY_BIND_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string())
            .parse()
            .map_err(|err| format!("invalid AI_GATEWAY_BIND_ADDR: {err}"))?;

        let home = env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "set HOME".to_string())?;
        let data_dir = match env::var("AI_GATEWAY_DATA_DIR") {
            Ok(path) if !path.trim().is_empty() => PathBuf::from(path),
            _ => home.join(".ai-gateway"),
        };

        let web_dir = home.join(".ai-gateway/web");
        let encryption_key = env::var(ENCRYPTION_KEY_ENV).map_err(|_| {
            format!("{ENCRYPTION_KEY_ENV} is required; generate one with `openssl rand -base64 32`")
        })?;
        let encryption = FieldEncryptor::from_base64_key(&encryption_key)?;

        Ok(Self {
            bind_addr,
            data_dir,
            web_dir,
            encryption,
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    pub fn sqlite_path(&self) -> PathBuf {
        self.data_dir.join("db.sqlite")
    }

    pub fn web_dir(&self) -> PathBuf {
        self.web_dir.clone()
    }

    pub fn encryption(&self) -> FieldEncryptor {
        self.encryption.clone()
    }

    #[cfg(test)]
    pub fn for_test(data_dir: PathBuf) -> Self {
        Self {
            bind_addr: "127.0.0.1:0".parse().expect("test bind address is valid"),
            web_dir: data_dir.join("web"),
            data_dir,
            encryption: FieldEncryptor::from_base64_key(
                "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
            )
            .expect("hard-coded test key is valid"),
        }
    }
}
