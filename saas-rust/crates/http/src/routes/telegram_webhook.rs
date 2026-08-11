// Telegram Bot Webhook integration
// Receives Telegram Updates via webhook and dispatches to per-merchant handlers.

use axum::{
    extract::{Path, State},
    Json,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::auth::resolve_scope;
use crate::routes::auth::{issue_session_token, OtpData, OtpStore};
use crate::routes::miniapp::{GrantData, GrantStore};
use crate::secret_crypto::SecretCipher;
use crate::state::AppState;
use crate::translate;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_integrations::litellm::LiteLLMClient;
use tgbot_integrations::telegram::{
    AnswerCallbackQueryParams, EditMessageTextParams, SendMessageParams, TelegramClient,
    TelegramConfig,
};
use tgbot_repos::{audit_repo, message_repo};
use tgbot_services::ad_text::ad_text_hash;
use tgbot_services::telegram_preferences::{
    build_console_entry_card, build_full_start_menu, console_entry_urls, language_menu_reply,
    merchant_console_by_owner, parse_language_callback, set_telegram_locale, telegram_locale,
    user_console_roles, LanguageCallback,
};
use tgbot_services::telegram_wallet::{
    wallet_admin_list_reply_for_locale, wallet_list_reply_for_locale, wallet_monitor_prompt,
};

// ---------------------------------------------------------------------------
// Update model — mirrors Telegram Bot API Update object
// ---------------------------------------------------------------------------

/// Telegram Update as received via webhook.
/// We keep the full JSON payload so downstream handlers can inspect any field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Value>,
    #[serde(default)]
    pub edited_message: Option<Value>,
    #[serde(default)]
    pub channel_post: Option<Value>,
    #[serde(default)]
    pub edited_channel_post: Option<Value>,
    #[serde(default)]
    pub inline_query: Option<Value>,
    #[serde(default)]
    pub chosen_inline_result: Option<Value>,
    #[serde(default)]
    pub callback_query: Option<Value>,
    #[serde(default)]
    pub shipping_query: Option<Value>,
    #[serde(default)]
    pub pre_checkout_query: Option<Value>,
    #[serde(default)]
    pub poll: Option<Value>,
    #[serde(default)]
    pub poll_answer: Option<Value>,
    #[serde(default)]
    pub my_chat_member: Option<Value>,
    #[serde(default)]
    pub chat_member: Option<Value>,
    #[serde(default)]
    pub chat_join_request: Option<Value>,
    /// Catch-all for fields we haven't explicitly listed.
    #[serde(flatten)]
    pub extra: Value,
}

// ---------------------------------------------------------------------------
// Update kind classification for dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateKind {
    Message,
    EditedMessage,
    ChannelPost,
    EditedChannelPost,
    InlineQuery,
    ChosenInlineResult,
    CallbackQuery,
    ShippingQuery,
    PreCheckoutQuery,
    Poll,
    PollAnswer,
    MyChatMember,
    ChatMember,
    ChatJoinRequest,
    Unknown,
}

impl TelegramUpdate {
    /// Classify this update into a dispatch kind.
    pub fn kind(&self) -> UpdateKind {
        if self.message.is_some() {
            UpdateKind::Message
        } else if self.edited_message.is_some() {
            UpdateKind::EditedMessage
        } else if self.channel_post.is_some() {
            UpdateKind::ChannelPost
        } else if self.edited_channel_post.is_some() {
            UpdateKind::EditedChannelPost
        } else if self.inline_query.is_some() {
            UpdateKind::InlineQuery
        } else if self.chosen_inline_result.is_some() {
            UpdateKind::ChosenInlineResult
        } else if self.callback_query.is_some() {
            UpdateKind::CallbackQuery
        } else if self.shipping_query.is_some() {
            UpdateKind::ShippingQuery
        } else if self.pre_checkout_query.is_some() {
            UpdateKind::PreCheckoutQuery
        } else if self.poll.is_some() {
            UpdateKind::Poll
        } else if self.poll_answer.is_some() {
            UpdateKind::PollAnswer
        } else if self.my_chat_member.is_some() {
            UpdateKind::MyChatMember
        } else if self.chat_member.is_some() {
            UpdateKind::ChatMember
        } else if self.chat_join_request.is_some() {
            UpdateKind::ChatJoinRequest
        } else {
            UpdateKind::Unknown
        }
    }
}

// ---------------------------------------------------------------------------
// Bot registry — maps bot_id (i64) to merchant metadata & token
// ---------------------------------------------------------------------------

/// Runtime entry for a registered bot.
#[derive(Debug, Clone)]
pub struct BotEntry {
    pub bot_id: i64,
    pub bot_token: String,
    pub tenant_id: String,
    pub merchant_id: String,
    pub bot_username: Option<String>,
}

/// Thread-safe registry of active bots, keyed by bot_id.
#[derive(Clone, Default)]
pub struct BotRegistry(Arc<RwLock<HashMap<i64, BotEntry>>>);

impl BotRegistry {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    pub async fn register(&self, entry: BotEntry) {
        let mut map = self.0.write().await;
        info!(bot_id = entry.bot_id, "Bot registered in webhook registry");
        map.insert(entry.bot_id, entry);
    }

    pub async fn unregister(&self, bot_id: i64) {
        let mut map = self.0.write().await;
        map.remove(&bot_id);
        info!(bot_id, "Bot unregistered from webhook registry");
    }

    pub async fn get(&self, bot_id: i64) -> Option<BotEntry> {
        let map = self.0.read().await;
        map.get(&bot_id).cloned()
    }

    pub async fn list_entries(&self) -> Vec<BotEntry> {
        let map = self.0.read().await;
        map.values().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Update dispatcher — routes updates to the appropriate handler
// ---------------------------------------------------------------------------

/// Handler function signature for processing a classified update.
pub type UpdateHandlerFn = Arc<dyn Fn(TelegramUpdate, BotEntry) -> DispatchResult + Send + Sync>;

/// Result of dispatching an update.
pub type DispatchResult =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>;

/// Dispatcher that maps (UpdateKind) -> handler.
/// Handlers are async functions that receive the update + bot context.
#[derive(Clone)]
pub struct UpdateDispatcher {
    handlers: Arc<RwLock<HashMap<UpdateKind, UpdateHandlerFn>>>,
    default_handler: Arc<RwLock<Option<UpdateHandlerFn>>>,
}

impl UpdateDispatcher {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(HashMap::new())),
            default_handler: Arc::new(RwLock::new(None)),
        }
    }

    /// Register a handler for a specific update kind.
    pub async fn on<F, Fut>(&self, kind: UpdateKind, handler: F)
    where
        F: Fn(TelegramUpdate, BotEntry) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let handler_fn: UpdateHandlerFn = Arc::new(move |update, entry| {
            let fut = handler(update, entry);
            Box::pin(fut)
        });
        let mut map = self.handlers.write().await;
        map.insert(kind, handler_fn);
    }

    /// Register a fallback handler for unmatched update kinds.
    pub async fn on_default<F, Fut>(&self, handler: F)
    where
        F: Fn(TelegramUpdate, BotEntry) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let handler_fn: UpdateHandlerFn = Arc::new(move |update, entry| {
            let fut = handler(update, entry);
            Box::pin(fut)
        });
        let mut dh = self.default_handler.write().await;
        *dh = Some(handler_fn);
    }

    /// Dispatch a single update to the matching handler.
    pub async fn dispatch(&self, update: TelegramUpdate, entry: BotEntry) -> Result<(), String> {
        let kind = update.kind();
        let handlers = self.handlers.read().await;

        if let Some(handler) = handlers.get(&kind) {
            debug!(?kind, update_id = update.update_id, "Dispatching update");
            handler(update, entry).await
        } else {
            drop(handlers);
            let dh = self.default_handler.read().await;
            if let Some(handler) = dh.as_ref() {
                debug!(
                    ?kind,
                    update_id = update.update_id,
                    "Dispatching to default handler"
                );
                handler(update, entry).await
            } else {
                warn!(
                    ?kind,
                    update_id = update.update_id,
                    "No handler for update kind"
                );
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Webhook verification — validate secret_token header (Telegram Bot API 6.0+)
// ---------------------------------------------------------------------------

/// Validate the X-Telegram-Bot-Api-Secret-Token header against the bot's configured secret.
/// Returns Ok(()) if valid or no secret configured; Err if mismatch.
pub fn verify_webhook_secret(
    headers: &axum::http::HeaderMap,
    expected_secret: &str,
) -> ApiResult<()> {
    let provided = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if constant_time_equal(provided.as_bytes(), expected_secret.as_bytes()) {
        Ok(())
    } else {
        warn!("Webhook secret token mismatch");
        Err(ApiError::Unauthorized)
    }
}

// ---------------------------------------------------------------------------
// Webhook endpoint handlers
// ---------------------------------------------------------------------------

/// POST /telegram/webhook/:bot_id
///
/// Receives a Telegram Update payload for a specific bot.
/// 1. Looks up the bot in the registry
/// 2. Validates the secret token header (if configured)
/// 3. Parses the Update
/// 4. Dispatches to the per-kind handler
pub async fn webhook_handler(
    State(state): State<AppState>,
    Path(bot_id): Path<i64>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    info!(bot_id, "Received webhook update");

    // 1. Resolve bot entry from registry
    let entry = state.bot_registry.get(bot_id).await.ok_or_else(|| {
        warn!(bot_id, "Webhook received for unknown bot_id");
        ApiError::NotFound(format!("Bot {} not found in registry", bot_id))
    })?;

    // 2. Verify webhook secret (if configured)
    // For multi-tenant bots, the secret is derived from the bot's token hash
    let webhook_secret = derive_webhook_secret(
        &state.config.secret_encryption_key,
        bot_id,
        &entry.bot_token,
    )
    .map_err(|error| ApiError::Anyhow(anyhow::anyhow!(error)))?;
    verify_webhook_secret(&headers, &webhook_secret)?;

    // 3. Parse update
    let update: TelegramUpdate = serde_json::from_value(body).map_err(|e| {
        warn!(bot_id, error = %e, "Failed to parse Telegram update");
        ApiError::BadRequest(format!("Invalid update payload: {}", e))
    })?;

    let kind = update.kind();
    let update_id = update.update_id;

    info!(
        bot_id,
        update_id,
        kind = ?kind,
        "Processing webhook update"
    );

    // 4. Dispatch to handler
    if let Some(dispatcher) = &state.update_dispatcher {
        match dispatcher.dispatch(update, entry).await {
            Ok(()) => {
                debug!(bot_id, update_id, "Update dispatched successfully");
            }
            Err(e) => {
                error!(bot_id, update_id, error = %e, "Handler returned error");
                return Err(ApiError::ServiceUnavailable(
                    "webhook processing failed; Telegram should retry".into(),
                ));
            }
        }
    } else {
        warn!(
            bot_id,
            update_id, "No update dispatcher configured, dropping update"
        );
        return Err(ApiError::ServiceUnavailable(
            "webhook dispatcher unavailable; Telegram should retry".into(),
        ));
    }

    Ok(Json(json!({})))
}

/// POST /telegram/webhook/:bot_id/register
///
/// Register a bot in the webhook registry so its webhook endpoint becomes active.
pub async fn register_bot(
    State(state): State<AppState>,
    Path(bot_id): Path<i64>,
    headers: axum::http::HeaderMap,
    Json(body): Json<RegisterBotRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let token_bot_id = body
        .bot_token
        .split_once(':')
        .and_then(|(value, _)| value.parse::<i64>().ok())
        .ok_or_else(|| ApiError::BadRequest("invalid bot_token format".into()))?;
    if token_bot_id != bot_id {
        return Err(ApiError::BadRequest("bot_token identity mismatch".into()));
    }
    let merchant_id = uuid::Uuid::parse_str(&body.merchant_id)
        .map_err(|_| ApiError::BadRequest("invalid merchant_id".into()))?;
    let merchant_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM merchants WHERE id = $1 AND tenant_id = $2 AND bot_id = $3)",
    )
    .bind(merchant_id)
    .bind(&body.tenant_id)
    .bind(bot_id)
    .fetch_one(state.db()?)
    .await?;
    if !merchant_exists {
        return Err(ApiError::NotFound("merchant bot binding not found".into()));
    }
    let entry = BotEntry {
        bot_id,
        bot_token: body.bot_token.clone(),
        tenant_id: body.tenant_id.clone(),
        merchant_id: body.merchant_id.clone(),
        bot_username: body.bot_username.clone(),
    };

    state.bot_registry.register(entry).await;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Bot {} registered for webhook", bot_id)
    })))
}

/// DELETE /telegram/webhook/:bot_id/register
///
/// Unregister a bot from the webhook registry.
pub async fn unregister_bot(
    State(state): State<AppState>,
    Path(bot_id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    state.bot_registry.unregister(bot_id).await;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Bot {} unregistered from webhook", bot_id)
    })))
}

/// GET /telegram/webhook/bots
///
/// List all registered bots (for admin/monitoring).
pub async fn list_bots(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let entries = state.bot_registry.list_entries().await;
    let bots: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "bot_id": e.bot_id,
                "tenant_id": e.tenant_id,
                "merchant_id": e.merchant_id,
                "bot_username": e.bot_username,
            })
        })
        .collect();

    Ok(Json(json!({
        "status": "ok",
        "data": bots
    })))
}

