use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const SOURCE_PROVIDER: &str = "openai";
const TARGET_PROVIDER: &str = "ai-gateway";
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AliasPatch {
    aliases: BTreeMap<String, SessionAlias>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionAlias {
    alias_id: String,
    alias_rollout_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionAliasSummary {
    pub state_path: String,
    pub patch_path: String,
    pub source_provider: String,
    pub target_provider: String,
    pub source_threads: usize,
    pub created: usize,
    pub existing: usize,
    pub skipped_missing_rollout: usize,
}

#[derive(Debug)]
struct SourceThread {
    id: String,
    rollout_path: PathBuf,
    archived: bool,
}

pub fn sync_openai_history_aliases(
    codex_dir: &Path,
    state_path: &Path,
    patch_path: &Path,
) -> Result<SessionAliasSummary, String> {
    let mut patch = read_alias_patch(patch_path)?;
    if !state_path.exists() {
        return Err(format!(
            "Codex state database does not exist: {}",
            state_path.display()
        ));
    }

    let conn = Connection::open(state_path).map_err(|err| {
        format!(
            "failed to open Codex state database {}: {err}",
            state_path.display()
        )
    })?;
    let columns = thread_columns(&conn)?;
    let source_threads = load_source_threads(&conn)?;

    let mut created = 0;
    let mut existing = 0;
    let mut skipped_missing_rollout = 0;

    for thread in &source_threads {
        if alias_exists(&conn, &patch, &thread.id)? {
            existing += 1;
            continue;
        }

        if !thread.rollout_path.is_file() {
            skipped_missing_rollout += 1;
            continue;
        }

        let alias_id = Uuid::new_v4().to_string();
        let alias_rollout_path =
            alias_rollout_path(codex_dir, &thread.rollout_path, &alias_id, thread.archived)?;

        rewrite_rollout_file(
            &thread.rollout_path,
            &alias_rollout_path,
            &thread.id,
            &alias_id,
        )?;
        insert_thread_alias(&conn, &columns, &thread.id, &alias_id, &alias_rollout_path)?;

        patch.aliases.insert(
            thread.id.clone(),
            SessionAlias {
                alias_id,
                alias_rollout_path,
            },
        );
        created += 1;
    }

    write_alias_patch(patch_path, &patch)?;

    Ok(SessionAliasSummary {
        state_path: state_path.display().to_string(),
        patch_path: patch_path.display().to_string(),
        source_provider: SOURCE_PROVIDER.to_string(),
        target_provider: TARGET_PROVIDER.to_string(),
        source_threads: source_threads.len(),
        created,
        existing,
        skipped_missing_rollout,
    })
}

fn read_alias_patch(patch_path: &Path) -> Result<AliasPatch, String> {
    match fs::read_to_string(patch_path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|err| format!("failed to parse session alias patch: {err}")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(AliasPatch::default()),
        Err(err) => Err(format!(
            "failed to read session alias patch {}: {err}",
            patch_path.display()
        )),
    }
}

fn write_alias_patch(patch_path: &Path, patch: &AliasPatch) -> Result<(), String> {
    if let Some(parent) = patch_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create alias patch dir: {err}"))?;
    }
    let content = serde_json::to_string_pretty(patch)
        .map_err(|err| format!("failed to serialize session alias patch: {err}"))?;
    fs::write(patch_path, content).map_err(|err| {
        format!(
            "failed to write session alias patch {}: {err}",
            patch_path.display()
        )
    })
}

fn load_source_threads(conn: &Connection) -> Result<Vec<SourceThread>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, rollout_path, archived
             FROM threads
             WHERE model_provider = ?1
             ORDER BY created_at_ms ASC, created_at ASC, id ASC",
        )
        .map_err(|err| format!("prepare source thread query failed: {err}"))?;
    let rows = stmt
        .query_map(params![SOURCE_PROVIDER], |row| {
            Ok(SourceThread {
                id: row.get(0)?,
                rollout_path: PathBuf::from(row.get::<_, String>(1)?),
                archived: row.get::<_, i64>(2)? != 0,
            })
        })
        .map_err(|err| format!("query source threads failed: {err}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("read source threads failed: {err}"))
}

fn alias_exists(conn: &Connection, patch: &AliasPatch, source_id: &str) -> Result<bool, String> {
    if let Some(alias) = patch.aliases.get(source_id) {
        let exists = conn
            .query_row(
                "SELECT 1 FROM threads WHERE id = ?1 AND model_provider = ?2",
                params![alias.alias_id, TARGET_PROVIDER],
                |_| Ok(()),
            )
            .optional()
            .map_err(|err| format!("check existing alias failed: {err}"))?
            .is_some();
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}

fn alias_rollout_path(
    codex_dir: &Path,
    source_path: &Path,
    alias_id: &str,
    archived: bool,
) -> Result<PathBuf, String> {
    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid rollout filename: {}", source_path.display()))?;
    let alias_file_name = replace_uuid_in_filename(file_name, alias_id);
    let root = if archived {
        codex_dir.join("archived_sessions")
    } else {
        codex_dir.join("sessions")
    };
    let (year, month, day) = rollout_date_parts(&alias_file_name)?;
    Ok(root.join(year).join(month).join(day).join(alias_file_name))
}

fn rollout_date_parts(file_name: &str) -> Result<(&str, &str, &str), String> {
    let date = file_name
        .strip_prefix("rollout-")
        .and_then(|name| name.get(..10))
        .ok_or_else(|| format!("invalid rollout filename: {file_name}"))?;
    let year = date
        .get(..4)
        .ok_or_else(|| format!("invalid rollout date in filename: {file_name}"))?;
    let month = date
        .get(5..7)
        .ok_or_else(|| format!("invalid rollout date in filename: {file_name}"))?;
    let day = date
        .get(8..10)
        .ok_or_else(|| format!("invalid rollout date in filename: {file_name}"))?;
    Ok((year, month, day))
}

fn replace_uuid_in_filename(file_name: &str, alias_id: &str) -> String {
    let suffix = if file_name.ends_with(".jsonl") {
        ".jsonl"
    } else {
        ""
    };
    let stem = file_name.strip_suffix(suffix).unwrap_or(file_name);
    let mut parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 6 {
        let len = parts.len();
        parts.splice(len - 5..len, alias_id.split('-'));
        format!("{}{}", parts.join("-"), suffix)
    } else {
        format!("{alias_id}-{stem}{suffix}")
    }
}

fn rewrite_rollout_file(
    source_path: &Path,
    alias_path: &Path,
    source_id: &str,
    alias_id: &str,
) -> Result<(), String> {
    let content = fs::read_to_string(source_path).map_err(|err| {
        format!(
            "failed to read source rollout {}: {err}",
            source_path.display()
        )
    })?;
    let rewritten = content
        .lines()
        .map(|line| rewrite_rollout_line(line, source_id, alias_id))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");

    if let Some(parent) = alias_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create alias rollout dir: {err}"))?;
    }
    fs::write(alias_path, format!("{rewritten}\n")).map_err(|err| {
        format!(
            "failed to write alias rollout {}: {err}",
            alias_path.display()
        )
    })
}

fn rewrite_rollout_line(line: &str, source_id: &str, alias_id: &str) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(line)
        .map_err(|err| format!("failed to parse rollout JSONL line: {err}"))?;
    rewrite_json_strings(&mut value, source_id, alias_id);
    serde_json::to_string(&value).map_err(|err| format!("failed to encode rollout line: {err}"))
}

