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

    pub async fn update(
        &self,
        provider_id: Option<String>,
        selected_model: Option<String>,
        selected_reasoning_effort: Option<String>,
        load_provider_preferences: bool,
    ) -> Result<SelectedRoute, String> {
        let mut current = self.route.lock().await;
        let (selected_model, selected_reasoning_effort) = if load_provider_preferences {
            match provider_id.as_deref() {
                Some(provider_id) => (
                    self.sqlite.load_provider_preferred_model(provider_id)?,
                    self.sqlite
                        .load_provider_preferred_reasoning_effort(provider_id)?,
                ),
                None => (None, None),
            }
        } else {
            (selected_model, selected_reasoning_effort)
        };
        let route = SelectedRoute {
            provider_id,
            selected_model,
            selected_reasoning_effort,
            updated_at: now_unix() as i64,
        };

        self.sqlite.upsert_route(&route)?;
        *current = route.clone();
        Ok(route)
    }
}

#[cfg(test)]
mod tests {
    use super::RouteStore;
    use crate::{
        models::{ApiProviderRecord, ProviderAuthMode, SelectedRoute},
        store::sqlite::SqliteStore,
    };
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[tokio::test]
    async fn update_persists_final_route_and_provider_preferences() {
        let db_path = unique_test_db_path("route-update");
        let sqlite = SqliteStore::for_test(db_path.clone()).expect("create sqlite store");
        let provider = ApiProviderRecord {
            id: "provider-a".to_string(),
            name: "Provider A".to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: "https://example.com/v1".to_string(),
            api_key: "key".to_string(),
            account_id: None,
            owner_user_id: None,
        };
        sqlite.upsert_provider(&provider).expect("save provider");
        sqlite
            .upsert_route(&SelectedRoute {
                provider_id: Some(provider.id.clone()),
                selected_model: Some("preferred-model".to_string()),
                selected_reasoning_effort: Some("medium".to_string()),
                updated_at: 1,
            })
            .expect("save provider preferences");
        let store = RouteStore {
            sqlite: sqlite.clone(),
            route: Arc::new(tokio::sync::Mutex::new(SelectedRoute::default())),
        };

        let preferred_route = store
            .update(Some(provider.id.clone()), None, None, true)
            .await
            .expect("load provider preferences");
        assert_eq!(
            preferred_route.selected_model,
            Some("preferred-model".to_string())
        );
        assert_eq!(
            preferred_route.selected_reasoning_effort,
            Some("medium".to_string())
        );

        let route = store
            .update(
                Some(provider.id.clone()),
                Some("model-a".to_string()),
                Some("high".to_string()),
                false,
            )
            .await
            .expect("update route");

        assert_eq!(store.get().await, route);
        assert_eq!(sqlite.load_route().expect("load route"), route);
        assert_eq!(
            sqlite
                .load_provider_preferred_model(&provider.id)
                .expect("load preferred model"),
            Some("model-a".to_string())
        );
        assert_eq!(
            sqlite
                .load_provider_preferred_reasoning_effort(&provider.id)
                .expect("load preferred effort"),
            Some("high".to_string())
        );

        let _ = fs::remove_file(db_path);
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
