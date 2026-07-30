use crate::openai_tokens::{ImportedOpenAIAuth, OpenAiTokenService};
use crate::support::time::now_unix;
use crate::upstream::build_http_client;
use reqwest::Client;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const DEVICE_TOKEN_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;
const DEFAULT_EXPIRY_SECONDS: i64 = 15 * 60;

#[derive(Clone, Debug)]
pub struct OpenAiDeviceLoginService {
    http: Client,
    sessions: Arc<Mutex<HashMap<String, DeviceLoginSession>>>,
}

#[derive(Clone, Debug)]
struct DeviceLoginSession {
    user_code: String,
    device_auth_id: String,
    verification_uri: String,
    interval_seconds: u64,
    expires_at: i64,
    last_polled_at: i64,
    status: DeviceLoginStatus,
}

#[derive(Clone, Debug)]
enum DeviceLoginStatus {
    Pending,
    Ready(DeviceAuthorization),
    Finalizing,
    Completed(DeviceLoginCompletion),
    Failed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct DeviceAuthorization {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Clone, Debug)]
pub struct DeviceLoginStart {
    pub login_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval_seconds: u64,
    pub expires_in: i64,
}

#[derive(Clone, Debug)]
pub enum DeviceLoginPoll {
    Pending(DeviceLoginStart),
    Ready,
    Finalizing,
    Completed(DeviceLoginCompletion),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct DeviceLoginCompletion {
    pub email: String,
    pub account_id: String,
    pub has_responses_write: bool,
}

#[derive(Debug, Deserialize)]
struct DeviceUserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: Option<IntervalValue>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IntervalValue {
    Number(u64),
    String(String),
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    id_token: Option<String>,
}

impl OpenAiDeviceLoginService {
    pub fn new() -> Self {
        Self {
            http: build_http_client(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start(&self) -> Result<DeviceLoginStart, String> {
        self.prune_expired().await;

        let response = self
            .http
            .post(DEVICE_USER_CODE_URL)
            .json(&serde_json::json!({ "client_id": CODEX_CLIENT_ID }))
            .send()
            .await
            .map_err(|error| format!("OpenAI device login request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("OpenAI device login response read failed: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "OpenAI device login request failed ({status}): {}",
                truncate_error_body(&body)
            ));
        }

        let payload: DeviceUserCodeResponse = serde_json::from_str(&body)
            .map_err(|error| format!("OpenAI device login response parse failed: {error}"))?;
        if payload.device_auth_id.trim().is_empty() || payload.user_code.trim().is_empty() {
            return Err("OpenAI device login response did not include a user code".to_string());
        }

        let interval_seconds = parse_interval(payload.interval);
        let expires_in = payload.expires_in.unwrap_or(DEFAULT_EXPIRY_SECONDS).max(1);
        let login_id = Uuid::new_v4().to_string();
        let start = DeviceLoginStart {
            login_id: login_id.clone(),
            user_code: payload.user_code,
            verification_uri: DEVICE_VERIFICATION_URL.to_string(),
            interval_seconds,
            expires_in,
        };
        self.sessions.lock().await.insert(
            login_id,
            DeviceLoginSession {
                user_code: start.user_code.clone(),
                device_auth_id: payload.device_auth_id,
                verification_uri: start.verification_uri.clone(),
                interval_seconds,
                expires_at: now_unix() as i64 + expires_in,
                last_polled_at: 0,
                status: DeviceLoginStatus::Pending,
            },
        );

        Ok(start)
    }

    pub async fn poll(&self, login_id: &str) -> Result<DeviceLoginPoll, String> {
        let session = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(login_id)
                .ok_or_else(|| "登录会话不存在或已过期，请重新开始".to_string())?;
            if session.expires_at <= now_unix() as i64 {
                session.status = DeviceLoginStatus::Failed("登录已超时，请重新开始".to_string());
            }

            match &session.status {
                DeviceLoginStatus::Ready(_) => return Ok(DeviceLoginPoll::Ready),
                DeviceLoginStatus::Finalizing => return Ok(DeviceLoginPoll::Finalizing),
                DeviceLoginStatus::Completed(completion) => {
                    return Ok(DeviceLoginPoll::Completed(completion.clone()));
                }
                DeviceLoginStatus::Failed(error) => {
                    return Ok(DeviceLoginPoll::Failed(error.clone()));
                }
                DeviceLoginStatus::Pending => {}
            }

            let now = now_unix() as i64;
            if now - session.last_polled_at < session.interval_seconds as i64 {
                return Ok(DeviceLoginPoll::Pending(start_from_session(
                    login_id, session,
                )));
            }
            session.last_polled_at = now;
            (
                session.device_auth_id.clone(),
                session.user_code.clone(),
                session.interval_seconds,
            )
        };

        let poll_result = self.request_device_token(&session.0, &session.1).await;
        let mut sessions = self.sessions.lock().await;
        let current = sessions
            .get_mut(login_id)
            .ok_or_else(|| "登录会话不存在或已过期，请重新开始".to_string())?;

        match poll_result {
            Ok(Some(authorization)) => {
                current.status = DeviceLoginStatus::Ready(authorization.clone());
                Ok(DeviceLoginPoll::Ready)
            }
            Ok(None) => Ok(DeviceLoginPoll::Pending(start_from_session(
                login_id, current,
            ))),
            Err(error) => {
                current.status = DeviceLoginStatus::Failed(error.clone());
                Ok(DeviceLoginPoll::Failed(error))
            }
        }
    }

    pub async fn begin_finalization(
        &self,
        login_id: &str,
    ) -> Result<Option<DeviceAuthorization>, String> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(login_id)
            .ok_or_else(|| "登录会话不存在或已过期，请重新开始".to_string())?;
        match &session.status {
            DeviceLoginStatus::Ready(authorization) => {
                let authorization = authorization.clone();
                session.status = DeviceLoginStatus::Finalizing;
                Ok(Some(authorization))
            }
            DeviceLoginStatus::Finalizing => Ok(None),
            DeviceLoginStatus::Completed(_) => Ok(None),
            DeviceLoginStatus::Failed(error) => Err(error.clone()),
            DeviceLoginStatus::Pending => Ok(None),
        }
    }

