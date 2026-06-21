//! Chat / Messages API — migrated from backend/app/api/chat.py
//!
//! Endpoints:
//!   GET  /api/users                     — inbox user list
//!   GET  /api/messages/{user_id}        — chat history
//!   POST /api/messages/{user_id}        — send message (text or image)
//!   POST /api/messages/{user_id}/delete — delete single message
//!   POST /api/messages/{user_id}/ai     — AI reply (stub)
//!   POST /api/mark_read/{user_id}       — mark messages as read
//!   POST /api/delete_chat/{user_id}     — soft-delete a chat
//!   GET  /api/search                    — global message search

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_repos::{audit_repo, message_repo, user_repo};

use crate::auth::resolve_scope;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Query / body structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub group_id: Option<i64>,
    pub user_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: Option<String>,
    pub reply_to_msg_id: Option<i64>,
    pub file_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteMessageRequest {
    pub msg_id: i64,
}

// ---------------------------------------------------------------------------
// GET /api/users
// ---------------------------------------------------------------------------

pub async fn get_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let users = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
        user_repo::get_inbox_list_for_tenant(&state.pool, tenant_id).await?
    } else {
        user_repo::get_inbox_list(&state.pool).await?
    };

    let total = users.len();
    tracing::info!("get_users: returning {total} users");
    Ok(Json(json!({"status": "ok", "data": users, "total": total})))
}

// ---------------------------------------------------------------------------
// GET /api/messages/{user_id}
// ---------------------------------------------------------------------------

pub async fn get_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let page = q.page.unwrap_or(1).max(1);
    let limit: i64 = 50;
    let offset = (page - 1) * limit;

    // Tenant isolation check for merchant users
    if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
        let bot_id = user_repo::get_bot_id_for_tenant(&state.pool, tenant_id)
            .await?
            .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;

        let belongs =
            user_repo::is_user_belong_to_tenant(&state.pool, user_id, tenant_id, bot_id).await?;

        if !belongs {
            // Audit the cross-tenant access attempt
            let _ = audit_repo::log_tenant_isolation_event(
                &state.pool,
                tenant_id,
                "cross_tenant_access_denied",
                "message",
                &user_id.to_string(),
                scope.role.as_str(),
                &format!("/api/messages/{user_id}"),
            )
            .await;

            return Err(ApiError::Forbidden);
        }
    }

    // Resolve bot_id for the query
    let bot_id = resolve_bot_id_for_scope(&state, &scope).await?;

    let messages = if let Some(bid) = bot_id {
        user_repo::get_chat_history(&state.pool, user_id, bid, limit, offset).await?
    } else {
        // Platform admin: try to find the bot_id from the merchant context
        // If not available, use user_id=0 as a sentinel (will return empty)
        user_repo::get_chat_history(&state.pool, user_id, 0, limit, offset).await?
    };

    let total = messages.len();
    Ok(Json(
        json!({"status": "ok", "data": messages, "total": total}),
    ))
}

// ---------------------------------------------------------------------------
// POST /api/messages/{user_id}
// ---------------------------------------------------------------------------

pub async fn send_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(body): Json<SendMessageRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let content = body.content.as_deref().unwrap_or("").trim();
    let file_id = body.file_id.as_deref().unwrap_or("");

    if content.is_empty() && file_id.is_empty() {
        return Err(ApiError::BadRequest("消息内容不能为空".into()));
    }

    // Resolve tenant_id and bot_id for this scope
    let tenant_id = scope.tenant_id.as_deref().unwrap_or("default");

    let bot_id = resolve_bot_id_for_scope(&state, &scope).await?;
    let admin_user_id: i64 = resolve_admin_user_id(&headers);

    // Determine message type
    let msg_type = if !file_id.is_empty() { "photo" } else { "text" };
    let display_content = if content.is_empty() && msg_type == "photo" {
        "[图片]"
    } else {
        content
    };

    // TODO: In a full implementation, the message would be sent to Telegram
    // via the bot engine first. For now, we save it directly to the database
    // with a placeholder tg_msg_id. The actual Telegram sending will be
    // wired up when the bot crate is integrated with the HTTP layer.

    let saved_msg_id = message_repo::save_message(
        &state.pool,
        admin_user_id, // from: admin
        user_id,       // to: telegram user
        Some(display_content),
        msg_type,
        if file_id.is_empty() {
            None
        } else {
            Some(file_id)
        },
        None, // media_url
        true, // is_read
        None, // tg_msg_id (placeholder)
        tenant_id,
        body.reply_to_msg_id,
    )
    .await?;

    tracing::info!("send_message: saved msg_id={saved_msg_id} to user {user_id}");

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "msg_id": saved_msg_id,
            "tg_msg_id": serde_json::Value::Null,
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/messages/{user_id}/delete
// ---------------------------------------------------------------------------

