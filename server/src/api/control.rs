use crate::{
    auth::{AuthService, RequestScope},
    models::{
        AddGatewayGroupMemberRequest, CreateApiProviderRequest, CreateGatewayGroupRequest,
        IssueSharedProviderLeaseRequest, ShareGatewayGroupProviderRequest,
        SharedProviderDescriptor, SharedProviderLease,
    },
    store::{GroupStore, ProviderStore},
    support::time::now_unix,
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

const SHARED_PROVIDER_LEASE_SECONDS: i64 = 5 * 60;

#[derive(Clone)]
pub struct ControlState {
    pub auth: AuthService,
    pub groups: GroupStore,
    pub providers: ProviderStore,
}

#[derive(Debug)]
pub struct ControlError {
    status: StatusCode,
    message: String,
}

impl ControlError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ControlError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

pub async fn healthz() -> &'static str {
    "ok"
}

#[derive(Debug, Deserialize)]
pub struct CreateManagedUserRequest {
    pub email: String,
    pub name: String,
    pub role: String,
    pub password: String,
}

pub async fn list_users(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
) -> Result<Json<Value>, ControlError> {
    require_admin(&scope)?;
    let users = state.auth.list_users().map_err(ControlError::internal)?;
    Ok(Json(json!({ "users": users })))
}

pub async fn create_user(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Json(request): Json<CreateManagedUserRequest>,
) -> Result<Json<Value>, ControlError> {
    require_admin(&scope)?;
    let auth = state.auth.clone();
    let user = tokio::task::spawn_blocking(move || {
        auth.create_user(
            &request.email,
            &request.name,
            &request.role,
            &request.password,
        )
    })
    .await
    .map_err(|error| ControlError::internal(format!("等待密码哈希任务失败：{error}")))?
    .map_err(ControlError::bad_request)?;
    Ok(Json(json!({ "user": user })))
}

pub async fn delete_user(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path(user_id): Path<i64>,
) -> Result<Json<Value>, ControlError> {
    require_admin(&scope)?;
    if scope.owner_user_id == Some(user_id) {
        return Err(ControlError::bad_request("不能删除当前登录账号"));
    }
    let deleted = state
        .auth
        .delete_user(user_id)
        .map_err(ControlError::internal)?;
    if !deleted {
        return Err(ControlError::not_found("账号不存在"));
    }
    Ok(Json(json!({ "deleted": true, "user_id": user_id })))
}

pub async fn list_shared_connections(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let providers = state.providers.list_for_owner(user_id).await;
    Ok(Json(json!({ "providers": providers })))
}

pub async fn create_shared_connection(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Json(request): Json<CreateApiProviderRequest>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let provider = state
        .providers
        .upsert_for_owner(user_id, request)
        .await
        .map_err(ControlError::bad_request)?;
    Ok(Json(json!({ "provider_id": provider.id })))
}

pub async fn update_shared_connection(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path(provider_id): Path<String>,
    Json(request): Json<CreateApiProviderRequest>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let provider = state
        .providers
        .update_for_owner(user_id, &provider_id, request)
        .await
        .map_err(ControlError::bad_request)?;
    Ok(Json(json!({ "provider_id": provider.id })))
}

