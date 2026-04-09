use crate::config::Config;
use reqwest::Client;
use serde::Deserialize;
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
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 900;

#[derive(Debug, Clone)]
pub struct OAuthClient {
    http: Client,
    config: Arc<Config>,
    pending: Arc<Mutex<HashMap<String, PendingAuth>>>,
}

#[derive(Debug, Clone)]
struct PendingAuth {
    redirect_uri: String,
    created_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
    pub email: String,
    pub name: Option<String>,
}

impl OAuthClient {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            http: Client::new(),
            config,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_auth_url(&self, redirect_uri: String) -> Result<String, String> {
        let state = Uuid::new_v4().to_string();
        self.pending.lock().await.insert(
            state.clone(),
            PendingAuth {
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

    pub async fn consume_redirect_uri(&self, state: &str) -> Result<String, String> {
        self.prune_pending().await;
        self.pending
            .lock()
            .await
            .remove(state)
            .map(|entry| entry.redirect_uri)
            .ok_or_else(|| "invalid or expired oauth state".to_string())
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

    pub async fn refresh_access_token(&self, refresh_token: &str) -> Result<TokenResponse, String> {
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

    async fn prune_pending(&self) {
        let cutoff = now_unix().saturating_sub(Duration::from_secs(600).as_secs());
        self.pending
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
