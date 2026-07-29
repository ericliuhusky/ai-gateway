use crate::{
    config::Config,
    crypto::FieldEncryptor,
    models::{
        AccountRecord, AccountType, ApiProviderBillingMode, ApiProviderRecord,
        CachedProviderModels, ProviderAuthMode, ProviderCompatibilityProfile,
        ProviderUpstreamProtocol, SelectedRoute,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{fs, path::PathBuf, sync::Arc};
#[derive(Clone, Debug)]
pub struct SqliteStore {
    db_path: PathBuf,
    encryption: FieldEncryptor,
}

impl SqliteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        fs::create_dir_all(config.data_dir())
            .map_err(|err| format!("create data dir failed: {err}"))?;

        let store = Self {
            db_path: config.sqlite_path(),
            encryption: config.encryption(),
        };
        store.init()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn for_test(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create test data dir failed: {err}"))?;
        }

        let store = Self {
            db_path,
            encryption: FieldEncryptor::from_base64_key(
                "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
            )
            .expect("hard-coded test key is valid"),
        };
        store.init()?;
        Ok(store)
    }

    pub fn load_accounts(&self) -> Result<Vec<AccountRecord>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, account_type, email, access_token, refresh_token, expiry_timestamp, client_id, upstream_account_id
                 FROM accounts
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare accounts query failed: {err}"))?;
        let encryption = self.encryption.clone();
        let rows = stmt
            .query_map([], move |row| {
                Ok(AccountRecord {
                    id: row.get(0)?,
                    account_type: account_type_from_str(&row.get::<_, String>(1)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    email: row.get(2)?,
                    access_token: encryption
                        .decrypt(&row.get::<_, String>(3)?)
                        .map_err(decrypt_conversion_error)?,
                    refresh_token: encryption
                        .decrypt(&row.get::<_, String>(4)?)
                        .map_err(decrypt_conversion_error)?,
                    expiry_timestamp: row.get(5)?,
                    client_id: row.get(6)?,
                    upstream_account_id: row.get(7)?,
                })
            })
            .map_err(|err| format!("query accounts failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read accounts failed: {err}"))
    }

    pub fn upsert_account(&self, account: &AccountRecord) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_account_record(&conn, &self.encryption, account)
    }

    pub fn load_providers(&self) -> Result<Vec<ApiProviderRecord>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, auth_mode, COALESCE(base_url, ''), COALESCE(api_key, ''), account_id,
                        upstream_protocol, compatibility_profile, billing_mode
                 FROM providers
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare providers query failed: {err}"))?;
        let encryption = self.encryption.clone();
        let rows = stmt
            .query_map([], move |row| {
                Ok(ApiProviderRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    auth_mode: provider_auth_mode_from_str(&row.get::<_, String>(2)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    base_url: row.get(3)?,
                    api_key: {
                        let api_key = row.get::<_, String>(4)?;
                        if api_key.is_empty() {
                            String::new()
                        } else {
                            encryption
                                .decrypt(&api_key)
                                .map_err(decrypt_conversion_error)?
                        }
                    },
                    account_id: row.get(5)?,
                    upstream_protocol: upstream_protocol_from_str(&row.get::<_, String>(6)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    compatibility_profile: compatibility_profile_from_str(
                        &row.get::<_, String>(7)?,
                    )
                    .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    billing_mode: billing_mode_from_str(&row.get::<_, String>(8)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                })
            })
            .map_err(|err| format!("query providers failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read providers failed: {err}"))
    }

    pub fn upsert_provider(&self, provider: &ApiProviderRecord) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_provider_record(&conn, &self.encryption, provider)
    }

    pub fn delete_provider(&self, provider_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute("DELETE FROM providers WHERE id = ?1", params![provider_id])
            .map_err(|err| format!("delete provider failed: {err}"))?;
        Ok(())
    }

    pub fn load_route(&self) -> Result<SelectedRoute, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT state.selected_provider_id, provider.preferred_model, state.route_updated_at
             FROM gateway_state AS state
             LEFT JOIN providers AS provider ON provider.id = state.selected_provider_id
             WHERE state.id = 1",
            [],
            |row| {
                Ok(SelectedRoute {
                    provider_id: row.get(0)?,
                    selected_model: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("load route failed: {err}"))
        .map(|route| route.unwrap_or_default())
    }

    pub fn upsert_route(&self, route: &SelectedRoute) -> Result<(), String> {
        let mut conn = self.connect()?;
        let tx = conn
            .transaction()
            .map_err(|err| format!("begin route transaction failed: {err}"))?;

        tx.execute(
            "INSERT INTO gateway_state (id, selected_provider_id, route_updated_at)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                selected_provider_id = excluded.selected_provider_id,
                route_updated_at = excluded.route_updated_at",
            params![route.provider_id, route.updated_at],
        )
        .map_err(|err| format!("upsert route failed: {err}"))?;

        if let Some(provider_id) = route.provider_id.as_deref() {
            tx.execute(
                "UPDATE providers SET preferred_model = ?1 WHERE id = ?2",
                params![route.selected_model, provider_id],
            )
            .map_err(|err| format!("update provider preferred model failed: {err}"))?;
        }

        tx.commit()
            .map_err(|err| format!("commit route transaction failed: {err}"))?;
        Ok(())
    }

    pub fn load_provider_preferred_model(
        &self,
        provider_id: &str,
    ) -> Result<Option<String>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT preferred_model FROM providers WHERE id = ?1",
            params![provider_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("load provider preferred model failed: {err}"))
        .map(|value| value.flatten())
    }

    pub fn load_cached_models(
        &self,
        provider_id: &str,
    ) -> Result<Option<CachedProviderModels>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT provider_id, models_json, updated_at
             FROM provider_model_cache
             WHERE provider_id = ?1",
            params![provider_id],
            |row| {
                Ok(CachedProviderModels {
                    provider_id: row.get(0)?,
                    models_json: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("load cached provider models failed: {err}"))
    }

    pub fn upsert_cached_models(&self, models: &CachedProviderModels) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO provider_model_cache (provider_id, models_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(provider_id) DO UPDATE SET
                models_json = excluded.models_json,
                updated_at = excluded.updated_at",
            params![models.provider_id, models.models_json, models.updated_at],
        )
        .map_err(|err| format!("upsert cached provider models failed: {err}"))?;
        Ok(())
    }

    pub fn delete_cached_models(&self, provider_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "DELETE FROM provider_model_cache WHERE provider_id = ?1",
            params![provider_id],
        )
        .map_err(|err| format!("delete cached provider models failed: {err}"))?;
        Ok(())
    }

    pub fn load_codex_client_version_override(&self) -> Result<Option<String>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT codex_client_version_override FROM gateway_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("load Codex client version override failed: {err}"))
        .map(|value| value.flatten())
    }

    pub fn set_codex_client_version_override(&self, value: Option<&str>) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO gateway_state (id, codex_client_version_override, route_updated_at)
             VALUES (1, ?1, 0)
             ON CONFLICT(id) DO UPDATE SET
                codex_client_version_override = excluded.codex_client_version_override",
            params![value],
        )
        .map_err(|err| format!("upsert Codex client version override failed: {err}"))?;
        Ok(())
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                account_type TEXT NOT NULL,
                email TEXT NOT NULL,
                access_token TEXT NOT NULL,
                refresh_token TEXT NOT NULL,
                expiry_timestamp INTEGER NOT NULL,
                client_id TEXT,
                upstream_account_id TEXT
            );

            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_mode TEXT NOT NULL CHECK (auth_mode IN ('api_key', 'account')),
                base_url TEXT,
                api_key TEXT,
                account_id TEXT,
                upstream_protocol TEXT NOT NULL CHECK (
                    upstream_protocol IN ('openai_responses', 'openai_chat_completions')
                ),
                compatibility_profile TEXT NOT NULL CHECK (
                    compatibility_profile IN ('official_openai', 'generic_openai', 'openai_codex')
                ),
                billing_mode TEXT NOT NULL CHECK (billing_mode IN ('metered', 'subscription')),
                preferred_model TEXT,
                CHECK (
                    (auth_mode = 'api_key' AND account_id IS NULL)
                    OR (auth_mode = 'account' AND account_id IS NOT NULL)
                ),
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_providers_account_id ON providers(account_id);

            CREATE TABLE IF NOT EXISTS gateway_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                selected_provider_id TEXT,
                codex_client_version_override TEXT,
                route_updated_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (selected_provider_id) REFERENCES providers(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS provider_model_cache (
                provider_id TEXT PRIMARY KEY,
                models_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
            );
            ",
        )
        .map_err(|err| format!("initialize sqlite schema failed: {err}"))?;
        Ok(())
    }

    fn connect(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.db_path).map_err(|err| {
            format!(
                "open sqlite database {} failed: {err}",
                self.db_path.display()
            )
        })?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(|err| format!("configure sqlite connection failed: {err}"))?;
        Ok(conn)
    }
}

