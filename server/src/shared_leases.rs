use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

pub const SHARED_PROVIDER_PREFIX: &str = "shared_";

#[derive(Clone, Default)]
pub struct SharedLeaseStore {
    expires_at: Arc<RwLock<HashMap<String, i64>>>,
}

impl SharedLeaseStore {
    pub fn authorize(&self, provider_id: &str, expires_at: i64) -> Result<(), String> {
        self.expires_at
            .write()
            .map_err(|_| "共享凭据租约锁异常".to_string())?
            .insert(provider_id.to_string(), expires_at);
        Ok(())
    }

    pub fn revoke(&self, provider_id: &str) -> Result<(), String> {
        self.expires_at
            .write()
            .map_err(|_| "共享凭据租约锁异常".to_string())?
            .remove(provider_id);
        Ok(())
    }

    pub fn ensure_active(&self, provider_id: &str) -> Result<(), String> {
        if !provider_id.starts_with(SHARED_PROVIDER_PREFIX) {
            return Ok(());
        }
        let expires_at = self
            .expires_at
            .read()
            .map_err(|_| "共享凭据租约锁异常".to_string())?
            .get(provider_id)
            .copied()
            .ok_or_else(|| "共享供应商尚未取得有效授权，请先同步共享连接".to_string())?;
        if expires_at <= now_unix() {
            return Err("共享供应商授权已经过期，请重新连接中心服务".to_string());
        }
        Ok(())
    }
}

pub fn local_shared_provider_id(central_provider_id: &str) -> String {
    format!("{SHARED_PROVIDER_PREFIX}{central_provider_id}")
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
