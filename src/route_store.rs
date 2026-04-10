use crate::{config::Config, models::RouteSelection};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct RouteStore {
    config: Arc<Config>,
    route: Arc<Mutex<RouteSelection>>,
}

impl RouteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let store = Self {
            config,
            route: Arc::new(Mutex::new(RouteSelection::default())),
        };
        store.ensure_dirs()?;
        Ok(store)
    }

    pub async fn load(&self) -> Result<RouteSelection, String> {
        let path = self.route_path();
        let route = if path.exists() {
            let content =
                fs::read_to_string(&path).map_err(|err| format!("read route failed: {err}"))?;
            serde_json::from_str::<RouteSelection>(&content)
                .map_err(|err| format!("parse route failed: {err}"))?
        } else {
            RouteSelection::default()
        };
        *self.route.lock().await = route.clone();
        Ok(route)
    }

    pub async fn get(&self) -> RouteSelection {
        self.route.lock().await.clone()
    }

    pub async fn set(&self, provider: Option<String>) -> Result<RouteSelection, String> {
        let route = RouteSelection {
            provider,
            updated_at: now_unix() as i64,
        };
        self.persist(&route)?;
        *self.route.lock().await = route.clone();
        Ok(route)
    }

    fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(self.config.data_dir())
            .map_err(|err| format!("create data dir failed: {err}"))
    }

    fn route_path(&self) -> PathBuf {
        self.config.data_dir().join("route.json")
    }

    fn persist(&self, route: &RouteSelection) -> Result<(), String> {
        let path = self.route_path();
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(route)
            .map_err(|err| format!("serialize route failed: {err}"))?;
        fs::write(&tmp, body).map_err(|err| format!("write temp route failed: {err}"))?;
        rename_replace(&tmp, &path)
    }
}

fn rename_replace(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        fs::remove_file(dst).map_err(|err| format!("remove old file failed: {err}"))?;
    }
    fs::rename(src, dst).map_err(|err| format!("rename failed: {err}"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
