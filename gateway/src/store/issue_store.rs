use crate::{
    config::Config,
    models::{GatewayIssue, GatewayIssueRecord},
    store::sqlite::SqliteStore,
};
use std::sync::Arc;

pub const GATEWAY_ISSUE_LIMIT: i64 = 200;
pub const GATEWAY_ISSUE_BODY_LIMIT: usize = 128 * 1024;

#[derive(Clone, Debug)]
pub struct IssueStore {
    sqlite: SqliteStore,
}

impl IssueStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            sqlite: SqliteStore::new(config)?,
        })
    }

    /// Failure observability must never make the gateway less available.
    /// Callers intentionally log and ignore storage errors.
    pub fn record(&self, issue: &GatewayIssueRecord) -> Result<(), String> {
        self.sqlite.record_gateway_issue(issue, GATEWAY_ISSUE_LIMIT)
    }

    pub fn list(&self, limit: i64) -> Result<Vec<GatewayIssue>, String> {
        self.sqlite
            .list_gateway_issues(limit.clamp(1, GATEWAY_ISSUE_LIMIT))
    }

    pub fn get(&self, issue_id: &str) -> Result<Option<GatewayIssue>, String> {
        self.sqlite.load_gateway_issue(issue_id)
    }

    pub fn clear(&self) -> Result<usize, String> {
        self.sqlite.clear_gateway_issues()
    }
}

pub fn truncate_issue_body(value: &str) -> (String, bool) {
    if value.len() <= GATEWAY_ISSUE_BODY_LIMIT {
        return (value.to_string(), false);
    }

    let mut end = GATEWAY_ISSUE_BODY_LIMIT;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::{GATEWAY_ISSUE_BODY_LIMIT, IssueStore, truncate_issue_body};
    use crate::{models::GatewayIssueRecord, store::sqlite::SqliteStore};
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn truncates_issue_bodies_on_utf8_boundaries() {
        let value = "故".repeat(GATEWAY_ISSUE_BODY_LIMIT);
        let (truncated, was_truncated) = truncate_issue_body(&value);

        assert!(was_truncated);
        assert!(truncated.len() <= GATEWAY_ISSUE_BODY_LIMIT);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn lists_and_clears_all_issues() {
        let store = IssueStore {
            sqlite: SqliteStore::for_test(unique_test_db_path("issues")).unwrap(),
        };
        store.record(&issue("issue_a", 10)).unwrap();
        store.record(&issue("issue_b", 20)).unwrap();

        let issues = store.list(50).unwrap();
        assert_eq!(issues.len(), 2);

        assert_eq!(store.clear().unwrap(), 2);
        assert!(store.list(50).unwrap().is_empty());
    }

    fn issue(id: &str, created_at: i64) -> GatewayIssueRecord {
        GatewayIssueRecord {
            id: id.to_string(),
            provider_id: "provider".to_string(),
            provider_name: "Provider".to_string(),
            model: "model".to_string(),
            upstream_url: "https://example.com/v1/responses".to_string(),
            failure_kind: "upstream_http_error".to_string(),
            status_code: Some(500),
            error_message: "failed".to_string(),
            upstream_response: "{\"error\":\"failed\"}".to_string(),
            upstream_response_truncated: false,
            created_at,
        }
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.sqlite"))
    }
}
