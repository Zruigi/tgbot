//! Group Members API — member listing, permissions, and actions
//!
//! Endpoints:
//!   GET  /api/group_members               — list members with filters
//!   GET  /api/group_members/permissions    — get member permission info
//!   POST /api/group_members/action         — single member action (kick/ban/mute/verify)
//!   POST /api/group_members/action_batch   — batch member actions
//!   POST /api/group_members/admin_batch    — batch admin promotion/demotion
//!   POST /api/group_members/promote_admin  — promote a member to admin
//!   POST /api/group_members/demote_admin   — demote an admin to member
//!   GET  /api/group_messages               — list group messages
//!   POST /api/group_messages/send          — send message to group
//!   POST /api/group_messages/reply         — reply to a group message
//!   POST /api/group_messages/delete        — delete a group message

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_integrations::telegram::{SendMessageParams, TelegramClient, TelegramConfig};

use crate::auth::resolve_scope;
use crate::state::AppState;

use super::platform::row_to_json;

// ---------------------------------------------------------------------------
// Query / body structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MembersQuery {
    pub group_id: Option<i64>,
    pub status: Option<String>,
    pub is_verified: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionsQuery {
    pub group_id: i64,
    pub user_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MemberActionRequest {
    pub group_id: i64,
    pub user_id: i64,
    pub action: String, // kick, ban, mute, unmute, verify, unverify
    pub reason: Option<String>,
    pub duration: Option<i32>, // duration in seconds for mute
}

#[derive(Debug, Deserialize)]
pub struct BatchActionRequest {
    pub group_id: i64,
    pub user_ids: Vec<i64>,
    pub action: String,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminBatchRequest {
    pub group_id: i64,
    pub user_ids: Vec<i64>,
    pub action: String, // promote, demote
}

#[derive(Debug, Deserialize)]
pub struct PromoteAdminRequest {
    pub group_id: i64,
    pub user_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct DemoteAdminRequest {
    pub group_id: i64,
    pub user_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct GroupMessagesQuery {
    pub group_id: i64,
    pub user_id: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub msg_type: Option<String>,
    pub is_advertising: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct GroupMessageSendRequest {
    pub group_id: i64,
    pub content: String,
    pub msg_type: Option<String>,
    pub file_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GroupMessageReplyRequest {
    pub group_id: i64,
    pub tg_msg_id: i64,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct GroupMessageDeleteRequest {
    pub group_id: i64,
    pub msg_id: i64,
}

// ---------------------------------------------------------------------------
// GET /api/group_members
// ---------------------------------------------------------------------------

pub async fn list_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<MembersQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let tenant_id = resolve_tenant_id(&scope)?;

    let rows = sqlx::query(
        r#"
        SELECT * FROM groups.group_members
        WHERE tenant_id = $1
          AND ($2::bigint IS NULL OR group_id = $2)
          AND ($3::text IS NULL OR status = $3)
          AND ($4::boolean IS NULL OR is_verified = $4)
        ORDER BY joined_at DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(tenant_id)
    .bind(q.group_id)
    .bind(q.status)
    .bind(q.is_verified)
    .bind(limit)
    .bind(offset)
    .fetch_all(state.db()?)
    .await?;

    let total = rows.len();
    Ok(Json(json!({
        "status": "ok",
        "data": rows.into_iter().map(row_to_json).collect::<Vec<_>>(),
        "total": total
    })))
}

// ---------------------------------------------------------------------------
// GET /api/group_members/permissions
// ---------------------------------------------------------------------------

pub async fn get_permissions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PermissionsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = resolve_tenant_id(&scope)?;

    // Verify the group belongs to this tenant
    let group_owned = sqlx::query(
        "SELECT 1 FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(q.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if group_owned.is_none() {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    // If user_id is specified, return that member's permissions
    if let Some(user_id) = q.user_id {
        let row = sqlx::query(
            r#"
            SELECT user_id, username, first_name, last_name, status, is_verified
            FROM groups.group_members
            WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
            "#,
        )
        .bind(q.group_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_optional(state.db()?)
        .await?;

        return Ok(Json(json!({
            "status": "ok",
            "data": row.map(row_to_json)
        })));
    }

    // Otherwise return permission summary for the group
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND status = 'active'",
    )
    .bind(q.group_id)
    .bind(tenant_id)
    .fetch_one(state.db()?)
    .await?;

    let verified = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND is_verified = true",
    )
    .bind(q.group_id)
    .bind(tenant_id)
    .fetch_one(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "group_id": q.group_id,
            "total_members": total,
            "verified_members": verified,
            "unverified_members": total - verified,
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_members/action
// ---------------------------------------------------------------------------

pub async fn member_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MemberActionRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    // Validate action
    let valid_actions = ["kick", "ban", "mute", "unmute", "verify", "unverify"];
    if !valid_actions.contains(&body.action.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "action must be one of: {}",
            valid_actions.join(", ")
        )));
    }

    // Verify the group belongs to this tenant
    let group_owned = sqlx::query(
        "SELECT 1 FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if group_owned.is_none() {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    match body.action.as_str() {
        "kick" | "ban" => {
            let new_status = if body.action == "kick" {
                "kicked"
            } else {
                "banned"
            };
            let row = sqlx::query(
                r#"
                UPDATE groups.group_members
                SET status = $1, updated_at = now()
                WHERE group_id = $2 AND user_id = $3 AND tenant_id = $4
                RETURNING *
                "#,
            )
            .bind(new_status)
            .bind(body.group_id)
            .bind(body.user_id)
            .bind(tenant_id)
            .fetch_optional(state.db()?)
            .await?;

            if row.is_none() {
                return Err(ApiError::NotFound("member_not_found".into()));
            }
            // TODO: send kick/ban command to Telegram bot
            tracing::info!("member_action: {} user {} in group {}", body.action, body.user_id, body.group_id);
        }
        "mute" => {
            let row = sqlx::query(
                r#"
                UPDATE groups.group_members
                SET status = 'muted', updated_at = now()
                WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
                RETURNING *
                "#,
            )
            .bind(body.group_id)
            .bind(body.user_id)
            .bind(tenant_id)
            .fetch_optional(state.db()?)
            .await?;

            if row.is_none() {
                return Err(ApiError::NotFound("member_not_found".into()));
            }
            // TODO: send mute command to Telegram bot
            tracing::info!("member_action: mute user {} in group {} for {:?}s", body.user_id, body.group_id, body.duration);
        }
        "unmute" => {
            let row = sqlx::query(
                r#"
                UPDATE groups.group_members
                SET status = 'active', updated_at = now()
                WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
                RETURNING *
                "#,
            )
            .bind(body.group_id)
            .bind(body.user_id)
            .bind(tenant_id)
            .fetch_optional(state.db()?)
            .await?;

            if row.is_none() {
                return Err(ApiError::NotFound("member_not_found".into()));
            }
            // TODO: send unmute command to Telegram bot
            tracing::info!("member_action: unmute user {} in group {}", body.user_id, body.group_id);
        }
        "verify" => {
            let row = sqlx::query(
                r#"
                UPDATE groups.group_members
                SET is_verified = true, verified_at = now(), status = 'active', updated_at = now()
                WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
                RETURNING *
                "#,
            )
            .bind(body.group_id)
            .bind(body.user_id)
            .bind(tenant_id)
            .fetch_optional(state.db()?)
            .await?;

            if row.is_none() {
                return Err(ApiError::NotFound("member_not_found".into()));
            }
            tracing::info!("member_action: verify user {} in group {}", body.user_id, body.group_id);
        }
        "unverify" => {
            let row = sqlx::query(
                r#"
                UPDATE groups.group_members
                SET is_verified = false, verified_at = NULL, updated_at = now()
                WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
                RETURNING *
                "#,
            )
            .bind(body.group_id)
            .bind(body.user_id)
            .bind(tenant_id)
            .fetch_optional(state.db()?)
            .await?;

            if row.is_none() {
                return Err(ApiError::NotFound("member_not_found".into()));
            }
            tracing::info!("member_action: unverify user {} in group {}", body.user_id, body.group_id);
        }
        _ => unreachable!(),
    }

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "group_id": body.group_id,
            "user_id": body.user_id,
            "action": body.action,
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_members/action_batch
// ---------------------------------------------------------------------------

pub async fn member_action_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BatchActionRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    let valid_actions = ["kick", "ban", "mute", "unmute", "verify", "unverify"];
    if !valid_actions.contains(&body.action.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "action must be one of: {}",
            valid_actions.join(", ")
        )));
    }

    if body.user_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids cannot be empty".into()));
    }

    if body.user_ids.len() > 100 {
        return Err(ApiError::BadRequest("batch size cannot exceed 100".into()));
    }

    // Verify the group belongs to this tenant
    let group_owned = sqlx::query(
        "SELECT 1 FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if group_owned.is_none() {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    let new_status = match body.action.as_str() {
        "kick" => Some("kicked"),
        "ban" => Some("banned"),
        "mute" => Some("muted"),
        "unmute" => Some("active"),
        _ => None,
    };

    let affected = if let Some(status) = new_status {
        let result = sqlx::query(
            r#"
            UPDATE groups.group_members
            SET status = $1, updated_at = now()
            WHERE group_id = $2 AND tenant_id = $3 AND user_id = ANY($4)
            "#,
        )
        .bind(status)
        .bind(body.group_id)
        .bind(tenant_id)
        .bind(&body.user_ids)
        .execute(state.db()?)
        .await?;
        result.rows_affected() as i64
    } else if body.action == "verify" {
        let result = sqlx::query(
            r#"
            UPDATE groups.group_members
            SET is_verified = true, verified_at = now(), status = 'active', updated_at = now()
            WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)
            "#,
        )
        .bind(body.group_id)
        .bind(tenant_id)
        .bind(&body.user_ids)
        .execute(state.db()?)
        .await?;
        result.rows_affected() as i64
    } else {
        // unverify
        let result = sqlx::query(
            r#"
            UPDATE groups.group_members
            SET is_verified = false, verified_at = NULL, updated_at = now()
            WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)
            "#,
        )
        .bind(body.group_id)
        .bind(tenant_id)
        .bind(&body.user_ids)
        .execute(state.db()?)
        .await?;
        result.rows_affected() as i64
    };

    // TODO: send batch action commands to Telegram bot

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "group_id": body.group_id,
            "action": body.action,
            "affected": affected,
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_members/admin_batch
// ---------------------------------------------------------------------------

