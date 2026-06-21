use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::{PlanCodeRequest, SaveSecretRefRequest, UpdateStatusRequest};
use uuid::Uuid;

// AES-256-GCM encryption for bot tokens
use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::auth::resolve_scope;
use crate::state::AppState;

use super::platform::{row_to_json, rows_to_json};

type HmacSha256 = Hmac<Sha256>;

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
    let (tenant_id, merchant_id) = scope.require_merchant()?;
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    if body.bot_token.trim().is_empty() {
        return Err(ApiError::BadRequest("bot_token required".into()));
    }

    // ============== FIXED: 使用 AES-256-GCM 加密 bot_token ==============
    let encrypted_token = encrypt_bot_token(&body.bot_token, &tenant_id)
        .map_err(|e| ApiError::BadRequest(format!("encryption failed: {}", e)))?;

    // 存储加密后的密文 (而非明文引用)
    let encrypted_ref = format!("encrypted:{}", encrypted_token);

    sqlx::query(
        "INSERT INTO tenant_secret_refs (tenant_id, secret_key, secret_ref, updated_at) VALUES ($1, 'TELEGRAM_BOT_TOKEN', $2, now()) ON CONFLICT (tenant_id, secret_key) DO UPDATE SET secret_ref = EXCLUDED.secret_ref, updated_at = now()",
    )
    .bind(&tenant_id)
    .bind(encrypted_ref)
    .execute(&state.pool)
    .await?;
    Ok(Json(
        json!({"status": "ok", "data": {"bot_token": "encrypted"}}),
    ))
}

// ============== AES-256-GCM 加密工具函数 ==============

fn encrypt_bot_token(plaintext: &str, tenant_id: &str) -> Result<String, String> {
    // 派生密钥: HMAC-SHA256(master_key, tenant_id)
    // Note: Exactly 32 bytes required for HMAC-SHA256 key
    const MASTER_KEY: &[u8; 32] = *b"abcdefghijklmnopqrstuvwxyz012";
    let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(MASTER_KEY).map_err(|e| e.to_string())?;
    mac.update(tenant_id.as_bytes());
    let derived_key: [u8; 32] = mac.finalize().into_bytes().into();

    // 生成随机 nonce
    let mut nonce_bytes = [0u8; 12];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    nonce_bytes[..8].copy_from_slice(&now.to_le_bytes());
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    (now, plaintext).hash(&mut hasher);
    nonce_bytes[8..].copy_from_slice(&hasher.finish().to_le_bytes()[..4]);

    // 加密
    let cipher = Aes256Gcm::new_from_slice(&derived_key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| e.to_string())?;

    // 组合: nonce (12) + ciphertext + tag (16)
    let mut result = nonce_bytes.to_vec();
    result.extend(ciphertext);

    Ok(BASE64.encode(&result))
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
    let (tenant_id, merchant_id) = scope.require_merchant()?;
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
    let (tenant_id, merchant_id) = scope.require_merchant()?;
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
    let (tenant_id, merchant_id) = scope.require_merchant()?;
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let row = sqlx::query("SELECT * FROM merchant_ai_quota WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(&state.pool)
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
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    // Get summary
    let summary_row = sqlx::query(
        r#"
        SELECT
            COUNT(*) AS total_requests,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            COALESCE(SUM(estimated_cost_usd), 0) AS total_cost,
            COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
            COALESCE(SUM(completion_tokens), 0) AS completion_tokens
        FROM ai_usage_logs
        WHERE tenant_id = $1
          AND created_at >= NOW() - INTERVAL '1 day' * $2
        "#,
    )
    .bind(&tenant_id)
    .bind(days)
    .fetch_optional(&state.pool)
    .await?;

    // Get daily breakdown
    let daily_rows = sqlx::query(
        r#"
        SELECT
            DATE(created_at) AS date,
            COUNT(*) AS requests,
            COALESCE(SUM(total_tokens), 0) AS tokens,
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
    .fetch_all(&state.pool)
    .await?;

    // Get by model
    let model_rows = sqlx::query(
        r#"
        SELECT
            model,
            COUNT(*) AS requests,
            COALESCE(SUM(total_tokens), 0) AS tokens,
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
    .fetch_all(&state.pool)
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
    let (tenant_id, merchant_id) = scope.require_merchant()?;
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
