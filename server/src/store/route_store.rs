use crate::{config::Config, models::SelectedProvider, store::sqlite::SqliteStore};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct RouteStore {
    sqlite: SqliteStore,
    route: Arc<Mutex<SelectedProvider>>,
}

impl RouteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let store = Self {
            sqlite: SqliteStore::new(config.clone())?,
            route: Arc::new(Mutex::new(SelectedProvider::default())),
        };
        Ok(store)
    }

    pub async fn load(&self) -> Result<SelectedProvider, String> {
        let route = self.sqlite.load_route()?;
        *self.route.lock().await = route.clone();
        Ok(route)
    }

    pub async fn get(&self) -> SelectedProvider {
        self.route.lock().await.clone()
    }

    pub async fn set_provider(
        &self,
        provider_id: Option<String>,
    ) -> Result<SelectedProvider, String> {
        let mut route = self.route.lock().await.clone();
        route.provider_id = provider_id;
        route.selected_model = None;
        route.updated_at = now_unix() as i64;
        self.sqlite.upsert_route(&route)?;
        *self.route.lock().await = route.clone();
        Ok(route)
    }

    pub async fn set_model(
        &self,
        selected_model: Option<String>,
    ) -> Result<SelectedProvider, String> {
        let mut route = self.route.lock().await.clone();
        route.selected_model = selected_model;
        route.updated_at = now_unix() as i64;
        self.sqlite.upsert_route(&route)?;
        *self.route.lock().await = route.clone();
        Ok(route)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
