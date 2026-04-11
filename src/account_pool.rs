use crate::{
    auth::{ImportedOpenAIAuth, OAuthClient, TokenResponse, UserInfo},
    config::Config,
    models::{AccountRecord, PROVIDER_GOOGLE_PROXY, PROVIDER_OPENAI_PROXY},
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
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| format!("invalid account filename: {}", path.display()))?
                .to_string();
            let account = serde_json::from_str::<AccountRecord>(&content)
                .map_err(|err| format!("parse account failed: {err}"))?
                .with_id(id);
            loaded.push(account);
        }

        *self.accounts.lock().await = loaded;
        Ok(self.accounts.lock().await.len())
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
        let expiry_timestamp = now + token.expires_in;

        let mut accounts = self.accounts.lock().await;
        let account = if let Some(existing) = accounts.iter_mut().find(|account| {
            account.email == user.email && account.provider() == PROVIDER_GOOGLE_PROXY
        }) {
            existing.name = PROVIDER_GOOGLE_PROXY.to_string();
            existing.provider_id = PROVIDER_GOOGLE_PROXY.to_string();
            existing.display_name = user.name.clone();
            existing.access_token = token.access_token;
            existing.refresh_token = refresh_token;
            existing.expiry_timestamp = expiry_timestamp;
            existing.client_id = None;
            existing.project_id = Some(project_id);
            existing.account_id = None;
            existing.updated_at = now;
            existing.last_used = now;
            existing.clone()
        } else {
            let account = AccountRecord {
                id: Uuid::new_v4().to_string(),
                name: PROVIDER_GOOGLE_PROXY.to_string(),
                provider_id: PROVIDER_GOOGLE_PROXY.to_string(),
                email: user.email,
                display_name: user.name,
                access_token: token.access_token,
                refresh_token,
                expiry_timestamp,
                client_id: None,
                project_id: Some(project_id),
                account_id: None,
                created_at: now,
                updated_at: now,
                last_used: now,
            };
            accounts.push(account.clone());
            account
        };

        self.persist_account(&account)?;
        Ok(account)
    }

    pub async fn add_openai_account(
        &self,
        imported: ImportedOpenAIAuth,
    ) -> Result<AccountRecord, String> {
        let now = now_unix() as i64;

        let mut accounts = self.accounts.lock().await;
        let account = if let Some(existing) = accounts.iter_mut().find(|account| {
            account.email == imported.email && account.provider() == PROVIDER_OPENAI_PROXY
        }) {
            existing.name = PROVIDER_OPENAI_PROXY.to_string();
            existing.provider_id = PROVIDER_OPENAI_PROXY.to_string();
            existing.access_token = imported.access_token;
            existing.refresh_token = imported.refresh_token;
            existing.expiry_timestamp = imported.expiry_timestamp;
            existing.client_id = Some(imported.client_id);
            existing.project_id = None;
            existing.account_id = imported.account_id;
            existing.updated_at = now;
            existing.last_used = now;
            existing.clone()
        } else {
            let account = AccountRecord {
                id: Uuid::new_v4().to_string(),
                name: PROVIDER_OPENAI_PROXY.to_string(),
                provider_id: PROVIDER_OPENAI_PROXY.to_string(),
                email: imported.email,
                display_name: None,
                access_token: imported.access_token,
                refresh_token: imported.refresh_token,
                expiry_timestamp: imported.expiry_timestamp,
                client_id: Some(imported.client_id),
                project_id: None,
                account_id: imported.account_id,
                created_at: now,
                updated_at: now,
                last_used: now,
            };
            accounts.push(account.clone());
            account
        };

        self.persist_account(&account)?;
        Ok(account)
    }

    pub async fn acquire_for_provider(
        &self,
        oauth: &OAuthClient,
        upstream: &UpstreamClient,
        provider: &str,
    ) -> Result<AccountRecord, String> {
        let snapshot: Vec<AccountRecord> = self
            .accounts
            .lock()
            .await
            .iter()
            .filter(|account| account.provider() == provider)
            .cloned()
            .collect();
        if snapshot.is_empty() {
            return Err(format!(
                "no {provider} accounts in pool; import or login before calling this route"
            ));
        }

        let start = self.next_index.fetch_add(1, Ordering::SeqCst);
        for offset in 0..snapshot.len() {
            let idx = (start + offset) % snapshot.len();
            match self
                .prepare_account_for_use(snapshot[idx].clone(), oauth, upstream)
                .await
            {
                Ok(account) => {
                    info!(
                        account_id = %account.id,
                        email = %account.email,
                        project_id = %account.project_id().unwrap_or(""),
                        index = idx,
                        "selected account from pool"
                    );
                    return Ok(account);
                }
                Err(err) => {
                    warn!(
                        "skipping unhealthy account for provider {}: {}",
                        provider, err
                    );
                }
            }
        }

        Err("no healthy accounts available".to_string())
    }

    pub async fn acquire_by_id(
        &self,
        oauth: &OAuthClient,
        upstream: &UpstreamClient,
        account_id: &str,
    ) -> Result<AccountRecord, String> {
        let account = self
            .find_by_id(account_id)
            .await
            .ok_or_else(|| format!("account not found: {account_id}"))?;
        self.prepare_account_for_use(account, oauth, upstream).await
    }

    pub async fn find_by_id(&self, account_id: &str) -> Option<AccountRecord> {
        self.accounts
            .lock()
            .await
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
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

    async fn prepare_account_for_use(
        &self,
        mut account: AccountRecord,
        oauth: &OAuthClient,
        upstream: &UpstreamClient,
    ) -> Result<AccountRecord, String> {
        if oauth.refresh_needed(account.expiry_timestamp) {
            info!(
                account_id = %account.id,
                email = %account.email,
                expires_at = account.expiry_timestamp,
                "refreshing access token before use"
            );
            let refreshed = if account.provider() == PROVIDER_OPENAI_PROXY {
                let client_id = account
                    .client_id()
                    .ok_or_else(|| "openai account missing oauth client id".to_string())?;
                oauth
                    .refresh_openai_access_token(client_id, account.refresh_token())
                    .await
            } else {
                oauth
                    .refresh_google_access_token(account.refresh_token())
                    .await
            };

            match refreshed {
                Ok(refreshed) => {
                    *account.access_token_mut() = refreshed.access_token;
                    account.set_expiry_timestamp(now_unix() as i64 + refreshed.expires_in);
                    if let Some(refresh_token) = refreshed.refresh_token {
                        *account.refresh_token_mut() = refresh_token;
                    }
                    account.updated_at = now_unix() as i64;
                    info!(
                        account_id = %account.id,
                        email = %account.email,
                        expires_at = account.expiry_timestamp,
                        "refreshed access token"
                    );
                }
                Err(err) => {
                    return Err(format!("refresh failed for {}: {err}", account.email));
                }
            }
        }

        if account.provider() == PROVIDER_GOOGLE_PROXY
            && account.project_id().unwrap_or("").is_empty()
        {
            info!(
                account_id = %account.id,
                email = %account.email,
                "fetching missing project_id"
            );
            match upstream.fetch_project_id(account.access_token()).await {
                Ok(project_id) => {
                    account.set_project_id(project_id);
                    account.updated_at = now_unix() as i64;
                    info!(
                        account_id = %account.id,
                        email = %account.email,
                        project_id = %account.project_id().unwrap_or(""),
                        "fetched project_id"
                    );
                }
                Err(err) => {
                    return Err(format!(
                        "project_id fetch failed for {}: {err}",
                        account.email
                    ));
                }
            }
        }

        let now = now_unix() as i64;
        account.last_used = now;
        account.updated_at = now;
        self.update_account(account.clone()).await?;
        Ok(account)
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
