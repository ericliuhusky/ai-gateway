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

    pub async fn set(&self, provider_id: Option<String>) -> Result<SelectedProvider, String> {
        let route = SelectedProvider {
            provider_id,
            updated_at: now_unix() as i64,
        };
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
