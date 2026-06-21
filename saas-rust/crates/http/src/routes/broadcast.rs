//! Broadcast and Push Task API routes.
//!
//! Endpoints:
//! - GET    /api/broadcast/targets              -> List broadcast targets
//! - POST   /api/broadcast/targets/sync         -> Sync targets from bot
//! - POST   /api/broadcast                      -> Send broadcast to users/chats
//! - POST   /api/broadcast/chats                -> Send broadcast to chats only
//! - POST   /api/broadcast/send                 -> Alias for /api/broadcast
//! - POST   /api/broadcast/unified              -> Unified broadcast endpoint
//! - GET    /api/push_tasks                     -> List push tasks
//! - POST   /api/push_tasks                     -> Create push task
//! - POST   /api/push_tasks/status              -> Update push task status
//! - POST   /api/push_tasks/run_now             -> Run push task immediately
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::resolve_scope;
use crate::state::AppState;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::{
    BroadcastChatsRequest, BroadcastSendRequest, BroadcastTargetsQuery, CreatePushTaskRequest,
    PushTasksQuery, RunPushTaskNowRequest, UpdatePushTaskStatusRequest,
};
use tgbot_repos::broadcast_repo;
use tgbot_repos::push_task_repo;

/// GET /api/broadcast/targets - List broadcast targets
pub async fn list_broadcast_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<BroadcastTargetsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    // Parse chat_types from comma-separated string
    let chat_types: Option<Vec<String>> = q
        .chat_types
        .as_ref()
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect());

    let enabled_only = q.enabled_only.unwrap_or(true);

    let targets = broadcast_repo::list_broadcast_targets(
        &state.pool,
        &tenant_id,
        chat_types.as_deref(),
        enabled_only,
    )
    .await
    .map_err(ApiError::Anyhow)?;

    let total = targets.len();
    Ok(Json(json!({ "status": "ok", "data": targets, "total": total })))
}

/// POST /api/broadcast/targets/sync - Sync targets from bot (placeholder)
pub async fn sync_broadcast_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    // Placeholder: In production, this would fetch chats from the Telegram bot
    // and upsert them to the database.
    // For now, we return the current list of targets.

    let targets = broadcast_repo::list_broadcast_targets(
        &state.pool,
        &tenant_id,
        None,
        true,
    )
    .await
    .map_err(ApiError::Anyhow)?;

    let summary = json!({
        "synced": 0,
        "total": targets.len(),
    });

    Ok(Json(json!({ "status": "ok", "summary": summary, "data": targets })))
}

/// POST /api/broadcast - Send broadcast to users and/or chats
pub async fn send_broadcast(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BroadcastSendRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let content = body.content.unwrap_or_default();
    if content.is_empty() {
        return Err(ApiError::BadRequest("content is required".into()));
    }

    let do_translate = body.do_translate.unwrap_or(false);
    let user_ids = body.user_ids.unwrap_or_default();
    let chat_ids = body.chat_ids.unwrap_or_default();

    if user_ids.is_empty() && chat_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids or chat_ids required".into()));
    }

    // Import broadcast service
    use tgbot_services::broadcast_service;

    let mut user_success_ids: Vec<i64> = Vec::new();
    let mut failed_items: Vec<Value> = Vec::new();

    // Send to users
    if !user_ids.is_empty() {
        let result = broadcast_service::send_broadcast_to_users(
            &state.pool,
            &user_ids,
            &content,
            do_translate,
        )
        .await
        .map_err(ApiError::Anyhow)?;

        let success_count = result.get("success").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
        for (i, uid) in user_ids.iter().enumerate() {
            if i < success_count {
                user_success_ids.push(*uid);
            } else if i < user_ids.len() {
                failed_items.push(json!({ "user_id": uid, "reason": "send failed" }));
            }
        }
    }

    // Send to chats
    let chat_summary = if !chat_ids.is_empty() {
        let result = broadcast_service::send_broadcast_to_chats(
            &state.pool,
            &chat_ids,
            &content,
            do_translate,
        )
        .await
        .map_err(ApiError::Anyhow)?;
        result
    } else {
        json!({ "total": 0, "success": 0, "failed": 0 })
    };

    let summary = json!({
        "total": user_ids.len() + chat_ids.len(),
        "success": user_success_ids.len() + chat_summary.get("success").and_then(|v| v.as_i64()).unwrap_or(0),
        "failed": failed_items.len() + chat_summary.get("failed").and_then(|v| v.as_i64()).unwrap_or(0),
        "success_user_ids": user_success_ids,
        "success_chat_ids": chat_summary.get("success_ids"),
        "failed_items": failed_items,
    });

    Ok(Json(json!({ "status": "ok", "summary": summary })))
}

