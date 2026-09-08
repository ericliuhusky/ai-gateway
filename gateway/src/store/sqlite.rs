use crate::{
    config::Config,
    models::{
        CachedProviderModels, GatewayIssue, GatewayIssueRecord, ProviderAuthMode, ProviderRecord,
        SelectedRoute,
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

    #[cfg(test)]
    pub fn for_test(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create test data dir failed: {err}"))?;
        }

        let store = Self { db_path };
        store.init()?;
        Ok(store)
    }

    pub fn load_providers(&self) -> Result<Vec<ProviderRecord>, String> {
        let conn = self.connect()?;
        let has_providers: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM providers)", [], |row| {
                row.get(0)
            })
            .map_err(|err| format!("check providers failed: {err}"))?;
        if !has_providers {
            return Ok(Vec::new());
        }
        let mut stmt = conn
            .prepare(
                "SELECT id, name, auth_mode, COALESCE(base_url, ''), COALESCE(api_key, ''),
                        email, access_token, refresh_token, expiry_timestamp, client_id,
                        upstream_account_id, owner_user_id
                 FROM providers
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare providers query failed: {err}"))?;
        let rows = stmt
            .query_map([], move |row| {
                Ok(ProviderRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    auth_mode: provider_auth_mode_from_str(&row.get::<_, String>(2)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    base_url: row.get(3)?,
                    api_key: row.get(4)?,
                    email: row.get(5)?,
                    access_token: row.get(6)?,
                    refresh_token: row.get(7)?,
                    expiry_timestamp: row.get(8)?,
                    client_id: row.get(9)?,
                    upstream_account_id: row.get(10)?,
                    owner_user_id: row.get(11)?,
                })
            })
            .map_err(|err| format!("query providers failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read providers failed: {err}"))
    }

    pub fn upsert_provider(&self, provider: &ProviderRecord) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_provider_record(&conn, provider)
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
            "SELECT state.selected_provider_id, provider.preferred_model,
                    provider.preferred_reasoning_effort, state.route_updated_at
             FROM gateway_state AS state
             LEFT JOIN providers AS provider ON provider.id = state.selected_provider_id
             WHERE state.id = 1",
            [],
            |row| {
                Ok(SelectedRoute {
                    provider_id: row.get(0)?,
                    selected_model: row.get(1)?,
                    selected_reasoning_effort: row.get(2)?,
                    updated_at: row.get(3)?,
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
                "UPDATE providers
                 SET preferred_model = ?1, preferred_reasoning_effort = ?2
                 WHERE id = ?3",
                params![
                    route.selected_model,
                    route.selected_reasoning_effort,
                    provider_id
                ],
            )
            .map_err(|err| format!("update provider routing preferences failed: {err}"))?;
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

    pub fn load_provider_preferred_reasoning_effort(
        &self,
        provider_id: &str,
    ) -> Result<Option<String>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT preferred_reasoning_effort FROM providers WHERE id = ?1",
            params![provider_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("load provider preferred reasoning effort failed: {err}"))
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

    pub fn record_gateway_issue(
        &self,
        issue: &GatewayIssueRecord,
        limit: i64,
    ) -> Result<(), String> {
        let owner_user_id = issue.owner_user_id.unwrap_or(0);
        let mut conn = self.connect()?;
        let transaction = conn
            .transaction()
            .map_err(|err| format!("begin gateway issue transaction failed: {err}"))?;
        transaction
            .execute(
                "INSERT INTO gateway_issues (
                    id, owner_user_id, provider_id, provider_name, model,
                    upstream_url, failure_kind, status_code, error_message,
                    upstream_response, upstream_response_truncated, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    issue.id,
                    owner_user_id,
                    issue.provider_id,
                    issue.provider_name,
                    issue.model,
                    issue.upstream_url,
                    issue.failure_kind,
                    issue.status_code,
                    issue.error_message,
                    issue.upstream_response,
                    i64::from(issue.upstream_response_truncated),
                    issue.created_at,
                ],
            )
            .map_err(|err| format!("insert gateway issue failed: {err}"))?;
        transaction
            .execute(
                "DELETE FROM gateway_issues
                 WHERE owner_user_id = ?1 AND id IN (
                    SELECT id FROM gateway_issues
                    WHERE owner_user_id = ?1
                    ORDER BY created_at DESC, rowid DESC
                    LIMIT -1 OFFSET ?2
                 )",
                params![owner_user_id, limit],
            )
            .map_err(|err| format!("trim gateway issues failed: {err}"))?;
        transaction
            .commit()
            .map_err(|err| format!("commit gateway issue transaction failed: {err}"))
    }

    pub fn list_gateway_issues(
        &self,
        owner_user_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<GatewayIssue>, String> {
        let conn = self.connect()?;
        let mut statement = conn
            .prepare(
                "SELECT id, provider_id, provider_name, model, upstream_url,
                        failure_kind, status_code, error_message, upstream_response,
                        upstream_response_truncated, created_at
                 FROM gateway_issues
                 WHERE owner_user_id = ?1
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT ?2",
            )
            .map_err(|err| format!("prepare gateway issue list failed: {err}"))?;
        statement
            .query_map(
                params![owner_user_id.unwrap_or(0), limit],
                gateway_issue_from_row,
            )
            .map_err(|err| format!("query gateway issues failed: {err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read gateway issues failed: {err}"))
    }

    pub fn load_gateway_issue(
        &self,
        owner_user_id: Option<i64>,
        issue_id: &str,
    ) -> Result<Option<GatewayIssue>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT id, provider_id, provider_name, model, upstream_url,
                    failure_kind, status_code, error_message, upstream_response,
                    upstream_response_truncated, created_at
             FROM gateway_issues
             WHERE owner_user_id = ?1 AND id = ?2",
            params![owner_user_id.unwrap_or(0), issue_id],
            gateway_issue_from_row,
        )
        .optional()
        .map_err(|err| format!("load gateway issue failed: {err}"))
    }

    pub fn clear_gateway_issues(&self, owner_user_id: Option<i64>) -> Result<usize, String> {
        let conn = self.connect()?;
        conn.execute(
            "DELETE FROM gateway_issues WHERE owner_user_id = ?1",
            params![owner_user_id.unwrap_or(0)],
        )
        .map_err(|err| format!("clear gateway issues failed: {err}"))
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_mode TEXT NOT NULL CHECK (auth_mode IN ('api_key', 'account')),
                base_url TEXT,
                api_key TEXT,
                email TEXT,
                access_token TEXT,
                refresh_token TEXT,
                expiry_timestamp INTEGER,
                client_id TEXT,
                upstream_account_id TEXT,
                preferred_model TEXT,
                preferred_reasoning_effort TEXT,
                owner_user_id INTEGER
            );

            CREATE TABLE IF NOT EXISTS gateway_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                selected_provider_id TEXT,
                route_updated_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (selected_provider_id) REFERENCES providers(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS provider_model_cache (
                provider_id TEXT PRIMARY KEY,
                models_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS gateway_issues (
                id TEXT PRIMARY KEY,
                owner_user_id INTEGER NOT NULL DEFAULT 0,
                provider_id TEXT NOT NULL,
                provider_name TEXT NOT NULL,
                model TEXT NOT NULL,
                upstream_url TEXT NOT NULL,
                failure_kind TEXT NOT NULL,
                status_code INTEGER,
                error_message TEXT NOT NULL,
                upstream_response TEXT NOT NULL,
                upstream_response_truncated INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_issues_owner_created
                ON gateway_issues(owner_user_id, created_at DESC);
            ",
        )
        .map_err(|err| format!("initialize sqlite schema failed: {err}"))?;
        migrate_accounts_into_providers(&conn)?;
        add_column_if_missing(
            &conn,
            "gateway_state",
            "route_updated_at INTEGER NOT NULL DEFAULT 0",
        )?;
        drop_database_encryption_key(&conn)?;
        conn.execute("INSERT OR IGNORE INTO gateway_state (id) VALUES (1)", [])
            .map_err(|err| format!("initialize gateway state failed: {err}"))?;
        add_column_if_missing(&conn, "providers", "owner_user_id INTEGER")?;
        drop_provider_compatibility_profile(&conn)?;
        migrate_gateway_issue_payloads(&conn)?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_providers_owner_user_id ON providers(owner_user_id);",
        )
        .map_err(|err| format!("create provider ownership index failed: {err}"))?;
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

fn add_column_if_missing(conn: &Connection, table: &str, definition: &str) -> Result<(), String> {
    match conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {definition}"), []) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "add column `{definition}` to `{table}` failed: {error}"
        )),
    }
}

