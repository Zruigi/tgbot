use axum::http::HeaderMap;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_domain::scope::Scope;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct SupabaseClaims {
    sub: String,
    #[serde(rename = "exp")]
    _exp: usize,
}

pub async fn resolve_scope(state: &AppState, headers: &HeaderMap) -> ApiResult<Scope> {
    if let Some(scope) = resolve_internal_scope(state, headers)? {
        return Ok(scope);
    }
    resolve_supabase_scope(state, headers).await
}

fn resolve_internal_scope(state: &AppState, headers: &HeaderMap) -> ApiResult<Option<Scope>> {
    if state.config.internal_saas_secret.is_empty() {
        return Ok(None);
    }
    let Some(secret) = header_str(headers, "x-saas-internal-secret") else {
        return Ok(None);
    };
    // ============== FIXED: 恒定时间比较防止时序攻击 ==============
    if !constant_time_compare(&secret, &state.config.internal_saas_secret) {
        return Err(ApiError::Unauthorized);
    }

    let role = match header_str(headers, "x-user-role").as_deref() {
        Some("platform_admin") => Role::PlatformAdmin,
        Some("merchant_owner") => Role::MerchantOwner,
        Some("merchant_admin") => Role::MerchantAdmin,
        Some("merchant_viewer") => Role::MerchantViewer,
        _ => return Err(ApiError::Forbidden),
    };
    let user_id = header_str(headers, "x-user-id").and_then(|v| Uuid::parse_str(&v).ok());
    let tenant_id = header_str(headers, "x-tenant-id").filter(|v| !v.is_empty());
    let merchant_id = header_str(headers, "x-merchant-id").and_then(|v| Uuid::parse_str(&v).ok());
    if role != Role::PlatformAdmin && (tenant_id.is_none() || merchant_id.is_none()) {
        return Err(ApiError::Forbidden);
    }

    Ok(Some(Scope {
        authenticated: true,
        user_id,
        role: role.clone(),
        actor_role: role,
        tenant_id,
        merchant_id,
    }))
}

async fn resolve_supabase_scope(state: &AppState, headers: &HeaderMap) -> ApiResult<Scope> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    if state.config.supabase_jwt_secret.is_empty() {
        return Err(ApiError::Unauthorized);
    }

    let claims = decode::<SupabaseClaims>(
        &token,
        &DecodingKey::from_secret(state.config.supabase_jwt_secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .map_err(|_| ApiError::Unauthorized)?
    .claims;
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| ApiError::Unauthorized)?;

    let platform_role = sqlx::query(
        "SELECT role FROM saas_user_roles WHERE user_id = $1 AND role = 'platform_admin'",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    if platform_role.is_some() {
        return Ok(Scope {
            authenticated: true,
            user_id: Some(user_id),
            role: Role::PlatformAdmin,
            actor_role: Role::PlatformAdmin,
            tenant_id: None,
            merchant_id: None,
        });
    }

    let row = sqlx::query(
        "SELECT tenant_id, merchant_id, role FROM tenant_memberships WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError::Forbidden)?;

    let role = match row.try_get::<String, _>("role")?.as_str() {
        "merchant_owner" => Role::MerchantOwner,
        "merchant_admin" => Role::MerchantAdmin,
        "merchant_viewer" => Role::MerchantViewer,
        _ => return Err(ApiError::Forbidden),
    };

    Ok(Scope {
        authenticated: true,
        user_id: Some(user_id),
        role: role.clone(),
        actor_role: role,
        tenant_id: Some(row.try_get("tenant_id")?),
        merchant_id: Some(row.try_get("merchant_id")?),
    })
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = header_str(headers, "authorization")?;
    value.strip_prefix("Bearer ").map(|v| v.trim().to_string())
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
}

// 恒定时间比较 - 防止时序攻击
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut result = 0u8;
    for i in 0..a_bytes.len() {
        result |= a_bytes[i] ^ b_bytes[i];
    }
    result == 0
}
