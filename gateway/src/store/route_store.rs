use crate::{config::Config, domain::SelectedRoute, store::sqlite::SqliteStore};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct RouteStore {
    sqlite: SqliteStore,
}

impl RouteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    pub fn get(&self) -> Result<SelectedRoute, String> {
        self.sqlite.load_route()
    }

    pub fn update(
        &self,
        provider_id: Option<String>,
        model: Option<String>,
        reasoning_effort: Option<String>,
        load_provider_preferences: bool,
    ) -> Result<SelectedRoute, String> {
        let (model, reasoning_effort) = if load_provider_preferences {
            match provider_id.as_deref() {
                Some(provider_id) => (
                    self.sqlite.load_provider_preferred_model(provider_id)?,
                    self.sqlite
                        .load_provider_preferred_reasoning_effort(provider_id)?,
                ),
                None => (None, None),
            }
        } else {
            (model, reasoning_effort)
        };
        let route = SelectedRoute {
            provider_id,
            model,
            reasoning_effort,
        };

        self.sqlite.upsert_route(&route)?;
        Ok(route)
    }
}

#[cfg(test)]
mod tests {
    use super::RouteStore;
    use crate::{
        domain::ProviderAuthMode, domain::SelectedRoute, store::ProviderRecord,
        store::sqlite::SqliteStore,
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[tokio::test]
    async fn update_persists_final_route_and_provider_preferences() {
        let db_path = unique_test_db_path("route-update");
        let sqlite = SqliteStore::for_test(db_path.clone()).expect("create sqlite store");
        let provider = ProviderRecord {
            id: "provider-a".to_string(),
            name: "Provider A".to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: Some("https://example.com/v1".to_string()),
            api_key: Some("key".to_string()),
            access_token: None,
            refresh_token: None,
            expiry_timestamp: None,
            client_id: None,
        };
        sqlite.upsert_provider(&provider).expect("save provider");
        sqlite
            .upsert_route(&SelectedRoute {
                provider_id: Some(provider.id.clone()),
                model: Some("preferred-model".to_string()),
                reasoning_effort: Some("medium".to_string()),
            })
            .expect("save provider preferences");
        let store = RouteStore {
            sqlite: sqlite.clone(),
        };

        let preferred_route = store
            .update(Some(provider.id.clone()), None, None, true)
            .expect("load provider preferences");
        assert_eq!(preferred_route.model, Some("preferred-model".to_string()));
        assert_eq!(preferred_route.reasoning_effort, Some("medium".to_string()));

        let route = store
            .update(
                Some(provider.id.clone()),
                Some("model-a".to_string()),
                Some("high".to_string()),
                false,
            )
            .expect("update route");

        assert_eq!(store.get().unwrap(), route);
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