/// POST /telegram/webhook/dispatch-test
///
/// Test endpoint: dispatch a raw update to a specific bot's handler (admin only).
pub async fn dispatch_test(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<DispatchTestRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let entry = state
        .bot_registry
        .get(body.bot_id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Bot {} not found", body.bot_id)))?;

    let update: TelegramUpdate = serde_json::from_value(body.update)
        .map_err(|e| ApiError::BadRequest(format!("Invalid update payload: {}", e)))?;

    let kind = update.kind();
    let update_id = update.update_id;

    if let Some(dispatcher) = &state.update_dispatcher {
        match dispatcher.dispatch(update, entry).await {
            Ok(()) => Ok(Json(json!({
                "status": "ok",
                "dispatched": true,
                "update_id": update_id,
                "kind": kind
            }))),
            Err(e) => Ok(Json(json!({
                "status": "handler_error",
                "dispatched": true,
                "update_id": update_id,
                "kind": kind,
                "error": e
            }))),
        }
    } else {
        Ok(Json(json!({
            "status": "ok",
            "dispatched": false,
            "message": "No dispatcher configured"
        })))
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterBotRequest {
    pub bot_token: String,
    pub tenant_id: String,
    pub merchant_id: String,
    pub bot_username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DispatchTestRequest {
    pub bot_id: i64,
    pub update: Value,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive a deterministic webhook secret from the bot token.
/// This allows us to validate incoming webhooks without storing a separate secret.
pub fn derive_webhook_secret(
    master_key: &str,
    bot_id: i64,
    bot_token: &str,
) -> Result<String, String> {
    if master_key.trim().is_empty() {
        return Err("TGBOT_SECRET_ENCRYPTION_KEY is required".into());
    }
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(master_key.as_bytes())
        .map_err(|_| "failed to initialize webhook secret".to_string())?;
    mac.update(format!("telegram-webhook\0{bot_id}\0{bot_token}").as_bytes());
    Ok(format!("wh_{}", hex::encode(mac.finalize().into_bytes())))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

/// Load all active merchant bots from the database into the registry.
/// Called at startup to warm the registry.
pub async fn load_bots_from_db(
    pool: &sqlx::PgPool,
    registry: &BotRegistry,
    encryption_key: &str,
) -> anyhow::Result<()> {
    let rows = sqlx::query(
        "SELECT m.id, m.tenant_id, m.bot_id, m.bot_username, secrets.secret_ref AS bot_token_ref \
         FROM merchants m \
         JOIN tenant_secret_refs secrets \
           ON secrets.tenant_id = m.tenant_id AND secrets.secret_key = 'TELEGRAM_BOT_TOKEN' \
         WHERE m.status IN ('active', 'active_free') AND m.bot_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let cipher = SecretCipher::from_encoded_key(encryption_key).map_err(anyhow::Error::msg)?;

    let mut count = 0u32;
    for row in rows {
        let merchant_id: uuid::Uuid = row.try_get("id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let bot_id: Option<i64> = row.try_get("bot_id")?;
        let bot_username: Option<String> = row.try_get("bot_username")?;
        let bot_token_ref: Option<String> = row.try_get("bot_token_ref")?;

        let Some(bot_token_ref) = bot_token_ref else {
            continue;
        };
        let bot_token = match cipher.decrypt(&tenant_id, "TELEGRAM_BOT_TOKEN", &bot_token_ref) {
            Ok(value) => value,
            Err(error) => {
                warn!(tenant_id, error, "Skipping bot with unreadable token");
                continue;
            }
        };

        if let Some(bid) = bot_id {
            registry
                .register(BotEntry {
                    bot_id: bid,
                    bot_token,
                    tenant_id,
                    merchant_id: merchant_id.to_string(),
                    bot_username,
                })
                .await;
            count += 1;
        }
    }

    info!(count, "Loaded bots into webhook registry from database");
    Ok(())
}

// ---------------------------------------------------------------------------
// Built-in default handler — logs and acks
// ---------------------------------------------------------------------------

/// Install the default handler set into the dispatcher.
/// This provides basic logging and no-op for unhandled update kinds.
pub async fn install_default_handlers(
    dispatcher: &UpdateDispatcher,
    pool: Option<sqlx::PgPool>,
    llm: LiteLLMClient,
    telegram_api_base_url: String,
    grant_store: GrantStore,
    otp_store: OtpStore,
    session_secret: String,
    frontend_url: String,
) {
    let message_pool = pool.clone();
    let message_llm = llm.clone();
    let message_api_base = telegram_api_base_url.clone();
    dispatcher
        .on(UpdateKind::Message, move |update, entry| {
            let pool = message_pool.clone();
            let llm = message_llm.clone();
            let api_base = message_api_base.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                if let Some(message) = &update.message {
                    handle_incoming_message(pool, &llm, &entry, message, false, &api_base).await?;
                }
                if let Some(msg) = &update.message {
                    let chat_id = msg.get("chat").and_then(|c| c.get("id"));
                    let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    let from = msg
                        .get("from")
                        .and_then(|f| f.get("username").and_then(|u| u.as_str()))
                        .unwrap_or("unknown");
                    info!(
                        bot_id = entry.bot_id,
                        ?chat_id,
                        from,
                        text_len = text.len(),
                        "Received message"
                    );
                }
                Ok(())
            }
        })
        .await;

    let edited_message_pool = pool.clone();
    let edited_message_llm = llm.clone();
    let edited_api_base = telegram_api_base_url.clone();
    dispatcher
        .on(UpdateKind::EditedMessage, move |update, entry| {
            let pool = edited_message_pool.clone();
            let llm = edited_message_llm.clone();
            let api_base = edited_api_base.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                if let Some(message) = &update.edited_message {
                    handle_incoming_message(pool, &llm, &entry, message, true, &api_base).await?;
                }
                Ok(())
            }
        })
        .await;

    // Callback query handler
    let callback_pool = pool.clone();
    let callback_grant_store = grant_store.clone();
    let callback_otp_store = otp_store.clone();
    let callback_session_secret = session_secret.clone();
    let callback_frontend_url = frontend_url.clone();
    dispatcher
        .on(UpdateKind::CallbackQuery, move |update, entry| {
            let pool = callback_pool.clone();
            let telegram_api_base_url = telegram_api_base_url.clone();
            let grant_store = callback_grant_store.clone();
            let otp_store = callback_otp_store.clone();
            let session_secret = callback_session_secret.clone();
            let frontend_url = callback_frontend_url.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                // 控制台菜单回调（主菜单 / 管理后台入口 / 商户控制台入口 / 客服）
                if handle_console_callback(
                    &update,
                    &entry,
                    Some(pool),
                    &telegram_api_base_url,
                    &grant_store,
                    &otp_store,
                    &session_secret,
                    &frontend_url,
                )
                .await?
                {
                    return Ok(());
                }
                if handle_channel_subscription_callback(
                    &update,
                    &entry,
                    Some(pool),
                    &telegram_api_base_url,
                )
                .await?
                {
                    return Ok(());
                }
                if handle_language_callback(&update, &entry, Some(pool), &telegram_api_base_url)
                    .await?
                {
                    return Ok(());
                }
                if handle_wallet_callback(&update, &entry, Some(pool), &telegram_api_base_url)
                    .await?
                {
                    return Ok(());
                }
                if handle_customer_callback(&update, &entry, Some(pool), &telegram_api_base_url)
                    .await?
                {
                    return Ok(());
                }
                if handle_verify_math_callback(&update, &entry, Some(pool), &telegram_api_base_url)
                    .await?
                {
                    return Ok(());
                }
                if handle_verify_quiz_callback(&update, &entry, Some(pool), &telegram_api_base_url)
                    .await?
                {
                    return Ok(());
                }
                if let Some(cq) = &update.callback_query {
                    let data = cq.get("data").and_then(|d| d.as_str()).unwrap_or("");
                    let callback_chat_id = cq
                        .get("message")
                        .and_then(|message| message.get("chat"))
                        .and_then(|chat| chat.get("id"))
                        .and_then(Value::as_i64);
                    let telegram_user_id = cq
                        .get("from")
                        .and_then(|from| from.get("id"))
                        .and_then(Value::as_i64);
                    if let Some(callback_id) = cq.get("id").and_then(Value::as_str) {
                        let _ = audit_repo::log_unsupported_telegram_callback(
                            pool,
                            &entry.tenant_id,
                            callback_id,
                            data,
                            callback_chat_id,
                            telegram_user_id,
                        )
                        .await;
                        if let Err(error) = acknowledge_unsupported_callback(
                            &entry,
                            callback_id,
                            &telegram_api_base_url,
                        )
                        .await
                        {
                            warn!(
                                bot_id = entry.bot_id,
                                error, "Failed to acknowledge unsupported callback query"
                            );
                        }
                    }
                    warn!(
                        bot_id = entry.bot_id,
                        data, "Unsupported Telegram callback query"
                    );
                }
                Ok(())
            }
        })
        .await;

    // Chat join request handler
    let join_request_pool = pool.clone();
    dispatcher
        .on(UpdateKind::ChatJoinRequest, move |update, entry| {
            let pool = join_request_pool.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                if let Some(jr) = &update.chat_join_request {
                    handle_chat_join_request(pool, &entry, jr).await?;
                    let chat_id = jr.get("chat").and_then(|c| c.get("id"));
                    let from = jr.get("from").and_then(|f| f.get("id"));
                    info!(
                        bot_id = entry.bot_id,
                        ?chat_id,
                        ?from,
                        "Received chat join request"
                    );
                }
                Ok(())
            }
        })
        .await;

    // My chat member handler
    let my_chat_member_pool = pool.clone();
    dispatcher
        .on(UpdateKind::MyChatMember, move |update, entry| {
            let pool = my_chat_member_pool.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                if let Some(cm) = &update.my_chat_member {
                    handle_chat_member_update(pool, &entry, cm, true).await?;
                    let chat_id = cm.get("chat").and_then(|c| c.get("id"));
                    info!(
                        bot_id = entry.bot_id,
                        ?chat_id,
                        "Chat member status changed"
                    );
                }
                Ok(())
            }
        })
        .await;

    let chat_member_pool = pool.clone();
    dispatcher
        .on(UpdateKind::ChatMember, move |update, entry| {
            let pool = chat_member_pool.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                if let Some(member) = &update.chat_member {
                    handle_chat_member_update(pool, &entry, member, false).await?;
                }
                Ok(())
            }
        })
        .await;

    // Default handler — catch-all
    dispatcher
        .on_default(|update, entry| async move {
            debug!(
                bot_id = entry.bot_id,
                update_id = update.update_id,
                kind = ?update.kind(),
                "Unhandled update kind (default handler)"
            );
            Ok(())
        })
        .await;
}

#[derive(Debug, PartialEq, Eq)]
struct InboundContent {
    content: Option<String>,
    msg_type: String,
    file_id: Option<String>,
    media_unique_id: Option<String>,
}

struct TrackedGroupPolicy {
    is_monitored: bool,
}

/// 取租户管理员 Telegram ID（merchants.owner_tg_user_id），未配置返回 None。
async fn merchant_owner_tg_id(pool: &sqlx::PgPool, tenant_id: &str) -> Result<Option<i64>, String> {
    let owner: Option<i64> = sqlx::query_scalar(
        "SELECT owner_tg_user_id FROM merchants WHERE tenant_id = $1 AND status IN ('active','active_free')",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "failed to load merchant owner".to_string())?;
    Ok(owner.filter(|value| *value > 0))
}

/// HTML 转义（对齐 Python escape()）：用户消息内容进 HTML 卡片前必须转义，防标签注入。
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 用 BotEntry 构造 Telegram 客户端（转发/回复消息用）。
fn telegram_for_bot(entry: &BotEntry, api_base_url: &str) -> TelegramClient {
    TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: api_base_url.to_string(),
        timeout_secs: 30,
    })
    .expect("telegram client build must not fail")
}

