//! Potential Users API — lead management for merchant onboarding
//!
//! Endpoints:
//!   GET  /api/potential_users                 — list potential users (leads)
//!   GET  /api/potential_users/tags            — get available profile tags
//!   POST /api/potential_users/sync            — sync potential users from groups
//!   POST /api/potential_users/{user_id}/promote  — promote a lead to tracked member

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};

use crate::auth::resolve_scope;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PotentialUsersQuery {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

// ---------------------------------------------------------------------------
// GET /api/potential_users
// ---------------------------------------------------------------------------

pub async fn list_potential_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PotentialUsersQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let offset = q.offset.unwrap_or(0).max(0);
    let limit = q.limit.unwrap_or(20).clamp(1, 200);

    let rows = sqlx::query(
        "SELECT user_id, username, first_name, last_name, message_text, keywords, profile_tags, \
                group_id, group_title, captured_at, created_at \
         FROM tenant_saas.potential_users \
         WHERE tenant_id = $1 \
         ORDER BY created_at DESC LIMIT $2 OFFSET $3"
    )
    .bind(tenant_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(state.db()?)
    .await?;

    let users: Vec<Value> = rows.iter().map(|r| {
        json!({
            "user_id": r.try_get::<i64, _>("user_id").unwrap_or(0),
            "username": r.try_get::<Option<String>, _>("username").unwrap_or(None),
            "first_name": r.try_get::<Option<String>, _>("first_name").unwrap_or(None),
            "message_text": r.try_get::<Option<String>, _>("message_text").unwrap_or(None),
            "keywords": r.try_get::<Option<String>, _>("keywords").unwrap_or(None),
            "profile_tags": r.try_get::<Option<String>, _>("profile_tags").unwrap_or(None),
            "group_id": r.try_get::<i64, _>("group_id").unwrap_or(0),
            "captured_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("captured_at").unwrap_or(None),
        })
    }).collect();

    Ok(Json(json!({"status": "ok", "data": users, "total": users.len()})))
}

// ---------------------------------------------------------------------------
// GET /api/potential_users/tags
// ---------------------------------------------------------------------------

pub async fn get_available_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let rows = sqlx::query(
        "SELECT DISTINCT unnest(string_to_array(profile_tags, ',')) AS tag \
         FROM tenant_saas.potential_users \
         WHERE tenant_id = $1 AND profile_tags IS NOT NULL AND profile_tags != '' \
         ORDER BY tag"
    )
    .bind(tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let tags: Vec<String> = rows.iter()
        .filter_map(|r| r.try_get::<String, _>("tag").ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    Ok(Json(json!({"status": "ok", "data": tags})))
}

// ---------------------------------------------------------------------------
// POST /api/potential_users/sync
// ---------------------------------------------------------------------------

pub async fn sync_potential_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    // Sync from group members that haven't been recorded as potential users
    let result = sqlx::query(
        r#"
        INSERT INTO tenant_saas.potential_users (user_id, username, first_name, group_id, group_title, tenant_id, captured_at)
        SELECT gm.user_id, gm.username, gm.first_name, gm.group_id, gm.group_title, gm.tenant_id, gm.joined_at
        FROM groups.group_members gm
        WHERE gm.tenant_id = $1
          AND NOT EXISTS (SELECT 1 FROM tenant_saas.potential_users pu WHERE pu.user_id = gm.user_id AND pu.tenant_id = $1)
        ON CONFLICT (user_id) DO NOTHING
        "#
    )
    .bind(tenant_id)
    .execute(state.db()?)
    .await?;

    let synced = result.rows_affected();
    tracing::info!("sync_potential_users: synced {} users", synced);

    Ok(Json(json!({"status": "ok", "synced": synced})))
}

// ---------------------------------------------------------------------------
// POST /api/potential_users/{user_id}/promote
// ---------------------------------------------------------------------------

pub async fn promote_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    // Remove from potential users table
    let deleted = sqlx::query("DELETE FROM tenant_saas.potential_users WHERE user_id = $1")
        .bind(user_id)
        .execute(state.db()?)
        .await?
        .rows_affected();

    Ok(Json(json!({
        "status": if deleted > 0 { "ok" } else { "error" },
        "user_id": user_id,
    })))
}
