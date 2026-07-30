use std::{env, net::SocketAddr, path::PathBuf};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:4242";
pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.146.0";

#[derive(Clone)]
pub struct Config {
    bind_addr: SocketAddr,
    data_dir: PathBuf,
    web_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bind_addr = DEFAULT_BIND_ADDR
            .parse()
            .map_err(|err| format!("invalid default bind address: {err}"))?;

        let home = env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "set HOME".to_string())?;
        let data_dir = home.join(".ai-gateway");

        let web_dir = home.join(".ai-gateway/web");

        Ok(Self {
            bind_addr,
            data_dir,
            web_dir,
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

    #[cfg(test)]
    pub fn for_test(data_dir: PathBuf) -> Self {
        Self {
            bind_addr: "127.0.0.1:0".parse().expect("test bind address is valid"),
            web_dir: data_dir.join("web"),
            data_dir,
        }
    }
}
