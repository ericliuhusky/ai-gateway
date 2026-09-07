use crate::{
    config::Config,
    models::{ChatGPTAuthRecord, PROVIDER_OPENAI_PROXY},
    openai_tokens::OpenAiTokenService,
    store::sqlite::SqliteStore,
    support::time::now_unix,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct AccountStore {
    sqlite: SqliteStore,
    records: Arc<Mutex<Vec<ChatGPTAuthRecord>>>,
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

    pub async fn add_openai_account(
        &self,
        account: ChatGPTAuthRecord,
    ) -> Result<ChatGPTAuthRecord, String> {
        let mut records = self.records.lock().await;
        if records.iter().any(|existing| {
            existing.email == account.email && existing.provider() == PROVIDER_OPENAI_PROXY
        }) {
            return Err(format!("OpenAI 账号已经存在: {}", account.email));
        }

        self.persist_account(&account)?;
        records.push(account.clone());
        Ok(account)
    }

    pub async fn acquire_by_id(
        &self,
        token_service: &OpenAiTokenService,
        account_id: &str,
    ) -> Result<ChatGPTAuthRecord, String> {
        let account = self
            .find_by_id(account_id)
            .await
            .ok_or_else(|| format!("账户不存在: {account_id}"))?;
        self.prepare_account_for_use(account, token_service).await
    }

    pub async fn find_by_id(&self, account_id: &str) -> Option<ChatGPTAuthRecord> {
        self.records
            .lock()
            .await
            .iter()
            .find(|account| account.id == account_id)
            .cloned()
    }

    pub async fn delete(&self, account_id: &str) -> Result<ChatGPTAuthRecord, String> {
        let mut records = self.records.lock().await;
        let index = records
            .iter()
            .position(|account| account.id == account_id)
            .ok_or_else(|| format!("账户不存在: {account_id}"))?;

        self.sqlite.delete_account(account_id)?;
        Ok(records.remove(index))
    }

    async fn update_account(&self, account: ChatGPTAuthRecord) -> Result<(), String> {
        self.persist_account(&account)?;
        let mut records = self.records.lock().await;
        if let Some(existing) = records.iter_mut().find(|item| item.id == account.id) {
            *existing = account.clone();
        } else {
            records.push(account.clone());
        }
        Ok(())
    }

    fn persist_account(&self, account: &ChatGPTAuthRecord) -> Result<(), String> {
        self.sqlite.upsert_account(account)
    }

    async fn prepare_account_for_use(
        &self,
        mut account: ChatGPTAuthRecord,
        token_service: &OpenAiTokenService,
    ) -> Result<ChatGPTAuthRecord, String> {
        if token_service.refresh_needed(account.expiry_timestamp) {
            let client_id = account
                .client_id()
                .ok_or_else(|| "openai account missing oauth client id".to_string())?;
            let refreshed = token_service
                .refresh_access_token(client_id, account.refresh_token())
                .await;

            match refreshed {
                Ok(refreshed) => {
                    *account.access_token_mut() = refreshed.access_token;
                    account.set_expiry_timestamp(now_unix() as i64 + refreshed.expires_in);
                    if let Some(refresh_token) = refreshed.refresh_token {
                        *account.refresh_token_mut() = refresh_token;
                    }
                }
                Err(err) => {
                    return Err(format!("refresh failed for {}: {err}", account.email));
                }
            }
        }

        self.update_account(account.clone()).await?;
        Ok(account)
    }
}
