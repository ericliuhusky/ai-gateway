use crate::config::Config;
use rusqlite::{Connection, params};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const DEFAULT_MAX_LOG_ROWS: usize = 20_000;
const DEFAULT_PRUNE_TO_ROWS: usize = 18_000;
const DEFAULT_BODY_LIMIT_CHARS: usize = 16_000;
const DEFAULT_ERROR_LIMIT_CHARS: usize = 4_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogStage {
    IngressRequest,
    EgressRequest,
    IngressResponse,
    EgressResponse,
    Error,
}

impl LogStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IngressRequest => "ingress_request",
            Self::EgressRequest => "egress_request",
            Self::IngressResponse => "ingress_response",
            Self::EgressResponse => "egress_response",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogEvent {
    pub request_id: String,
    pub stage: LogStage,
    pub status_code: Option<u16>,
    pub ingress_protocol: Option<String>,
    pub egress_protocol: Option<String>,
    pub provider_name: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
    pub model: Option<String>,
    pub stream: bool,
    pub method: Option<String>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub body: Option<String>,
    pub error_message: Option<String>,
    pub elapsed_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct LogStore {
    db_path: PathBuf,
    max_rows: usize,
    prune_to_rows: usize,
    body_limit_chars: usize,
    error_limit_chars: usize,
    write_guard: Arc<Mutex<()>>,
}

impl LogStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Self::with_options(
            config.log_sqlite_path(),
            DEFAULT_MAX_LOG_ROWS,
            DEFAULT_PRUNE_TO_ROWS,
            DEFAULT_BODY_LIMIT_CHARS,
            DEFAULT_ERROR_LIMIT_CHARS,
        )
    }

    fn with_options(
        db_path: PathBuf,
        max_rows: usize,
        prune_to_rows: usize,
        body_limit_chars: usize,
        error_limit_chars: usize,
    ) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create log data dir failed: {err}"))?;
        }

        let store = Self {
            db_path,
            max_rows,
            prune_to_rows: prune_to_rows.min(max_rows),
            body_limit_chars,
            error_limit_chars,
            write_guard: Arc::new(Mutex::new(())),
        };
        store.init()?;
        Ok(store)
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    pub fn max_rows(&self) -> usize {
        self.max_rows
    }

    pub async fn record(&self, event: LogEvent) -> Result<(), String> {
        let _guard = self.write_guard.lock().await;
        let conn = self.connect()?;
        let (body, body_truncated) =
            truncate_optional(event.body.as_deref(), self.body_limit_chars);
        let (error_message, error_truncated) =
            truncate_optional(event.error_message.as_deref(), self.error_limit_chars);

        conn.execute(
            "INSERT INTO gateway_logs (
                request_id,
                stage,
                status_code,
                ingress_protocol,
                egress_protocol,
                provider_name,
                account_id,
                account_email,
                model,
                stream,
                method,
                path,
                url,
                body,
                body_truncated,
                error_message,
                error_truncated,
                elapsed_ms,
                created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                event.request_id,
                event.stage.as_str(),
                event.status_code.map(i64::from),
                event.ingress_protocol,
                event.egress_protocol,
                event.provider_name,
                event.account_id,
                event.account_email,
                event.model,
                if event.stream { 1_i64 } else { 0_i64 },
                event.method,
                event.path,
                event.url,
                body,
                if body_truncated { 1_i64 } else { 0_i64 },
                error_message,
                if error_truncated { 1_i64 } else { 0_i64 },
                event.elapsed_ms,
                now_unix() as i64,
            ],
        )
        .map_err(|err| format!("insert gateway log failed: {err}"))?;

        self.prune_if_needed(&conn)?;
        Ok(())
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS gateway_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id TEXT NOT NULL,
                stage TEXT NOT NULL,
                status_code INTEGER,
                ingress_protocol TEXT,
                egress_protocol TEXT,
                provider_name TEXT,
                account_id TEXT,
                account_email TEXT,
                model TEXT,
                stream INTEGER NOT NULL,
                method TEXT,
                path TEXT,
                url TEXT,
                body TEXT,
                body_truncated INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                error_truncated INTEGER NOT NULL DEFAULT 0,
                elapsed_ms INTEGER,
                created_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_gateway_logs_request_id
                ON gateway_logs (request_id);
            CREATE INDEX IF NOT EXISTS idx_gateway_logs_created_at
                ON gateway_logs (created_at);
            ",
        )
        .map_err(|err| format!("initialize log sqlite schema failed: {err}"))?;
        Ok(())
    }

    fn prune_if_needed(&self, conn: &Connection) -> Result<(), String> {
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM gateway_logs", [], |row| row.get(0))
            .map_err(|err| format!("count gateway logs failed: {err}"))?;

        if total <= self.max_rows as i64 {
            return Ok(());
        }

        let rows_to_delete = total - self.prune_to_rows as i64;
        conn.execute(
            "DELETE FROM gateway_logs
             WHERE id IN (
                SELECT id
                FROM gateway_logs
                ORDER BY id ASC
                LIMIT ?1
             )",
            params![rows_to_delete],
        )
        .map_err(|err| format!("prune gateway logs failed: {err}"))?;

        Ok(())
    }

    fn connect(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path).map_err(|err| {
            format!(
                "open log sqlite database {} failed: {err}",
                self.db_path.display()
            )
        })
    }
}

