use crate::{
    config::Config,
    domain::{ProviderAuthMode, SelectedRoute},
    store::ProviderRecord,
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
        let mut stmt = conn
            .prepare(
                "SELECT id, name, auth_mode, base_url, api_key,
                        access_token, refresh_token, expiry_timestamp, client_id
                 FROM providers
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare providers query failed: {err}"))?;
        let rows = stmt
            .query_map([], move |row| {
                let auth_mode = provider_auth_mode_from_str(&row.get::<_, String>(2)?)
                    .map_err(rusqlite::Error::ToSqlConversionFailure)?;
                Ok(ProviderRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    auth_mode,
                    base_url: row.get(3)?,
                    api_key: row.get(4)?,
                    access_token: row.get(5)?,
                    refresh_token: row.get(6)?,
                    expiry_timestamp: row.get(7)?,
                    client_id: row.get(8)?,
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
                    provider.preferred_reasoning_effort
             FROM gateway_state AS state
             LEFT JOIN providers AS provider ON provider.id = state.selected_provider_id
             WHERE state.id = 1",
            [],
            |row| {
                Ok(SelectedRoute {
                    provider_id: row.get(0)?,
                    model: row.get(1)?,
                    reasoning_effort: row.get(2)?,
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
            "INSERT INTO gateway_state (id, selected_provider_id)
             VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET
                selected_provider_id = excluded.selected_provider_id",
            params![route.provider_id],
        )
        .map_err(|err| format!("upsert route failed: {err}"))?;

        if let Some(provider_id) = route.provider_id.as_deref() {
            tx.execute(
                "UPDATE providers
                 SET preferred_model = ?1, preferred_reasoning_effort = ?2
                 WHERE id = ?3",
                params![route.model, route.reasoning_effort, provider_id],
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
                access_token TEXT,
                refresh_token TEXT,
                expiry_timestamp INTEGER,
                client_id TEXT,
                preferred_model TEXT,
                preferred_reasoning_effort TEXT
            );

            CREATE TABLE IF NOT EXISTS gateway_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                selected_provider_id TEXT,
                FOREIGN KEY (selected_provider_id) REFERENCES providers(id) ON DELETE SET NULL
            );

            ",
        )
        .map_err(|err| format!("initialize sqlite schema failed: {err}"))?;
        conn.execute("INSERT OR IGNORE INTO gateway_state (id) VALUES (1)", [])
            .map_err(|err| format!("initialize gateway state failed: {err}"))?;
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

fn upsert_provider_record(conn: &Connection, provider: &ProviderRecord) -> Result<(), String> {
    let (base_url, api_key, access_token, refresh_token, expiry_timestamp, client_id) =
        match &provider.auth_mode {
            ProviderAuthMode::ApiKey => (
                provider.base_url.as_deref(),
                provider.api_key.as_deref(),
                None,
                None,
                None,
                None,
            ),
            ProviderAuthMode::Account => (
                None,
                None,
                provider.access_token.as_deref(),
                provider.refresh_token.as_deref(),
                provider.expiry_timestamp,
                provider.client_id.as_deref(),
            ),
        };
    let auth_mode = provider_auth_mode_to_str(&provider.auth_mode);
    conn.execute(
        "INSERT INTO providers (
            id, name, auth_mode, base_url, api_key, access_token,
            refresh_token, expiry_timestamp, client_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            auth_mode = excluded.auth_mode,
            base_url = excluded.base_url,
            api_key = excluded.api_key,
            access_token = excluded.access_token,
            refresh_token = excluded.refresh_token,
            expiry_timestamp = excluded.expiry_timestamp,
            client_id = excluded.client_id",
        params![
            provider.id,
            provider.name,
            auth_mode,
            base_url,
            api_key,
            access_token,
            refresh_token,
            expiry_timestamp,
            client_id
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
    use crate::{
        domain::{ProviderAuthMode, SelectedRoute},
        store::ProviderRecord,
    };
    use rusqlite::Connection;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn creates_only_the_current_schema() {
        let db_path = unique_test_db_path("schema");
        SqliteStore::for_test(db_path.clone()).expect("create database");
        let conn = Connection::open(&db_path).expect("open database");
        let tables = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(tables, ["gateway_state", "providers"]);

        let columns = conn
            .prepare("PRAGMA table_info(providers)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            columns,
            [
                "id",
                "name",
                "auth_mode",
                "base_url",
                "api_key",
                "access_token",
                "refresh_token",
                "expiry_timestamp",
                "client_id",
                "preferred_model",
                "preferred_reasoning_effort",
            ]
        );
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn deleting_provider_clears_active_route() {
        let db_path = unique_test_db_path("provider-cascade");
        let store = SqliteStore::for_test(db_path.clone()).expect("create compact database");
        let provider = api_provider("provider-a");
        store.upsert_provider(&provider).expect("save provider");
        store
            .upsert_route(&SelectedRoute {
                provider_id: Some(provider.id.clone()),
                model: Some("model-a".to_string()),
                reasoning_effort: Some("high".to_string()),
            })
            .expect("save route");

        store
            .delete_provider(&provider.id)
            .expect("delete provider");

        assert_eq!(
            store.load_route().unwrap(),
            SelectedRoute {
                provider_id: None,
                model: None,
                reasoning_effort: None,
            }
        );
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn account_provider_uses_null_transport_credentials() {
        let db_path = unique_test_db_path("account-provider-null-credentials");
        let store = SqliteStore::for_test(db_path.clone()).expect("create compact database");
        let provider = ProviderRecord {
            id: "provider-account".to_string(),
            name: "account@example.com".to_string(),
            auth_mode: ProviderAuthMode::Account,
            base_url: None,
            api_key: None,
            access_token: Some("access".to_string()),
            refresh_token: Some("refresh".to_string()),
            expiry_timestamp: Some(1),
            client_id: Some("client".to_string()),
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
            name: "account@example.com".to_string(),
            auth_mode: ProviderAuthMode::Account,
            base_url: None,
            api_key: None,
            access_token: Some("access-secret".to_string()),
            refresh_token: Some("refresh-secret".to_string()),
            expiry_timestamp: Some(1),
            client_id: None,
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
            Some("sk-test".to_string())
        );

        let _ = fs::remove_file(db_path);
    }

    fn api_provider(id: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.to_string(),
            name: id.to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: Some("https://example.com/v1".to_string()),
            api_key: Some("sk-test".to_string()),
            access_token: None,
            refresh_token: None,
            expiry_timestamp: None,
            client_id: None,
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
