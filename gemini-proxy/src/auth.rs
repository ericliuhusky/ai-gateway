use crate::config::Config;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const OPENAI_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 900;

#[derive(Debug, Clone)]
pub struct OAuthClient {
    http: Client,
    config: Arc<Config>,
    pending_google: Arc<Mutex<HashMap<String, PendingGoogleAuth>>>,
    pending_openai: Arc<Mutex<HashMap<String, PendingOpenAIAuth>>>,
}

#[derive(Debug, Clone)]
struct PendingGoogleAuth {
    redirect_uri: String,
    created_at: u64,
}

#[derive(Debug, Clone)]
struct PendingOpenAIAuth {
    code_verifier: String,
    created_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    #[serde(default)]
    #[allow(dead_code)]
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImportedOpenAIAuth {
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_timestamp: i64,
    pub client_id: String,
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
    #[serde(default)]
    https_api_openai_com_profile: Option<OpenAIProfileClaims>,
}

#[derive(Debug, Deserialize)]
struct OpenAIProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

impl OAuthClient {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            http: Client::new(),
            config,
            pending_google: Arc::new(Mutex::new(HashMap::new())),
            pending_openai: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_auth_url(&self, redirect_uri: String) -> Result<String, String> {
        let state = Uuid::new_v4().to_string();
        self.pending_google.lock().await.insert(
            state.clone(),
            PendingGoogleAuth {
                redirect_uri: redirect_uri.clone(),
                created_at: now_unix(),
            },
        );

        let scopes = [
            "https://www.googleapis.com/auth/cloud-platform",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
            "https://www.googleapis.com/auth/cclog",
            "https://www.googleapis.com/auth/experimentsandconfigs",
        ]
        .join(" ");

        let params = [
            ("client_id", self.config.google_client_id()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", scopes.as_str()),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("include_granted_scopes", "true"),
            ("state", state.as_str()),
        ];

        Url::parse_with_params(AUTH_URL, params)
            .map(|url| url.to_string())
            .map_err(|err| format!("failed to create auth url: {err}"))
    }

    pub async fn create_openai_auth_url(&self) -> Result<String, String> {
        let state = random_urlsafe(32);
        let code_verifier = random_urlsafe(64);
        let code_challenge = pkce_challenge(&code_verifier);

        self.pending_openai.lock().await.insert(
            state.clone(),
            PendingOpenAIAuth {
                code_verifier,
                created_at: now_unix(),
            },
        );

        let params = [
            ("response_type", "code"),
            ("client_id", OPENAI_CLIENT_ID),
            ("redirect_uri", self.config.openai_callback_url()),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", code_challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("state", state.as_str()),
        ];

        Url::parse_with_params(OPENAI_AUTH_URL, params)
            .map(|url| url.to_string())
            .map_err(|err| format!("failed to create openai auth url: {err}"))
    }

    pub async fn consume_redirect_uri(&self, state: &str) -> Result<String, String> {
        self.prune_pending_google().await;
        self.pending_google
            .lock()
            .await
            .remove(state)
            .map(|entry| entry.redirect_uri)
            .ok_or_else(|| "invalid or expired oauth state".to_string())
    }

    pub async fn consume_openai_code_verifier(&self, state: &str) -> Result<String, String> {
        self.prune_pending_openai().await;
        self.pending_openai
            .lock()
            .await
            .remove(state)
            .map(|entry| entry.code_verifier)
            .ok_or_else(|| "invalid or expired openai oauth state".to_string())
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, String> {
        let params = [
            ("client_id", self.config.google_client_id()),
            ("client_secret", self.config.google_client_secret()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ];

        let response = self
            .http
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| format!("token exchange failed: {err}"))?;

        if response.status().is_success() {
            response
                .json::<TokenResponse>()
                .await
                .map_err(|err| format!("token parse failed: {err}"))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(format!("token exchange failed: {body}"))
        }
    }

    pub async fn refresh_google_access_token(
        &self,
        refresh_token: &str,
    ) -> Result<TokenResponse, String> {
        let params = [
            ("client_id", self.config.google_client_id()),
            ("client_secret", self.config.google_client_secret()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let response = self
            .http
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| format!("token refresh failed: {err}"))?;

        if response.status().is_success() {
            response
                .json::<TokenResponse>()
                .await
                .map_err(|err| format!("refresh parse failed: {err}"))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(format!("refresh failed: {body}"))
        }
    }

    pub async fn refresh_openai_access_token(
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
            .map_err(|err| format!("openai token refresh failed: {err}"))?;

        if response.status().is_success() {
            response
                .json::<TokenResponse>()
                .await
                .map_err(|err| format!("openai refresh parse failed: {err}"))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(format!("openai refresh failed: {body}"))
        }
    }

    pub async fn exchange_openai_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, String> {
        let params = [
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CLIENT_ID),
            ("code", code),
            ("redirect_uri", self.config.openai_callback_url()),
            ("code_verifier", code_verifier),
        ];

        let response = self
            .http
            .post(OPENAI_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|err| format!("openai token exchange failed: {err}"))?;

        if response.status().is_success() {
            response
                .json::<TokenResponse>()
                .await
                .map_err(|err| format!("openai token parse failed: {err}"))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(format!("openai token exchange failed: {body}"))
        }
    }

    pub fn openai_auth_from_token_response(
        &self,
        token: TokenResponse,
    ) -> Result<ImportedOpenAIAuth, String> {
        let access_claims = decode_openai_claims(&token.access_token)?;
        let id_claims = token
            .id_token
            .as_deref()
            .map(decode_openai_claims)
            .transpose()?;
        let email = openai_email_from_claims(&access_claims)
            .or_else(|| id_claims.as_ref().and_then(openai_email_from_claims))
            .ok_or_else(|| "failed to determine email from OpenAI token response".to_string())?;
        let expiry_timestamp = access_claims
            .exp
            .ok_or_else(|| "missing exp in OpenAI access token".to_string())?;
        let refresh_token = token
            .refresh_token
            .ok_or_else(|| "openai did not return refresh_token".to_string())?;

        Ok(ImportedOpenAIAuth {
            email,
            access_token: token.access_token,
            refresh_token,
            expiry_timestamp,
            client_id: access_claims
                .client_id
                .unwrap_or_else(|| OPENAI_CLIENT_ID.to_string()),
            scopes: access_claims.scopes,
        })
    }

    pub async fn get_user_info(&self, access_token: &str) -> Result<UserInfo, String> {
        let response = self
            .http
            .get(USERINFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|err| format!("userinfo request failed: {err}"))?;

        if response.status().is_success() {
            response
                .json::<UserInfo>()
                .await
                .map_err(|err| format!("userinfo parse failed: {err}"))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(format!("userinfo failed: {body}"))
        }
    }

    pub fn refresh_needed(&self, expiry_timestamp: i64) -> bool {
        expiry_timestamp <= now_unix() as i64 + TOKEN_REFRESH_SKEW_SECONDS
    }

    async fn prune_pending_google(&self) {
        let cutoff = now_unix().saturating_sub(Duration::from_secs(600).as_secs());
        self.pending_google
            .lock()
            .await
            .retain(|_, pending| pending.created_at >= cutoff);
    }

    async fn prune_pending_openai(&self) {
        let cutoff = now_unix().saturating_sub(Duration::from_secs(600).as_secs());
        self.pending_openai
            .lock()
            .await
            .retain(|_, pending| pending.created_at >= cutoff);
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

fn random_urlsafe(bytes_len: usize) -> String {
    let mut bytes = Vec::with_capacity(bytes_len);
    while bytes.len() < bytes_len {
        bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    bytes.truncate(bytes_len);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}
