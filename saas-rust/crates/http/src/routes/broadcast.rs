use std::collections::HashSet;

use axum::{
    extract::{FromRequest, Multipart, Query, Request, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_domain::models::{
    BroadcastChatsRequest, BroadcastSendRequest, BroadcastTargetsQuery, CreatePushTaskRequest,
    PushTasksQuery, RunPushTaskNowRequest, UpdatePushTaskStatusRequest,
};
use tgbot_integrations::telegram::{
    Message as TelegramMessage, SendMessageParams, SendPhotoParams, TelegramClient, TelegramConfig,
};
use tgbot_repos::{broadcast_repo, message_repo, push_task_repo, user_repo};

use crate::auth::resolve_scope;
use crate::state::AppState;

const MAX_BROADCAST_TARGETS: usize = 500;
const MAX_BROADCAST_IMAGE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone)]
enum BroadcastMedia {
    Upload {
        bytes: Vec<u8>,
        filename: String,
        mime_type: String,
    },
    TelegramFile(String),
}

struct BroadcastPayload {
    content: String,
    do_translate: bool,
    user_ids: Vec<i64>,
    chat_ids: Vec<i64>,
    buttons: Option<Value>,
    retry_count: u8,
    media: Option<BroadcastMedia>,
}

pub async fn list_broadcast_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BroadcastTargetsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    // 平台 admin 允许：回退到默认租户（对齐 Web 端其他端点的 tenant 回退策略）
    let tenant_id = if scope.role == Role::PlatformAdmin {
        scope.tenant_id.clone().unwrap_or_else(|| "default".into())
    } else {
        let (tid, _) = scope.require_merchant()?;
        tid
    };
    let chat_types = query.chat_types.as_ref().map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let targets = broadcast_repo::list_broadcast_targets(
        state.db()?,
        &tenant_id,
        chat_types.as_deref(),
        query.enabled_only.unwrap_or(true),
    )
    .await?;
    Ok(Json(json!({
        "status": "ok",
        "total": targets.len(),
        "data": targets,
    })))
}

pub async fn sync_broadcast_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let tenant_id = if scope.role == Role::PlatformAdmin {
        scope.tenant_id.clone().unwrap_or_else(|| "default".into())
    } else {
        let (tid, _) = scope.require_merchant()?;
        tid
    };
    let result = sqlx::query(
        r#"
        INSERT INTO broadcast.broadcast_targets
            (chat_id, chat_title, chat_type, can_broadcast, tenant_id, updated_at)
        SELECT group_id, COALESCE(group_title, ''), 'supergroup', true, tenant_id, now()
        FROM groups.tracked_groups
        WHERE tenant_id = $1
        ON CONFLICT (chat_id, tenant_id) DO UPDATE SET
            chat_title = EXCLUDED.chat_title,
            chat_type = EXCLUDED.chat_type,
            updated_at = now()
        "#,
    )
    .bind(&tenant_id)
    .execute(state.db()?)
    .await?;
    let targets =
        broadcast_repo::list_broadcast_targets(state.db()?, &tenant_id, None, true).await?;
    Ok(Json(json!({
        "status": "ok",
        "summary": { "synced": result.rows_affected(), "total": targets.len() },
        "data": targets,
    })))
}

pub async fn send_broadcast(
    State(state): State<AppState>,
    request: Request,
) -> ApiResult<Json<Value>> {
    let headers = request.headers().clone();
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let (tenant_id, _) = scope.require_merchant()?;
    let payload = parse_broadcast_request(request, &state).await?;
    let summary = execute_broadcast(&state, &tenant_id, payload).await?;
    let failed_items = summary["failed_items"].clone();
    Ok(Json(json!({
        "status": "ok",
        "summary": summary,
        "failed_items": failed_items,
    })))
}

pub async fn broadcast_chats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BroadcastChatsRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let (tenant_id, _) = scope.require_merchant()?;
    let payload = normalize_payload(
        body.content.unwrap_or_default(),
        body.do_translate.unwrap_or(false),
        Vec::new(),
        body.chat_ids.unwrap_or_default(),
        None,
        body.retry_count.unwrap_or(1),
        None,
    )?;
    let summary = execute_broadcast(&state, &tenant_id, payload).await?;
    let failed_items = summary["failed_items"].clone();
    Ok(Json(json!({
        "status": "ok",
        "summary": summary,
        "failed_items": failed_items,
    })))
}

