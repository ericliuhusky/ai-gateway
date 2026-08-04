use crate::{config::Config, models::AutoRoutingSettings};
use std::sync::Arc;

use super::sqlite::SqliteStore;

#[derive(Clone, Debug)]
pub struct SettingsStore {
    sqlite: SqliteStore,
}

#[derive(Clone, Debug)]
pub struct SecuritySettings {
    pub encryption_key_configured: bool,
    pub feishu_app_id: String,
    pub feishu_app_secret_configured: bool,
    pub auth_required: bool,
}

impl SettingsStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    pub fn codex_client_version_override(&self) -> Result<Option<String>, String> {
        self.sqlite.load_codex_client_version_override()
    }

    pub fn set_codex_client_version(&self, version: &str) -> Result<(), String> {
        self.sqlite.set_codex_client_version_override(Some(version))
    }

    pub fn clear_codex_client_version(&self) -> Result<(), String> {
        self.sqlite.set_codex_client_version_override(None)
    }

    pub fn security_settings(&self) -> Result<SecuritySettings, String> {
        let value = self.sqlite.database_security_settings()?;
        Ok(SecuritySettings {
            encryption_key_configured: !value.encryption_key.is_empty(),
            feishu_app_id: value.feishu_app_id,
            feishu_app_secret_configured: !value.feishu_app_secret.is_empty(),
            auth_required: value.auth_required,
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

    pub fn update_security_settings(
        &self,
        encryption_key: Option<&str>,
        feishu_app_id: &str,
        feishu_app_secret: Option<&str>,
        auth_required: bool,
    ) -> Result<SecuritySettings, String> {
        self.sqlite.update_database_security_settings(
            encryption_key,
            feishu_app_id,
            feishu_app_secret,
            auth_required,
        )?;
        self.security_settings()
    }

    pub fn regenerate_database_encryption_key(&self) -> Result<SecuritySettings, String> {
        let key = crate::crypto::FieldEncryptor::generate_base64_key()?;
        self.sqlite.set_database_encryption_key(&key)?;
        self.security_settings()
    }

    pub fn instance_auto_routing_settings(
        &self,
        instance_id: &str,
    ) -> Result<AutoRoutingSettings, String> {
        self.sqlite.load_instance_auto_routing_settings(instance_id)
    }

    pub fn set_instance_auto_routing_settings(
        &self,
        instance_id: &str,
        settings: &AutoRoutingSettings,
    ) -> Result<(), String> {
        self.sqlite
            .set_instance_auto_routing_settings(instance_id, settings)
    }

    pub fn clear_instance_auto_routing_provider(&self, provider_id: &str) -> Result<(), String> {
        self.sqlite
            .clear_instance_auto_routing_provider(provider_id)
    }

    pub fn auto_routing_settings(&self) -> Result<AutoRoutingSettings, String> {
        self.sqlite.load_auto_routing_settings()
    }

    pub fn set_auto_routing_settings(&self, settings: &AutoRoutingSettings) -> Result<(), String> {
        self.sqlite.set_auto_routing_settings(settings)
    }

    pub fn clear_auto_routing_provider(&self, provider_id: &str) -> Result<(), String> {
        let mut settings = self.auto_routing_settings()?;
        let mut changed = false;
        for target in [
            &mut settings.light,
            &mut settings.standard,
            &mut settings.pro,
            &mut settings.max,
        ] {
            if target
                .as_ref()
                .is_some_and(|target| target.provider_id == provider_id)
            {
                *target = None;
                changed = true;
            }
        }
        if changed {
            settings.enabled = false;
            self.set_auto_routing_settings(&settings)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SettingsStore;
    use crate::{
        models::{AutoRoutingSettings, RoutingModelTarget},
        store::sqlite::SqliteStore,
    };
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn stores_and_clears_codex_client_version_override() {
        let sqlite = SqliteStore::for_test(unique_test_db_path("settings"))
            .expect("create settings database");
        let store = SettingsStore { sqlite };

        assert_eq!(store.codex_client_version_override().unwrap(), None);
        store.set_codex_client_version("0.147.0").unwrap();
        assert_eq!(
            store.codex_client_version_override().unwrap().as_deref(),
            Some("0.147.0")
        );
        store.clear_codex_client_version().unwrap();
        assert_eq!(store.codex_client_version_override().unwrap(), None);
    }

    #[test]
    fn stores_automatic_routing_settings() {
        let sqlite = SqliteStore::for_test(unique_test_db_path("automatic-routing"))
            .expect("create settings database");
        let store = SettingsStore { sqlite };
        let settings = AutoRoutingSettings {
            enabled: true,
            light: Some(target("provider_a", "small")),
            standard: Some(target("provider_b", "medium")),
            pro: Some(target("provider_b", "large")),
            max: Some(target("provider_c", "xlarge")),
            low_confidence_threshold: 0.7,
        };

        store.set_auto_routing_settings(&settings).unwrap();
        assert_eq!(store.auto_routing_settings().unwrap(), settings);
    }

    #[test]
    fn regenerates_encryption_key_and_preserves_security_settings() {
        let sqlite = SqliteStore::for_test(unique_test_db_path("regenerate-encryption-key"))
            .expect("create settings database");
        sqlite
            .update_database_security_settings(None, "cli_test", Some("app-secret"), true)
            .expect("save security settings");
        let store = SettingsStore { sqlite };
        let old_key = store
            .sqlite
            .database_security_settings()
            .expect("load old security settings")
            .encryption_key;

        let settings = store
            .regenerate_database_encryption_key()
            .expect("regenerate key");

        let new_key = store
            .sqlite
            .database_security_settings()
            .expect("load new security settings")
            .encryption_key;
        assert_ne!(old_key, new_key);
        assert_eq!(settings.feishu_app_id, "cli_test");
        assert!(settings.feishu_app_secret_configured);
        assert!(settings.auth_required);
        assert_eq!(
            store.feishu_credentials().expect("decrypt credentials"),
            ("cli_test".to_string(), "app-secret".to_string())
        );
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }

    fn target(provider_id: &str, model: &str) -> RoutingModelTarget {
        RoutingModelTarget {
            provider_id: provider_id.to_string(),
            model: model.to_string(),
            reasoning_effort: None,
        }
    }
}
