use crate::{
    config::Config,
    models::{
        ApiProviderSummary, GatewayGroupDetail, GatewayGroupMember, GatewayGroupProvider,
        GatewayGroupSummary, GatewayUserSummary,
    },
    store::sqlite::SqliteStore,
};
use rusqlite::{OptionalExtension, params};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct GroupStore {
    sqlite: SqliteStore,
}

impl GroupStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let sqlite = SqliteStore::new(config)?;
        sqlite.initialize_group_schema()?;
        Ok(Self { sqlite })
    }

    pub fn search_users(
        &self,
        query: &str,
        exclude_user_id: i64,
    ) -> Result<Vec<GatewayUserSummary>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        let pattern = format!("%{}%", query.trim());
        let mut statement = conn
            .prepare(
                "SELECT id, name, COALESCE(
                    (SELECT avatar_url FROM gateway_feishu_identities WHERE user_id = gateway_users.id), ''
                 )
                 FROM gateway_users
                 WHERE id != ?1 AND (name LIKE ?2 COLLATE NOCASE OR email LIKE ?2 COLLATE NOCASE)
                 ORDER BY name COLLATE NOCASE
                 LIMIT 20",
            )
            .map_err(|error| format!("prepare user search failed: {error}"))?;
        let rows = statement
            .query_map(params![exclude_user_id, pattern], |row| {
                Ok(GatewayUserSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    avatar_url: row.get(2)?,
                })
            })
            .map_err(|error| format!("search users failed: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read searched users failed: {error}"))
    }

    pub fn create_group(
        &self,
        owner_user_id: i64,
        name: &str,
        now: i64,
    ) -> Result<GatewayGroupSummary, String> {
        let name = name.trim();
        if name.is_empty() || name.len() > 80 {
            return Err("群组名称长度必须为 1-80 个字符".to_string());
        }
        let conn = self.sqlite.connect_for_auth()?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| format!("begin create group transaction failed: {error}"))?;
        tx.execute(
            "INSERT INTO gateway_groups (name, owner_user_id, created_at) VALUES (?1, ?2, ?3)",
            params![name, owner_user_id, now],
        )
        .map_err(|error| format!("create group failed: {error}"))?;
        let group_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO gateway_group_members (group_id, user_id, role, joined_at)
             VALUES (?1, ?2, 'owner', ?3)",
            params![group_id, owner_user_id, now],
        )
        .map_err(|error| format!("add group owner failed: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit create group transaction failed: {error}"))?;
        self.get_summary(group_id, owner_user_id)?
            .ok_or_else(|| "创建群组后无法读取群组".to_string())
    }

    pub fn list_for_user(&self, user_id: i64) -> Result<Vec<GatewayGroupSummary>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        let mut statement = conn
            .prepare(
                "SELECT g.id, g.name, g.owner_user_id, owner.name,
                        gm.role, COUNT(DISTINCT members.user_id), COUNT(DISTINCT providers.provider_id), g.created_at
                 FROM gateway_groups AS g
                 JOIN gateway_users AS owner ON owner.id = g.owner_user_id
                 JOIN gateway_group_members AS gm ON gm.group_id = g.id AND gm.user_id = ?1
                 LEFT JOIN gateway_group_members AS members ON members.group_id = g.id
                 LEFT JOIN gateway_group_providers AS providers ON providers.group_id = g.id
                 GROUP BY g.id, g.name, g.owner_user_id, owner.name, gm.role, g.created_at
                 ORDER BY g.created_at DESC",
            )
            .map_err(|error| format!("prepare group list failed: {error}"))?;
        let rows = statement
            .query_map(params![user_id], |row| {
                Ok(GatewayGroupSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    owner_user_id: row.get(2)?,
                    owner_name: row.get(3)?,
                    role: row.get(4)?,
                    member_count: row.get(5)?,
                    provider_count: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|error| format!("query groups failed: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read groups failed: {error}"))
    }

    pub fn get_detail(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<Option<GatewayGroupDetail>, String> {
        let Some(group) = self.get_summary(group_id, user_id)? else {
            return Ok(None);
        };
        let conn = self.sqlite.connect_for_auth()?;

        let mut member_statement = conn
            .prepare(
                "SELECT gm.user_id, u.name,
                        COALESCE((SELECT avatar_url FROM gateway_feishu_identities WHERE user_id = u.id), ''),
                        gm.role
                 FROM gateway_group_members AS gm
                 JOIN gateway_users AS u ON u.id = gm.user_id
                 WHERE gm.group_id = ?1
                 ORDER BY CASE gm.role WHEN 'owner' THEN 0 ELSE 1 END, u.name COLLATE NOCASE",
            )
            .map_err(|error| format!("prepare group member query failed: {error}"))?;
        let members = member_statement
            .query_map(params![group_id], |row| {
                Ok(GatewayGroupMember {
                    user_id: row.get(0)?,
                    name: row.get(1)?,
                    avatar_url: row.get(2)?,
                    role: row.get(3)?,
                })
            })
            .map_err(|error| format!("query group members failed: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read group members failed: {error}"))?;

        let mut provider_statement = conn
            .prepare(
                "SELECT p.id, p.name, p.auth_mode, COALESCE(p.base_url, ''), p.account_id,
                        p.upstream_protocol, p.compatibility_profile,
                        gp.shared_by_user_id, u.name, gp.shared_at,
                        CASE WHEN gp.shared_by_user_id = ?2 OR g.owner_user_id = ?2 THEN 1 ELSE 0 END
                 FROM gateway_group_providers AS gp
                 JOIN providers AS p ON p.id = gp.provider_id
                 JOIN gateway_users AS u ON u.id = gp.shared_by_user_id
                 JOIN gateway_groups AS g ON g.id = gp.group_id
                 WHERE gp.group_id = ?1
                 ORDER BY gp.shared_at DESC",
            )
            .map_err(|error| format!("prepare group provider query failed: {error}"))?;
        let providers = provider_statement
            .query_map(params![group_id, user_id], |row| {
                Ok(GatewayGroupProvider {
                    provider: ApiProviderSummary {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        auth_mode: provider_auth_mode(&row.get::<_, String>(2)?)?,
                        base_url: row.get(3)?,
                        account_id: row.get(4)?,
                        account_email: None,
                        upstream_protocol: upstream_protocol(&row.get::<_, String>(5)?)?,
                        compatibility_profile: compatibility_profile(&row.get::<_, String>(6)?)?,
                        shared: true,
                    },
                    shared_by_user_id: row.get(7)?,
                    shared_by_name: row.get(8)?,
                    shared_at: row.get(9)?,
                    can_remove: row.get::<_, i64>(10)? != 0,
                })
            })
            .map_err(|error| format!("query group providers failed: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read group providers failed: {error}"))?;

        Ok(Some(GatewayGroupDetail {
            group,
            members,
            providers,
        }))
    }

    pub fn add_member(
        &self,
        group_id: i64,
        actor_user_id: i64,
        user_id: i64,
        is_admin: bool,
        now: i64,
    ) -> Result<(), String> {
        self.ensure_group_manager(group_id, actor_user_id, is_admin)?;
        let conn = self.sqlite.connect_for_auth()?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM gateway_users WHERE id = ?1)",
                params![user_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check group member user failed: {error}"))?;
        if !exists {
            return Err("用户不存在".to_string());
        }
        conn.execute(
            "INSERT OR IGNORE INTO gateway_group_members (group_id, user_id, role, joined_at)
             VALUES (?1, ?2, 'member', ?3)",
            params![group_id, user_id, now],
        )
        .map_err(|error| format!("add group member failed: {error}"))?;
        Ok(())
    }

    pub fn remove_member(
        &self,
        group_id: i64,
        actor_user_id: i64,
        user_id: i64,
        is_admin: bool,
    ) -> Result<(), String> {
        self.ensure_group_manager(group_id, actor_user_id, is_admin)?;
        let conn = self.sqlite.connect_for_auth()?;
        let owner: i64 = conn
            .query_row(
                "SELECT owner_user_id FROM gateway_groups WHERE id = ?1",
                params![group_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("load group owner failed: {error}"))?;
        if owner == user_id {
            return Err("不能移除群组创建者".to_string());
        }
        conn.execute(
            "DELETE FROM gateway_group_members WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )
        .map_err(|error| format!("remove group member failed: {error}"))?;
        Ok(())
    }

    pub fn share_provider(
        &self,
        group_id: i64,
        actor_user_id: i64,
        provider_id: &str,
        now: i64,
    ) -> Result<(), String> {
        self.ensure_member(group_id, actor_user_id)?;
        let conn = self.sqlite.connect_for_auth()?;
        let owns_provider: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM providers WHERE id = ?1 AND owner_user_id = ?2
                )",
                params![provider_id, actor_user_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check provider ownership failed: {error}"))?;
        if !owns_provider {
            return Err("只能共享自己拥有的供应商".to_string());
        }
        conn.execute(
            "INSERT OR IGNORE INTO gateway_group_providers
                (group_id, provider_id, shared_by_user_id, shared_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![group_id, provider_id, actor_user_id, now],
        )
        .map_err(|error| format!("share provider to group failed: {error}"))?;
        Ok(())
    }

    pub fn unshare_provider(
        &self,
        group_id: i64,
        actor_user_id: i64,
        provider_id: &str,
        is_admin: bool,
    ) -> Result<(), String> {
        self.ensure_member(group_id, actor_user_id)?;
        let conn = self.sqlite.connect_for_auth()?;
        let allowed: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM gateway_group_providers AS gp
                    JOIN gateway_groups AS g ON g.id = gp.group_id
                    WHERE gp.group_id = ?1 AND gp.provider_id = ?2
                      AND (gp.shared_by_user_id = ?3 OR g.owner_user_id = ?3 OR ?4 = 1)
                )",
                params![group_id, provider_id, actor_user_id, i64::from(is_admin)],
                |row| row.get(0),
            )
            .map_err(|error| format!("check provider unshare permission failed: {error}"))?;
        if !allowed {
            return Err("没有取消该供应商共享的权限".to_string());
        }
        conn.execute(
            "DELETE FROM gateway_group_providers WHERE group_id = ?1 AND provider_id = ?2",
            params![group_id, provider_id],
        )
        .map_err(|error| format!("unshare provider failed: {error}"))?;
        Ok(())
    }

    fn ensure_member(&self, group_id: i64, user_id: i64) -> Result<(), String> {
        let conn = self.sqlite.connect_for_auth()?;
        let member: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM gateway_group_members WHERE group_id = ?1 AND user_id = ?2
                )",
                params![group_id, user_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check group membership failed: {error}"))?;
        if member {
            Ok(())
        } else {
            Err("你不是该群组成员".to_string())
        }
    }

    fn ensure_group_manager(
        &self,
        group_id: i64,
        user_id: i64,
        is_admin: bool,
    ) -> Result<(), String> {
        if is_admin {
            return Ok(());
        }
        let conn = self.sqlite.connect_for_auth()?;
        let manager: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM gateway_groups
                    WHERE id = ?1 AND owner_user_id = ?2
                )",
                params![group_id, user_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check group manager failed: {error}"))?;
        if manager {
            Ok(())
        } else {
            Err("只有群组创建者可以管理成员".to_string())
        }
    }

    fn get_summary(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<Option<GatewayGroupSummary>, String> {
        let conn = self.sqlite.connect_for_auth()?;
        conn.query_row(
            "SELECT g.id, g.name, g.owner_user_id, owner.name, gm.role,
                    (SELECT COUNT(*) FROM gateway_group_members WHERE group_id = g.id),
                    (SELECT COUNT(*) FROM gateway_group_providers WHERE group_id = g.id),
                    g.created_at
             FROM gateway_groups AS g
             JOIN gateway_users AS owner ON owner.id = g.owner_user_id
             JOIN gateway_group_members AS gm ON gm.group_id = g.id AND gm.user_id = ?2
             WHERE g.id = ?1",
            params![group_id, user_id],
            |row| {
                Ok(GatewayGroupSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    owner_user_id: row.get(2)?,
                    owner_name: row.get(3)?,
                    role: row.get(4)?,
                    member_count: row.get(5)?,
                    provider_count: row.get(6)?,
                    created_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("load group summary failed: {error}"))
    }
}

fn provider_auth_mode(value: &str) -> rusqlite::Result<crate::models::ProviderAuthMode> {
    match value {
        "api_key" => Ok(crate::models::ProviderAuthMode::ApiKey),
        "account" => Ok(crate::models::ProviderAuthMode::Account),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn upstream_protocol(value: &str) -> rusqlite::Result<crate::models::ProviderUpstreamProtocol> {
    match value {
        "openai_responses" => Ok(crate::models::ProviderUpstreamProtocol::OpenAiResponses),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn compatibility_profile(
    value: &str,
) -> rusqlite::Result<crate::models::ProviderCompatibilityProfile> {
    match value {
        "official_openai" => Ok(crate::models::ProviderCompatibilityProfile::OfficialOpenAi),
        "generic_openai" => Ok(crate::models::ProviderCompatibilityProfile::GenericOpenAi),
        "openai_codex" => Ok(crate::models::ProviderCompatibilityProfile::OpenAiCodex),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

#[cfg(test)]
mod tests {
    use super::GroupStore;
    use crate::{
        models::{
            ApiProviderRecord, ProviderAuthMode, ProviderCompatibilityProfile,
            ProviderUpstreamProtocol,
        },
        store::sqlite::SqliteStore,
    };
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn supports_member_search_and_shared_provider_visibility() {
        let db_path = unique_db_path();
        let sqlite = SqliteStore::for_test(db_path.clone()).expect("create sqlite");
        sqlite
            .initialize_group_schema()
            .expect("initialize group schema");
        sqlite
            .initialize_user_auth_schema()
            .expect("initialize user schema");
        sqlite
            .connect_for_auth()
            .expect("connect")
            .execute(
                "INSERT INTO gateway_users (email, name, password_hash, role)
                 VALUES ('owner@example.com', 'Owner', '', 'user'),
                        ('member@example.com', 'Member', '', 'user')",
                [],
            )
            .expect("insert users");
        let store = GroupStore {
            sqlite: sqlite.clone(),
        };
        let group = store.create_group(1, "Team", 100).expect("create group");
        store
            .add_member(group.id, 1, 2, false, 101)
            .expect("add member");
        sqlite
            .upsert_provider(&ApiProviderRecord {
                id: "provider_1".to_string(),
                name: "Shared".to_string(),
                auth_mode: ProviderAuthMode::ApiKey,
                base_url: "https://example.com/v1".to_string(),
                api_key: "sk-test".to_string(),
                account_id: None,
                upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
                compatibility_profile: ProviderCompatibilityProfile::GenericOpenAi,
                owner_user_id: Some(1),
            })
            .expect("insert provider");
        store
            .share_provider(group.id, 1, "provider_1", 102)
            .expect("share provider");

        let searched = store.search_users("Member", 1).expect("search users");
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].id, 2);
        let detail = store
            .get_detail(group.id, 2)
            .expect("load detail")
            .expect("detail");
        assert_eq!(detail.members.len(), 2);
        assert_eq!(detail.providers.len(), 1);
        assert_eq!(detail.providers[0].provider.id, "provider_1");

        let shared = sqlite
            .shared_provider_ids_for_user(2)
            .expect("shared provider ids");
        assert!(shared.contains("provider_1"));

        let _ = std::fs::remove_file(db_path);
    }

    fn unique_db_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "ai_gateway_groups_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }
}