/// 取用户最近一条文本消息（双向翻译语言检测参考）。
async fn user_last_text(
    pool: &sqlx::PgPool,
    user_id: i64,
    tenant_id: &str,
) -> Result<Option<String>, String> {
    let content: Option<String> = sqlx::query_scalar(
        "SELECT content FROM chat.messages \
         WHERE from_userid = $1 AND tenant_id = $2 AND is_deleted = false \
           AND NULLIF(btrim(content), '') IS NOT NULL \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "failed to load user last message".to_string())?;
    Ok(content)
}

/// 管理员回复投递：双向翻译（中文 → 用户语言）+ sendMessage + 入库 + 译文写回 + 标记已读。
/// reply_to_tg_msg_id 为 None 时表示管理员直接发消息（非回复），不引用原消息。
async fn deliver_admin_reply(
    pool: &sqlx::PgPool,
    llm: &LiteLLMClient,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    target_user: i64,
    text: &str,
    reply_to_tg_msg_id: Option<i64>,
    admin_id: i64,
    admin_name: &str,
) -> Result<(), String> {
    // 双向翻译：管理员中文回复 → 用户语言
    let mut send_text = text.to_string();
    let mut translated_text: Option<String> = None;
    if let Ok(Some(customer_msg)) = user_last_text(pool, target_user, &entry.tenant_id).await {
        if let Some(translated) =
            translate::translate_to_customer(llm, pool, text, &customer_msg, target_user, &entry.tenant_id).await
        {
            send_text = translated.clone();
            translated_text = Some(translated);
        }
    }
    // 身份标签（对齐 Python format_admin_tag）：客服 <a>名字</a>：
    let esc_name = html_escape(admin_name);
    let esc_body = html_escape(&send_text);
    let final_text = format!(
        "客服 <a href=\"tg://user?id={admin_id}\">{esc_name}</a>：\n{esc_body}"
    );
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let sent = telegram
        .send_message(tgbot_integrations::telegram::SendMessageParams {
            chat_id: target_user,
            text: final_text,
            parse_mode: Some("HTML".to_string()),
            reply_markup: None,
            reply_to_message_id: reply_to_tg_msg_id,
        })
        .await
        .map_err(|_| "failed to deliver admin reply".to_string())?;
    let saved_id = message_repo::save_message(
        pool,
        entry.bot_id,
        target_user,
        Some(text),
        "text",
        None,
        None,
        true,
        Some(sent.message_id),
        &entry.tenant_id,
        reply_to_tg_msg_id,
    )
    .await
    .map_err(|_| "failed to persist admin reply".to_string())?;
    if let Some(translated) = translated_text {
        if translated != text {
            sqlx::query(
                "UPDATE chat.messages SET translated_text = $2, original_text = $3 \
                 WHERE id = $1 AND tenant_id = $4",
            )
            .bind(saved_id)
            .bind(&translated)
            .bind(text)
            .bind(&entry.tenant_id)
            .execute(pool)
            .await
            .map_err(|_| "failed to persist reply translation".to_string())?;
        }
    }
    // 标记该用户会话已读
    sqlx::query(
        "UPDATE chat.messages SET is_read = true \
         WHERE from_userid = $1 AND tenant_id = $2 AND is_read = false",
    )
    .bind(target_user)
    .bind(&entry.tenant_id)
    .execute(pool)
    .await
    .map_err(|_| "failed to mark read".to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// P3 入群验证（math 算术题 / captcha 数字验证码 / quiz 答题 + 静音 + 回调解禁）
// ---------------------------------------------------------------------------

/// 内存验证状态：(group_id, user_id) -> (正确答案, 验证消息 id, 验证类型)
/// 三种验证类型共用：
///   - math：answer = a + b（i64 算式结果）
///   - captcha：answer = 6 位数字验证码解析为 i64（Python _make_captcha 取
///     randint(100000,999999)，天然无前导零，可无损存入 i64）
///   - quiz：answer = 正确答案下标 correct_idx（i64）
/// 重启后丢失，验证中断的用户重新触发即可（Python 同样依赖内存 _verifying_users）。
static VERIFY_STATE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(i64, i64), (i64, i64, String)>>> =
    std::sync::OnceLock::new();

fn verify_state() -> &'static std::sync::Mutex<std::collections::HashMap<(i64, i64), (i64, i64, String)>> {
    VERIFY_STATE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 清理指定用户的验证状态（用户离开/被踢/验证超时后调用，防止重加入时被 has_pending 拦截）。
fn clear_verify_state(group_id: i64, user_id: i64) {
    if let Ok(mut state) = verify_state().lock() {
        state.remove(&(group_id, user_id));
    }
}

/// 简单确定性伪随机（避免引入 rand 依赖）：按时间种子生成 1..=9 的数字。
fn pseudo_int(seed: u64, max: i64) -> i64 {
    (seed % max as u64) as i64 + 1
}

/// 新成员入群触发验证：查群配置 → 静音 → 按 verify_type 派发（math/captcha/quiz）。
async fn start_group_verify(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    group_id: i64,
    user_id: i64,
    first_name: &str,
) -> Result<(), String> {
    let row = sqlx::query(
        "SELECT verify_enabled, verify_type, verify_timeout, verify_questions \
         FROM groups.tracked_groups \
         WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(&entry.tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "failed to load group verify config".to_string())?;
    let Some(row) = row else {
        return Ok(());
    };
    let enabled: bool = row.try_get("verify_enabled").unwrap_or(false);
    let vtype: String = row.try_get("verify_type").ok().unwrap_or_default();
    // 超时秒数（P2-6：对齐 Python get_group_verify_config，默认 120）
    let timeout_secs: i64 = row.try_get("verify_timeout").unwrap_or(120).max(10);
    let questions: Option<String> = row.try_get("verify_questions").ok().flatten();
    if !enabled || !matches!(vtype.as_str(), "math" | "captcha" | "quiz") {
        return Ok(());
    }

    // 已验证用户跳过；同时读取 status/pending_verify_msg_id 用于兜底清理过期会话
    let member = sqlx::query(
        "SELECT is_verified, status, pending_verify_msg_id FROM groups.group_members \
         WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(&entry.tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "failed to check member verify state".to_string())?;
    if member
        .as_ref()
        .map(|row| row.try_get::<bool, _>("is_verified").unwrap_or(false))
        .unwrap_or(false)
    {
        return Ok(());
    }

    // 已存在未完成的验证会话则跳过（对齐 Python _verifying_users 去重）。
    // P1-5 兜底：超时踢（verify_timeout_worker）只清 DB、不清 webhook 内存状态；
    // 若用户已被踢（status=left/banned）或 DB pending 已清空而内存仍残留，则清除
    // 过期状态后继续重新发题，避免用户重进群时被 has_pending 拦截绕过验证。
    let has_pending = verify_state()
        .lock()
        .map(|state| state.contains_key(&(group_id, user_id)))
        .unwrap_or(false);
    if has_pending {
        let stale = member.as_ref().map(|row| {
            let status: String = row.try_get("status").unwrap_or_default();
            let pending_msg: Option<i64> = row.try_get("pending_verify_msg_id").ok().flatten();
            matches!(status.as_str(), "left" | "banned") || pending_msg.is_none()
        }).unwrap_or(false);
        if stale {
            clear_verify_state(group_id, user_id);
        } else {
            return Ok(());
        }
    }

    // 静音
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let _ = telegram
        .restrict_chat_member(
            group_id,
            user_id,
            serde_json::json!({
                "can_send_messages": false,
                "can_send_polls": false,
                "can_send_other_messages": false,
                "can_add_web_page_previews": false
            }),
            None,
        )
        .await;

    match vtype.as_str() {
        "captcha" => {
            start_captcha_verify(pool, entry, telegram_api_base_url, group_id, user_id, first_name, timeout_secs)
                .await
        }
        "quiz" => {
            start_quiz_verify(
                pool,
                entry,
                telegram_api_base_url,
                group_id,
                user_id,
                first_name,
                questions.as_deref(),
                timeout_secs,
            )
            .await
        }
        _ => {
            start_math_verify(pool, entry, telegram_api_base_url, group_id, user_id, first_name, timeout_secs)
                .await
        }
    }
}

/// 记录验证挑战：写内存状态 + 持久化 pending_verify_msg_id + 入队超时踢
/// （超时秒数读 tracked_groups.verify_timeout，对齐 Python join_verify.py）。
async fn record_verify_challenge(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    group_id: i64,
    user_id: i64,
    answer: i64,
    verify_type: &str,
    msg_id: i64,
    timeout_secs: i64,
) {
    if let Ok(mut state) = verify_state().lock() {
        state.insert((group_id, user_id), (answer, msg_id, verify_type.to_string()));
    }
    let _ = sqlx::query(
        "UPDATE groups.group_members SET pending_verify_msg_id = $3 \
         WHERE group_id = $1 AND user_id = $2 AND tenant_id = $4",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(msg_id)
    .bind(&entry.tenant_id)
    .execute(pool)
    .await;
    // 超时踢：发题时入队延迟 job（对齐 Python join_verify.py，时长来自群配置）
    let _ = sqlx::query(
        "INSERT INTO job_queue (tenant_id, job_type, payload, priority, max_attempts, run_at) \
         VALUES ($1, 'verify_timeout_kick', jsonb_build_object('group_id', $2, 'user_id', $3, 'msg_id', $4), 50, 3, now() + make_interval(secs => $5))",
    )
    .bind(&entry.tenant_id)
    .bind(group_id)
    .bind(user_id)
    .bind(msg_id)
    .bind(timeout_secs)
    .execute(pool)
    .await;
}

/// 生成 6 位数字验证码（对齐 Python _make_captcha：randint(100000, 999999)）。
fn make_captcha(seed: u64) -> i64 {
    // 确定性伪随机，结果落在 [100000, 999999]
    100000 + ((seed.wrapping_mul(2654435761) ^ (seed >> 16)) % 900000) as i64
}

/// captcha 验证：静音后发文字消息（无按钮），用户回复验证码作答。
async fn start_captcha_verify(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    group_id: i64,
    user_id: i64,
    first_name: &str,
    timeout_secs: i64,
) -> Result<(), String> {
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
        .wrapping_add(user_id as u64);
    let code = make_captcha(now_nanos);
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let text = format!(
        "👋 欢迎 {first_name} 加入！\n\n请回复下方验证码完成入群验证：\n🔐 <code>{code}</code>\n\n验证通过后自动解除禁言，{timeout_secs} 秒内未验证将被移出群组。"
    );
    match telegram
        .send_message(tgbot_integrations::telegram::SendMessageParams {
            chat_id: group_id,
            text,
            parse_mode: Some("HTML".to_string()),
            reply_markup: None,
            reply_to_message_id: None,
        })
        .await
    {
        Ok(sent) => {
            record_verify_challenge(pool, entry, group_id, user_id, code, "captcha", sent.message_id, timeout_secs)
                .await;
            info!(group_id, user_id, code, "Captcha verify challenge sent");
        }
        Err(error) => warn!(%error, group_id, user_id, "Failed to send captcha verify challenge"),
    }
    Ok(())
}

/// quiz 验证：从 verify_questions 随机选题 → 带选项按钮（verify_quiz 回调）。
async fn start_quiz_verify(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    group_id: i64,
    user_id: i64,
    first_name: &str,
    questions_json: Option<&str>,
    timeout_secs: i64,
) -> Result<(), String> {
    // 题库为空时回退到 captcha（对齐 Python _start_quiz_verify: if not questions）
    let questions: Vec<serde_json::Value> = questions_json
        .and_then(|json| serde_json::from_str::<Vec<serde_json::Value>>(json).ok())
        .unwrap_or_default();
    if questions.is_empty() {
        return start_captcha_verify(pool, entry, telegram_api_base_url, group_id, user_id, first_name, timeout_secs)
            .await;
    }
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
        .wrapping_add(user_id as u64);
    let q = &questions[(now_nanos as usize) % questions.len()];
    let question_text = q
        .get("question")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("请回答问题")
        .to_string();
    let options: Vec<String> = q
        .get("options")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    if options.is_empty() {
        return start_captcha_verify(
            pool,
            entry,
            telegram_api_base_url,
            group_id,
            user_id,
            first_name,
            timeout_secs,
        )
        .await;
    }
    let correct_idx: i64 = q
        .get("correct")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
        .clamp(0, options.len() as i64 - 1);

    let buttons: Vec<serde_json::Value> = options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            serde_json::json!({
                "text": opt,
                "callback_data": format!("verify_quiz:{group_id}:{user_id}:{idx}:{correct_idx}")
            })
        })
        .collect();
    let keyboard = serde_json::json!({
        "inline_keyboard": buttons.iter().map(|b| vec![b.clone()]).collect::<Vec<_>>()
    });
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let text = format!(
        "👋 欢迎 {first_name} 加入！\n\n请回答以下问题完成入群验证：\n❓ {question_text}\n\n点击正确选项，验证后自动解除禁言。"
    );
    match telegram
        .send_message(tgbot_integrations::telegram::SendMessageParams {
            chat_id: group_id,
            text,
            parse_mode: None,
            reply_markup: Some(keyboard),
            reply_to_message_id: None,
        })
        .await
    {
        Ok(sent) => {
            record_verify_challenge(pool, entry, group_id, user_id, correct_idx, "quiz", sent.message_id, timeout_secs)
                .await;
            info!(group_id, user_id, correct_idx, "Quiz verify challenge sent");
        }
        Err(error) => warn!(%error, group_id, user_id, "Failed to send quiz verify challenge"),
    }
    Ok(())
}

/// math 验证（原有逻辑，抽成独立函数）：算术题 + 4 选项按钮。
async fn start_math_verify(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    group_id: i64,
    user_id: i64,
    first_name: &str,
    timeout_secs: i64,
) -> Result<(), String> {
    // 生成算术题（a + b = ?）
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
        .wrapping_add(user_id as u64);
    let a = pseudo_int(now_nanos, 9);
    let b = pseudo_int(now_nanos / 7, 9);
    let correct = a + b;

    // 选项：正确答案 + 3 个不同干扰值
    let mut options = vec![correct];
    let mut seed = now_nanos;
    while options.len() < 4 {
        seed = seed.wrapping_mul(31).wrapping_add(7);
        let candidate = correct as i64 + (seed % 11) as i64 - 5;
        if candidate > 0 && !options.contains(&candidate) {
            options.push(candidate);
        }
    }
    options.sort_by_key(|_v| {
        seed = seed.wrapping_mul(17).wrapping_add(3);
        seed % 1000
    });

    // 发送算术验证消息（4 个选项按钮）
    let buttons: Vec<serde_json::Value> = options
        .iter()
        .map(|value| {
            serde_json::json!({
                "text": value.to_string(),
                "callback_data": format!("verify_math:{group_id}:{user_id}:{value}")
            })
        })
        .collect();
    let keyboard = serde_json::json!({
        "inline_keyboard": [
            [buttons[0].clone(), buttons[1].clone()],
            [buttons[2].clone(), buttons[3].clone()]
        ]
    });
    let text = format!(
        "⚠️ 欢迎 {first_name}！请完成入群验证：**{a} + {b} = ?**（点击正确选项，验证后自动解除禁言）"
    );
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    match telegram
        .send_message(tgbot_integrations::telegram::SendMessageParams {
            chat_id: group_id,
            text,
            parse_mode: Some("Markdown".to_string()),
            reply_markup: Some(keyboard),
            reply_to_message_id: None,
        })
        .await
    {
        Ok(sent) => {
            record_verify_challenge(pool, entry, group_id, user_id, correct, "math", sent.message_id, timeout_secs)
                .await;
            info!(group_id, user_id, correct, "Group verify challenge sent");
        }
        Err(error) => warn!(%error, group_id, user_id, "Failed to send group verify challenge"),
    }
    Ok(())
}

/// 三种验证类型共用的通过处理：解除静音 + 标记 is_verified + 删验证消息 + 清内存状态。
/// math / quiz 回调与 captcha 文字回复均调用此函数。
async fn pass_group_verify(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    group_id: i64,
    user_id: i64,
) -> Result<(), String> {
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let _ = telegram
        .restrict_chat_member(
            group_id,
            user_id,
            serde_json::json!({
                "can_send_messages": true,
                "can_send_polls": true,
                "can_send_other_messages": true,
                "can_add_web_page_previews": true
            }),
            None,
        )
        .await;
    // 先从状态中取出验证消息 id（drop guard），再删除消息
    let verify_msg_id = verify_state()
        .lock()
        .ok()
        .and_then(|mut state| state.remove(&(group_id, user_id)).map(|(_, msg_id, _)| msg_id));
    if let Some(msg_id) = verify_msg_id {
        let _ = telegram.delete_message(group_id, msg_id).await;
    }
    let _ = sqlx::query(
        "UPDATE groups.group_members SET is_verified = true, pending_verify_msg_id = NULL, updated_at = now() \
         WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(&entry.tenant_id)
    .execute(pool)
    .await;
    info!(group_id, user_id, "Group verify passed");
    Ok(())
}

/// captcha 文字回复验证（对齐 Python handle_verify_message）：
/// 群内文本消息匹配该 (group,user) 的 captcha 答案 → 通过；否则删除作答消息保留挑战。
/// 返回 Ok(true) 表示这是一条验证作答消息（已消费，不应再落库）。
async fn handle_captcha_reply(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    group_id: i64,
    user_id: i64,
    text: &str,
    answer_message_id: i64,
) -> Result<bool, String> {
    let pending = verify_state()
        .lock()
        .ok()
        .and_then(|state| state.get(&(group_id, user_id)).cloned());
    let Some((answer, _msg_id, vtype)) = pending else {
        return Ok(false);
    };
    if vtype != "captcha" {
        return Ok(false);
    }
    // P2-7：以 / 开头的命令不当作验证作答（不消费、不删除）
    if text.trim_start().starts_with('/') {
        return Ok(false);
    }
    // 先比较后删除作答消息（对齐 Python handle_verify_message：比对成功后删）
    let matched = text.trim() == answer.to_string();
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let _ = telegram.delete_message(group_id, answer_message_id).await;
    if matched {
        pass_group_verify(pool, entry, telegram_api_base_url, group_id, user_id).await?;
        let _ = telegram
            .send_message(tgbot_integrations::telegram::SendMessageParams {
                chat_id: group_id,
                text: "✅ 验证通过，欢迎加入！".to_string(),
                parse_mode: None,
                reply_markup: None,
                reply_to_message_id: None,
            })
            .await;
        info!(group_id, user_id, "Captcha verify passed by reply");
    } else {
        // 答案错误：删除作答消息，保留验证挑战（对齐 Python）
        info!(group_id, user_id, "Captcha verify wrong answer");
    }
    Ok(true)
}

/// 入群验证回调：verify_math:{group_id}:{user_id}:{value}
async fn handle_verify_math_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: Option<&sqlx::PgPool>,
    telegram_api_base_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(rest) = data.strip_prefix("verify_math:") else {
        return Ok(false);
    };
    let Some(pool) = pool else {
        return Ok(false);
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 3 {
        return Ok(false);
    }
    let (Ok(group_id), Ok(user_id), Ok(chosen)) = (
        parts[0].parse::<i64>(),
        parts[1].parse::<i64>(),
        parts[2].parse::<i64>(),
    ) else {
        return Ok(false);
    };
    let Some(callback_id) = callback.get("id").and_then(Value::as_str) else {
        return Ok(false);
    };
    // 仅允许被验证用户本人点击
    let actor = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64);
    if actor != Some(user_id) {
        let telegram = telegram_for_bot(entry, telegram_api_base_url);
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("请本人完成验证".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    }

    let correct = verify_state()
        .lock()
        .map(|state| state.get(&(group_id, user_id)).map(|(answer, _, _)| *answer))
        .ok()
        .flatten();
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let Some(correct) = correct else {
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("验证已过期，请重新进群验证".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    };

    if chosen != correct {
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("❌ 答案错误，请重新选择".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    }

    // 验证通过：共用解禁 + 标记 + 删题 + 清状态
    pass_group_verify(pool, entry, telegram_api_base_url, group_id, user_id).await?;
    let _ = telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: Some("✅ 验证通过，欢迎加入！".to_string()),
            show_alert: Some(false),
        })
        .await;
    Ok(true)
}

/// 入群验证回调：verify_quiz:{group_id}:{user_id}:{choice_idx}:{correct_idx}
async fn handle_verify_quiz_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: Option<&sqlx::PgPool>,
    telegram_api_base_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(rest) = data.strip_prefix("verify_quiz:") else {
        return Ok(false);
    };
    let Some(pool) = pool else {
        return Ok(false);
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 4 {
        return Ok(false);
    }
    let (Ok(group_id), Ok(user_id), Ok(chosen), Ok(correct)) = (
        parts[0].parse::<i64>(),
        parts[1].parse::<i64>(),
        parts[2].parse::<i64>(),
        parts[3].parse::<i64>(),
    ) else {
        return Ok(false);
    };
    let Some(callback_id) = callback.get("id").and_then(Value::as_str) else {
        return Ok(false);
    };
    // 仅允许被验证用户本人点击
    let actor = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64);
    if actor != Some(user_id) {
        let telegram = telegram_for_bot(entry, telegram_api_base_url);
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("请本人完成验证".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    }

    // 必须存在未完成的验证会话
    let has_pending = verify_state()
        .lock()
        .map(|state| state.contains_key(&(group_id, user_id)))
        .unwrap_or(false);
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    if !has_pending {
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("验证已过期，请重新进群验证".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    }

    if chosen != correct {
        // P1-3：答错即踢出（对齐 Python join_verify.py:566-586：ban+unban、清状态、删验证消息）
        let _ = telegram.ban_chat_member(group_id, user_id).await;
        let _ = telegram.unban_chat_member(group_id, user_id).await;
        let challenge_msg_id = verify_state()
            .lock()
            .ok()
            .and_then(|mut state| state.remove(&(group_id, user_id)).map(|(_, msg_id, _)| msg_id));
        if let Some(msg_id) = challenge_msg_id {
            let _ = telegram.delete_message(group_id, msg_id).await;
        }
        let _ = sqlx::query(
            "UPDATE groups.group_members SET pending_verify_msg_id = NULL, status = 'left', updated_at = now() \
             WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(group_id)
        .bind(user_id)
        .bind(&entry.tenant_id)
        .execute(pool)
        .await;
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("❌ 答案错误，已移出群组".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    }

    // 验证通过：共用解禁 + 标记 + 删题 + 清状态
    pass_group_verify(pool, entry, telegram_api_base_url, group_id, user_id).await?;
    let _ = telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: Some("✅ 验证通过，欢迎加入！".to_string()),
            show_alert: Some(false),
        })
        .await;
    Ok(true)
}

async fn handle_incoming_message(
    pool: &sqlx::PgPool,
    llm: &LiteLLMClient,
    entry: &BotEntry,
    message: &Value,
    edited: bool,
    telegram_api_base_url: &str,
) -> Result<(), String> {
    let Some(chat) = message.get("chat") else {
        return Ok(());
    };
    let Some(chat_id) = chat.get("id").and_then(Value::as_i64) else {
        return Ok(());
    };
    let chat_type = chat.get("type").and_then(Value::as_str).unwrap_or("");
    let Some(message_id) = message
        .get("message_id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
    else {
        return Ok(());
    };
    let Some(from) = message.get("from") else {
        return Ok(());
    };
    if from.get("is_bot").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(());
    }
    let user_id = from
        .get("id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "Telegram message has no sender id".to_string())?;
    let username = optional_nonempty_string(from, "username");
    let first_name = optional_nonempty_string(from, "first_name");
    let last_name = optional_nonempty_string(from, "last_name");
    let message_ts = message
        .get("date")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let reply_to_message_id = message
        .get("reply_to_message")
        .and_then(|reply| reply.get("message_id"))
        .and_then(Value::as_i64);
    let inbound = extract_inbound_content(message);

    match chat_type {
        "private" => {
            // P2 客服闭环：管理员回复转发的消息 → 直接发回用户（带双向翻译）。
            // 对齐 Python：管理员回复推送消息（reply_to）保持直接发送；
            // 非回复消息改为写入 pending_admin_messages（不直接发），给管理员确认按钮。
            if let Some(admin_id) = merchant_owner_tg_id(pool, &entry.tenant_id).await? {
                if user_id == admin_id {
                    let Some(inbound_ref) = inbound.as_ref() else {
                        return Ok(());
                    };
                    let text = inbound_ref
                        .content
                        .as_deref()
                        .map(str::trim)
                        .filter(|text| !text.is_empty());
                    let is_command = text.is_some_and(|text| text.starts_with('/'));
                    let has_media = inbound_ref.file_id.is_some();
                    if (text.is_some() && !is_command) || has_media {
                        // 1) 回复推送消息 → 反查目标用户
                        let mut target_user: Option<i64> = None;
                        let mut reply_to_tg_msg_id: Option<i64> = None;
                        if let Some(reply_to) = reply_to_message_id {
                            let quoted_msg_id: Option<i64> = sqlx::query_scalar(
                                "SELECT quoted_msg_id FROM chat.messages \
                                 WHERE from_userid = $1 AND to_userid = $2 AND tg_msg_id = $3 \
                                   AND tenant_id = $4 AND quoted_msg_id IS NOT NULL \
                                 ORDER BY id DESC LIMIT 1",
                            )
                            .bind(entry.bot_id)
                            .bind(admin_id)
                            .bind(reply_to)
                            .bind(&entry.tenant_id)
                            .fetch_optional(pool)
                            .await
                            .map_err(|_| "failed to resolve admin forward".to_string())?;
                            if let Some(orig_tg_msg_id) = quoted_msg_id {
                                let resolved: Option<i64> = sqlx::query_scalar(
                                    "SELECT from_userid FROM chat.messages \
                                     WHERE tg_msg_id = $1 AND tenant_id = $2 \
                                       AND from_userid <> $3 AND from_userid > 0 \
                                     ORDER BY id DESC LIMIT 1",
                                )
                                .bind(orig_tg_msg_id)
                                .bind(&entry.tenant_id)
                                .bind(entry.bot_id)
                                .fetch_optional(pool)
                                .await
                                .map_err(|_| "failed to resolve reply target".to_string())?;
                                target_user = resolved;
                                reply_to_tg_msg_id = Some(orig_tg_msg_id);
                            }
                        }
                        // 2) 回复分支：保持直接发送（对齐 Python 回复消息即时回复）
                        if let Some(target_user) = target_user {
                            if let Some(text) = text {
                                deliver_admin_reply(
                                    pool,
                                    llm,
                                    entry,
                                    telegram_api_base_url,
                                    target_user,
                                    text,
                                    reply_to_tg_msg_id,
                                    user_id,
                                    first_name.as_deref().unwrap_or("客服"),
                                )
                                .await?;
                                info!(admin_id, target_user, "Admin reply delivered to user");
                            }
                            return Ok(());
                        }
                        // 3) 非回复分支：路由最近活跃用户 → 写入 pending_admin_messages，
                        //    给管理员确认按钮（对齐 Python quick_send/select_other_user/cancel_send）
                        let latest: Option<i64> = sqlx::query_scalar(
                            "SELECT from_userid FROM chat.messages \
                             WHERE to_userid = $1 AND tenant_id = $2 \
                               AND from_userid <> $1 AND from_userid > 0 \
                               AND is_deleted = false AND msg_type = 'text' \
                               AND NULLIF(btrim(content), '') IS NOT NULL \
                               AND left(btrim(content), 1) <> '/' \
                             ORDER BY id DESC LIMIT 1",
                        )
                        .bind(entry.bot_id)
                        .bind(&entry.tenant_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|_| "failed to resolve latest active user".to_string())?;
                        if let Some(latest_user) = latest {
                            let msg_uuid = insert_pending_admin_message(
                                pool,
                                admin_id,
                                text.unwrap_or(""),
                                &inbound_ref.msg_type,
                                inbound_ref.file_id.as_deref(),
                                Some(latest_user),
                            )
                            .await?;
                            let telegram = telegram_for_bot(entry, telegram_api_base_url);
                            let markup = serde_json::json!({
                                "inline_keyboard": [
                                    [{"text": "✅ 确认发送", "callback_data": format!("quick_send:{latest_user}:{msg_uuid}")}],
                                    [{"text": "👥 选择其他用户", "callback_data": format!("select_other_user:{msg_uuid}")}],
                                    [{"text": "❌ 取消", "callback_data": format!("cancel_send:{msg_uuid}")}]
                                ]
                            });
                            let _ = telegram
                                .send_message(tgbot_integrations::telegram::SendMessageParams {
                                    chat_id: admin_id,
                                    text: "📤 消息已暂存，请确认发送对象：".to_string(),
                                    parse_mode: None,
                                    reply_markup: Some(markup),
                                    reply_to_message_id: Some(message_id),
                                })
                                .await;
                            info!(admin_id, latest_user, msg_uuid, "Admin message staged for confirmation");
                            return Ok(());
                        }
                        // 4) 无目标用户：提示管理员（对齐 Python "无法直接发送"）
                        let telegram = telegram_for_bot(entry, telegram_api_base_url);
                        let _ = telegram
                            .send_message(tgbot_integrations::telegram::SendMessageParams {
                                chat_id: admin_id,
                                text: "❌ 无法直接发送：您未引用任何消息，且数据库中暂无活跃用户。\n请回复推送卡片，或从 Web 端发送。".to_string(),
                                parse_mode: None,
                                reply_markup: None,
                                reply_to_message_id: Some(message_id),
                            })
                            .await;
                        return Ok(());
                    }
                }
            }
            upsert_chat_user(
                pool,
                user_id,
                username.as_deref(),
                first_name.as_deref(),
                last_name.as_deref(),
            )
            .await?;
            let existing_id = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM chat.messages \
                 WHERE tenant_id = $1 AND tg_msg_id = $2 \
                   AND from_userid = $3 AND to_userid = $4 \
                 ORDER BY id DESC LIMIT 1",
            )
            .bind(&entry.tenant_id)
            .bind(message_id)
            .bind(user_id)
            .bind(entry.bot_id)
            .fetch_optional(pool)
            .await
            .map_err(|_| "failed to check private message duplicate".to_string())?;
            if let Some(existing_id) = existing_id {
                if edited {
                    sqlx::query(
                        "UPDATE chat.messages SET content = $2, msg_type = $3, file_id = $4, \
                         original_text = COALESCE(original_text, content), status = 'edited' \
                         WHERE id = $1 AND tenant_id = $5",
                    )
                    .bind(existing_id)
                    .bind(inbound.as_ref().and_then(|value| value.content.as_deref()))
                    .bind(
                        inbound
                            .as_ref()
                            .map(|value| value.msg_type.as_str())
                            .unwrap_or("service"),
                    )
                    .bind(inbound.as_ref().and_then(|value| value.file_id.as_deref()))
                    .bind(&entry.tenant_id)
                    .execute(pool)
                    .await
                    .map_err(|_| "failed to update private message".to_string())?;
                }
                return Ok(());
            }
            let Some(inbound) = inbound else {
                return Ok(());
            };
            let stored_id = message_repo::save_message(
                pool,
                user_id,
                entry.bot_id,
                inbound.content.as_deref(),
                &inbound.msg_type,
                inbound.file_id.as_deref(),
                None,
                false,
                Some(message_id),
                &entry.tenant_id,
                reply_to_message_id,
            )
            .await
            .map_err(|_| "failed to persist private message".to_string())?;
            if let Some(message_ts) = message_ts {
                sqlx::query(
                    "UPDATE chat.messages SET timestamp = to_timestamp($2) \
                     WHERE id = $1 AND tenant_id = $3",
                )
                .bind(stored_id)
                .bind(message_ts)
                .bind(&entry.tenant_id)
                .execute(pool)
                .await
                .map_err(|_| "failed to persist private message timestamp".to_string())?;
            }
            // 客服自动翻译（用户 → 管理员，译为中文）：
            // 文本/媒体 caption 且启发式命中时调用 LLM，译文写回 translated_text/original_text。
            // 与 Python translator.py 行为一致：翻译失败/无需翻译时静默跳过，不影响消息落库。
            if inbound
                .content
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty())
            {
                if let Some(text) = inbound.content.as_deref() {
                    if let Some(translated) =
                        translate::translate_to_admin(llm, pool, text).await
                    {
                        sqlx::query(
                            "UPDATE chat.messages SET translated_text = $2, original_text = $3 \
                             WHERE id = $1 AND tenant_id = $4",
                        )
                        .bind(stored_id)
                        .bind(&translated)
                        .bind(text)
                        .bind(&entry.tenant_id)
                        .execute(pool)
                        .await
                        .map_err(|_| "failed to persist message translation".to_string())?;
                    }
                }
            }
            // 命令消息入队由 DB 触发器处理（migration 0012 telegram_command_reply_outbox，
            // chat.messages INSERT 后自动入队 telegram_command_reply job），无需代码显式入队。
            sqlx::query(
                "UPDATE chat.admin_inbox SET username = $2, updated_at = now() \
                 WHERE user_id = $1 AND tenant_id = $3",
            )
            .bind(user_id)
            .bind(username.as_deref().unwrap_or(""))
            .bind(&entry.tenant_id)
            .execute(pool)
            .await
            .map_err(|_| "failed to update private inbox identity".to_string())?;
            // P2 客服闭环：用户消息转发给管理员 Telegram（带操作按钮）。
            // 转发消息入库（quoted_msg_id 指向用户原消息），管理员回复该转发消息时反查用户。
            // 对齐 Python：卡片显示「原文 + 译文」（译文来自用户→管理员翻译）。
            if let Some(admin_id) = merchant_owner_tg_id(pool, &entry.tenant_id).await? {
                if admin_id != user_id {
                    let display_text = inbound
                        .content
                        .as_deref()
                        .map(|text| text.trim())
                        .filter(|text| !text.is_empty())
                        .map(|text| text.chars().take(300).collect::<String>())
                        .unwrap_or_else(|| "[媒体消息]".to_string());
                    let card_translated: Option<String> = sqlx::query_scalar(
                        "SELECT translated_text FROM chat.messages WHERE id = $1",
                    )
                    .bind(stored_id)
                    .fetch_optional(pool)
                    .await
                    .ok()
                    .flatten();
                    let esc_display = html_escape(&display_text);
                    let esc_name = html_escape(first_name.as_deref().unwrap_or("用户"));
                    let esc_username = html_escape(username.as_deref().unwrap_or(""));
                    let identity_line = if esc_username.is_empty() {
                        format!("<a href=\"tg://user?id={user_id}\">{esc_name}</a>")
                    } else {
                        format!("<a href=\"tg://user?id={user_id}\">{esc_name}</a> (@{esc_username})")
                    };
                    let push_text = if let Some(translated) = card_translated {
                        if !translated.trim().is_empty() && translated.trim() != display_text {
                            let esc_translated = html_escape(&translated);
                            format!(
                                "💬 新私聊消息 · {identity_line}\n\n原文: {esc_display}\n译文: {esc_translated}\n\n💡 点击下方按钮快速处理，或直接回复此消息。",
                            )
                        } else {
                            format!(
                                "💬 新私聊消息 · {identity_line}\n\n内容: {esc_display}\n\n💡 点击下方按钮快速处理，或直接回复此消息。",
                            )
                        }
                    } else {
                        format!(
                            "💬 新私聊消息 · {identity_line}\n\n内容: {esc_display}\n\n💡 点击下方按钮快速处理，或直接回复此消息。",
                        )
                    };
                    let telegram = telegram_for_bot(entry, telegram_api_base_url);
                    let markup = serde_json::json!({
                        "inline_keyboard": [[
                            {"text": "📥 已读", "callback_data": format!("cs_mark_read:{user_id}")},
                            {"text": "⚡ 回复", "callback_data": format!("cs_quick_reply:{user_id}")},
                            {"text": "✖ 结束", "callback_data": format!("cs_close_session:{user_id}")}
                        ]]
                    });
                    match telegram
                        .send_message(tgbot_integrations::telegram::SendMessageParams {
                            chat_id: admin_id,
                            text: push_text.clone(),
                            parse_mode: Some("HTML".to_string()),
                            reply_markup: Some(markup),
                            reply_to_message_id: None,
                        })
                        .await
                    {
                        Ok(sent) => {
                            // 转发记录入库：tg_msg_id=转发消息 id，quoted_msg_id=用户原消息 id
                            let forward_id = sqlx::query_scalar::<_, i64>(
                                "INSERT INTO chat.messages \
                                 (trace_id, from_userid, to_userid, content, msg_type, is_read, tg_msg_id, \
                                  quoted_msg_id, status, tenant_id) \
                                 VALUES (gen_random_uuid()::text, $1, $2, $3, 'text', true, $4, $5, 'sent', $6) \
                                 RETURNING id",
                            )
                            .bind(entry.bot_id)
                            .bind(admin_id)
                            .bind(&push_text)
                            .bind(sent.message_id)
                            .bind(message_id)
                            .bind(&entry.tenant_id)
                            .fetch_one(pool)
                            .await
                            .map_err(|_| "failed to persist admin forward".to_string())?;
                            let _ = forward_id;
                            info!(admin_id, user_id, "Forwarded private message to admin");
                        }
                        Err(error) => warn!(%error, admin_id, user_id, "Failed to forward private message to admin"),
                    }
                }
            }
            return Ok(());
        }
        "group" | "supergroup" => {
            let group_title =
                optional_nonempty_string(chat, "title").unwrap_or_else(|| "Unknown".into());
            let Some(group_policy) =
                claim_or_refresh_group(pool, entry, chat_id, &group_title).await?
            else {
                return Ok(());
            };
            upsert_group_member(
                pool,
                &entry.tenant_id,
                chat_id,
                &group_title,
                user_id,
                username.as_deref(),
                first_name.as_deref(),
                last_name.as_deref(),
                "active",
            )
            .await?;
            apply_message_membership_events(pool, entry, message, chat_id, &group_title, telegram_api_base_url).await?;
            // P3 captcha 文字回复验证（对齐 Python handle_verify_message）：
            // 命中验证作答消息即消费（删除作答消息），不再作为普通群消息落库。
            if let Some(text) = message.get("text").and_then(Value::as_str) {
                if handle_captcha_reply(
                    pool,
                    entry,
                    telegram_api_base_url,
                    chat_id,
                    user_id,
                    text,
                    message_id,
                )
                .await?
                {
                    return Ok(());
                }
            }
            if !group_policy.is_monitored {
                return Ok(());
            }
            let Some(inbound) = inbound else {
                return Ok(());
            };
            let ad_text_hash = inbound
                .content
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ad_text_hash);
            let quoted_text = message
                .get("reply_to_message")
                .and_then(|reply| reply.get("text").or_else(|| reply.get("caption")))
                .and_then(Value::as_str);
            if edited {
                sqlx::query(
                    "UPDATE groups.group_messages SET content = $4, msg_type = $5, file_id = $6, \
                     media_unique_id = $7, ad_text_hash = $8, quoted_tg_msg_id = $9, \
                     quoted_text = $10, message_ts = COALESCE($11, message_ts), \
                     timestamp = COALESCE(to_timestamp($11), timestamp) \
                     WHERE group_id = $1 AND tg_msg_id = $2 AND tenant_id = $3",
                )
                .bind(chat_id)
                .bind(message_id)
                .bind(&entry.tenant_id)
                .bind(inbound.content.as_deref())
                .bind(&inbound.msg_type)
                .bind(inbound.file_id.as_deref())
                .bind(inbound.media_unique_id.as_deref())
                .bind(ad_text_hash.as_deref())
                .bind(reply_to_message_id)
                .bind(quoted_text)
                .bind(message_ts)
                .execute(pool)
                .await
                .map_err(|_| "failed to update group message".to_string())?;
            } else {
                sqlx::query(
                    "INSERT INTO groups.group_messages \
                     (group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, \
                      file_id, media_unique_id, ad_text_hash, quoted_tg_msg_id, quoted_text, \
                      tenant_id, message_ts, timestamp) \
                     SELECT $1, $2, $3, $4, 'user', $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                            COALESCE(to_timestamp($13), now()) \
                     WHERE NOT EXISTS (SELECT 1 FROM groups.group_messages \
                       WHERE group_id = $1 AND tg_msg_id = $4 AND tenant_id = $12)",
                )
                .bind(chat_id)
                .bind(user_id)
                .bind(username.as_deref())
                .bind(message_id)
                .bind(inbound.content.as_deref())
                .bind(&inbound.msg_type)
                .bind(inbound.file_id.as_deref())
                .bind(inbound.media_unique_id.as_deref())
                .bind(ad_text_hash.as_deref())
                .bind(reply_to_message_id)
                .bind(quoted_text)
                .bind(&entry.tenant_id)
                .bind(message_ts)
                .execute(pool)
                .await
                .map_err(|_| "failed to persist group message".to_string())?;
            }
        }
        _ => {}
    }
    Ok(())
}

async fn claim_or_refresh_group(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    group_id: i64,
    group_title: &str,
) -> Result<Option<TrackedGroupPolicy>, String> {
    let row = sqlx::query(
        "INSERT INTO groups.tracked_groups (group_id, group_title, tenant_id, first_seen, updated_at) \
         VALUES ($1, $2, $3, now(), now()) \
         ON CONFLICT (group_id) DO UPDATE SET group_title = EXCLUDED.group_title, updated_at = now() \
         WHERE groups.tracked_groups.tenant_id = EXCLUDED.tenant_id \
         RETURNING is_monitored",
    )
    .bind(group_id)
    .bind(group_title)
    .bind(&entry.tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "failed to refresh tracked group".to_string())?;
    let Some(row) = row else {
        debug!(
            group_id,
            tenant_id = %entry.tenant_id,
            "Ignoring Telegram update for a group owned by another tenant"
        );
        return Ok(None);
    };
    Ok(Some(TrackedGroupPolicy {
        is_monitored: row
            .try_get("is_monitored")
            .map_err(|_| "tracked group has invalid monitor state".to_string())?,
    }))
}

async fn upsert_chat_user(
    pool: &sqlx::PgPool,
    user_id: i64,
    username: Option<&str>,
    first_name: Option<&str>,
    last_name: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO chat.users (user_id, username, first_name, last_name) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (user_id) DO UPDATE SET username = EXCLUDED.username, \
         first_name = EXCLUDED.first_name, last_name = EXCLUDED.last_name, updated_at = now()",
    )
    .bind(user_id)
    .bind(username)
    .bind(first_name)
    .bind(last_name)
    .execute(pool)
    .await
    .map_err(|_| "failed to persist Telegram user".to_string())?;
    Ok(())
}

async fn upsert_group_member(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    group_id: i64,
    group_title: &str,
    user_id: i64,
    username: Option<&str>,
    first_name: Option<&str>,
    last_name: Option<&str>,
    status: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO groups.group_members \
         (user_id, username, first_name, last_name, group_id, group_title, tenant_id, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (group_id, user_id) DO UPDATE SET username = EXCLUDED.username, \
         first_name = EXCLUDED.first_name, last_name = EXCLUDED.last_name, \
         group_title = EXCLUDED.group_title, status = EXCLUDED.status, updated_at = now() \
         WHERE groups.group_members.tenant_id = EXCLUDED.tenant_id",
    )
    .bind(user_id)
    .bind(username)
    .bind(first_name)
    .bind(last_name)
    .bind(group_id)
    .bind(group_title)
    .bind(tenant_id)
    .bind(status)
    .execute(pool)
    .await
    .map_err(|_| "failed to persist group member".to_string())?;
    Ok(())
}

async fn apply_message_membership_events(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    message: &Value,
    group_id: i64,
    group_title: &str,
    telegram_api_base_url: &str,
) -> Result<(), String> {
    if let Some(users) = message.get("new_chat_members").and_then(Value::as_array) {
        for user in users {
            let Some(user_id) = user
                .get("id")
                .and_then(Value::as_i64)
                .filter(|value| *value > 0)
            else {
                continue;
            };
            if user_id == entry.bot_id {
                continue;
            }
            upsert_group_member(
                pool,
                &entry.tenant_id,
                group_id,
                group_title,
                user_id,
                optional_nonempty_string(user, "username").as_deref(),
                optional_nonempty_string(user, "first_name").as_deref(),
                optional_nonempty_string(user, "last_name").as_deref(),
                "active",
            )
            .await?;
            // P3 入群验证：新成员触发算术验证（math），通过后解除静音
            start_group_verify(
                pool,
                entry,
                telegram_api_base_url,
                group_id,
                user_id,
                optional_nonempty_string(user, "first_name")
                    .as_deref()
                    .unwrap_or("新成员"),
            )
            .await?;
        }
    }
    if let Some(user) = message.get("left_chat_member") {
        if let Some(user_id) = user
            .get("id")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
        {
            clear_verify_state(group_id, user_id);
            sqlx::query(
                "UPDATE groups.group_members SET status = 'left', updated_at = now() \
                 WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
            )
            .bind(group_id)
            .bind(user_id)
            .bind(&entry.tenant_id)
            .execute(pool)
            .await
            .map_err(|_| "failed to persist departing group member".to_string())?;
        }
    }
    Ok(())
}

async fn handle_chat_join_request(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    request: &Value,
) -> Result<(), String> {
    let chat = request
        .get("chat")
        .ok_or_else(|| "join request has no chat".to_string())?;
    let group_id = chat
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "join request has no group id".to_string())?;
    let group_title = optional_nonempty_string(chat, "title").unwrap_or_else(|| "Unknown".into());
    if claim_or_refresh_group(pool, entry, group_id, &group_title)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let user = request
        .get("from")
        .ok_or_else(|| "join request has no user".to_string())?;
    let user_id = user
        .get("id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "join request has invalid user".to_string())?;
    upsert_group_member(
        pool,
        &entry.tenant_id,
        group_id,
        &group_title,
        user_id,
        optional_nonempty_string(user, "username").as_deref(),
        optional_nonempty_string(user, "first_name").as_deref(),
        optional_nonempty_string(user, "last_name").as_deref(),
        "pending",
    )
    .await
}

async fn handle_chat_member_update(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    update: &Value,
    is_bot_update: bool,
) -> Result<(), String> {
    let chat = update
        .get("chat")
        .ok_or_else(|| "member update has no chat".to_string())?;
    let group_id = chat
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "member update has no group id".to_string())?;
    let group_title = optional_nonempty_string(chat, "title").unwrap_or_else(|| "Unknown".into());
    let member = update
        .get("new_chat_member")
        .ok_or_else(|| "member update has no new state".to_string())?;
    let telegram_status = member
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("left");
    if is_bot_update {
        if claim_or_refresh_group(pool, entry, group_id, &group_title)
            .await?
            .is_none()
        {
            return Ok(());
        }
        if matches!(telegram_status, "left" | "kicked") {
            sqlx::query(
                "UPDATE groups.tracked_groups SET is_monitored = false, updated_at = now() \
                 WHERE group_id = $1 AND tenant_id = $2",
            )
            .bind(group_id)
            .bind(&entry.tenant_id)
            .execute(pool)
            .await
            .map_err(|_| "failed to disable departed group".to_string())?;
        }
        return Ok(());
    }
    if claim_or_refresh_group(pool, entry, group_id, &group_title)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let user = member
        .get("user")
        .ok_or_else(|| "member update has no user".to_string())?;
    let user_id = user
        .get("id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "member update has invalid user".to_string())?;
    let status = persisted_member_status(member);
    if matches!(status, "left" | "banned") {
        clear_verify_state(group_id, user_id);
    }
    upsert_group_member(
        pool,
        &entry.tenant_id,
        group_id,
        &group_title,
        user_id,
        optional_nonempty_string(user, "username").as_deref(),
        optional_nonempty_string(user, "first_name").as_deref(),
        optional_nonempty_string(user, "last_name").as_deref(),
        status,
    )
    .await
}

fn extract_inbound_content(message: &Value) -> Option<InboundContent> {
    if let Some(text) = message.get("text").and_then(Value::as_str) {
        return Some(InboundContent {
            content: Some(text.to_string()),
            msg_type: "text".into(),
            file_id: None,
            media_unique_id: None,
        });
    }
    let caption = message
        .get("caption")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(photo) = message.get("photo").and_then(Value::as_array) {
        return Some(InboundContent {
            content: caption,
            msg_type: "photo".into(),
            file_id: photo
                .iter()
                .rev()
                .find_map(|item| item.get("file_id").and_then(Value::as_str))
                .map(str::to_string),
            media_unique_id: photo
                .iter()
                .rev()
                .find_map(|item| item.get("file_unique_id").and_then(Value::as_str))
                .map(str::to_string),
        });
    }
    for (field, msg_type) in [
        ("document", "document"),
        ("video", "video"),
        ("audio", "audio"),
        ("voice", "voice"),
        ("animation", "animation"),
        ("sticker", "sticker"),
    ] {
        if let Some(media) = message.get(field) {
            return Some(InboundContent {
                content: caption.clone().or_else(|| {
                    media
                        .get("emoji")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                }),
                msg_type: msg_type.into(),
                file_id: media
                    .get("file_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                media_unique_id: media
                    .get("file_unique_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
    None
}

fn persisted_member_status(member: &Value) -> &'static str {
    match member.get("status").and_then(Value::as_str) {
        Some("creator" | "administrator") => "admin",
        Some("member") => "active",
        Some("restricted")
            if member
                .get("is_member")
                .and_then(Value::as_bool)
                .unwrap_or(false) =>
        {
            "active"
        }
        Some("kicked") => "banned",
        _ => "left",
    }
}

fn require_webhook_pool(pool: Option<&sqlx::PgPool>) -> Result<&sqlx::PgPool, String> {
    pool.ok_or_else(|| "webhook persistence unavailable".to_string())
}

fn optional_nonempty_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WalletCallback {
    List,
    AdminList,
    Add,
    BackToStart,
    Deactivate(i64),
    AdminDeactivate(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalletCallbackMode {
    User,
    Admin,
}

fn parse_wallet_callback(data: &str) -> Option<WalletCallback> {
    match data {
        "wallet_my_list" => return Some(WalletCallback::List),
        "wallet_admin_all" => return Some(WalletCallback::AdminList),
        "wallet_monitor" => return Some(WalletCallback::Add),
        "back_to_start" => return Some(WalletCallback::BackToStart),
        _ => {}
    }
    let (mode, monitor_id) = if let Some(monitor_id) = data.strip_prefix("uw_u_") {
        (WalletCallbackMode::User, monitor_id)
    } else if let Some(monitor_id) = data.strip_prefix("uw_a_") {
        (WalletCallbackMode::Admin, monitor_id)
    } else {
        return None;
    };
    if monitor_id.is_empty() || !monitor_id.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    monitor_id
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .map(|monitor_id| match mode {
            WalletCallbackMode::User => WalletCallback::Deactivate(monitor_id),
            WalletCallbackMode::Admin => WalletCallback::AdminDeactivate(monitor_id),
        })
}

/// 控制台菜单回调：主菜单 / Web 管理后台入口 / 商户控制台入口 / 联系客服
///
/// 对齐 Python handlers/start.py 的卡片菜单迁移：
/// - back_to_start     → 重建完整主菜单（管理员/商户/用户角色区分）
/// - admin_web_otp     → 生成 Web 管理后台入口（web_app + 浏览器兜底 OTP）
/// - merchant_web_otp  → 生成商户控制台入口（无商户则提示未绑定）
/// - contact_admin     → 客服说明
async fn handle_console_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: Option<&sqlx::PgPool>,
    telegram_api_base_url: &str,
    grant_store: &GrantStore,
    otp_store: &OtpStore,
    session_secret: &str,
    frontend_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    let action = match data {
        "back_to_start" | "admin_web_otp" | "merchant_web_otp" | "contact_admin" => data,
        _ => return Ok(false),
    };
    let Some(pool) = pool else {
        return Ok(false);
    };
    let Some(callback_id) = callback.get("id").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(user_id) = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
    else {
        return Ok(false);
    };
    let message = callback.get("message");
    let callback_chat_id = message
        .and_then(|message| message.get("chat"))
        .and_then(|chat| chat.get("id"))
        .and_then(Value::as_i64);
    let message_id = message
        .and_then(|message| message.get("message_id"))
        .and_then(Value::as_i64);
    if callback_chat_id != Some(user_id) {
        answer_console_callback(
            telegram_api_base_url,
            entry,
            callback_id,
            "请在私聊中操作",
            true,
        )
        .await?;
        return Ok(true);
    }
    let Some(message_id) = message_id else {
        answer_console_callback(telegram_api_base_url, entry, callback_id, "请求无效", true)
            .await?;
        return Ok(true);
    };
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: telegram_api_base_url.to_string(),
        timeout_secs: 30,
    })
    .map_err(|_| "failed to initialize Telegram client".to_string())?;
    let first_name = callback
        .get("from")
        .and_then(|from| from.get("first_name"))
        .and_then(Value::as_str)
        .unwrap_or("朋友");
    let locale = telegram_locale(pool, &entry.tenant_id, user_id)
        .await
        .unwrap_or_else(|_| "zh-CN".to_string());

    let (text, reply_markup) = match action {
        "back_to_start" => {
            let (is_platform_admin, has_merchant) = user_console_roles(pool, user_id)
                .await
                .map_err(|error| error.to_string())?;
            build_full_start_menu(is_platform_admin, has_merchant, first_name, user_id, &locale)
        }
        "admin_web_otp" => {
            let grant = uuid::Uuid::new_v4().simple().to_string();
            grant_store
                .insert(
                    grant.clone(),
                    GrantData {
                        grant_hash: String::new(),
                        issued_for: user_id.to_string(),
                        role_hint: "platform_admin".to_string(),
                        tenant_id: String::new(),
                        merchant_id: None,
                        entry_intent: "admin_console".to_string(),
                        expires_at: String::new(),
                        ua_fingerprint: String::new(),
                        proof_salt: String::new(),
                    },
                )
                .await;
            let (otp, otp_data) = make_console_otp(
                session_secret,
                "platform_admin",
                "",
                None,
                &user_id.to_string(),
                300.0,
            );
            otp_store.insert(otp.clone(), otp_data).await;
            let (miniapp_url, browser_url) =
                console_entry_urls(frontend_url, "admin_console", &grant, Some(&otp), None, None);
            build_console_entry_card(&locale, &miniapp_url, &browser_url, false)
        }
        "merchant_web_otp" => {
            let merchant = merchant_console_by_owner(pool, user_id)
                .await
                .map_err(|error| error.to_string())?;
            match merchant {
                Some((tenant_id, legacy_merchant_id)) => {
                    let grant = uuid::Uuid::new_v4().simple().to_string();
                    grant_store
                        .insert(
                            grant.clone(),
                            GrantData {
                                grant_hash: String::new(),
                                issued_for: user_id.to_string(),
                                role_hint: "merchant_owner".to_string(),
                                tenant_id: tenant_id.clone(),
                                merchant_id: legacy_merchant_id,
                                entry_intent: "merchant_console".to_string(),
                                expires_at: String::new(),
                                ua_fingerprint: String::new(),
                                proof_salt: String::new(),
                            },
                        )
                        .await;
                    let (otp, otp_data) = make_console_otp(
                        session_secret,
                        "merchant_owner",
                        &tenant_id,
                        legacy_merchant_id,
                        &user_id.to_string(),
                        300.0,
                    );
                    otp_store.insert(otp.clone(), otp_data).await;
                    let (miniapp_url, browser_url) = console_entry_urls(
                        frontend_url,
                        "merchant_console",
                        &grant,
                        Some(&otp),
                        Some(&tenant_id),
                        legacy_merchant_id,
                    );
                    build_console_entry_card(&locale, &miniapp_url, &browser_url, true)
                }
                None => (
                    "⚠️ 当前 Telegram 账号未绑定商户".to_string(),
                    Some(json!({
                        "inline_keyboard": [[{
                            "text": "⬅️ 返回主菜单",
                            "callback_data": "back_to_start"
                        }]]
                    })),
                ),
            }
        }
        "contact_admin" => (
            "🎧 人工客服\n\n请直接在当前私聊窗口发送文字、图片、语音或文件，消息会进入客服双向会话。\n客服回复后，你也会在这里继续收到消息。"
                .to_string(),
            Some(json!({
                "inline_keyboard": [[{
                    "text": "⬅️ 返回主菜单",
                    "callback_data": "back_to_start"
                }]]
            })),
        ),
        _ => unreachable!(),
    };

    match telegram
        .edit_message_text(EditMessageTextParams {
            chat_id: user_id,
            message_id,
            text: text.clone(),
            parse_mode: None,
            reply_markup: reply_markup.clone(),
        })
        .await
    {
        Ok(_) => {}
        Err(edit_error) => {
            // edit 失败（消息已被删/不存在等）→ 发送新消息兜底（对齐 Python）
            warn!(
                bot_id = entry.bot_id,
                error = %edit_error,
                "console callback edit_message_text failed, falling back to send_message"
            );
            if let Err(send_error) = telegram
                .send_message(SendMessageParams {
                    chat_id: user_id,
                    text,
                    parse_mode: None,
                    reply_markup,
                    reply_to_message_id: None,
                })
                .await
            {
                warn!(
                    bot_id = entry.bot_id,
                    error = %send_error,
                    "console callback send_message fallback failed"
                );
            }
        }
    }
    answer_console_callback(telegram_api_base_url, entry, callback_id, "", false).await?;
    Ok(true)
}

/// 生成控制台入口 OTP（含预签发的 session token），返回 (otp, otp_data)
fn make_console_otp(
    session_secret: &str,
    role: &str,
    tenant_id: &str,
    merchant_id: Option<i64>,
    issued_for: &str,
    ttl_secs: f64,
) -> (String, OtpData) {
    let otp = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let token = issue_session_token(
        session_secret,
        role,
        tenant_id,
        merchant_id,
        Some(issued_for),
        24 * 3600,
    )
    .unwrap_or_default();
    let expire_at = chrono::Utc::now().timestamp() as f64 + ttl_secs;
    (
        otp,
        OtpData {
            token,
            expire_at,
            role: role.to_string(),
            tenant_id: tenant_id.to_string(),
            merchant_id,
            issued_for: Some(issued_for.to_string()),
        },
    )
}

async fn answer_console_callback(
    telegram_api_base_url: &str,
    entry: &BotEntry,
    callback_id: &str,
    text: &str,
    show_alert: bool,
) -> Result<(), String> {
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: telegram_api_base_url.to_string(),
        timeout_secs: 30,
    })
    .map_err(|_| "failed to initialize Telegram client".to_string())?;
    telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: (!text.is_empty()).then(|| text.to_string()),
            show_alert: Some(show_alert),
        })
        .await
        .map_err(|_| "Telegram console callback acknowledgement failed".to_string())
}

/// 客服系统回调：cs_mark_read / cs_quick_reply / cs_close_session
/// + 批量补发（quick_send / select_other_user / cancel_send / user_page / confirm_send_to）。
/// （Python cs_callback_handler 的 Rust 版本）。
async fn handle_customer_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: Option<&sqlx::PgPool>,
    telegram_api_base_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(pool) = pool else {
        return Ok(false);
    };

    // 批量补发回调（管理员操作，内部校验权限；前缀不匹配时返回 Ok(false)）
    if handle_pending_send_callback(update, entry, pool, telegram_api_base_url).await? {
        return Ok(true);
    }

    let (action, user_id_str) = if let Some(rest) = data.strip_prefix("cs_mark_read:") {
        ("mark_read", rest)
    } else if let Some(rest) = data.strip_prefix("cs_quick_reply:") {
        ("quick_reply", rest)
    } else if let Some(rest) = data.strip_prefix("cs_close_session:") {
        ("close_session", rest)
    } else {
        return Ok(false);
    };
    let Ok(user_id) = user_id_str.parse::<i64>() else {
        return Ok(false);
    };
    let Some(callback_id) = callback.get("id").and_then(Value::as_str) else {
        return Ok(false);
    };

    // P1-2：cs_* 均为管理员操作，统一校验租户 owner/admin（对齐 Python cs_callback_handler 入口 ADMIN_TG_IDS 检查）
    let Some(actor) = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
    else {
        return Ok(false);
    };
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let is_admin = is_tenant_admin(pool, &entry.tenant_id, actor).await?;
    if !is_admin {
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("权限不足".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    }

    let (ok_text, err_text) = match action {
        "mark_read" => {
            sqlx::query(
                "UPDATE chat.messages SET is_read = true \
                 WHERE from_userid = $1 AND tenant_id = $2 AND is_read = false",
            )
            .bind(user_id)
            .bind(&entry.tenant_id)
            .execute(pool)
            .await
            .map_err(|_| "failed to mark read".to_string())?;
            sqlx::query(
                "UPDATE chat.admin_inbox SET unread_count = 0, updated_at = now() \
                 WHERE user_id = $1 AND tenant_id = $2",
            )
            .bind(user_id)
            .bind(&entry.tenant_id)
            .execute(pool)
            .await
            .map_err(|_| "failed to reset inbox unread".to_string())?;
            ("已标记为已读 ✅", "标记已读失败")
        }
        "quick_reply" => ("请直接回复这条消息 👆", "操作失败"),
        "close_session" => {
            sqlx::query(
                "UPDATE chat.admin_inbox SET unread_count = 0, updated_at = now() \
                 WHERE user_id = $1 AND tenant_id = $2",
            )
            .bind(user_id)
            .bind(&entry.tenant_id)
            .execute(pool)
            .await
            .map_err(|_| "failed to close session".to_string())?;
            ("会话已结束 ✅", "结束会话失败")
        }
        _ => return Ok(false),
    };

    telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: Some(ok_text.to_string()),
            show_alert: Some(false),
        })
        .await
        .map_err(|_| err_text.to_string())?;
    info!(bot_id = entry.bot_id, action, user_id, "Customer service callback handled");
    Ok(true)
}

// ---------------------------------------------------------------------------
// 批量补发（pending_admin_messages）辅助函数
// ---------------------------------------------------------------------------

/// 待确认管理员消息行（对齐 Python pending_admin_messages 表结构）。
struct PendingAdminMessage {
    msg_uuid: String,
    admin_id: i64,
    content: Option<String>,
    msg_type: String,
    file_id: Option<String>,
    target_user_id: Option<i64>,
    status: String,
    /// 入队时间（quick_send 批次路由按 timestamp 取 <= 当前的消息）
    timestamp: chrono::DateTime<chrono::Utc>,
}

/// 校验 callback 发起者是否为租户 owner/admin（对齐 Python ADMIN_TG_IDS 权限校验）。
async fn is_tenant_admin(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    user_id: i64,
) -> Result<bool, String> {
    let is_admin: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM merchants
             WHERE tenant_id = $1 AND owner_tg_user_id = $2
         ) OR EXISTS (
             SELECT 1
             FROM tenant_memberships membership
             JOIN auth.users user_record ON user_record.id = membership.user_id
             WHERE membership.tenant_id = $1
               AND user_record.tg_user_id = $2
               AND membership.role IN ('merchant_owner', 'merchant_admin')
         )",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|_| "failed to verify admin permission".to_string())?;
    Ok(is_admin)
}

