use crate::{
    config::Config,
    crypto::FieldEncryptor,
    models::{
        AccountRecord, AccountType, ApiProviderRecord, AutoRoutingSettings, CachedProviderModels,
        ProviderAuthMode, ProviderCompatibilityProfile, ProviderUpstreamProtocol,
        ROUTING_LOW_CONFIDENCE_THRESHOLD, RoutingModelTarget, SelectedRoute, TurnRouteLog,
        TurnRouteLogUpdate,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
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
    pub auth_required: bool,
}

impl SqliteStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        fs::create_dir_all(config.data_dir())
            .map_err(|err| format!("create data dir failed: {err}"))?;

        let store = Self {
            db_path: config.sqlite_path(),
        };
        store.init()?;
        #[cfg(test)]
        store.set_database_encryption_key("MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=")?;
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
        store.set_database_encryption_key("MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=")?;
        Ok(store)
    }

    pub fn load_accounts(&self) -> Result<Vec<AccountRecord>, String> {
        let conn = self.connect()?;
        let has_accounts: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM accounts)", [], |row| {
                row.get(0)
            })
            .map_err(|err| format!("check accounts failed: {err}"))?;
        if !has_accounts {
            return Ok(Vec::new());
        }
        let mut stmt = conn
            .prepare(
                "SELECT id, account_type, email, access_token, refresh_token, expiry_timestamp, client_id, upstream_account_id, owner_user_id
                 FROM accounts
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare accounts query failed: {err}"))?;
        let Some(encryption) = self.optional_encryption()? else {
            // A gateway must remain bootable for first-run setup. Existing
            // credential rows stay untouched and are deliberately unavailable
            // until an administrator configures the database encryption key.
            return Ok(Vec::new());
        };
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
                    owner_user_id: row.get(8)?,
                })
            })
            .map_err(|err| format!("query accounts failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read accounts failed: {err}"))
    }

    pub fn upsert_account(&self, account: &AccountRecord) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_account_record(&conn, &self.encryption()?, account)
    }

    pub fn delete_account(&self, account_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute("DELETE FROM accounts WHERE id = ?1", params![account_id])
            .map_err(|err| format!("delete account failed: {err}"))?;
        Ok(())
    }

    pub fn load_providers(&self) -> Result<Vec<ApiProviderRecord>, String> {
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
                "SELECT id, name, auth_mode, COALESCE(base_url, ''), COALESCE(api_key, ''), account_id,
                        upstream_protocol, compatibility_profile, owner_user_id
                 FROM providers
                 ORDER BY rowid ASC",
            )
            .map_err(|err| format!("prepare providers query failed: {err}"))?;
        let Some(encryption) = self.optional_encryption()? else {
            // See load_accounts: do not make a missing setup key fatal at
            // startup, and never expose credential-backed providers without it.
            return Ok(Vec::new());
        };
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
                    owner_user_id: row.get(8)?,
                })
            })
            .map_err(|err| format!("query providers failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read providers failed: {err}"))
    }

    pub fn upsert_provider(&self, provider: &ApiProviderRecord) -> Result<(), String> {
        let conn = self.connect()?;
        upsert_provider_record(&conn, &self.encryption()?, provider)
    }

    pub fn delete_provider(&self, provider_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute("DELETE FROM providers WHERE id = ?1", params![provider_id])
            .map_err(|err| format!("delete provider failed: {err}"))?;
        Ok(())
    }

    pub fn delete_instance_route(&self, instance_id: &str) -> Result<bool, String> {
        let conn = self.connect()?;
        let deleted = conn
            .execute(
                "DELETE FROM gateway_instance_state WHERE instance_id = ?1",
                params![instance_id],
            )
            .map_err(|err| format!("delete instance route failed: {err}"))?;
        Ok(deleted > 0)
    }

    pub fn list_instance_ids(&self) -> Result<Vec<String>, String> {
        let conn = self.connect()?;
        let mut statement = conn
            .prepare("SELECT instance_id FROM gateway_instance_state ORDER BY instance_id COLLATE NOCASE")
            .map_err(|err| format!("prepare instance list query failed: {err}"))?;
        let rows = statement
            .query_map([], |row| row.get(0))
            .map_err(|err| format!("query instance list failed: {err}"))?;
        rows.collect::<Result<Vec<String>, _>>()
            .map_err(|err| format!("read instance list failed: {err}"))
    }

    pub fn load_instance_route(&self, instance_id: &str) -> Result<SelectedRoute, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT selected_provider_id, selected_model, selected_reasoning_effort, route_updated_at
             FROM gateway_instance_state WHERE instance_id = ?1",
            params![instance_id],
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
        .map_err(|err| format!("load instance route failed: {err}"))
        .map(|route| route.unwrap_or_default())
    }

    pub fn upsert_instance_route(
        &self,
        instance_id: &str,
        route: &SelectedRoute,
    ) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO gateway_instance_state (
                instance_id, selected_provider_id, selected_model, selected_reasoning_effort, route_updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(instance_id) DO UPDATE SET
                selected_provider_id = excluded.selected_provider_id,
                selected_model = excluded.selected_model,
                selected_reasoning_effort = excluded.selected_reasoning_effort,
                route_updated_at = excluded.route_updated_at",
            params![
                instance_id,
                route.provider_id,
                route.selected_model,
                route.selected_reasoning_effort,
                route.updated_at
            ],
        )
        .map_err(|err| format!("upsert instance route failed: {err}"))?;
        Ok(())
    }

    pub fn clear_instance_routes_for_provider(&self, provider_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "UPDATE gateway_instance_state
             SET selected_provider_id = NULL, selected_model = NULL, selected_reasoning_effort = NULL
             WHERE selected_provider_id = ?1",
            params![provider_id],
        )
        .map_err(|err| format!("clear instance routes for provider failed: {err}"))?;
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

    pub fn load_instance_auto_routing_settings(
        &self,
        instance_id: &str,
    ) -> Result<AutoRoutingSettings, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT routing_light_target, routing_standard_target, routing_pro_target,
                    routing_max_target, routing_enabled, routing_low_confidence_threshold
             FROM gateway_instance_state WHERE instance_id = ?1",
            params![instance_id],
            |row| {
                Ok(AutoRoutingSettings {
                    enabled: row.get::<_, i64>(4)? != 0,
                    light: routing_target_from_storage(row.get(0)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    standard: routing_target_from_storage(row.get(1)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    pro: routing_target_from_storage(row.get(2)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    max: routing_target_from_storage(row.get(3)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    low_confidence_threshold: ROUTING_LOW_CONFIDENCE_THRESHOLD,
                })
            },
        )
        .optional()
        .map_err(|err| format!("load instance automatic routing settings failed: {err}"))
        .map(|settings| settings.unwrap_or_default())
    }

    pub fn set_instance_auto_routing_settings(
        &self,
        instance_id: &str,
        settings: &AutoRoutingSettings,
    ) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO gateway_instance_state (
                instance_id, routing_enabled, routing_light_target, routing_standard_target,
                routing_pro_target, routing_max_target, routing_low_confidence_threshold
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(instance_id) DO UPDATE SET
                routing_enabled = excluded.routing_enabled,
                routing_light_target = excluded.routing_light_target,
                routing_standard_target = excluded.routing_standard_target,
                routing_pro_target = excluded.routing_pro_target,
                routing_max_target = excluded.routing_max_target,
                routing_low_confidence_threshold = excluded.routing_low_confidence_threshold",
            params![
                instance_id,
                i64::from(settings.enabled),
                routing_target_to_storage(settings.light.as_ref())?,
                routing_target_to_storage(settings.standard.as_ref())?,
                routing_target_to_storage(settings.pro.as_ref())?,
                routing_target_to_storage(settings.max.as_ref())?,
                ROUTING_LOW_CONFIDENCE_THRESHOLD,
            ],
        )
        .map_err(|err| format!("upsert instance automatic routing settings failed: {err}"))?;
        Ok(())
    }

    pub fn clear_instance_auto_routing_provider(&self, provider_id: &str) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "UPDATE gateway_instance_state SET routing_enabled = 0
             WHERE routing_classifier_target LIKE '%' || ?1 || '%'
                OR routing_light_target LIKE '%' || ?1 || '%'
                OR routing_standard_target LIKE '%' || ?1 || '%'
                OR routing_pro_target LIKE '%' || ?1 || '%'
                OR routing_max_target LIKE '%' || ?1 || '%'",
            params![provider_id],
        )
        .map_err(|err| format!("clear instance automatic routing provider failed: {err}"))?;
        Ok(())
    }

    pub fn load_auto_routing_settings(&self) -> Result<AutoRoutingSettings, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT routing_light_target, routing_standard_target, routing_pro_target,
                    routing_max_target, routing_enabled, routing_low_confidence_threshold
             FROM gateway_state WHERE id = 1",
            [],
            |row| {
                Ok(AutoRoutingSettings {
                    enabled: row.get::<_, i64>(4)? != 0,
                    light: routing_target_from_storage(row.get(0)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    standard: routing_target_from_storage(row.get(1)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    pro: routing_target_from_storage(row.get(2)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    max: routing_target_from_storage(row.get(3)?)
                        .map_err(rusqlite::Error::ToSqlConversionFailure)?,
                    low_confidence_threshold: ROUTING_LOW_CONFIDENCE_THRESHOLD,
                })
            },
        )
        .optional()
        .map_err(|err| format!("load automatic routing settings failed: {err}"))
        .map(|settings| settings.unwrap_or_default())
    }

    pub fn set_auto_routing_settings(&self, settings: &AutoRoutingSettings) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO gateway_state (
                id, routing_enabled, routing_light_target, routing_standard_target,
                routing_pro_target, routing_max_target, routing_low_confidence_threshold,
                route_updated_at
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, 0)
             ON CONFLICT(id) DO UPDATE SET
                routing_enabled = excluded.routing_enabled,
                routing_light_target = excluded.routing_light_target,
                routing_standard_target = excluded.routing_standard_target,
                routing_pro_target = excluded.routing_pro_target,
                routing_max_target = excluded.routing_max_target,
                routing_low_confidence_threshold = excluded.routing_low_confidence_threshold",
            params![
                i64::from(settings.enabled),
                routing_target_to_storage(settings.light.as_ref())?,
                routing_target_to_storage(settings.standard.as_ref())?,
                routing_target_to_storage(settings.pro.as_ref())?,
                routing_target_to_storage(settings.max.as_ref())?,
                ROUTING_LOW_CONFIDENCE_THRESHOLD,
            ],
        )
        .map_err(|err| format!("upsert automatic routing settings failed: {err}"))?;
        Ok(())
    }

    pub fn load_turn_route_log(&self, turn_id: &str) -> Result<Option<TurnRouteLog>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT turn_id, provider_id, model, routing_mode, routing_reason, routing_detail,
                    routing_tier, classifier_confidence, classifier_output,
                    reasoning_effort, user_input_preview,
                    started_at, updated_at, request_count, tool_round_count
             FROM turn_route_logs WHERE turn_id = ?1",
            params![turn_id],
            turn_route_log_from_row,
        )
        .optional()
        .map_err(|err| format!("load turn route log failed: {err}"))
    }

    pub fn list_turn_route_logs(&self, limit: i64) -> Result<Vec<TurnRouteLog>, String> {
        let conn = self.connect()?;
        let mut statement = conn
            .prepare(
                "SELECT turn_id, provider_id, model, routing_mode, routing_reason, routing_detail,
                        routing_tier, classifier_confidence, classifier_output,
                        reasoning_effort, user_input_preview,
                        started_at, updated_at, request_count, tool_round_count
                 FROM turn_route_logs
                 ORDER BY updated_at DESC, rowid DESC
                 LIMIT ?1",
            )
            .map_err(|err| format!("prepare turn route log list failed: {err}"))?;
        statement
            .query_map(params![limit], turn_route_log_from_row)
            .map_err(|err| format!("query turn route logs failed: {err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read turn route logs failed: {err}"))
    }

    pub fn list_turn_route_logs_for_prefix(
        &self,
        prefix: &str,
        limit: i64,
    ) -> Result<Vec<TurnRouteLog>, String> {
        let conn = self.connect()?;
        let mut statement = conn
            .prepare(
                "SELECT turn_id, provider_id, model, routing_mode, routing_reason, routing_detail,
                        routing_tier, classifier_confidence, classifier_output,
                        reasoning_effort, user_input_preview,
                        started_at, updated_at, request_count, tool_round_count
                 FROM turn_route_logs
                 WHERE turn_id LIKE ?1
                 ORDER BY updated_at DESC, rowid DESC
                 LIMIT ?2",
            )
            .map_err(|err| format!("prepare scoped turn route log list failed: {err}"))?;
        statement
            .query_map(
                params![format!("{prefix}%"), limit],
                turn_route_log_from_row,
            )
            .map_err(|err| format!("query scoped turn route logs failed: {err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read scoped turn route logs failed: {err}"))
    }

    pub fn record_turn_route_log(
        &self,
        update: &TurnRouteLogUpdate,
        limit: i64,
    ) -> Result<(), String> {
        let mut conn = self.connect()?;
        let transaction = conn
            .transaction()
            .map_err(|err| format!("begin turn route log transaction failed: {err}"))?;
        transaction
            .execute(
                "INSERT INTO turn_route_logs (
                    turn_id, provider_id, model, routing_mode, routing_reason, routing_detail,
                    routing_tier, classifier_confidence, classifier_output,
                    reasoning_effort, user_input_preview,
                    started_at, updated_at, request_count, tool_round_count
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, 1, ?13)
                 ON CONFLICT(turn_id) DO UPDATE SET
                    updated_at = excluded.updated_at,
                    request_count = turn_route_logs.request_count + 1,
                    tool_round_count = turn_route_logs.tool_round_count + excluded.tool_round_count,
                    reasoning_effort = COALESCE(excluded.reasoning_effort, turn_route_logs.reasoning_effort),
                    user_input_preview = COALESCE(turn_route_logs.user_input_preview, excluded.user_input_preview)",
                params![
                    update.turn_id,
                    update.provider_id,
                    update.model,
                    update.routing_mode,
                    update.routing_reason,
                    update.routing_detail,
                    update.routing_tier,
                    update.classifier_confidence,
                    update.classifier_output,
                    update.reasoning_effort,
                    update.user_input_preview,
                    update.timestamp,
                    i64::from(update.is_tool_round),
                ],
            )
            .map_err(|err| format!("upsert turn route log failed: {err}"))?;
        transaction
            .execute(
                "DELETE FROM turn_route_logs
                 WHERE turn_id IN (
                    SELECT turn_id FROM turn_route_logs
                    ORDER BY updated_at DESC, rowid DESC
                    LIMIT -1 OFFSET ?1
                 )",
                params![limit],
            )
            .map_err(|err| format!("trim turn route logs failed: {err}"))?;
        transaction
            .commit()
            .map_err(|err| format!("commit turn route log transaction failed: {err}"))
    }

    pub(crate) fn initialize_user_auth_schema(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch("
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
                last_seen_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS gateway_feishu_identities (
                user_id INTEGER PRIMARY KEY REFERENCES gateway_users(id) ON DELETE CASCADE,
                tenant_key TEXT NOT NULL DEFAULT '',
                open_id TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                avatar_url TEXT NOT NULL DEFAULT '',
                UNIQUE (tenant_key, open_id)
            );
            CREATE TABLE IF NOT EXISTS gateway_access_tokens (
                token_hash TEXT PRIMARY KEY,
                token_ciphertext TEXT NOT NULL,
                user_id INTEGER NOT NULL REFERENCES gateway_users(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_sessions_user_id ON gateway_sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_gateway_sessions_expires_at ON gateway_sessions(expires_at);
            CREATE INDEX IF NOT EXISTS idx_gateway_access_tokens_user_id ON gateway_access_tokens(user_id);
        ").map_err(|err| format!("initialize user auth schema failed: {err}"))?;
        match conn.execute(
            "ALTER TABLE gateway_users ADD COLUMN role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user'))",
            [],
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("duplicate column name") => {}
            Err(error) => return Err(format!("add gateway user role column failed: {error}")),
        }
        conn.execute(
            "UPDATE gateway_users
             SET role = 'admin'
             WHERE id = (SELECT id FROM gateway_users ORDER BY id ASC LIMIT 1)
               AND NOT EXISTS (SELECT 1 FROM gateway_users WHERE role = 'admin')",
            [],
        )
        .map_err(|error| format!("backfill initial gateway admin failed: {error}"))?;
        match conn.execute(
            "ALTER TABLE gateway_access_tokens ADD COLUMN token_ciphertext TEXT",
            [],
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("duplicate column name") => {}
            Err(error) => {
                return Err(format!(
                    "add gateway access token ciphertext column failed: {error}"
                ));
            }
        }
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
        .map_err(|error| {
            format!("create unique gateway access token owner index failed: {error}")
        })?;
        Ok(())
    }

    pub(crate) fn connect_for_auth(&self) -> Result<Connection, String> {
        self.connect()
    }

    pub(crate) fn database_security_settings(&self) -> Result<DatabaseSecuritySettings, String> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT COALESCE(database_encryption_key, ''), COALESCE(feishu_app_id, ''),
                    COALESCE(feishu_app_secret, ''), auth_required
             FROM gateway_state WHERE id = 1",
            [],
            |row| {
                Ok(DatabaseSecuritySettings {
                    encryption_key: row.get(0)?,
                    feishu_app_id: row.get(1)?,
                    feishu_app_secret: row.get(2)?,
                    auth_required: row.get::<_, i64>(3)? != 0,
                })
            },
        )
        .map_err(|err| format!("load database security settings failed: {err}"))
    }

    pub(crate) fn set_database_encryption_key(&self, key: &str) -> Result<(), String> {
        let key = key.trim();
        let new_encryptor = FieldEncryptor::from_base64_key(key)?;
        let mut conn = self.connect()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| format!("begin database encryption key transaction failed: {err}"))?;
        let current = database_security_settings_from(&transaction)?;
        rotate_database_encryption_key(&transaction, &current.encryption_key, key, &new_encryptor)?;
        transaction
            .commit()
            .map_err(|err| format!("commit database encryption key transaction failed: {err}"))
    }

    pub(crate) fn update_database_security_settings(
        &self,
        encryption_key: Option<&str>,
        feishu_app_id: &str,
        feishu_app_secret: Option<&str>,
        auth_required: bool,
    ) -> Result<(), String> {
        let requested_key = encryption_key.map(str::trim).filter(|key| !key.is_empty());
        let requested_encryptor = requested_key
            .map(FieldEncryptor::from_base64_key)
            .transpose()?;
        let mut conn = self.connect()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| format!("begin database security settings transaction failed: {err}"))?;
        let settings = database_security_settings_from(&transaction)?;
        let effective_key = requested_key.unwrap_or(&settings.encryption_key);
        let effective_encryptor = if effective_key.is_empty() {
            None
        } else {
            Some(match requested_encryptor.as_ref() {
                Some(encryptor) => encryptor.clone(),
                None => FieldEncryptor::from_base64_key(effective_key)?,
            })
        };

        if auth_required
            && (effective_key.is_empty()
                || feishu_app_id.trim().is_empty()
                || (settings.feishu_app_secret.is_empty()
                    && feishu_app_secret.is_none_or(|value| value.trim().is_empty())))
        {
            return Err(
                "启用账户登录前，必须先设置数据库加密密钥、飞书 App ID 和 App Secret".to_string(),
            );
        }
        let key_rotated = requested_key.is_some() && settings.encryption_key != effective_key;
        if let Some(new_encryptor) = requested_encryptor.as_ref() {
            rotate_database_encryption_key(
                &transaction,
                &settings.encryption_key,
                effective_key,
                new_encryptor,
            )?;
        }
        let app_secret = match feishu_app_secret.filter(|value| !value.trim().is_empty()) {
            Some(value) => effective_encryptor
                .as_ref()
                .ok_or_else(|| "尚未设置数据库加密密钥；请先在管理员设置中完成配置".to_string())?
                .encrypt(value)?,
            None if key_rotated => transaction
                .query_row(
                    "SELECT COALESCE(feishu_app_secret, '') FROM gateway_state WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|err| format!("load rotated Feishu App Secret failed: {err}"))?,
            None => settings.feishu_app_secret,
        };
        transaction
            .execute(
                "UPDATE gateway_state
             SET database_encryption_key = ?1, feishu_app_id = ?2, feishu_app_secret = ?3,
                 auth_required = ?4
             WHERE id = 1",
                params![
                    effective_key,
                    feishu_app_id.trim(),
                    app_secret,
                    i64::from(auth_required)
                ],
            )
            .map_err(|err| format!("save database security settings failed: {err}"))?;
        transaction
            .commit()
            .map_err(|err| format!("commit database security settings transaction failed: {err}"))
    }

    pub(crate) fn encryption(&self) -> Result<FieldEncryptor, String> {
        self.optional_encryption()?
            .ok_or_else(|| "尚未设置数据库加密密钥；请先在管理员设置中完成配置".to_string())
    }

    fn optional_encryption(&self) -> Result<Option<FieldEncryptor>, String> {
        let key = self.database_security_settings()?.encryption_key;
        (!key.is_empty())
            .then(|| FieldEncryptor::from_base64_key(&key))
            .transpose()
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS gateway_users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                email TEXT NOT NULL UNIQUE COLLATE NOCASE,
                name TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user'))
            );

            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                account_type TEXT NOT NULL,
                email TEXT NOT NULL,
                access_token TEXT NOT NULL,
                refresh_token TEXT NOT NULL,
                expiry_timestamp INTEGER NOT NULL,
                client_id TEXT,
                upstream_account_id TEXT,
                owner_user_id INTEGER REFERENCES gateway_users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_mode TEXT NOT NULL CHECK (auth_mode IN ('api_key', 'account')),
                base_url TEXT,
                api_key TEXT,
                account_id TEXT,
                upstream_protocol TEXT NOT NULL CHECK (
                    upstream_protocol = 'openai_responses'
                ),
                compatibility_profile TEXT NOT NULL CHECK (
                    compatibility_profile IN ('official_openai', 'generic_openai', 'openai_codex')
                ),
                preferred_model TEXT,
                preferred_reasoning_effort TEXT,
                owner_user_id INTEGER REFERENCES gateway_users(id) ON DELETE CASCADE,
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
                database_encryption_key TEXT NOT NULL DEFAULT '',
                feishu_app_id TEXT NOT NULL DEFAULT '',
                feishu_app_secret TEXT NOT NULL DEFAULT '',
                auth_required INTEGER NOT NULL DEFAULT 0,
                routing_enabled INTEGER NOT NULL DEFAULT 0,
                routing_classifier_target TEXT,
                routing_light_target TEXT,
                routing_standard_target TEXT,
                routing_pro_target TEXT,
                routing_max_target TEXT,
                routing_low_confidence_threshold REAL NOT NULL DEFAULT 0.7,
                route_updated_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (selected_provider_id) REFERENCES providers(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS gateway_instance_state (
                instance_id TEXT PRIMARY KEY,
                selected_provider_id TEXT,
                selected_model TEXT,
                selected_reasoning_effort TEXT,
                routing_enabled INTEGER NOT NULL DEFAULT 0,
                routing_classifier_target TEXT,
                routing_light_target TEXT,
                routing_standard_target TEXT,
                routing_pro_target TEXT,
                routing_max_target TEXT,
                routing_low_confidence_threshold REAL NOT NULL DEFAULT 0.7,
                route_updated_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (selected_provider_id) REFERENCES providers(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS provider_model_cache (
                provider_id TEXT PRIMARY KEY,
                models_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS turn_route_logs (
                turn_id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                routing_mode TEXT NOT NULL,
                routing_reason TEXT NOT NULL DEFAULT 'unknown',
                routing_detail TEXT,
                routing_tier TEXT,
                classifier_confidence REAL,
                classifier_output TEXT,
                reasoning_effort TEXT,
                user_input_preview TEXT,
                started_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                request_count INTEGER NOT NULL,
                tool_round_count INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_turn_route_logs_updated_at
                ON turn_route_logs(updated_at DESC);
            ",
        )
        .map_err(|err| format!("initialize sqlite schema failed: {err}"))?;
        add_column_if_missing(
            &conn,
            "accounts",
            "owner_user_id INTEGER REFERENCES gateway_users(id) ON DELETE CASCADE",
        )?;
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
            "gateway_state",
            "auth_required INTEGER NOT NULL DEFAULT 0",
        )?;
        let created_gateway_state = conn
            .execute("INSERT OR IGNORE INTO gateway_state (id) VALUES (1)", [])
            .map_err(|err| format!("initialize gateway state failed: {err}"))?;
        if created_gateway_state == 1 {
            let key = FieldEncryptor::generate_base64_key()?;
            conn.execute(
                "UPDATE gateway_state SET database_encryption_key = ?1 WHERE id = 1",
                params![key],
            )
            .map_err(|err| format!("initialize database encryption key failed: {err}"))?;
        }
        add_column_if_missing(
            &conn,
            "providers",
            "owner_user_id INTEGER REFERENCES gateway_users(id) ON DELETE CASCADE",
        )?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_accounts_owner_user_id ON accounts(owner_user_id);
             CREATE INDEX IF NOT EXISTS idx_providers_owner_user_id ON providers(owner_user_id);",
        )
        .map_err(|err| format!("create ownership indexes failed: {err}"))?;
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

fn database_security_settings_from(
    transaction: &Transaction<'_>,
) -> Result<DatabaseSecuritySettings, String> {
    transaction
        .query_row(
            "SELECT COALESCE(database_encryption_key, ''), COALESCE(feishu_app_id, ''),
                    COALESCE(feishu_app_secret, ''), auth_required
             FROM gateway_state WHERE id = 1",
            [],
            |row| {
                Ok(DatabaseSecuritySettings {
                    encryption_key: row.get(0)?,
                    feishu_app_id: row.get(1)?,
                    feishu_app_secret: row.get(2)?,
                    auth_required: row.get::<_, i64>(3)? != 0,
                })
            },
        )
        .map_err(|err| format!("load database security settings failed: {err}"))
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
            "accounts",
            "access_token",
            "rotate account access tokens",
            &current_encryptor,
            new_encryptor,
        )?;
        reencrypt_column(
            transaction,
            "accounts",
            "refresh_token",
            "rotate account refresh tokens",
            &current_encryptor,
            new_encryptor,
        )?;
        reencrypt_column(
            transaction,
            "providers",
            "api_key",
            "rotate provider API keys",
            &current_encryptor,
            new_encryptor,
        )?;
        reencrypt_column(
            transaction,
            "gateway_state",
            "feishu_app_secret",
            "rotate Feishu App Secret",
            &current_encryptor,
            new_encryptor,
        )?;
        let has_access_tokens: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'table' AND name = 'gateway_access_tokens'
                )",
                [],
                |row| row.get(0),
            )
            .map_err(|err| format!("check gateway access token table failed: {err}"))?;
        if has_access_tokens {
            reencrypt_column(
                transaction,
                "gateway_access_tokens",
                "token_ciphertext",
                "rotate gateway access tokens",
                &current_encryptor,
                new_encryptor,
            )?;
        }
    }
    transaction
        .execute(
            "UPDATE gateway_state SET database_encryption_key = ?1 WHERE id = 1",
            params![new_key],
        )
        .map_err(|err| format!("save database encryption key failed: {err}"))?;
    Ok(())
}

