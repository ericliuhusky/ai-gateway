use crate::{
    config::Config,
    models::{GatewayLogDetail, GatewayLogSummary},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const DEFAULT_MAX_LOG_ROWS: usize = 20_000;
const DEFAULT_PRUNE_TO_ROWS: usize = 18_000;
const DEFAULT_ERROR_LIMIT_CHARS: usize = 4_000;
const DEFAULT_BODY_LIMIT_CHARS: usize = 200_000;

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
    pub id: String,
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
    error_limit_chars: usize,
    body_limit_chars: usize,
    write_guard: Arc<Mutex<()>>,
    enabled: Arc<AtomicBool>,
}

impl LogStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Self::with_options(
            config.log_sqlite_path(),
            DEFAULT_MAX_LOG_ROWS,
            DEFAULT_PRUNE_TO_ROWS,
            DEFAULT_ERROR_LIMIT_CHARS,
            DEFAULT_BODY_LIMIT_CHARS,
        )
    }

    fn with_options(
        db_path: PathBuf,
        max_rows: usize,
        prune_to_rows: usize,
        error_limit_chars: usize,
        body_limit_chars: usize,
    ) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create log data dir failed: {err}"))?;
        }

        let store = Self {
            db_path,
            max_rows,
            prune_to_rows: prune_to_rows.min(max_rows),
            error_limit_chars,
            body_limit_chars,
            write_guard: Arc::new(Mutex::new(())),
            enabled: Arc::new(AtomicBool::new(true)),
        };
        store.init()?;
        store
            .enabled
            .store(store.load_enabled_setting()?, Ordering::Relaxed);
        Ok(store)
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    pub fn max_rows(&self) -> usize {
        self.max_rows
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub async fn record(&self, event: LogEvent) -> Result<(), String> {
        if !self.is_enabled() {
            return Ok(());
        }

        let _guard = self.write_guard.lock().await;
        if !self.is_enabled() {
            return Ok(());
        }

        let conn = self.connect()?;
        let created_at = now_unix() as i64;
        let (body, body_truncated) = truncate_optional(event.body.as_deref(), self.body_limit_chars);
        let (error_message, error_truncated) =
            truncate_optional(event.error_message.as_deref(), self.error_limit_chars);

        conn.execute(
            "INSERT INTO gateway_logs (
                id,
                created_at,
                updated_at,
                provider_name,
                account_id,
                account_email,
                model,
                stream,
                ingress_protocol,
                egress_protocol
            ) VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO NOTHING",
            params![
                event.id,
                created_at,
                event.provider_name,
                event.account_id,
                event.account_email,
                event.model,
                if event.stream { 1_i64 } else { 0_i64 },
                event.ingress_protocol,
                event.egress_protocol,
            ],
        )
        .map_err(|err| format!("insert gateway log failed: {err}"))?;

        let update_sql = match event.stage {
            LogStage::IngressRequest => {
                "UPDATE gateway_logs
                 SET updated_at = ?2,
                     provider_name = COALESCE(?3, provider_name),
                     account_id = COALESCE(?4, account_id),
                     account_email = COALESCE(?5, account_email),
                     model = COALESCE(?6, model),
                     stream = ?7,
                     ingress_protocol = COALESCE(?8, ingress_protocol),
                     method = COALESCE(?9, method),
                     path = COALESCE(?10, path),
                     ingress_request_body = COALESCE(?11, ingress_request_body),
                     ingress_request_body_truncated = ?12,
                     error_message = COALESCE(?13, error_message),
                     error_truncated = ?14
                 WHERE id = ?1"
            }
            LogStage::EgressRequest => {
                "UPDATE gateway_logs
                 SET updated_at = ?2,
                     provider_name = COALESCE(?3, provider_name),
                     account_id = COALESCE(?4, account_id),
                     account_email = COALESCE(?5, account_email),
                     model = COALESCE(?6, model),
                     stream = ?7,
                     ingress_protocol = COALESCE(?8, ingress_protocol),
                     egress_protocol = COALESCE(?15, egress_protocol),
                     method = COALESCE(?9, method),
                     path = COALESCE(?10, path),
                     egress_request_url = COALESCE(?16, egress_request_url),
                     egress_request_body = COALESCE(?11, egress_request_body),
                     egress_request_body_truncated = ?12,
                     error_message = COALESCE(?13, error_message),
                     error_truncated = ?14
                 WHERE id = ?1"
            }
            LogStage::IngressResponse => {
                "UPDATE gateway_logs
                 SET updated_at = ?2,
                     provider_name = COALESCE(?3, provider_name),
                     account_id = COALESCE(?4, account_id),
                     account_email = COALESCE(?5, account_email),
                     model = COALESCE(?6, model),
                     stream = ?7,
                     ingress_protocol = COALESCE(?8, ingress_protocol),
                     egress_protocol = COALESCE(?15, egress_protocol),
                     egress_request_url = COALESCE(?16, egress_request_url),
                     ingress_response_status_code = COALESCE(?17, ingress_response_status_code),
                     ingress_response_body = COALESCE(?11, ingress_response_body),
                     ingress_response_body_truncated = ?12,
                     elapsed_ms = COALESCE(?18, elapsed_ms),
                     error_message = COALESCE(?13, error_message),
                     error_truncated = ?14
                 WHERE id = ?1"
            }
            LogStage::EgressResponse => {
                "UPDATE gateway_logs
                 SET updated_at = ?2,
                     provider_name = COALESCE(?3, provider_name),
                     account_id = COALESCE(?4, account_id),
                     account_email = COALESCE(?5, account_email),
                     model = COALESCE(?6, model),
                     stream = ?7,
                     ingress_protocol = COALESCE(?8, ingress_protocol),
                     egress_protocol = COALESCE(?15, egress_protocol),
                     method = COALESCE(?9, method),
                     path = COALESCE(?10, path),
                     egress_response_status_code = COALESCE(?17, egress_response_status_code),
                     egress_response_body = COALESCE(?11, egress_response_body),
                     egress_response_body_truncated = ?12,
                     elapsed_ms = COALESCE(?18, elapsed_ms),
                     error_message = COALESCE(?13, error_message),
                     error_truncated = ?14
                 WHERE id = ?1"
            }
            LogStage::Error => {
                "UPDATE gateway_logs
                 SET updated_at = ?2,
                     provider_name = COALESCE(?3, provider_name),
                     account_id = COALESCE(?4, account_id),
                     account_email = COALESCE(?5, account_email),
                     model = COALESCE(?6, model),
                     stream = ?7,
                     ingress_protocol = COALESCE(?8, ingress_protocol),
                     egress_protocol = COALESCE(?15, egress_protocol),
                     method = COALESCE(?9, method),
                     path = COALESCE(?10, path),
                     egress_request_url = COALESCE(?16, egress_request_url),
                     error_message = COALESCE(?13, error_message),
                     error_truncated = ?14,
                     elapsed_ms = COALESCE(?18, elapsed_ms)
                 WHERE id = ?1"
            }
        };

        conn.execute(
            update_sql,
            params![
                event.id,
                created_at,
                event.provider_name,
                event.account_id,
                event.account_email,
                event.model,
                if event.stream { 1_i64 } else { 0_i64 },
                event.ingress_protocol,
                event.method,
                event.path,
                body,
                if body_truncated { 1_i64 } else { 0_i64 },
                error_message,
                if error_truncated { 1_i64 } else { 0_i64 },
                event.egress_protocol,
                event.url,
                event.status_code.map(i64::from),
                event.elapsed_ms,
            ],
        )
        .map_err(|err| format!("update gateway log failed: {err}"))?;

        self.prune_if_needed(&conn)?;
        Ok(())
    }

    pub fn list_request_summaries(&self, limit: usize) -> Result<Vec<GatewayLogSummary>, String> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "
                SELECT
                    id,
                    created_at,
                    updated_at,
                    provider_name,
                    account_email,
                    model,
                    stream,
                    COALESCE(egress_response_status_code, ingress_response_status_code) AS status_code,
                    CASE WHEN error_message IS NOT NULL THEN 1 ELSE 0 END AS has_error,
                    error_message,
                    ingress_protocol,
                    egress_protocol
                FROM gateway_logs
                ORDER BY updated_at DESC
                LIMIT ?1
                ",
            )
            .map_err(|err| format!("prepare log summaries query failed: {err}"))?;

        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(GatewayLogSummary {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    provider_name: row.get(3)?,
                    account_email: row.get(4)?,
                    model: row.get(5)?,
                    stream: row.get::<_, i64>(6)? != 0,
                    status_code: row.get::<_, Option<i64>>(7)?.map(|value| value as u16),
                    has_error: row.get::<_, i64>(8)? != 0,
                    error_message: row.get(9)?,
                    ingress_protocol: row.get(10)?,
                    egress_protocol: row.get(11)?,
                })
            })
            .map_err(|err| format!("query log summaries failed: {err}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read log summaries failed: {err}"))
    }

    pub fn load_request(&self, id: &str) -> Result<Option<GatewayLogDetail>, String> {
        let conn = self.connect()?;
        conn.query_row(
            "
            SELECT
                id,
                created_at,
                updated_at,
                provider_name,
                account_id,
                account_email,
                model,
                stream,
                ingress_protocol,
                egress_protocol,
                method,
                path,
                egress_request_url,
                ingress_request_body,
                ingress_request_body_truncated,
                egress_request_body,
                egress_request_body_truncated,
                ingress_response_status_code,
                ingress_response_body,
                ingress_response_body_truncated,
                egress_response_status_code,
                egress_response_body,
                egress_response_body_truncated,
                error_message,
                error_truncated,
                elapsed_ms
            FROM gateway_logs
            WHERE id = ?1
            ",
            params![id],
            |row| {
                Ok(GatewayLogDetail {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    provider_name: row.get(3)?,
                    account_id: row.get(4)?,
                    account_email: row.get(5)?,
                    model: row.get(6)?,
                    stream: row.get::<_, i64>(7)? != 0,
                    ingress_protocol: row.get(8)?,
                    egress_protocol: row.get(9)?,
                    method: row.get(10)?,
                    path: row.get(11)?,
                    egress_request_url: row.get(12)?,
                    ingress_request_body: row.get(13)?,
                    ingress_request_body_truncated: row.get::<_, i64>(14)? != 0,
                    egress_request_body: row.get(15)?,
                    egress_request_body_truncated: row.get::<_, i64>(16)? != 0,
                    ingress_response_status_code: row.get::<_, Option<i64>>(17)?.map(|value| value as u16),
                    ingress_response_body: row.get(18)?,
                    ingress_response_body_truncated: row.get::<_, i64>(19)? != 0,
                    egress_response_status_code: row.get::<_, Option<i64>>(20)?.map(|value| value as u16),
                    egress_response_body: row.get(21)?,
                    egress_response_body_truncated: row.get::<_, i64>(22)? != 0,
                    error_message: row.get(23)?,
                    error_truncated: row.get::<_, i64>(24)? != 0,
                    elapsed_ms: row.get(25)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("read request log failed: {err}"))
    }

    pub async fn set_enabled(&self, enabled: bool) -> Result<bool, String> {
        let _guard = self.write_guard.lock().await;
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO log_settings (id, enabled)
             VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled",
            params![if enabled { 1_i64 } else { 0_i64 }],
        )
        .map_err(|err| format!("update log enabled setting failed: {err}"))?;
        self.enabled.store(enabled, Ordering::Relaxed);
        Ok(enabled)
    }

    pub async fn clear(&self) -> Result<(), String> {
        let _guard = self.write_guard.lock().await;
        let conn = self.connect()?;
        conn.execute("DELETE FROM gateway_logs", [])
            .map_err(|err| format!("clear gateway logs failed: {err}"))?;
        Ok(())
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS log_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                enabled INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS gateway_logs (
                id TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                provider_name TEXT,
                account_id TEXT,
                account_email TEXT,
                model TEXT,
                stream INTEGER NOT NULL DEFAULT 0,
                ingress_protocol TEXT,
                egress_protocol TEXT,
                method TEXT,
                path TEXT,
                egress_request_url TEXT,
                ingress_request_body TEXT,
                ingress_request_body_truncated INTEGER NOT NULL DEFAULT 0,
                egress_request_body TEXT,
                egress_request_body_truncated INTEGER NOT NULL DEFAULT 0,
                ingress_response_status_code INTEGER,
                ingress_response_body TEXT,
                ingress_response_body_truncated INTEGER NOT NULL DEFAULT 0,
                egress_response_status_code INTEGER,
                egress_response_body TEXT,
                egress_response_body_truncated INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                error_truncated INTEGER NOT NULL DEFAULT 0,
                elapsed_ms INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_gateway_logs_updated_at
                ON gateway_logs (updated_at);
            ",
        )
        .map_err(|err| format!("initialize log sqlite schema failed: {err}"))?;
        conn.execute(
            "INSERT INTO log_settings (id, enabled)
             VALUES (1, 1)
             ON CONFLICT(id) DO NOTHING",
            [],
        )
        .map_err(|err| format!("initialize log setting failed: {err}"))?;
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
                ORDER BY updated_at ASC
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

    fn load_enabled_setting(&self) -> Result<bool, String> {
        let conn = self.connect()?;
        let enabled = conn
            .query_row("SELECT enabled FROM log_settings WHERE id = 1", [], |row| {
                row.get::<_, i64>(0)
            })
            .optional()
            .map_err(|err| format!("load log enabled setting failed: {err}"))?
            .unwrap_or(1);
        Ok(enabled != 0)
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
        let store = LogStore::with_options(db_path.clone(), 3, 2, 128, 1024).expect("create log store");

        for idx in 0..4 {
            store
                .record(LogEvent {
                    id: idx.to_string(),
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
                "SELECT id FROM gateway_logs ORDER BY updated_at ASC, id ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("load remaining id");
        assert_eq!(first_remaining, "2");

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn merges_request_and_response_into_single_row() {
        let db_path = unique_test_db_path("merge");
        let store = LogStore::with_options(db_path.clone(), 10, 8, 8, 1024).expect("create log store");

        store
            .record(LogEvent {
                id: "same".to_string(),
                stage: LogStage::IngressRequest,
                status_code: None,
                ingress_protocol: Some("openai-responses".to_string()),
                egress_protocol: None,
                provider_name: Some("demo".to_string()),
                account_id: None,
                account_email: Some("demo@example.com".to_string()),
                model: Some("gpt-5.4".to_string()),
                stream: false,
                method: Some("POST".to_string()),
                path: Some("/openai/v1/responses".to_string()),
                url: None,
                body: Some("request".to_string()),
                error_message: None,
                elapsed_ms: None,
            })
            .await
            .expect("insert request");

        store
            .record(LogEvent {
                id: "same".to_string(),
                stage: LogStage::EgressResponse,
                status_code: Some(200),
                ingress_protocol: Some("openai-responses".to_string()),
                egress_protocol: None,
                provider_name: Some("demo".to_string()),
                account_id: None,
                account_email: Some("demo@example.com".to_string()),
                model: Some("gpt-5.4".to_string()),
                stream: false,
                method: Some("POST".to_string()),
                path: Some("/openai/v1/responses".to_string()),
                url: None,
                body: Some("response".to_string()),
                error_message: None,
                elapsed_ms: Some(12),
            })
            .await
            .expect("insert response");

        let conn = rusqlite::Connection::open(&db_path).expect("open test db");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM gateway_logs", [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(count, 1);

        let (request_body, response_body, status_code): (String, String, i64) = conn
            .query_row(
                "SELECT ingress_request_body, egress_response_body, egress_response_status_code
                 FROM gateway_logs WHERE id = 'same'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("load merged row");
        assert_eq!(request_body, "request");
        assert_eq!(response_body, "response");
        assert_eq!(status_code, 200);

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn truncates_error_message() {
        let db_path = unique_test_db_path("truncate");
        let store = LogStore::with_options(db_path.clone(), 10, 8, 8, 1024).expect("create log store");

        store
            .record(LogEvent {
                id: "truncate".to_string(),
                stage: LogStage::EgressResponse,
                status_code: Some(400),
                ingress_protocol: Some("openai-responses".to_string()),
                egress_protocol: None,
                provider_name: Some("demo".to_string()),
                account_id: None,
                account_email: None,
                model: Some("gpt-5.4".to_string()),
                stream: false,
                method: None,
                path: None,
                url: None,
                body: Some("abcdefghijklmnopqrstuvwxyz".to_string()),
                error_message: Some("1234567890".to_string()),
                elapsed_ms: Some(15),
            })
            .await
            .expect("insert row");

        let conn = rusqlite::Connection::open(&db_path).expect("open test db");
        let (body, body_truncated, error_message, error_truncated): (String, i64, String, i64) =
            conn.query_row(
                "SELECT egress_response_body, egress_response_body_truncated, error_message, error_truncated
                 FROM gateway_logs
                 LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("load row");

        assert_eq!(body, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(body_truncated, 0);
        assert_eq!(error_message, "12345678...<truncated>");
        assert_eq!(error_truncated, 1);

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn skips_recording_when_disabled() {
        let db_path = unique_test_db_path("disabled");
        let store = LogStore::with_options(db_path.clone(), 10, 8, 128, 1024).expect("create log store");

        store.set_enabled(false).await.expect("disable log store");
        store
            .record(LogEvent {
                id: "disabled".to_string(),
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
                body: Some("body".to_string()),
                error_message: None,
                elapsed_ms: None,
            })
            .await
            .expect("skip write while disabled");

        let conn = rusqlite::Connection::open(&db_path).expect("open test db");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM gateway_logs", [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(count, 0);

        let enabled: i64 = conn
            .query_row("SELECT enabled FROM log_settings WHERE id = 1", [], |row| row.get(0))
            .expect("load enabled flag");
        assert_eq!(enabled, 0);

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn clears_logs_without_changing_enabled_setting() {
        let db_path = unique_test_db_path("clear");
        let store = LogStore::with_options(db_path.clone(), 10, 8, 128, 1024).expect("create log store");

        store
            .record(LogEvent {
                id: "clear".to_string(),
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
                body: Some("body".to_string()),
                error_message: None,
                elapsed_ms: None,
            })
            .await
            .expect("insert log row");

        store.clear().await.expect("clear logs");

        let conn = rusqlite::Connection::open(&db_path).expect("open test db");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM gateway_logs", [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(count, 0);

        let enabled: i64 = conn
            .query_row("SELECT enabled FROM log_settings WHERE id = 1", [], |row| row.get(0))
            .expect("load enabled flag");
        assert_eq!(enabled, 1);

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
