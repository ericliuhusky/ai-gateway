use rusqlite::{Connection, TransactionBehavior, params, types::Value};
use serde::Serialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use url::Url;
use uuid::Uuid;

const MARKER_PREFIX: &str = "# ai-gateway.previous-model-provider: ";
const GATEWAY_PROVIDER: &str = "ai-gateway";

#[derive(Debug, Clone, Serialize)]
pub struct DefaultCodexStatus {
    pub started: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexConfigurationResult {
    pub changed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CodexInstancePaths {
    pub codex_home: PathBuf,
    pub electron_home: PathBuf,
}

pub fn prepare_codex_instance(
    instance_id: &str,
    gateway_base_url: &str,
) -> Result<CodexInstancePaths, String> {
    validate_instance_id(instance_id)?;
    let template_home = codex_dir()?;
    let root = instance_root(instance_id)?;
    prepare_codex_instance_at(&template_home, &root, gateway_base_url)
}

pub fn delete_codex_instance(instance_id: &str) -> Result<bool, String> {
    validate_instance_id(instance_id)?;
    delete_codex_instance_at(&instance_root(instance_id)?)
}

fn prepare_codex_instance_at(
    template_home: &Path,
    root: &Path,
    gateway_base_url: &str,
) -> Result<CodexInstancePaths, String> {
    let gateway_base_url = normalize_gateway_url(gateway_base_url)?;
    let paths = CodexInstancePaths {
        codex_home: root.join("codex-home"),
        electron_home: root.join("electron"),
    };

    if root.exists() {
        if !paths.codex_home.join("config.toml").is_file() {
            return Err(format!("Codex 实例本地配置不完整：{}", root.display()));
        }
        fs::create_dir_all(&paths.electron_home)
            .map_err(|error| format!("创建 Codex 实例数据目录失败：{error}"))?;
        return Ok(paths);
    }

    fs::create_dir_all(&paths.codex_home)
        .map_err(|error| format!("创建 Codex 实例配置目录失败：{error}"))?;
    let create_result = (|| {
        link_shared_path(
            &template_home.join("skills"),
            &paths.codex_home.join("skills"),
        )?;
        link_shared_path(
            &template_home.join("rules"),
            &paths.codex_home.join("rules"),
        )?;
        link_shared_path(
            &template_home.join("AGENTS.md"),
            &paths.codex_home.join("AGENTS.md"),
        )?;
        let config_path = paths.codex_home.join("config.toml");
        let source = read_optional(&template_home.join("config.toml"))?;
        let (config, _) =
            configure_gateway_config_for_home(&source, &gateway_base_url, Some(&paths.codex_home));
        write_if_changed(&config_path, config.as_bytes())?;
        fs::create_dir_all(&paths.electron_home)
            .map_err(|error| format!("创建 Codex 实例 Electron 数据目录失败：{error}"))?;
        Ok::<(), String>(())
    })();
    if let Err(error) = create_result {
        let _ = fs::remove_dir_all(root);
        return Err(error);
    }
    Ok(paths)
}

fn delete_codex_instance_at(root: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("读取 Codex 实例目录失败：{error}")),
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(root).map_err(|error| format!("删除 Codex 实例文件失败：{error}"))?;
    } else {
        fs::remove_file(root).map_err(|error| format!("删除 Codex 实例文件失败：{error}"))?;
    }
    Ok(true)
}

pub fn default_codex_status() -> Result<DefaultCodexStatus, String> {
    let config_path = codex_dir()?.join("config.toml");
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DefaultCodexStatus { started: false });
        }
        Err(error) => return Err(format!("读取 Codex 配置失败：{error}")),
    };
    Ok(DefaultCodexStatus {
        started: is_gateway_configured(&content),
    })
}

