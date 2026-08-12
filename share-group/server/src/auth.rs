use axum::{
    Json,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;
use uuid::Uuid;

use crate::{
    config::Config,
    store::{LoginIdentity, ManagedUser, SettingsStore, UserRole, UserStore},
};

const FEISHU_AUTHORIZE_URL: &str = "https://accounts.feishu.cn/open-apis/authen/v1/authorize";
const FEISHU_TOKEN_URL: &str = "https://open.feishu.cn/open-apis/authen/v2/oauth/token";
const FEISHU_USER_INFO_URL: &str = "https://open.feishu.cn/open-apis/authen/v1/user_info";
const SESSION_COOKIE_NAME: &str = "ai_gateway_session";
const SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 30;
const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;
const GATEWAY_ERROR_PREFIX: &str = "AI网关错误：";
const UPSTREAM_ERROR_PREFIX: &str = "上游服务错误：";

#[derive(Clone)]
pub struct AuthService {
    users: UserStore,
    settings: SettingsStore,
    http: Client,
    states: Arc<Mutex<HashMap<String, PendingLogin>>>,
    bootstrap_admin: Option<(String, String, String)>,
}

#[derive(Clone)]
struct PendingLogin {
    redirect_uri: String,
    return_url: String,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicUser {
    pub id: i64,
    pub name: String,
    pub avatar_url: String,
    pub role: String,
}

#[derive(Debug, Clone)]
pub struct RequestScope {
    pub owner_user_id: Option<i64>,
    pub is_admin: bool,
}

#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub mode: String,
    pub feishu_login_configured: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub ok: bool,
    pub user: PublicUser,
}

#[derive(Debug, Deserialize)]
pub struct EmailLoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct FeishuCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

impl AuthService {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        Ok(Self {
            users: UserStore::new(config.clone())?,
            settings: SettingsStore::new(config.clone())?,
            http: Client::new(),
            states: Arc::new(Mutex::new(HashMap::new())),
            bootstrap_admin: config.bootstrap_admin()?,
        })
    }

    pub fn initialize(&self) -> Result<(), String> {
        self.users.initialize()?;
        if let Some((email, name, password)) = self.bootstrap_admin.as_ref() {
            self.users.ensure_bootstrap_admin(email, name, password)?;
        }
        Ok(())
    }

    pub fn feishu_configured(&self) -> bool {
        self.settings
            .security_settings()
            .map(|settings| {
                !settings.feishu_app_id.is_empty() && settings.feishu_app_secret_configured
            })
            .unwrap_or(false)
    }

    pub fn begin_feishu_login(&self, headers: &HeaderMap) -> Result<String, String> {
        if !self.feishu_configured() {
            return Err("缺少飞书 App ID 或 App Secret，无法使用飞书登录".to_string());
        }
        let root_url = request_root_url(headers)?;
        let redirect_uri = format!("{root_url}/auth/feishu/callback");
        let state = Uuid::new_v4().to_string();
        self.save_state(&state, &redirect_uri, &root_url)?;

        let (app_id, _) = self.settings.feishu_credentials()?;
        let mut url = Url::parse(FEISHU_AUTHORIZE_URL).map_err(|error| error.to_string())?;
        url.query_pairs_mut()
            .append_pair("client_id", &app_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("state", &state);
        Ok(url.to_string())
    }

    pub async fn complete_feishu_login(
        &self,
        query: FeishuCallbackQuery,
    ) -> Result<(PublicUser, String, String), String> {
        if let Some(error) = query.error.filter(|value| !value.trim().is_empty()) {
            return Err(format!(
                "飞书授权失败：{error} {}",
                query.error_description.unwrap_or_default()
            ));
        }
        let code = query
            .code
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "飞书回调缺少 code".to_string())?;
        let state = query
            .state
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "飞书回调缺少 state".to_string())?;
        let pending = self
            .consume_state(&state)?
            .ok_or_else(|| "登录状态已失效，请重新发起飞书登录".to_string())?;
        let access_token = self.exchange_code(&code, &pending.redirect_uri).await?;
        let profile = self.fetch_user_info(&access_token).await?;
        let user = self.users.upsert_feishu_user(
            &profile.tenant_key,
            &profile.open_id,
            &profile.name,
            &profile.avatar_url,
        )?;
        let session_id = self.create_session(user.id)?;
        Ok((
            PublicUser {
                id: user.id,
                name: profile.name,
                avatar_url: profile.avatar_url,
                role: user.role.as_str().to_string(),
            },
            session_id,
            pending.return_url,
        ))
    }

    pub fn verify_login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<LoginIdentity>, String> {
        self.users.verify_login(email, password)
    }

    pub fn create_user(
        &self,
        email: &str,
        name: &str,
        role: &str,
        password: &str,
    ) -> Result<PublicUser, String> {
        let role = match role {
            "admin" => UserRole::Admin,
            "user" => UserRole::User,
            _ => return Err("角色只支持 admin 或 user".to_string()),
        };
        let user = self
            .users
            .create_managed_user(email, name, role, password)?;
        Ok(PublicUser {
            id: user.id,
            name: name.trim().to_string(),
            avatar_url: String::new(),
            role: user.role.as_str().to_string(),
        })
    }

    pub fn list_users(&self) -> Result<Vec<ManagedUser>, String> {
        self.users.list_all_users()
    }

    pub fn delete_user(&self, user_id: i64) -> Result<bool, String> {
        self.users.delete_user(user_id)
    }

    pub fn public_user(&self, user_id: i64) -> Result<Option<PublicUser>, String> {
        self.users.find_user_for_session(user_id).map(|profile| {
            profile.map(|profile| PublicUser {
                id: profile.user.id,
                name: profile.name,
                avatar_url: profile.avatar_url,
                role: profile.user.role.as_str().to_string(),
            })
        })
    }

    pub fn user_from_headers(&self, headers: &HeaderMap) -> Result<Option<PublicUser>, String> {
        let Some(session_id) = cookie_value(headers, SESSION_COOKIE_NAME) else {
            return Ok(None);
        };
        let now = now_unix();
        let Some(session) = self.users.find_session(&session_id)? else {
            return Ok(None);
        };
        if session.expires_at <= now {
            self.users.delete_session(&session_id)?;
            return Ok(None);
        }
        self.users.touch_session(&session_id, now)?;
        self.public_user(session.user_id)
    }

    pub fn delete_session_from_headers(&self, headers: &HeaderMap) -> Result<(), String> {
        if let Some(session_id) = cookie_value(headers, SESSION_COOKIE_NAME) {
            self.users.delete_session(&session_id)?;
        }
        Ok(())
    }

    pub fn gateway_access_token(&self, user_id: i64) -> Result<String, String> {
        if let Some(token) = self.users.load_gateway_access_token(user_id)? {
            return Ok(token);
        }
        self.regenerate_gateway_access_token(user_id)
    }

    pub fn regenerate_gateway_access_token(&self, user_id: i64) -> Result<String, String> {
        let token = format!("agw_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        self.users.replace_gateway_access_token(user_id, &token)?;
        Ok(token)
    }

    fn gateway_scope_from_headers(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<RequestScope>, String> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(token) = token else {
            return Ok(None);
        };
        Ok(self
            .users
            .find_user_by_gateway_access_token(token)?
            .map(|user| RequestScope {
                owner_user_id: Some(user.id),
                is_admin: user.role == UserRole::Admin,
            }))
    }

    pub fn session_cookie_header(&self, session_id: &str) -> String {
        format!(
            "{SESSION_COOKIE_NAME}={session_id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECS}"
        )
    }

    pub fn expired_session_cookie_header(&self) -> String {
        format!("{SESSION_COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
    }

    fn create_session(&self, user_id: i64) -> Result<String, String> {
        let session_id = Uuid::new_v4().to_string();
        let now = now_unix();
        self.users
            .create_session(&session_id, user_id, now + SESSION_TTL_SECS)?;
        Ok(session_id)
    }

    fn save_state(&self, state: &str, redirect_uri: &str, return_url: &str) -> Result<(), String> {
        let now = now_unix();
        let mut states = self
            .states
            .lock()
            .map_err(|_| "飞书登录状态锁异常".to_string())?;
        states.retain(|_, pending| now - pending.created_at <= OAUTH_STATE_TTL_SECS);
        states.insert(
            state.to_string(),
            PendingLogin {
                redirect_uri: redirect_uri.to_string(),
                return_url: return_url.to_string(),
                created_at: now,
            },
        );
        Ok(())
    }

    fn consume_state(&self, state: &str) -> Result<Option<PendingLogin>, String> {
        let now = now_unix();
        let mut states = self
            .states
            .lock()
            .map_err(|_| "飞书登录状态锁异常".to_string())?;
        states.retain(|_, pending| now - pending.created_at <= OAUTH_STATE_TTL_SECS);
        Ok(states.remove(state))
    }

    async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<String, String> {
        let (app_id, app_secret) = self.settings.feishu_credentials()?;
        let response = self
            .http
            .post(FEISHU_TOKEN_URL)
            .json(&serde_json::json!({
                "grant_type": "authorization_code", "client_id": app_id,
                "client_secret": app_secret, "code": code, "redirect_uri": redirect_uri,
            }))
            .send()
            .await
            .map_err(|error| format!("请求飞书 OAuth token 失败：{error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("解析飞书 OAuth token 响应失败：{error}"))?;
        ensure_feishu_success("OAuth token", status.is_success(), &body)?;
        body.get("data")
            .unwrap_or(&body)
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| "飞书 OAuth token 响应缺少 access_token".to_string())
    }

    async fn fetch_user_info(&self, access_token: &str) -> Result<FeishuProfile, String> {
        let response = self
            .http
            .get(FEISHU_USER_INFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| format!("请求飞书用户信息失败：{error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("解析飞书用户信息响应失败：{error}"))?;
        ensure_feishu_success("用户信息", status.is_success(), &body)?;
        let data = body.get("data").unwrap_or(&body);
        let open_id = string_value(data, &["open_id", "openId"]);
        if open_id.is_empty() {
            return Err("飞书用户信息缺少 open_id".to_string());
        }
        Ok(FeishuProfile {
            tenant_key: string_value(data, &["tenant_key", "tenantKey"]),
            open_id,
            name: {
                let name = string_value(data, &["name", "en_name", "enName"]);
                if name.is_empty() {
                    "飞书用户".to_string()
                } else {
                    name
                }
            },
            avatar_url: string_value(data, &["avatar_url", "avatar_thumb", "avatar_big"]),
        })
    }
}

struct FeishuProfile {
    tenant_key: String,
    open_id: String,
    name: String,
    avatar_url: String,
}

pub async fn auth_status(State(auth): State<AuthService>) -> Json<AuthStatus> {
    Json(AuthStatus {
        mode: "required".to_string(),
        feishu_login_configured: auth.feishu_configured(),
    })
}

pub async fn feishu_authorize(
    State(auth): State<AuthService>,
    headers: HeaderMap,
) -> Result<Redirect, AuthError> {
    let url = auth
        .begin_feishu_login(&headers)
        .map_err(AuthError::precondition_failed)?;
    Ok(Redirect::temporary(&url))
}

pub async fn feishu_callback(
    State(auth): State<AuthService>,
    Query(query): Query<FeishuCallbackQuery>,
) -> Result<Response, AuthError> {
    let (user, session_id, return_url) = auth
        .complete_feishu_login(query)
        .await
        .map_err(AuthError::bad_gateway)?;
    let mut response = Redirect::temporary(&return_url).into_response();
    let value = HeaderValue::from_str(&auth.session_cookie_header(&session_id))
        .map_err(|_| AuthError::internal("生成 session Cookie 失败"))?;
    response.headers_mut().insert(header::SET_COOKIE, value);
    let _ = user;
    Ok(response)
}

pub async fn email_login(
    State(auth): State<AuthService>,
    Json(request): Json<EmailLoginRequest>,
) -> Result<Response, AuthError> {
    let auth_for_verify = auth.clone();
    let identity = tokio::task::spawn_blocking(move || {
        auth_for_verify.verify_login(&request.email, &request.password)
    })
    .await
    .map_err(|error| AuthError::internal(format!("等待密码验证任务失败：{error}")))?
    .map_err(AuthError::internal)?
    .ok_or_else(AuthError::unauthorized)?;
    let session_id = auth
        .create_session(identity.user.id)
        .map_err(AuthError::internal)?;
    let user = PublicUser {
        id: identity.user.id,
        name: identity.name,
        avatar_url: String::new(),
        role: identity.user.role.as_str().to_string(),
    };
    let mut response = Json(LoginResponse { ok: true, user }).into_response();
    let value = HeaderValue::from_str(&auth.session_cookie_header(&session_id))
        .map_err(|_| AuthError::internal("生成 session Cookie 失败"))?;
    response.headers_mut().insert(header::SET_COOKIE, value);
    Ok(response)
}

pub async fn me(
    State(auth): State<AuthService>,
    headers: HeaderMap,
) -> Result<Json<LoginResponse>, AuthError> {
    let user = auth
        .user_from_headers(&headers)
        .map_err(AuthError::internal)?
        .ok_or_else(AuthError::unauthorized)?;
    Ok(Json(LoginResponse { ok: true, user }))
}

pub async fn logout(
    State(auth): State<AuthService>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    auth.delete_session_from_headers(&headers)
        .map_err(AuthError::internal)?;
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    let value = HeaderValue::from_str(&auth.expired_session_cookie_header())
        .map_err(|_| AuthError::internal("生成 session Cookie 失败"))?;
    response.headers_mut().insert(header::SET_COOKIE, value);
    Ok(response)
}

pub async fn gateway_access_token(
    State(auth): State<AuthService>,
    headers: HeaderMap,
) -> Result<Json<Value>, AuthError> {
    let user = auth
        .user_from_headers(&headers)
        .map_err(AuthError::internal)?
        .ok_or_else(AuthError::unauthorized)?;
    let token = auth
        .gateway_access_token(user.id)
        .map_err(AuthError::internal)?;
    Ok(Json(serde_json::json!({ "access_token": token })))
}

pub async fn regenerate_gateway_access_token(
    State(auth): State<AuthService>,
    headers: HeaderMap,
) -> Result<Json<Value>, AuthError> {
    let user = auth
        .user_from_headers(&headers)
        .map_err(AuthError::internal)?
        .ok_or_else(AuthError::unauthorized)?;
    let token = auth
        .regenerate_gateway_access_token(user.id)
        .map_err(AuthError::internal)?;
    Ok(Json(serde_json::json!({ "access_token": token })))
}

pub async fn require_auth(
    State(auth): State<AuthService>,
    mut request: Request,
    next: Next,
) -> Response {
    match auth.user_from_headers(request.headers()) {
        Ok(Some(user)) => {
            request.extensions_mut().insert(RequestScope {
                owner_user_id: Some(user.id),
                is_admin: user.role == UserRole::Admin.as_str(),
            });
            next.run(request).await
        }
        Ok(None) => AuthError::unauthorized().into_response(),
        Err(error) => AuthError::internal(error).into_response(),
    }
}

pub async fn require_gateway_auth(
    State(auth): State<AuthService>,
    mut request: Request,
    next: Next,
) -> Response {
    match auth.gateway_scope_from_headers(request.headers()) {
        Ok(Some(scope)) => {
            request.extensions_mut().insert(scope);
            next.run(request).await
        }
        Ok(None) => match auth.user_from_headers(request.headers()) {
            Ok(Some(user)) => {
                request.extensions_mut().insert(RequestScope {
                    owner_user_id: Some(user.id),
                    is_admin: user.role == UserRole::Admin.as_str(),
                });
                next.run(request).await
            }
            Ok(None) => AuthError::unauthorized().into_response(),
            Err(error) => AuthError::internal(error).into_response(),
        },
        Err(error) => AuthError::internal(error).into_response(),
    }
}

fn request_root_url(headers: &HeaderMap) -> Result<String, String> {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "无法从请求头确定服务公开地址".to_string())?;
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("http");
    Ok(format!("{scheme}://{host}"))
}

fn ensure_feishu_success(action: &str, status_ok: bool, body: &Value) -> Result<(), String> {
    if !status_ok {
        return Err(format!("飞书 {action} 请求失败：{body}"));
    }
    match body.get("code").and_then(Value::as_i64) {
        Some(0) | None => Ok(()),
        Some(code) => Err(format!(
            "飞书 {action} 返回错误：code={code}, msg={}",
            body.get("msg")
                .or_else(|| body.get("message"))
                .and_then(Value::as_str)
                .unwrap_or_default()
        )),
    }
}

fn string_value(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            value
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_default()
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name && !value.trim().is_empty()).then(|| value.trim().to_string())
    })
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub struct AuthError {
    status: StatusCode,
    message: String,
    source: AuthErrorSource,
}

#[derive(Clone, Copy)]
enum AuthErrorSource {
    Gateway,
    Upstream,
}
impl AuthError {
    fn precondition_failed(message: String) -> Self {
        Self {
            status: StatusCode::PRECONDITION_FAILED,
            message,
            source: AuthErrorSource::Gateway,
        }
    }
    fn bad_gateway(message: String) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message,
            source: AuthErrorSource::Upstream,
        }
    }
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "请先登录中心服务".to_string(),
            source: AuthErrorSource::Gateway,
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            source: AuthErrorSource::Gateway,
        }
    }
}
impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let message = match self.source {
            AuthErrorSource::Gateway => format!("{GATEWAY_ERROR_PREFIX}{}", self.message),
            AuthErrorSource::Upstream => format!("{UPSTREAM_ERROR_PREFIX}{}", self.message),
        };
        (self.status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
