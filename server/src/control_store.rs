use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

const CONFIG_FILE: &str = "config.json";
const SECRET_FILE: &str = "credentials.json.enc";
const MASTER_KEY_FILE: &str = "credentials.key";

#[derive(Clone)]
pub struct LocalStore {
    root: PathBuf,
    inner: Arc<Mutex<StoreState>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalGatewayStatus {
    pub local_gateway_url: String,
    pub control_plane_url: Option<String>,
    pub device_id: String,
    pub sharing_configured: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PublicConfig {
    device_id: String,
    #[serde(default)]
    control_plane_url: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SecretConfig {
    #[serde(default)]
    control_access_token: Option<String>,
}

struct StoreState {
    config: PublicConfig,
    secrets: SecretConfig,
}

impl LocalStore {
    pub fn open() -> Result<Self, String> {
        let home = env::var_os("HOME").ok_or_else(|| "未设置 HOME 环境变量".to_string())?;
        let root = PathBuf::from(home).join(".ai-gateway").join("config");
        Self::open_at(root, true)
    }

    pub(crate) fn open_at(root: PathBuf, apply_environment: bool) -> Result<Self, String> {
        fs::create_dir_all(&root)
            .map_err(|error| format!("创建本地 Gateway 配置目录失败：{error}"))?;

        let config_path = root.join(CONFIG_FILE);
        let config = if config_path.exists() {
            // Unknown fields from the former all-in-one client store are
            // intentionally ignored. Provider state belongs exclusively
            // to Gateway Server's encrypted SQLite database.
            read_json::<PublicConfig>(&config_path)?
        } else {
            PublicConfig {
                device_id: Uuid::new_v4().to_string(),
                control_plane_url: None,
            }
        };

        let master_key = load_or_create_master_key(&root)?;
        let secret_path = root.join(SECRET_FILE);
        let secrets = if secret_path.exists() {
            // serde ignores the legacy `providers` field. Rewriting below
            // removes the duplicate credential copy after migration.
            decrypt_secret_file(&secret_path, &master_key)?
        } else {
            SecretConfig::default()
        };

        let store = Self {
            root,
            inner: Arc::new(Mutex::new(StoreState { config, secrets })),
        };
        if apply_environment {
            store.apply_environment_configuration()?;
        }
        {
            let state = store.lock()?;
            store.persist(&state)?;
        }
        Ok(store)
    }

    pub fn status(&self, local_gateway_url: &str) -> Result<LocalGatewayStatus, String> {
        let state = self.lock()?;
        Ok(LocalGatewayStatus {
            local_gateway_url: local_gateway_url.to_string(),
            control_plane_url: state.config.control_plane_url.clone(),
            device_id: state.config.device_id.clone(),
            sharing_configured: sharing_is_configured(&state),
        })
    }

    pub fn configure_control_plane(&self, url: String, access_token: String) -> Result<(), String> {
        let normalized_url = normalize_control_plane_url(&url)?;
        let access_token = access_token.trim();
        if access_token.is_empty() {
            return Err("中心服务访问令牌不能为空".to_string());
        }
        let mut state = self.lock()?;
        state.config.control_plane_url = Some(normalized_url);
        state.secrets.control_access_token = Some(access_token.to_string());
        self.persist(&state)
    }

    pub fn clear_control_plane(&self) -> Result<(), String> {
        let mut state = self.lock()?;
        state.config.control_plane_url = None;
        state.secrets.control_access_token = None;
        self.persist(&state)
    }

    pub fn control_plane_credentials(&self) -> Result<(String, String, String), String> {
        let state = self.lock()?;
        let url = state
            .config
            .control_plane_url
            .clone()
            .ok_or_else(|| "尚未配置中心服务地址".to_string())?;
        let token = state
            .secrets
            .control_access_token
            .clone()
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| "尚未配置中心服务访问令牌".to_string())?;
        Ok((url, token, state.config.device_id.clone()))
    }

    pub fn sharing_is_configured(&self) -> bool {
        self.lock()
            .map(|state| sharing_is_configured(&state))
            .unwrap_or(false)
    }

    fn apply_environment_configuration(&self) -> Result<(), String> {
        let Ok(url) = env::var("AI_GATEWAY_CONTROL_URL") else {
            return Ok(());
        };
        let Ok(token) = env::var("AI_GATEWAY_CONTROL_TOKEN") else {
            return Ok(());
        };
        self.configure_control_plane(url, token)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StoreState>, String> {
        self.inner
            .lock()
            .map_err(|_| "本地凭据存储锁异常".to_string())
    }

    fn persist(&self, state: &StoreState) -> Result<(), String> {
        write_json(&self.root.join(CONFIG_FILE), &state.config)?;
        let key = load_or_create_master_key(&self.root)?;
        encrypt_secret_file(&self.root.join(SECRET_FILE), &key, &state.secrets)
    }
}

fn sharing_is_configured(state: &StoreState) -> bool {
    state.config.control_plane_url.is_some()
        && state
            .secrets
            .control_access_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty())
}

pub(crate) fn normalize_control_plane_url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    let url =
        reqwest::Url::parse(value).map_err(|_| "服务地址必须是有效的 http(s) URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("服务地址必须是有效的 http(s) URL".to_string());
    }
    Ok(value.to_string())
}

fn load_or_create_master_key(root: &Path) -> Result<[u8; 32], String> {
    let path = root.join(MASTER_KEY_FILE);
    if path.exists() {
        let encoded =
            fs::read_to_string(&path).map_err(|error| format!("读取本地凭据密钥失败：{error}"))?;
        let decoded = BASE64
            .decode(encoded.trim())
            .map_err(|_| "本地凭据密钥格式无效".to_string())?;
        return decoded
            .try_into()
            .map_err(|_| "本地凭据密钥长度无效".to_string());
    }
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).map_err(|_| "生成本地凭据密钥失败".to_string())?;
    write_private_file(&path, BASE64.encode(key).as_bytes())?;
    Ok(key)
}

