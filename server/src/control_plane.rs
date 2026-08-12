use crate::control_store::LocalStore;
use crate::{
    LocalGatewayHandle, ProviderCompatibilityProfile, ProviderUpstreamProtocol,
    SharedProviderLeaseInput,
};
use reqwest::{
    Client, Method, StatusCode,
    header::{COOKIE, SET_COOKIE},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, time::Duration};
use tokio::time::MissedTickBehavior;

const SHARED_SYNC_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
struct SharedProviderDescriptor {
    id: String,
}

#[derive(Debug, Deserialize)]
struct SharedProvidersResponse {
    providers: Vec<SharedProviderDescriptor>,
}

#[derive(Debug, Deserialize)]
struct SharedProviderLease {
    provider_id: String,
    name: String,
    base_url: String,
    api_key: String,
    upstream_protocol: ProviderUpstreamProtocol,
    compatibility_profile: ProviderCompatibilityProfile,
    expires_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SharedSyncStatus {
    pub provider_count: usize,
    pub renewed_count: usize,
}

#[derive(Debug, Deserialize)]
pub struct ControlLoginInput {
    pub url: String,
    pub email: String,
    pub password: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ControlUser {
    pub id: i64,
    pub name: String,
    pub avatar_url: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct ControlLoginResult {
    pub user: ControlUser,
    pub control_plane_url: String,
}

#[derive(Debug, Deserialize)]
pub struct ControlRequestInput {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub body: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    user: ControlUser,
}

pub async fn login_control_plane(
    store: LocalStore,
    input: ControlLoginInput,
) -> Result<ControlLoginResult, String> {
    let control_plane_url = crate::control_store::normalize_control_plane_url(&input.url)?;
    let email = input.email.trim();
    if email.is_empty() || input.password.is_empty() {
        return Err("请输入中心账号和密码".to_string());
    }

    let client = Client::new();
    let response = client
        .post(format!("{control_plane_url}/auth/login"))
        .json(&serde_json::json!({
            "email": email,
            "password": input.password,
        }))
        .send()
        .await
        .map_err(|error| format!("连接中心服务失败：{error}"))?;
    let response = ensure_success(response).await?;
    let session_cookie = response
        .headers()
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "中心登录响应缺少 Session Cookie".to_string())?;
    let login: LoginResponse = response
        .json()
        .await
        .map_err(|error| format!("解析中心登录响应失败：{error}"))?;

    let token_response = client
        .get(format!("{control_plane_url}/auth/access-tokens"))
        .header(COOKIE, session_cookie)
        .send()
        .await
        .map_err(|error| format!("获取中心访问令牌失败：{error}"))?;
    let token_response = ensure_success(token_response).await?;
    let token_payload: Value = token_response
        .json()
        .await
        .map_err(|error| format!("解析中心访问令牌失败：{error}"))?;
    let access_token = token_payload
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "中心访问令牌响应无效".to_string())?;

    store.configure_control_plane(control_plane_url.clone(), access_token.to_string())?;
    Ok(ControlLoginResult {
        user: login.user,
        control_plane_url,
    })
}

pub async fn request_control_plane(
    store: LocalStore,
    input: ControlRequestInput,
) -> Result<Value, String> {
    let method = Method::from_bytes(input.method.trim().as_bytes())
        .map_err(|_| "不支持的中心请求方法".to_string())?;
    if !matches!(
        method,
        Method::GET | Method::POST | Method::PUT | Method::DELETE
    ) {
        return Err("中心请求只支持 GET、POST、PUT 和 DELETE".to_string());
    }
    let path = input.path.trim();
    if !allowed_control_path(path) {
        return Err("客户端拒绝访问该中心服务路径".to_string());
    }
    let (control_plane_url, access_token, _) = store.control_plane_credentials()?;
    let client = Client::new();
    let mut request = client
        .request(method, format!("{control_plane_url}{path}"))
        .bearer_auth(access_token);
    if let Some(body) = input.body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("连接中心服务失败：{error}"))?;
    let response = ensure_success(response).await?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if content_type.contains("application/json") {
        response
            .json()
            .await
            .map_err(|error| format!("解析中心服务响应失败：{error}"))
    } else {
        response
            .text()
            .await
            .map(Value::String)
            .map_err(|error| format!("读取中心服务响应失败：{error}"))
    }
}

pub async fn publish_shared_connection(
    store: LocalStore,
    gateway: LocalGatewayHandle,
    local_provider_id: String,
) -> Result<String, String> {
    let source = gateway
        .shareable_provider_source(local_provider_id.trim())
        .await?;
    let list = request_control_plane(
        store.clone(),
        ControlRequestInput {
            method: "GET".to_string(),
            path: "/shared-connections".to_string(),
            body: None,
        },
    )
    .await?;
    let existing_id = list
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|providers| {
            providers.iter().find_map(|provider| {
                let same_name =
                    provider.get("name").and_then(Value::as_str) == Some(source.name.as_str());
                same_name
                    .then(|| provider.get("id").and_then(Value::as_str))
                    .flatten()
                    .map(ToString::to_string)
            })
        });
    let body = serde_json::json!({
        "name": source.name,
        "base_url": source.base_url,
        "api_key": source.api_key,
        "compatibility_profile": source.compatibility_profile,
    });
    let response = request_control_plane(
        store,
        ControlRequestInput {
            method: if existing_id.is_some() {
                "PUT".to_string()
            } else {
                "POST".to_string()
            },
            path: existing_id
                .as_ref()
                .map(|id| format!("/shared-connections/{id}"))
                .unwrap_or_else(|| "/shared-connections".to_string()),
            body: Some(body),
        },
    )
    .await?;
    response
        .get("provider_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "中心服务没有返回共享连接标识".to_string())
}

