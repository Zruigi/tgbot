use axum::{extract::State, http::HeaderMap, Json};
use serde_json::json;

use crate::auth::resolve_scope;
use crate::state::AppState;
use tgbot_domain::error::{ApiError, ApiResult};

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"status": "ok", "service": "tgbot-rust"}))
}

pub async fn readiness(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let pool = state
        .pool
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("postgresql_unavailable".into()))?;
    sqlx::query("SELECT 1")
        .execute(pool)
        .await
        .map_err(|_| ApiError::ServiceUnavailable("postgresql_unavailable".into()))?;

    Ok(Json(json!({
        "status": "ok",
        "service": "tgbot-rust",
        "database": "postgresql",
        "workers": state.worker_enabled,
    })))
}

/// GET /api/session/context - Get current session context
pub async fn session_context_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    // Try to resolve scope from headers, but don't fail if not authenticated
    match resolve_scope(&state, &headers).await {
        Ok(scope) => Ok(Json(json!({
            "status": "ok",
            "data": {
                "authenticated": scope.authenticated,
                "role": scope.role.as_str(),
                "actor_role": scope.actor_role.as_str(),
                "tenant_id": scope.tenant_id.unwrap_or_default(),
                "merchant_id": scope.merchant_id.map(|v| v.to_string()),
                "user_id": scope.user_id.map(|v| v.to_string())
            }
        }))),
        // If authentication fails, return unauthenticated state
        Err(ApiError::Unauthorized) => Ok(Json(json!({
            "status": "ok",
            "data": {
                "authenticated": false,
                "role": "platform_admin",
                "actor_role": "platform_admin",
                "tenant_id": "",
                "merchant_id": serde_json::Value::Null,
                "user_id": serde_json::Value::Null
            }
        }))),
        Err(e) => Err(e),
    }
}

/// POST /api/session/context - Session context management (read-only in production)
/// This endpoint is intentionally disabled for security reasons -
/// switching session context in production is not allowed.
pub async fn session_context_post(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // First verify the user is authenticated
    let _scope = resolve_scope(&_state, &headers).await?;

    // Session context switching is disabled in production
    // This prevents privilege escalation attacks
    Err(tgbot_domain::error::ApiError::Forbidden)
}
