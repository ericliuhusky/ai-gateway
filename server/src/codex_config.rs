use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table, value};

const MODEL_PROVIDER_KEY: &str = "model_provider";
const MODEL_PROVIDERS_KEY: &str = "model_providers";
const GATEWAY_PROVIDER_ID: &str = "ai-gateway";
const GATEWAY_PROVIDER_NAME: &str = "ai-gateway";
const GATEWAY_BASE_URL: &str = "http://127.0.0.1:10100/openai/v1";
const GATEWAY_WIRE_API: &str = "responses";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexConfigPatch {
    config_path: PathBuf,
    previous_model_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplySummary {
    pub patch_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreSummary {
    pub restored_model_provider: Option<String>,
    pub patch_removed: bool,
}

pub fn apply_takeover(config_path: &Path, patch_path: &Path) -> Result<ApplySummary, String> {
    let mut doc = read_config_document(config_path)?;

    if !patch_path.exists() {
        write_patch(
            patch_path,
            &CodexConfigPatch {
                config_path: config_path.to_path_buf(),
                previous_model_provider: current_model_provider(&doc),
            },
        )?;
    }

    ensure_gateway_provider_table(&mut doc)?;
    doc[MODEL_PROVIDERS_KEY][GATEWAY_PROVIDER_ID]["name"] = value(GATEWAY_PROVIDER_NAME);
    doc[MODEL_PROVIDERS_KEY][GATEWAY_PROVIDER_ID]["base_url"] = value(GATEWAY_BASE_URL);
    doc[MODEL_PROVIDERS_KEY][GATEWAY_PROVIDER_ID]["wire_api"] = value(GATEWAY_WIRE_API);
    doc[MODEL_PROVIDER_KEY] = value(GATEWAY_PROVIDER_ID);

    write_config_document(config_path, &doc)?;

    Ok(ApplySummary {
        patch_path: patch_path.display().to_string(),
    })
}

pub fn restore_takeover(config_path: &Path, patch_path: &Path) -> Result<RestoreSummary, String> {
    if !patch_path.exists() {
        return Err("no Codex config patch available".to_string());
    }

    let patch = read_patch(patch_path)?;
    let mut doc = read_config_document(config_path)?;

    remove_gateway_provider(&mut doc);
    match patch.previous_model_provider.as_deref() {
        Some(provider) => doc[MODEL_PROVIDER_KEY] = value(provider),
        None => {
            doc.as_table_mut().remove(MODEL_PROVIDER_KEY);
        }
    }

    write_config_document(config_path, &doc)?;
    fs::remove_file(patch_path)
        .map_err(|err| format!("failed to remove Codex config patch: {err}"))?;

    Ok(RestoreSummary {
        restored_model_provider: patch.previous_model_provider,
        patch_removed: true,
    })
}

pub fn patch_exists(patch_path: &Path) -> bool {
    patch_path.exists()
}

fn read_config_document(config_path: &Path) -> Result<DocumentMut, String> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(format!(
                "failed to read Codex config {}: {err}",
                config_path.display()
            ));
        }
    };

    if content.trim().is_empty() {
        return Ok(DocumentMut::new());
    }

    content.parse::<DocumentMut>().map_err(|err| {
        format!(
            "failed to parse Codex config {}: {err}",
            config_path.display()
        )
    })
}

fn write_config_document(config_path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create Codex config directory {}: {err}",
                parent.display()
            )
        })?;
    }

    fs::write(config_path, doc.to_string()).map_err(|err| {
        format!(
            "failed to write Codex config {}: {err}",
            config_path.display()
        )
    })
}

fn write_patch(patch_path: &Path, patch: &CodexConfigPatch) -> Result<(), String> {
    if let Some(parent) = patch_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create Codex config patch directory {}: {err}",
                parent.display()
            )
        })?;
    }
    let content = serde_json::to_string_pretty(patch)
        .map_err(|err| format!("failed to serialize Codex config patch: {err}"))?;
    fs::write(patch_path, content).map_err(|err| {
        format!(
            "failed to write Codex config patch {}: {err}",
            patch_path.display()
        )
    })
}

fn read_patch(patch_path: &Path) -> Result<CodexConfigPatch, String> {
    let content = fs::read_to_string(patch_path).map_err(|err| {
        format!(
            "failed to read Codex config patch {}: {err}",
            patch_path.display()
        )
    })?;
    let patch: CodexConfigPatch = serde_json::from_str(&content)
        .map_err(|err| format!("failed to parse Codex config patch: {err}"))?;
    Ok(patch)
}

