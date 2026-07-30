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

    pub fn clear_auto_routing_provider(&self, provider_id: &str) -> Result<(), String> {
        let mut settings = self.auto_routing_settings()?;
        let mut changed = false;
        for target in [
            &mut settings.classifier,
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
            classifier: Some(target("provider_a", "small")),
            light: Some(target("provider_a", "small")),
            standard: Some(target("provider_b", "medium")),
            pro: Some(target("provider_b", "large")),
            max: Some(target("provider_c", "xlarge")),
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

    fn target(provider_id: &str, model: &str) -> RoutingModelTarget {
        RoutingModelTarget {
            provider_id: provider_id.to_string(),
            model: model.to_string(),
        }
    }
}