pub fn start_default_codex(gateway_base_url: &str) -> Result<CodexConfigurationResult, String> {
    let gateway_base_url = normalize_gateway_url(gateway_base_url)?;
    let codex_dir = codex_dir()?;
    fs::create_dir_all(&codex_dir).map_err(|error| format!("创建 Codex 配置目录失败：{error}"))?;
    let _lock = ConfigLock::acquire(&codex_dir)?;
    let config_path = codex_dir.join("config.toml");
    let source = read_optional(&config_path)?;
    let (next, previous_provider) = configure_gateway_config(&source, &gateway_base_url);
    let changed = write_if_changed(&config_path, next.as_bytes())?;
    let warnings = sync_history_aliases(&codex_dir, &previous_provider);
    Ok(CodexConfigurationResult { changed, warnings })
}

pub fn stop_default_codex() -> Result<CodexConfigurationResult, String> {
    let codex_dir = codex_dir()?;
    let config_path = codex_dir.join("config.toml");
    let source = match read_optional(&config_path)? {
        Some(source) => source,
        None => {
            return Ok(CodexConfigurationResult {
                changed: false,
                warnings: Vec::new(),
            });
        }
    };
    let _lock = ConfigLock::acquire(&codex_dir)?;
    let next = restore_gateway_config(&source)?;
    let mut changed = write_if_changed(&config_path, next.as_bytes())?;
    changed |= restore_authentication(&codex_dir)?;
    Ok(CodexConfigurationResult {
        changed,
        warnings: Vec::new(),
    })
}

fn codex_dir() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or_else(|| "未设置 HOME 环境变量".to_string())?;
    Ok(PathBuf::from(home).join(".codex"))
}

fn instance_root(instance_id: &str) -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or_else(|| "未设置 HOME 环境变量".to_string())?;
    Ok(PathBuf::from(home)
        .join(".ai-gateway")
        .join("codex-instances")
        .join(instance_id))
}

fn validate_instance_id(instance_id: &str) -> Result<(), String> {
    if instance_id.is_empty()
        || instance_id.starts_with('.')
        || !instance_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(format!(
            "实例名称只能包含字母、数字、_ 或 -，且不能以 . 开头：{instance_id}"
        ));
    }
    Ok(())
}

fn link_shared_path(source: &Path, target: &Path) -> Result<(), String> {
    if !source.exists() || target.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
            .map_err(|error| format!("链接共享 Codex 文件失败：{error}"))?;
    }
    #[cfg(not(unix))]
    {
        if source.is_dir() {
            return Err("当前系统不支持创建 Codex 实例共享目录链接".to_string());
        }
        fs::copy(source, target).map_err(|error| format!("复制共享 Codex 文件失败：{error}"))?;
    }
    Ok(())
}

fn replace_codex_home(line: &str, codex_home: &Path) -> Option<String> {
    let trimmed = line.trim_start();
    let prefix_length = line.len() - trimmed.len();
    let remainder = trimmed.strip_prefix("CODEX_HOME")?.trim_start();
    remainder.strip_prefix('=')?;
    let home = codex_home
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    Some(format!(
        "{}CODEX_HOME = \"{}\"",
        &line[..prefix_length],
        home
    ))
}

fn normalize_gateway_url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    let url = Url::parse(value).map_err(|_| "Gateway 地址必须是有效的 http(s) URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Gateway 地址必须是有效的 http(s) URL".to_string());
    }
    Ok(value.to_string())
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取 {} 失败：{error}", path.display())),
    }
}

