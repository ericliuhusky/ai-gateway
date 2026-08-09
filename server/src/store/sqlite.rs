use crate::{
    config::Config,
    crypto::FieldEncryptor,
    models::{
        ApiProviderRecord, ProviderAuthMode, ProviderCompatibilityProfile, ProviderUpstreamProtocol,
    },
};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use std::{fs, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct SqliteStore {
    db_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct DatabaseSecuritySettings {
    pub encryption_key: String,
    pub feishu_app_id: String,
    pub feishu_app_secret: String,
}

impl SqliteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        fs::create_dir_all(config.data_dir())
            .map_err(|error| format!("create data dir failed: {error}"))?;
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
                .map_err(|error| format!("create test data dir failed: {error}"))?;
        }
        let store = Self { db_path };
        store.init()?;
        store.set_database_encryption_key("MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=")?;
        Ok(store)
    }

    pub fn load_providers(&self) -> Result<Vec<ApiProviderRecord>, String> {
        let conn = self.connect()?;
        let encryption = self.encryption()?;
        let mut statement = conn
            .prepare(
                "SELECT id, name, COALESCE(base_url, ''), COALESCE(api_key, ''),
                        upstream_protocol, compatibility_profile, owner_user_id
                 FROM providers
                 WHERE auth_mode = 'api_key'
                   AND compatibility_profile IN ('official_openai', 'generic_openai')
                   AND owner_user_id IS NOT NULL
                 ORDER BY rowid",
            )
            .map_err(|error| format!("prepare providers query failed: {error}"))?;
        let rows = statement
            .query_map([], move |row| {
                let encrypted_key = row.get::<_, String>(3)?;
                Ok(ApiProviderRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    auth_mode: ProviderAuthMode::ApiKey,
                    base_url: row.get(2)?,
                    api_key: if encrypted_key.is_empty() {
                        String::new()
                    } else {
                        encryption
                            .decrypt(&encrypted_key)
                            .map_err(decrypt_conversion_error)?
                    },
                    upstream_protocol: upstream_protocol_from_str(&row.get::<_, String>(4)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    compatibility_profile: compatibility_profile_from_str(
                        &row.get::<_, String>(5)?,
                    )
                    .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    owner_user_id: row.get(6)?,
                })
            })
            .map_err(|error| format!("query providers failed: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read providers failed: {error}"))
    }

    pub fn upsert_provider(&self, provider: &ApiProviderRecord) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO providers (
                id, name, auth_mode, base_url, api_key,
                upstream_protocol, compatibility_profile, owner_user_id
             ) VALUES (?1, ?2, 'api_key', ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                auth_mode = 'api_key',
                base_url = excluded.base_url,
                api_key = excluded.api_key,
                upstream_protocol = excluded.upstream_protocol,
                compatibility_profile = excluded.compatibility_profile,
                owner_user_id = excluded.owner_user_id",
            params![
                provider.id,
                provider.name,
                provider.base_url,
                self.encryption()?.encrypt(&provider.api_key)?,
                upstream_protocol_to_str(&provider.upstream_protocol),
                compatibility_profile_to_str(&provider.compatibility_profile),
                provider.owner_user_id,
            ],
        )
        .map_err(|error| format!("upsert provider failed: {error}"))?;
        Ok(())
    }

    pub fn delete_provider(&self, provider_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute("DELETE FROM providers WHERE id = ?1", params![provider_id])
            .map_err(|error| format!("delete provider failed: {error}"))?;
        Ok(())
    }

    pub fn shared_provider_ids_for_user(
        &self,
        user_id: i64,
    ) -> Result<std::collections::HashSet<String>, String> {
        let conn = self.connect()?;
        let mut statement = conn
            .prepare(
                "SELECT DISTINCT gp.provider_id
                 FROM gateway_group_providers AS gp
                 JOIN gateway_group_members AS gm ON gm.group_id = gp.group_id
                 JOIN providers AS p ON p.id = gp.provider_id
                 WHERE gm.user_id = ?1
                   AND p.auth_mode = 'api_key'
                   AND p.compatibility_profile IN ('official_openai', 'generic_openai')",
            )
            .map_err(|error| format!("prepare shared provider query failed: {error}"))?;
        let rows = statement
            .query_map(params![user_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query shared providers failed: {error}"))?;
        rows.collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(|error| format!("read shared providers failed: {error}"))
    }

    pub(crate) fn initialize_user_auth_schema(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(AUTH_SCHEMA)
            .map_err(|error| format!("initialize user auth schema failed: {error}"))?;
        add_column_if_missing(
            &conn,
            "gateway_users",
            "role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user'))",
        )?;
        conn.execute(
            "UPDATE gateway_users SET role = 'user' WHERE role NOT IN ('admin', 'user')",
            [],
        )
        .map_err(|error| format!("normalize gateway user roles failed: {error}"))?;
        conn.execute(
            "UPDATE gateway_users
             SET role = 'admin'
             WHERE id = (SELECT id FROM gateway_users ORDER BY id LIMIT 1)
               AND NOT EXISTS (SELECT 1 FROM gateway_users WHERE role = 'admin')",
            [],
        )
        .map_err(|error| format!("initialize first admin failed: {error}"))?;
        add_column_if_missing(&conn, "gateway_access_tokens", "token_ciphertext TEXT")?;
        conn.execute(
            "DELETE FROM gateway_access_tokens
             WHERE token_ciphertext IS NULL
                OR rowid NOT IN (
                    SELECT MAX(rowid) FROM gateway_access_tokens GROUP BY user_id
                )",
            [],
        )
        .map_err(|error| format!("migrate gateway access tokens failed: {error}"))?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_gateway_access_tokens_user_id_unique
             ON gateway_access_tokens(user_id)",
            [],
        )
        .map_err(|error| format!("create gateway token owner index failed: {error}"))?;
        Ok(())
    }

    pub fn initialize_group_schema(&self) -> Result<(), String> {
        self.connect()?
            .execute_batch(GROUP_SCHEMA)
            .map_err(|error| format!("initialize group schema failed: {error}"))
    }

    pub(crate) fn connect_for_auth(&self) -> Result<Connection, String> {
        self.connect()
    }

    pub(crate) fn database_security_settings(&self) -> Result<DatabaseSecuritySettings, String> {
        let conn = self.connect()?;
        database_security_settings_from_connection(&conn)
    }

    #[cfg(test)]
    pub(crate) fn set_database_encryption_key(&self, key: &str) -> Result<(), String> {
        let key = key.trim();
        let new_encryptor = FieldEncryptor::from_base64_key(key)?;
        let mut conn = self.connect()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("begin encryption key transaction failed: {error}"))?;
        let current = database_security_settings_from(&transaction)?;
        rotate_database_encryption_key(&transaction, &current.encryption_key, key, &new_encryptor)?;
        transaction
            .commit()
            .map_err(|error| format!("commit encryption key transaction failed: {error}"))
    }

    pub(crate) fn update_database_security_settings(
        &self,
        encryption_key: Option<&str>,
        feishu_app_id: &str,
        feishu_app_secret: Option<&str>,
    ) -> Result<(), String> {
        let mut conn = self.connect()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("begin security settings transaction failed: {error}"))?;
        let current = database_security_settings_from(&transaction)?;
        let requested_key = encryption_key
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let effective_key = requested_key.unwrap_or(&current.encryption_key);
        let encryptor = FieldEncryptor::from_base64_key(effective_key)?;
        if let Some(new_key) = requested_key
            && new_key != current.encryption_key
        {
            rotate_database_encryption_key(
                &transaction,
                &current.encryption_key,
                new_key,
                &encryptor,
            )?;
        }
        let encrypted_secret = match feishu_app_secret
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(secret) => encryptor.encrypt(secret)?,
            None => transaction
                .query_row(
                    "SELECT COALESCE(feishu_app_secret, '') FROM gateway_state WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| format!("load Feishu secret failed: {error}"))?,
        };
        transaction
            .execute(
                "UPDATE gateway_state
                 SET database_encryption_key = ?1, feishu_app_id = ?2,
                     feishu_app_secret = ?3
                 WHERE id = 1",
                params![effective_key, feishu_app_id.trim(), encrypted_secret],
            )
            .map_err(|error| format!("save security settings failed: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit security settings failed: {error}"))
    }

    pub(crate) fn encryption(&self) -> Result<FieldEncryptor, String> {
        let key = self.database_security_settings()?.encryption_key;
        FieldEncryptor::from_base64_key(&key)
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(CONTROL_SCHEMA)
            .map_err(|error| format!("initialize control database failed: {error}"))?;
        self.initialize_user_auth_schema()?;
        self.initialize_group_schema()?;

        add_column_if_missing(
            &conn,
            "gateway_state",
            "database_encryption_key TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(
            &conn,
            "gateway_state",
            "feishu_app_id TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(
            &conn,
            "gateway_state",
            "feishu_app_secret TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(
            &conn,
            "providers",
            "owner_user_id INTEGER REFERENCES gateway_users(id) ON DELETE CASCADE",
        )?;
        let inserted = conn
            .execute("INSERT OR IGNORE INTO gateway_state (id) VALUES (1)", [])
            .map_err(|error| format!("initialize gateway state failed: {error}"))?;
        let current_key: String = conn
            .query_row(
                "SELECT COALESCE(database_encryption_key, '') FROM gateway_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("load database encryption key failed: {error}"))?;
        if inserted == 1 || current_key.is_empty() {
            let key = FieldEncryptor::generate_base64_key()?;
            conn.execute(
                "UPDATE gateway_state SET database_encryption_key = ?1 WHERE id = 1",
                params![key],
            )
            .map_err(|error| format!("initialize database encryption key failed: {error}"))?;
        }
        Ok(())
    }

    fn connect(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.db_path)
            .map_err(|error| format!("open database {} failed: {error}", self.db_path.display()))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;",
        )
        .map_err(|error| format!("configure sqlite connection failed: {error}"))?;
        Ok(conn)
    }
}