fn migrate_accounts_into_providers(conn: &Connection) -> Result<(), String> {
    if !table_exists(conn, "accounts")? {
        return Ok(());
    }

    if table_has_column(conn, "accounts", "upstream_account_id")?
        && !table_has_column(conn, "accounts", "account_id")?
    {
        conn.execute(
            "ALTER TABLE accounts RENAME COLUMN upstream_account_id TO account_id",
            [],
        )
        .map_err(|err| format!("rename legacy account id column failed: {err}"))?;
    }

    if !table_has_column(conn, "providers", "account_id")? {
        conn.execute("DROP TABLE accounts", [])
            .map_err(|err| format!("remove orphaned accounts table failed: {err}"))?;
        return Ok(());
    }

    for definition in [
        "email TEXT",
        "access_token TEXT",
        "refresh_token TEXT",
        "expiry_timestamp INTEGER",
        "client_id TEXT",
        "upstream_account_id TEXT",
        "owner_user_id INTEGER",
    ] {
        add_column_if_missing(conn, "providers", definition)?;
    }

    conn.execute(
        "UPDATE providers
         SET email = (SELECT email FROM accounts WHERE accounts.id = providers.account_id),
             access_token = (SELECT access_token FROM accounts WHERE accounts.id = providers.account_id),
             refresh_token = (SELECT refresh_token FROM accounts WHERE accounts.id = providers.account_id),
             expiry_timestamp = (SELECT expiry_timestamp FROM accounts WHERE accounts.id = providers.account_id),
             client_id = (SELECT client_id FROM accounts WHERE accounts.id = providers.account_id),
             upstream_account_id = (SELECT account_id FROM accounts WHERE accounts.id = providers.account_id)
         WHERE providers.account_id IS NOT NULL",
        [],
    )
    .map_err(|err| format!("copy account credentials into providers failed: {err}"))?;

    conn.execute_batch(
        "PRAGMA foreign_keys = OFF;
         BEGIN;
         CREATE TABLE providers_new (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            auth_mode TEXT NOT NULL CHECK (auth_mode IN ('api_key', 'account')),
            base_url TEXT,
            api_key TEXT,
            email TEXT,
            access_token TEXT,
            refresh_token TEXT,
            expiry_timestamp INTEGER,
            client_id TEXT,
            upstream_account_id TEXT,
            preferred_model TEXT,
            preferred_reasoning_effort TEXT,
            owner_user_id INTEGER
         );
         INSERT INTO providers_new (
            id, name, auth_mode, base_url, api_key, email, access_token,
            refresh_token, expiry_timestamp, client_id, upstream_account_id,
            preferred_model, preferred_reasoning_effort, owner_user_id
         )
         SELECT
            id, name, auth_mode, base_url, api_key, email, access_token,
            refresh_token, expiry_timestamp, client_id, upstream_account_id,
            preferred_model, preferred_reasoning_effort, owner_user_id
         FROM providers;
         DROP TABLE providers;
         ALTER TABLE providers_new RENAME TO providers;
         DROP TABLE accounts;
         COMMIT;
         PRAGMA foreign_keys = ON;",
    )
    .map_err(|err| format!("migrate accounts into providers failed: {err}"))
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(|err| format!("inspect table `{table}` failed: {err}"))
}