/// 写入一条待确认管理员消息，返回 msg_uuid（30 分钟过期，对齐 Python）。
/// P2-8：msg_uuid 截断为 8 位（对齐 Python `str(uuid.uuid4())[:8]`），
/// 避免 confirm_send_to/quick_send 等 callback_data 超过 Telegram 64 字节上限。
async fn insert_pending_admin_message(
    pool: &sqlx::PgPool,
    admin_id: i64,
    content: &str,
    msg_type: &str,
    file_id: Option<&str>,
    target_user_id: Option<i64>,
) -> Result<String, String> {
    let msg_uuid = uuid::Uuid::new_v4().as_simple().to_string()[..8].to_string();
    sqlx::query(
        "INSERT INTO platform.pending_admin_messages \
         (msg_uuid, admin_id, content, msg_type, file_id, target_user_id, status, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, 'pending', now() + interval '30 minutes')",
    )
    .bind(&msg_uuid)
    .bind(admin_id)
    .bind(content)
    .bind(msg_type)
    .bind(file_id)
    .bind(target_user_id)
    .execute(pool)
    .await
    .map_err(|_| "failed to persist pending admin message".to_string())?;
    Ok(msg_uuid)
}

/// 待确认消息是否过期（30 分钟，对齐 Python is_pending_message_expired）。
async fn pending_message_expired(pool: &sqlx::PgPool, msg_uuid: &str) -> Result<bool, String> {
    let expired: bool = sqlx::query_scalar(
        "SELECT COALESCE(expires_at <= now(), false) FROM platform.pending_admin_messages \
         WHERE msg_uuid = $1",
    )
    .bind(msg_uuid)
    .fetch_one(pool)
    .await
    .map_err(|_| "failed to check pending message expiry".to_string())?;
    Ok(expired)
}

