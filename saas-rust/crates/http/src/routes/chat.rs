//! Chat / Messages API — migrated from backend/app/api/chat.py
//!
//! Endpoints:
//!   GET  /api/users                     — inbox user list
//!   GET  /api/messages/{user_id}        — chat history
//!   POST /api/messages/{user_id}        — send message (text or image)
//!   POST /api/messages/{user_id}/delete — delete single message
//!   POST /api/messages/{user_id}/ai     — AI reply compatibility alias
//!   POST /api/mark_read/{user_id}       — mark messages as read
//!   POST /api/delete_chat/{user_id}     — soft-delete a chat
//!   GET  /api/search                    — global message search
//!   GET  /api/file/{file_id}            — file proxy via Telegram
//!   POST /api/upload/{user_id}          — upload image and send
//!   GET  /ws                            — WebSocket for real-time updates

use axum::{
    extract::ws::{Message, WebSocket},
    extract::{FromRequest, Multipart, Path, Query, Request, State, WebSocketUpgrade},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_integrations::telegram::{
    GetFileParams, SendMessageParams, SendPhotoParams, TelegramClient, TelegramConfig,
};
use tgbot_repos::{audit_repo, message_repo, user_repo};

use crate::auth::resolve_scope;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Query / body structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
    #[serde(alias = "tenantId")]
    pub tenant_id: Option<String>,
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

struct UploadedImage {
    bytes: Vec<u8>,
    filename: String,
    mime_type: String,
    content: String,
    reply_to_msg_id: Option<i64>,
}

enum OutgoingMessage {
    Json(SendMessageRequest),
    Image(UploadedImage),
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
        user_repo::get_inbox_list_for_tenant(state.db()?, tenant_id).await?
    } else {
        user_repo::get_inbox_list(state.db()?).await?
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
        let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, tenant_id)
            .await?
            .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;

        let belongs =
            user_repo::is_user_belong_to_tenant(state.db()?, user_id, tenant_id, bot_id).await?;

        if !belongs {
            // Audit the cross-tenant access attempt
            let _ = audit_repo::log_tenant_isolation_event(
                state.db()?,
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

    // Resolve bot_id for the query. Platform administrators may pass ?tenant_id;
    // when absent, fall back to the user's most recent tenant (align Python,
    // which queries unconditionally for platform admins).
    let (bot_id, history_tenant_id) = if scope.role == Role::PlatformAdmin {
        match q
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|tenant_id| !tenant_id.is_empty())
        {
            Some(tenant_id) => {
                let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, tenant_id)
                    .await?
                    .ok_or_else(|| ApiError::BadRequest("tenant has no bot configured".into()))?;
                (Some(bot_id), Some(tenant_id.to_owned()))
            }
            None => {
                let tenant = resolve_user_tenant(state.db()?, user_id).await?.ok_or_else(|| {
                    ApiError::BadRequest("unable to resolve tenant for chat history".into())
                })?;
                let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, &tenant)
                    .await?
                    .ok_or_else(|| ApiError::BadRequest("tenant has no bot configured".into()))?;
                (Some(bot_id), Some(tenant))
            }
        }
    } else {
        (
            resolve_bot_id_for_scope(&state, &scope).await?,
            scope.tenant_id.clone(),
        )
    };

    let messages = if let Some(bid) = bot_id {
        user_repo::get_chat_history(
            state.db()?,
            user_id,
            bid,
            history_tenant_id.as_deref(),
            limit,
            offset,
        )
        .await?
    } else {
        return Err(ApiError::BadRequest(
            "chat history requires a tenant bot context".into(),
        ));
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
    Path(user_id): Path<i64>,
    request: Request,
) -> ApiResult<Json<Value>> {
    let headers = request.headers().clone();
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    // 对齐 Python：负 user_id（虚拟助手会话）拒绝
    if user_id < 0 {
        return Err(ApiError::BadRequest(
            "虚拟助手会话请使用 /api/ai/chat".into(),
        ));
    }
    // 平台 admin（scope.tenant_id 为空）时：优先用请求参数 ?tenant_id，
    // 否则从目标用户最近消息反查租户（对齐 Python api_send_message 平台管理员可全局发送）。
    let tenant_id = match scope.tenant_id.as_deref() {
        Some(tenant) => tenant.to_string(),
        None => {
            let uri = request.uri();
            let query_tenant = uri
                .query()
                .and_then(|q| {
                    q.split('&').find_map(|pair| {
                        let mut parts = pair.splitn(2, '=');
                        if parts.next() == Some("tenant_id") {
                            parts.next().map(|v| v.to_string())
                        } else {
                            None
                        }
                    })
                });
            match query_tenant {
                Some(tenant) => tenant,
                None => resolve_user_tenant(state.db()?, user_id)
                    .await?
                    .ok_or(ApiError::Forbidden)?,
            }
        }
    };
    let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, &tenant_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
    ensure_tenant_user(&state, user_id, &tenant_id, bot_id).await?;
    let telegram = telegram_for_tenant_bot(&state, &tenant_id, bot_id).await?;
    let outgoing = parse_outgoing_message(request, &state).await?;

    // 双向翻译结果（管理员回复 → 用户语言），在 match 外写回数据库
    let mut translated_text: Option<String> = None;
    let (sent, content, msg_type, file_id, reply_to_msg_id) = match outgoing {
        OutgoingMessage::Json(body) => {
            let content = body.content.unwrap_or_default().trim().to_string();
            let file_id = body.file_id.unwrap_or_default().trim().to_string();
            if content.is_empty() && file_id.is_empty() {
                return Err(ApiError::BadRequest("消息内容不能为空".into()));
            }
            if file_id.len() > 512 {
                return Err(ApiError::BadRequest("file_id is too long".into()));
            }
            let content_limit = if file_id.is_empty() { 4096 } else { 1024 };
            if content.chars().count() > content_limit {
                return Err(ApiError::BadRequest(format!(
                    "message exceeds {content_limit} characters"
                )));
            }
            // 双向翻译：管理员回复 → 用户语言（取用户最近一条消息检测语言）。
            // 译文发给用户，数据库保留原文 + 译文（对齐 Python translate_to_customer）。
            let mut send_text = content.clone();
            if file_id.is_empty() && !content.is_empty() {
                if let Ok(Some(customer_msg)) = last_user_message(state.db()?, user_id, &tenant_id)
                    .await
                {
                    if let Some(translated) = crate::translate::translate_to_customer(
                        &state.llm,
                        state.db()?,
                        &content,
                        &customer_msg,
                        user_id,
                        &tenant_id,
                    )
                    .await
                    {
                        send_text = translated.clone();
                        translated_text = Some(translated);
                    }
                }
            }
            let sent = if file_id.is_empty() {
                telegram
                    .send_message(SendMessageParams {
                        chat_id: user_id,
                        text: send_text.clone(),
                        parse_mode: None,
                        reply_markup: None,
                        reply_to_message_id: body.reply_to_msg_id,
                    })
                    .await
            } else {
                telegram
                    .send_photo(SendPhotoParams {
                        chat_id: user_id,
                        photo: file_id.clone(),
                        caption: (!content.is_empty()).then(|| content.clone()),
                        parse_mode: None,
                        reply_to_message_id: body.reply_to_msg_id,
                        reply_markup: None,
                    })
                    .await
            }
            .map_err(ApiError::Anyhow)?;
            let returned_file_id = sent
                .photo
                .as_ref()
                .and_then(|photos| photos.last())
                .map(|photo| photo.file_id.clone())
                .or((!file_id.is_empty()).then_some(file_id));
            let msg_type = if returned_file_id.is_some() {
                "photo"
            } else {
                "text"
            };
            (
                sent,
                content,
                msg_type,
                returned_file_id,
                body.reply_to_msg_id,
            )
        }
        OutgoingMessage::Image(image) => {
            let sent = telegram
                .send_photo_bytes(
                    user_id,
                    image.bytes,
                    &image.filename,
                    &image.mime_type,
                    Some(&image.content),
                    image.reply_to_msg_id,
                    None,
                )
                .await
                .map_err(ApiError::Anyhow)?;
            let file_id = sent
                .photo
                .as_ref()
                .and_then(|photos| photos.last())
                .map(|photo| photo.file_id.clone())
                .ok_or_else(|| ApiError::Anyhow(anyhow::anyhow!("Telegram returned no photo")))?;
            (
                sent,
                image.content,
                "photo",
                Some(file_id),
                image.reply_to_msg_id,
            )
        }
    };
    let display_content = if content.is_empty() && msg_type == "photo" {
        "[图片]"
    } else {
        &content
    };

    let saved_msg_id = message_repo::save_message(
        state.db()?,
        bot_id,
        user_id,
        Some(display_content),
        msg_type,
        file_id.as_deref(),
        None,
        true,
        Some(sent.message_id),
        &tenant_id,
        reply_to_msg_id,
    )
    .await?;

    // 双向翻译：译文写回 translated_text（对齐 Python 管理端「原文 + 译文」展示）
    if let Some(translated) = translated_text {
        if !content.is_empty() && translated != content {
            sqlx::query(
                "UPDATE chat.messages SET translated_text = $2, original_text = $3 \
                 WHERE id = $1 AND tenant_id = $4",
            )
            .bind(saved_msg_id)
            .bind(&translated)
            .bind(&content)
            .bind(tenant_id)
            .execute(state.db()?)
            .await
            .map_err(|_| ApiError::Anyhow(anyhow::anyhow!("failed to persist translation")))?;
        }
    }

    tracing::info!("send_message: saved msg_id={saved_msg_id} to user {user_id}");

    Ok(Json(json!({
        "status": "ok",
        "msg_id": saved_msg_id,
        "data": {
            "msg_id": saved_msg_id,
            "tg_msg_id": sent.message_id,
            "file_id": file_id,
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
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, tenant_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
    ensure_tenant_user(&state, user_id, tenant_id, bot_id).await?;

    if body.msg_id <= 0 {
        return Err(ApiError::BadRequest("消息 ID 无效".into()));
    }

    let success =
        message_repo::delete_single_message(state.db()?, tenant_id, bot_id, user_id, body.msg_id)
            .await?;

    if success {
        Ok(Json(json!({"status": "ok"})))
    } else {
        Err(ApiError::BadRequest("删除失败".into()))
    }
}

// ---------------------------------------------------------------------------
// POST /api/messages/{user_id}/ai
// ---------------------------------------------------------------------------

pub async fn ai_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(mut body): Json<super::ai::AiChatRequest>,
) -> ApiResult<Json<Value>> {
    body.user_id = Some(user_id);
    super::ai::ai_chat(State(state), headers, Json(body)).await
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
    require_write_access(&scope)?;

    // 平台 admin（tenant 为空）时反查用户租户 + 租户 bot（对齐 send_message fallback）
    let (tenant_id, bid) = match scope.tenant_id.as_deref() {
        Some(tenant) => {
            let bid = resolve_bot_id_for_scope(&state, &scope)
                .await?
                .ok_or(ApiError::Forbidden)?;
            (tenant.to_string(), bid)
        }
        None => {
            let tenant = resolve_user_tenant(state.db()?, user_id)
                .await?
                .ok_or(ApiError::Forbidden)?;
            let bid = user_repo::get_bot_id_for_tenant(state.db()?, &tenant)
                .await?
                .ok_or(ApiError::Forbidden)?;
            (tenant, bid)
        }
    };
    ensure_tenant_user(&state, user_id, &tenant_id, bid).await?;
    user_repo::mark_messages_as_read(state.db()?, user_id, bid, &tenant_id).await?;

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
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;
    let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, tenant_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
    ensure_tenant_user(&state, user_id, tenant_id, bot_id).await?;
    user_repo::soft_delete_chat(state.db()?, user_id, tenant_id).await?;

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
        state.db()?,
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
// GET /api/file/{file_id} — file proxy via Telegram
// ---------------------------------------------------------------------------

pub async fn get_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    if file_id.is_empty() || file_id.len() > 512 {
        return Err(ApiError::BadRequest("invalid file_id".into()));
    }
    // 平台管理员会话无 tenant_id：回退到默认租户（生产主 bot）
    let tenant_id = scope.tenant_id.as_deref().unwrap_or("default");
    let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, tenant_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
    let telegram = telegram_for_tenant_bot(&state, tenant_id, bot_id).await?;
    let file = telegram
        .get_file(GetFileParams {
            file_id: file_id.clone(),
        })
        .await
        .map_err(ApiError::Anyhow)?;
    let bytes = telegram
        .download_file_bytes(&file, 10 * 1024 * 1024)
        .await
        .map_err(ApiError::Anyhow)?;
    let mime_type = mime_type_for_path(file.file_path.as_deref());
    let url = format!("data:{mime_type};base64,{}", BASE64.encode(bytes));

    Ok(Json(json!({
        "status": "ok",
        "url": url.clone(),
        "data": { "url": url, "mime_type": mime_type }
    })))
}

// ---------------------------------------------------------------------------
// GET /ws — WebSocket endpoint for real-time admin updates
// ---------------------------------------------------------------------------

pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_socket(socket))
}

async fn handle_ws_socket(mut socket: WebSocket) {
    // Simple ping/pong read loop
    loop {
        match socket.recv().await {
            Some(Ok(Message::Text(text))) => {
                if text == "ping" {
                    if socket.send(Message::Text("pong".into())).await.is_err() {
                        break;
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => break,
            _ => {}
        }
    }
    tracing::info!("WebSocket client disconnected");
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
    let bot_id = user_repo::get_bot_id_for_tenant(state.db()?, tenant_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
    Ok(Some(bot_id))
}

fn require_write_access(scope: &tgbot_domain::scope::Scope) -> ApiResult<()> {
    // 对齐 Python：平台管理员也可通过 Web 端向任意租户用户发送消息
    if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::PlatformAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// 平台管理员发送时反查目标用户所属租户（最近一条消息的 tenant_id）。
async fn resolve_user_tenant(pool: &sqlx::PgPool, user_id: i64) -> Result<Option<String>, sqlx::Error> {
    let tenant: Option<String> = sqlx::query_scalar(
        "SELECT tenant_id FROM chat.messages \
         WHERE (from_userid = $1 OR to_userid = $1) AND tenant_id IS NOT NULL \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(tenant)
}

async fn ensure_tenant_user(
    state: &AppState,
    user_id: i64,
    tenant_id: &str,
    bot_id: i64,
) -> ApiResult<()> {
    if user_repo::is_user_belong_to_tenant(state.db()?, user_id, tenant_id, bot_id).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// 取用户最近一条文本消息（用于双向翻译的语言检测参考）。
async fn last_user_message(
    pool: &sqlx::PgPool,
    user_id: i64,
    tenant_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let content: Option<String> = sqlx::query_scalar(
        "SELECT content FROM chat.messages \
         WHERE from_userid = $1 AND tenant_id = $2 AND is_deleted = false \
           AND NULLIF(btrim(content), '') IS NOT NULL \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(content)
}

async fn telegram_for_tenant_bot(
    state: &AppState,
    tenant_id: &str,
    bot_id: i64,
) -> ApiResult<TelegramClient> {
    let entry = state
        .bot_registry
        .get(bot_id)
        .await
        .filter(|entry| entry.tenant_id == tenant_id)
        .ok_or_else(|| ApiError::BadRequest("merchant bot is not active".into()))?;
    TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token,
        api_base_url: std::env::var("TELEGRAM_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.telegram.org".into()),
        timeout_secs: 30,
    })
    .map_err(ApiError::Anyhow)
}

async fn parse_outgoing_message(request: Request, state: &AppState) -> ApiResult<OutgoingMessage> {
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
        let Json(body) = Json::<SendMessageRequest>::from_request(request, state)
            .await
            .map_err(|error| ApiError::BadRequest(format!("invalid JSON body: {error}")))?;
        return Ok(OutgoingMessage::Json(body));
    }

    let multipart = Multipart::from_request(request, state)
        .await
        .map_err(|error| ApiError::BadRequest(format!("invalid multipart body: {error}")))?;
    parse_uploaded_image(multipart)
        .await
        .map(OutgoingMessage::Image)
}

async fn parse_uploaded_image(mut multipart: Multipart) -> ApiResult<UploadedImage> {
    let mut image = None;
    let mut content = String::new();
    let mut reply_to_msg_id = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(format!("invalid multipart field: {error}")))?
    {
        match field.name().unwrap_or_default() {
            "image" => {
                let filename = field.file_name().unwrap_or("image.jpg").to_string();
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
                if bytes.is_empty() || bytes.len() > 10 * 1024 * 1024 {
                    return Err(ApiError::BadRequest(
                        "image must be between 1 byte and 10 MiB".into(),
                    ));
                }
                image = Some((bytes.to_vec(), filename, mime_type));
            }
            "content" => {
                content = field
                    .text()
                    .await
                    .map_err(|error| ApiError::BadRequest(format!("invalid caption: {error}")))?
                    .trim()
                    .to_string();
                if content.chars().count() > 1024 {
                    return Err(ApiError::BadRequest(
                        "image caption exceeds 1024 characters".into(),
                    ));
                }
            }
            "reply_to_msg_id" => {
                let value = field
                    .text()
                    .await
                    .map_err(|error| ApiError::BadRequest(format!("invalid reply id: {error}")))?;
                reply_to_msg_id = Some(
                    value
                        .trim()
                        .parse::<i64>()
                        .map_err(|_| ApiError::BadRequest("invalid reply_to_msg_id".into()))?,
                );
            }
            _ => {}
        }
    }
    let (bytes, filename, mime_type) =
        image.ok_or_else(|| ApiError::BadRequest("未找到图片文件".into()))?;
    Ok(UploadedImage {
        bytes,
        filename,
        mime_type,
        content,
        reply_to_msg_id,
    })
}

fn mime_type_for_path(path: Option<&str>) -> &'static str {
    match path
        .and_then(|value| value.rsplit_once('.').map(|(_, extension)| extension))
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "ogg" | "oga" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}