const CONTROL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS gateway_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    database_encryption_key TEXT NOT NULL DEFAULT '',
    feishu_app_id TEXT NOT NULL DEFAULT '',
    feishu_app_secret TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    auth_mode TEXT NOT NULL CHECK (auth_mode = 'api_key'),
    base_url TEXT,
    api_key TEXT,
    upstream_protocol TEXT NOT NULL DEFAULT 'openai_responses',
    compatibility_profile TEXT NOT NULL DEFAULT 'generic_openai',
    owner_user_id INTEGER REFERENCES gateway_users(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_providers_owner_user_id ON providers(owner_user_id);
"#;

const AUTH_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS gateway_users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE COLLATE NOCASE,
    name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user'))
);
CREATE TABLE IF NOT EXISTS gateway_sessions (
    session_id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS gateway_feishu_identities (
    user_id INTEGER PRIMARY KEY REFERENCES gateway_users(id) ON DELETE CASCADE,
    tenant_key TEXT NOT NULL,
    open_id TEXT NOT NULL,
    name TEXT NOT NULL,
    avatar_url TEXT NOT NULL DEFAULT '',
    UNIQUE (tenant_key, open_id)
);
CREATE TABLE IF NOT EXISTS gateway_access_tokens (
    token_hash TEXT PRIMARY KEY,
    token_ciphertext TEXT,
    user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_gateway_sessions_user_id ON gateway_sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_gateway_sessions_expires_at ON gateway_sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_gateway_access_tokens_user_id ON gateway_access_tokens(user_id);
"#;

const GROUP_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS gateway_groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    owner_user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS gateway_group_members (
    group_id INTEGER NOT NULL REFERENCES gateway_groups(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'member')),
    joined_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (group_id, user_id)
);
CREATE TABLE IF NOT EXISTS gateway_group_providers (
    group_id INTEGER NOT NULL REFERENCES gateway_groups(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    shared_by_user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
    shared_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (group_id, provider_id)
);
CREATE TABLE IF NOT EXISTS gateway_client_devices (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
    last_seen_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS gateway_client_leases (
    user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES gateway_client_devices(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    issued_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, device_id, provider_id)
);
CREATE INDEX IF NOT EXISTS idx_gateway_group_members_user
    ON gateway_group_members(user_id, group_id);
CREATE INDEX IF NOT EXISTS idx_gateway_group_providers_provider
    ON gateway_group_providers(provider_id, group_id);
CREATE INDEX IF NOT EXISTS idx_gateway_client_leases_provider
    ON gateway_client_leases(provider_id, user_id);
"#;

fn database_security_settings_from_connection(
    conn: &Connection,
) -> Result<DatabaseSecuritySettings, String> {
    conn.query_row(
        "SELECT COALESCE(database_encryption_key, ''), COALESCE(feishu_app_id, ''),
                COALESCE(feishu_app_secret, '')
         FROM gateway_state WHERE id = 1",
        [],
        |row| {
            Ok(DatabaseSecuritySettings {
                encryption_key: row.get(0)?,
                feishu_app_id: row.get(1)?,
                feishu_app_secret: row.get(2)?,
            })
        },
    )
    .map_err(|error| format!("load database security settings failed: {error}"))
}

fn database_security_settings_from(
    transaction: &Transaction<'_>,
) -> Result<DatabaseSecuritySettings, String> {
    transaction
        .query_row(
            "SELECT COALESCE(database_encryption_key, ''), COALESCE(feishu_app_id, ''),
                    COALESCE(feishu_app_secret, '')
             FROM gateway_state WHERE id = 1",
            [],
            |row| {
                Ok(DatabaseSecuritySettings {
                    encryption_key: row.get(0)?,
                    feishu_app_id: row.get(1)?,
                    feishu_app_secret: row.get(2)?,
                })
            },
        )
        .map_err(|error| format!("load database security settings failed: {error}"))
}

fn rotate_database_encryption_key(
    transaction: &Transaction<'_>,
    current_key: &str,
    new_key: &str,
    new_encryptor: &FieldEncryptor,
) -> Result<(), String> {
    if !current_key.is_empty() && current_key != new_key {
        let current_encryptor = FieldEncryptor::from_base64_key(current_key)?;
        reencrypt_column(
            transaction,
            "providers",
            "api_key",
            &current_encryptor,
            new_encryptor,
        )?;
        reencrypt_column(
            transaction,
            "gateway_state",
            "feishu_app_secret",
            &current_encryptor,
            new_encryptor,
        )?;
        reencrypt_column(
            transaction,
            "gateway_access_tokens",
            "token_ciphertext",
            &current_encryptor,
            new_encryptor,
        )?;
    }
    transaction
        .execute(
            "UPDATE gateway_state SET database_encryption_key = ?1 WHERE id = 1",
            params![new_key],
        )
        .map_err(|error| format!("save database encryption key failed: {error}"))?;
    Ok(())
}

fn reencrypt_column(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    current_encryptor: &FieldEncryptor,
    new_encryptor: &FieldEncryptor,
) -> Result<(), String> {
    let query = format!(
        "SELECT rowid, {column} FROM {table} WHERE {column} IS NOT NULL AND {column} <> ''"
    );
    let values = {
        let mut statement = transaction
            .prepare(&query)
            .map_err(|error| format!("prepare credential rotation failed: {error}"))?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query credential rotation failed: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read credential rotation failed: {error}"))?
    };
    let update = format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2");
    for (rowid, ciphertext) in values {
        let plaintext = current_encryptor.decrypt(&ciphertext)?;
        transaction
            .execute(&update, params![new_encryptor.encrypt(&plaintext)?, rowid])
            .map_err(|error| format!("save rotated credential failed: {error}"))?;
    }
    Ok(())
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

fn upstream_protocol_from_str(
    value: &str,
) -> Result<ProviderUpstreamProtocol, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "openai_responses" => Ok(ProviderUpstreamProtocol::OpenAiResponses),
        _ => Err(format!("unknown upstream protocol: {value}").into()),
    }
}

fn compatibility_profile_from_str(
    value: &str,
) -> Result<ProviderCompatibilityProfile, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "official_openai" => Ok(ProviderCompatibilityProfile::OfficialOpenAi),
        "generic_openai" => Ok(ProviderCompatibilityProfile::GenericOpenAi),
        _ => Err(format!("unknown compatibility profile: {value}").into()),
    }
}

fn upstream_protocol_to_str(value: &ProviderUpstreamProtocol) -> &'static str {
    match value {
        ProviderUpstreamProtocol::OpenAiResponses => "openai_responses",
    }
}

fn compatibility_profile_to_str(value: &ProviderCompatibilityProfile) -> &'static str {
    match value {
        ProviderCompatibilityProfile::OfficialOpenAi => "official_openai",
        ProviderCompatibilityProfile::GenericOpenAi => "generic_openai",
    }
}

fn decrypt_conversion_error(error: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
    )
}
