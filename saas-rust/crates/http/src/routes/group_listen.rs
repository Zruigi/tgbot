//! Group Listen API — frontend-compatible group monitoring endpoints
//!
//! Endpoints:
//!   GET  /api/group_listen/stats                    — get monitor statistics (TBD)
//!   POST /api/group_listen/toggle                   — toggle monitoring for a group
//!   POST /api/group_listen/revoke_sync              — toggle revoke sync
//!   POST /api/group_listen/create_sync_chat         — create a revoke sync chat
//!   GET  /api/group_listen/sync_chat_info           — get revoke sync chat info
//!   GET  /api/group_listen/verified_users           — list verified users
//!   GET  /api/group_listen/unverified_users         — list unverified users
//!   POST /api/group_listen/send_channel_reminder    — send channel subscription reminder
//!   GET  /api/group_listen/sync_chat_assignments    — get sync chat assignments
//!   GET  /api/group_photo                           ��� get group photo file_id (stub)
//!   GET  /api/group_member_count                    — get group member count (stub)

use axum::{extract::State, http::HeaderMap, Json};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};

use crate::auth::resolve_scope;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GroupIdQuery {
    pub group_id: i64,
}

// ---------------------------------------------------------------------------
// GET /api/group_listen/stats
// ---------------------------------------------------------------------------

pub async fn get_monitor_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    // Return a basic summary (placeholder)
    Ok(Json(json!({
        "status": "ok",
        "total_groups": 0,
        "monitored_groups": 0,
        "active_groups": 0,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_listen/toggle
// ---------------------------------------------------------------------------

pub async fn toggle_group_listen(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let group_id = body.get("group_id").and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::BadRequest("missing group_id".into()))?;
    let enabled = body.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    sqlx::query(
        "UPDATE tracked_groups SET is_monitored = $1, updated_at = now() WHERE group_id = $2 AND tenant_id = $3"
    )
    .bind(enabled)
    .bind(group_id)
    .bind(tenant_id)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({"status": "ok", "group_id": group_id, "is_monitored": enabled})))
}

// ---------------------------------------------------------------------------
// POST /api/group_listen/revoke_sync
// ---------------------------------------------------------------------------

pub async fn toggle_revoke_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let group_id = body.get("group_id").and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::BadRequest("missing group_id".into()))?;
    let enabled = body.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    sqlx::query(
        "UPDATE tracked_groups SET delete_on_revoke = $1, updated_at = now() WHERE group_id = $2 AND tenant_id = $3"
    )
    .bind(enabled)
    .bind(group_id)
    .bind(tenant_id)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({"status": "ok", "group_id": group_id, "delete_on_revoke": enabled})))
}

// ---------------------------------------------------------------------------
// POST /api/group_listen/create_sync_chat
// ---------------------------------------------------------------------------

pub async fn create_revoke_sync_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let group_id = body.get("group_id").and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::BadRequest("missing group_id".into()))?;
    let sync_chat_id = body.get("sync_chat_id").and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::BadRequest("missing sync_chat_id".into()))?;

    sqlx::query(
        "UPDATE tracked_groups SET revoke_sync_chat_id = $1, updated_at = now() WHERE group_id = $2 AND tenant_id = $3"
    )
    .bind(sync_chat_id)
    .bind(group_id)
    .bind(tenant_id)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({"status": "ok", "group_id": group_id, "revoke_sync_chat_id": sync_chat_id})))
}

// ---------------------------------------------------------------------------
// GET /api/group_listen/sync_chat_info
// ---------------------------------------------------------------------------

pub async fn get_revoke_sync_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<GroupIdQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let row = sqlx::query(
        "SELECT revoke_sync_chat_id FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2"
    )
    .bind(query.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    match row {
        Some(r) => {
            let sync_chat_id: Option<i64> = r.try_get("revoke_sync_chat_id").ok();
            Ok(Json(json!({
                "status": "ok",
                "group_id": query.group_id,
                "revoke_sync_chat_id": sync_chat_id,
            })))
        }
        None => Err(ApiError::NotFound("group_not_found".into())),
    }
}

// ---------------------------------------------------------------------------
// GET /api/group_listen/verified_users
// ---------------------------------------------------------------------------