fn upsert_account_record(
    conn: &Connection,
    encryption: &FieldEncryptor,
    account: &AccountRecord,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO accounts (
            id, account_type, email, access_token, refresh_token, expiry_timestamp, client_id, upstream_account_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            account_type = excluded.account_type,
            email = excluded.email,
            access_token = excluded.access_token,
            refresh_token = excluded.refresh_token,
            expiry_timestamp = excluded.expiry_timestamp,
            client_id = excluded.client_id,
            upstream_account_id = excluded.upstream_account_id",
        params![
            account.id,
            account_type_to_str(&account.account_type),
            account.email,
            encryption.encrypt(&account.access_token)?,
            encryption.encrypt(&account.refresh_token)?,
            account.expiry_timestamp,
            account.client_id,
            account.upstream_account_id
        ],
    )
    .map_err(|err| format!("upsert account failed: {err}"))?;
    Ok(())
}

fn upsert_provider_record(
    conn: &Connection,
    encryption: &FieldEncryptor,
    provider: &ApiProviderRecord,
) -> Result<(), String> {
    let (base_url, api_key) = match provider.auth_mode {
        ProviderAuthMode::ApiKey => (
            Some(provider.base_url.as_str()),
            Some(encryption.encrypt(&provider.api_key)?),
        ),
        ProviderAuthMode::Account => (None, None),
    };
    conn.execute(
        "INSERT INTO providers (
            id, name, auth_mode, base_url, api_key, account_id,
            upstream_protocol, compatibility_profile, billing_mode
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            auth_mode = excluded.auth_mode,
            base_url = excluded.base_url,
            api_key = excluded.api_key,
            account_id = excluded.account_id,
            upstream_protocol = excluded.upstream_protocol,
            compatibility_profile = excluded.compatibility_profile,
            billing_mode = excluded.billing_mode",
        params![
            provider.id,
            provider.name,
            provider_auth_mode_to_str(&provider.auth_mode),
            base_url,
            api_key,
            provider.account_id.as_deref(),
            upstream_protocol_to_str(&provider.upstream_protocol),
            compatibility_profile_to_str(&provider.compatibility_profile),
            billing_mode_to_str(&provider.billing_mode)
        ],
    )
    .map_err(|err| format!("upsert provider failed: {err}"))?;
    Ok(())
}

