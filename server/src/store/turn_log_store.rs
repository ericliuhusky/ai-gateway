use crate::{
    config::Config,
    models::{TurnRouteLog, TurnRouteLogUpdate},
    store::sqlite::SqliteStore,
};
use std::sync::Arc;

pub const TURN_LOG_LIMIT: i64 = 1_000;

#[derive(Clone, Debug)]
pub struct TurnLogStore {
    sqlite: SqliteStore,
}

impl TurnLogStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    /// Observability must not affect inference availability, so callers intentionally
    /// treat storage failures as non-fatal.
    pub fn record(&self, update: &TurnRouteLogUpdate) -> Result<(), String> {
        self.sqlite.record_turn_route_log(update, TURN_LOG_LIMIT)
    }

    pub fn record_for_owner(
        &self,
        owner_user_id: Option<i64>,
        update: &TurnRouteLogUpdate,
    ) -> Result<(), String> {
        let mut update = update.clone();
        update.turn_id = scoped_turn_id(owner_user_id, &update.turn_id);
        self.sqlite.record_turn_route_log(&update, TURN_LOG_LIMIT)
    }

    pub fn get(&self, turn_id: &str) -> Result<Option<TurnRouteLog>, String> {
        self.sqlite.load_turn_route_log(turn_id)
    }

    pub fn get_for_owner(
        &self,
        owner_user_id: Option<i64>,
        turn_id: &str,
    ) -> Result<Option<TurnRouteLog>, String> {
        self.sqlite
            .load_turn_route_log(&scoped_turn_id(owner_user_id, turn_id))
    }

    pub fn list(&self, limit: i64) -> Result<Vec<TurnRouteLog>, String> {
        self.sqlite
            .list_turn_route_logs(limit.clamp(1, TURN_LOG_LIMIT))
    }

    pub fn list_for_owner(
        &self,
        owner_user_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<TurnRouteLog>, String> {
        let limit = limit.clamp(1, TURN_LOG_LIMIT);
        let Some(owner_user_id) = owner_user_id else {
            return self.list(limit);
        };
        let prefix = turn_prefix(owner_user_id);
        self.sqlite
            .list_turn_route_logs_for_prefix(&prefix, limit)
            .map(|turns| {
                turns
                    .into_iter()
                    .map(|mut turn| {
                        turn.turn_id = turn
                            .turn_id
                            .strip_prefix(&prefix)
                            .unwrap_or(&turn.turn_id)
                            .to_string();
                        turn
                    })
                    .collect()
            })
    }
}

fn turn_prefix(owner_user_id: i64) -> String {
    format!("__user_{owner_user_id}__")
}

fn scoped_turn_id(owner_user_id: Option<i64>, turn_id: &str) -> String {
    owner_user_id
        .map(|owner_user_id| format!("{}{}", turn_prefix(owner_user_id), turn_id))
        .unwrap_or_else(|| turn_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::TurnLogStore;
    use crate::{models::TurnRouteLogUpdate, store::sqlite::SqliteStore};
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn keeps_one_row_per_turn_and_counts_tool_rounds() {
        let store = TurnLogStore {
            sqlite: SqliteStore::for_test(unique_test_db_path("turn-log")).unwrap(),
        };
        let first = TurnRouteLogUpdate {
            turn_id: "turn_a".to_string(),
            provider_id: "provider_a".to_string(),
            model: "light".to_string(),
            routing_mode: "classifier".to_string(),
            routing_reason: "classifier_selected".to_string(),
            routing_detail: None,
            routing_tier: Some("light".to_string()),
            classifier_confidence: Some(0.9),
            classifier_output: Some("{\"tier\":\"light\",\"confidence\":0.9}".to_string()),
            reasoning_effort: Some("low".to_string()),
            user_input_preview: Some("hello".to_string()),
            is_tool_round: false,
            timestamp: 10,
        };
        store.record(&first).unwrap();
        store
            .record(&TurnRouteLogUpdate {
                is_tool_round: true,
                timestamp: 20,
                ..first.clone()
            })
            .unwrap();

        let log = store.get("turn_a").unwrap().unwrap();
        assert_eq!(log.model, "light");
        assert_eq!(log.request_count, 2);
        assert_eq!(log.tool_round_count, 1);
        assert_eq!(log.started_at, 10);
        assert_eq!(log.updated_at, 20);
    }

    #[test]
    fn evicts_least_recently_used_turns_above_the_limit() {
        let store = TurnLogStore {
            sqlite: SqliteStore::for_test(unique_test_db_path("turn-log-lru")).unwrap(),
        };
        for timestamp in 1..=3 {
            store
                .sqlite
                .record_turn_route_log(
                    &TurnRouteLogUpdate {
                        turn_id: format!("turn_{timestamp}"),
                        provider_id: "provider_a".to_string(),
                        model: "light".to_string(),
                        routing_mode: "classifier".to_string(),
                        routing_reason: "classifier_selected".to_string(),
                        routing_detail: None,
                        routing_tier: Some("light".to_string()),
                        classifier_confidence: Some(0.9),
                        classifier_output: None,
                        reasoning_effort: None,
                        user_input_preview: None,
                        is_tool_round: false,
                        timestamp,
                    },
                    2,
                )
                .unwrap();
        }

        assert!(store.get("turn_1").unwrap().is_none());
        assert_eq!(
            store
                .list(2)
                .unwrap()
                .into_iter()
                .map(|turn| turn.turn_id)
                .collect::<Vec<_>>(),
            vec!["turn_3", "turn_2"]
        );
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