pub async fn delete_single_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(body): Json<DeleteMessageRequest>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    if body.msg_id <= 0 {
        return Err(ApiError::BadRequest("消息 ID 无效".into()));
    }

    let success = message_repo::delete_single_message(&state.pool, user_id, body.msg_id).await?;

    if success {
        Ok(Json(json!({"status": "ok"})))
    } else {
        Err(ApiError::BadRequest("删除失败".into()))
    }
}

// ---------------------------------------------------------------------------
// POST /api/messages/{user_id}/ai  (stub)
// ---------------------------------------------------------------------------

pub async fn ai_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    // Stub: the actual AI reply pipeline is handled by the workers crate.
    // This endpoint will be fully implemented when the agent crate integration
    // is complete.
    tracing::info!("ai_reply: stub called for user_id={user_id}");
    Ok(Json(json!({
        "status": "ok",
        "data": {
            "msg_id": serde_json::Value::Null,
            "content": "[AI回复功能尚未集成]",
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/mark_read/{user_id}
// ---------------------------------------------------------------------------

pub async fn mark_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let bot_id = resolve_bot_id_for_scope(&state, &scope).await?;
    let bid = bot_id.unwrap_or(0);

    user_repo::mark_messages_as_read(&state.pool, user_id, bid).await?;

    Ok(Json(json!({"status": "ok"})))
}

// ---------------------------------------------------------------------------
// POST /api/delete_chat/{user_id}
// ---------------------------------------------------------------------------

pub async fn delete_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    user_repo::soft_delete_chat(&state.pool, user_id).await?;

    Ok(Json(json!({"status": "ok"})))
}

// ---------------------------------------------------------------------------
// GET /api/search
// ---------------------------------------------------------------------------

pub async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let keyword = q.q.as_deref().unwrap_or("").trim();
    if keyword.is_empty() {
        return Err(ApiError::BadRequest("请输入搜索关键词".into()));
    }

    let limit = q.limit.unwrap_or(30).clamp(1, 200);

    let tenant_id = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        scope.tenant_id.as_deref()
    } else {
        None
    };

    let bot_id = resolve_bot_id_for_scope(&state, &scope).await?;

    let results = message_repo::search_messages(
        &state.pool,
        keyword,
        q.group_id,
        q.user_id,
        limit,
        tenant_id,
        bot_id,
    )
    .await?;

    let total = results.len();
    Ok(Json(
        json!({"status": "ok", "data": results, "total": total}),
    ))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the bot_id (Telegram bot user id) for the current scope.
/// Returns None for platform admins (who can see everything).
async fn resolve_bot_id_for_scope(
    state: &AppState,
    scope: &tgbot_domain::scope::Scope,
) -> ApiResult<Option<i64>> {
    if matches!(scope.role, Role::PlatformAdmin) {
        return Ok(None);
    }
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    let bot_id = user_repo::get_bot_id_for_tenant(&state.pool, tenant_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
    Ok(Some(bot_id))
}

/// Extract the admin user id from the X-Admin-User-Id header.
/// Falls back to 0 if not present (used as from_userid when saving messages).
fn resolve_admin_user_id(headers: &HeaderMap) -> i64 {
    headers
        .get("x-admin-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