fn truncate_optional(value: Option<&str>, limit: usize) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };

    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(limit).collect();
    let was_truncated = chars.next().is_some();
    if was_truncated {
        (Some(format!("{truncated}...<truncated>")), true)
    } else {
        (Some(truncated), false)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{LogEvent, LogStage, LogStore};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[tokio::test]
    async fn prunes_oldest_rows_after_limit() {
        let db_path = unique_test_db_path("prune");
        let store =
            LogStore::with_options(db_path.clone(), 3, 2, 128, 128).expect("create log store");

        for idx in 0..4 {
            store
                .record(LogEvent {
                    request_id: format!("req_{idx}"),
                    stage: LogStage::IngressRequest,
                    status_code: None,
                    ingress_protocol: Some("openai-responses".to_string()),
                    egress_protocol: None,
                    provider_name: None,
                    account_id: None,
                    account_email: None,
                    model: Some("gpt-5.4".to_string()),
                    stream: false,
                    method: Some("POST".to_string()),
                    path: Some("/openai/v1/responses".to_string()),
                    url: None,
                    body: Some(format!("body-{idx}")),
                    error_message: None,
                    elapsed_ms: None,
                })
                .await
                .expect("insert log row");
        }

        let conn = rusqlite::Connection::open(&db_path).expect("open test db");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM gateway_logs", [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(count, 2);

        let first_remaining: String = conn
            .query_row(
                "SELECT request_id FROM gateway_logs ORDER BY id ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("load remaining request id");
        assert_eq!(first_remaining, "req_2");

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn truncates_large_bodies() {
        let db_path = unique_test_db_path("truncate");
        let store = LogStore::with_options(db_path.clone(), 10, 8, 8, 8).expect("create log store");

        store
            .record(LogEvent {
                request_id: "req_truncate".to_string(),
                stage: LogStage::IngressResponse,
                status_code: Some(200),
                ingress_protocol: Some("openai-responses".to_string()),
                egress_protocol: Some("native-responses".to_string()),
                provider_name: Some("demo".to_string()),
                account_id: None,
                account_email: None,
                model: Some("gpt-5.4".to_string()),
                stream: false,
                method: None,
                path: None,
                url: Some("https://example.com/v1/responses".to_string()),
                body: Some("abcdefghijklmnopqrstuvwxyz".to_string()),
                error_message: Some("1234567890".to_string()),
                elapsed_ms: Some(15),
            })
            .await
            .expect("insert truncated row");

        let conn = rusqlite::Connection::open(&db_path).expect("open test db");
        let (body, body_truncated, error_message, error_truncated): (String, i64, String, i64) =
            conn.query_row(
                "SELECT body, body_truncated, error_message, error_truncated
                 FROM gateway_logs
                 LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("load truncated row");

        assert_eq!(body, "abcdefgh...<truncated>");
        assert_eq!(body_truncated, 1);
        assert_eq!(error_message, "12345678...<truncated>");
        assert_eq!(error_truncated, 1);

        let _ = fs::remove_file(db_path);
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_{prefix}_{unique}.db"))
    }
}