pub async fn admin_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AdminBatchRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    if body.user_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids cannot be empty".into()));
    }

    if !matches!(body.action.as_str(), "promote" | "demote") {
        return Err(ApiError::BadRequest(
            "action must be promote or demote".into(),
        ));
    }

    // Verify the group belongs to this tenant
    let group_owned = sqlx::query(
        "SELECT 1 FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if group_owned.is_none() {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    // TODO: send promote/demote commands to Telegram bot
    // For now, just update the status in the database
    let new_status = if body.action == "promote" {
        "admin"
    } else {
        "active"
    };

    let result = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET status = $1, updated_at = now()
        WHERE group_id = $2 AND tenant_id = $3 AND user_id = ANY($4)
        "#,
    )
    .bind(new_status)
    .bind(body.group_id)
    .bind(tenant_id)
    .bind(&body.user_ids)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "group_id": body.group_id,
            "action": body.action,
            "affected": result.rows_affected(),
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_members/promote_admin
// ---------------------------------------------------------------------------

pub async fn promote_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PromoteAdminRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    // Verify the group belongs to this tenant
    let group_owned = sqlx::query(
        "SELECT 1 FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if group_owned.is_none() {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    let row = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET status = 'admin', updated_at = now()
        WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(body.group_id)
    .bind(body.user_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if row.is_none() {
        return Err(ApiError::NotFound("member_not_found".into()));
    }

    // TODO: send promote command to Telegram bot
    tracing::info!("promote_admin: user {} in group {}", body.user_id, body.group_id);

    Ok(Json(json!({
        "status": "ok",
        "data": row_to_json(row.unwrap())
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_members/demote_admin
// ---------------------------------------------------------------------------

pub async fn demote_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DemoteAdminRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    // Verify the group belongs to this tenant
    let group_owned = sqlx::query(
        "SELECT 1 FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if group_owned.is_none() {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    let row = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET status = 'active', updated_at = now()
        WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3 AND status = 'admin'
        RETURNING *
        "#,
    )
    .bind(body.group_id)
    .bind(body.user_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if row.is_none() {
        return Err(ApiError::NotFound("member_not_found_or_not_admin".into()));
    }

    // TODO: send demote command to Telegram bot
    tracing::info!("demote_admin: user {} in group {}", body.user_id, body.group_id);

    Ok(Json(json!({
        "status": "ok",
        "data": row_to_json(row.unwrap())
    })))
}

// ---------------------------------------------------------------------------
// GET /api/group_messages
// ---------------------------------------------------------------------------

pub async fn list_group_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<GroupMessagesQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let tenant_id = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    } else {
        ""
    };

    let rows = if tenant_id.is_empty() {
        // Platform admin: no tenant filter
        sqlx::query(
            r#"
            SELECT * FROM groups.group_messages
            WHERE group_id = $1
              AND ($2::bigint IS NULL OR user_id = $2)
              AND ($3::text IS NULL OR msg_type = $3)
              AND ($4::boolean IS NULL OR is_advertising = $4)
            ORDER BY timestamp DESC
            LIMIT $5 OFFSET $6
            "#,
        )
        .bind(q.group_id)
        .bind(q.user_id)
        .bind(q.msg_type)
        .bind(q.is_advertising)
        .bind(limit)
        .bind(offset)
        .fetch_all(state.db()?)
        .await?
    } else {
        // Merchant-scoped: filter by tenant_id
        sqlx::query(
            r#"
            SELECT * FROM groups.group_messages
            WHERE group_id = $1 AND tenant_id = $2
              AND ($3::bigint IS NULL OR user_id = $3)
              AND ($4::text IS NULL OR msg_type = $4)
              AND ($5::boolean IS NULL OR is_advertising = $5)
            ORDER BY timestamp DESC
            LIMIT $6 OFFSET $7
            "#,
        )
        .bind(q.group_id)
        .bind(tenant_id)
        .bind(q.user_id)
        .bind(q.msg_type)
        .bind(q.is_advertising)
        .bind(limit)
        .bind(offset)
        .fetch_all(state.db()?)
        .await?
    };

    let total = rows.len();
    Ok(Json(json!({
        "status": "ok",
        "data": rows.into_iter().map(row_to_json).collect::<Vec<_>>(),
        "total": total
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_messages/send
// ---------------------------------------------------------------------------

pub async fn send_group_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GroupMessageSendRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    // Verify the group belongs to this tenant
    let group_owned = sqlx::query(
        "SELECT 1 FROM tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if group_owned.is_none() {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    if body.content.trim().is_empty() && body.file_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ApiError::BadRequest("content or file_id required".into()));
    }

    let msg_type = body
        .msg_type
        .as_deref()
        .unwrap_or_else(|| if body.file_id.is_some() { "photo" } else { "text" });

    let admin_user_id = headers
        .get("x-admin-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    // TODO: Actually send the message to Telegram group via bot engine.
    // For now, we store it in the database with a placeholder tg_msg_id.
    let row = sqlx::query(
        r#"
        INSERT INTO groups.group_messages (
            group_id, user_id, username, content, msg_type, file_id,
            sender_type, tenant_id, timestamp
        ) VALUES ($1, $2, 'admin', $3, $4, $5, 'admin', $6, now())
        RETURNING id
        "#,
    )
    .bind(body.group_id)
    .bind(admin_user_id)
    .bind(body.content.trim())
    .bind(msg_type)
    .bind(body.file_id.as_deref().unwrap_or("").trim())
    .bind(tenant_id)
    .fetch_one(state.db()?)
    .await?;

    let msg_id: i64 = row.try_get("id")?;

    tracing::info!("send_group_message: saved msg_id={msg_id} to group {}", body.group_id);

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "msg_id": msg_id,
            "tg_msg_id": Value::Null,
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_messages/reply
// ---------------------------------------------------------------------------

pub async fn reply_group_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GroupMessageReplyRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    // Find the original message to get the group_id
    let orig = sqlx::query(
        "SELECT group_id, content FROM groups.group_messages WHERE tg_msg_id = $1 AND tenant_id = $2",
    )
    .bind(body.tg_msg_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    let orig = orig.ok_or_else(|| ApiError::NotFound("original_message_not_found".into()))?;

    let orig_group_id: i64 = orig.try_get("group_id")?;
    let orig_content: Option<String> = orig.try_get("content").ok().flatten();

    if orig_group_id != body.group_id {
        return Err(ApiError::BadRequest(
            "tg_msg_id does not belong to the specified group".into(),
        ));
    }

    if body.content.trim().is_empty() {
        return Err(ApiError::BadRequest("content required".into()));
    }

    let admin_user_id = headers
        .get("x-admin-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    let quoted_text = orig_content.unwrap_or_default();
    let quoted_text_ref = if quoted_text.len() > 200 {
        &quoted_text[..200]
    } else {
        &quoted_text
    };

    // Step 1: Send the reply to Telegram group via Bot API
    let tg_msg_id: Option<i64> = {
        // Get bot token from registry
        let bot_entry = state.bot_registry.get(body.group_id).await;

        if let Some(entry) = bot_entry {
            // Create Telegram client with this bot's token
            let config = TelegramConfig {
                bot_token: entry.bot_token,
                api_base_url: "https://api.telegram.org".to_string(),
                timeout_secs: 30,
            };

            match TelegramClient::new(config) {
                Ok(client) => {
                    let params = SendMessageParams {
                        chat_id: body.group_id,
                        text: body.content.trim().to_string(),
                        parse_mode: Some("HTML".to_string()),
                        reply_markup: None,
                        // Key fix: set reply_to_message_id to make this a reply
                        reply_to_message_id: Some(body.tg_msg_id),
                    };

                    match client.send_message(params).await {
                        Ok(msg) => Some(msg.message_id),
                        Err(e) => {
                            tracing::warn!("Failed to send reply to Telegram: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to create Telegram client: {}", e);
                    None
                }
            }
        } else {
            tracing::warn!("No bot registered for group_id={}", body.group_id);
            None
        }
    };

    // Step 2: Save the reply to database
    let row = sqlx::query(
        r#"
        INSERT INTO groups.group_messages (
            group_id, user_id, username, content, msg_type,
            tg_msg_id, quoted_tg_msg_id, quoted_text, sender_type, tenant_id, timestamp
        ) VALUES ($1, $2, 'admin', $3, 'text', $4, $5, $6, 'admin', $7, now())
        RETURNING id
        "#,
    )
    .bind(body.group_id)
    .bind(admin_user_id)
    .bind(body.content.trim())
    .bind(tg_msg_id)  // Store the actual Telegram message ID
    .bind(body.tg_msg_id)  // Store the message being replied to
    .bind(quoted_text_ref)
    .bind(tenant_id)
    .fetch_one(state.db()?)
    .await?;

    let msg_id: i64 = row.try_get("id")?;

    tracing::info!("reply_group_message: saved msg_id={msg_id}, tg_msg_id={:?} replying to original tg_msg_id={}", tg_msg_id, body.tg_msg_id);

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "msg_id": msg_id,
            "tg_msg_id": tg_msg_id,
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_messages/delete
// ---------------------------------------------------------------------------

pub async fn delete_group_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GroupMessageDeleteRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;

    let tenant_id = resolve_tenant_id(&scope)?;

    // Verify the message belongs to this tenant and group
    let result = sqlx::query(
        "DELETE FROM groups.group_messages WHERE id = $1 AND group_id = $2 AND tenant_id = $3",
    )
    .bind(body.msg_id)
    .bind(body.group_id)
    .bind(tenant_id)
    .execute(state.db()?)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("message_not_found".into()));
    }

    // TODO: Send delete command to Telegram bot

    Ok(Json(json!({"status": "ok"})))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the tenant_id from the scope. Platform admins get an empty string
/// which means "no tenant filter" (they can see everything).
fn resolve_tenant_id(scope: &tgbot_domain::scope::Scope) -> Result<&str, ApiError> {
    if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)
    } else {
        // Platform admin: no tenant filter
        Ok("")
    }
}

/// Require write access (owner or admin role). Viewers are read-only.
fn require_write_access(scope: &tgbot_domain::scope::Scope) -> Result<(), ApiError> {
    if matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin | Role::PlatformAdmin) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
