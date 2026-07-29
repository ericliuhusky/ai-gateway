use crate::{
    config::Config,
    models::{
        AccountRecord, AccountType, ApiProviderBillingMode, ApiProviderRecord,
        CachedProviderModels, ProviderAuthMode, ProviderCompatibilityProfile,
        ProviderUpstreamProtocol, SelectedRoute,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{fs, path::PathBuf, sync::Arc};
use url::Url;

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

    pub fn load_accounts(&self) -> Result<Vec<AccountRecord>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, account_type, email, access_token, refresh_token, expiry_timestamp, client_id, upstream_account_id
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
                    upstream_account_id: row.get(7)?,
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
                "SELECT id, name, auth_mode, base_url, api_key, account_id, upstream_protocol, compatibility_profile, billing_mode
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
        upsert_provider_record(&conn, provider)
    }

    pub fn delete_provider(&self, provider_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute("DELETE FROM providers WHERE id = ?1", params![provider_id])
            .map_err(|err| format!("delete provider failed: {err}"))?;
        conn.execute(
            "DELETE FROM provider_models WHERE provider_id = ?1",
            params![provider_id],
        )
        .map_err(|err| format!("delete cached provider models failed: {err}"))?;
        conn.execute(
            "DELETE FROM provider_selected_models WHERE provider_id = ?1",
            params![provider_id],
        )
        .map_err(|err| format!("delete provider selected model failed: {err}"))?;
        Ok(())
    }

    pub fn load_route(&self) -> Result<SelectedRoute, String> {
        let conn = self.connect()?;
        let mut route = conn
            .query_row(
                "SELECT provider_id, updated_at FROM selected_provider WHERE id = 1",
                [],
                |row| {
                    Ok(SelectedRoute {
                        provider_id: row.get(0)?,
                        selected_model: None,
                        updated_at: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(|err| format!("load route failed: {err}"))?
            .unwrap_or_default();

        if let Some(provider_id) = route.provider_id.as_deref() {
            route.selected_model = load_provider_selected_model_record(&conn, provider_id)?;
        }

        Ok(route)
    }

    pub fn upsert_route(&self, route: &SelectedRoute) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_route_record(&conn, route)?;
        if let Some(provider_id) = route.provider_id.as_deref() {
            set_provider_selected_model_record(
                &conn,
                provider_id,
                route.selected_model.as_deref(),
                route.updated_at,
            )?;
        }
        Ok(())
    }

    pub fn load_provider_selected_model(
        &self,
        provider_id: &str,
    ) -> Result<Option<String>, String> {
        let conn = self.connect()?;
        load_provider_selected_model_record(&conn, provider_id)
    }

    pub fn load_cached_models(
        &self,
        provider_id: &str,
    ) -> Result<Option<CachedProviderModels>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT provider_id, models_json, updated_at FROM provider_models WHERE provider_id = ?1",
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
            "INSERT INTO provider_models (provider_id, models_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(provider_id) DO UPDATE SET
                models_json = excluded.models_json,
                updated_at = excluded.updated_at",
            params![models.provider_id, models.models_json, models.updated_at],
        )
        .map_err(|err| format!("upsert cached provider models failed: {err}"))?;
        Ok(())
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
                upstream_account_id TEXT
            );

            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_mode TEXT NOT NULL,
                base_url TEXT NOT NULL,
                api_key TEXT NOT NULL,
                account_id TEXT,
                uses_chat_completions INTEGER NOT NULL DEFAULT 0,
                upstream_protocol TEXT NOT NULL,
                compatibility_profile TEXT NOT NULL,
                billing_mode TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS selected_provider (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                provider_id TEXT,
                selected_model TEXT,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS provider_selected_models (
                provider_id TEXT PRIMARY KEY,
                selected_model TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS provider_models (
                provider_id TEXT PRIMARY KEY,
                models_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            ",
        )
        .map_err(|err| format!("initialize sqlite schema failed: {err}"))?;
        migrate_provider_schema(&conn)?;

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
            account.access_token,
            account.refresh_token,
            account.expiry_timestamp,
            account.client_id,
            account.upstream_account_id
        ],
    )
    .map_err(|err| format!("upsert account failed: {err}"))?;
    Ok(())
}

fn upsert_provider_record(conn: &Connection, provider: &ApiProviderRecord) -> Result<(), String> {
    conn.execute(
        "INSERT INTO providers (
            id, name, auth_mode, base_url, api_key, account_id, uses_chat_completions,
            upstream_protocol, compatibility_profile, billing_mode
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            auth_mode = excluded.auth_mode,
            base_url = excluded.base_url,
            api_key = excluded.api_key,
            account_id = excluded.account_id,
            uses_chat_completions = excluded.uses_chat_completions,
            upstream_protocol = excluded.upstream_protocol,
            compatibility_profile = excluded.compatibility_profile,
            billing_mode = excluded.billing_mode",
        params![
            provider.id,
            provider.name,
            provider_auth_mode_to_str(&provider.auth_mode),
            provider.base_url,
            provider.api_key,
            provider.account_id,
            if provider.uses_chat_completions() {
                1
            } else {
                0
            },
            upstream_protocol_to_str(&provider.upstream_protocol),
            compatibility_profile_to_str(&provider.compatibility_profile),
            billing_mode_to_str(&provider.billing_mode)
        ],
    )
    .map_err(|err| format!("upsert provider failed: {err}"))?;
    Ok(())
}

fn migrate_provider_schema(conn: &Connection) -> Result<(), String> {
    ensure_column(conn, "providers", "upstream_protocol", "TEXT")?;
    ensure_column(conn, "providers", "compatibility_profile", "TEXT")?;

    conn.execute(
        "UPDATE providers
         SET upstream_protocol = CASE
            WHEN uses_chat_completions = 1 THEN 'openai_chat_completions'
            ELSE 'openai_responses'
         END
         WHERE upstream_protocol IS NULL OR trim(upstream_protocol) = ''",
        [],
    )
    .map_err(|err| format!("migrate provider upstream protocol failed: {err}"))?;
    conn.execute(
        "UPDATE providers
         SET upstream_protocol = CASE upstream_protocol
            WHEN 'open_ai_chat_completions' THEN 'openai_chat_completions'
            WHEN 'open_ai_responses' THEN 'openai_responses'
            ELSE upstream_protocol
         END",
        [],
    )
    .map_err(|err| format!("normalize provider upstream protocol failed: {err}"))?;

    conn.execute(
        "UPDATE providers
         SET compatibility_profile = CASE compatibility_profile
            WHEN 'official_open_ai' THEN 'official_openai'
            WHEN 'generic_open_ai' THEN 'generic_openai'
            WHEN 'open_ai_codex' THEN 'openai_codex'
            ELSE compatibility_profile
         END",
        [],
    )
    .map_err(|err| format!("normalize provider compatibility profile failed: {err}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, auth_mode, base_url
             FROM providers
             WHERE compatibility_profile IS NULL OR trim(compatibility_profile) = ''",
        )
        .map_err(|err| format!("prepare provider profile migration failed: {err}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|err| format!("query provider profile migration failed: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("read provider profile migration failed: {err}"))?;

    for (id, auth_mode, base_url) in rows {
        let profile = if auth_mode == "account" {
            ProviderCompatibilityProfile::OpenAiCodex
        } else if is_official_openai_base_url(&base_url) {
            ProviderCompatibilityProfile::OfficialOpenAi
        } else {
            ProviderCompatibilityProfile::GenericOpenAi
        };
        conn.execute(
            "UPDATE providers SET compatibility_profile = ?1 WHERE id = ?2",
            params![compatibility_profile_to_str(&profile), id],
        )
        .map_err(|err| format!("migrate provider compatibility profile failed: {err}"))?;
    }

    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|err| format!("prepare {table} schema query failed: {err}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("query {table} schema failed: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("read {table} schema failed: {err}"))?;
    if columns.iter().any(|existing| existing == column) {
        return Ok(false);
    }

    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )
    .map_err(|err| format!("add {table}.{column} failed: {err}"))?;
    Ok(true)
}

fn is_official_openai_base_url(base_url: &str) -> bool {
    Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"))
}

fn upsert_route_record(conn: &Connection, route: &SelectedRoute) -> Result<(), String> {
    conn.execute(
        "INSERT INTO selected_provider (id, provider_id, selected_model, updated_at)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
            provider_id = excluded.provider_id,
            selected_model = excluded.selected_model,
            updated_at = excluded.updated_at",
        params![route.provider_id, Option::<String>::None, route.updated_at],
    )
    .map_err(|err| format!("upsert route failed: {err}"))?;
    Ok(())
}

fn load_provider_selected_model_record(
    conn: &Connection,
    provider_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT selected_model FROM provider_selected_models WHERE provider_id = ?1",
        params![provider_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|err| format!("load provider selected model failed: {err}"))
}

fn set_provider_selected_model_record(
    conn: &Connection,
    provider_id: &str,
    selected_model: Option<&str>,
    updated_at: i64,
) -> Result<(), String> {
    if let Some(selected_model) = selected_model {
        conn.execute(
            "INSERT INTO provider_selected_models (provider_id, selected_model, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(provider_id) DO UPDATE SET
                selected_model = excluded.selected_model,
                updated_at = excluded.updated_at",
            params![provider_id, selected_model, updated_at],
        )
        .map_err(|err| format!("upsert provider selected model failed: {err}"))?;
    } else {
        conn.execute(
            "DELETE FROM provider_selected_models WHERE provider_id = ?1",
            params![provider_id],
        )
        .map_err(|err| format!("delete provider selected model failed: {err}"))?;
    }

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

#[cfg(test)]
mod tests {
    use super::SqliteStore;
    use crate::models::{ProviderCompatibilityProfile, ProviderUpstreamProtocol};
    use rusqlite::Connection;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn migrates_legacy_provider_protocol_and_profile_columns() {
        let db_path = unique_test_db_path("legacy-provider-profile");
        let conn = Connection::open(&db_path).expect("create legacy database");
        conn.execute_batch(
            "
            CREATE TABLE providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_mode TEXT NOT NULL,
                base_url TEXT NOT NULL,
                api_key TEXT NOT NULL,
                account_id TEXT,
                uses_chat_completions INTEGER NOT NULL DEFAULT 0,
                billing_mode TEXT NOT NULL
            );
            INSERT INTO providers (
                id, name, auth_mode, base_url, api_key, account_id,
                uses_chat_completions, billing_mode
            ) VALUES
                ('official', 'official', 'api_key', 'https://api.openai.com/v1', 'sk-a', NULL, 0, 'metered'),
                ('chat', 'chat', 'api_key', 'https://example.com/v1', 'sk-b', NULL, 1, 'metered'),
                ('account', 'account', 'account', '', '', 'account-1', 0, 'subscription');
            ",
        )
        .expect("write legacy provider rows");
        drop(conn);

        let store = SqliteStore::for_test(db_path.clone()).expect("migrate legacy database");
        let providers = store.load_providers().expect("load migrated providers");

        assert_eq!(
            providers[0].compatibility_profile,
            ProviderCompatibilityProfile::OfficialOpenAi
        );
        assert_eq!(
            providers[0].upstream_protocol,
            ProviderUpstreamProtocol::OpenAiResponses
        );
        assert_eq!(
            providers[1].compatibility_profile,
            ProviderCompatibilityProfile::GenericOpenAi
        );
        assert_eq!(
            providers[1].upstream_protocol,
            ProviderUpstreamProtocol::OpenAiChatCompletions
        );
        assert_eq!(
            providers[2].compatibility_profile,
            ProviderCompatibilityProfile::OpenAiCodex
        );

        let _ = fs::remove_file(db_path);
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