// Legacy databases may still contain the removed instance_id column. The
// migration reads old rows but does not carry that field into the new schema.
fn migrate_gateway_issue_payloads(conn: &Connection) -> Result<(), String> {
    if table_has_column(conn, "gateway_issues", "upstream_response")? {
        return Ok(());
    }

    conn.execute_batch(
        "BEGIN;
         CREATE TABLE gateway_issues_new (
            id TEXT PRIMARY KEY,
            owner_user_id INTEGER NOT NULL DEFAULT 0,
            provider_id TEXT NOT NULL,
            provider_name TEXT NOT NULL,
            model TEXT NOT NULL,
            upstream_url TEXT NOT NULL,
            failure_kind TEXT NOT NULL,
            status_code INTEGER,
            error_message TEXT NOT NULL,
            upstream_response TEXT NOT NULL,
            upstream_response_truncated INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
         );
         INSERT INTO gateway_issues_new (
            id, owner_user_id, provider_id, provider_name, model,
            upstream_url, failure_kind, status_code, error_message,
            upstream_response, upstream_response_truncated, created_at
         )
         SELECT
            id, owner_user_id, provider_id, provider_name, model,
            upstream_url, failure_kind, status_code, error_message,
            COALESCE(response_body, ''), response_truncated, created_at
         FROM gateway_issues;
         DROP TABLE gateway_issues;
         ALTER TABLE gateway_issues_new RENAME TO gateway_issues;
         CREATE INDEX IF NOT EXISTS idx_gateway_issues_owner_created
            ON gateway_issues(owner_user_id, created_at DESC);
         COMMIT;",
    )
    .map_err(|err| format!("migrate gateway issue payloads failed: {err}"))
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|err| format!("inspect `{table}` columns failed: {err}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("query `{table}` columns failed: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("read `{table}` columns failed: {err}"))?;
    Ok(columns.iter().any(|name| name == column))
}