pub async fn list_push_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PushTasksQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _) = scope.require_merchant()?;
    let tasks = push_task_repo::list_push_tasks(
        state.db()?,
        Some(&tenant_id),
        query.status.as_deref(),
        query.limit.unwrap_or(50).clamp(1, 200),
        query.offset.unwrap_or(0).max(0),
    )
    .await?;
    Ok(Json(
        json!({ "status": "ok", "total": tasks.len(), "data": tasks }),
    ))
}

pub async fn create_push_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreatePushTaskRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let content = body.content.unwrap_or_default().trim().to_string();
    if content.is_empty() || content.chars().count() > 4096 {
        return Err(ApiError::BadRequest(
            "content must contain 1 to 4096 characters".into(),
        ));
    }
    let target_chat_ids = dedupe_ids(body.target_chat_ids.unwrap_or_default())?;
    if target_chat_ids.is_empty() {
        return Err(ApiError::BadRequest("target_chat_ids required".into()));
    }
    ensure_allowed_chats(state.db()?, &tenant_id, &target_chat_ids).await?;
    let start_at = body.start_at.unwrap_or_default();
    let start = DateTime::parse_from_rfc3339(start_at.trim())
        .map_err(|_| ApiError::BadRequest("invalid start_at format".into()))?
        .with_timezone(&Utc);
    let interval_hours = body.interval_hours.unwrap_or(6);
    if !(1..=24 * 365).contains(&interval_hours) {
        return Err(ApiError::BadRequest(
            "interval_hours must be between 1 and 8760".into(),
        ));
    }
    let task_name = body
        .task_name
        .unwrap_or_else(|| format!("Task-{}", Utc::now().timestamp()))
        .trim()
        .to_string();
    if task_name.is_empty() || task_name.chars().count() > 200 {
        return Err(ApiError::BadRequest("invalid task_name".into()));
    }
    let start_at = start.to_rfc3339();
    let task_id = push_task_repo::create_push_task(
        state.db()?,
        &tenant_id,
        Some(merchant_id),
        &task_name,
        &content,
        None,
        body.do_translate.unwrap_or(false),
        &target_chat_ids,
        &start_at,
        interval_hours,
        "running",
        &start_at,
    )
    .await?;
    Ok(Json(json!({ "status": "ok", "task_id": task_id })))
}

pub async fn update_push_task_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdatePushTaskStatusRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let (tenant_id, _) = scope.require_merchant()?;
    if !matches!(body.status.as_str(), "paused" | "running") {
        return Err(ApiError::BadRequest(
            "status must be 'paused' or 'running'".into(),
        ));
    }
    if !push_task_repo::update_push_task_status(state.db()?, &tenant_id, body.task_id, &body.status)
        .await?
    {
        return Err(ApiError::NotFound("task not found".into()));
    }
    Ok(Json(json!({ "status": "ok" })))
}

pub async fn run_push_task_now(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunPushTaskNowRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let (tenant_id, _) = scope.require_merchant()?;
    let task = push_task_repo::get_push_task(state.db()?, &tenant_id, body.task_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("task not found".into()))?;
    let media = task
        .get("image_file_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|value| BroadcastMedia::TelegramFile(value.to_string()));
    let payload = normalize_payload(
        task.get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        task.get("do_translate")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        Vec::new(),
        json_i64_array(task.get("target_chat_ids")),
        None,
        1,
        media,
    )?;
    let summary = execute_broadcast(&state, &tenant_id, payload).await?;
    let interval_hours = task
        .get("interval_hours")
        .and_then(Value::as_i64)
        .unwrap_or(6)
        .clamp(1, 8760);
    let now = Utc::now();
    let next_run = now + Duration::hours(interval_hours);
    let updated = push_task_repo::update_push_task_run_result(
        state.db()?,
        &tenant_id,
        body.task_id,
        &now.to_rfc3339(),
        &next_run.to_rfc3339(),
        &summary,
        0,
    )
    .await?;
    if !updated {
        return Err(ApiError::NotFound("task not found".into()));
    }
    Ok(Json(json!({
        "status": "ok",
        "summary": summary,
        "next_run_at": next_run.to_rfc3339(),
    })))
}

async fn parse_broadcast_request(
    request: Request,
    state: &AppState,
) -> ApiResult<BroadcastPayload> {
    let is_multipart = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .to_ascii_lowercase()
                .starts_with("multipart/form-data")
        });
    if !is_multipart {
        let Json(body) = Json::<BroadcastSendRequest>::from_request(request, state)
            .await
            .map_err(|error| ApiError::BadRequest(format!("invalid JSON body: {error}")))?;
        return normalize_payload(
            body.content.unwrap_or_default(),
            body.do_translate.unwrap_or(false),
            body.user_ids.unwrap_or_default(),
            body.chat_ids.unwrap_or_default(),
            body.buttons,
            body.retry_count.unwrap_or(1),
            None,
        );
    }
    let multipart = Multipart::from_request(request, state)
        .await
        .map_err(|error| ApiError::BadRequest(format!("invalid multipart body: {error}")))?;
    parse_broadcast_multipart(multipart).await
}