/// POST /api/broadcast/chats - Send broadcast to chats only
pub async fn broadcast_chats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BroadcastChatsRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let content = body.content.unwrap_or_default();
    if content.is_empty() {
        return Err(ApiError::BadRequest("content is required".into()));
    }

    let chat_ids = body.chat_ids.unwrap_or_default();
    if chat_ids.is_empty() {
        return Err(ApiError::BadRequest("chat_ids required".into()));
    }

    let do_translate = body.do_translate.unwrap_or(false);

    use tgbot_services::broadcast_service;

    let summary = broadcast_service::send_broadcast_to_chats(
        &state.pool,
        &chat_ids,
        &content,
        do_translate,
    )
    .await
    .map_err(ApiError::Anyhow)?;

    let failed_items: Vec<Value> = summary
        .get("failed_items")
        .and_then(|v| v.as_array())
        .map(|arr| arr.clone())
        .unwrap_or_default();

    Ok(Json(json!({ "status": "ok", "summary": summary, "failed_items": failed_items })))
}

/// POST /api/broadcast/send - Alias for /api/broadcast
pub async fn send_broadcast_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BroadcastSendRequest>,
) -> ApiResult<Json<Value>> {
    send_broadcast(State(state), headers, Json(body)).await
}

/// POST /api/broadcast/unified - Unified broadcast endpoint (same as /api/broadcast)
pub async fn broadcast_unified(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BroadcastSendRequest>,
) -> ApiResult<Json<Value>> {
    send_broadcast(State(state), headers, Json(body)).await
}

/// GET /api/push_tasks - List push tasks
pub async fn list_push_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PushTasksQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let tasks =
        push_task_repo::list_push_tasks(&state.pool, Some(&tenant_id), q.status.as_deref(), limit, offset)
            .await
            .map_err(ApiError::Anyhow)?;

    let total = tasks.len();
    Ok(Json(json!({ "status": "ok", "data": tasks, "total": total })))
}

/// POST /api/push_tasks - Create a new push task
pub async fn create_push_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreatePushTaskRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let task_name = body
        .task_name
        .unwrap_or_else(|| format!("Task-{}", Utc::now().timestamp()));

    let content = body.content.unwrap_or_default();
    if content.is_empty() {
        return Err(ApiError::BadRequest("content is required".into()));
    }

    let target_chat_ids = body.target_chat_ids.unwrap_or_default();
    if target_chat_ids.is_empty() {
        return Err(ApiError::BadRequest("target_chat_ids required".into()));
    }

    let start_at = body.start_at.unwrap_or_default();
    if start_at.is_empty() {
        return Err(ApiError::BadRequest("start_at is required".into()));
    }

    let interval_hours = body.interval_hours.unwrap_or(6);
    if interval_hours <= 0 {
        return Err(ApiError::BadRequest("interval_hours must be > 0".into()));
    }

    let do_translate = body.do_translate.unwrap_or(false);

    // Parse start_at as timestamp
    let start_dt = DateTime::parse_from_rfc3339(&start_at)
        .map_err(|_| ApiError::BadRequest("invalid start_at format".into()))?
        .with_timezone(&Utc);

    let next_run_at = start_dt.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let task_id = push_task_repo::create_push_task(
        &state.pool,
        &tenant_id,
        merchant_id,
        &task_name,
        &content,
        None, // image_file_id
        do_translate,
        &target_chat_ids,
        &start_at,
        interval_hours,
        "running",
        &next_run_at,
    )
    .await
    .map_err(ApiError::Anyhow)?;

    Ok(Json(json!({ "status": "ok", "task_id": task_id })))
}

/// POST /api/push_tasks/status - Update push task status
pub async fn update_push_task_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdatePushTaskStatusRequest>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    let status = body.status.as_str();
    if status != "paused" && status != "running" {
        return Err(ApiError::BadRequest(
            "status must be 'paused' or 'running'".into(),
        ));
    }

    let ok = push_task_repo::update_push_task_status(&state.pool, body.task_id, status)
        .await
        .map_err(ApiError::Anyhow)?;

    if !ok {
        return Err(ApiError::NotFound("task not found".into()));
    }

    Ok(Json(json!({ "status": "ok" })))
}

/// POST /api/push_tasks/run_now - Run push task immediately
pub async fn run_push_task_now(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunPushTaskNowRequest>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    // Get the task
    let task = push_task_repo::get_push_task(&state.pool, body.task_id)
        .await
        .map_err(ApiError::Anyhow)?
        .ok_or_else(|| ApiError::NotFound("task not found".into()))?;

    // Execute the task
    use tgbot_services::broadcast_service;

    let content: String = task
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let do_translate: bool = task
        .get("do_translate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let target_chat_ids: Vec<i64> = task
        .get("target_chat_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_default();

    let summary = broadcast_service::send_broadcast_to_chats(
        &state.pool,
        &target_chat_ids,
        &content,
        do_translate,
    )
    .await
    .map_err(ApiError::Anyhow)?;

    // Calculate next run time
    let interval_hours: i32 = task
        .get("interval_hours")
        .and_then(|v| v.as_i64())
        .unwrap_or(6) as i32;

    let now = Utc::now();
    let next_run = now + Duration::hours(interval_hours as i64);
    let next_run_at = next_run.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let last_run_at = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let retry_count: i32 = task
        .get("retry_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    push_task_repo::update_push_task_run_result(
        &state.pool,
        body.task_id,
        &last_run_at,
        &next_run_at,
        &summary,
        retry_count,
    )
    .await
    .map_err(ApiError::Anyhow)?;

    Ok(Json(json!({
        "status": "ok",
        "summary": summary,
        "next_run_at": next_run_at
    })))
}