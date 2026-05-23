use axum::{extract::State, http::HeaderMap, Json};
use serde_json::json;

use crate::{auth::resolve_scope, error::ApiResult, state::AppState};

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"status": "ok", "service": "tgbot-saas-rust"}))
}

pub async fn session_context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    Ok(Json(json!({
        "status": "ok",
        "data": {
            "authenticated": scope.authenticated,
            "role": scope.role.as_str(),
            "actor_role": scope.actor_role.as_str(),
            "tenant_id": scope.tenant_id.unwrap_or_default(),
            "merchant_id": scope.merchant_id.map(|v| v.to_string()),
            "user_id": scope.user_id.map(|v| v.to_string())
        }
    })))
}
