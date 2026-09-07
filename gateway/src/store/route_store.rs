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