fn rewrite_json_strings(value: &mut Value, source_id: &str, alias_id: &str) {
    match value {
        Value::String(text) if text == source_id => {
            *text = alias_id.to_string();
        }
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
        Value::Array(items) => {
            for item in items {
                rewrite_json_strings(item, source_id, alias_id);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if key == "model_provider" && item == SOURCE_PROVIDER {
                    *item = Value::String(TARGET_PROVIDER.to_string());
                } else {
                    rewrite_json_strings(item, source_id, alias_id);
                }
            }
        }
    }
}

fn thread_columns(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(threads)")
        .map_err(|err| format!("prepare table_info failed: {err}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("query table_info failed: {err}"))?;
    let columns = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("read table_info failed: {err}"))?;
    if columns.is_empty() {
        return Err("Codex state database is missing `threads` columns".to_string());
    }
    Ok(columns)
}

fn insert_thread_alias(
    conn: &Connection,
    columns: &[String],
    source_id: &str,
    alias_id: &str,
    alias_rollout_path: &Path,
) -> Result<(), String> {
    let column_list = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let select_list = columns
        .iter()
        .map(|column| match column.as_str() {
            "id" => "?1".to_string(),
            "rollout_path" => "?2".to_string(),
            "model_provider" => "?3".to_string(),
            other => quote_identifier(other),
        })
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "INSERT OR IGNORE INTO threads ({column_list})
         SELECT {select_list}
         FROM threads
         WHERE id = ?4"
    );
    conn.execute(
        &sql,
        params![
            alias_id,
            alias_rollout_path.display().to_string(),
            TARGET_PROVIDER,
            source_id
        ],
    )
    .map_err(|err| format!("insert thread alias failed: {err}"))?;
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rewrites_session_id_and_provider_in_rollout_line() {
        let line = r#"{"type":"session_meta","payload":{"id":"old","model_provider":"openai","nested":["old"]}}"#;
        let rewritten = rewrite_rollout_line(line, "old", "new").expect("rewrite line");
        assert!(rewritten.contains(r#""id":"new""#));
        assert!(rewritten.contains(r#""model_provider":"ai-gateway""#));
        assert!(rewritten.contains(r#""nested":["new"]"#));
    }

    #[test]
    fn replaces_uuid_in_rollout_filename_without_dropping_suffix() {
        let file_name = "rollout-2026-05-25T00-00-00-019e5def-5a97-7ad1-a6f4-d4dac6999a6a.jsonl";
        let alias = "d76321d6-ab1e-4dde-89a8-01bb5bde487f";
        assert_eq!(
            replace_uuid_in_filename(file_name, alias),
            "rollout-2026-05-25T00-00-00-d76321d6-ab1e-4dde-89a8-01bb5bde487f.jsonl"
        );
    }

    #[test]
    fn sync_aliases_copies_existing_openai_threads() {
        let dir = test_dir("sync");
        let codex_dir = dir.join(".codex");
        let state_path = codex_dir.join("state_5.sqlite");
        let patch_path = dir.join(".ai-gateway/session-aliases.json");
        fs::create_dir_all(codex_dir.join("sessions/2026/05/25")).expect("create dirs");
        let rollout_path =
            codex_dir.join("sessions/2026/05/25/rollout-2026-05-25T00-00-00-old.jsonl");
        fs::write(
            &rollout_path,
            r#"{"type":"session_meta","payload":{"id":"old","model_provider":"openai"}}"#,
        )
        .expect("write rollout");

        let conn = Connection::open(&state_path).expect("open db");
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                created_at_ms INTEGER,
                archived INTEGER NOT NULL DEFAULT 0,
                preview TEXT NOT NULL DEFAULT ''
            )",
            [],
        )
        .expect("create table");
        conn.execute(
            "INSERT INTO threads (id, rollout_path, model_provider, created_at, created_at_ms, archived, preview)
             VALUES (?1, ?2, 'openai', 1, 1000, 0, 'hello')",
            params!["old", rollout_path.display().to_string()],
        )
        .expect("insert thread");
        drop(conn);

        let summary =
            sync_openai_history_aliases(&codex_dir, &state_path, &patch_path).expect("sync");
        assert_eq!(summary.source_threads, 1);
        assert_eq!(summary.created, 1);

        let conn = Connection::open(&state_path).expect("open db");
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM threads WHERE model_provider = 'ai-gateway'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 1);
    }

    fn test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ai-gateway-codex-history-{name}-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