/// 查询待确认管理员消息。
async fn fetch_pending_admin_message(
    pool: &sqlx::PgPool,
    msg_uuid: &str,
) -> Result<Option<PendingAdminMessage>, String> {
    let row = sqlx::query(
        "SELECT msg_uuid, admin_id, content, msg_type, file_id, target_user_id, status, timestamp \
         FROM platform.pending_admin_messages WHERE msg_uuid = $1",
    )
    .bind(msg_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|_| "failed to load pending admin message".to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(PendingAdminMessage {
        msg_uuid: row.try_get("msg_uuid").map_err(|_| "invalid pending msg_uuid".to_string())?,
        admin_id: row.try_get("admin_id").unwrap_or(0),
        content: row.try_get("content").ok().flatten(),
        msg_type: row.try_get("msg_type").unwrap_or_else(|_| "text".to_string()),
        file_id: row.try_get("file_id").ok().flatten(),
        target_user_id: row.try_get("target_user_id").ok().flatten(),
        status: row.try_get("status").unwrap_or_else(|_| "pending".to_string()),
        timestamp: row.try_get("timestamp").unwrap_or_else(|_| chrono::Utc::now()),
    }))
}

/// 查询某管理员名下全部待确认消息（quick_send 批次，timestamp <= 当前，对齐 Python）。
async fn fetch_pending_admin_batch(
    pool: &sqlx::PgPool,
    admin_id: i64,
    up_to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<PendingAdminMessage>, String> {
    let rows = sqlx::query(
        "SELECT msg_uuid, admin_id, content, msg_type, file_id, target_user_id, status, timestamp \
         FROM platform.pending_admin_messages \
         WHERE admin_id = $1 AND status = 'pending' AND timestamp <= $2 \
         ORDER BY timestamp ASC",
    )
    .bind(admin_id)
    .bind(up_to)
    .fetch_all(pool)
    .await
    .map_err(|_| "failed to load pending admin batch".to_string())?;
    let mut batch = Vec::new();
    for row in rows {
        batch.push(PendingAdminMessage {
            msg_uuid: row.try_get("msg_uuid").map_err(|_| "invalid pending msg_uuid".to_string())?,
            admin_id: row.try_get("admin_id").unwrap_or(0),
            content: row.try_get("content").ok().flatten(),
            msg_type: row.try_get("msg_type").unwrap_or_else(|_| "text".to_string()),
            file_id: row.try_get("file_id").ok().flatten(),
            target_user_id: row.try_get("target_user_id").ok().flatten(),
            status: row.try_get("status").unwrap_or_else(|_| "pending".to_string()),
            timestamp: row.try_get("timestamp").unwrap_or_else(|_| chrono::Utc::now()),
        });
    }
    Ok(batch)
}

/// 发送待确认消息给目标用户（带客服身份标签 + 入库 + 标记 sent）。
async fn send_pending_message_to_user(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    telegram_api_base_url: &str,
    admin_id: i64,
    admin_name: &str,
    target_user: i64,
    pending: &PendingAdminMessage,
) -> Result<(), String> {
    let telegram = telegram_for_bot(entry, telegram_api_base_url);
    let esc_name = html_escape(admin_name);
    let body = pending.content.as_deref().unwrap_or("");
    let esc_body = html_escape(body);
    let text = format!("客服 <a href=\"tg://user?id={admin_id}\">{esc_name}</a>：\n{esc_body}");

    let sent = if pending.msg_type == "photo" && pending.file_id.is_some() {
        telegram
            .send_photo(tgbot_integrations::telegram::SendPhotoParams {
                chat_id: target_user,
                photo: pending.file_id.clone().unwrap_or_default(),
                caption: Some(text),
                parse_mode: Some("HTML".to_string()),
                reply_to_message_id: None,
                reply_markup: None,
            })
            .await
            .map_err(|_| "failed to send pending photo".to_string())?
    } else if pending.msg_type == "document" && pending.file_id.is_some() {
        telegram
            .send_document(tgbot_integrations::telegram::SendDocumentParams {
                chat_id: target_user,
                document: pending.file_id.clone().unwrap_or_default(),
                caption: Some(text),
                parse_mode: Some("HTML".to_string()),
            })
            .await
            .map_err(|_| "failed to send pending document".to_string())?
    } else {
        telegram
            .send_message(tgbot_integrations::telegram::SendMessageParams {
                chat_id: target_user,
                text,
                parse_mode: Some("HTML".to_string()),
                reply_markup: None,
                reply_to_message_id: None,
            })
            .await
            .map_err(|_| "failed to send pending admin message".to_string())?
    };

    message_repo::save_message(
        pool,
        entry.bot_id,
        target_user,
        Some(body),
        &pending.msg_type,
        pending.file_id.as_deref(),
        None,
        true,
        Some(sent.message_id),
        &entry.tenant_id,
        None,
    )
    .await
    .map_err(|_| "failed to persist pending admin message".to_string())?;

    sqlx::query(
        "UPDATE platform.pending_admin_messages \
         SET status = 'sent', target_user_id = $2, expires_at = NULL, timestamp = now() \
         WHERE msg_uuid = $1",
    )
    .bind(&pending.msg_uuid)
    .bind(target_user)
    .execute(pool)
    .await
    .map_err(|_| "failed to mark pending message sent".to_string())?;
    Ok(())
}

/// 租户活跃用户列表（对齐 Python get_inbox_list，按最近活跃排序）。
/// P2-9：仅取 7 天内有活跃的用户（对齐 Python get_inbox_list 的 7 天窗口）。
async fn active_inbox_users(
    pool: &sqlx::PgPool,
    tenant_id: &str,
) -> Result<Vec<(i64, Option<String>)>, String> {
    let rows = sqlx::query(
        "SELECT user_id, username FROM chat.admin_inbox \
         WHERE tenant_id = $1 AND last_active >= now() - interval '7 days' \
         ORDER BY last_active DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .map_err(|_| "failed to load active users".to_string())?;
    let mut users = Vec::new();
    for row in rows {
        let user_id: i64 = row.try_get("user_id").unwrap_or(0);
        let username: Option<String> = row.try_get("username").ok().flatten();
        if user_id > 0 {
            users.push((user_id, username));
        }
    }
    Ok(users)
}

/// 管理员最近一次「选择其他用户」对应的 msg_uuid（user_page 分页需要，前缀不含 uuid）。
static PENDING_SEND_CONTEXT: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<i64, String>>,
> = std::sync::OnceLock::new();

fn pending_send_context() -> &'static std::sync::Mutex<std::collections::HashMap<i64, String>> {
    PENDING_SEND_CONTEXT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 批量补发回调：quick_send / select_other_user / cancel_send / user_page / confirm_send_to。
/// 前缀不匹配返回 Ok(false)；命中后校验管理员权限并返回 Ok(true)。
async fn handle_pending_send_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: &sqlx::PgPool,
    telegram_api_base_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    if !(data.starts_with("quick_send:")
        || data.starts_with("select_other_user:")
        || data.starts_with("cancel_send:")
        || data.starts_with("user_page:")
        || data.starts_with("confirm_send_to:"))
    {
        return Ok(false);
    }
    let Some(callback_id) = callback.get("id").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(actor) = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
    else {
        return Ok(false);
    };
    let admin_name = callback
        .get("from")
        .and_then(|from| from.get("first_name"))
        .and_then(Value::as_str)
        .unwrap_or("客服");
    let message_id = callback
        .get("message")
        .and_then(|message| message.get("message_id"))
        .and_then(Value::as_i64);
    let telegram = telegram_for_bot(entry, telegram_api_base_url);

    // 管理员操作权限校验（对齐 Python ADMIN_TG_IDS 检查）
    let is_admin = is_tenant_admin(pool, &entry.tenant_id, actor).await?;
    if !is_admin {
        let _ = telegram
            .answer_callback_query(AnswerCallbackQueryParams {
                callback_query_id: callback_id.to_string(),
                text: Some("权限不足".to_string()),
                show_alert: Some(true),
            })
            .await;
        return Ok(true);
    }

    // quick_send:{target_id}:{msg_uuid}：把该管理员名下所有 pending（timestamp <= 当前）
    // 的消息按批次发给 target，逐条 status → sent（对齐 Python customer_service.py:358-468）。
    if let Some(rest) = data.strip_prefix("quick_send:") {
        let mut parts = rest.splitn(2, ':');
        let (Some(target_str), Some(msg_uuid)) = (parts.next(), parts.next()) else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("❌ 消息索引丢失".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        };
        let Ok(target) = target_str.parse::<i64>() else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("❌ 目标用户无效".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        };
        if target <= 0 || msg_uuid.is_empty() {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("❌ 参数无效".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        }
        let Some(pending) = fetch_pending_admin_message(pool, msg_uuid).await? else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("⚠️ 消息已处理或不存在".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        };
        if pending.status != "pending" {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("⚠️ 消息已处理或不存在".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        }
        // 过期检查（对齐 Python is_pending_message_expired）
        if pending_message_expired(pool, msg_uuid).await? {
            let _ = sqlx::query(
                "UPDATE platform.pending_admin_messages SET status = 'expired' \
                 WHERE msg_uuid = $1",
            )
            .bind(msg_uuid)
            .execute(pool)
            .await;
            if let Some(message_id) = message_id {
                let _ = telegram
                    .edit_message_text(EditMessageTextParams {
                        chat_id: actor,
                        message_id,
                        text: "⏰ 这条待发送消息已超过30分钟，默认失效。".to_string(),
                        parse_mode: None,
                        reply_markup: None,
                    })
                    .await;
            }
            return Ok(true);
        }
        // 批次：该管理员名下所有早于等于当前消息的 pending（对齐 Python pending_batch）
        let batch = fetch_pending_admin_batch(pool, actor, pending.timestamp).await?;
        if batch.is_empty() {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("⚠️ 批次已完成或已失效".to_string()),
                    show_alert: Some(true),
                })
                .await;
            if let Some(message_id) = message_id {
                let _ = telegram
                    .edit_message_text(EditMessageTextParams {
                        chat_id: actor,
                        message_id,
                        text: "✅ 所有待处理消息已发送完毕。".to_string(),
                        parse_mode: None,
                        reply_markup: None,
                    })
                    .await;
            }
            return Ok(true);
        }
        let mut sent_count = 0_usize;
        let mut first_error: Option<String> = None;
        for item in &batch {
            match send_pending_message_to_user(
                pool,
                entry,
                telegram_api_base_url,
                actor,
                admin_name,
                target,
                item,
            )
            .await
            {
                Ok(()) => sent_count += 1,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if sent_count > 0 {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some(format!("✅ 成功补发 {sent_count} 条消息")),
                    show_alert: Some(false),
                })
                .await;
            if let Some(message_id) = message_id {
                let _ = telegram
                    .edit_message_text(EditMessageTextParams {
                        chat_id: actor,
                        message_id,
                        text: format!("✅ <b>成功补发 {sent_count} 条消息</b>\n目标用户：<code>{target}</code>"),
                        parse_mode: Some("HTML".to_string()),
                        reply_markup: None,
                    })
                    .await;
            }
        } else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some(format!("❌ 发送失败: {}", first_error.unwrap_or_else(|| "未知错误".into()))),
                    show_alert: Some(true),
                })
                .await;
        }
        return Ok(true);
    }

    // select_other_user:{msg_uuid}：列出活跃用户（分页 user_page）
    if let Some(msg_uuid) = data.strip_prefix("select_other_user:") {
        if msg_uuid.is_empty() {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("❌ 索引丢失".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        }
        let users = active_inbox_users(pool, &entry.tenant_id).await?;
        if users.is_empty() {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("暂无活跃用户".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        }
        // 记录该管理员当前操作的 msg_uuid（user_page 分页时使用）
        if let Ok(mut context) = pending_send_context().lock() {
            context.insert(actor, msg_uuid.to_string());
        }
        let keyboard = build_user_page_keyboard(&users, 0, msg_uuid);
        let _ = telegram
            .edit_message_text(EditMessageTextParams {
                chat_id: actor,
                message_id: message_id.unwrap_or(0),
                text: "请选择要补发的目标用户：".to_string(),
                parse_mode: None,
                reply_markup: Some(keyboard),
            })
            .await;
        return Ok(true);
    }

    // cancel_send:{msg_uuid}：取消
    if let Some(msg_uuid) = data.strip_prefix("cancel_send:") {
        if !msg_uuid.is_empty() {
            // P2-9：对齐 Python update_pending_admin_msg_status(msg_uuid, 'expired')，
            // 统一用 'expired' 表示已失效，避免后续 quick_send 再次命中
            let _ = sqlx::query(
                "UPDATE platform.pending_admin_messages SET status = 'expired' \
                 WHERE msg_uuid = $1 AND status = 'pending'",
            )
            .bind(msg_uuid)
            .execute(pool)
            .await;
        }
        if let Some(message_id) = message_id {
            let _ = telegram
                .edit_message_text(EditMessageTextParams {
                    chat_id: actor,
                    message_id,
                    text: "🚫 已取消发送。提示：回复用户请尽量使用“左滑引用”。".to_string(),
                    parse_mode: None,
                    reply_markup: None,
                })
                .await;
        }
        return Ok(true);
    }

    // user_page:{page}：分页用户列表（msg_uuid 取该管理员最近一次 select_other_user）
    if let Some(page_str) = data.strip_prefix("user_page:") {
        let Ok(page) = page_str.parse::<i64>().map(|value| value.max(0)) else {
            return Ok(false);
        };
        let msg_uuid = pending_send_context()
            .lock()
            .map(|context| context.get(&actor).cloned())
            .ok()
            .flatten();
        let Some(msg_uuid) = msg_uuid else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("❌ 索引丢失，请重新选择".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        };
        let users = active_inbox_users(pool, &entry.tenant_id).await?;
        if users.is_empty() {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("暂无活跃用户".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        }
        let keyboard = build_user_page_keyboard(&users, page, &msg_uuid);
        let _ = telegram
            .edit_message_text(EditMessageTextParams {
                chat_id: actor,
                message_id: message_id.unwrap_or(0),
                text: format!("请选择要补发的目标用户（第 {} 页）：", page + 1),
                parse_mode: None,
                reply_markup: Some(keyboard),
            })
            .await;
        return Ok(true);
    }

    // confirm_send_to:{user_id}:{msg_uuid}：确认发给指定用户
    if let Some(rest) = data.strip_prefix("confirm_send_to:") {
        let mut parts = rest.splitn(2, ':');
        let (Some(target_str), Some(msg_uuid)) = (parts.next(), parts.next()) else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("❌ 消息索引丢失".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        };
        let Ok(target) = target_str.parse::<i64>() else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("❌ 目标用户无效".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        };
        let Some(pending) = fetch_pending_admin_message(pool, msg_uuid).await? else {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("⚠️ 消息已处理或不存在".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        };
        if pending.status != "pending" {
            let _ = telegram
                .answer_callback_query(AnswerCallbackQueryParams {
                    callback_query_id: callback_id.to_string(),
                    text: Some("⚠️ 消息已处理或不存在".to_string()),
                    show_alert: Some(true),
                })
                .await;
            return Ok(true);
        }
        // P1-4：confirm_send_to 补 30 分钟过期检查（对齐 Python py:1424-1428）
        if pending_message_expired(pool, msg_uuid).await? {
            let _ = sqlx::query(
                "UPDATE platform.pending_admin_messages SET status = 'expired' \
                 WHERE msg_uuid = $1",
            )
            .bind(msg_uuid)
            .execute(pool)
            .await;
            if let Some(message_id) = message_id {
                let _ = telegram
                    .edit_message_text(EditMessageTextParams {
                        chat_id: actor,
                        message_id,
                        text: "⏰ 这条待发送消息已超过30分钟，默认失效。".to_string(),
                        parse_mode: None,
                        reply_markup: None,
                    })
                    .await;
            }
            return Ok(true);
        }
        match send_pending_message_to_user(
            pool,
            entry,
            telegram_api_base_url,
            actor,
            admin_name,
            target,
            &pending,
        )
        .await
        {
            Ok(()) => {
                let _ = telegram
                    .answer_callback_query(AnswerCallbackQueryParams {
                        callback_query_id: callback_id.to_string(),
                        text: Some("✅ 已发送".to_string()),
                        show_alert: Some(false),
                    })
                    .await;
                if let Some(message_id) = message_id {
                    let _ = telegram
                        .edit_message_text(EditMessageTextParams {
                            chat_id: actor,
                            message_id,
                            text: format!("✅ 已发送给用户 <code>{target}</code>", target = target),
                            parse_mode: Some("HTML".to_string()),
                            reply_markup: None,
                        })
                        .await;
                }
            }
            Err(error) => {
                let _ = telegram
                    .answer_callback_query(AnswerCallbackQueryParams {
                        callback_query_id: callback_id.to_string(),
                        text: Some(format!("❌ 发送失败: {error}")),
                        show_alert: Some(true),
                    })
                    .await;
            }
        }
        return Ok(true);
    }

    Ok(false)
}

/// 构建用户选择键盘（每页最多 10 人；含分页与取消按钮）。
fn build_user_page_keyboard(users: &[(i64, Option<String>)], page: i64, msg_uuid: &str) -> Value {
    const PAGE_SIZE: usize = 10;
    let total_pages = ((users.len() + PAGE_SIZE - 1) / PAGE_SIZE).max(1) as i64;
    let page = page.clamp(0, total_pages - 1);
    let start = (page as usize) * PAGE_SIZE;
    let slice = users.iter().skip(start).take(PAGE_SIZE);

    let mut rows: Vec<Vec<Value>> = Vec::new();
    for (user_id, username) in slice {
        let label = username
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(|name| format!("👤 @{name} (ID: {user_id})"))
            .unwrap_or_else(|| format!("👤 用户 (ID: {user_id})"));
        rows.push(vec![serde_json::json!({
            "text": label,
            "callback_data": format!("confirm_send_to:{user_id}:{msg_uuid}")
        })]);
    }
    let mut nav_row: Vec<Value> = Vec::new();
    if page > 0 {
        nav_row.push(serde_json::json!({
            "text": "⬅️ 上一页",
            "callback_data": format!("user_page:{}", page - 1)
        }));
    }
    if page + 1 < total_pages {
        nav_row.push(serde_json::json!({
            "text": "下一页 ➡️",
            "callback_data": format!("user_page:{}", page + 1)
        }));
    }
    if !nav_row.is_empty() {
        rows.push(nav_row);
    }
    rows.push(vec![serde_json::json!({
        "text": "❌ 取消",
        "callback_data": format!("cancel_send:{msg_uuid}")
    })]);
    serde_json::json!({ "inline_keyboard": rows })
}

async fn handle_wallet_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: Option<&sqlx::PgPool>,
    telegram_api_base_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(action) = parse_wallet_callback(data) else {
        return Ok(false);
    };
    let callback_id = callback
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "wallet callback has no id".to_string())?;
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: telegram_api_base_url.to_string(),
        timeout_secs: 30,
    })
    .map_err(|_| "failed to initialize Telegram client".to_string())?;
    let user_id = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    let message = callback.get("message");
    let callback_chat_id = message
        .and_then(|message| message.get("chat"))
        .and_then(|chat| chat.get("id"))
        .and_then(Value::as_i64);
    let message_id = message
        .and_then(|message| message.get("message_id"))
        .and_then(Value::as_i64);
    let Some(user_id) = user_id else {
        answer_wallet_callback(&telegram, callback_id, "请求无效", true).await?;
        return Ok(true);
    };
    if callback_chat_id != Some(user_id) {
        answer_wallet_callback(&telegram, callback_id, "请在私聊中操作", true).await?;
        return Ok(true);
    }
    let Some(message_id) = message_id else {
        answer_wallet_callback(&telegram, callback_id, "请求无效", true).await?;
        return Ok(true);
    };
    let Some(pool) = pool else {
        answer_wallet_callback(&telegram, callback_id, "服务暂不可用", true).await?;
        return Err("wallet callback requires PostgreSQL".into());
    };
    let locale = telegram_locale(pool, &entry.tenant_id, user_id)
        .await
        .map_err(|_| "failed to load Telegram language preference".to_string())?;
    if matches!(&action, WalletCallback::Add) {
        let enabled: bool = sqlx::query_scalar(
            "SELECT COALESCE(current_setting('tgbot.telegram_wallet_monitors_enabled', true), 'false') = 'true'",
        )
        .fetch_one(pool)
        .await
        .map_err(|_| "failed to read wallet monitor feature gate".to_string())?;
        if !enabled {
            answer_wallet_callback(&telegram, callback_id, "地址监听功能未启用", true).await?;
            return Ok(true);
        }
    }
    if matches!(
        &action,
        WalletCallback::AdminList | WalletCallback::AdminDeactivate(_)
    ) {
        let is_admin: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM merchants
                 WHERE tenant_id = $1 AND owner_tg_user_id = $2
             ) OR EXISTS (
                 SELECT 1
                 FROM tenant_memberships membership
                 JOIN auth.users user_record ON user_record.id = membership.user_id
                 WHERE membership.tenant_id = $1
                   AND user_record.tg_user_id = $2
                   AND membership.role IN ('merchant_owner', 'merchant_admin')
             )",
        )
        .bind(&entry.tenant_id)
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(|_| "failed to verify wallet admin permission".to_string())?;
        if !is_admin {
            answer_wallet_callback(&telegram, callback_id, "权限不足", true).await?;
            return Ok(true);
        }
    }
    let (text, reply_markup) = match action {
        WalletCallback::List => {
            wallet_list_reply_for_locale(pool, &entry.tenant_id, user_id, &locale)
                .await
                .map_err(|_| "failed to load wallet monitor list".to_string())?
        }
        WalletCallback::AdminList => {
            wallet_admin_list_reply_for_locale(pool, &entry.tenant_id, &locale)
                .await
                .map_err(|_| "failed to load tenant wallet monitor list".to_string())?
        }
        WalletCallback::Add => {
            let prompt = wallet_monitor_prompt(&locale);
            let back_label = if locale == "en" {
                "Back to my monitors"
            } else {
                "返回我的监听"
            };
            (
                prompt,
                Some(json!({
                    "inline_keyboard": [[{
                        "text": back_label,
                        "callback_data": "wallet_my_list"
                    }]]
                })),
            )
        }
        WalletCallback::BackToStart => {
            let (is_platform_admin, has_merchant) = user_console_roles(pool, user_id)
                .await
                .map_err(|error| error.to_string())?;
            build_full_start_menu(
                is_platform_admin,
                has_merchant,
                callback
                    .get("from")
                    .and_then(|from| from.get("first_name"))
                    .and_then(Value::as_str)
                    .unwrap_or("朋友"),
                user_id,
                &locale,
            )
        }
        WalletCallback::Deactivate(monitor_id) => {
            let deactivated = sqlx::query(
                "UPDATE address_monitors SET is_active = false, updated_at = now() \
                 WHERE monitor_id = $1 AND tenant_id = $2 AND user_id = $3 AND is_active = true \
                 RETURNING monitor_id",
            )
            .bind(monitor_id)
            .bind(&entry.tenant_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .map_err(|_| "failed to deactivate wallet monitor".to_string())?;
            if deactivated.is_none() {
                answer_wallet_callback(&telegram, callback_id, "未找到可取消的监听", true).await?;
                return Ok(true);
            }
            wallet_list_reply_for_locale(pool, &entry.tenant_id, user_id, &locale)
                .await
                .map_err(|_| "failed to reload wallet monitor list".to_string())?
        }
        WalletCallback::AdminDeactivate(monitor_id) => {
            let deactivated = sqlx::query(
                "UPDATE address_monitors SET is_active = false, updated_at = now() \
                 WHERE monitor_id = $1 AND tenant_id = $2 AND is_active = true \
                 RETURNING monitor_id",
            )
            .bind(monitor_id)
            .bind(&entry.tenant_id)
            .fetch_optional(pool)
            .await
            .map_err(|_| "failed to deactivate tenant wallet monitor".to_string())?;
            if deactivated.is_none() {
                answer_wallet_callback(&telegram, callback_id, "未找到可取消的监听", true).await?;
                return Ok(true);
            }
            wallet_admin_list_reply_for_locale(pool, &entry.tenant_id, &locale)
                .await
                .map_err(|_| "failed to reload tenant wallet monitor list".to_string())?
        }
    };
    telegram
        .edit_message_text(EditMessageTextParams {
            chat_id: user_id,
            message_id,
            text,
            parse_mode: None,
            reply_markup,
        })
        .await
        .map_err(|_| "Telegram wallet menu update failed".to_string())?;
    answer_wallet_callback(&telegram, callback_id, "", false).await?;
    Ok(true)
}

