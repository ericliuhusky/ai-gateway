use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, consts::U12},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};

const ENCRYPTED_VALUE_PREFIX: &str = "aigw:v1:";
const NONCE_LENGTH: usize = 12;
const KEY_LENGTH: usize = 32;

/// AES-256-GCM encryption for database values that contain credentials.
///
/// Values use a versioned format:
/// `aigw:v1:<base64url nonce>:<base64url ciphertext-and-auth-tag>`.
#[derive(Clone)]
pub struct FieldEncryptor {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for FieldEncryptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FieldEncryptor(<redacted>)")
    }
}

impl FieldEncryptor {
    /// Generates a Base64-encoded 32-byte AES-256-GCM key, equivalent to
    /// `openssl rand -base64 32`.
    pub fn generate_base64_key() -> Result<String, String> {
        let mut key = [0_u8; KEY_LENGTH];
        getrandom::fill(&mut key).map_err(|_| "generate database encryption key failed")?;
        Ok(STANDARD.encode(key))
    }

    pub fn from_base64_key(encoded_key: &str) -> Result<Self, String> {
        let key = URL_SAFE_NO_PAD
            .decode(encoded_key.trim())
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(encoded_key.trim()))
            .map_err(|_| "数据库加密密钥必须为 Base64 编码".to_string())?;
        if key.len() != KEY_LENGTH {
            return Err("数据库加密密钥解码后必须恰好为 32 字节（AES-256-GCM）".to_string());
        }

        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&key)
                .map_err(|_| "failed to initialize AES-256-GCM".to_string())?,
        })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, String> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        getrandom::fill(&mut nonce).map_err(|_| "generate database encryption nonce failed")?;
        let nonce = Nonce::<U12>::try_from(nonce.as_slice())
            .map_err(|_| "generate database encryption nonce failed".to_string())?;
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| "encrypt database value failed".to_string())?;

        Ok(format!(
            "{ENCRYPTED_VALUE_PREFIX}{}:{}",
            URL_SAFE_NO_PAD.encode(nonce.as_slice()),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ))
    }

    pub fn decrypt(&self, encrypted_value: &str) -> Result<String, String> {
        let encoded_value = encrypted_value
            .strip_prefix(ENCRYPTED_VALUE_PREFIX)
            .ok_or_else(|| {
                "control-plane database credential is not encrypted; re-import it through the control API"
                    .to_string()
            })?;
        let (encoded_nonce, encoded_ciphertext) = encoded_value
            .split_once(':')
            .ok_or_else(|| "encrypted database credential has an invalid format".to_string())?;
        let nonce = URL_SAFE_NO_PAD
            .decode(encoded_nonce)
            .map_err(|_| "encrypted database credential has an invalid nonce".to_string())?;
        if nonce.len() != NONCE_LENGTH {
            return Err("encrypted database credential has an invalid nonce length".to_string());
        }
        let nonce = Nonce::<U12>::try_from(nonce.as_slice())
            .map_err(|_| "encrypted database credential has an invalid nonce".to_string())?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(encoded_ciphertext)
            .map_err(|_| "encrypted database credential has an invalid ciphertext".to_string())?;
        let plaintext = self
            .cipher
            .decrypt(&nonce, ciphertext.as_ref())
            .map_err(|_| {
                "failed to decrypt database credential; verify the database encryption key"
                    .to_string()
            })?;

        String::from_utf8(plaintext)
            .map_err(|_| "decrypted database credential is not valid UTF-8".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{ENCRYPTED_VALUE_PREFIX, FieldEncryptor};

    const TEST_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

    #[test]
    fn encrypts_and_decrypts_a_value() {
        let encryptor = FieldEncryptor::from_base64_key(TEST_KEY).expect("valid test key");
        let encrypted = encryptor.encrypt("secret-value").expect("encrypt");

        assert!(encrypted.starts_with(ENCRYPTED_VALUE_PREFIX));
        assert_ne!(encrypted, "secret-value");
        assert_eq!(encryptor.decrypt(&encrypted).unwrap(), "secret-value");
    }

    #[test]
    fn rejects_tampered_ciphertext() {
        let encryptor = FieldEncryptor::from_base64_key(TEST_KEY).expect("valid test key");
        let mut encrypted = encryptor.encrypt("secret-value").expect("encrypt");
        let ciphertext_start = encrypted.rfind(':').unwrap() + 1;
        let replacement = if encrypted[ciphertext_start..].starts_with('A') {
            "B"
        } else {
            "A"
        };
        encrypted.replace_range(ciphertext_start..ciphertext_start + 1, replacement);

        assert!(encryptor.decrypt(&encrypted).is_err());
    }

    #[test]
    fn generates_valid_database_encryption_keys() {
        let key = FieldEncryptor::generate_base64_key().expect("generate key");
        assert!(FieldEncryptor::from_base64_key(&key).is_ok());
    }
}
