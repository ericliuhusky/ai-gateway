use crate::{config::Config, models::AutoRoutingSettings};
use std::sync::Arc;

use super::sqlite::SqliteStore;

#[derive(Clone, Debug)]
pub struct SettingsStore {
    sqlite: SqliteStore,
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

    pub fn auto_routing_settings(&self) -> Result<AutoRoutingSettings, String> {
        self.sqlite.load_auto_routing_settings()
    }

    pub fn set_auto_routing_settings(&self, settings: &AutoRoutingSettings) -> Result<(), String> {
        self.sqlite.set_auto_routing_settings(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::SettingsStore;
    use crate::{models::AutoRoutingSettings, store::sqlite::SqliteStore};
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
            classifier_model: Some("small".to_string()),
            cheap_model: Some("small".to_string()),
            standard_model: Some("medium".to_string()),
            strong_model: Some("large".to_string()),
            low_confidence_threshold: 0.8,
        };

        store.set_auto_routing_settings(&settings).unwrap();
        assert_eq!(store.auto_routing_settings().unwrap(), settings);
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