async fn handle_language_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: Option<&sqlx::PgPool>,
    telegram_api_base_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(action) = parse_language_callback(data) else {
        return Ok(false);
    };
    let callback_id = callback
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "language callback has no id".to_string())?;
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: telegram_api_base_url.to_string(),
        timeout_secs: 30,
    })
    .map_err(|_| "failed to initialize Telegram client".to_string())?;
    let user_id = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    let message = callback.get("message");
    let callback_chat_id = message
        .and_then(|message| message.get("chat"))
        .and_then(|chat| chat.get("id"))
        .and_then(Value::as_i64);
    let message_id = message
        .and_then(|message| message.get("message_id"))
        .and_then(Value::as_i64);
    let Some(user_id) = user_id else {
        answer_language_callback(&telegram, callback_id, "请求无效", true).await?;
        return Ok(true);
    };
    if callback_chat_id != Some(user_id) {
        answer_language_callback(&telegram, callback_id, "请在私聊中操作", true).await?;
        return Ok(true);
    }
    let Some(message_id) = message_id else {
        answer_language_callback(&telegram, callback_id, "请求无效", true).await?;
        return Ok(true);
    };
    let Some(pool) = pool else {
        answer_language_callback(&telegram, callback_id, "服务暂不可用", true).await?;
        return Err("language callback requires PostgreSQL".into());
    };

    let (text, reply_markup, answer_text) = match action {
        LanguageCallback::Menu => {
            let locale = telegram_locale(pool, &entry.tenant_id, user_id)
                .await
                .map_err(|_| "failed to load Telegram language preference".to_string())?;
            let (text, reply_markup) = language_menu_reply(&locale);
            (text, reply_markup, "")
        }
        LanguageCallback::Set(locale) => {
            let locale = set_telegram_locale(pool, &entry.tenant_id, user_id, locale)
                .await
                .map_err(|_| "failed to save Telegram language preference".to_string())?;
            let (is_platform_admin, has_merchant) = user_console_roles(pool, user_id)
                .await
                .map_err(|error| error.to_string())?;
            let (text, reply_markup) = build_full_start_menu(
                is_platform_admin,
                has_merchant,
                callback
                    .get("from")
                    .and_then(|from| from.get("first_name"))
                    .and_then(Value::as_str)
                    .unwrap_or("朋友"),
                user_id,
                &locale,
            );
            let answer_text = if locale == "en" {
                "Language updated"
            } else {
                "语言已更新"
            };
            (text, reply_markup, answer_text)
        }
    };
    telegram
        .edit_message_text(EditMessageTextParams {
            chat_id: user_id,
            message_id,
            text,
            parse_mode: None,
            reply_markup,
        })
        .await
        .map_err(|_| "Telegram language menu update failed".to_string())?;
    answer_language_callback(&telegram, callback_id, answer_text, false).await?;
    Ok(true)
}

