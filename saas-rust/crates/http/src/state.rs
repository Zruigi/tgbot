use crate::routes::auth::CsrfStore;
use crate::routes::auth::OtpStore;
use crate::routes::miniapp::GrantStore;
use crate::routes::telegram_webhook::{BotRegistry, UpdateDispatcher};
use std::sync::Arc;
use tgbot_domain::config::Config;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::worker_telemetry::WorkerTelemetry;
use tgbot_integrations::litellm::LiteLLMClient;
use tgbot_integrations::supabase_rest::SupabaseRestClient;
use tgbot_integrations::telegram::{TelegramClient, TelegramConfig};
use tgbot_repos::user_repo;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Direct PG pool — None when PG TCP is blocked in this environment.
    pub pool: Option<sqlx::PgPool>,
    /// Supabase REST client as fallback / additional transport.
    pub supabase: SupabaseRestClient,
    pub llm: LiteLLMClient,
    pub otp_store: OtpStore,
    pub csrf_store: CsrfStore,
    pub grant_store: GrantStore,
    /// Registry of active Telegram bots, keyed by bot_id.
    pub bot_registry: BotRegistry,
    /// True when the formal app started its PostgreSQL-backed scheduler.
    /// This is deliberately a coarse readiness signal; per-merchant telemetry
    /// is reported separately by the runtime-status endpoint.
    pub worker_enabled: bool,
    /// In-process scheduler telemetry exposed to platform status checks.
    pub worker_telemetry: WorkerTelemetry,
    /// Update dispatcher for routing Telegram updates to handlers.
    pub update_dispatcher: Option<Arc<UpdateDispatcher>>,
    /// WebSocket 广播通道：向所有 /ws 连接推送实时事件（对齐 Python notify_web_clients）。
    pub ws_tx: tokio::sync::broadcast::Sender<String>,
}

impl AppState {
    /// Returns a reference to the PostgreSQL pool, or an error when in REST-only mode.
    pub fn db(&self) -> ApiResult<&sqlx::PgPool> {
        self.pool
            .as_ref()
            .ok_or_else(|| ApiError::BadRequest("PostgreSQL unavailable — REST mode only".into()))
    }

    /// 向所有 WebSocket 客户端广播一条事件（对齐 Python web_admin.notify_web_clients）。
    /// 事件格式 `{"type": ..., "data": {...}}`，前端 ws.ts JSON.parse 后按 type 分发。
    pub fn broadcast(&self, event_type: &str, data: serde_json::Value) {
        let payload =
            serde_json::json!({ "type": event_type, "data": data }).to_string();
        let _ = self.ws_tx.send(payload);
    }

    pub async fn tenant_telegram(&self, tenant_id: &str) -> ApiResult<(i64, TelegramClient)> {
        let bot_id = user_repo::get_bot_id_for_tenant(self.db()?, tenant_id)
            .await?
            .ok_or_else(|| ApiError::BadRequest("merchant has no bot configured".into()))?;
        let entry = self
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
        Ok((bot_id, telegram))
    }
}
