use crate::{
    config::Config,
    models::{
        AccountRecord, AccountType, ApiProviderBillingMode, ApiProviderRecord,
        ProviderAuthMode, ProviderExtensionRecord, SelectedProvider,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{fs, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct SqliteStore {
    db_path: PathBuf,
}

impl SqliteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        fs::create_dir_all(config.data_dir())
            .map_err(|err| format!("create data dir failed: {err}"))?;

        let store = Self {
            db_path: config.sqlite_path(),
        };
        store.init()?;
        Ok(store)
    }

    pub fn load_accounts(&self) -> Result<Vec<AccountRecord>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, account_type, email, access_token, refresh_token, expiry_timestamp, client_id, project_id, upstream_account_id
                 FROM accounts
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare accounts query failed: {err}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AccountRecord {
                    id: row.get(0)?,
                    account_type: account_type_from_str(&row.get::<_, String>(1)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    email: row.get(2)?,
                    access_token: row.get(3)?,
                    refresh_token: row.get(4)?,
                    expiry_timestamp: row.get(5)?,
                    client_id: row.get(6)?,
                    project_id: row.get(7)?,
                    upstream_account_id: row.get(8)?,
                })
            })
            .map_err(|err| format!("query accounts failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read accounts failed: {err}"))
    }

    pub fn upsert_account(&self, account: &AccountRecord) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_account_record(&conn, account)
    }

    pub fn load_providers(&self) -> Result<Vec<ApiProviderRecord>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, auth_mode, base_url, api_key, account_id, billing_mode
                 FROM providers
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare providers query failed: {err}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ApiProviderRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    auth_mode: provider_auth_mode_from_str(&row.get::<_, String>(2)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    base_url: row.get(3)?,
                    api_key: row.get(4)?,
                    account_id: row.get(5)?,
                    billing_mode: billing_mode_from_str(&row.get::<_, String>(6)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                })
            })
            .map_err(|err| format!("query providers failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read providers failed: {err}"))
    }

    pub fn upsert_provider(&self, provider: &ApiProviderRecord) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_provider_record(&conn, provider)
    }

    pub fn load_provider_extensions(&self) -> Result<Vec<ProviderExtensionRecord>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT provider_id, extension_type, user_id, access_token
                 FROM provider_extensions
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare provider extensions query failed: {err}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProviderExtensionRecord {
                    provider_id: row.get(0)?,
                    extension_type: row.get(1)?,
                    user_id: row.get(2)?,
                    access_token: row.get(3)?,
                })
            })
            .map_err(|err| format!("query provider extensions failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read provider extensions failed: {err}"))
    }

    pub fn upsert_provider_extension(&self, extension: &ProviderExtensionRecord) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO provider_extensions (
                provider_id, extension_type, user_id, access_token
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_id, extension_type) DO UPDATE SET
                user_id = excluded.user_id,
                access_token = excluded.access_token",
            params![
                extension.provider_id,
                extension.extension_type,
                extension.user_id,
                extension.access_token,
            ],
        )
        .map_err(|err| format!("upsert provider extension failed: {err}"))?;
        Ok(())
    }

    pub fn load_route(&self) -> Result<SelectedProvider, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT provider_id, updated_at FROM selected_provider WHERE id = 1",
            [],
            |row| {
                Ok(SelectedProvider {
                    provider_id: row.get(0)?,
                    updated_at: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("load route failed: {err}"))?
        .map_or_else(|| Ok(SelectedProvider::default()), Ok)
    }

    pub fn upsert_route(&self, route: &SelectedProvider) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_route_record(&conn, route)
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                account_type TEXT NOT NULL,
                email TEXT NOT NULL,
                access_token TEXT NOT NULL,
                refresh_token TEXT NOT NULL,
                expiry_timestamp INTEGER NOT NULL,
                client_id TEXT,
                project_id TEXT,
                upstream_account_id TEXT
            );

            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                auth_mode TEXT NOT NULL,
                base_url TEXT NOT NULL,
                api_key TEXT NOT NULL,
                account_id TEXT,
                billing_mode TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS selected_provider (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                provider_id TEXT,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS provider_extensions (
                provider_id TEXT NOT NULL,
                extension_type TEXT NOT NULL,
                user_id TEXT NOT NULL,
                access_token TEXT NOT NULL,
                PRIMARY KEY (provider_id, extension_type)
            );
            ",
        )
        .map_err(|err| format!("initialize sqlite schema failed: {err}"))?;
        Ok(())
    }

    fn connect(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path).map_err(|err| {
            format!(
                "open sqlite database {} failed: {err}",
                self.db_path.display()
            )
        })
    }
}

fn upsert_account_record(conn: &Connection, account: &AccountRecord) -> Result<(), String> {
    conn.execute(
        "INSERT INTO accounts (
            id, account_type, email, access_token, refresh_token, expiry_timestamp, client_id, project_id, upstream_account_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            account_type = excluded.account_type,
            email = excluded.email,
            access_token = excluded.access_token,
            refresh_token = excluded.refresh_token,
            expiry_timestamp = excluded.expiry_timestamp,
            client_id = excluded.client_id,
            project_id = excluded.project_id,
            upstream_account_id = excluded.upstream_account_id",
        params![
            account.id,
            account_type_to_str(&account.account_type),
            account.email,
            account.access_token,
            account.refresh_token,
            account.expiry_timestamp,
            account.client_id,
            account.project_id,
            account.upstream_account_id
        ],
    )
    .map_err(|err| format!("upsert account failed: {err}"))?;
    Ok(())
}

fn upsert_provider_record(conn: &Connection, provider: &ApiProviderRecord) -> Result<(), String> {
    conn.execute(
        "INSERT INTO providers (
            id, name, auth_mode, base_url, api_key, account_id, billing_mode
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            auth_mode = excluded.auth_mode,
            base_url = excluded.base_url,
            api_key = excluded.api_key,
            account_id = excluded.account_id,
            billing_mode = excluded.billing_mode",
        params![
            provider.id,
            provider.name,
            provider_auth_mode_to_str(&provider.auth_mode),
            provider.base_url,
            provider.api_key,
            provider.account_id,
            billing_mode_to_str(&provider.billing_mode)
        ],
    )
    .map_err(|err| format!("upsert provider failed: {err}"))?;
    Ok(())
}

fn upsert_route_record(conn: &Connection, route: &SelectedProvider) -> Result<(), String> {
    conn.execute(
        "INSERT INTO selected_provider (id, provider_id, updated_at)
         VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET
            provider_id = excluded.provider_id,
            updated_at = excluded.updated_at",
        params![route.provider_id, route.updated_at],
    )
    .map_err(|err| format!("upsert route failed: {err}"))?;
    Ok(())
}

fn account_type_to_str(value: &AccountType) -> &'static str {
    match value {
        AccountType::Openai => "openai",
        AccountType::Google => "google",
    }
}

fn account_type_from_str(
    value: &str,
) -> Result<AccountType, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "openai" => Ok(AccountType::Openai),
        "google" => Ok(AccountType::Google),
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