pub async fn delete_shared_connection(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path(provider_id): Path<String>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    state
        .providers
        .delete_for_owner(user_id, &provider_id)
        .await
        .map_err(ControlError::not_found)?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn list_groups(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let groups = state
        .groups
        .list_for_user(user_id)
        .map_err(ControlError::internal)?;
    Ok(Json(json!({ "groups": groups })))
}

pub async fn create_group(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Json(request): Json<CreateGatewayGroupRequest>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let group = state
        .groups
        .create_group(user_id, &request.name, now_unix() as i64)
        .map_err(ControlError::bad_request)?;
    Ok(Json(json!(group)))
}

pub async fn get_group(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path(group_id): Path<i64>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let group = state
        .groups
        .get_detail(group_id, user_id)
        .map_err(ControlError::internal)?
        .ok_or_else(|| ControlError::not_found("群组不存在或你不是成员"))?;
    Ok(Json(json!(group)))
}

pub async fn add_group_member(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path(group_id): Path<i64>,
    Json(request): Json<AddGatewayGroupMemberRequest>,
) -> Result<Json<Value>, ControlError> {
    let actor_user_id = group_user_id(&scope)?;
    state
        .groups
        .add_member(
            group_id,
            actor_user_id,
            request.user_id,
            scope.is_admin,
            now_unix() as i64,
        )
        .map_err(ControlError::bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_group_member(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path((group_id, user_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, ControlError> {
    let actor_user_id = group_user_id(&scope)?;
    state
        .groups
        .remove_member(group_id, actor_user_id, user_id, scope.is_admin)
        .map_err(ControlError::bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn share_group_connection(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path(group_id): Path<i64>,
    Json(request): Json<ShareGatewayGroupProviderRequest>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    state
        .groups
        .share_provider(group_id, user_id, &request.provider_id, now_unix() as i64)
        .map_err(ControlError::bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn unshare_group_connection(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path((group_id, provider_id)): Path<(i64, String)>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    state
        .groups
        .unshare_provider(group_id, user_id, &provider_id, scope.is_admin)
        .map_err(ControlError::bad_request)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct UserSearchQuery {
    #[serde(default)]
    pub q: String,
}

pub async fn search_users(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Query(query): Query<UserSearchQuery>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    if query.q.trim().is_empty() {
        return Ok(Json(json!({ "users": [] })));
    }
    let users = state
        .groups
        .search_users(&query.q, user_id)
        .map_err(ControlError::internal)?;
    Ok(Json(json!({ "users": users })))
}

pub async fn list_client_shared_providers(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let mut descriptors = Vec::new();
    for provider in state.providers.list_visible_for_user(user_id).await {
        if !provider.shared {
            continue;
        }
        let shared_by_name = state
            .groups
            .shared_provider_owner_name(user_id, &provider.id)
            .map_err(ControlError::internal)?
            .unwrap_or_else(|| "群组成员".to_string());
        descriptors.push(SharedProviderDescriptor {
            id: provider.id,
            name: provider.name,
            base_url: provider.base_url,
            upstream_protocol: provider.upstream_protocol,
            compatibility_profile: provider.compatibility_profile,
            shared_by_name,
        });
    }
    Ok(Json(json!({ "providers": descriptors })))
}

pub async fn client_me(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
) -> Result<Json<Value>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let user = state
        .auth
        .public_user(user_id)
        .map_err(ControlError::internal)?
        .ok_or_else(|| ControlError::not_found("中心账号不存在"))?;
    Ok(Json(json!({ "user": user })))
}

pub async fn issue_client_shared_provider_lease(
    State(state): State<ControlState>,
    Extension(scope): Extension<RequestScope>,
    Path(provider_id): Path<String>,
    Json(request): Json<IssueSharedProviderLeaseRequest>,
) -> Result<Json<SharedProviderLease>, ControlError> {
    let user_id = group_user_id(&scope)?;
    let provider_id = provider_id.trim();
    if provider_id.is_empty() || provider_id.len() > 128 {
        return Err(ControlError::bad_request("供应商标识无效"));
    }
    let provider = state
        .providers
        .find_shared_by_id_for_user(user_id, provider_id)
        .await
        .ok_or_else(|| ControlError::not_found("共享供应商不存在或你没有使用权限"))?;
    let now = now_unix() as i64;
    let expires_at = now + SHARED_PROVIDER_LEASE_SECONDS;
    state
        .groups
        .record_client_lease(user_id, &request.device_id, provider_id, expires_at, now)
        .map_err(ControlError::forbidden)?;
    Ok(Json(SharedProviderLease {
        provider_id: provider.id,
        name: provider.name,
        base_url: provider.base_url,
        api_key: provider.api_key,
        upstream_protocol: provider.upstream_protocol,
        compatibility_profile: provider.compatibility_profile,
        expires_at,
    }))
}

fn group_user_id(scope: &RequestScope) -> Result<i64, ControlError> {
    scope
        .owner_user_id
        .ok_or_else(|| ControlError::forbidden("群组和共享功能需要中心登录身份"))
}

fn require_admin(scope: &RequestScope) -> Result<(), ControlError> {
    if scope.is_admin {
        Ok(())
    } else {
        Err(ControlError::forbidden("只有管理员可以管理中心账号"))
    }
}
