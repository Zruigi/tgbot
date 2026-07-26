//! Group Listen API — frontend-compatible group monitoring endpoints
//!
//! Endpoints:
//!   GET  /api/group_listen/stats                    — get monitor statistics
//!   POST /api/group_listen/toggle                   — toggle monitoring for a group
//!   POST /api/group_listen/revoke_sync              — toggle revoke sync
//!   POST /api/group_listen/create_sync_chat         — create a revoke sync chat
//!   GET  /api/group_listen/sync_chat_info           — get revoke sync chat info
//!   GET  /api/group_listen/verified_users           — list verified users
//!   GET  /api/group_listen/unverified_users         — list unverified users
//!   POST /api/group_listen/send_channel_reminder    — send channel subscription reminder
//!   GET  /api/group_listen/sync_chat_assignments    — get sync chat assignments
//!   GET  /api/group_photo                           — get group photo file_id
//!   GET  /api/group_member_count                    — get group member count

use axum::{extract::State, http::HeaderMap, Json};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_integrations::telegram::SendMessageParams;

use crate::auth::resolve_scope;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GroupIdQuery {
    pub group_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct ChannelReminderRequest {
    pub group_id: i64,
    pub user_ids: Vec<i64>,
    pub channel_link: String,
}

// ---------------------------------------------------------------------------
// GET /api/group_listen/stats
// ---------------------------------------------------------------------------

pub async fn get_monitor_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    let row = sqlx::query(
        "SELECT COUNT(*)::bigint AS total_groups, \
                COUNT(*) FILTER (WHERE is_monitored)::bigint AS monitored_groups, \
                COUNT(*) FILTER (WHERE updated_at >= now() - interval '10 minutes')::bigint AS active_groups \
         FROM groups.tracked_groups WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(state.db()?)
    .await?;
    Ok(Json(json!({
        "status": "ok",
        "total_groups": row.try_get::<i64, _>("total_groups")?,
        "monitored_groups": row.try_get::<i64, _>("monitored_groups")?,
        "active_groups": row.try_get::<i64, _>("active_groups")?,
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
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let group_id = body
        .get("group_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::BadRequest("missing group_id".into()))?;
    let enabled = body
        .get("enabled")
        .or_else(|| body.get("listen_status"))
        .and_then(|value| value.as_bool())
        .unwrap_or(true);

    let updated = sqlx::query(
        "UPDATE groups.tracked_groups SET is_monitored = $1, updated_at = now() WHERE group_id = $2 AND tenant_id = $3"
    )
    .bind(enabled)
    .bind(group_id)
    .bind(tenant_id)
    .execute(state.db()?)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    Ok(Json(
        json!({"status": "ok", "group_id": group_id, "is_monitored": enabled}),
    ))
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
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let group_id = body
        .get("group_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError::BadRequest("missing group_id".into()))?;
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let updated = sqlx::query(
        "UPDATE groups.tracked_groups SET delete_on_revoke = $1, updated_at = now() WHERE group_id = $2 AND tenant_id = $3"
    )
    .bind(enabled)
    .bind(group_id)
    .bind(tenant_id)
    .execute(state.db()?)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    Ok(Json(
        json!({"status": "ok", "group_id": group_id, "delete_on_revoke": enabled}),
    ))
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
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let chat_id = body.get("chat_id").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str()?.trim().parse().ok())
    });

    if let Some(chat_id) = chat_id {
        sqlx::query(
            "INSERT INTO tenant_saas.tenant_configs (tenant_id, config_key, config_value, updated_at) \
             VALUES ($1, 'revoke_sync_chat_id', $2, now()) \
             ON CONFLICT (tenant_id, config_key) DO UPDATE \
             SET config_value = EXCLUDED.config_value, updated_at = now()",
        )
        .bind(tenant_id)
        .bind(chat_id.to_string())
        .execute(state.db()?)
        .await?;
    } else {
        sqlx::query(
            "DELETE FROM tenant_saas.tenant_configs \
             WHERE tenant_id = $1 AND config_key = 'revoke_sync_chat_id'",
        )
        .bind(tenant_id)
        .execute(state.db()?)
        .await?;
        sqlx::query(
            "UPDATE groups.tracked_groups SET revoke_sync_chat_id = NULL, delete_on_revoke = false, updated_at = now() \
             WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .execute(state.db()?)
        .await?;
    }

    Ok(Json(json!({
        "status": "ok",
        "chat_id": chat_id,
        "data": {"chat_id": chat_id}
    })))
}

// ---------------------------------------------------------------------------
// GET /api/group_listen/sync_chat_info
// ---------------------------------------------------------------------------

pub async fn get_revoke_sync_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let chat_id = sqlx::query_scalar::<_, String>(
        "SELECT config_value FROM tenant_saas.tenant_configs \
         WHERE tenant_id = $1 AND config_key = 'revoke_sync_chat_id'",
    )
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?
    .and_then(|value| value.parse::<i64>().ok());
    let rows = sqlx::query(
        "SELECT group_id, group_title FROM groups.tracked_groups \
         WHERE tenant_id = $1 AND revoke_sync_chat_id = $2 ORDER BY group_id",
    )
    .bind(tenant_id)
    .bind(chat_id)
    .fetch_all(state.db()?)
    .await?;
    let assigned_groups: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "group_id": row.try_get::<i64, _>("group_id").unwrap_or_default(),
                "group_title": row.try_get::<Option<String>, _>("group_title").unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(json!({
        "status": "ok",
        "chat_id": chat_id,
        "data": {"chat_id": chat_id, "assigned_groups": assigned_groups}
    })))
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
         ORDER BY verified_at DESC LIMIT 200",
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

    Ok(Json(
        json!({"status": "ok", "data": users, "total": users.len()}),
    ))
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
         ORDER BY joined_at DESC LIMIT 200",
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

    Ok(Json(
        json!({"status": "ok", "data": users, "total": users.len()}),
    ))
}

// ---------------------------------------------------------------------------
// POST /api/group_listen/send_channel_reminder
// ---------------------------------------------------------------------------

pub async fn send_channel_reminder(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ChannelReminderRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    if body.user_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids cannot be empty".into()));
    }
    if body.user_ids.len() > 100 {
        return Err(ApiError::BadRequest("batch size cannot exceed 100".into()));
    }
    let channel_link = normalize_channel_link(&body.channel_link)?;
    let verify_channel_id = sqlx::query_scalar::<_, Option<String>>(
        "SELECT verify_channel_id FROM groups.tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::NotFound("group_not_found".into()))?
    .and_then(|value| value.parse::<i64>().ok());
    let mut seen_ids = HashSet::new();
    let requested_ids: Vec<i64> = body
        .user_ids
        .iter()
        .copied()
        .filter(|user_id| *user_id > 0 && seen_ids.insert(*user_id))
        .collect();
    if requested_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids must contain a user".into()));
    }
    let member_rows = sqlx::query(
        "SELECT user_id, username FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)",
    )
    .bind(body.group_id)
    .bind(tenant_id)
    .bind(&requested_ids)
    .fetch_all(state.db()?)
    .await?;
    let members: HashMap<i64, Option<String>> = member_rows
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get::<i64, _>("user_id")?,
                row.try_get::<Option<String>, _>("username")?,
            ))
        })
        .collect::<Result<_, sqlx::Error>>()?;
    let (_, telegram) = state.tenant_telegram(tenant_id).await?;
    let has_verify_button = verify_channel_id.is_some();
    let mut inline_keyboard = vec![vec![json!({
        "text": "📢 直接关注",
        "url": channel_link,
    })]];
    if let Some(verify_channel_id) = verify_channel_id {
        inline_keyboard.push(vec![json!({
            "text": "✅ 回补订阅解开禁言",
            "callback_data": format!("subscribe_{}_{}", body.group_id, verify_channel_id),
        })]);
    }
    let reply_markup = json!({ "inline_keyboard": inline_keyboard });
    let expires_at = Utc::now() + Duration::hours(12);
    let mut results = Vec::with_capacity(requested_ids.len());
    let mut sent_count = 0usize;
    for user_id in requested_ids {
        let Some(username) = members.get(&user_id) else {
            results.push(json!({ "user_id": user_id, "error": "member_not_found" }));
            continue;
        };
        let mention = username
            .as_deref()
            .filter(|value| is_telegram_username(value))
            .map(|value| format!("@{value}"))
            .unwrap_or_else(|| format!("<a href=\"tg://user?id={user_id}\">用户</a>"));
        let verification_hint = if has_verify_button {
            "\n🤖 或点击下方按钮: 回补订阅解开禁言"
        } else {
            ""
        };
        let text = format!(
            "🔔 {mention}，请关注我们的频道以获得最新信息！\n\n📢 频道链接: {channel_link}{verification_hint}\n\n⏰ 提醒有效期为12小时"
        );
        match telegram
            .send_message(SendMessageParams {
                chat_id: body.group_id,
                text,
                parse_mode: Some("HTML".into()),
                reply_markup: Some(reply_markup.clone()),
                reply_to_message_id: None,
            })
            .await
        {
            Ok(message) => {
                let reminder_id = sqlx::query_scalar::<_, i64>(
                    "INSERT INTO groups.channel_reminders (group_id, user_id, message_id, expires_at, tenant_id) VALUES ($1, $2, $3, $4, $5) RETURNING id",
                )
                .bind(body.group_id)
                .bind(user_id)
                .bind(message.message_id)
                .bind(&expires_at)
                .bind(tenant_id)
                .fetch_one(state.db()?)
                .await?;
                sent_count += 1;
                results.push(json!({
                    "user_id": user_id,
                    "username": username,
                    "message_id": message.message_id,
                    "reminder_id": reminder_id,
                    "sent_at": Utc::now(),
                }));
            }
            Err(error) => results.push(json!({
                "user_id": user_id,
                "error": error.to_string(),
            })),
        }
    }

    Ok(Json(json!({
        "status": "ok",
        "sent_count": sent_count,
        "results": results,
    })))
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
         FROM groups.tracked_groups \
         WHERE revoke_sync_chat_id IS NOT NULL AND tenant_id = $1 \
         ORDER BY updated_at DESC",
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

    Ok(Json(
        json!({"status": "ok", "data": assignments, "total": assignments.len()}),
    ))
}