async fn answer_language_callback(
    telegram: &TelegramClient,
    callback_id: &str,
    text: &str,
    show_alert: bool,
) -> Result<(), String> {
    telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: (!text.is_empty()).then(|| text.to_string()),
            show_alert: Some(show_alert),
        })
        .await
        .map_err(|_| "Telegram callback acknowledgement failed".to_string())
}

async fn answer_wallet_callback(
    telegram: &TelegramClient,
    callback_id: &str,
    text: &str,
    show_alert: bool,
) -> Result<(), String> {
    telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: (!text.is_empty()).then(|| text.to_string()),
            show_alert: Some(show_alert),
        })
        .await
        .map_err(|_| "Telegram callback acknowledgement failed".to_string())
}

async fn handle_channel_subscription_callback(
    update: &TelegramUpdate,
    entry: &BotEntry,
    pool: Option<&sqlx::PgPool>,
    telegram_api_base_url: &str,
) -> Result<bool, String> {
    let Some(callback) = update.callback_query.as_ref() else {
        return Ok(false);
    };
    let Some(data) = callback.get("data").and_then(Value::as_str) else {
        return Ok(false);
    };
    if !data.starts_with("subscribe_") && !data.starts_with("unmute_verify:") {
        return Ok(false);
    }

    let callback_id = callback
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "channel subscription callback has no id".to_string())?;
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: telegram_api_base_url.to_string(),
        timeout_secs: 30,
    })
    .map_err(|_| "failed to initialize Telegram client".to_string())?;
    let parsed =
        parse_channel_subscription_callback(data).or_else(|| parse_unmute_verify_callback(data));
    let user_id = callback
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64);
    let message = callback.get("message");
    let callback_chat_id = message
        .and_then(|message| message.get("chat"))
        .and_then(|chat| chat.get("id"))
        .and_then(Value::as_i64);
    let message_id = message
        .and_then(|message| message.get("message_id"))
        .and_then(Value::as_i64);
    let (group_id, channel_id, user_id) = match (parsed, user_id) {
        (Some((group_id, channel_id)), Some(user_id))
            if user_id > 0
                && (callback_chat_id == Some(group_id) || callback_chat_id == Some(user_id)) =>
        {
            (group_id, channel_id, user_id)
        }
        _ => {
            answer_subscription_callback(&telegram, callback_id, "验证请求无效", true).await?;
            return Ok(true);
        }
    };
    let Some(pool) = pool else {
        answer_subscription_callback(&telegram, callback_id, "验证服务暂不可用", true).await?;
        return Err("channel subscription callback requires PostgreSQL".into());
    };

    let config = sqlx::query(
        "SELECT verify_enabled, verify_type, verify_channel_id \
         FROM groups.tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(&entry.tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "failed to load channel verification config".to_string())?;
    let valid_config = config.is_some_and(|row| {
        row.try_get::<bool, _>("verify_enabled").unwrap_or(false)
            && row.try_get::<String, _>("verify_type").ok().as_deref() == Some("channel")
            && row
                .try_get::<Option<String>, _>("verify_channel_id")
                .ok()
                .flatten()
                .and_then(|value| value.parse::<i64>().ok())
                == Some(channel_id)
    });
    if !valid_config {
        answer_subscription_callback(&telegram, callback_id, "验证配置已更新，请刷新后重试", true)
            .await?;
        return Ok(true);
    }

    let member = match telegram.get_chat_member(channel_id, user_id).await {
        Ok(member) => member,
        Err(_) => {
            answer_subscription_callback(&telegram, callback_id, "暂时无法检查关注状态", true)
                .await?;
            return Err("Telegram channel membership check failed".into());
        }
    };
    if !telegram_member_is_subscribed(&member) {
        answer_subscription_callback(&telegram, callback_id, "请先关注频道", false).await?;
        return Ok(true);
    }

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
        .map_err(|_| "Telegram member unmute failed".to_string())?;
    sqlx::query(
        "UPDATE groups.group_members SET status = 'active', is_verified = true, \
         verified_at = now(), updated_at = now() \
         WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(&entry.tenant_id)
    .execute(pool)
    .await
    .map_err(|_| "failed to persist channel verification".to_string())?;
    answer_subscription_callback(&telegram, callback_id, "✅ 验证成功！您已被解禁", true).await?;

    if callback_chat_id == Some(group_id) {
        if let Some(message_id) = message_id {
            if telegram.delete_message(group_id, message_id).await.is_ok() {
                sqlx::query(
                    "UPDATE groups.channel_reminders SET is_deleted = true \
                 WHERE group_id = $1 AND user_id = $2 AND message_id = $3 AND tenant_id = $4",
                )
                .bind(group_id)
                .bind(user_id)
                .bind(message_id)
                .bind(&entry.tenant_id)
                .execute(pool)
                .await
                .map_err(|_| "failed to mark channel reminder deleted".to_string())?;
            }
        }
    }

    Ok(true)
}

fn parse_channel_subscription_callback(data: &str) -> Option<(i64, i64)> {
    let mut parts = data.split('_');
    if parts.next()? != "subscribe" {
        return None;
    }
    let group_id = parts.next()?.parse().ok()?;
    let channel_id = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((group_id, channel_id))
}

fn parse_unmute_verify_callback(data: &str) -> Option<(i64, i64)> {
    let mut parts = data.split(':');
    if parts.next()? != "unmute_verify" {
        return None;
    }
    let group_id = parts.next()?.parse().ok()?;
    let channel_id = parts.next()?.parse().ok()?;
    if parts.next().is_some() || group_id == 0 || channel_id == 0 {
        return None;
    }
    Some((group_id, channel_id))
}

fn telegram_member_is_subscribed(member: &Value) -> bool {
    match member.get("status").and_then(Value::as_str) {
        Some("creator" | "administrator" | "member") => true,
        Some("restricted") => member
            .get("is_member")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        _ => false,
    }
}

async fn answer_subscription_callback(
    telegram: &TelegramClient,
    callback_id: &str,
    text: &str,
    show_alert: bool,
) -> Result<(), String> {
    telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: Some(text.to_string()),
            show_alert: Some(show_alert),
        })
        .await
        .map_err(|_| "Telegram callback acknowledgement failed".to_string())
}