fn current_model_provider(doc: &DocumentMut) -> Option<String> {
    doc.get(MODEL_PROVIDER_KEY)
        .and_then(Item::as_str)
        .map(str::to_string)
}

fn ensure_gateway_provider_table(doc: &mut DocumentMut) -> Result<(), String> {
    if !doc.as_table().contains_key(MODEL_PROVIDERS_KEY) {
        doc[MODEL_PROVIDERS_KEY] = Item::Table(Table::new());
    }
    let providers = doc[MODEL_PROVIDERS_KEY]
        .as_table_mut()
        .ok_or("Codex config `model_providers` must be a table")?;
    if !providers.contains_key(GATEWAY_PROVIDER_ID) {
        providers[GATEWAY_PROVIDER_ID] = Item::Table(Table::new());
    }
    providers[GATEWAY_PROVIDER_ID]
        .as_table_mut()
        .ok_or("Codex config `model_providers.ai-gateway` must be a table")?;
    Ok(())
}

fn remove_gateway_provider(doc: &mut DocumentMut) {
    let Some(providers) = doc
        .get_mut(MODEL_PROVIDERS_KEY)
        .and_then(Item::as_table_mut)
    else {
        return;
    };

    providers.remove(GATEWAY_PROVIDER_ID);
    if providers.is_empty() {
        doc.as_table_mut().remove(MODEL_PROVIDERS_KEY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn apply_takeover_sets_gateway_provider_and_model_provider() {
        let dir = test_dir("apply");
        let config_path = dir.join("config.toml");
        let patch_path = dir.join("patch.json");
        fs::write(
            &config_path,
            r#"model = "gpt-5"
disable_response_storage = false

[mcp_servers.demo]
command = "demo"
"#,
        )
        .expect("write config");

        apply_takeover(&config_path, &patch_path).expect("apply takeover");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains(r#"model = "gpt-5""#));
        assert!(content.contains("disable_response_storage = false"));
        assert!(content.contains("[mcp_servers.demo]"));
        assert!(content.contains(r#"model_provider = "ai-gateway""#));
        assert!(content.contains("[model_providers.ai-gateway]"));
        assert!(content.contains(r#"name = "ai-gateway""#));
        assert!(content.contains(r#"base_url = "http://127.0.0.1:10100/openai/v1""#));
        assert!(content.contains(r#"wire_api = "responses""#));
    }

    #[test]
    fn restore_takeover_removes_gateway_provider_and_restores_previous_provider() {
        let dir = test_dir("restore");
        let config_path = dir.join("config.toml");
        let patch_path = dir.join("patch.json");
        fs::write(
            &config_path,
            r#"model_provider = "openai"
model = "gpt-5"
"#,
        )
        .expect("write config");

        apply_takeover(&config_path, &patch_path).expect("apply takeover");
        restore_takeover(&config_path, &patch_path).expect("restore takeover");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains(r#"model_provider = "openai""#));
        assert!(content.contains(r#"model = "gpt-5""#));
        assert!(!content.contains("[model_providers.ai-gateway]"));
        assert!(!patch_path.exists());
    }

    #[test]
    fn restore_takeover_removes_model_provider_when_it_was_absent() {
        let dir = test_dir("restore-absent");
        let config_path = dir.join("config.toml");
        let patch_path = dir.join("patch.json");
        fs::write(&config_path, r#"model = "gpt-5""#).expect("write config");

        apply_takeover(&config_path, &patch_path).expect("apply takeover");
        restore_takeover(&config_path, &patch_path).expect("restore takeover");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_provider"));
        assert!(content.contains(r#"model = "gpt-5""#));
    }

    #[test]
    fn repeated_apply_preserves_original_restore_point() {
        let dir = test_dir("repeat");
        let config_path = dir.join("config.toml");
        let patch_path = dir.join("patch.json");
        fs::write(&config_path, r#"model_provider = "openai""#).expect("write config");

        apply_takeover(&config_path, &patch_path).expect("first apply");
        apply_takeover(&config_path, &patch_path).expect("second apply");
        restore_takeover(&config_path, &patch_path).expect("restore takeover");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains(r#"model_provider = "openai""#));
    }

    fn test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ai-gateway-codex-config-{name}-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