pub async fn save_sync_chat_assignments(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    let chat_id = body
        .get("chat_id")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str()?.trim().parse().ok())
        })
        .ok_or_else(|| ApiError::BadRequest("chat_id is required".into()))?;
    let mut group_ids: Vec<i64> = body
        .get("group_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::BadRequest("group_ids is required".into()))?
        .iter()
        .filter_map(Value::as_i64)
        .collect();
    group_ids.sort_unstable();
    group_ids.dedup();

    let configured_chat_id = sqlx::query_scalar::<_, String>(
        "SELECT config_value FROM tenant_saas.tenant_configs \
         WHERE tenant_id = $1 AND config_key = 'revoke_sync_chat_id'",
    )
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?
    .and_then(|value| value.parse::<i64>().ok());
    if configured_chat_id != Some(chat_id) {
        return Err(ApiError::BadRequest(
            "chat_id is not configured for this tenant".into(),
        ));
    }

    let mut transaction = state.db()?.begin().await?;
    let existing_rows = sqlx::query(
        "SELECT group_id FROM groups.tracked_groups \
         WHERE tenant_id = $1 AND revoke_sync_chat_id = $2",
    )
    .bind(tenant_id)
    .bind(chat_id)
    .fetch_all(&mut *transaction)
    .await?;
    let existing_ids: Vec<i64> = existing_rows
        .iter()
        .filter_map(|row| row.try_get("group_id").ok())
        .collect();

    if !group_ids.is_empty() {
        let owned_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM groups.tracked_groups \
             WHERE tenant_id = $1 AND group_id = ANY($2)",
        )
        .bind(tenant_id)
        .bind(&group_ids)
        .fetch_one(&mut *transaction)
        .await?;
        if owned_count != group_ids.len() as i64 {
            return Err(ApiError::BadRequest(
                "one or more groups do not belong to this tenant".into(),
            ));
        }
    }

    sqlx::query(
        "UPDATE groups.tracked_groups SET revoke_sync_chat_id = NULL, updated_at = now() \
         WHERE tenant_id = $1 AND revoke_sync_chat_id = $2",
    )
    .bind(tenant_id)
    .bind(chat_id)
    .execute(&mut *transaction)
    .await?;
    let removed_ids: Vec<i64> = existing_ids
        .into_iter()
        .filter(|group_id| !group_ids.contains(group_id))
        .collect();
    if !removed_ids.is_empty() {
        sqlx::query(
            "UPDATE groups.tracked_groups SET delete_on_revoke = false, updated_at = now() \
             WHERE tenant_id = $1 AND group_id = ANY($2)",
        )
        .bind(tenant_id)
        .bind(&removed_ids)
        .execute(&mut *transaction)
        .await?;
    }
    if !group_ids.is_empty() {
        sqlx::query(
            "UPDATE groups.tracked_groups SET revoke_sync_chat_id = $1, updated_at = now() \
             WHERE tenant_id = $2 AND group_id = ANY($3)",
        )
        .bind(chat_id)
        .bind(tenant_id)
        .bind(&group_ids)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Json(json!({
        "status": "ok",
        "message": "校对群授权已保存",
        "data": {"assigned_group_ids": group_ids}
    })))
}

