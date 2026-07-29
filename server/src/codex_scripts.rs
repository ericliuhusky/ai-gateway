use axum::{
    http::{
        HeaderValue,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};

const SETUP_SCRIPT: &str = include_str!("../scripts/codex-setup.sh");
const RESTORE_SCRIPT: &str = include_str!("../scripts/codex-restore.sh");

pub async fn setup_script() -> Response {
    shell_script_response(SETUP_SCRIPT, "setup.sh")
}

pub async fn restore_script() -> Response {
    shell_script_response(RESTORE_SCRIPT, "restore.sh")
}

fn shell_script_response(script: &'static str, filename: &'static str) -> Response {
    let disposition = HeaderValue::from_str(&format!("inline; filename=\"{filename}\""))
        .expect("static script filename must be a valid header value");

    (
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("text/x-shellscript; charset=utf-8"),
            ),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (CONTENT_DISPOSITION, disposition),
        ],
        script,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::{
        fs,
        path::PathBuf,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn setup_script_requires_a_gateway_url_and_writes_responses_provider() {
        assert!(SETUP_SCRIPT.contains("gateway_base_url=${1:-}"));
        assert!(SETUP_SCRIPT.contains("[model_providers.ai-gateway]"));
        assert!(SETUP_SCRIPT.contains("wire_api = \"responses\""));
        assert!(SETUP_SCRIPT.contains("# ai-gateway.previous-model-provider: "));
        assert!(SETUP_SCRIPT.contains("state_5.sqlite"));
        assert!(SETUP_SCRIPT.contains("aliases.tsv"));
        assert!(SETUP_SCRIPT.contains("BEGIN IMMEDIATE;"));
        assert!(!SETUP_SCRIPT.contains("codex-config.before-ai-gateway.toml"));
    }

    #[test]
    fn restore_script_restores_only_the_previous_provider() {
        assert!(RESTORE_SCRIPT.contains("# ai-gateway.previous-model-provider: "));
        assert!(RESTORE_SCRIPT.contains("Provider 配置已保留"));
        assert!(!RESTORE_SCRIPT.contains("codex-config.before-ai-gateway.toml"));
        assert!(!RESTORE_SCRIPT.contains("state_5.sqlite"));
    }

    #[test]
    fn setup_script_syncs_history_aliases_idempotently() {
        if Command::new("sqlite3").arg("--version").output().is_err() {
            return;
        }

        let test_dir = test_dir("history-aliases");
        let codex_dir = test_dir.join("codex");
        let sessions_dir = codex_dir.join("sessions/2026/07/29");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        let source_id = "019fabcd-1234-7abc-8def-1234567890ab";
        let source_rollout =
            sessions_dir.join(format!("rollout-2026-07-29T10-00-00-{source_id}.jsonl"));
        fs::write(
            &source_rollout,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{source_id}\",\"model_provider\":\"openai\"}}}}\n\
                 {{\"type\":\"event_msg\",\"payload\":{{\"message\":\"keep embedded {source_id}\"}}}}\n"
            ),
        )
        .expect("write source rollout");
        fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"openai\"\nmodel = \"gpt-5.4\"\n",
        )
        .expect("write config");

        let state_path = codex_dir.join("state_5.sqlite");
        let conn = Connection::open(&state_path).expect("open state db");
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                source TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL,
                title TEXT NOT NULL,
                sandbox_policy TEXT NOT NULL,
                approval_mode TEXT NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER,
                preview TEXT NOT NULL DEFAULT ''
            );",
        )
        .expect("create threads table");
        conn.execute(
            "INSERT INTO threads (
                id, rollout_path, created_at, updated_at, source, model_provider, cwd,
                title, sandbox_policy, approval_mode, archived, created_at_ms, preview
             ) VALUES (?1, ?2, 1, 1, 'cli', 'openai', '/tmp/project',
                'Test history', '{}', 'never', 0, 1000, 'hello')",
            params![source_id, source_rollout.display().to_string()],
        )
        .expect("insert source thread");
        drop(conn);

        let script_path = test_dir.join("setup.sh");
        fs::write(&script_path, SETUP_SCRIPT).expect("write setup script");

        run_setup_script(&script_path, &codex_dir);

        let mapping = fs::read_to_string(codex_dir.join(".ai-gateway-history/aliases.tsv"))
            .expect("read alias mapping");
        let mapping_fields = mapping
            .lines()
            .find(|line| !line.starts_with('#'))
            .expect("alias mapping row")
            .split('\t')
            .collect::<Vec<_>>();
        assert_eq!(mapping_fields.len(), 4);
        assert_eq!(mapping_fields[0], "openai");
        assert_eq!(mapping_fields[1], source_id);

        let alias_id = mapping_fields[2];
        let alias_rollout = fs::read_to_string(mapping_fields[3]).expect("read alias rollout");
        assert!(alias_rollout.contains(&format!("\"id\":\"{alias_id}\"")));
        assert!(alias_rollout.contains("\"model_provider\":\"ai-gateway\""));
        assert!(alias_rollout.contains(&format!("keep embedded {source_id}")));

        let conn = Connection::open(&state_path).expect("reopen state db");
        let alias_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM threads WHERE model_provider = 'ai-gateway'",
                [],
                |row| row.get(0),
            )
            .expect("count aliases");
        assert_eq!(alias_count, 1);
        drop(conn);

        run_setup_script(&script_path, &codex_dir);

        let conn = Connection::open(&state_path).expect("reopen state db after rerun");
        let alias_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM threads WHERE model_provider = 'ai-gateway'",
                [],
                |row| row.get(0),
            )
            .expect("count aliases after rerun");
        assert_eq!(alias_count, 1);
        assert!(
            codex_dir
                .join(".ai-gateway-history/state_5.before-first-sync.sqlite")
                .is_file()
        );

        fs::remove_dir_all(test_dir).expect("remove test dir");
    }

    fn run_setup_script(script_path: &PathBuf, codex_dir: &PathBuf) {
        let output = Command::new("sh")
            .arg(script_path)
            .arg("https://gateway.example.com/openai/v1")
            .env("HOME", codex_dir.parent().expect("test home"))
            .env("CODEX_HOME", codex_dir)
            .output()
            .expect("run setup script");
        assert!(
            output.status.success(),
            "setup script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ai-gateway-codex-script-{name}-{}-{nanos}",
            std::process::id()
        ))
    }
}