fn is_root_table(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

fn is_gateway_table(line: &str) -> bool {
    let compact = line.trim_start();
    let Some(suffix) = compact.strip_prefix("[model_providers.ai-gateway]") else {
        return false;
    };
    suffix.trim().is_empty() || suffix.trim_start().starts_with('#')
}

fn root_model_provider(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let remainder = trimmed.strip_prefix("model_provider")?.trim_start();
    let remainder = remainder.strip_prefix('=')?.trim_start();
    Some(remainder)
}

fn configure_gateway_config(source: &Option<String>, gateway_base_url: &str) -> (String, String) {
    configure_gateway_config_for_home(source, gateway_base_url, None)
}

fn configure_gateway_config_for_home(
    source: &Option<String>,
    gateway_base_url: &str,
    codex_home: Option<&Path>,
) -> (String, String) {
    let source = source.as_deref().unwrap_or_default();
    let previous_provider = detect_previous_provider(source);
    let mut root = Vec::new();
    let mut rest = Vec::new();
    let mut in_root = true;
    let mut skipping_gateway = false;
    let mut marker: Option<String> = None;
    let mut first_provider: Option<String> = None;

    for source_line in source.lines() {
        let rewritten_line = codex_home.and_then(|home| replace_codex_home(source_line, home));
        let line = rewritten_line.as_deref().unwrap_or(source_line);
        if skipping_gateway {
            if is_root_table(line) {
                skipping_gateway = false;
            } else {
                continue;
            }
        }
        if is_gateway_table(line) {
            skipping_gateway = true;
            continue;
        }
        if in_root && is_root_table(line) {
            in_root = false;
        }
        if in_root {
            if let Some(value) = line.strip_prefix(MARKER_PREFIX) {
                marker.get_or_insert_with(|| format!("{MARKER_PREFIX}{value}"));
                continue;
            }
            if root_model_provider(line).is_some() {
                first_provider.get_or_insert_with(|| line.to_string());
                continue;
            }
            root.push(line.to_string());
        } else {
            rest.push(line.to_string());
        }
    }

    let marker = marker
        .or_else(|| first_provider.map(|line| format!("{MARKER_PREFIX}{line}")))
        .unwrap_or_else(|| format!("{MARKER_PREFIX}<absent>"));
    let mut lines = vec![marker, format!("model_provider = \"{GATEWAY_PROVIDER}\"")];
    lines.append(&mut root);
    lines.append(&mut rest);
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines.push(String::new());
    lines.push("[model_providers.ai-gateway]".to_string());
    lines.push("name = \"ai-gateway\"".to_string());
    lines.push(format!("base_url = \"{gateway_base_url}\""));
    lines.push("wire_api = \"responses\"".to_string());
    (lines.join("\n") + "\n", previous_provider)
}

fn restore_gateway_config(source: &str) -> Result<String, String> {
    let mut root = Vec::new();
    let mut rest = Vec::new();
    let mut in_root = true;
    let mut skipping_gateway = false;
    let mut marker: Option<String> = None;

    for line in source.lines() {
        if skipping_gateway {
            if is_root_table(line) {
                skipping_gateway = false;
            } else {
                continue;
            }
        }
        if is_gateway_table(line) {
            skipping_gateway = true;
            continue;
        }
        if in_root && is_root_table(line) {
            in_root = false;
        }
        if in_root {
            if let Some(value) = line.strip_prefix(MARKER_PREFIX) {
                marker.get_or_insert_with(|| value.to_string());
                continue;
            }
            if root_model_provider(line).is_some() {
                continue;
            }
            root.push(line.to_string());
        } else {
            rest.push(line.to_string());
        }
    }

    let marker = marker.ok_or_else(|| "没有找到 AI Gateway 保存的原模型供应商".to_string())?;
    let mut lines = Vec::new();
    if marker != "<absent>" {
        lines.push(marker);
    }
    lines.append(&mut root);
    lines.append(&mut rest);
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    Ok(lines.join("\n") + "\n")
}

fn detect_previous_provider(source: &str) -> String {
    for line in source.lines() {
        if is_root_table(line) {
            break;
        }
        let candidate = line.strip_prefix(MARKER_PREFIX).unwrap_or(line);
        if let Some(value) = parse_provider_value(candidate) {
            return value;
        }
    }
    "openai".to_string()
}

fn parse_provider_value(line: &str) -> Option<String> {
    let value = root_model_provider(line)?
        .strip_prefix('"')?
        .split('"')
        .next()?;
    (!value.is_empty()
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-')))
    .then(|| value.to_string())
}

fn write_if_changed(path: &Path, content: &[u8]) -> Result<bool, String> {
    if fs::read(path).ok().as_deref() == Some(content) {
        return Ok(false);
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Codex 配置路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建 Codex 配置目录失败：{error}"))?;
    let temporary = parent.join(format!(
        ".{}.ai-gateway-{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        Uuid::new_v4()
    ));
    fs::write(&temporary, content)
        .map_err(|error| format!("写入 {} 失败：{error}", temporary.display()))?;
    set_private_permissions(&temporary)?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("保存 {} 失败：{error}", path.display()))?;
    Ok(true)
}

fn restore_authentication(codex_dir: &Path) -> Result<bool, String> {
    let auth_path = codex_dir.join("auth.json");
    let backup_path = codex_dir.join(".ai-gateway-auth.before-setup.json");
    let absent_marker = codex_dir.join(".ai-gateway-auth.was-absent");
    if backup_path.exists() {
        fs::rename(&backup_path, &auth_path)
            .map_err(|error| format!("恢复 Codex 登录凭据失败：{error}"))?;
        let _ = fs::remove_file(absent_marker);
        return Ok(true);
    }
    if absent_marker.exists() {
        let _ = fs::remove_file(&auth_path);
        fs::remove_file(absent_marker)
            .map_err(|error| format!("清理 Codex 登录标记失败：{error}"))?;
        return Ok(true);
    }
    Ok(false)
}

struct ConfigLock {
    path: PathBuf,
}
impl ConfigLock {
    fn acquire(codex_dir: &Path) -> Result<Self, String> {
        let path = codex_dir.join(".ai-gateway-config.lock");
        fs::create_dir(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "另一个 AI Gateway 配置操作正在运行".to_string()
            } else {
                format!("创建 Codex 配置锁失败：{error}")
            }
        })?;
        Ok(Self { path })
    }
}
impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn set_private_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置 {} 权限失败：{error}", path.display()))?;
    }
    Ok(())
}