async fn acknowledge_unsupported_callback(
    entry: &BotEntry,
    callback_id: &str,
    telegram_api_base_url: &str,
) -> Result<(), String> {
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: telegram_api_base_url.to_string(),
        timeout_secs: 30,
    })
    .map_err(|_| "failed to initialize Telegram client".to_string())?;
    telegram
        .answer_callback_query(AnswerCallbackQueryParams {
            callback_query_id: callback_id.to_string(),
            text: Some("当前 Beta 暂不支持此操作".to_string()),
            show_alert: Some(true),
        })
        .await
        .map_err(|_| "Telegram callback acknowledgement failed".to_string())
}

#[cfg(test)]
mod channel_subscription_tests {
    use super::*;

    #[test]
    fn parses_strict_channel_subscription_callback() {
        assert_eq!(
            parse_channel_subscription_callback("subscribe_-10001_-30001"),
            Some((-10001, -30001))
        );
        assert_eq!(
            parse_channel_subscription_callback("subscribe_bad_-30001"),
            None
        );
        assert_eq!(
            parse_channel_subscription_callback("subscribe_-10001_-30001_extra"),
            None
        );
        assert_eq!(
            parse_channel_subscription_callback("unrelated_-10001_-30001"),
            None
        );
    }

    #[test]
    fn parses_strict_wallet_callbacks() {
        assert_eq!(
            parse_wallet_callback("wallet_my_list"),
            Some(WalletCallback::List)
        );
        assert_eq!(
            parse_wallet_callback("wallet_admin_all"),
            Some(WalletCallback::AdminList)
        );
        assert_eq!(
            parse_wallet_callback("wallet_monitor"),
            Some(WalletCallback::Add)
        );
        assert_eq!(
            parse_wallet_callback("back_to_start"),
            Some(WalletCallback::BackToStart)
        );
        assert_eq!(
            parse_wallet_callback("uw_u_42"),
            Some(WalletCallback::Deactivate(42))
        );
        assert_eq!(parse_wallet_callback("uw_u_0"), None);
        assert_eq!(parse_wallet_callback("uw_u_-1"), None);
        assert_eq!(parse_wallet_callback("uw_u_42_extra"), None);
        assert_eq!(
            parse_wallet_callback("uw_a_42"),
            Some(WalletCallback::AdminDeactivate(42))
        );
        assert_eq!(parse_wallet_callback("uw_a_0"), None);
        assert_eq!(parse_wallet_callback("uw_a_42_extra"), None);
    }

    #[test]
    fn parses_legacy_unmute_verify_callback() {
        assert_eq!(
            parse_unmute_verify_callback("unmute_verify:-10001:-30001"),
            Some((-10001, -30001))
        );
        assert_eq!(
            parse_unmute_verify_callback("unmute_verify:bad:-30001"),
            None
        );
        assert_eq!(
            parse_unmute_verify_callback("unmute_verify:-10001:-30001:extra"),
            None
        );
    }

    #[test]
    fn accepts_only_active_channel_members() {
        assert!(telegram_member_is_subscribed(
            &json!({ "status": "member" })
        ));
        assert!(telegram_member_is_subscribed(&json!({
            "status": "restricted",
            "is_member": true,
        })));
        assert!(!telegram_member_is_subscribed(&json!({
            "status": "restricted",
            "is_member": false,
        })));
        assert!(!telegram_member_is_subscribed(&json!({ "status": "left" })));
    }

    #[test]
    fn persists_restricted_non_members_as_left() {
        assert_eq!(
            persisted_member_status(&json!({
                "status": "restricted",
                "is_member": true,
            })),
            "active"
        );
        assert_eq!(
            persisted_member_status(&json!({
                "status": "restricted",
                "is_member": false,
            })),
            "left"
        );
        assert_eq!(
            persisted_member_status(&json!({ "status": "administrator" })),
            "admin"
        );
        assert_eq!(
            persisted_member_status(&json!({ "status": "kicked" })),
            "banned"
        );
    }

    #[test]
    fn extracts_text_and_largest_photo() {
        assert_eq!(
            extract_inbound_content(&json!({ "text": "hello" })),
            Some(InboundContent {
                content: Some("hello".into()),
                msg_type: "text".into(),
                file_id: None,
                media_unique_id: None,
            })
        );
        assert_eq!(
            extract_inbound_content(&json!({
                "caption": "image",
                "photo": [
                    { "file_id": "small", "file_unique_id": "same-photo" },
                    { "file_id": "large", "file_unique_id": "same-photo" }
                ],
            })),
            Some(InboundContent {
                content: Some("image".into()),
                msg_type: "photo".into(),
                file_id: Some("large".into()),
                media_unique_id: Some("same-photo".into()),
            })
        );
    }

    #[tokio::test]
    async fn webhook_persistence_failure_is_reported_without_postgres() {
        let dispatcher = UpdateDispatcher::new();
        let llm = LiteLLMClient::new(tgbot_integrations::litellm::LiteLLMConfig {
            base_url: "http://127.0.0.1:4000".into(),
            api_key: String::new(),
            default_model: "gpt-5.6-luna".into(),
            timeout_secs: 5,
        })
        .unwrap();
        install_default_handlers(&dispatcher, None, llm, "http://127.0.0.1".to_string()).await;
        let update: TelegramUpdate = serde_json::from_value(json!({
            "update_id": 1,
            "message": {
                "message_id": 1,
                "from": {"id": 2, "is_bot": false},
                "chat": {"id": 2, "type": "private"},
                "date": 1,
                "text": "database outage"
            }
        }))
        .expect("test update should deserialize");
        let result = dispatcher
            .dispatch(
                update,
                BotEntry {
                    bot_id: 7,
                    bot_token: "test-token".to_string(),
                    tenant_id: "tenant-beta".to_string(),
                    merchant_id: "merchant-beta".to_string(),
                    bot_username: Some("beta_bot".to_string()),
                },
            )
            .await;
        assert_eq!(result, Err("webhook persistence unavailable".to_string()));
    }

    #[test]
    fn ignores_service_only_messages() {
        assert_eq!(
            extract_inbound_content(&json!({ "new_chat_members": [] })),
            None
        );
    }
}