async fn parse_broadcast_multipart(mut multipart: Multipart) -> ApiResult<BroadcastPayload> {
    let mut content = String::new();
    let mut do_translate = false;
    let mut user_ids = Vec::new();
    let mut chat_ids = Vec::new();
    let mut buttons = None;
    let mut retry_count = 1;
    let mut media = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(format!("invalid multipart field: {error}")))?
    {
        let field_name = field.name().unwrap_or_default().to_string();
        match field_name.as_str() {
            "image" => {
                let filename = field.file_name().unwrap_or("broadcast.jpg").to_string();
                let mime_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                if !mime_type.starts_with("image/") {
                    return Err(ApiError::BadRequest(
                        "only image uploads are supported".into(),
                    ));
                }
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|error| ApiError::BadRequest(format!("invalid image: {error}")))?;
                if bytes.is_empty() || bytes.len() > MAX_BROADCAST_IMAGE_BYTES {
                    return Err(ApiError::BadRequest(
                        "image must be between 1 byte and 10 MiB".into(),
                    ));
                }
                media = Some(BroadcastMedia::Upload {
                    bytes: bytes.to_vec(),
                    filename,
                    mime_type,
                });
            }
            name => {
                let value = field
                    .text()
                    .await
                    .map_err(|error| ApiError::BadRequest(format!("invalid {name}: {error}")))?;
                match name {
                    "content" => content = value,
                    "do_translate" => do_translate = parse_truthy(&value),
                    "user_ids" => user_ids = parse_i64_array(&value, "user_ids")?,
                    "chat_ids" => chat_ids = parse_i64_array(&value, "chat_ids")?,
                    "buttons" => {
                        buttons = Some(serde_json::from_str(&value).map_err(|_| {
                            ApiError::BadRequest("buttons must be a JSON array".into())
                        })?)
                    }
                    "retry_count" => {
                        retry_count = value
                            .trim()
                            .parse::<i32>()
                            .map_err(|_| ApiError::BadRequest("invalid retry_count".into()))?
                    }
                    _ => {}
                }
            }
        }
    }
    normalize_payload(
        content,
        do_translate,
        user_ids,
        chat_ids,
        buttons,
        retry_count,
        media,
    )
}

fn normalize_payload(
    content: String,
    do_translate: bool,
    user_ids: Vec<i64>,
    chat_ids: Vec<i64>,
    buttons: Option<Value>,
    retry_count: i32,
    media: Option<BroadcastMedia>,
) -> ApiResult<BroadcastPayload> {
    let content = content.trim().to_string();
    let content_limit = if media.is_some() { 1024 } else { 4096 };
    if content.is_empty() && media.is_none() {
        return Err(ApiError::BadRequest("content or image is required".into()));
    }
    if content.chars().count() > content_limit {
        return Err(ApiError::BadRequest(format!(
            "content exceeds {content_limit} characters"
        )));
    }
    let user_ids = dedupe_ids(user_ids)?;
    let chat_ids = dedupe_ids(chat_ids)?;
    if user_ids.is_empty() && chat_ids.is_empty() {
        return Err(ApiError::BadRequest("user_ids or chat_ids required".into()));
    }
    if user_ids.len() + chat_ids.len() > MAX_BROADCAST_TARGETS {
        return Err(ApiError::BadRequest(format!(
            "broadcast supports at most {MAX_BROADCAST_TARGETS} targets"
        )));
    }
    Ok(BroadcastPayload {
        content,
        do_translate,
        user_ids,
        chat_ids,
        buttons: build_reply_markup(buttons)?,
        retry_count: retry_count.clamp(0, 3) as u8,
        media,
    })
}