// ---------------------------------------------------------------------------
// GET /api/group_photo
// ---------------------------------------------------------------------------

pub async fn get_group_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<GroupIdQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    ensure_group_owned(&state, tenant_id, query.group_id).await?;
    let (_, telegram) = state.tenant_telegram(tenant_id).await?;
    let chat = telegram
        .get_chat(query.group_id)
        .await
        .map_err(ApiError::Anyhow)?;
    let file_id = chat
        .get("photo")
        .and_then(|photo| photo.get("small_file_id"))
        .and_then(Value::as_str);
    Ok(Json(json!({
        "status": "ok",
        "file_id": file_id,
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
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    ensure_group_owned(&state, tenant_id, query.group_id).await?;
    let (_, telegram) = state.tenant_telegram(tenant_id).await?;
    let count = telegram
        .get_chat_member_count(query.group_id)
        .await
        .map_err(ApiError::Anyhow)?;

    Ok(Json(json!({
        "status": "ok",
        "count": count,
    })))
}

async fn ensure_group_owned(state: &AppState, tenant_id: &str, group_id: i64) -> ApiResult<()> {
    let owned = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM groups.tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(tenant_id)
    .fetch_optional(state.db()?)
    .await?;
    if owned.is_some() {
        Ok(())
    } else {
        Err(ApiError::NotFound("group_not_found".into()))
    }
}

fn require_write_access(scope: &tgbot_domain::scope::Scope) -> ApiResult<()> {
    if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::PlatformAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn normalize_channel_link(value: &str) -> ApiResult<String> {
    let value = value.trim();
    let path = value
        .strip_prefix("https://t.me/")
        .or_else(|| value.strip_prefix("http://t.me/"))
        .or_else(|| value.strip_prefix("t.me/"))
        .or_else(|| value.strip_prefix('@'))
        .unwrap_or(value);
    if path.is_empty()
        || path.len() > 160
        || path.starts_with('/')
        || path.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '+' | '-' | '/'))
        })
    {
        return Err(ApiError::BadRequest("invalid Telegram channel link".into()));
    }
    Ok(format!("https://t.me/{path}"))
}

fn is_telegram_username(value: &str) -> bool {
    (5..=32).contains(&value.len())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}
