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
    let (next, _) = configure_gateway_config(&source, &gateway_base_url);
    let changed = write_if_changed(&config_path, next.as_bytes())?;
    Ok(CodexConfigurationResult { changed })
}

pub fn stop_default_codex() -> Result<CodexConfigurationResult, String> {
    let codex_dir = codex_dir()?;
    let config_path = codex_dir.join("config.toml");
    let source = match read_optional(&config_path)? {
        Some(source) => source,
        None => {
            return Ok(CodexConfigurationResult { changed: false });
        }
    };
    let _lock = ConfigLock::acquire(&codex_dir)?;
    let next = restore_gateway_config(&source)?;
    let mut changed = write_if_changed(&config_path, next.as_bytes())?;
    changed |= restore_authentication(&codex_dir)?;
    Ok(CodexConfigurationResult { changed })
}

fn codex_dir() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or_else(|| "未设置 HOME 环境变量".to_string())?;
    Ok(PathBuf::from(home).join(".codex"))
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
    let source = source.as_deref().unwrap_or_default();
    let previous_provider = detect_previous_provider(source);
    let mut root = Vec::new();
    let mut rest = Vec::new();
    let mut in_root = true;
    let mut skipping_gateway = false;
    let mut marker: Option<String> = None;
    let mut first_provider: Option<String> = None;

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_and_restore_preserve_root_configuration() {
        let source = "model_provider = \"openai\"\nmodel = \"gpt-5\"\n[features]\nweb_search_request = true\n";
        let (configured, previous) =
            configure_gateway_config(&Some(source.to_string()), "http://127.0.0.1:42401/v1");
        assert_eq!(previous, "openai");
        assert!(is_gateway_configured(&configured));
        assert!(configured.contains("wire_api = \"responses\""));
        let restored = restore_gateway_config(&configured).expect("restore configuration");
        assert_eq!(restored, source);
    }
    #[test]
    fn setup_is_idempotent() {
        let (first, _) = configure_gateway_config(&None, "http://127.0.0.1:42401/v1");
        let (second, _) =
            configure_gateway_config(&Some(first.clone()), "http://127.0.0.1:42401/v1");
        assert_eq!(first, second);
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