fn drop_provider_compatibility_profile(conn: &Connection) -> Result<(), String> {
    if !table_has_column(conn, "providers", "compatibility_profile")? {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE providers DROP COLUMN compatibility_profile",
        [],
    )
    .map_err(|err| format!("remove provider compatibility profile failed: {err}"))?;
    Ok(())
}

fn drop_database_encryption_key(conn: &Connection) -> Result<(), String> {
    if !table_has_column(conn, "gateway_state", "database_encryption_key")? {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE gateway_state DROP COLUMN database_encryption_key",
        [],
    )
    .map_err(|err| format!("remove database encryption key failed: {err}"))?;
    Ok(())
}

fn gateway_issue_from_row(row: &rusqlite::Row<'_>) -> Result<GatewayIssue, rusqlite::Error> {
    Ok(GatewayIssue {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        provider_name: row.get(2)?,
        model: row.get(3)?,
        upstream_url: row.get(4)?,
        failure_kind: row.get(5)?,
        status_code: row.get(6)?,
        error_message: row.get(7)?,
        upstream_response: row.get(8)?,
        upstream_response_truncated: row.get::<_, i64>(9)? != 0,
        created_at: row.get(10)?,
    })
}

fn upsert_provider_record(conn: &Connection, provider: &ProviderRecord) -> Result<(), String> {
    let (base_url, api_key) = match provider.auth_mode {
        ProviderAuthMode::ApiKey => (
            Some(provider.base_url.as_str()),
            Some(provider.api_key.as_str()),
        ),
        ProviderAuthMode::Account => (None, None),
    };
    conn.execute(
        "INSERT INTO providers (
            id, name, auth_mode, base_url, api_key, email, access_token,
            refresh_token, expiry_timestamp, client_id, upstream_account_id,
            owner_user_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            auth_mode = excluded.auth_mode,
            base_url = excluded.base_url,
            api_key = excluded.api_key,
            email = excluded.email,
            access_token = excluded.access_token,
            refresh_token = excluded.refresh_token,
            expiry_timestamp = excluded.expiry_timestamp,
            client_id = excluded.client_id,
            upstream_account_id = excluded.upstream_account_id,
            owner_user_id = excluded.owner_user_id",
        params![
            provider.id,
            provider.name,
            provider_auth_mode_to_str(&provider.auth_mode),
            base_url,
            api_key,
            provider.email.as_deref(),
            provider.access_token.as_deref(),
            provider.refresh_token.as_deref(),
            provider.expiry_timestamp,
            provider.client_id.as_deref(),
            provider.upstream_account_id.as_deref(),
            provider.owner_user_id
        ],
    )
    .map_err(|err| format!("upsert provider failed: {err}"))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::SqliteStore;
    use crate::models::{CachedProviderModels, ProviderAuthMode, ProviderRecord, SelectedRoute};
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
                selected_reasoning_effort: Some("high".to_string()),
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
                selected_reasoning_effort: None,
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
        let provider = ProviderRecord {
            id: "provider-account".to_string(),
            name: "account".to_string(),
            auth_mode: ProviderAuthMode::Account,
            base_url: String::new(),
            api_key: String::new(),
            email: Some("account@example.com".to_string()),
            access_token: Some("access".to_string()),
            refresh_token: Some("refresh".to_string()),
            expiry_timestamp: Some(1),
            client_id: Some("client".to_string()),
            upstream_account_id: Some("upstream".to_string()),
            owner_user_id: None,
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
    fn stores_credentials_as_plaintext() {
        let db_path = unique_test_db_path("plaintext-credentials");
        let store = SqliteStore::for_test(db_path.clone()).expect("create database");
        let account_provider = ProviderRecord {
            id: "provider-account".to_string(),
            name: "account".to_string(),
            auth_mode: ProviderAuthMode::Account,
            base_url: String::new(),
            api_key: String::new(),
            email: Some("account@example.com".to_string()),
            access_token: Some("access-secret".to_string()),
            refresh_token: Some("refresh-secret".to_string()),
            expiry_timestamp: Some(1),
            client_id: None,
            upstream_account_id: None,
            owner_user_id: None,
        };
        store
            .upsert_provider(&account_provider)
            .expect("save account provider");
        store
            .upsert_provider(&api_provider("provider-1"))
            .expect("save provider");

        let conn = Connection::open(&db_path).expect("open database");
        let (access_token, refresh_token): (String, String) = conn
            .query_row(
                "SELECT access_token, refresh_token FROM providers WHERE id = ?1",
                ["provider-account"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read account provider credentials");
        let api_key: String = conn
            .query_row(
                "SELECT api_key FROM providers WHERE id = ?1",
                ["provider-1"],
                |row| row.get(0),
            )
            .expect("read provider credential");

        for (stored, plaintext) in [
            (access_token, "access-secret".to_string()),
            (refresh_token, "refresh-secret".to_string()),
            (api_key, "sk-test".to_string()),
        ] {
            assert_eq!(stored, plaintext);
        }
        let loaded_account_provider = store
            .load_providers()
            .unwrap()
            .into_iter()
            .find(|provider| provider.id == "provider-account")
            .unwrap();
        assert_eq!(
            loaded_account_provider.access_token.as_deref(),
            Some("access-secret")
        );
        assert_eq!(
            loaded_account_provider.refresh_token.as_deref(),
            Some("refresh-secret")
        );
        assert_eq!(
            store
                .load_providers()
                .unwrap()
                .into_iter()
                .find(|provider| provider.id == "provider-1")
                .unwrap()
                .api_key,
            "sk-test"
        );

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn migrates_legacy_accounts_into_provider_records() {
        let db_path = unique_test_db_path("accounts-to-providers");
        let conn = Connection::open(&db_path).expect("create legacy database");
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                access_token TEXT NOT NULL,
                refresh_token TEXT NOT NULL,
                expiry_timestamp INTEGER NOT NULL,
                client_id TEXT,
                upstream_account_id TEXT
             );
             CREATE TABLE providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_mode TEXT NOT NULL CHECK (auth_mode IN ('api_key', 'account')),
                base_url TEXT,
                api_key TEXT,
                account_id TEXT,
                preferred_model TEXT,
                preferred_reasoning_effort TEXT,
                owner_user_id INTEGER,
                FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
             );
             INSERT INTO accounts (
                id, email, access_token, refresh_token, expiry_timestamp,
                client_id, upstream_account_id
             ) VALUES (
                'account-1', 'user@example.com', 'access-secret', 'refresh-secret',
                1700000000, 'client-1', 'upstream-1'
             );
             INSERT INTO providers (
                id, name, auth_mode, base_url, api_key, account_id,
                preferred_model, preferred_reasoning_effort, owner_user_id
             ) VALUES (
                'provider-1', 'GPT账户', 'account', NULL, NULL, 'account-1',
                'gpt-5', 'high', NULL
             );",
        )
        .expect("create legacy provider schema");
        drop(conn);

        let store = SqliteStore {
            db_path: db_path.clone(),
        };
        store.init().expect("migrate legacy provider schema");

        let conn = Connection::open(&db_path).expect("open migrated database");
        let accounts_exist: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'accounts')",
                [],
                |row| row.get(0),
            )
            .expect("check accounts table");
        assert!(!accounts_exist);

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(providers)")
            .expect("prepare provider column query")
            .query_map([], |row| row.get(1))
            .expect("query provider columns")
            .collect::<Result<_, _>>()
            .expect("read provider columns");
        assert!(!columns.iter().any(|column| column == "account_id"));

        let provider = store
            .load_providers()
            .expect("load migrated providers")
            .into_iter()
            .find(|provider| provider.id == "provider-1")
            .expect("migrated provider exists");
        assert_eq!(provider.email.as_deref(), Some("user@example.com"));
        assert_eq!(provider.access_token.as_deref(), Some("access-secret"));
        assert_eq!(provider.refresh_token.as_deref(), Some("refresh-secret"));
        assert_eq!(provider.expiry_timestamp, Some(1700000000));
        assert_eq!(provider.client_id.as_deref(), Some("client-1"));
        assert_eq!(provider.upstream_account_id.as_deref(), Some("upstream-1"));

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn migrates_gateway_issues_to_drop_request_payloads() {
        let db_path = unique_test_db_path("gateway-issue-payloads");
        let conn = Connection::open(&db_path).expect("create legacy database");
        conn.execute_batch(
            "CREATE TABLE gateway_issues (
                id TEXT PRIMARY KEY,
                owner_user_id INTEGER NOT NULL DEFAULT 0,
                instance_id TEXT,
                provider_id TEXT NOT NULL,
                provider_name TEXT NOT NULL,
                model TEXT NOT NULL,
                upstream_url TEXT NOT NULL,
                failure_kind TEXT NOT NULL,
                status_code INTEGER,
                error_message TEXT NOT NULL,
                request_body TEXT NOT NULL,
                response_body TEXT,
                request_truncated INTEGER NOT NULL DEFAULT 0,
                response_truncated INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
             );
             INSERT INTO gateway_issues (
                id, owner_user_id, instance_id, provider_id, provider_name, model,
                upstream_url, failure_kind, status_code, error_message,
                request_body, response_body, request_truncated, response_truncated, created_at
             ) VALUES (
                'legacy', 0, NULL, 'provider', 'Provider', 'model',
                'https://example.com/v1/responses', 'upstream_http_error', 500, 'failed',
                '{\"input\":\"secret\"}', '{\"error\":\"failed\"}', 0, 1, 1
             );",
        )
        .expect("create legacy gateway issues");
        drop(conn);

        let store = SqliteStore {
            db_path: db_path.clone(),
        };
        store.init().expect("migrate legacy database");

        let conn = Connection::open(&db_path).expect("open migrated database");
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(gateway_issues)")
            .expect("prepare column query")
            .query_map([], |row| row.get(1))
            .expect("query columns")
            .collect::<Result<_, _>>()
            .expect("read columns");
        assert!(!columns.iter().any(|column| column == "request_body"));
        assert!(!columns.iter().any(|column| column == "response_body"));
        assert!(columns.iter().any(|column| column == "upstream_response"));

        let issue = store
            .load_gateway_issue(None, "legacy")
            .expect("load migrated issue")
            .expect("migrated issue exists");
        assert_eq!(issue.upstream_response, "{\"error\":\"failed\"}");
        assert!(issue.upstream_response_truncated);

        let _ = fs::remove_file(db_path);
    }

    fn api_provider(id: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.to_string(),
            name: id.to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: "https://example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            email: None,
            access_token: None,
            refresh_token: None,
            expiry_timestamp: None,
            client_id: None,
            upstream_account_id: None,
            owner_user_id: None,
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
