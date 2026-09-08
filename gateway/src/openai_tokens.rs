use crate::models::ChatGPTAuthRecord;
use crate::support::time::now_unix;
use crate::upstream::build_http_client;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Client;
use serde::Deserialize;

const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 900;

#[derive(Debug, Clone)]
pub struct OpenAiTokenService {
    http: Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAITokenClaims {
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, rename = "https://api.openai.com/profile")]
    profile: Option<OpenAIProfileClaims>,
}

#[derive(Debug, Deserialize)]
struct OpenAIProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

impl OpenAiTokenService {
    pub fn new() -> Self {
        Self {
            http: build_http_client(),
        }
    }

    pub async fn refresh_access_token(
        &self,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<TokenResponse, String> {
        let params = [
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let response = self
            .http
            .post(OPENAI_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| format!("刷新 OpenAI Token 失败：{err}"))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("刷新 OpenAI Token 失败：{body}"));
        }

        let token = response
            .json::<TokenResponse>()
            .await
            .map_err(|err| format!("解析 OpenAI 刷新响应失败：{err}"))?;
        Ok(token)
    }

    pub fn import_codex_tokens(
        &self,
        access_token: String,
        refresh_token: String,
        account_id_hint: Option<String>,
    ) -> Result<ChatGPTAuthRecord, String> {
        let access_claims = decode_openai_claims(&access_token)?;
        let email = openai_email_from_claims(&access_claims)
            .ok_or_else(|| "无法从粘贴的 Codex Token 中确定邮箱".to_string())?;
        let expiry_timestamp = access_claims
            .exp
            .ok_or_else(|| "OpenAI 访问 Token 缺少 exp 字段".to_string())?;
        Ok(ChatGPTAuthRecord::new(
            email,
            access_token,
            refresh_token,
            expiry_timestamp,
            Some(
                access_claims
                    .client_id
                    .clone()
                    .unwrap_or_else(|| CODEX_CLIENT_ID.to_string()),
            ),
            account_id_hint,
        ))
    }

    pub fn refresh_needed(&self, expiry_timestamp: i64) -> bool {
        expiry_timestamp <= now_unix() as i64 + TOKEN_REFRESH_SKEW_SECONDS
    }
}

fn decode_openai_claims(token: &str) -> Result<OpenAITokenClaims, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "JWT 载荷无效".to_string())?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|err| format!("解码 JWT 载荷失败：{err}"))?;
    serde_json::from_slice::<OpenAITokenClaims>(&bytes)
        .map_err(|err| format!("解析 JWT 载荷失败：{err}"))
}

fn openai_email_from_claims(claims: &OpenAITokenClaims) -> Option<String> {
    claims.email.clone().or_else(|| {
        claims
            .profile
            .as_ref()
            .and_then(|profile| profile.email.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::{decode_openai_claims, openai_email_from_claims};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    #[test]
    fn extracts_supported_claims_from_access_token() {
        let payload = serde_json::json!({
            "exp": 1_700_000_000,
            "client_id": "app_test",
            "email": "user@example.com",
        });
        let payload_encoded = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let token = format!("header.{payload_encoded}.sig");

        let claims = decode_openai_claims(&token).expect("should decode jwt payload");
        assert_eq!(
            openai_email_from_claims(&claims).as_deref(),
            Some("user@example.com")
        );
        assert_eq!(claims.exp, Some(1_700_000_000));
        assert_eq!(claims.client_id.as_deref(), Some("app_test"));
    }

    #[test]
    fn extracts_email_from_openai_profile_claim() {
        let payload = serde_json::json!({
            "exp": 1_700_000_000,
            "https://api.openai.com/profile": {
                "email": "user@example.com",
                "email_verified": true,
            },
        });
        let payload_encoded = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let token = format!("header.{payload_encoded}.sig");

        let claims = decode_openai_claims(&token).expect("should decode jwt payload");
        assert_eq!(
            openai_email_from_claims(&claims).as_deref(),
            Some("user@example.com")
        );
    }
}
