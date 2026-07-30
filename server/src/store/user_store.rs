use crate::{config::Config, store::sqlite::SqliteStore};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct GatewayUser {
    pub id: i64,
    pub role: UserRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserRole {
    Admin,
    User,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    fn from_str(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GatewaySession {
    pub user_id: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug)]
pub struct FeishuUserProfile {
    pub user: GatewayUser,
    pub name: String,
    pub avatar_url: String,
}

#[derive(Clone, Debug)]
pub struct UserStore {
    sqlite: SqliteStore,
}

impl UserStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    pub fn initialize(&self) -> Result<(), String> {
        self.sqlite.initialize_user_auth_schema()
    }

    pub fn upsert_feishu_user(
        &self,
        tenant_key: &str,
        open_id: &str,
        name: &str,
        avatar_url: &str,
    ) -> Result<GatewayUser, String> {
        let mut conn = self.sqlite.connect_for_auth()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("begin Feishu user transaction failed: {error}"))?;
        let existing_user_id = transaction
            .query_row(
                "SELECT user_id FROM gateway_feishu_identities WHERE tenant_key = ?1 AND open_id = ?2",
                params![tenant_key, open_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("find Feishu identity failed: {error}"))?;

        let user = if let Some(user_id) = existing_user_id {
            transaction
                .execute(
                    "UPDATE gateway_users SET name = ?1 WHERE id = ?2",
                    params![name, user_id],
                )
                .map_err(|error| format!("update Feishu user failed: {error}"))?;
            transaction.execute(
                "UPDATE gateway_feishu_identities SET name = ?1, avatar_url = ?2 WHERE user_id = ?3",
                params![name, avatar_url, user_id],
            ).map_err(|error| format!("update Feishu identity failed: {error}"))?;
            transaction
                .query_row(
                    "SELECT id, role FROM gateway_users WHERE id = ?1",
                    params![user_id],
                    user_from_row,
                )
                .map_err(|error| format!("load Feishu user failed: {error}"))?
        } else {
            let email = format!("feishu:{}:{}", tenant_key, open_id);
            let has_users: bool = transaction
                .query_row("SELECT EXISTS(SELECT 1 FROM gateway_users)", [], |row| {
                    row.get(0)
                })
                .map_err(|error| format!("check gateway user count failed: {error}"))?;
            let role = if has_users {
                UserRole::User
            } else {
                UserRole::Admin
            };
            let user = transaction.query_row(
                "INSERT INTO gateway_users (email, name, password_hash, role) VALUES (?1, ?2, '', ?3) RETURNING id, role",
                params![email, name, role.as_str()], user_from_row,
            ).map_err(|error| format!("create Feishu user failed: {error}"))?;
            transaction.execute(
                "INSERT INTO gateway_feishu_identities (user_id, tenant_key, open_id, name, avatar_url) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![user.id, tenant_key, open_id, name, avatar_url],
            ).map_err(|error| format!("create Feishu identity failed: {error}"))?;
            user
        };
        transaction
            .commit()
            .map_err(|error| format!("commit Feishu user transaction failed: {error}"))?;
        Ok(user)
    }

    pub fn find_feishu_user_by_id(
        &self,
        user_id: i64,
    ) -> Result<Option<FeishuUserProfile>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.query_row(
            "SELECT u.id, u.role, f.name, f.avatar_url
             FROM gateway_users AS u
             JOIN gateway_feishu_identities AS f ON f.user_id = u.id
             WHERE u.id = ?1",
            params![user_id],
            |row| {
                Ok(FeishuUserProfile {
                    user: GatewayUser {
                        id: row.get(0)?,
                        role: UserRole::from_str(&row.get::<_, String>(1)?)?,
                    },
                    name: row.get(2)?,
                    avatar_url: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("find Feishu user failed: {error}"))
    }

    pub fn create_session(
        &self,
        session_id: &str,
        user_id: i64,
        expires_at: i64,
    ) -> Result<(), String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.execute(
            "INSERT INTO gateway_sessions (session_id, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![session_id, user_id, expires_at],
        )
        .map_err(|error| format!("create gateway session failed: {error}"))?;
        Ok(())
    }

    pub fn find_session(&self, session_id: &str) -> Result<Option<GatewaySession>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.query_row(
            "SELECT user_id, expires_at FROM gateway_sessions WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok(GatewaySession {
                    user_id: row.get(0)?,
                    expires_at: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("find gateway session failed: {error}"))
    }

    pub fn touch_session(&self, session_id: &str, now: i64) -> Result<(), String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.execute(
            "UPDATE gateway_sessions SET last_seen_at = ?1 WHERE session_id = ?2",
            params![now, session_id],
        )
        .map_err(|error| format!("touch gateway session failed: {error}"))?;
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.execute(
            "DELETE FROM gateway_sessions WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|error| format!("delete gateway session failed: {error}"))?;
        Ok(())
    }

    pub fn load_gateway_access_token(&self, user_id: i64) -> Result<Option<String>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        let ciphertext = conn
            .query_row(
                "SELECT token_ciphertext FROM gateway_access_tokens WHERE user_id = ?1",
                params![user_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| format!("load gateway access token failed: {error}"))?
            .flatten();
        ciphertext
            .map(|value| self.sqlite.encryption()?.decrypt(&value))
            .transpose()
    }

    pub fn replace_gateway_access_token(&self, user_id: i64, token: &str) -> Result<(), String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.execute(
            "INSERT INTO gateway_access_tokens (token_hash, token_ciphertext, user_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id) DO UPDATE SET
                token_hash = excluded.token_hash,
                token_ciphertext = excluded.token_ciphertext,
                created_at = unixepoch()",
            params![
                token_hash(token),
                self.sqlite.encryption()?.encrypt(token)?,
                user_id
            ],
        )
        .map_err(|error| format!("replace gateway access token failed: {error}"))?;
        Ok(())
    }

    pub fn find_user_by_gateway_access_token(
        &self,
        token: &str,
    ) -> Result<Option<GatewayUser>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.query_row(
            "SELECT u.id, u.role
             FROM gateway_access_tokens AS t
             JOIN gateway_users AS u ON u.id = t.user_id
             WHERE t.token_hash = ?1",
            params![token_hash(token)],
            user_from_row,
        )
        .optional()
        .map_err(|error| format!("find gateway access token failed: {error}"))
    }
}

fn user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GatewayUser> {
    Ok(GatewayUser {
        id: row.get(0)?,
        role: UserRole::from_str(&row.get::<_, String>(1)?)?,
    })
}

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}