#[derive(Debug)]
struct ThreadSource {
    id: String,
    rollout_path: String,
    archived: bool,
}

fn sync_history_aliases(codex_dir: &Path, source_provider: &str) -> Vec<String> {
    if source_provider == GATEWAY_PROVIDER {
        return Vec::new();
    }
    let state_path = codex_dir.join("state_5.sqlite");
    if !state_path.exists() {
        return Vec::new();
    }
    match sync_history_aliases_inner(codex_dir, source_provider, &state_path) {
        Ok(()) => Vec::new(),
        Err(error) => vec![format!("Codex 历史同步已跳过：{error}")],
    }
}

fn sync_history_aliases_inner(
    codex_dir: &Path,
    source_provider: &str,
    state_path: &Path,
) -> Result<(), String> {
    let history_dir = codex_dir.join(".ai-gateway-history");
    fs::create_dir_all(&history_dir).map_err(|error| format!("创建历史目录失败：{error}"))?;
    let mut connection = Connection::open(state_path)
        .map_err(|error| format!("打开 Codex state 数据库失败：{error}"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(10))
        .map_err(|error| format!("设置数据库等待时间失败：{error}"))?;
    let has_threads: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'threads')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("检查 Codex 历史数据库失败：{error}"))?;
    if !has_threads {
        return Ok(());
    }

    let columns = connection
        .prepare("SELECT name FROM pragma_table_info('threads') ORDER BY cid")
        .map_err(|error| format!("读取 Codex 历史表结构失败：{error}"))?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("读取 Codex 历史表结构失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取 Codex 历史表结构失败：{error}"))?;
    if !["id", "rollout_path", "model_provider"]
        .iter()
        .all(|required| columns.iter().any(|column| column == required))
    {
        return Ok(());
    }
    if columns.iter().any(|column| {
        !column
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_')
    }) {
        return Ok(());
    }

    let sources = connection.prepare(
        "SELECT id, rollout_path, archived FROM threads WHERE model_provider = ?1 ORDER BY created_at, id"
    ).map_err(|error| format!("读取 Codex 历史失败：{error}"))?
        .query_map([source_provider], |row| Ok(ThreadSource { id: row.get(0)?, rollout_path: row.get(1)?, archived: row.get::<_, i64>(2)? != 0 }))
        .map_err(|error| format!("读取 Codex 历史失败：{error}"))?
        .collect::<Result<Vec<_>, _>>().map_err(|error| format!("读取 Codex 历史失败：{error}"))?;
    if sources.is_empty() {
        return Ok(());
    }

    let backup_path = history_dir.join("state_5.before-first-sync.sqlite");
    if !backup_path.exists() {
        let path = backup_path.display().to_string().replace('\'', "''");
        connection
            .execute_batch(&format!("VACUUM INTO '{path}'"))
            .map_err(|error| format!("创建 Codex state 备份失败：{error}"))?;
        set_private_permissions(&backup_path)?;
    }

    let mapping_path = history_dir.join("aliases.tsv");
    let mut mappings = read_alias_mappings(&mapping_path)?;
    let mut changed_mapping = false;
    let mut inserts = Vec::new();
    for source in sources {
        if !is_safe_thread_id(&source.id) {
            continue;
        }
        let key = (source_provider.to_string(), source.id.clone());
        let (alias_id, alias_rollout) = if let Some(existing) = mappings.get(&key) {
            (existing.0.clone(), PathBuf::from(&existing.1))
        } else {
            let alias_id = Uuid::new_v4().to_string();
            let alias_rollout = alias_rollout_path(codex_dir, &source, &alias_id)?;
            mappings.insert(
                key.clone(),
                (alias_id.clone(), alias_rollout.display().to_string()),
            );
            changed_mapping = true;
            (alias_id, alias_rollout)
        };
        if !is_safe_thread_id(&alias_id) || !alias_rollout.starts_with(codex_dir) {
            continue;
        }
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM threads WHERE id = ?1 AND model_provider = ?2)",
                params![alias_id, GATEWAY_PROVIDER],
                |row| row.get(0),
            )
            .map_err(|error| format!("检查 Codex 历史别名失败：{error}"))?;
        if !alias_rollout.exists() {
            rewrite_rollout(
                Path::new(&source.rollout_path),
                &alias_rollout,
                &source.id,
                &alias_id,
                source_provider,
            )?;
        }
        if !exists {
            inserts.push((source.id, alias_id, alias_rollout.display().to_string()));
        }
    }
    if changed_mapping {
        write_alias_mappings(&mapping_path, &mappings)?;
    }
    if inserts.is_empty() {
        return Ok(());
    }

    let quoted_columns = columns
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let select_columns = columns
        .iter()
        .map(|column| match column.as_str() {
            "id" | "rollout_path" | "model_provider" => "?".to_string(),
            _ => format!("\"{column}\""),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO threads ({quoted_columns}) SELECT {select_columns} FROM threads WHERE id = ? AND model_provider = ?"
    );
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("写入 Codex 历史别名失败：{error}"))?;
    for (source_id, alias_id, alias_rollout) in inserts {
        let mut values = Vec::<Value>::new();
        for column in &columns {
            match column.as_str() {
                "id" => values.push(Value::Text(alias_id.clone())),
                "rollout_path" => values.push(Value::Text(alias_rollout.clone())),
                "model_provider" => values.push(Value::Text(GATEWAY_PROVIDER.to_string())),
                _ => {}
            }
        }
        values.push(Value::Text(source_id));
        values.push(Value::Text(source_provider.to_string()));
        transaction
            .execute(&sql, rusqlite::params_from_iter(values))
            .map_err(|error| format!("写入 Codex 历史别名失败：{error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("提交 Codex 历史别名失败：{error}"))?;
    Ok(())
}

fn is_safe_thread_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '-')
}