fn account_type_to_str(value: &AccountType) -> &'static str {
    match value {
        AccountType::Openai => "openai",
    }
}

fn account_type_from_str(
    value: &str,
) -> Result<AccountType, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "openai" => Ok(AccountType::Openai),
        other => Err(format!("unknown account_type: {other}").into()),
    }
}

fn provider_auth_mode_to_str(value: &ProviderAuthMode) -> &'static str {
    match value {
        ProviderAuthMode::ApiKey => "api_key",
        ProviderAuthMode::Account => "account",
    }
}

fn provider_auth_mode_from_str(
    value: &str,
) -> Result<ProviderAuthMode, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "api_key" => Ok(ProviderAuthMode::ApiKey),
        "account" => Ok(ProviderAuthMode::Account),
        other => Err(format!("unknown auth_mode: {other}").into()),
    }
}

fn upstream_protocol_to_str(value: &ProviderUpstreamProtocol) -> &'static str {
    match value {
        ProviderUpstreamProtocol::OpenAiResponses => "openai_responses",
        ProviderUpstreamProtocol::OpenAiChatCompletions => "openai_chat_completions",
    }
}

fn upstream_protocol_from_str(
    value: &str,
) -> Result<ProviderUpstreamProtocol, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "openai_responses" => Ok(ProviderUpstreamProtocol::OpenAiResponses),
        "openai_chat_completions" => Ok(ProviderUpstreamProtocol::OpenAiChatCompletions),
        other => Err(format!("unknown upstream_protocol: {other}").into()),
    }
}

fn compatibility_profile_to_str(value: &ProviderCompatibilityProfile) -> &'static str {
    match value {
        ProviderCompatibilityProfile::OfficialOpenAi => "official_openai",
        ProviderCompatibilityProfile::GenericOpenAi => "generic_openai",
        ProviderCompatibilityProfile::OpenAiCodex => "openai_codex",
    }
}

fn compatibility_profile_from_str(
    value: &str,
) -> Result<ProviderCompatibilityProfile, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "official_openai" => Ok(ProviderCompatibilityProfile::OfficialOpenAi),
        "generic_openai" => Ok(ProviderCompatibilityProfile::GenericOpenAi),
        "openai_codex" => Ok(ProviderCompatibilityProfile::OpenAiCodex),
        other => Err(format!("unknown compatibility_profile: {other}").into()),
    }
}

fn billing_mode_to_str(value: &ApiProviderBillingMode) -> &'static str {
    match value {
        ApiProviderBillingMode::Metered => "metered",
        ApiProviderBillingMode::Subscription => "subscription",
    }
}

fn billing_mode_from_str(
    value: &str,
) -> Result<ApiProviderBillingMode, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "metered" => Ok(ApiProviderBillingMode::Metered),
        "subscription" => Ok(ApiProviderBillingMode::Subscription),
        other => Err(format!("unknown billing_mode: {other}").into()),
    }
}

fn decrypt_conversion_error(error: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
}

