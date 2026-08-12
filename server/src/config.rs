use std::{env, path::PathBuf};

pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.146.0";

#[derive(Clone)]
pub struct Config {
    data_dir: PathBuf,
}

impl Config {
    pub fn local() -> Result<Self, String> {
        let home = env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "未设置 HOME 环境变量".to_string())?;
        let data_dir = home.join(".ai-gateway").join("data");
        Ok(Self { data_dir })
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    pub fn sqlite_path(&self) -> PathBuf {
        self.data_dir.join("db.sqlite")
    }

    #[cfg(test)]
    pub fn for_test(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}