fn alias_rollout_path(
    codex_dir: &Path,
    source: &ThreadSource,
    alias_id: &str,
) -> Result<PathBuf, String> {
    let source_file = Path::new(&source.rollout_path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "历史任务 rollout 文件名无效".to_string())?;
    if !source_file.starts_with("rollout-")
        || !source_file.ends_with(".jsonl")
        || !source_file.contains(&source.id)
    {
        return Err("历史任务 rollout 文件名无效".to_string());
    }
    let alias_file = source_file.replacen(&source.id, alias_id, 1);
    if source.archived {
        return Ok(codex_dir.join("archived_sessions").join(alias_file));
    }
    let date = source_file
        .strip_prefix("rollout-")
        .and_then(|name| name.get(..10))
        .filter(|date| {
            date.as_bytes().get(4) == Some(&b'-') && date.as_bytes().get(7) == Some(&b'-')
        })
        .ok_or_else(|| "历史任务 rollout 日期无效".to_string())?;
    Ok(codex_dir
        .join("sessions")
        .join(&date[..4])
        .join(&date[5..7])
        .join(&date[8..10])
        .join(alias_file))
}

fn rewrite_rollout(
    source: &Path,
    target: &Path,
    source_id: &str,
    alias_id: &str,
    source_provider: &str,
) -> Result<(), String> {
    let content =
        fs::read_to_string(source).map_err(|error| format!("读取历史 rollout 失败：{error}"))?;
    let id_needle = format!("\"{source_id}\"");
    let provider_needle = format!("\"model_provider\":\"{source_provider}\"");
    if !content.contains(&id_needle) || !content.contains(&provider_needle) {
        return Err("历史 rollout 内容无法安全转换".to_string());
    }
    let content = content
        .replace(&id_needle, &format!("\"{alias_id}\""))
        .replace(&provider_needle, "\"model_provider\":\"ai-gateway\"");
    let parent = target
        .parent()
        .ok_or_else(|| "历史 rollout 目标路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建历史 rollout 目录失败：{error}"))?;
    write_if_changed(target, content.as_bytes())?;
    Ok(())
}

