use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::{PlanCodeRequest, Role, SaveSecretRefRequest, UpdateStatusRequest};
use tgbot_integrations::telegram::{SetWebhookParams, TelegramClient, TelegramConfig};
use uuid::Uuid;

use super::telegram_webhook::{derive_webhook_secret, BotEntry};
use crate::auth::resolve_scope;
use crate::secret_crypto::SecretCipher;
use crate::state::AppState;

use super::platform::{row_to_json, rows_to_json};

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    pub limit: Option<i64>,
    pub days: Option<i64>,
}

pub async fn profile(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let row = sqlx::query("SELECT *, EXISTS(SELECT 1 FROM tenant_secret_refs WHERE tenant_id = $1 AND secret_key = 'TELEGRAM_BOT_TOKEN') AS has_bot_token FROM merchants WHERE id = $2 AND tenant_id = $1")
        .bind(&tenant_id)
        .bind(merchant_id)
        .fetch_optional(state.db()?)
        .await?
        .ok_or_else(|| ApiError::NotFound("merchant_not_found".into()))?;
    let mut data = row_to_json(row);
    if let Some(object) = data.as_object_mut() {
        let has_bot_token = object
            .get("has_bot_token")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        object.insert(
            "bot_token".into(),
            if has_bot_token {
                Value::String("exists".into())
            } else {
                Value::Null
            },
        );
    }
    Ok(Json(json!({"status": "ok", "data": data})))
}

pub async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateStatusRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    if matches!(scope.role, Role::MerchantViewer) {
        return Err(ApiError::Forbidden);
    }
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let row = sqlx::query(
        "UPDATE merchants SET status = $3, onboarding_completed = COALESCE($4, onboarding_completed), updated_at = now() WHERE tenant_id = $1 AND id = $2 RETURNING *",
    )
    .bind(&tenant_id)
    .bind(merchant_id)
    .bind(body.status)
    .bind(body.onboarding_completed)
    .fetch_one(state.db()?)
    .await?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn save_bot_token_ref(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SaveSecretRefRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    if !matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin) {
        return Err(ApiError::Forbidden);
    }
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let bot_token = body.bot_token.trim();
    let (token_bot_id, token_secret) = bot_token
        .split_once(':')
        .ok_or_else(|| ApiError::BadRequest("invalid bot_token format".into()))?;
    let token_bot_id = token_bot_id
        .parse::<i64>()
        .map_err(|_| ApiError::BadRequest("invalid bot_token format".into()))?;
    if token_bot_id <= 0 || token_secret.len() < 10 {
        return Err(ApiError::BadRequest("bot_token required".into()));
    }

    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: bot_token.into(),
        api_base_url: std::env::var("TELEGRAM_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.telegram.org".into()),
        timeout_secs: 10,
    })
    .map_err(|_| ApiError::BadRequest("failed to initialize Telegram client".into()))?;
    let bot = telegram
        .get_me()
        .await
        .map_err(|_| ApiError::BadRequest("Telegram rejected bot_token".into()))?;
    if !bot.is_bot || bot.id != token_bot_id {
        return Err(ApiError::BadRequest("bot_token identity mismatch".into()));
    }

    let webhook_base_url = std::env::var("TELEGRAM_WEBHOOK_BASE_URL")
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string();
    let webhook_url = if state.config.telegram_polling_enabled || webhook_base_url.is_empty() {
        // Polling mode: never re-register the webhook (it would conflict with
        // getUpdates via HTTP 409). The receive-mode switch is a boot-time decision.
        None
    } else {
        if !(webhook_base_url.starts_with("https://")
            || webhook_base_url.starts_with("http://127.0.0.1"))
        {
            return Err(ApiError::BadRequest(
                "TELEGRAM_WEBHOOK_BASE_URL must use HTTPS".into(),
            ));
        }
        let secret = derive_webhook_secret(&state.config.secret_encryption_key, bot.id, bot_token)
            .map_err(ApiError::BadRequest)?;
        let url = format!("{webhook_base_url}/telegram/webhook/{}", bot.id);
        telegram
            .set_webhook(SetWebhookParams {
                url: url.clone(),
                secret_token: Some(secret),
                max_connections: Some(40),
                allowed_updates: None,
            })
            .await
            .map_err(|_| ApiError::BadRequest("failed to configure Telegram webhook".into()))?;
        Some(url)
    };

    let cipher = SecretCipher::from_encoded_key(&state.config.secret_encryption_key)
        .map_err(ApiError::BadRequest)?;
    let encrypted_ref = cipher
        .encrypt(&tenant_id, "TELEGRAM_BOT_TOKEN", bot_token)
        .map_err(ApiError::BadRequest)?;

    let mut transaction = state.db()?.begin().await?;
    sqlx::query(
        "INSERT INTO tenant_secret_refs (tenant_id, secret_key, secret_ref, updated_at) \
         VALUES ($1, 'TELEGRAM_BOT_TOKEN', $2, now()) \
         ON CONFLICT (tenant_id, secret_key) DO UPDATE \
         SET secret_ref = EXCLUDED.secret_ref, updated_at = now()",
    )
    .bind(&tenant_id)
    .bind(&encrypted_ref)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE merchants SET bot_id = $3, bot_username = $4, runtime_version = runtime_version + 1, \
         updated_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind(&tenant_id)
    .bind(merchant_id)
    .bind(bot.id)
    .bind(bot.username.as_deref())
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO tenant_audit_logs \
         (tenant_id, merchant_id, actor_user_id, operator_role, action, resource_type, resource_id, payload) \
         VALUES ($1, $2, $3, $4, 'bot_token_updated', 'merchant_bot', $5, $6)",
    )
    .bind(&tenant_id)
    .bind(merchant_id)
    .bind(scope.user_id)
    .bind(match scope.role {
        Role::MerchantOwner => "merchant_owner",
        _ => "merchant_admin",
    })
    .bind(bot.id.to_string())
    .bind(json!({"bot_username": bot.username}))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    state
        .bot_registry
        .register(BotEntry {
            bot_id: bot.id,
            bot_token: bot_token.into(),
            tenant_id: tenant_id.clone(),
            merchant_id: merchant_id.to_string(),
            bot_username: bot.username.clone(),
        })
        .await;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "id": bot.id,
            "username": bot.username,
            "first_name": bot.first_name,
            "is_bot": bot.is_bot,
            "webhook_configured": webhook_url.is_some(),
            "webhook_url": webhook_url,
        }
    })))
}

