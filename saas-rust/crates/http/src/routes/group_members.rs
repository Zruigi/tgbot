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
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashSet;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_integrations::telegram::{SendMessageParams, SendPhotoParams, TelegramClient};

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
    pub duration: Option<i32>,
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
    pub permissions: Option<Value>,
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
    pub reply_to_tg_msg_id: Option<i64>,
    pub quoted_text: Option<String>,
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
    pub msg_id: Option<i64>,
    pub tg_msg_id: Option<i64>,
    pub user_id: Option<i64>,
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
        WHERE ($1::text IS NULL OR tenant_id = $1)
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
    let tenant_id = resolve_group_tenant_id(&state, &scope, q.group_id).await?;

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
        .bind(&tenant_id)
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
    .bind(&tenant_id)
    .fetch_one(state.db()?)
    .await?;

    let verified = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND is_verified = true",
    )
    .bind(q.group_id)
    .bind(&tenant_id)
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
    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;
    let valid_actions = ["kick", "ban", "mute", "unmute", "verify", "unverify"];
    if !valid_actions.contains(&body.action.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "action must be one of: {}",
            valid_actions.join(", ")
        )));
    }
    ensure_member_exists(&state, &tenant_id, body.group_id, body.user_id).await?;

    if matches!(body.action.as_str(), "kick" | "ban" | "mute" | "unmute") {
        let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
        ensure_regular_telegram_member(&telegram, body.group_id, body.user_id).await?;
        execute_telegram_member_action(
            &telegram,
            body.group_id,
            body.user_id,
            &body.action,
            body.duration,
        )
        .await?;
    }

    let updated = match body.action.as_str() {
        "verify" => sqlx::query(
            "UPDATE groups.group_members SET is_verified = true, verified_at = now(), status = 'active', updated_at = now() WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(body.group_id)
        .bind(body.user_id)
        .bind(&tenant_id)
        .execute(state.db()?)
        .await?
        .rows_affected(),
        "unverify" => sqlx::query(
            "UPDATE groups.group_members SET is_verified = false, verified_at = NULL, updated_at = now() WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(body.group_id)
        .bind(body.user_id)
        .bind(&tenant_id)
        .execute(state.db()?)
        .await?
        .rows_affected(),
        action => {
            let status = match action {
                "kick" => "kicked",
                "ban" => "banned",
                "mute" => "muted",
                "unmute" => "active",
                _ => unreachable!(),
            };
            sqlx::query(
                "UPDATE groups.group_members SET status = $1, updated_at = now() WHERE group_id = $2 AND user_id = $3 AND tenant_id = $4",
            )
            .bind(status)
            .bind(body.group_id)
            .bind(body.user_id)
            .bind(&tenant_id)
            .execute(state.db()?)
            .await?
            .rows_affected()
        }
    };

    if updated == 0 {
        return Err(ApiError::NotFound("member_not_found".into()));
    }

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "group_id": body.group_id,
            "user_id": body.user_id,
            "action": body.action,
            "reason": body.reason,
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

    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;

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

    let requested_ids: Vec<i64> = body
        .user_ids
        .iter()
        .copied()
        .filter(|user_id| *user_id > 0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if requested_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids must contain a user".into()));
    }
    let existing_ids: HashSet<i64> = sqlx::query_scalar(
        "SELECT user_id FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)",
    )
    .bind(body.group_id)
    .bind(&tenant_id)
    .bind(&requested_ids)
    .fetch_all(state.db()?)
    .await?
    .into_iter()
    .collect();
    let mut success_ids = Vec::new();
    let mut failed = Vec::new();

    if matches!(body.action.as_str(), "kick" | "ban" | "mute" | "unmute") {
        let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
        for user_id in &requested_ids {
            if !existing_ids.contains(user_id) {
                failed.push(json!({ "user_id": user_id, "reason": "member_not_found" }));
                continue;
            }
            let result = async {
                ensure_regular_telegram_member(&telegram, body.group_id, *user_id).await?;
                execute_telegram_member_action(
                    &telegram,
                    body.group_id,
                    *user_id,
                    &body.action,
                    body.duration,
                )
                .await
            }
            .await;
            match result {
                Ok(()) => success_ids.push(*user_id),
                Err(error) => failed.push(json!({
                    "user_id": user_id,
                    "reason": error.to_string(),
                })),
            }
        }
    } else {
        success_ids.extend(
            requested_ids
                .iter()
                .filter(|id| existing_ids.contains(id))
                .copied(),
        );
        failed.extend(
            requested_ids
                .iter()
                .filter(|id| !existing_ids.contains(id))
                .map(|user_id| json!({ "user_id": user_id, "reason": "member_not_found" })),
        );
    }

    let affected = if success_ids.is_empty() {
        0
    } else if body.action == "verify" {
        sqlx::query(
            "UPDATE groups.group_members SET is_verified = true, verified_at = now(), status = 'active', updated_at = now() WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)",
        )
        .bind(body.group_id)
        .bind(&tenant_id)
        .bind(&success_ids)
        .execute(state.db()?)
        .await?
        .rows_affected()
    } else if body.action == "unverify" {
        sqlx::query(
            "UPDATE groups.group_members SET is_verified = false, verified_at = NULL, updated_at = now() WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)",
        )
        .bind(body.group_id)
        .bind(&tenant_id)
        .bind(&success_ids)
        .execute(state.db()?)
        .await?
        .rows_affected()
    } else {
        let status = match body.action.as_str() {
            "kick" => "kicked",
            "ban" => "banned",
            "mute" => "muted",
            "unmute" => "active",
            _ => unreachable!(),
        };
        sqlx::query(
            "UPDATE groups.group_members SET status = $1, updated_at = now() WHERE group_id = $2 AND tenant_id = $3 AND user_id = ANY($4)",
        )
        .bind(status)
        .bind(body.group_id)
        .bind(&tenant_id)
        .bind(&success_ids)
        .execute(state.db()?)
        .await?
        .rows_affected()
    };

    let success_count = success_ids.len();
    let failed_count = failed.len();
    Ok(Json(json!({
        "status": "ok",
        "action": body.action,
        "success_ids": success_ids,
        "failed": failed,
        "summary": {
            "total": requested_ids.len(),
            "success": success_count,
            "failed": failed_count,
        },
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

    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;

    if body.user_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids cannot be empty".into()));
    }
    if body.user_ids.len() > 100 {
        return Err(ApiError::BadRequest("batch size cannot exceed 100".into()));
    }

    if !matches!(body.action.as_str(), "promote" | "demote") {
        return Err(ApiError::BadRequest(
            "action must be promote or demote".into(),
        ));
    }

    let new_status = if body.action == "promote" {
        "admin"
    } else {
        "active"
    };
    let requested_ids: Vec<i64> = body
        .user_ids
        .iter()
        .copied()
        .filter(|user_id| *user_id > 0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let existing_ids: HashSet<i64> = sqlx::query_scalar(
        "SELECT user_id FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)",
    )
    .bind(body.group_id)
    .bind(&tenant_id)
    .bind(&requested_ids)
    .fetch_all(state.db()?)
    .await?
    .into_iter()
    .collect();
    let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
    let mut success_ids = Vec::new();
    let mut failed = Vec::new();
    for user_id in &requested_ids {
        if !existing_ids.contains(user_id) {
            failed.push(json!({ "user_id": user_id, "reason": "member_not_found" }));
            continue;
        }
        let result = async {
            ensure_not_group_creator(&telegram, body.group_id, *user_id).await?;
            telegram
                .promote_chat_member(
                    body.group_id,
                    *user_id,
                    admin_permissions(body.action == "promote", None),
                )
                .await
                .map_err(ApiError::Anyhow)?;
            Ok::<(), ApiError>(())
        }
        .await;
        match result {
            Ok(()) => success_ids.push(*user_id),
            Err(error) => failed.push(json!({
                "user_id": user_id,
                "reason": error.to_string(),
            })),
        }
    }
    let affected = if success_ids.is_empty() {
        0
    } else {
        sqlx::query(
            "UPDATE groups.group_members SET status = $1, updated_at = now() WHERE group_id = $2 AND tenant_id = $3 AND user_id = ANY($4)",
        )
        .bind(new_status)
        .bind(body.group_id)
        .bind(&tenant_id)
        .bind(&success_ids)
        .execute(state.db()?)
        .await?
        .rows_affected()
    };
    let success_count = success_ids.len();
    let failed_count = failed.len();

    Ok(Json(json!({
        "status": "ok",
        "action": body.action,
        "success_ids": success_ids,
        "failed": failed,
        "summary": {
            "total": requested_ids.len(),
            "success": success_count,
            "failed": failed_count,
        },
        "data": {
            "group_id": body.group_id,
            "action": body.action,
            "affected": affected,
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

    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;

    ensure_member_exists(&state, &tenant_id, body.group_id, body.user_id).await?;
    let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
    ensure_not_group_creator(&telegram, body.group_id, body.user_id).await?;
    telegram
        .promote_chat_member(
            body.group_id,
            body.user_id,
            admin_permissions(true, body.permissions.as_ref()),
        )
        .await
        .map_err(ApiError::Anyhow)?;

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
    .bind(&tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if row.is_none() {
        return Err(ApiError::NotFound("member_not_found".into()));
    }

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

    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;

    ensure_member_exists(&state, &tenant_id, body.group_id, body.user_id).await?;
    let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
    ensure_not_group_creator(&telegram, body.group_id, body.user_id).await?;
    telegram
        .promote_chat_member(body.group_id, body.user_id, admin_permissions(false, None))
        .await
        .map_err(ApiError::Anyhow)?;

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
    .bind(&tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if row.is_none() {
        return Err(ApiError::NotFound("member_not_found_or_not_admin".into()));
    }

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

    let tenant_id = resolve_tenant_id(&scope)?;

    let rows = sqlx::query(
        r#"
        SELECT * FROM groups.group_messages
        WHERE group_id = $1
          AND ($2::text IS NULL OR tenant_id = $2)
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
    .await?;

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

    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;

    if body.content.trim().is_empty() && body.file_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ApiError::BadRequest("content or file_id required".into()));
    }

    let content = body.content.trim();
    let file_id = body
        .file_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let msg_type = if file_id.is_some() { "photo" } else { "text" };
    let content_limit = if file_id.is_some() { 1024 } else { 4096 };
    if content.chars().count() > content_limit {
        return Err(ApiError::BadRequest(format!(
            "content exceeds {content_limit} characters"
        )));
    }
    if let Some(requested_type) = body.msg_type.as_deref() {
        if requested_type != msg_type {
            return Err(ApiError::BadRequest(
                "msg_type does not match payload".into(),
            ));
        }
    }
    let (bot_id, telegram) = state.tenant_telegram(&tenant_id).await?;
    let sent = if let Some(file_id) = file_id {
        telegram
            .send_photo(SendPhotoParams {
                chat_id: body.group_id,
                photo: file_id.to_string(),
                caption: (!content.is_empty()).then(|| content.to_string()),
                parse_mode: Some("HTML".into()),
                reply_to_message_id: body.reply_to_tg_msg_id,
                reply_markup: None,
            })
            .await
            .map_err(ApiError::Anyhow)?
    } else {
        telegram
            .send_message(SendMessageParams {
                chat_id: body.group_id,
                text: content.to_string(),
                parse_mode: Some("HTML".into()),
                reply_markup: None,
                reply_to_message_id: body.reply_to_tg_msg_id,
            })
            .await
            .map_err(ApiError::Anyhow)?
    };
    let quoted_text = body
        .quoted_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(200).collect::<String>());
    let row = sqlx::query(
        r#"
        INSERT INTO groups.group_messages (
            group_id, user_id, username, content, msg_type, file_id,
            tg_msg_id, quoted_tg_msg_id, quoted_text, sender_type,
            tenant_id, timestamp, message_ts
        ) VALUES ($1, $2, 'admin', $3, $4, $5, $6, $7, $8, 'admin', $9, now(), $10)
        RETURNING id
        "#,
    )
    .bind(body.group_id)
    .bind(bot_id)
    .bind(content)
    .bind(msg_type)
    .bind(file_id)
    .bind(sent.message_id)
    .bind(body.reply_to_tg_msg_id)
    .bind(quoted_text)
    .bind(&tenant_id)
    .bind(sent.date)
    .fetch_one(state.db()?)
    .await?;

    let msg_id: i64 = row.try_get("id")?;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "msg_id": msg_id,
            "tg_msg_id": sent.message_id,
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

    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;

    // Find the original message to get the group_id
    let orig = sqlx::query(
        "SELECT group_id, content FROM groups.group_messages WHERE tg_msg_id = $1 AND tenant_id = $2",
    )
    .bind(body.tg_msg_id)
    .bind(&tenant_id)
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

    let quoted_text: String = orig_content.unwrap_or_default().chars().take(200).collect();
    let (bot_id, telegram) = state.tenant_telegram(&tenant_id).await?;
    let sent = telegram
        .send_message(SendMessageParams {
            chat_id: body.group_id,
            text: body.content.trim().to_string(),
            parse_mode: Some("HTML".into()),
            reply_markup: None,
            reply_to_message_id: Some(body.tg_msg_id),
        })
        .await
        .map_err(ApiError::Anyhow)?;
    let row = sqlx::query(
        r#"
        INSERT INTO groups.group_messages (
            group_id, user_id, username, content, msg_type,
            tg_msg_id, quoted_tg_msg_id, quoted_text, sender_type,
            tenant_id, timestamp, message_ts
        ) VALUES ($1, $2, 'admin', $3, 'text', $4, $5, $6, 'admin', $7, now(), $8)
        RETURNING id
        "#,
    )
    .bind(body.group_id)
    .bind(bot_id)
    .bind(body.content.trim())
    .bind(sent.message_id)
    .bind(body.tg_msg_id)
    .bind(&quoted_text)
    .bind(&tenant_id)
    .bind(sent.date)
    .fetch_one(state.db()?)
    .await?;

    let msg_id: i64 = row.try_get("id")?;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "msg_id": msg_id,
            "tg_msg_id": sent.message_id,
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

    let tenant_id = resolve_group_tenant_id(&state, &scope, body.group_id).await?;

    if body.msg_id.is_none() && body.tg_msg_id.is_none() {
        return Err(ApiError::BadRequest("msg_id or tg_msg_id required".into()));
    }
    let row = sqlx::query(
        r#"
        SELECT id, tg_msg_id
        FROM groups.group_messages
        WHERE group_id = $1 AND tenant_id = $2
          AND (($3::bigint IS NOT NULL AND id = $3)
            OR ($4::bigint IS NOT NULL AND tg_msg_id = $4))
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(body.group_id)
    .bind(&tenant_id)
    .bind(body.msg_id)
    .bind(body.tg_msg_id)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::NotFound("message_not_found".into()))?;
    let msg_id: i64 = row.try_get("id")?;
    let tg_msg_id: i64 = row
        .try_get::<Option<i64>, _>("tg_msg_id")?
        .ok_or_else(|| ApiError::BadRequest("message has no Telegram id".into()))?;
    let (_, telegram) = state.tenant_telegram(&tenant_id).await?;
    telegram
        .delete_message(body.group_id, tg_msg_id)
        .await
        .map_err(ApiError::Anyhow)?;
    sqlx::query(
        "DELETE FROM groups.group_messages WHERE id = $1 AND group_id = $2 AND tenant_id = $3",
    )
    .bind(msg_id)
    .bind(body.group_id)
    .bind(&tenant_id)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "data": { "msg_id": msg_id, "tg_msg_id": tg_msg_id }
    })))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn resolve_group_tenant_id(
    state: &AppState,
    scope: &tgbot_domain::scope::Scope,
    group_id: i64,
) -> ApiResult<String> {
    let tenant_hint = resolve_tenant_id(scope)?;
    sqlx::query_scalar::<_, String>(
        "SELECT tenant_id FROM groups.tracked_groups \
         WHERE group_id = $1 AND ($2::text IS NULL OR tenant_id = $2)",
    )
    .bind(group_id)
    .bind(tenant_hint)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::NotFound("group_not_found".into()))
}

async fn ensure_member_exists(
    state: &AppState,
    tenant_id: &str,
    group_id: i64,
    user_id: i64,
) -> ApiResult<()> {
    let exists = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM groups.group_members WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(ApiError::NotFound("member_not_found".into()))
    }
}

async fn telegram_member_status(
    telegram: &TelegramClient,
    group_id: i64,
    user_id: i64,
) -> ApiResult<String> {
    let member = telegram
        .get_chat_member(group_id, user_id)
        .await
        .map_err(ApiError::Anyhow)?;
    member
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ApiError::BadRequest("Telegram member status unavailable".into()))
}

async fn ensure_regular_telegram_member(
    telegram: &TelegramClient,
    group_id: i64,
    user_id: i64,
) -> ApiResult<()> {
    let status = telegram_member_status(telegram, group_id, user_id).await?;
    if matches!(status.as_str(), "administrator" | "creator") {
        Err(ApiError::BadRequest(
            "cannot operate on an administrator or group creator".into(),
        ))
    } else {
        Ok(())
    }
}

async fn ensure_not_group_creator(
    telegram: &TelegramClient,
    group_id: i64,
    user_id: i64,
) -> ApiResult<()> {
    if telegram_member_status(telegram, group_id, user_id).await? == "creator" {
        Err(ApiError::BadRequest(
            "cannot change group creator permissions".into(),
        ))
    } else {
        Ok(())
    }
}

async fn execute_telegram_member_action(
    telegram: &TelegramClient,
    group_id: i64,
    user_id: i64,
    action: &str,
    duration_seconds: Option<i32>,
) -> ApiResult<()> {
    match action {
        "kick" => {
            telegram
                .ban_chat_member(group_id, user_id)
                .await
                .map_err(ApiError::Anyhow)?;
            telegram
                .unban_chat_member(group_id, user_id)
                .await
                .map_err(ApiError::Anyhow)?;
        }
        "ban" => {
            telegram
                .ban_chat_member(group_id, user_id)
                .await
                .map_err(ApiError::Anyhow)?;
        }
        "mute" => {
            let duration = duration_seconds.unwrap_or(1800).clamp(60, 31_536_000);
            let until_date = (Utc::now() + Duration::seconds(i64::from(duration))).timestamp();
            telegram
                .restrict_chat_member(
                    group_id,
                    user_id,
                    json!({ "can_send_messages": false }),
                    Some(until_date),
                )
                .await
                .map_err(ApiError::Anyhow)?;
        }
        "unmute" => {
            telegram
                .restrict_chat_member(
                    group_id,
                    user_id,
                    json!({
                        "can_send_messages": true,
                        "can_send_polls": true,
                        "can_send_other_messages": true,
                        "can_add_web_page_previews": true,
                        "can_change_info": false,
                        "can_invite_users": true,
                        "can_pin_messages": false,
                    }),
                    None,
                )
                .await
                .map_err(ApiError::Anyhow)?;
        }
        _ => {
            return Err(ApiError::BadRequest(
                "unsupported Telegram member action".into(),
            ))
        }
    }
    Ok(())
}

fn admin_permissions(promote: bool, custom: Option<&Value>) -> Value {
    const KEYS: [&str; 14] = [
        "can_manage_chat",
        "can_delete_messages",
        "can_manage_video_chats",
        "can_restrict_members",
        "can_promote_members",
        "can_change_info",
        "can_invite_users",
        "can_pin_messages",
        "can_post_messages",
        "can_edit_messages",
        "can_post_stories",
        "can_edit_stories",
        "can_delete_stories",
        "is_anonymous",
    ];
    let defaults = json!({
        "can_manage_chat": promote,
        "can_delete_messages": promote,
        "can_manage_video_chats": false,
        "can_restrict_members": promote,
        "can_promote_members": false,
        "can_change_info": false,
        "can_invite_users": promote,
        "can_pin_messages": promote,
        "can_post_messages": false,
        "can_edit_messages": false,
        "can_post_stories": false,
        "can_edit_stories": false,
        "can_delete_stories": false,
        "is_anonymous": false,
    });
    let mut permissions = defaults.as_object().cloned().unwrap_or_default();
    if promote {
        if let Some(custom) = custom.and_then(Value::as_object) {
            for key in KEYS {
                if let Some(value) = custom.get(key).and_then(Value::as_bool) {
                    permissions.insert(key.to_string(), Value::Bool(value));
                }
            }
        }
    }
    Value::Object(permissions)
}

fn resolve_tenant_id(scope: &tgbot_domain::scope::Scope) -> Result<Option<&str>, ApiError> {
    match &scope.role {
        Role::PlatformAdmin => Ok(None),
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer => {
            Ok(Some(scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?))
        }
    }
}

/// Require write access (owner or admin role). Viewers are read-only.
fn require_write_access(scope: &tgbot_domain::scope::Scope) -> Result<(), ApiError> {
    if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::PlatformAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(role: Role, tenant_id: Option<&str>) -> tgbot_domain::scope::Scope {
        tgbot_domain::scope::Scope {
            authenticated: true,
            user_id: None,
            actor_key: None,
            role: role.clone(),
            actor_role: role,
            tenant_id: tenant_id.map(str::to_string),
            merchant_id: None,
        }
    }

    #[test]
    fn platform_scope_uses_no_tenant_filter() {
        assert_eq!(
            resolve_tenant_id(&scope(Role::PlatformAdmin, None)).unwrap(),
            None
        );
    }

    #[test]
    fn merchant_scope_keeps_tenant_filter() {
        assert_eq!(
            resolve_tenant_id(&scope(Role::MerchantAdmin, Some("tenant-a"))).unwrap(),
            Some("tenant-a")
        );
    }

    #[test]
    fn merchant_scope_without_tenant_is_forbidden() {
        assert!(resolve_tenant_id(&scope(Role::MerchantAdmin, None)).is_err());
    }
}