pub async fn get_verified_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let rows = sqlx::query(
        "SELECT user_id, username, first_name, group_id, group_title, joined_at, verified_at \
         FROM groups.group_members \
         WHERE is_verified = true AND tenant_id = $1 \
         ORDER BY verified_at DESC LIMIT 200"
    )
    .bind(tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let users: Vec<Value> = rows.iter().map(|r| {
        json!({
            "user_id": r.try_get::<i64, _>("user_id").unwrap_or(0),
            "username": r.try_get::<Option<String>, _>("username").unwrap_or(None),
            "first_name": r.try_get::<Option<String>, _>("first_name").unwrap_or(None),
            "group_id": r.try_get::<i64, _>("group_id").unwrap_or(0),
            "group_title": r.try_get::<Option<String>, _>("group_title").unwrap_or(None),
            "joined_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("joined_at").unwrap_or(None),
            "verified_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("verified_at").unwrap_or(None),
        })
    }).collect();

    Ok(Json(json!({"status": "ok", "data": users, "total": users.len()})))
}

// ---------------------------------------------------------------------------
// GET /api/group_listen/unverified_users
// ---------------------------------------------------------------------------

pub async fn get_unverified_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let rows = sqlx::query(
        "SELECT user_id, username, first_name, group_id, group_title, joined_at \
         FROM groups.group_members \
         WHERE is_verified = false AND tenant_id = $1 \
         ORDER BY joined_at DESC LIMIT 200"
    )
    .bind(tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let users: Vec<Value> = rows.iter().map(|r| {
        json!({
            "user_id": r.try_get::<i64, _>("user_id").unwrap_or(0),
            "username": r.try_get::<Option<String>, _>("username").unwrap_or(None),
            "first_name": r.try_get::<Option<String>, _>("first_name").unwrap_or(None),
            "group_id": r.try_get::<i64, _>("group_id").unwrap_or(0),
            "group_title": r.try_get::<Option<String>, _>("group_title").unwrap_or(None),
            "joined_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("joined_at").unwrap_or(None),
        })
    }).collect();

    Ok(Json(json!({"status": "ok", "data": users, "total": users.len()})))
}

// ---------------------------------------------------------------------------
// POST /api/group_listen/send_channel_reminder
// ---------------------------------------------------------------------------

pub async fn send_channel_reminder(
    _state: State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;

    // TODO: Integrate with Telegram bot to send actual channel subscription reminders
    // For now, return a stub success response
    tracing::info!("send_channel_reminder: stub called");
    Ok(Json(json!({"status": "ok"})))
}

// ---------------------------------------------------------------------------
// GET /api/group_listen/sync_chat_assignments
// ---------------------------------------------------------------------------

pub async fn get_sync_chat_assignments(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let rows = sqlx::query(
        "SELECT group_id, group_title, revoke_sync_chat_id \
         FROM tracked_groups \
         WHERE revoke_sync_chat_id IS NOT NULL AND tenant_id = $1 \
         ORDER BY updated_at DESC"
    )
    .bind(tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let assignments: Vec<Value> = rows.iter().map(|r| {
        json!({
            "group_id": r.try_get::<i64, _>("group_id").unwrap_or(0),
            "group_title": r.try_get::<Option<String>, _>("group_title").unwrap_or(None),
            "revoke_sync_chat_id": r.try_get::<Option<i64>, _>("revoke_sync_chat_id").unwrap_or(None),
        })
    }).collect();

    Ok(Json(json!({"status": "ok", "data": assignments, "total": assignments.len()})))
}

// ---------------------------------------------------------------------------
// GET /api/group_photo
// ---------------------------------------------------------------------------

pub async fn get_group_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<GroupIdQuery>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    // TODO: Call Telegram Bot API to get actual chat photo
    Ok(Json(json!({
        "status": "ok",
        "file_id": null,
    })))
}

// ---------------------------------------------------------------------------
// GET /api/group_member_count
// ---------------------------------------------------------------------------

pub async fn get_group_member_count(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<GroupIdQuery>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    let count: Option<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT member_count FROM tracked_groups WHERE group_id = $1"
    )
    .bind(query.group_id)
    .fetch_optional(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "count": count.unwrap_or(0),
    })))
}

