// Telegram Bot Webhook integration
// Receives Telegram Updates via webhook and dispatches to per-merchant handlers.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Column, Row, TypeInfo};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use tracing::{info, warn, error, debug};

use crate::state::AppState;
use tgbot_domain::error::{ApiError, ApiResult};

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
pub type DispatchResult = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>;

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
                debug!(?kind, update_id = update.update_id, "Dispatching to default handler");
                handler(update, entry).await
            } else {
                warn!(?kind, update_id = update.update_id, "No handler for update kind");
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
    if expected_secret.is_empty() {
        // No secret configured — skip verification
        return Ok(());
    }

    let provided = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided == expected_secret {
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
    let entry = state
        .bot_registry
        .get(bot_id)
        .await
        .ok_or_else(|| {
            warn!(bot_id, "Webhook received for unknown bot_id");
            ApiError::NotFound(format!("Bot {} not found in registry", bot_id))
        })?;

    // 2. Verify webhook secret (if configured)
    // For multi-tenant bots, the secret is derived from the bot's token hash
    let webhook_secret = derive_webhook_secret(&entry.bot_token);
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
                // We still return 200 to Telegram to avoid retries
                // but log the error for monitoring
            }
        }
    } else {
        warn!(bot_id, update_id, "No update dispatcher configured, dropping update");
    }

    // Always return 200 OK to Telegram Bot API
    Ok(Json(json!({})))
}

/// POST /telegram/webhook/:bot_id/register
///
/// Register a bot in the webhook registry so its webhook endpoint becomes active.
pub async fn register_bot(
    State(state): State<AppState>,
    Path(bot_id): Path<i64>,
    Json(body): Json<RegisterBotRequest>,
) -> ApiResult<Json<Value>> {
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
) -> ApiResult<Json<Value>> {
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
) -> ApiResult<Json<Value>> {
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
    Json(body): Json<DispatchTestRequest>,
) -> ApiResult<Json<Value>> {
    let entry = state
        .bot_registry
        .get(body.bot_id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Bot {} not found", body.bot_id)))?;

    let update: TelegramUpdate = serde_json::from_value(body.update).map_err(|e| {
        ApiError::BadRequest(format!("Invalid update payload: {}", e))
    })?;

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
fn derive_webhook_secret(bot_token: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    bot_token.hash(&mut hasher);
    format!("wh_{:016x}", hasher.finish())
}

/// Load all active merchant bots from the database into the registry.
/// Called at startup to warm the registry.
pub async fn load_bots_from_db(pool: &sqlx::PgPool, registry: &BotRegistry) -> anyhow::Result<()> {
    let rows = sqlx::query(
        "SELECT id, tenant_id, bot_id, bot_username, bot_token_ref \
         FROM merchants \
         WHERE status = 'active' AND bot_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    let mut count = 0u32;
    for row in rows {
        let merchant_id: uuid::Uuid = row.try_get("id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let bot_id: Option<i64> = row.try_get("bot_id")?;
        let bot_username: Option<String> = row.try_get("bot_username")?;
        let bot_token_ref: Option<String> = row.try_get("bot_token_ref")?;

        // bot_token_ref stores a reference/pointer to the actual token;
        // In production this would be decrypted from a secrets manager.
        // For now, we use the ref as the token itself if available.
        let bot_token = bot_token_ref.unwrap_or_default();

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
pub async fn install_default_handlers(dispatcher: &UpdateDispatcher) {
    // Message handler
    dispatcher
        .on(UpdateKind::Message, |update, entry| async move {
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
        })
        .await;

    // Callback query handler
    dispatcher
        .on(UpdateKind::CallbackQuery, |update, entry| async move {
            if let Some(cq) = &update.callback_query {
                let data = cq.get("data").and_then(|d| d.as_str()).unwrap_or("");
                info!(
                    bot_id = entry.bot_id,
                    data,
                    "Received callback query"
                );
            }
            Ok(())
        })
        .await;

    // Chat join request handler
    dispatcher
        .on(UpdateKind::ChatJoinRequest, |update, entry| async move {
            if let Some(jr) = &update.chat_join_request {
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
        })
        .await;

    // My chat member handler
    dispatcher
        .on(UpdateKind::MyChatMember, |update, entry| async move {
            if let Some(cm) = &update.my_chat_member {
                let chat_id = cm.get("chat").and_then(|c| c.get("id"));
                info!(
                    bot_id = entry.bot_id,
                    ?chat_id,
                    "Chat member status changed"
                );
            }
            Ok(())
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
