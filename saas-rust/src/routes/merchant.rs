use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{require_merchant, resolve_scope},
    error::{ApiError, ApiResult},
    models::{PlanCodeRequest, SaveSecretRefRequest, UpdateStatusRequest},
    state::AppState,
};

use super::platform::{row_to_json, rows_to_json};

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    pub limit: Option<i64>,
    pub days: Option<i64>,
}

pub async fn profile(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = require_merchant(&scope)?;
    let row = sqlx::query("SELECT *, EXISTS(SELECT 1 FROM tenant_secret_refs WHERE tenant_id = $1 AND secret_key = 'TELEGRAM_BOT_TOKEN') AS has_bot_token FROM merchants WHERE id = $2 AND tenant_id = $1")
        .bind(&tenant_id)
        .bind(merchant_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound("merchant_not_found".into()))?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateStatusRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = require_merchant(&scope)?;
    let row = sqlx::query(
        "UPDATE merchants SET status = $3, onboarding_completed = COALESCE($4, onboarding_completed), updated_at = now() WHERE tenant_id = $1 AND id = $2 RETURNING *",
    )
    .bind(&tenant_id)
    .bind(merchant_id)
    .bind(body.status)
    .bind(body.onboarding_completed)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn save_bot_token_ref(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SaveSecretRefRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = require_merchant(&scope)?;
    if body.bot_token.trim().is_empty() {
        return Err(ApiError::BadRequest("bot_token required".into()));
    }
    let secret_ref = format!("supabase-vault://tenant/{tenant_id}/TELEGRAM_BOT_TOKEN");
    sqlx::query(
        "INSERT INTO tenant_secret_refs (tenant_id, secret_key, secret_ref, updated_at) VALUES ($1, 'TELEGRAM_BOT_TOKEN', $2, now()) ON CONFLICT (tenant_id, secret_key) DO UPDATE SET secret_ref = EXCLUDED.secret_ref, updated_at = now()",
    )
    .bind(&tenant_id)
    .bind(secret_ref)
    .execute(&state.pool)
    .await?;
    Ok(Json(
        json!({"status": "ok", "data": {"bot_token": "exists"}}),
    ))
}

pub async fn subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = require_merchant(&scope)?;
    let row = sqlx::query("SELECT * FROM merchant_subscriptions WHERE tenant_id = $1 AND merchant_id = $2 ORDER BY created_at DESC LIMIT 1")
        .bind(tenant_id)
        .bind(merchant_id)
        .fetch_optional(&state.pool)
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
    let (tenant_id, merchant_id) = require_merchant(&scope)?;
    let plan = sqlx::query("SELECT price_usdt::float8 AS price_usdt FROM subscription_plans WHERE plan_code = $1 AND status = 'active'")
        .bind(&body.plan_code)
        .fetch_optional(&state.pool)
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
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn latest_payment_order(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = require_merchant(&scope)?;
    let row = sqlx::query("SELECT * FROM payment_orders WHERE tenant_id = $1 AND merchant_id = $2 ORDER BY created_at DESC LIMIT 1")
        .bind(tenant_id)
        .bind(merchant_id)
        .fetch_optional(&state.pool)
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
    let (tenant_id, merchant_id) = require_merchant(&scope)?;
    let order_id = parse_order_id(&body)?;
    let row = sqlx::query("UPDATE payment_orders SET status = 'paid', tx_hash = $4, updated_at = now() WHERE tenant_id = $1 AND merchant_id = $2 AND id = $3 AND status IN ('pending', 'paid') RETURNING *")
        .bind(tenant_id)
        .bind(merchant_id)
        .bind(order_id)
        .bind(tx_hash)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound("payment_order_not_found".into()))?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn features(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = require_merchant(&scope)?;
    let rows = sqlx::query("SELECT feature_key, enabled, limits_json FROM merchant_features WHERE tenant_id = $1 ORDER BY feature_key")
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?;
    let total = rows.len();
    Ok(Json(
        json!({"status": "ok", "data": rows_to_json(rows), "total": total}),
    ))
}

pub async fn ai_quota(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = require_merchant(&scope)?;
    let row = sqlx::query("SELECT * FROM merchant_ai_quota WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(Json(json!({"status": "ok", "data": row.map(row_to_json)})))
}

pub async fn ai_usage(
    State(_state): State<AppState>,
    _headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<Value>> {
    let days = q.days.unwrap_or(7).clamp(1, 90);
    Ok(Json(
        json!({"status": "ok", "data": {"days": days, "total": 0, "items": []}}),
    ))
}

pub async fn audit_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = require_merchant(&scope)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query(
        "SELECT * FROM tenant_audit_logs WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(&state.pool)
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
    let (tenant_id, merchant_id) = require_merchant(&scope)?;
    let order_id = parse_order_id(&body)?;
    let row = sqlx::query("UPDATE payment_orders SET status = $4, updated_at = now() WHERE tenant_id = $1 AND merchant_id = $2 AND id = $3 RETURNING *")
        .bind(tenant_id)
        .bind(merchant_id)
        .bind(order_id)
        .bind(status)
        .fetch_optional(&state.pool)
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