pub async fn sync_shared_providers(
    store: LocalStore,
    gateway: LocalGatewayHandle,
) -> Result<SharedSyncStatus, String> {
    let (control_plane_url, access_token, device_id) = store.control_plane_credentials()?;
    let client = Client::new();
    let response = client
        .get(format!("{control_plane_url}/client/v1/shared-providers"))
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|error| format!("连接中心服务失败：{error}"))?;
    let response = ensure_success(response).await?;
    let payload: SharedProvidersResponse = response
        .json()
        .await
        .map_err(|error| format!("解析共享供应商列表失败：{error}"))?;

    let visible_ids: HashSet<String> = payload
        .providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect();
    let existing_ids = gateway.shared_provider_ids().await;
    for provider_id in existing_ids {
        if !visible_ids.contains(&provider_id) {
            gateway.revoke_shared_provider(&provider_id).await?;
        }
    }

    let mut renewed_count = 0;
    for descriptor in payload.providers {
        let lease_response = client
            .post(format!(
                "{control_plane_url}/client/v1/shared-providers/{}/lease",
                descriptor.id
            ))
            .bearer_auth(&access_token)
            .json(&serde_json::json!({ "device_id": device_id }))
            .send()
            .await
            .map_err(|error| format!("申请共享供应商租约失败：{error}"))?;

        if !lease_response.status().is_success() {
            if matches!(
                lease_response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ) {
                gateway.revoke_shared_provider(&descriptor.id).await?;
            }
            return Err(response_error(lease_response).await);
        }

        let lease: SharedProviderLease = lease_response
            .json()
            .await
            .map_err(|error| format!("解析共享供应商租约失败：{error}"))?;
        if lease.provider_id != descriptor.id {
            gateway.revoke_shared_provider(&descriptor.id).await?;
            return Err("中心服务返回了不匹配的共享供应商租约".to_string());
        }
        gateway
            .upsert_shared_provider(SharedProviderLeaseInput {
                central_provider_id: lease.provider_id,
                name: lease.name,
                base_url: lease.base_url,
                api_key: lease.api_key,
                upstream_protocol: lease.upstream_protocol,
                compatibility_profile: lease.compatibility_profile,
                expires_at: lease.expires_at,
            })
            .await?;
        renewed_count += 1;
    }

    Ok(SharedSyncStatus {
        provider_count: visible_ids.len(),
        renewed_count,
    })
}

pub fn spawn_periodic_shared_sync(store: LocalStore, gateway: LocalGatewayHandle) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SHARED_SYNC_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !store.sharing_is_configured() {
                continue;
            }
            if let Err(error) = sync_shared_providers(store.clone(), gateway.clone()).await {
                eprintln!("同步中心共享授权失败：{error}");
            }
        }
    });
}

fn allowed_control_path(path: &str) -> bool {
    let path_only = path.split('?').next().unwrap_or(path);
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains("://")
        || path.contains('\\')
        || path_only
            .split('/')
            .any(|segment| matches!(segment, "." | ".."))
    {
        return false;
    }
    matches!(path, "/client/v1/me" | "/groups" | "/shared-connections")
        || path.starts_with("/groups/")
        || path.starts_with("/shared-connections/")
        || path.starts_with("/users/search?")
        || path == "/users"
        || path.starts_with("/users/")
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(response_error(response).await)
}

async fn response_error(response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|payload| {
            payload
                .get("error")
                .and_then(|value| value.as_str().map(ToString::to_string))
                .or_else(|| {
                    payload
                        .pointer("/error/message")
                        .and_then(|value| value.as_str().map(ToString::to_string))
                })
        })
        .filter(|message| !message.is_empty())
        .unwrap_or(body);
    format!("中心服务拒绝请求 ({status})：{message}")
}

#[cfg(test)]
mod tests {
    use super::allowed_control_path;

    #[test]
    fn center_bridge_only_allows_control_plane_routes() {
        for path in [
            "/client/v1/me",
            "/groups",
            "/groups/1",
            "/groups/1/members",
            "/shared-connections",
            "/shared-connections/provider-id",
            "/users/search?q=test",
        ] {
            assert!(allowed_control_path(path), "{path} should be allowed");
        }
        for path in [
            "/openai/v1/responses",
            "/usage/summary",
            "/client/v1/shared-providers",
            "https://example.com/groups",
            "//example.com/groups",
        ] {
            assert!(!allowed_control_path(path), "{path} should be rejected");
        }
    }
}
