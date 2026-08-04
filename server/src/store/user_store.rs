use crate::{config::Config, store::sqlite::SqliteStore};
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const ARGON2_TIME_COST: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;
const ARGON2_VERSION: u32 = 19;
const MAX_NAME_LEN: usize = 80;
const MAX_EMAIL_LEN: usize = 255;

#[derive(Clone, Debug, Serialize)]
pub struct ManagedUser {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub avatar_url: String,
    pub role: String,
    pub has_password: bool,
}

#[derive(Clone, Debug)]
pub struct LoginIdentity {
    pub user: GatewayUser,
    pub name: String,
    pub password_hash: String,
}

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

    pub fn find_user_for_session(&self, user_id: i64) -> Result<Option<FeishuUserProfile>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.query_row(
            "SELECT u.id, u.role,
                    COALESCE(f.name, u.name) AS name,
                    COALESCE(f.avatar_url, '') AS avatar_url
             FROM gateway_users AS u
             LEFT JOIN gateway_feishu_identities AS f ON f.user_id = u.id
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
        .map_err(|error| format!("find session user failed: {error}"))
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

    pub fn create_managed_user(
        &self,
        email: &str,
        name: &str,
        role: UserRole,
        password: &str,
    ) -> Result<GatewayUser, String> {
        let email = email.trim();
        let name = name.trim();
        if email.is_empty() || email.len() > MAX_EMAIL_LEN {
            return Err(format!("邮箱长度必须为 1-{MAX_EMAIL_LEN} 个字符"));
        }
        if !email.contains('@') || email.contains(' ') {
            return Err("邮箱格式不正确".to_string());
        }
        if name.is_empty() || name.len() > MAX_NAME_LEN {
            return Err(format!("姓名长度必须为 1-{MAX_NAME_LEN} 个字符"));
        }
        if password.is_empty() || password.len() < 6 {
            return Err("密码至少 6 个字符".to_string());
        }
        if password.chars().any(|c| c.is_whitespace()) {
            return Err("密码不能包含空白字符".to_string());
        }
        let password_hash = hash_password(password)?;
        let conn = self.sqlite.connect_for_auth()?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM gateway_users WHERE email = ?1 COLLATE NOCASE)",
                params![email],
                |row| row.get(0),
            )
            .map_err(|error| format!("check managed user email failed: {error}"))?;
        if exists {
            return Err("该邮箱已被注册".to_string());
        }
        conn.query_row(
            "INSERT INTO gateway_users (email, name, password_hash, role) VALUES (?1, ?2, ?3, ?4) RETURNING id, role",
            params![email, name, password_hash, role.as_str()],
            user_from_row,
        )
        .map_err(|error| format!("create managed user failed: {error}"))
    }

    pub fn verify_login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<LoginIdentity>, String> {
        let email = email.trim();
        let conn = self.sqlite.connect_for_auth()?;
        let identity = conn
            .query_row(
                "SELECT u.id, u.role, u.name, u.password_hash
                 FROM gateway_users AS u
                 WHERE u.email = ?1 COLLATE NOCASE",
                params![email],
                |row| {
                    Ok(LoginIdentity {
                        user: GatewayUser {
                            id: row.get(0)?,
                            role: UserRole::from_str(&row.get::<_, String>(1)?)?,
                        },
                        name: row.get(2)?,
                        password_hash: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("find login user failed: {error}"))?;
        let Some(identity) = identity else {
            return Ok(None);
        };
        if identity.password_hash.is_empty() || !verify_password(password, &identity.password_hash)
        {
            return Ok(None);
        }
        Ok(Some(identity))
    }

    pub fn list_all_users(&self) -> Result<Vec<ManagedUser>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        let mut statement = conn
            .prepare(
                "SELECT u.id, u.email, u.name, u.role,
                        COALESCE((SELECT avatar_url FROM gateway_feishu_identities WHERE user_id = u.id), ''),
                        CASE WHEN u.password_hash != '' THEN 1 ELSE 0 END
                 FROM gateway_users AS u
                 ORDER BY u.id",
            )
            .map_err(|error| format!("prepare managed user list failed: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(ManagedUser {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    name: row.get(2)?,
                    role: row.get(3)?,
                    avatar_url: row.get(4)?,
                    has_password: row.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(|error| format!("list managed users failed: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read managed users failed: {error}"))
    }

    pub fn delete_user(&self, user_id: i64) -> Result<(), String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.execute("DELETE FROM gateway_users WHERE id = ?1", params![user_id])
            .map_err(|error| format!("delete managed user failed: {error}"))?;
        Ok(())
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

fn argon2id() -> Argon2<'static> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_TIME_COST,
        ARGON2_PARALLELISM,
        Some(32),
    )
    .expect("Argon2id parameters must be valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_password(password: &str) -> Result<String, String> {
    if password.is_empty() {
        return Err("密码不能为空".to_string());
    }
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).map_err(|_| "生成密码盐值失败".to_string())?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| format!("编码密码盐值失败：{error}"))?;
    argon2id()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| format!("生成密码哈希失败：{error}"))
}

