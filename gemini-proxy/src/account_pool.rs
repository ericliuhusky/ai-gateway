use crate::{
    auth::{OAuthClient, TokenResponse, UserInfo},
    config::Config,
    models::{AccountRecord, AccountSummary, TokenData},
    upstream::UpstreamClient,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct AccountPool {
    config: Arc<Config>,
    accounts: Arc<Mutex<Vec<AccountRecord>>>,
    next_index: Arc<AtomicUsize>,
}

impl AccountPool {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let pool = Self {
            config,
            accounts: Arc::new(Mutex::new(Vec::new())),
            next_index: Arc::new(AtomicUsize::new(0)),
        };
        pool.ensure_dirs()?;
        Ok(pool)
    }

    pub async fn load(&self) -> Result<usize, String> {
        let accounts_dir = self.accounts_dir();
        let mut loaded = Vec::new();

        for entry in fs::read_dir(accounts_dir).map_err(|err| format!("read_dir failed: {err}"))? {
            let entry = entry.map_err(|err| format!("dir entry failed: {err}"))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let content =
                fs::read_to_string(&path).map_err(|err| format!("read account failed: {err}"))?;
            let account = serde_json::from_str::<AccountRecord>(&content)
                .map_err(|err| format!("parse account failed: {err}"))?;
            loaded.push(account);
        }

        *self.accounts.lock().await = loaded;
        Ok(self.accounts.lock().await.len())
    }

    pub async fn list(&self) -> Vec<AccountSummary> {
        self.accounts
            .lock()
            .await
            .iter()
            .map(|account| AccountSummary {
                id: account.id.clone(),
                email: account.email.clone(),
                name: account.name.clone(),
                has_project_id: account.token.project_id.is_some(),
                disabled: account.disabled,
                last_used: account.last_used,
            })
            .collect()
    }

    pub async fn add_oauth_account(
        &self,
        user: UserInfo,
        token: TokenResponse,
        project_id: String,
    ) -> Result<AccountRecord, String> {
        let refresh_token = token
            .refresh_token
            .ok_or_else(|| "google did not return refresh_token".to_string())?;
        let now = now_unix() as i64;

        let token_data = TokenData {
            access_token: token.access_token,
            refresh_token,
            expires_in: token.expires_in,
            expiry_timestamp: now + token.expires_in,
            token_type: if token.token_type.is_empty() {
                "Bearer".to_string()
            } else {
                token.token_type
            },
            email: Some(user.email.clone()),
            project_id: Some(project_id),
            oauth_client_key: None,
        };

        let mut accounts = self.accounts.lock().await;
        let account = if let Some(existing) = accounts
            .iter_mut()
            .find(|account| account.email == user.email)
        {
            existing.name = user.name.clone();
            existing.token = token_data;
            existing.disabled = false;
            existing.disabled_reason = None;
            existing.last_used = now;
            existing.clone()
        } else {
            let account = AccountRecord {
                id: Uuid::new_v4().to_string(),
                email: user.email,
                name: user.name,
                token: token_data,
                created_at: now,
                last_used: now,
                disabled: false,
                disabled_reason: None,
            };
            accounts.push(account.clone());
            account
        };

        self.persist_account(&account)?;
        Ok(account)
    }

    pub async fn acquire(
        &self,
        oauth: &OAuthClient,
        upstream: &UpstreamClient,
    ) -> Result<AccountRecord, String> {
        let snapshot = self.accounts.lock().await.clone();
        if snapshot.is_empty() {
            return Err("no accounts in pool; login first via /auth/google/start".to_string());
        }

        let start = self.next_index.fetch_add(1, Ordering::SeqCst);
        for offset in 0..snapshot.len() {
            let idx = (start + offset) % snapshot.len();
            let mut account = snapshot[idx].clone();
            if account.disabled {
                info!(
                    account_id = %account.id,
                    email = %account.email,
                    index = idx,
                    "skipping disabled account"
                );
                continue;
            }

            if oauth.refresh_needed(account.token.expiry_timestamp) {
                info!(
                    account_id = %account.id,
                    email = %account.email,
                    expires_at = account.token.expiry_timestamp,
                    "refreshing access token before use"
                );
                match oauth
                    .refresh_access_token(&account.token.refresh_token)
                    .await
                {
                    Ok(refreshed) => {
                        account.token.access_token = refreshed.access_token;
                        account.token.expires_in = refreshed.expires_in;
                        account.token.expiry_timestamp = now_unix() as i64 + refreshed.expires_in;
                        if !refreshed.token_type.is_empty() {
                            account.token.token_type = refreshed.token_type;
                        }
                        info!(
                            account_id = %account.id,
                            email = %account.email,
                            expires_at = account.token.expiry_timestamp,
                            "refreshed access token"
                        );
                    }
                    Err(err) => {
                        warn!("refresh failed for {}: {}", account.email, err);
                        self.mark_disabled(&account.id, err).await?;
                        continue;
                    }
                }
            }

            if account.token.project_id.as_deref().unwrap_or("").is_empty() {
                info!(
                    account_id = %account.id,
                    email = %account.email,
                    "fetching missing project_id"
                );
                match upstream.fetch_project_id(&account.token.access_token).await {
                    Ok(project_id) => {
                        account.token.project_id = Some(project_id);
                        info!(
                            account_id = %account.id,
                            email = %account.email,
                            project_id = %account.token.project_id.as_deref().unwrap_or(""),
                            "fetched project_id"
                        );
                    }
                    Err(err) => {
                        warn!("project_id fetch failed for {}: {}", account.email, err);
                        continue;
                    }
                }
            }

            account.last_used = now_unix() as i64;
            self.update_account(account.clone()).await?;
            info!(
                account_id = %account.id,
                email = %account.email,
                project_id = %account.token.project_id.as_deref().unwrap_or(""),
                index = idx,
                "selected account from pool"
            );

            return Ok(account);
        }

        Err("no healthy accounts available".to_string())
    }

    async fn update_account(&self, account: AccountRecord) -> Result<(), String> {
        let mut accounts = self.accounts.lock().await;
        if let Some(existing) = accounts.iter_mut().find(|item| item.id == account.id) {
            *existing = account.clone();
        } else {
            accounts.push(account.clone());
        }
        self.persist_account(&account)
    }

    async fn mark_disabled(&self, account_id: &str, reason: String) -> Result<(), String> {
        let mut accounts = self.accounts.lock().await;
        if let Some(account) = accounts.iter_mut().find(|item| item.id == account_id) {
            account.disabled = true;
            account.disabled_reason = Some(reason);
            self.persist_account(account)?;
        }
        Ok(())
    }

    fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(self.accounts_dir())
            .map_err(|err| format!("create data dir failed: {err}"))
    }

    fn accounts_dir(&self) -> PathBuf {
        self.config.data_dir().join("accounts")
    }

    fn persist_account(&self, account: &AccountRecord) -> Result<(), String> {
        let path = self.account_path(&account.id);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(account)
            .map_err(|err| format!("serialize account failed: {err}"))?;
        fs::write(&tmp, body).map_err(|err| format!("write temp account failed: {err}"))?;
        rename_replace(&tmp, &path)
    }

    fn account_path(&self, account_id: &str) -> PathBuf {
        self.accounts_dir().join(format!("{account_id}.json"))
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
