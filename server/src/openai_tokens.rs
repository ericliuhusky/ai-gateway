use crate::support::time::now_unix;
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
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImportedOpenAIAuth {
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_timestamp: i64,
    pub client_id: String,
    pub account_id: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAITokenClaims {
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, rename = "scp")]
    scopes: Vec<String>,
    #[serde(default, rename = "https://api.openai.com/profile")]
    https_api_openai_com_profile: Option<OpenAIProfileClaims>,
    #[serde(default, rename = "https://api.openai.com/auth")]
    https_api_openai_com_auth: Option<OpenAIAuthClaims>,
}

#[derive(Debug, Deserialize)]
struct OpenAIProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIAuthClaims {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    chatgpt_account_user_id: Option<String>,
    #[serde(default)]
    chatgpt_user_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
}

impl OpenAiTokenService {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
    }

    pub async fn refresh_access_token(
        &self,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<RefreshedOpenAiToken, String> {
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
            .map_err(|err| format!("OpenAI token refresh failed: {err}"))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("OpenAI token refresh failed: {body}"));
        }

        let token = response
            .json::<TokenResponse>()
            .await
            .map_err(|err| format!("OpenAI refresh response parse failed: {err}"))?;
        Ok(RefreshedOpenAiToken {
            access_token: token.access_token,
            expires_in: token.expires_in,
            refresh_token: token.refresh_token,
        })
    }

    pub fn import_codex_tokens(
        &self,
        access_token: String,
        refresh_token: String,
        id_token: Option<String>,
        account_id_hint: Option<String>,
    ) -> Result<ImportedOpenAIAuth, String> {
        let access_claims = decode_openai_claims(&access_token)?;
        let id_claims = id_token.as_deref().map(decode_openai_claims).transpose()?;
        let email = openai_email_from_claims(&access_claims)
            .or_else(|| id_claims.as_ref().and_then(openai_email_from_claims))
            .ok_or_else(|| "failed to determine email from pasted Codex tokens".to_string())?;
        let expiry_timestamp = access_claims
            .exp
            .ok_or_else(|| "missing exp in OpenAI access token".to_string())?;
        let account_id = openai_account_id_from_claims(&access_claims)
            .or_else(|| id_claims.as_ref().and_then(openai_account_id_from_claims))
            .or(account_id_hint);

        Ok(ImportedOpenAIAuth {
            email,
            access_token,
            refresh_token,
            expiry_timestamp,
            client_id: access_claims
                .client_id
                .clone()
                .unwrap_or_else(|| CODEX_CLIENT_ID.to_string()),
            account_id,
            scopes: access_claims.scopes,
        })
    }

    pub fn refresh_needed(&self, expiry_timestamp: i64) -> bool {
        expiry_timestamp <= now_unix() as i64 + TOKEN_REFRESH_SKEW_SECONDS
    }
}

#[derive(Debug, Clone)]
pub struct RefreshedOpenAiToken {
    pub access_token: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
}

fn decode_openai_claims(token: &str) -> Result<OpenAITokenClaims, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "invalid jwt payload".to_string())?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|err| format!("failed to decode jwt payload: {err}"))?;
    serde_json::from_slice::<OpenAITokenClaims>(&bytes)
        .map_err(|err| format!("failed to parse jwt payload: {err}"))
}

fn openai_email_from_claims(claims: &OpenAITokenClaims) -> Option<String> {
    claims.email.clone().or_else(|| {
        claims
            .https_api_openai_com_profile
            .as_ref()
            .and_then(|profile| profile.email.clone())
    })
}

fn openai_account_id_from_claims(claims: &OpenAITokenClaims) -> Option<String> {
    claims.https_api_openai_com_auth.as_ref().and_then(|auth| {
        auth.chatgpt_account_id
            .clone()
            .or_else(|| auth.chatgpt_account_user_id.clone())
            .or_else(|| auth.chatgpt_user_id.clone())
            .or_else(|| auth.user_id.clone())
    })
}

pub fn extract_openai_chatgpt_account_id(access_token: &str) -> Option<String> {
    let claims = decode_openai_claims(access_token).ok()?;
    openai_account_id_from_claims(&claims)
}

#[cfg(test)]
mod tests {
    use super::{decode_openai_claims, openai_account_id_from_claims, openai_email_from_claims};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    #[test]
    fn extracts_openai_identity_from_url_claims() {
        let payload = serde_json::json!({
            "exp": 1_700_000_000,
            "client_id": "app_test",
            "email": "user@example.com",
            "scp": ["openid", "profile"],
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc_123"
            },
            "https://api.openai.com/profile": {
                "email": "profile@example.com"
            }
        });
        let payload_encoded = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let token = format!("header.{payload_encoded}.sig");

        let claims = decode_openai_claims(&token).expect("should decode jwt payload");
        assert_eq!(
            openai_account_id_from_claims(&claims).as_deref(),
            Some("acc_123")
        );
        assert_eq!(
            openai_email_from_claims(&claims).as_deref(),
            Some("user@example.com")
        );
    }
}
