use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

use tgbot_domain::config::Config;
use crate::routes::auth::OtpStore;
use crate::routes::auth::CsrfStore;
use crate::routes::miniapp::GrantStore;
use crate::routes::telegram_webhook::{BotRegistry, UpdateDispatcher};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: sqlx::PgPool,
    pub otp_store: OtpStore,
    pub csrf_store: CsrfStore,
    pub grant_store: GrantStore,
    /// Registry of active Telegram bots, keyed by bot_id.
    pub bot_registry: BotRegistry,
    /// Update dispatcher for routing Telegram updates to handlers.
    pub update_dispatcher: Option<Arc<UpdateDispatcher>>,
}