fn reencrypt_column(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    operation: &str,
    current_encryptor: &FieldEncryptor,
    new_encryptor: &FieldEncryptor,
) -> Result<(), String> {
    let select = format!(
        "SELECT rowid, {column} FROM {table} WHERE {column} IS NOT NULL AND {column} <> ''"
    );
    let values = {
        let mut statement = transaction
            .prepare(&select)
            .map_err(|err| format!("{operation}: prepare query failed: {err}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|err| format!("{operation}: query failed: {err}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("{operation}: read values failed: {err}"))?
    };
    let update = format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2");
    for (rowid, ciphertext) in values {
        let plaintext = current_encryptor
            .decrypt(&ciphertext)
            .map_err(|err| format!("{operation}: {err}"))?;
        let ciphertext = new_encryptor
            .encrypt(&plaintext)
            .map_err(|err| format!("{operation}: {err}"))?;
        transaction
            .execute(&update, params![ciphertext, rowid])
            .map_err(|err| format!("{operation}: save value failed: {err}"))?;
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

fn turn_route_log_from_row(row: &rusqlite::Row<'_>) -> Result<TurnRouteLog, rusqlite::Error> {
    Ok(TurnRouteLog {
        turn_id: row.get(0)?,
        provider_id: row.get(1)?,
        model: row.get(2)?,
        routing_mode: row.get(3)?,
        routing_reason: row.get(4)?,
        routing_detail: row.get(5)?,
        routing_tier: row.get(6)?,
        classifier_confidence: row.get(7)?,
        classifier_output: row.get(8)?,
        reasoning_effort: row.get(9)?,
        user_input_preview: row.get(10)?,
        started_at: row.get(11)?,
        updated_at: row.get(12)?,
        request_count: row.get(13)?,
        tool_round_count: row.get(14)?,
    })
}

fn routing_target_from_storage(
    stored_target: Option<String>,
) -> Result<Option<RoutingModelTarget>, Box<dyn std::error::Error + Send + Sync>> {
    stored_target
        .map(|target| serde_json::from_str(&target))
        .transpose()
        .map_err(|err| format!("decode routing target failed: {err}").into())
}

fn routing_target_to_storage(
    target: Option<&RoutingModelTarget>,
) -> Result<Option<String>, String> {
    target
        .map(serde_json::to_string)
        .transpose()
        .map_err(|err| format!("encode routing target failed: {err}"))
}

fn upsert_account_record(
    conn: &Connection,
    encryption: &FieldEncryptor,
    account: &AccountRecord,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO accounts (
            id, account_type, email, access_token, refresh_token, expiry_timestamp, client_id, upstream_account_id, owner_user_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            account_type = excluded.account_type,
            email = excluded.email,
            access_token = excluded.access_token,
            refresh_token = excluded.refresh_token,
            expiry_timestamp = excluded.expiry_timestamp,
            client_id = excluded.client_id,
            upstream_account_id = excluded.upstream_account_id,
            owner_user_id = excluded.owner_user_id",
        params![
            account.id,
            account_type_to_str(&account.account_type),
            account.email,
            encryption.encrypt(&account.access_token)?,
            encryption.encrypt(&account.refresh_token)?,
            account.expiry_timestamp,
            account.client_id,
            account.upstream_account_id,
            account.owner_user_id
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
            upstream_protocol, compatibility_profile, owner_user_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            auth_mode = excluded.auth_mode,
            base_url = excluded.base_url,
            api_key = excluded.api_key,
            account_id = excluded.account_id,
            upstream_protocol = excluded.upstream_protocol,
            compatibility_profile = excluded.compatibility_profile,
            owner_user_id = excluded.owner_user_id",
        params![
            provider.id,
            provider.name,
            provider_auth_mode_to_str(&provider.auth_mode),
            base_url,
            api_key,
            provider.account_id.as_deref(),
            upstream_protocol_to_str(&provider.upstream_protocol),
            compatibility_profile_to_str(&provider.compatibility_profile),
            provider.owner_user_id
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
    }
}

fn upstream_protocol_from_str(
    value: &str,
) -> Result<ProviderUpstreamProtocol, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "openai_responses" => Ok(ProviderUpstreamProtocol::OpenAiResponses),
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

fn decrypt_conversion_error(error: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
}

#[cfg(test)]
mod tests {
    use super::SqliteStore;
    use crate::{
        crypto::FieldEncryptor,
        models::{
            AccountRecord, AccountType, ApiProviderRecord, CachedProviderModels, ProviderAuthMode,
            ProviderCompatibilityProfile, ProviderUpstreamProtocol, SelectedRoute,
        },
    };
    use rusqlite::{Connection, params};
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
        let account = AccountRecord {
            id: "account-1".to_string(),
            account_type: AccountType::Openai,
            email: "account@example.com".to_string(),
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expiry_timestamp: 1,
            client_id: Some("client".to_string()),
            upstream_account_id: Some("upstream".to_string()),
            owner_user_id: None,
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
            owner_user_id: None,
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

    #[test]
    fn changing_database_encryption_key_reencrypts_all_credentials() {
        const NEW_KEY: &str = "ZmVkY2JhOTg3NjU0MzIxMGZlZGNiYTk4NzY1NDMyMTA=";

        let db_path = unique_test_db_path("rotate-encryption-key");
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
            owner_user_id: None,
        };
        store.upsert_account(&account).expect("save account");
        store
            .upsert_provider(&api_provider("provider-1"))
            .expect("save provider");
        store
            .initialize_user_auth_schema()
            .expect("initialize auth tables");
        store
            .update_database_security_settings(None, "cli_test", Some("app-secret"), false)
            .expect("save Feishu credentials");

        let old_encryptor = store.encryption().expect("load old encryptor");
        let conn = Connection::open(&db_path).expect("open encrypted database");
        conn.execute(
            "INSERT INTO gateway_users (email, name, password_hash) VALUES (?1, ?2, ?3)",
            params!["user@example.com", "User", ""],
        )
        .expect("create gateway user");
        conn.execute(
            "INSERT INTO gateway_access_tokens (token_hash, token_ciphertext, user_id)
             VALUES (?1, ?2, ?3)",
            params![
                "token-hash",
                old_encryptor
                    .encrypt("gateway-token")
                    .expect("encrypt token"),
                1
            ],
        )
        .expect("save gateway access token");
        let before: Vec<String> = conn
            .prepare(
                "SELECT access_token FROM accounts
                 UNION ALL SELECT refresh_token FROM accounts
                 UNION ALL SELECT api_key FROM providers
                 UNION ALL SELECT feishu_app_secret FROM gateway_state
                 UNION ALL SELECT token_ciphertext FROM gateway_access_tokens",
            )
            .expect("prepare ciphertext query")
            .query_map([], |row| row.get(0))
            .expect("query ciphertext")
            .collect::<Result<_, _>>()
            .expect("read ciphertext");

        store
            .update_database_security_settings(Some(NEW_KEY), "cli_test", None, false)
            .expect("rotate encryption key");

        let rotated_account = store.load_accounts().unwrap().remove(0);
        assert_eq!(rotated_account.access_token, account.access_token);
        assert_eq!(rotated_account.refresh_token, account.refresh_token);
        assert_eq!(store.load_providers().unwrap()[0].api_key, "sk-test");
        let settings = store
            .database_security_settings()
            .expect("load security settings");
        assert_eq!(settings.encryption_key, NEW_KEY);
        let new_encryptor = FieldEncryptor::from_base64_key(NEW_KEY).expect("new encryptor");
        assert_eq!(
            new_encryptor
                .decrypt(&settings.feishu_app_secret)
                .expect("decrypt Feishu secret"),
            "app-secret"
        );
        let token_ciphertext: String = conn
            .query_row(
                "SELECT token_ciphertext FROM gateway_access_tokens WHERE user_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("load gateway token");
        assert_eq!(
            new_encryptor
                .decrypt(&token_ciphertext)
                .expect("decrypt gateway token"),
            "gateway-token"
        );

        let after: Vec<String> = conn
            .prepare(
                "SELECT access_token FROM accounts
                 UNION ALL SELECT refresh_token FROM accounts
                 UNION ALL SELECT api_key FROM providers
                 UNION ALL SELECT feishu_app_secret FROM gateway_state
                 UNION ALL SELECT token_ciphertext FROM gateway_access_tokens",
            )
            .expect("prepare rotated ciphertext query")
            .query_map([], |row| row.get(0))
            .expect("query rotated ciphertext")
            .collect::<Result<_, _>>()
            .expect("read rotated ciphertext");
        assert_ne!(before, after);
        assert!(old_encryptor.decrypt(&after[0]).is_err());

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn initializes_new_databases_with_a_generated_encryption_key() {
        let db_path = unique_test_db_path("generated-encryption-key");
        let store = SqliteStore {
            db_path: db_path.clone(),
        };

        store.init().expect("initialize database");

        let settings = store
            .database_security_settings()
            .expect("load security settings");
        assert!(!settings.encryption_key.is_empty());
        assert!(FieldEncryptor::from_base64_key(&settings.encryption_key).is_ok());

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
