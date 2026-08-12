use std::{env, fs, path::PathBuf};

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
        let data_dir = home.join(".ai-gateway").join("server").join("gateway");
        migrate_legacy_data(&home, &data_dir)?;
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

fn migrate_legacy_data(home: &std::path::Path, data_dir: &std::path::Path) -> Result<(), String> {
    if data_dir.exists() {
        return Ok(());
    }
    let legacy_dir = home.join(".ai-gateway-client").join("gateway");
    if !legacy_dir.is_dir() {
        return Ok(());
    }
    let parent = data_dir
        .parent()
        .ok_or_else(|| "Gateway 数据目录无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建 Gateway 数据目录失败：{error}"))?;
    fs::rename(&legacy_dir, data_dir).map_err(|error| {
        format!(
            "迁移旧 Gateway 数据目录失败（{} -> {}）：{error}",
            legacy_dir.display(),
            data_dir.display()
        )
    })
}
