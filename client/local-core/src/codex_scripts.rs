use axum::{
    http::{
        HeaderValue,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};

const SETUP_SCRIPT: &str = include_str!("../scripts/codex-setup.sh");
const RESTORE_SCRIPT: &str = include_str!("../scripts/codex-restore.sh");
const INSTANCES_SCRIPT: &str = include_str!("../scripts/codex-instances.sh");

pub async fn setup_script() -> Response {
    shell_script_response(SETUP_SCRIPT, "setup.sh")
}

pub async fn restore_script() -> Response {
    shell_script_response(RESTORE_SCRIPT, "restore.sh")
}

pub async fn instances_script() -> Response {
    shell_script_response(INSTANCES_SCRIPT, "instances.sh")
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn setup_script_requires_a_gateway_url_and_writes_responses_provider() {
        assert!(SETUP_SCRIPT.contains("gateway_base_url=${1:-}"));
        assert!(SETUP_SCRIPT.contains("[model_providers.ai-gateway]"));
        assert!(SETUP_SCRIPT.contains("wire_api = \"responses\""));
        assert!(SETUP_SCRIPT.contains("gateway_access_token=${2:-}"));
        assert!(SETUP_SCRIPT.contains("\"OPENAI_API_KEY\""));
        assert!(SETUP_SCRIPT.contains(".ai-gateway-auth.before-setup.json"));
        assert!(!SETUP_SCRIPT.contains("experimental_bearer_token"));
        assert!(!SETUP_SCRIPT.contains("bearer_token_env_var"));
        assert!(SETUP_SCRIPT.contains("# ai-gateway.previous-model-provider: "));
        assert!(SETUP_SCRIPT.contains("state_5.sqlite"));
        assert!(SETUP_SCRIPT.contains("aliases.tsv"));
        assert!(SETUP_SCRIPT.contains("BEGIN IMMEDIATE;"));
        assert!(SETUP_SCRIPT.contains("restart_codex"));
        assert!(SETUP_SCRIPT.contains("cmp -s"));
        assert!(SETUP_SCRIPT.contains("AI Gateway 设置与目标设置一致，无需更新"));
        assert!(!SETUP_SCRIPT.contains("codex-config.before-ai-gateway.toml"));
    }

    #[test]
    fn restore_script_removes_the_gateway_provider_configuration() {
        assert!(RESTORE_SCRIPT.contains("# ai-gateway.previous-model-provider: "));
        assert!(RESTORE_SCRIPT.contains("skipping_gateway"));
        assert!(RESTORE_SCRIPT.contains("已移除 ai-gateway Provider 配置和切换标记"));
        assert!(!RESTORE_SCRIPT.contains("codex-config.before-ai-gateway.toml"));
        assert!(!RESTORE_SCRIPT.contains("state_5.sqlite"));
    }

    #[test]
    fn instances_script_uses_isolated_codex_and_electron_directories() {
        assert!(INSTANCES_SCRIPT.contains("CODEX_HOME=$codex_home"));
        assert!(INSTANCES_SCRIPT.contains("CODEX_ELECTRON_USER_DATA_PATH=$electron_home"));
        assert!(INSTANCES_SCRIPT.contains("--user-data-dir=$electron_home"));
        assert!(INSTANCES_SCRIPT.contains("auth.json is intentionally never copied"));
        assert!(INSTANCES_SCRIPT.contains("link_shared_path \"$template_home/skills\""));
        assert!(INSTANCES_SCRIPT.contains("instances.sh delete <name>"));
        assert!(INSTANCES_SCRIPT.contains("gateway-api-key"));
        assert!(INSTANCES_SCRIPT.contains("\"OPENAI_API_KEY\""));
        assert!(!INSTANCES_SCRIPT.contains("experimental_bearer_token"));
        assert!(!INSTANCES_SCRIPT.contains("bearer_token_env_var"));
        assert!(INSTANCES_SCRIPT.contains("rm -rf \"$root\""));
    }

    #[cfg(unix)]
    #[test]
    fn instances_script_creates_an_isolated_profile_without_copying_auth() {
        let test_dir = test_dir("instances");
        let home_dir = test_dir.join("home");
        let template_home = home_dir.join(".codex");
        let instances_root = home_dir.join(".ai-gateway/codex-instances");
        let bin_dir = test_dir.join("bin");
        let open_args_path = test_dir.join("open-args");
        fs::create_dir_all(template_home.join("skills")).expect("create template skills");
        fs::create_dir_all(&bin_dir).expect("create fake bin");
        fs::write(
            template_home.join("config.toml"),
            "model_provider = \"openai\"\n\
             model = \"gpt-5.4\"\n\
             [mcp_servers.node_repl.env]\n\
             CODEX_HOME = \"/old/default/.codex\"\n",
        )
        .expect("write template config");
        fs::write(
            template_home.join("auth.json"),
            "{\"tokens\":\"must-not-copy\"}",
        )
        .expect("write template auth");

        let fake_uname = bin_dir.join("uname");
        fs::write(&fake_uname, "#!/bin/sh\nprintf '%s\\n' Darwin\n").expect("write fake uname");
        let fake_open = bin_dir.join("open");
        fs::write(
            &fake_open,
            "#!/bin/sh\nscript_dir=$(CDPATH= cd -- \"$(dirname \"$0\")\" && pwd)\nprintf '%s\\n' \"$@\" > \"$script_dir/../open-args\"\n",
        )
        .expect("write fake open");
        for executable in [&fake_uname, &fake_open] {
            let mut permissions = fs::metadata(executable)
                .expect("read fake executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(executable, permissions).expect("mark fake executable executable");
        }

        let script_path = test_dir.join("instances.sh");
        fs::write(&script_path, INSTANCES_SCRIPT).expect("write instances script");
        let path = format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").expect("PATH must be present")
        );
        let output = Command::new("sh")
            .arg(&script_path)
            .args([
                "create",
                "account-a",
                "https://gateway.example.com/openai/v1",
            ])
            .env("HOME", &home_dir)
            .env("PATH", &path)
            .output()
            .expect("run instances script");
        assert!(
            output.status.success(),
            "instances script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let codex_home = instances_root.join("account-a/codex-home");
        let config =
            fs::read_to_string(codex_home.join("config.toml")).expect("read instance config");
        assert!(config.contains("model_provider = \"ai-gateway\""));
        assert!(config.contains("base_url = \"https://gateway.example.com/openai/v1\""));
        assert!(config.contains(&format!("CODEX_HOME = \"{}\"", codex_home.display())));
        assert!(!codex_home.join("auth.json").exists());
        assert!(
            fs::symlink_metadata(codex_home.join("skills"))
                .expect("read linked skills metadata")
                .file_type()
                .is_symlink()
        );

        let open_args = fs::read_to_string(&open_args_path).expect("read fake open arguments");
        assert!(open_args.contains(&format!("CODEX_HOME={}", codex_home.display())));
        assert!(open_args.contains(&format!(
            "--user-data-dir={}",
            instances_root.join("account-a/electron").display()
        )));

        let output = Command::new("sh")
            .arg(&script_path)
            .args(["delete", "account-a"])
            .env("HOME", &home_dir)
            .env("PATH", &path)
            .output()
            .expect("delete isolated instance");
        assert!(
            output.status.success(),
            "instances delete failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!instances_root.join("account-a").exists());

        fs::remove_dir_all(test_dir).expect("remove test dir");
    }

    #[cfg(unix)]
    #[test]
    fn restore_script_cleans_gateway_toml_configuration() {
        let test_dir = test_dir("restore-config");
        let codex_dir = test_dir.join(".codex");
        fs::create_dir_all(&codex_dir).expect("create codex dir");
        fs::write(
            codex_dir.join("config.toml"),
            "# ai-gateway.previous-model-provider: model_provider = \"openai\"\n\
             model_provider = \"ai-gateway\"\n\
             model = \"gpt-5.4\"\n\
             [model_providers.ai-gateway]\n\
             name = \"ai-gateway\"\n\
             base_url = \"https://gateway.example.com/openai/v1\"\n\
             wire_api = \"responses\"\n\
             [features]\n\
             web_search_request = true\n",
        )
        .expect("write config");

        let script_path = test_dir.join("restore.sh");
        fs::write(&script_path, RESTORE_SCRIPT).expect("write restore script");
        let output = Command::new("sh")
            .arg(&script_path)
            .env("HOME", &test_dir)
            .output()
            .expect("run restore script");
        assert!(
            output.status.success(),
            "restore script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let config =
            fs::read_to_string(codex_dir.join("config.toml")).expect("read cleaned config");
        assert!(config.contains("model_provider = \"openai\""));
        assert!(!config.contains("ai-gateway"));
        assert!(config.contains("[features]"));

        fs::remove_dir_all(test_dir).expect("remove test dir");
    }

    #[cfg(unix)]
    #[test]
    fn setup_and_restore_preserve_original_codex_auth() {
        let test_dir = test_dir("setup-auth");
        let codex_dir = test_dir.join(".codex");
        fs::create_dir_all(&codex_dir).expect("create codex dir");
        fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"openai\"\nmodel = \"gpt-5.4\"\n",
        )
        .expect("write config");
        let original_auth = "{\"tokens\":{\"access_token\":\"original\"}}\n";
        fs::write(codex_dir.join("auth.json"), original_auth).expect("write original auth");

        let setup_path = test_dir.join("setup.sh");
        fs::write(&setup_path, SETUP_SCRIPT).expect("write setup script");
        let test_path = path_with_fake_uname(&test_dir, "TestOS");
        let setup = Command::new("sh")
            .arg(&setup_path)
            .args([
                "https://gateway.example.com/openai/v1",
                "agw_test_gateway_key",
            ])
            .env("HOME", &test_dir)
            .env("PATH", &test_path)
            .output()
            .expect("run setup script");
        assert!(
            setup.status.success(),
            "setup script failed: {}",
            String::from_utf8_lossy(&setup.stderr)
        );

        let gateway_auth =
            fs::read_to_string(codex_dir.join("auth.json")).expect("read gateway auth");
        assert!(gateway_auth.contains("\"OPENAI_API_KEY\": \"agw_test_gateway_key\""));
        assert_eq!(
            fs::read_to_string(codex_dir.join(".ai-gateway-auth.before-setup.json"))
                .expect("read auth backup"),
            original_auth
        );

        let update = Command::new("sh")
            .arg(&setup_path)
            .args([
                "https://gateway-two.example.com/openai/v1",
                "agw_updated_gateway_key",
            ])
            .env("HOME", &test_dir)
            .env("PATH", &test_path)
            .output()
            .expect("rerun setup script with changed settings");
        assert!(
            update.status.success(),
            "updated setup script failed: {}",
            String::from_utf8_lossy(&update.stderr)
        );
        assert!(String::from_utf8_lossy(&update.stdout).contains("AI Gateway 设置已更新"));
        assert!(
            fs::read_to_string(codex_dir.join("config.toml"))
                .expect("read updated config")
                .contains("base_url = \"https://gateway-two.example.com/openai/v1\"")
        );
        assert!(
            fs::read_to_string(codex_dir.join("auth.json"))
                .expect("read updated gateway auth")
                .contains("\"OPENAI_API_KEY\": \"agw_updated_gateway_key\"")
        );

        let unchanged = Command::new("sh")
            .arg(&setup_path)
            .args([
                "https://gateway-two.example.com/openai/v1",
                "agw_updated_gateway_key",
            ])
            .env("HOME", &test_dir)
            .env("PATH", &test_path)
            .output()
            .expect("rerun setup script with unchanged settings");
        assert!(unchanged.status.success());
        assert!(
            String::from_utf8_lossy(&unchanged.stdout)
                .contains("AI Gateway 设置与目标设置一致，无需更新")
        );

        let restore_path = test_dir.join("restore.sh");
        fs::write(&restore_path, RESTORE_SCRIPT).expect("write restore script");
        let restore = Command::new("sh")
            .arg(&restore_path)
            .env("HOME", &test_dir)
            .output()
            .expect("run restore script");
        assert!(
            restore.status.success(),
            "restore script failed: {}",
            String::from_utf8_lossy(&restore.stderr)
        );
        assert_eq!(
            fs::read_to_string(codex_dir.join("auth.json")).expect("read restored auth"),
            original_auth
        );
        assert!(
            !codex_dir
                .join(".ai-gateway-auth.before-setup.json")
                .exists()
        );

        fs::remove_dir_all(test_dir).expect("remove test dir");
    }

    #[cfg(unix)]
    #[test]
    fn setup_script_syncs_history_aliases_idempotently() {
        if Command::new("sqlite3").arg("--version").output().is_err() {
            return;
        }

        let test_dir = test_dir("history-aliases");
        let codex_dir = test_dir.join(".codex");
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

    #[cfg(unix)]
    fn run_setup_script(script_path: &Path, codex_dir: &Path) {
        let test_path = path_with_fake_uname(codex_dir.parent().expect("test home"), "TestOS");
        let output = Command::new("sh")
            .arg(script_path)
            .arg("https://gateway.example.com/openai/v1")
            .env("HOME", codex_dir.parent().expect("test home"))
            .env("PATH", test_path)
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

    #[cfg(unix)]
    fn path_with_fake_uname(root: &std::path::Path, system_name: &str) -> String {
        let bin_dir = root.join("test-bin");
        fs::create_dir_all(&bin_dir).expect("create test bin");
        let uname = bin_dir.join("uname");
        fs::write(
            &uname,
            format!("#!/bin/sh\nprintf '%s\\n' '{system_name}'\n"),
        )
        .expect("write fake uname");
        let mut permissions = fs::metadata(&uname).expect("read fake uname").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&uname, permissions).expect("mark fake uname executable");
        format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").expect("PATH must be present")
        )
    }
}