#[cfg(test)]
mod tests {
    use super::SqliteStore;
    use crate::models::{
        AccountRecord, AccountType, ApiProviderBillingMode, ApiProviderRecord,
        CachedProviderModels, ProviderAuthMode, ProviderCompatibilityProfile,
        ProviderUpstreamProtocol, SelectedRoute,
    };
    use rusqlite::Connection;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn deleting_provider_cascades_cache_and_clears_active_route() {
        let db_path = unique_test_db_path("provider-cascade");
        let store = SqliteStore::for_test(db_path.clone()).expect("create compact database");
        let provider = api_provider("provider-a");
        store.upsert_provider(&provider).expect("save provider");
        store
            .upsert_cached_models(&CachedProviderModels {
                provider_id: provider.id.clone(),
                models_json: "{\"object\":\"list\",\"data\":[]}".to_string(),
                updated_at: 1,
            })
            .expect("save model cache");
        store
            .upsert_route(&SelectedRoute {
                provider_id: Some(provider.id.clone()),
                selected_model: Some("model-a".to_string()),
                updated_at: 2,
            })
            .expect("save route");

        store
            .delete_provider(&provider.id)
            .expect("delete provider");

        assert_eq!(
            store.load_route().unwrap(),
            SelectedRoute {
                provider_id: None,
                selected_model: None,
                updated_at: 2,
            }
        );
        assert!(store.load_cached_models(&provider.id).unwrap().is_none());

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn account_provider_uses_null_transport_credentials() {
        let db_path = unique_test_db_path("account-provider-null-credentials");
        let store = SqliteStore::for_test(db_path.clone()).expect("create compact database");
        let account = AccountRecord {
            id: "account-1".to_string(),
            account_type: AccountType::Openai,
            email: "account@example.com".to_string(),
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expiry_timestamp: 1,
            client_id: Some("client".to_string()),
            upstream_account_id: Some("upstream".to_string()),
        };
        store.upsert_account(&account).expect("save account");
        let provider = ApiProviderRecord {
            id: "provider-account".to_string(),
            name: "account".to_string(),
            auth_mode: ProviderAuthMode::Account,
            base_url: String::new(),
            api_key: String::new(),
            account_id: Some(account.id.clone()),
            upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
            compatibility_profile: ProviderCompatibilityProfile::OpenAiCodex,
            billing_mode: ApiProviderBillingMode::Subscription,
        };
        store.upsert_provider(&provider).expect("save provider");

        let conn = Connection::open(&db_path).expect("open compact database");
        let (base_url, api_key): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT base_url, api_key FROM providers WHERE id = 'provider-account'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read provider row");
        assert_eq!(base_url, None);
        assert_eq!(api_key, None);

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn stores_credentials_as_authenticated_ciphertext() {
        let db_path = unique_test_db_path("encrypted-credentials");
        let store = SqliteStore::for_test(db_path.clone()).expect("create encrypted database");
        let account = AccountRecord {
            id: "account-1".to_string(),
            account_type: AccountType::Openai,
            email: "account@example.com".to_string(),
            access_token: "access-secret".to_string(),
            refresh_token: "refresh-secret".to_string(),
            expiry_timestamp: 1,
            client_id: None,
            upstream_account_id: None,
        };
        store.upsert_account(&account).expect("save account");
        store
            .upsert_provider(&api_provider("provider-1"))
            .expect("save provider");

        let conn = Connection::open(&db_path).expect("open encrypted database");
        let (access_token, refresh_token): (String, String) = conn
            .query_row(
                "SELECT access_token, refresh_token FROM accounts WHERE id = ?1",
                ["account-1"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read account credentials");
        let api_key: String = conn
            .query_row(
                "SELECT api_key FROM providers WHERE id = ?1",
                ["provider-1"],
                |row| row.get(0),
            )
            .expect("read provider credential");

        for (stored, plaintext) in [
            (access_token, "access-secret"),
            (refresh_token, "refresh-secret"),
            (api_key, "sk-test"),
        ] {
            assert!(stored.starts_with("aigw:v1:"));
            assert_ne!(stored, plaintext);
            assert!(!stored.contains(plaintext));
        }
        let loaded_account = store.load_accounts().unwrap().pop().unwrap();
        assert_eq!(loaded_account.access_token, account.access_token);
        assert_eq!(loaded_account.refresh_token, account.refresh_token);
        assert_eq!(store.load_providers().unwrap()[0].api_key, "sk-test");

        let _ = fs::remove_file(db_path);
    }

    fn api_provider(id: &str) -> ApiProviderRecord {
        ApiProviderRecord {
            id: id.to_string(),
            name: id.to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: "https://example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            account_id: None,
            upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
            compatibility_profile: ProviderCompatibilityProfile::GenericOpenAi,
            billing_mode: ApiProviderBillingMode::Metered,
        }
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