    pub async fn exchange_authorization(
        &self,
        authorization: &DeviceAuthorization,
        tokens: &OpenAiTokenService,
    ) -> Result<ImportedOpenAIAuth, String> {
        let params = [
            ("grant_type", "authorization_code"),
            ("code", authorization.authorization_code.as_str()),
            ("redirect_uri", DEVICE_TOKEN_REDIRECT_URI),
            ("client_id", CODEX_CLIENT_ID),
            ("code_verifier", authorization.code_verifier.as_str()),
        ];
        let response = self
            .http
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|error| format!("OpenAI token exchange failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("OpenAI token exchange response read failed: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "OpenAI token exchange failed ({status}): {}",
                truncate_error_body(&body)
            ));
        }

        let payload: OAuthTokenResponse = serde_json::from_str(&body)
            .map_err(|error| format!("OpenAI token exchange response parse failed: {error}"))?;
        tokens.import_codex_tokens(
            payload.access_token,
            payload.refresh_token,
            payload.id_token,
            None,
        )
    }

    pub async fn complete(&self, login_id: &str, completion: DeviceLoginCompletion) {
        if let Some(session) = self.sessions.lock().await.get_mut(login_id) {
            session.status = DeviceLoginStatus::Completed(completion);
        }
    }

    pub async fn fail(&self, login_id: &str, error: String) {
        if let Some(session) = self.sessions.lock().await.get_mut(login_id) {
            session.status = DeviceLoginStatus::Failed(error);
        }
    }

    pub async fn cancel(&self, login_id: &str) -> Result<(), String> {
        let removed = self.sessions.lock().await.remove(login_id);
        if removed.is_none() {
            return Err("登录会话不存在或已结束".to_string());
        }
        Ok(())
    }

    async fn request_device_token(
        &self,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<Option<DeviceAuthorization>, String> {
        let response = self
            .http
            .post(DEVICE_TOKEN_URL)
            .json(&serde_json::json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .await
            .map_err(|error| format!("OpenAI device login polling failed: {error}"))?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            format!("OpenAI device login polling response read failed: {error}")
        })?;
        if status.as_u16() == 403 || status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(format!(
                "OpenAI device login polling failed ({status}): {}",
                truncate_error_body(&body)
            ));
        }

        let payload: DeviceTokenResponse = serde_json::from_str(&body).map_err(|error| {
            format!("OpenAI device login polling response parse failed: {error}")
        })?;
        if payload.authorization_code.trim().is_empty() || payload.code_verifier.trim().is_empty() {
            return Err("OpenAI device login approval response was incomplete".to_string());
        }
        Ok(Some(DeviceAuthorization {
            authorization_code: payload.authorization_code,
            code_verifier: payload.code_verifier,
        }))
    }

    async fn prune_expired(&self) {
        let now = now_unix() as i64;
        self.sessions
            .lock()
            .await
            .retain(|_, session| session.expires_at > now);
    }
}

fn start_from_session(login_id: &str, session: &DeviceLoginSession) -> DeviceLoginStart {
    DeviceLoginStart {
        login_id: login_id.to_string(),
        user_code: session.user_code.clone(),
        verification_uri: session.verification_uri.clone(),
        interval_seconds: session.interval_seconds,
        expires_in: (session.expires_at - now_unix() as i64).max(0),
    }
}

fn parse_interval(interval: Option<IntervalValue>) -> u64 {
    match interval {
        Some(IntervalValue::Number(seconds)) if seconds > 0 => seconds,
        Some(IntervalValue::String(seconds)) => seconds
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|seconds| *seconds > 0)
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS),
        _ => DEFAULT_POLL_INTERVAL_SECONDS,
    }
}

fn truncate_error_body(body: &str) -> String {
    body.trim().chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use super::{IntervalValue, parse_interval};

    #[test]
    fn parses_device_poll_interval() {
        assert_eq!(parse_interval(Some(IntervalValue::Number(7))), 7);
        assert_eq!(
            parse_interval(Some(IntervalValue::String("9".to_string()))),
            9
        );
        assert_eq!(
            parse_interval(Some(IntervalValue::String("0".to_string()))),
            5
        );
        assert_eq!(parse_interval(None), 5);
    }
}