async fn execute_broadcast(
    state: &AppState,
    tenant_id: &str,
    mut payload: BroadcastPayload,
) -> ApiResult<Value> {
    let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, tenant_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
    let entry = state
        .bot_registry
        .get(bot_id)
        .await
        .filter(|entry| entry.tenant_id == tenant_id)
        .ok_or_else(|| ApiError::BadRequest("merchant bot is not active".into()))?;
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token,
        api_base_url: std::env::var("TELEGRAM_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.telegram.org".into()),
        timeout_secs: 30,
    })
    .map_err(ApiError::Anyhow)?;

    if payload.do_translate && !payload.content.is_empty() {
        payload.content = state
            .llm
            .simple_chat(
                "Translate the message into English when it is not English, otherwise into Simplified Chinese. Preserve Telegram HTML tags and return only the translation.",
                &payload.content,
            )
            .await
            .map_err(ApiError::Anyhow)?;
    }

    let allowed_users = allowed_user_ids(state.db()?, tenant_id, bot_id, &payload.user_ids).await?;
    let allowed_chats = allowed_chat_ids(state.db()?, tenant_id, &payload.chat_ids).await?;
    let total = payload.user_ids.len() + payload.chat_ids.len();
    let mut success_user_ids = Vec::new();
    let mut success_chat_ids = Vec::new();
    let mut failed_items = Vec::new();

    for user_id in &payload.user_ids {
        if !allowed_users.contains(user_id) {
            failed_items.push(json!({
                "user_id": user_id,
                "reason": "target is not available for this tenant",
            }));
            continue;
        }
        match deliver_with_retry(&telegram, *user_id, &payload).await {
            Ok(sent) => {
                let file_id = sent
                    .photo
                    .as_ref()
                    .and_then(|photos| photos.last())
                    .map(|photo| photo.file_id.as_str());
                if let Err(error) = message_repo::save_message(
                    state.db()?,
                    bot_id,
                    *user_id,
                    Some(if payload.content.is_empty() {
                        "[图片]"
                    } else {
                        &payload.content
                    }),
                    if payload.media.is_some() {
                        "photo"
                    } else {
                        "text"
                    },
                    file_id,
                    None,
                    true,
                    Some(sent.message_id),
                    tenant_id,
                    None,
                )
                .await
                {
                    tracing::error!(user_id, error = %error, "broadcast delivered but persistence failed");
                }
                success_user_ids.push(*user_id);
            }
            Err(error) => {
                tracing::warn!(user_id, error = %error, "broadcast user delivery failed");
                failed_items
                    .push(json!({ "user_id": user_id, "reason": "Telegram delivery failed" }));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(35)).await;
    }

    for chat_id in &payload.chat_ids {
        if !allowed_chats.contains(chat_id) {
            failed_items.push(json!({
                "chat_id": chat_id,
                "reason": "target is not available for this tenant",
            }));
            continue;
        }
        match deliver_with_retry(&telegram, *chat_id, &payload).await {
            Ok(_) => success_chat_ids.push(*chat_id),
            Err(error) => {
                tracing::warn!(chat_id, error = %error, "broadcast chat delivery failed");
                failed_items
                    .push(json!({ "chat_id": chat_id, "reason": "Telegram delivery failed" }));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(35)).await;
    }

    Ok(json!({
        "total": total,
        "success": success_user_ids.len() + success_chat_ids.len(),
        "failed": failed_items.len(),
        "success_user_ids": success_user_ids,
        "success_chat_ids": success_chat_ids,
        "failed_items": failed_items,
        "translated": payload.do_translate,
    }))
}

async fn deliver_with_retry(
    telegram: &TelegramClient,
    chat_id: i64,
    payload: &BroadcastPayload,
) -> anyhow::Result<TelegramMessage> {
    let mut last_error = None;
    for attempt in 0..=payload.retry_count {
        let result = match &payload.media {
            Some(BroadcastMedia::Upload {
                bytes,
                filename,
                mime_type,
            }) => {
                telegram
                    .send_photo_bytes(
                        chat_id,
                        bytes.clone(),
                        filename,
                        mime_type,
                        Some(&payload.content),
                        None,
                        payload.buttons.as_ref(),
                    )
                    .await
            }
            Some(BroadcastMedia::TelegramFile(file_id)) => {
                telegram
                    .send_photo(SendPhotoParams {
                        chat_id,
                        photo: file_id.clone(),
                        caption: (!payload.content.is_empty()).then(|| payload.content.clone()),
                        parse_mode: Some("HTML".into()),
                        reply_to_message_id: None,
                        reply_markup: payload.buttons.clone(),
                    })
                    .await
            }
            None => {
                telegram
                    .send_message(SendMessageParams {
                        chat_id,
                        text: payload.content.clone(),
                        parse_mode: Some("HTML".into()),
                        reply_markup: payload.buttons.clone(),
                        reply_to_message_id: None,
                    })
                    .await
            }
        };
        match result {
            Ok(message) => return Ok(message),
            Err(error) => last_error = Some(error),
        }
        if attempt < payload.retry_count {
            tokio::time::sleep(std::time::Duration::from_millis(
                100 * 2_u64.pow(attempt as u32),
            ))
            .await;
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Telegram delivery failed")))
}

async fn allowed_user_ids(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    bot_id: i64,
    requested: &[i64],
) -> ApiResult<HashSet<i64>> {
    if requested.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT CASE WHEN from_userid = $2 THEN to_userid ELSE from_userid END AS user_id
        FROM chat.messages
        WHERE tenant_id = $1
          AND (from_userid = $2 OR to_userid = $2)
          AND is_deleted = false
          AND (CASE WHEN from_userid = $2 THEN to_userid ELSE from_userid END) = ANY($3)
        "#,
    )
    .bind(tenant_id)
    .bind(bot_id)
    .bind(requested)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<i64, _>("user_id").ok())
        .collect())
}

