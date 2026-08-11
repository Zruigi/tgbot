//! Identity routes — 基于官方 Telegram API 的智能身份识别与绑定
//!
//!   GET  /api/identity/identify?user_id=&tenant_id=&limit=  只读识别（A 在哪些群/频道、是否管理员）
//!   POST /api/identity/bind   {"user_id":123,"tenant_id":"...","limit":N}  识别并绑定
//!
//! 权限：PlatformAdmin / MerchantOwner / MerchantAdmin。
//! 商户角色强制扫描本租户；平台管理员需显式传 tenant_id。

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_domain::scope::Scope;
use tgbot_services::identity_service::{
    bind_identity, bind_result_json, identify_identity, identify_report_json,
};

use crate::auth::resolve_scope;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct IdentifyQuery {
    pub user_id: Option<i64>,
    pub tenant_id: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct BindRequest {
    pub user_id: Option<i64>,
    pub tenant_id: Option<String>,
    pub limit: Option<i64>,
}

fn require_write_access(scope: &Scope) -> ApiResult<()> {
    if matches!(
        scope.role,
        Role::PlatformAdmin | Role::MerchantOwner | Role::MerchantAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// 租户解析：平台管理员必须显式传 tenant_id；商户角色强制本租户
fn resolve_tenant(scope: &Scope, hint: Option<&str>) -> ApiResult<String> {
    if scope.role == Role::PlatformAdmin {
        let hint = hint.map(str::trim).filter(|value| !value.is_empty());
        return Ok(hint
            .map(str::to_string)
            .ok_or_else(|| ApiError::BadRequest("tenant_id required for platform admin".into()))?);
    }
    scope
        .tenant_id
        .clone()
        .ok_or(ApiError::Forbidden)
}

// ---------------------------------------------------------------------------
// GET /api/identity/identify
// ---------------------------------------------------------------------------

pub async fn identify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<IdentifyQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let user_id = query
        .user_id
        .ok_or_else(|| ApiError::BadRequest("user_id required".into()))?;
    let tenant_id = resolve_tenant(&scope, query.tenant_id.as_deref())?;

    let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
    let report = identify_identity(state.db()?, &telegram, &tenant_id, user_id, query.limit)
        .await
        .map_err(ApiError::Anyhow)?;

    Ok(Json(json!({
        "status": "ok",
        "tenant_id": tenant_id,
        "data": identify_report_json(&report),
    })))
}

// ---------------------------------------------------------------------------
// POST /api/identity/bind
// ---------------------------------------------------------------------------

pub async fn bind(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BindRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let user_id = body
        .user_id
        .ok_or_else(|| ApiError::BadRequest("user_id required".into()))?;
    let tenant_id = resolve_tenant(&scope, body.tenant_id.as_deref())?;

    let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
    let report = identify_identity(state.db()?, &telegram, &tenant_id, user_id, body.limit)
        .await
        .map_err(ApiError::Anyhow)?;
    let result = bind_identity(state.db()?, &tenant_id, user_id, &report)
        .await
        .map_err(ApiError::Anyhow)?;

    Ok(Json(json!({
        "status": "ok",
        "tenant_id": tenant_id,
        "data": bind_result_json(&result),
    })))
}