fn read_alias_mappings(
    path: &Path,
) -> Result<std::collections::BTreeMap<(String, String), (String, String)>, String> {
    let mut result = std::collections::BTreeMap::new();
    let Some(content) = read_optional(path)? else {
        return Ok(result);
    };
    for line in content.lines().filter(|line| !line.starts_with('#')) {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() == 4 {
            result.insert(
                (fields[0].to_string(), fields[1].to_string()),
                (fields[2].to_string(), fields[3].to_string()),
            );
        }
    }
    Ok(result)
}

fn write_alias_mappings(
    path: &Path,
    mappings: &std::collections::BTreeMap<(String, String), (String, String)>,
) -> Result<(), String> {
    let mut content = String::from("# source_provider\tsource_id\talias_id\talias_rollout_path\n");
    for ((provider, source), (alias, rollout)) in mappings {
        content.push_str(&format!("{provider}\t{source}\t{alias}\t{rollout}\n"));
    }
    write_if_changed(path, content.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_and_restore_preserve_root_configuration() {
        let source = "model_provider = \"openai\"\nmodel = \"gpt-5\"\n[features]\nweb_search_request = true\n";
        let (configured, previous) =
            configure_gateway_config(&Some(source.to_string()), "http://127.0.0.1:4242/openai/v1");
        assert_eq!(previous, "openai");
        assert!(is_gateway_configured(&configured));
        assert!(configured.contains("wire_api = \"responses\""));
        let restored = restore_gateway_config(&configured).expect("restore configuration");
        assert_eq!(restored, source);
    }
    #[test]
    fn setup_is_idempotent() {
        let (first, _) = configure_gateway_config(&None, "http://127.0.0.1:4242/openai/v1");
        let (second, _) =
            configure_gateway_config(&Some(first.clone()), "http://127.0.0.1:4242/openai/v1");
        assert_eq!(first, second);
    }

    #[test]
    fn history_sync_creates_idempotent_gateway_aliases() {
        let root = unique_test_dir();
        let codex_dir = root.join(".codex");
        let sessions = codex_dir.join("sessions/2026/08/11");
        fs::create_dir_all(&sessions).expect("create sessions");
        let source_id = "019fabcd-1234-7abc-8def-1234567890ab";
        let rollout = sessions.join(format!("rollout-2026-08-11T10-00-00-{source_id}.jsonl"));
        fs::write(
            &rollout,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{source_id}\",\"model_provider\":\"openai\"}}}}\n"
            ),
        )
        .expect("write rollout");
        let state_path = codex_dir.join("state_5.sqlite");
        let connection = Connection::open(&state_path).expect("open state database");
        connection
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    model_provider TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0
                );",
            )
            .expect("create threads");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, created_at, model_provider, archived) VALUES (?1, ?2, 1, 'openai', 0)",
                params![source_id, rollout.display().to_string()],
            )
            .expect("insert source thread");
        drop(connection);

        sync_history_aliases_inner(&codex_dir, "openai", &state_path).expect("sync history");
        sync_history_aliases_inner(&codex_dir, "openai", &state_path).expect("resync history");

        let mapping = fs::read_to_string(codex_dir.join(".ai-gateway-history/aliases.tsv"))
            .expect("read alias mapping");
        let mapping = mapping
            .lines()
            .find(|line| !line.starts_with('#'))
            .expect("mapping row");
        let fields = mapping.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 4);
        let alias_rollout = fs::read_to_string(fields[3]).expect("read alias rollout");
        assert!(alias_rollout.contains(&format!("\"id\":\"{}\"", fields[2])));
        assert!(alias_rollout.contains("\"model_provider\":\"ai-gateway\""));
        let connection = Connection::open(&state_path).expect("reopen state database");
        let aliases: i64 = connection
            .query_row(
                "SELECT count(*) FROM threads WHERE model_provider = 'ai-gateway'",
                [],
                |row| row.get(0),
            )
            .expect("count aliases");
        assert_eq!(aliases, 1);
        assert!(
            codex_dir
                .join(".ai-gateway-history/state_5.before-first-sync.sqlite")
                .exists()
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn prepares_opens_ready_instance_profile_without_copying_auth() {
        let root = unique_test_dir();
        let template = root.join("template");
        let instance_root = root.join("instances/account-a");
        fs::create_dir_all(template.join("skills")).expect("create template skills");
        fs::write(
            template.join("config.toml"),
            "model_provider = \"openai\"\n[mcp_servers.node_repl.env]\nCODEX_HOME = \"/old/home\"\n",
        )
        .expect("write template config");
        fs::write(template.join("auth.json"), "{\"tokens\":\"do-not-copy\"}")
            .expect("write template auth");

        let paths = prepare_codex_instance_at(
            &template,
            &instance_root,
            "http://127.0.0.1:4242/instances/account-a/openai/v1",
        )
        .expect("prepare instance");
        let config =
            fs::read_to_string(paths.codex_home.join("config.toml")).expect("read instance config");
        assert!(config.contains("model_provider = \"ai-gateway\""));
        assert!(
            config.contains("base_url = \"http://127.0.0.1:4242/instances/account-a/openai/v1\"")
        );
        assert!(config.contains(&format!("CODEX_HOME = \"{}\"", paths.codex_home.display())));
        assert!(!paths.codex_home.join("auth.json").exists());
        assert!(
            fs::symlink_metadata(paths.codex_home.join("skills"))
                .expect("read skills link")
                .file_type()
                .is_symlink()
        );

        fs::write(paths.codex_home.join("config.toml"), "sentinel")
            .expect("change configured instance");
        prepare_codex_instance_at(
            &template,
            &instance_root,
            "http://127.0.0.1:4242/instances/account-a/openai/v1",
        )
        .expect("reuse configured instance");
        assert_eq!(
            fs::read_to_string(paths.codex_home.join("config.toml")).expect("read sentinel"),
            "sentinel"
        );
        assert!(delete_codex_instance_at(&instance_root).expect("delete instance"));
        assert!(!instance_root.exists());
        fs::remove_dir_all(root).expect("remove test directory");
    }

    fn unique_test_dir() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("ai-gateway-codex-config-{unique}"))
    }
}

fn is_gateway_configured(content: &str) -> bool {
    let mut root = true;
    let mut has_marker = false;
    let mut has_provider = false;
    let mut has_gateway_table = false;
    for line in content.lines() {
        if root && is_root_table(line) {
            root = false;
        }
        if root && line.starts_with(MARKER_PREFIX) {
            has_marker = true;
        }
        if root && root_model_provider(line).is_some_and(|value| value.trim() == "\"ai-gateway\"") {
            has_provider = true;
        }
        if is_gateway_table(line) {
            has_gateway_table = true;
        }
    }
    has_marker && has_provider && has_gateway_table
}