fn verify_password(password: &str, stored: &str) -> bool {
    let Ok(hash) = PasswordHash::new(stored) else {
        return false;
    };
    if hash.algorithm.as_str() != "argon2id"
        || hash.version != Some(ARGON2_VERSION)
        || hash.params.get_decimal("m") != Some(ARGON2_MEMORY_KIB)
        || hash.params.get_decimal("t") != Some(ARGON2_TIME_COST)
        || hash.params.get_decimal("p") != Some(ARGON2_PARALLELISM)
    {
        return false;
    }
    argon2id()
        .verify_password(password.as_bytes(), &hash)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_db_path() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "ai_gateway_user_store_{}_{}.db",
            std::process::id(),
            stamp
        ));
        path
    }

    #[test]
    fn creates_verifies_lists_and_deletes_managed_users() {
        let db_path = unique_db_path();
        let sqlite = SqliteStore::for_test(db_path.clone()).expect("create sqlite");
        sqlite
            .initialize_user_auth_schema()
            .expect("initialize schema");
        let store = UserStore { sqlite };

        let user = store
            .create_managed_user("dev@example.com", "开发调试", UserRole::Admin, "secret123")
            .expect("create user");
        assert_eq!(user.role, UserRole::Admin);

        let identity = store
            .verify_login("DEV@example.com", "secret123")
            .expect("verify login")
            .expect("identity present");
        assert_eq!(identity.user.id, user.id);
        assert_eq!(identity.name, "开发调试");

        assert!(
            store
                .verify_login("dev@example.com", "wrong-password")
                .expect("verify")
                .is_none()
        );
        assert!(
            store
                .verify_login("nobody@example.com", "secret123")
                .expect("verify")
                .is_none()
        );

        // duplicate email rejected
        let dup = store.create_managed_user("dev@example.com", "重复", UserRole::User, "secret123");
        assert!(dup.is_err());

        // short password rejected
        let short = store.create_managed_user("a@b.com", "短", UserRole::User, "abc");
        assert!(short.is_err());

        let users = store.list_all_users().expect("list users");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].email, "dev@example.com");
        assert!(users[0].has_password);

        store.delete_user(user.id).expect("delete user");
        assert_eq!(store.list_all_users().expect("list").len(), 0);
    }

    #[test]
    fn argon2id_hash_is_salted_and_rejects_non_argon2_formats() {
        let hash_a = hash_password("s3cret").expect("hash");
        let hash_b = hash_password("s3cret").expect("hash");
        assert!(hash_a.starts_with("$argon2id$v=19$m=65536,t=3,p=1$"));
        assert_ne!(
            hash_a, hash_b,
            "different salts must produce different hashes"
        );
        assert!(verify_password("s3cret", &hash_a));
        assert!(!verify_password("wrong", &hash_a));
        assert!(!verify_password(
            "s3cret",
            &hash_a.replacen("m=65536", "m=19456", 1),
        ));
        assert!(!verify_password("s3cret", ""));
        assert!(!verify_password("s3cret", "sha256$nothex$100000$ff"));
    }
}