pub async fn subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let row = sqlx::query("SELECT * FROM merchant_subscriptions WHERE tenant_id = $1 AND merchant_id = $2 ORDER BY created_at DESC LIMIT 1")
        .bind(tenant_id)
        .bind(merchant_id)
        .fetch_optional(state.db()?)
        .await?;
    Ok(Json(json!({"status": "ok", "data": row.map(row_to_json)})))
}

pub async fn plans(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    super::platform::list_plans(State(state), headers).await
}

pub async fn create_payment_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PlanCodeRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let plan = sqlx::query("SELECT price_usdt::float8 AS price_usdt FROM subscription_plans WHERE plan_code = $1 AND status = 'active'")
        .bind(&body.plan_code)
        .fetch_optional(state.db()?)
        .await?
        .ok_or_else(|| ApiError::NotFound("subscription_plan_not_found".into()))?;
    let amount: f64 = plan.try_get("price_usdt")?;
    let row = sqlx::query(
        "INSERT INTO payment_orders (tenant_id, merchant_id, plan_code, amount_usdt, base_amount_usdt, status, expire_at) VALUES ($1, $2, $3, $4, $4, 'pending', now() + interval '30 minutes') RETURNING *",
    )
    .bind(&tenant_id)
    .bind(merchant_id)
    .bind(body.plan_code)
    .bind(amount)
    .fetch_one(state.db()?)
    .await?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn latest_payment_order(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let row = sqlx::query("SELECT * FROM payment_orders WHERE tenant_id = $1 AND merchant_id = $2 ORDER BY created_at DESC LIMIT 1")
        .bind(tenant_id)
        .bind(merchant_id)
        .fetch_optional(state.db()?)
        .await?;
    Ok(Json(json!({"status": "ok", "data": row.map(row_to_json)})))
}

pub async fn mark_payment_order_paid(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    update_own_order_status(state, headers, body, "paid").await
}

