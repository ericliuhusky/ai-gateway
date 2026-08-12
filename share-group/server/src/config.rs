use std::{env, net::SocketAddr, path::PathBuf};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:4242";

#[derive(Clone)]
pub struct Config {
    bind_addr: SocketAddr,
    data_dir: PathBuf,
    database_encryption_key: Option<String>,
    feishu_app_id: Option<String>,
    feishu_app_secret: Option<String>,
    bootstrap_admin_email: Option<String>,
    bootstrap_admin_password: Option<String>,
    bootstrap_admin_name: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let bind_addr = DEFAULT_BIND_ADDR
            .parse()
            .map_err(|err| format!("默认监听地址无效：{err}"))?;

        let home = env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "未设置 HOME 环境变量".to_string())?;
        let data_dir = home.join(".ai-gateway");

        Ok(Self {
            bind_addr,
            data_dir,
            database_encryption_key: optional_env("AI_GATEWAY_DATABASE_ENCRYPTION_KEY"),
            feishu_app_id: optional_env("AI_GATEWAY_FEISHU_APP_ID"),
            feishu_app_secret: optional_env("AI_GATEWAY_FEISHU_APP_SECRET"),
            bootstrap_admin_email: optional_env("AI_GATEWAY_BOOTSTRAP_ADMIN_EMAIL"),
            bootstrap_admin_password: optional_env("AI_GATEWAY_BOOTSTRAP_ADMIN_PASSWORD"),
            bootstrap_admin_name: optional_env("AI_GATEWAY_BOOTSTRAP_ADMIN_NAME"),
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

    pub fn database_encryption_key(&self) -> Option<String> {
        self.database_encryption_key.clone()
    }

    pub fn feishu_app_id(&self) -> Option<String> {
        self.feishu_app_id.clone()
    }

    pub fn feishu_app_secret(&self) -> Option<String> {
        self.feishu_app_secret.clone()
    }

    pub fn has_security_overrides(&self) -> bool {
        self.database_encryption_key.is_some()
            || self.feishu_app_id.is_some()
            || self.feishu_app_secret.is_some()
    }

    pub fn bootstrap_admin(&self) -> Result<Option<(String, String, String)>, String> {
        match (
            self.bootstrap_admin_email.as_ref(),
            self.bootstrap_admin_password.as_ref(),
        ) {
            (None, None) => Ok(None),
            (Some(email), Some(password)) => Ok(Some((
                email.clone(),
                self.bootstrap_admin_name
                    .clone()
                    .unwrap_or_else(|| "中心服务管理员".to_string()),
                password.clone(),
            ))),
            _ => Err(
                "AI_GATEWAY_BOOTSTRAP_ADMIN_EMAIL 和 AI_GATEWAY_BOOTSTRAP_ADMIN_PASSWORD 必须同时设置"
                    .to_string(),
            ),
        }
    }

    #[cfg(test)]
    pub fn for_test(data_dir: PathBuf) -> Self {
        Self {
            bind_addr: "127.0.0.1:0".parse().expect("test bind address is valid"),
            data_dir,
            database_encryption_key: None,
            feishu_app_id: None,
            feishu_app_secret: None,
            bootstrap_admin_email: None,
            bootstrap_admin_password: None,
            bootstrap_admin_name: None,
        }
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
