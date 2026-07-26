use anyhow::anyhow;
use axum::{extract::State, http::HeaderMap, Json};
use serde_json::{json, Value};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_services::telegram_auto_replies::{
    normalize_auto_reply_config, TelegramAutoReplyConfig, TELEGRAM_AUTO_REPLY_CONFIG_KEY,
};

use crate::auth::resolve_scope;
use crate::state::AppState;

pub async fn get_auto_reply_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _) = scope.require_merchant()?;
    let config = load_config(&state, &tenant_id).await?;

    Ok(Json(json!({"status": "ok", "data": config})))
}

pub async fn save_auto_reply_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(config): Json<TelegramAutoReplyConfig>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let (tenant_id, _) = scope.require_merchant()?;
    let config = normalize_auto_reply_config(config).map_err(ApiError::BadRequest)?;
    let config_json = serde_json::to_string(&config)
        .map_err(|error| anyhow!("failed to serialize auto-reply config: {error}"))?;

    sqlx::query(
        "INSERT INTO tenant_saas.tenant_configs \
         (tenant_id, config_key, config_value, remark, updated_at) \
         VALUES ($1, $2, $3, 'Telegram auto reply config v1', now()) \
         ON CONFLICT (tenant_id, config_key) DO UPDATE \
         SET config_value = EXCLUDED.config_value, \
             remark = EXCLUDED.remark, \
             updated_at = now()",
    )
    .bind(&tenant_id)
    .bind(TELEGRAM_AUTO_REPLY_CONFIG_KEY)
    .bind(config_json)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({"status": "ok", "data": config})))
}

async fn load_config(state: &AppState, tenant_id: &str) -> ApiResult<TelegramAutoReplyConfig> {
    let config_json = sqlx::query_scalar::<_, Option<String>>(
        "SELECT config_value FROM tenant_saas.tenant_configs \
         WHERE tenant_id = $1 AND config_key = $2",
    )
    .bind(tenant_id)
    .bind(TELEGRAM_AUTO_REPLY_CONFIG_KEY)
    .fetch_optional(state.db()?)
    .await?
    .flatten();

    let Some(config_json) = config_json else {
        return Ok(TelegramAutoReplyConfig::default());
    };
    let config: TelegramAutoReplyConfig = serde_json::from_str(&config_json)
        .map_err(|error| anyhow!("stored auto-reply config is invalid: {error}"))?;
    normalize_auto_reply_config(config)
        .map_err(|error| anyhow!("stored auto-reply config is invalid: {error}"))
        .map_err(ApiError::from)
}

fn require_write_access(scope: &tgbot_domain::scope::Scope) -> ApiResult<()> {
    if matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