fn encrypt_secret_file(path: &Path, key: &[u8; 32], secret: &SecretConfig) -> Result<(), String> {
    let plaintext =
        serde_json::to_vec(secret).map_err(|error| format!("序列化本地凭据失败：{error}"))?;
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| "初始化本地凭据加密失败".to_string())?;
    let mut nonce = [0_u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| "生成本地凭据随机数失败".to_string())?;
    let ciphertext = cipher
        .encrypt(&Nonce::from(nonce), plaintext.as_ref())
        .map_err(|_| "加密本地凭据失败".to_string())?;
    let encoded = serde_json::json!({
        "version": 1,
        "nonce": BASE64.encode(nonce),
        "ciphertext": BASE64.encode(ciphertext),
    });
    let body =
        serde_json::to_vec(&encoded).map_err(|error| format!("保存本地凭据失败：{error}"))?;
    write_private_file(path, &body)
}

fn decrypt_secret_file(path: &Path, key: &[u8; 32]) -> Result<SecretConfig, String> {
    let encrypted: EncryptedSecretFile = read_json(path)?;
    if encrypted.version != 1 {
        return Err("本地凭据文件版本不受支持".to_string());
    }
    let nonce = BASE64
        .decode(encrypted.nonce)
        .map_err(|_| "本地凭据随机数格式无效".to_string())?;
    let nonce: [u8; 12] = nonce
        .try_into()
        .map_err(|_| "本地凭据随机数长度无效".to_string())?;
    let ciphertext = BASE64
        .decode(encrypted.ciphertext)
        .map_err(|_| "本地凭据密文格式无效".to_string())?;
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| "初始化本地凭据解密失败".to_string())?;
    let plaintext = cipher
        .decrypt(&Nonce::from(nonce), ciphertext.as_ref())
        .map_err(|_| "无法解密本地凭据".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|error| format!("读取本地凭据失败：{error}"))
}

#[derive(Deserialize)]
struct EncryptedSecretFile {
    version: u8,
    nonce: String,
    ciphertext: String,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let content =
        fs::read(path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    serde_json::from_slice(&content)
        .map_err(|error| format!("解析 {} 失败：{error}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("序列化 {} 失败：{error}", path.display()))?;
    write_private_file(path, &content)
}

fn write_private_file(path: &Path, content: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)
        .map_err(|error| format!("写入 {} 失败：{error}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置 {} 权限失败：{error}", temporary.display()))?;
    }
    fs::rename(&temporary, path).map_err(|error| format!("保存 {} 失败：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn persists_control_token_only_in_encrypted_secret_file() {
        let root = unique_test_dir();
        let store = LocalStore::open_at(root.clone(), false).expect("open local store");
        store
            .configure_control_plane(
                "https://control.example.com/".to_string(),
                "agw_super_secret".to_string(),
            )
            .expect("configure control plane");

        let public_config =
            fs::read_to_string(root.join(CONFIG_FILE)).expect("read public configuration");
        assert!(public_config.contains("https://control.example.com"));
        assert!(!public_config.contains("agw_super_secret"));

        let encrypted = fs::read_to_string(root.join(SECRET_FILE)).expect("read encrypted secrets");
        assert!(!encrypted.contains("agw_super_secret"));
        assert!(encrypted.contains("\"ciphertext\""));

        let reopened = LocalStore::open_at(root.clone(), false).expect("reopen local store");
        let (url, token, device_id) = reopened
            .control_plane_credentials()
            .expect("load control credentials");
        assert_eq!(url, "https://control.example.com");
        assert_eq!(token, "agw_super_secret");
        assert!(!device_id.is_empty());

        reopened
            .clear_control_plane()
            .expect("clear control-plane configuration");
        assert!(!reopened.sharing_is_configured());
        assert!(reopened.control_plane_credentials().is_err());

        let _ = fs::remove_dir_all(root);
    }

    fn unique_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ai_gateway_server_store_{unique}"))
    }
}
