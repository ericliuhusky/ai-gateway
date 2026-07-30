use crate::{
    config::Config, models::SelectedRoute, store::sqlite::SqliteStore, support::time::now_unix,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct RouteStore {
    sqlite: SqliteStore,
    route: Arc<Mutex<SelectedRoute>>,
}

impl RouteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let store = Self {
            sqlite: SqliteStore::new(config.clone())?,
            route: Arc::new(Mutex::new(SelectedRoute::default())),
        };
        Ok(store)
    }

    pub async fn load(&self) -> Result<(), String> {
        let route = self.sqlite.load_route()?;
        *self.route.lock().await = route.clone();
        Ok(())
    }

    pub async fn get(&self) -> SelectedRoute {
        self.route.lock().await.clone()
    }

    pub async fn set_provider(&self, provider_id: Option<String>) -> Result<SelectedRoute, String> {
        let mut route = self.route.lock().await.clone();
        route.selected_model = match provider_id.as_deref() {
            Some(provider_id) => self.sqlite.load_provider_preferred_model(provider_id)?,
            None => None,
        };
        route.selected_reasoning_effort = match provider_id.as_deref() {
            Some(provider_id) => self
                .sqlite
                .load_provider_preferred_reasoning_effort(provider_id)?,
            None => None,
        };
        route.provider_id = provider_id;
        route.updated_at = now_unix() as i64;
        self.sqlite.upsert_route(&route)?;
        *self.route.lock().await = route.clone();
        Ok(route)
    }

    pub async fn set_model(&self, selected_model: Option<String>) -> Result<SelectedRoute, String> {
        let mut route = self.route.lock().await.clone();
        route.selected_model = selected_model;
        route.updated_at = now_unix() as i64;
        self.sqlite.upsert_route(&route)?;
        *self.route.lock().await = route.clone();
        Ok(route)
    }

    pub async fn set_reasoning_effort(
        &self,
        selected_reasoning_effort: Option<String>,
    ) -> Result<SelectedRoute, String> {
        let mut route = self.route.lock().await.clone();
        route.selected_reasoning_effort = selected_reasoning_effort;
        route.updated_at = now_unix() as i64;
        self.sqlite.upsert_route(&route)?;
        *self.route.lock().await = route.clone();
        Ok(route)
    }
}

#[cfg(test)]
mod tests {
    use super::RouteStore;
    use crate::{
        models::{
            ApiProviderRecord, ProviderAuthMode, ProviderCompatibilityProfile,
            ProviderUpstreamProtocol, SelectedRoute,
        },
        store::sqlite::SqliteStore,
    };
    use std::{
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn remembers_selected_model_per_provider() {
        let sqlite = test_sqlite_store("provider-selected-models");
        for provider_id in ["provider_a", "provider_b"] {
            sqlite
                .upsert_provider(&api_provider(provider_id))
                .expect("save provider");
        }
        let store = RouteStore {
            sqlite,
            route: Arc::new(Mutex::new(SelectedRoute::default())),
        };

        let route = store
            .set_provider(Some("provider_a".to_string()))
            .await
            .expect("select provider a");
        assert_eq!(route.selected_model, None);

        let route = store
            .set_model(Some("model_e".to_string()))
            .await
            .expect("select model e for provider a");
        assert_eq!(route.selected_model.as_deref(), Some("model_e"));
        let route = store
            .set_reasoning_effort(Some("high".to_string()))
            .await
            .expect("select high effort for provider a");
        assert_eq!(route.selected_reasoning_effort.as_deref(), Some("high"));

        let route = store
            .set_provider(Some("provider_b".to_string()))
            .await
            .expect("select provider b");
        assert_eq!(route.selected_model, None);
        assert_eq!(route.selected_reasoning_effort, None);

        let route = store
            .set_model(Some("model_f".to_string()))
            .await
            .expect("select model f for provider b");
        assert_eq!(route.selected_model.as_deref(), Some("model_f"));

        let route = store
            .set_provider(Some("provider_a".to_string()))
            .await
            .expect("switch back to provider a");
        assert_eq!(route.selected_model.as_deref(), Some("model_e"));
        assert_eq!(route.selected_reasoning_effort.as_deref(), Some("high"));

        let route = store
            .set_provider(Some("provider_b".to_string()))
            .await
            .expect("switch back to provider b");
        assert_eq!(route.selected_model.as_deref(), Some("model_f"));
    }

    fn test_sqlite_store(prefix: &str) -> SqliteStore {
        let db_path = unique_test_db_path(prefix);
        SqliteStore::for_test(db_path).expect("create sqlite store")
    }

    fn api_provider(id: &str) -> ApiProviderRecord {
        ApiProviderRecord {
            id: id.to_string(),
            name: id.to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: "https://example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            account_id: None,
            upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
            compatibility_profile: ProviderCompatibilityProfile::GenericOpenAi,
        }
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