pub async fn submit_payment_tx(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let tx_hash = body
        .get("tx_hash")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if tx_hash.is_empty() {
        return Err(ApiError::BadRequest("tx_hash required".into()));
    }
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let order_id = parse_order_id(&body)?;
    let row = sqlx::query("UPDATE payment_orders SET status = 'paid', tx_hash = $4, updated_at = now() WHERE tenant_id = $1 AND merchant_id = $2 AND id = $3 AND status IN ('pending', 'paid') RETURNING *")
        .bind(tenant_id)
        .bind(merchant_id)
        .bind(order_id)
        .bind(tx_hash)
        .fetch_optional(state.db()?)
        .await?
        .ok_or_else(|| ApiError::NotFound("payment_order_not_found".into()))?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn features(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let rows = sqlx::query("SELECT feature_key, enabled, limits_json FROM merchant_features WHERE tenant_id = $1 ORDER BY feature_key")
        .bind(tenant_id)
        .fetch_all(state.db()?)
        .await?;
    let total = rows.len();
    Ok(Json(
        json!({"status": "ok", "data": rows_to_json(rows), "total": total}),
    ))
}

pub async fn ai_quota(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let row = sqlx::query("SELECT * FROM merchant_ai_quota WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(state.db()?)
        .await?;
    Ok(Json(json!({"status": "ok", "data": row.map(row_to_json)})))
}

pub async fn ai_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let days = q.days.unwrap_or(7).clamp(1, 90);
    let _limit = q.limit.unwrap_or(50).clamp(1, 200);

    // Get summary
    let summary_row = sqlx::query(
        r#"
        SELECT
            COUNT(*) AS total_requests,
            COALESCE(SUM(total_tokens), 0)::bigint AS total_tokens,
            COALESCE(SUM(estimated_cost_usd), 0) AS total_cost,
            COALESCE(SUM(prompt_tokens), 0)::bigint AS prompt_tokens,
            COALESCE(SUM(completion_tokens), 0)::bigint AS completion_tokens
        FROM ai_usage_logs
        WHERE tenant_id = $1
          AND created_at >= NOW() - INTERVAL '1 day' * $2
        "#,
    )
    .bind(&tenant_id)
    .bind(days)
    .fetch_optional(state.db()?)
    .await?;

    // Get daily breakdown
    let daily_rows = sqlx::query(
        r#"
        SELECT
            TO_CHAR(DATE(created_at), 'YYYY-MM-DD') AS date,
            COUNT(*) AS requests,
            COALESCE(SUM(total_tokens), 0)::bigint AS tokens,
            COALESCE(SUM(estimated_cost_usd), 0) AS cost
        FROM ai_usage_logs
        WHERE tenant_id = $1
          AND created_at >= NOW() - INTERVAL '1 day' * $2
        GROUP BY DATE(created_at)
        ORDER BY date DESC
        LIMIT 30
        "#,
    )
    .bind(&tenant_id)
    .bind(days)
    .fetch_all(state.db()?)
    .await?;

    // Get by model
    let model_rows = sqlx::query(
        r#"
        SELECT
            model,
            COUNT(*) AS requests,
            COALESCE(SUM(total_tokens), 0)::bigint AS tokens,
            COALESCE(SUM(estimated_cost_usd), 0) AS cost
        FROM ai_usage_logs
        WHERE tenant_id = $1
          AND created_at >= NOW() - INTERVAL '1 day' * $2
        GROUP BY model
        ORDER BY cost DESC
        LIMIT 10
        "#,
    )
    .bind(&tenant_id)
    .bind(days)
    .fetch_all(state.db()?)
    .await?;

    // Build response
    let summary = if let Some(row) = summary_row {
        let total_requests: i64 = row.try_get("total_requests").unwrap_or(0);
        let total_tokens: i64 = row.try_get("total_tokens").unwrap_or(0);
        let total_cost: f64 = row.try_get("total_cost").unwrap_or(0.0);
        let prompt_tokens: i64 = row.try_get("prompt_tokens").unwrap_or(0);
        let completion_tokens: i64 = row.try_get("completion_tokens").unwrap_or(0);
        json!({
            "total_requests": total_requests,
            "total_tokens": total_tokens,
            "total_cost": total_cost,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
        })
    } else {
        json!({
            "total_requests": 0,
            "total_tokens": 0,
            "total_cost": 0.0,
            "prompt_tokens": 0,
            "completion_tokens": 0,
        })
    };

    let daily_items: Vec<Value> = daily_rows
        .iter()
        .map(|row| {
            let date: String = row.try_get("date").unwrap_or_default();
            let requests: i64 = row.try_get("requests").unwrap_or(0);
            let tokens: i64 = row.try_get("tokens").unwrap_or(0);
            let cost: f64 = row.try_get("cost").unwrap_or(0.0);
            json!({
                "date": date,
                "requests": requests,
                "tokens": tokens,
                "cost": cost,
            })
        })
        .collect();

    let by_model: Vec<Value> = model_rows
        .iter()
        .map(|row| {
            let model: String = row.try_get("model").unwrap_or_default();
            let requests: i64 = row.try_get("requests").unwrap_or(0);
            let tokens: i64 = row.try_get("tokens").unwrap_or(0);
            let cost: f64 = row.try_get("cost").unwrap_or(0.0);
            json!({
                "model": model,
                "requests": requests,
                "tokens": tokens,
                "cost": cost,
            })
        })
        .collect();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "days": days,
            "summary": summary,
            "daily": daily_items,
            "by_model": by_model,
        }
    })))
}

pub async fn audit_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query(
        "SELECT * FROM tenant_audit_logs WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(state.db()?)
    .await?;
    let total = rows.len();
    Ok(Json(
        json!({"status": "ok", "data": rows_to_json(rows), "total": total}),
    ))
}

async fn update_own_order_status(
    state: AppState,
    headers: HeaderMap,
    body: Value,
    status: &str,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;
    let order_id = parse_order_id(&body)?;
    let row = sqlx::query("UPDATE payment_orders SET status = $4, updated_at = now() WHERE tenant_id = $1 AND merchant_id = $2 AND id = $3 RETURNING *")
        .bind(tenant_id)
        .bind(merchant_id)
        .bind(order_id)
        .bind(status)
        .fetch_optional(state.db()?)
        .await?
        .ok_or_else(|| ApiError::NotFound("payment_order_not_found".into()))?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

fn parse_order_id(body: &Value) -> ApiResult<Uuid> {
    body.get("order_id")
        .and_then(Value::as_str)
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| ApiError::BadRequest("order_id invalid".into()))
}
