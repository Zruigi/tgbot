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
use crate::secret_crypto::SecretCipher;
use crate::state::AppState;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_integrations::telegram::{
    AnswerCallbackQueryParams, EditMessageTextParams, TelegramClient, TelegramConfig,
};
use tgbot_repos::{audit_repo, message_repo};
use tgbot_services::ad_text::ad_text_hash;
use tgbot_services::telegram_preferences::{
    language_menu_reply, localized_start_menu_reply, parse_language_callback, set_telegram_locale,
    telegram_locale, LanguageCallback,
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
    telegram_api_base_url: String,
) {
    let message_pool = pool.clone();
    dispatcher
        .on(UpdateKind::Message, move |update, entry| {
            let pool = message_pool.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                if let Some(message) = &update.message {
                    handle_incoming_message(pool, &entry, message, false).await?;
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
    dispatcher
        .on(UpdateKind::EditedMessage, move |update, entry| {
            let pool = edited_message_pool.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
                if let Some(message) = &update.edited_message {
                    handle_incoming_message(pool, &entry, message, true).await?;
                }
                Ok(())
            }
        })
        .await;

    // Callback query handler
    let callback_pool = pool.clone();
    dispatcher
        .on(UpdateKind::CallbackQuery, move |update, entry| {
            let pool = callback_pool.clone();
            let telegram_api_base_url = telegram_api_base_url.clone();
            async move {
                let pool = require_webhook_pool(pool.as_ref())?;
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

async fn handle_incoming_message(
    pool: &sqlx::PgPool,
    entry: &BotEntry,
    message: &Value,
    edited: bool,
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
            apply_message_membership_events(pool, entry, message, chat_id, &group_title).await?;
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
        }
    }
    if let Some(user) = message.get("left_chat_member") {
        if let Some(user_id) = user
            .get("id")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
        {
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
        WalletCallback::BackToStart => localized_start_menu_reply(
            callback
                .get("from")
                .and_then(|from| from.get("first_name"))
                .and_then(Value::as_str)
                .unwrap_or("朋友"),
            &locale,
        ),
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
            let (text, reply_markup) = localized_start_menu_reply(
                callback
                    .get("from")
                    .and_then(|from| from.get("first_name"))
                    .and_then(Value::as_str)
                    .unwrap_or("朋友"),
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
        install_default_handlers(&dispatcher, None, "http://127.0.0.1".to_string()).await;
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
