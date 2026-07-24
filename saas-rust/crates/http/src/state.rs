use std::sync::Arc;
use tgbot_domain::config::Config;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_integrations::supabase_rest::SupabaseRestClient;
use crate::routes::auth::OtpStore;
use crate::routes::auth::CsrfStore;
use crate::routes::miniapp::GrantStore;
use crate::routes::telegram_webhook::{BotRegistry, UpdateDispatcher};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Direct PG pool — None when PG TCP is blocked in this environment.
    pub pool: Option<sqlx::PgPool>,
    /// Supabase REST client as fallback / additional transport.
    pub supabase: SupabaseRestClient,
    pub otp_store: OtpStore,
    pub csrf_store: CsrfStore,
    pub grant_store: GrantStore,
    /// Registry of active Telegram bots, keyed by bot_id.
    pub bot_registry: BotRegistry,
    /// Update dispatcher for routing Telegram updates to handlers.
    pub update_dispatcher: Option<Arc<UpdateDispatcher>>,
}

impl AppState {
    /// Returns a reference to the PostgreSQL pool, or an error when in REST-only mode.
    pub fn db(&self) -> ApiResult<&sqlx::PgPool> {
        self.pool
            .as_ref()
            .ok_or_else(|| ApiError::BadRequest("PostgreSQL unavailable — REST mode only".into()))
    }
}
