use crate::{config::Config, store::sqlite::SqliteStore};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct SettingsStore {
    sqlite: SqliteStore,
}

#[derive(Clone, Debug)]
pub struct SecuritySettings {
    pub feishu_app_id: String,
    pub feishu_app_secret_configured: bool,
}

impl SettingsStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let store = Self {
            sqlite: SqliteStore::new(config.clone())?,
        };
        if config.has_security_overrides() {
            let current = store.security_settings()?;
            let feishu_app_id = config.feishu_app_id().unwrap_or(current.feishu_app_id);
            store.sqlite.update_database_security_settings(
                config.database_encryption_key().as_deref(),
                &feishu_app_id,
                config.feishu_app_secret().as_deref(),
            )?;
        }
        Ok(store)
    }

    pub fn security_settings(&self) -> Result<SecuritySettings, String> {
        let value = self.sqlite.database_security_settings()?;
        Ok(SecuritySettings {
            feishu_app_id: value.feishu_app_id,
            feishu_app_secret_configured: !value.feishu_app_secret.is_empty(),
        })
    }

    pub fn feishu_credentials(&self) -> Result<(String, String), String> {
        let value = self.sqlite.database_security_settings()?;
        if value.feishu_app_secret.is_empty() {
            return Ok((value.feishu_app_id, String::new()));
        }
        Ok((
            value.feishu_app_id,
            self.sqlite
                .encryption()?
                .decrypt(&value.feishu_app_secret)?,
        ))
    }
}