async fn allowed_chat_ids(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    requested: &[i64],
) -> ApiResult<HashSet<i64>> {
    if requested.is_empty() {
        return Ok(HashSet::new());
    }
    let ids = sqlx::query_scalar::<_, i64>(
        "SELECT chat_id FROM broadcast.broadcast_targets WHERE tenant_id = $1 AND can_broadcast = true AND chat_id = ANY($2)",
    )
    .bind(tenant_id)
    .bind(requested)
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().collect())
}

async fn ensure_allowed_chats(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    requested: &[i64],
) -> ApiResult<()> {
    let allowed = allowed_chat_ids(pool, tenant_id, requested).await?;
    if allowed.len() == requested.len() {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "one or more target chats are unavailable for this tenant".into(),
        ))
    }
}

fn build_reply_markup(buttons: Option<Value>) -> ApiResult<Option<Value>> {
    let Some(buttons) = buttons else {
        return Ok(None);
    };
    let items = buttons
        .as_array()
        .ok_or_else(|| ApiError::BadRequest("buttons must be an array".into()))?;
    if items.is_empty() {
        return Ok(None);
    }
    if items.len() > 6 {
        return Err(ApiError::BadRequest(
            "at most 6 buttons are supported".into(),
        ));
    }
    let mut rows = Vec::new();
    for chunk in items.chunks(2) {
        let mut row = Vec::new();
        for item in chunk {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let url = item.get("url").and_then(Value::as_str).unwrap_or("").trim();
            if text.is_empty()
                || text.chars().count() > 64
                || !(url.starts_with("https://") || url.starts_with("http://"))
                || url.len() > 2048
            {
                return Err(ApiError::BadRequest("invalid broadcast button".into()));
            }
            row.push(json!({ "text": text, "url": url }));
        }
        rows.push(Value::Array(row));
    }
    Ok(Some(json!({ "inline_keyboard": rows })))
}

fn parse_i64_array(value: &str, name: &str) -> ApiResult<Vec<i64>> {
    serde_json::from_str(value)
        .map_err(|_| ApiError::BadRequest(format!("{name} must be a JSON integer array")))
}

fn json_i64_array(value: Option<&Value>) -> Vec<i64> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn dedupe_ids(ids: Vec<i64>) -> ApiResult<Vec<i64>> {
    if ids.iter().any(|id| *id == 0) {
        return Err(ApiError::BadRequest("target ids must be non-zero".into()));
    }
    let mut seen = HashSet::new();
    Ok(ids.into_iter().filter(|id| seen.insert(*id)).collect())
}

fn parse_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn require_write_access(scope: &tgbot_domain::scope::Scope) -> ApiResult<()> {
    if matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin | Role::PlatformAdmin) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
