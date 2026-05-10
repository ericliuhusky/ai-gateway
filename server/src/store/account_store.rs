use crate::{
    auth::{
        ImportedOpenAIAuth, OAuthClient, TokenResponse, UserInfo, extract_openai_chatgpt_account_id,
    },
    config::Config,
    models::{AccountRecord, AccountType, PROVIDER_GOOGLE_PROXY, PROVIDER_OPENAI_PROXY},
    store::sqlite::SqliteStore,
    support::time::now_unix,
    upstream::UpstreamClient,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct AccountStore {
    sqlite: SqliteStore,
    records: Arc<Mutex<Vec<AccountRecord>>>,
}

impl AccountStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config.clone())?,
            records: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub async fn load(&self) -> Result<(), String> {
        let loaded = self.sqlite.load_accounts()?;
        *self.records.lock().await = loaded;
        Ok(())
    }

    pub async fn add_google_account(
        &self,
        user: UserInfo,
        token: TokenResponse,
        project_id: Option<String>,
    ) -> Result<AccountRecord, String> {
        let refresh_token = token
            .refresh_token
            .ok_or_else(|| "google did not return refresh_token".to_string())?;
        let expiry_timestamp = now_unix() as i64 + token.expires_in;

        let mut records = self.records.lock().await;
        if records.iter().any(|account| {
            account.email == user.email && account.provider() == PROVIDER_GOOGLE_PROXY
        }) {
            return Err(format!("Google 账号已经存在: {}", user.email));
        }

        let account = AccountRecord {
            id: Uuid::new_v4().to_string(),
            account_type: AccountType::Google,
            email: user.email,
            access_token: token.access_token,
            refresh_token,
            expiry_timestamp,
            client_id: None,
            project_id,
            upstream_account_id: None,
        };
        records.push(account.clone());

        self.persist_account(&account)?;
        Ok(account)
    }

    pub async fn add_openai_account(
        &self,
        imported: ImportedOpenAIAuth,
    ) -> Result<AccountRecord, String> {
        let mut records = self.records.lock().await;
        if records.iter().any(|account| {
            account.email == imported.email && account.provider() == PROVIDER_OPENAI_PROXY
        }) {
            return Err(format!("OpenAI 账号已经存在: {}", imported.email));
        }

        let account = AccountRecord {
            id: Uuid::new_v4().to_string(),
            account_type: AccountType::Openai,
            email: imported.email,
            access_token: imported.access_token,
            refresh_token: imported.refresh_token,
            expiry_timestamp: imported.expiry_timestamp,
            client_id: Some(imported.client_id),
            project_id: None,
            upstream_account_id: imported.account_id,
        };
        records.push(account.clone());

        self.persist_account(&account)?;
        Ok(account)
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
            .ok_or_else(|| format!("账户不存在: {account_id}"))?;
        self.prepare_account_for_use(account, oauth, upstream).await
    }

    pub async fn find_by_id(&self, account_id: &str) -> Option<AccountRecord> {
        self.records
            .lock()
            .await
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    }

    async fn update_account(&self, account: AccountRecord) -> Result<(), String> {
        let mut records = self.records.lock().await;
        if let Some(existing) = records.iter_mut().find(|item| item.id == account.id) {
            *existing = account.clone();
        } else {
            records.push(account.clone());
        }
        self.persist_account(&account)
    }

    fn persist_account(&self, account: &AccountRecord) -> Result<(), String> {
        self.sqlite.upsert_account(account)
    }

    async fn prepare_account_for_use(
        &self,
        mut account: AccountRecord,
        oauth: &OAuthClient,
        upstream: &UpstreamClient,
    ) -> Result<AccountRecord, String> {
        if oauth.refresh_needed(account.expiry_timestamp) {
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
                    if account.provider() == PROVIDER_OPENAI_PROXY {
                        account.upstream_account_id =
                            extract_openai_chatgpt_account_id(account.access_token());
                    }
                }
                Err(err) => {
                    return Err(format!("refresh failed for {}: {err}", account.email));
                }
            }
        }

        if account.provider() == PROVIDER_GOOGLE_PROXY
            && account.project_id().unwrap_or("").is_empty()
        {
            if let Ok(project_id) = upstream.fetch_project_id(account.access_token()).await {
                account.set_project_id(project_id);
            }
        }

        if account.provider() == PROVIDER_OPENAI_PROXY && account.upstream_account_id.is_none() {
            account.upstream_account_id = extract_openai_chatgpt_account_id(account.access_token());
        }

        self.update_account(account.clone()).await?;
        Ok(account)
    }
}
