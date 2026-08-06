use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{Column, Row, TypeInfo};
use std::collections::HashMap;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::{CreateMerchantRequest, Merchant, OrderIdRequest, UpsertPlanRequest};
use uuid::Uuid;

use crate::auth::resolve_scope;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub merchant_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_merchants(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    // PG direct mode
    if let Some(ref pool) = state.pool {
        let rows = if let Some(status) = q.status {
            sqlx::query(
                r#"
                SELECT
                    m.id, m.owner_tg_user_id AS user_id,
                    COALESCE(NULLIF(m.owner_username, ''), NULLIF(m.merchant_name, ''), 'User ' || m.owner_tg_user_id::text) AS username,
                    m.created_at AS registered_at,
                    COALESCE(s.plan_code, m.plan_type, 'free') AS paid_option,
                    COALESCE(s.end_at, m.subscription_expires_at) AS expires_at,
                    (COALESCE(s.plan_code, m.plan_type, 'free') <> 'free'
                     OR COALESCE(s.status, m.subscription_status, '') IN ('active', 'active_subscription', 'paid', 'confirmed')
                    ) AS "isPaid"
                FROM merchants m
                LEFT JOIN LATERAL (
                    SELECT * FROM merchant_subscriptions s
                    WHERE s.merchant_id = m.id
                    ORDER BY COALESCE(s.end_at, s.created_at) DESC, s.created_at DESC
                    LIMIT 1
                ) s ON true
                WHERE m.status = $1
                ORDER BY m.updated_at DESC LIMIT $2 OFFSET $3
                "#,
            ).bind(status).bind(limit).bind(offset).fetch_all(pool).await?
        } else {
            sqlx::query(
                r#"SELECT m.id, m.owner_tg_user_id AS user_id,
                COALESCE(NULLIF(m.owner_username, ''), NULLIF(m.merchant_name, ''), 'User ' || m.owner_tg_user_id::text) AS username,
                m.created_at AS registered_at,
                COALESCE(s.plan_code, m.plan_type, 'free') AS paid_option,
                COALESCE(s.end_at, m.subscription_expires_at) AS expires_at,
                (COALESCE(s.plan_code, m.plan_type, 'free') <> 'free'
                 OR COALESCE(s.status, m.subscription_status, '') IN ('active', 'active_subscription', 'paid', 'confirmed')
                ) AS "isPaid"
                FROM merchants m
                LEFT JOIN LATERAL (SELECT * FROM merchant_subscriptions s WHERE s.merchant_id = m.id
                ORDER BY COALESCE(s.end_at, s.created_at) DESC, s.created_at DESC LIMIT 1) s ON true
                ORDER BY m.updated_at DESC LIMIT $1 OFFSET $2"#,
            ).bind(limit).bind(offset).fetch_all(pool).await?
        };
        let total = rows.len();
        return Ok(Json(
            json!({"status": "ok", "data": rows_to_json(rows), "total": total}),
        ));
    }

    // REST fallback — simplified (no JOINs via REST)
    let mut filters = vec![];
    if let Some(ref s) = q.status {
        filters.push(("status", s.as_str()));
    }
    match state.supabase.select("merchants", "id,tenant_id,merchant_name,owner_username,status,plan_type,subscription_status,created_at", &filters, Some(limit)).await {
        Ok(merchants) => {
            let total = merchants.len();
            Ok(Json(json!({"status": "ok", "data": merchants, "total": total})))
        }
        Err(e) => Err(ApiError::BadRequest(format!("REST error: {e}"))),
    }
}

pub async fn create_merchant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateMerchantRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    if body.merchant_name.trim().is_empty() {
        return Err(ApiError::BadRequest("merchant_name required".into()));
    }
    let tenant_id = body
        .tenant_id
        .unwrap_or_else(|| format!("tenant_{}", Uuid::new_v4().simple()));
    let mut tx = state.db()?.begin().await?;
    let merchant = sqlx::query_as::<_, Merchant>(
        r#"
        INSERT INTO merchants (tenant_id, owner_user_id, owner_tg_user_id, owner_username, merchant_name, status)
        VALUES ($1, $2, $3, $4, $5, 'pending')
        RETURNING *
        "#,
    )
    .bind(&tenant_id)
    .bind(body.owner_user_id)
    .bind(body.owner_tg_user_id)
    .bind(body.owner_username.as_deref())
    .bind(body.merchant_name.trim())
    .fetch_one(&mut *tx)
    .await?;
    if let Some(owner_user_id) = body.owner_user_id {
        sqlx::query(
            "INSERT INTO tenant_memberships (user_id, tenant_id, merchant_id, role) VALUES ($1, $2, $3, 'merchant_owner') ON CONFLICT (user_id, tenant_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(owner_user_id)
        .bind(&merchant.tenant_id)
        .bind(merchant.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"status": "ok", "data": merchant})))
}

pub async fn delete_merchant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(merchant_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let result = sqlx::query("DELETE FROM merchants WHERE id = $1")
        .bind(merchant_id)
        .execute(state.db()?)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("merchant_not_found".into()));
    }
    Ok(Json(json!({"status": "ok"})))
}

pub async fn confirm_onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let merchant_id = parse_uuid_field(&body, "merchant_id")?;
    let merchant = sqlx::query_as::<_, Merchant>(
        "UPDATE merchants SET onboarding_completed = true, status = 'active_free', updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(merchant_id)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::NotFound("merchant_not_found".into()))?;
    Ok(Json(json!({"status": "ok", "data": merchant})))
}

pub async fn list_plans(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    if scope.role.as_str() != "platform_admin" && scope.tenant_id.is_none() {
        return Err(ApiError::Forbidden);
    }

    // PG + REST dual-transport
    if let Some(ref pool) = state.pool {
        let plans = sqlx::query(
            "SELECT * FROM subscription_plans WHERE status = 'active' ORDER BY price_usdt ASC",
        )
        .fetch_all(pool)
        .await?;
        let total = plans.len();
        return Ok(Json(
            json!({"status": "ok", "data": rows_to_json(plans), "total": total}),
        ));
    }

    // REST fallback
    match state
        .supabase
        .select(
            "subscription_plans",
            "plan_code,plan_name,price_usdt,duration_days,features_json,status",
            &[("status", "active")],
            Some(100),
        )
        .await
    {
        Ok(plans) => {
            let total = plans.len();
            Ok(Json(json!({"status": "ok", "data": plans, "total": total})))
        }
        Err(e) => Err(ApiError::BadRequest(format!("REST error: {e}"))),
    }
}

pub async fn upsert_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertPlanRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let plan = sqlx::query(
        r#"
        INSERT INTO subscription_plans (plan_code, plan_name, price_usdt, duration_days, features_json, status, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (plan_code) DO UPDATE SET
            plan_name = EXCLUDED.plan_name,
            price_usdt = EXCLUDED.price_usdt,
            duration_days = EXCLUDED.duration_days,
            features_json = EXCLUDED.features_json,
            status = EXCLUDED.status,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(body.plan_code)
    .bind(body.plan_name)
    .bind(body.price_usdt)
    .bind(body.duration_days)
    .bind(body.features_json)
    .bind(body.status.unwrap_or_else(|| "active".into()))
    .fetch_one(state.db()?)
    .await?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(plan)})))
}

pub async fn list_subscriptions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let rows = sqlx::query(
        r#"
        SELECT s.*, m.merchant_name
        FROM merchant_subscriptions s
        JOIN merchants m ON m.id = s.merchant_id
        WHERE ($1::text IS NULL OR s.status = $1)
          AND ($2::uuid IS NULL OR s.merchant_id = $2)
        ORDER BY s.created_at DESC
        LIMIT $3
        "#,
    )
    .bind(q.status)
    .bind(q.merchant_id)
    .bind(limit)
    .fetch_all(state.db()?)
    .await?;
    let total = rows.len();
    Ok(Json(
        json!({"status": "ok", "data": rows_to_json(rows), "total": total}),
    ))
}

pub async fn list_payment_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let rows = sqlx::query(
        r#"
        SELECT p.*, m.merchant_name
        FROM payment_orders p
        JOIN merchants m ON m.id = p.merchant_id
        WHERE ($1::text IS NULL OR p.status = $1)
          AND ($2::uuid IS NULL OR p.merchant_id = $2)
        ORDER BY p.created_at DESC
        LIMIT $3
        "#,
    )
    .bind(q.status)
    .bind(q.merchant_id)
    .bind(limit)
    .fetch_all(state.db()?)
    .await?;
    let total = rows.len();
    Ok(Json(
        json!({"status": "ok", "data": rows_to_json(rows), "total": total}),
    ))
}

pub async fn confirm_payment_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OrderIdRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let row = sqlx::query("UPDATE payment_orders SET status = 'confirmed', updated_at = now() WHERE id = $1 RETURNING *")
        .bind(body.order_id)
        .fetch_optional(state.db()?)
        .await?
        .ok_or_else(|| ApiError::NotFound("payment_order_not_found".into()))?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn reject_payment_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OrderIdRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let result = sqlx::query(
        "UPDATE payment_orders SET status = 'rejected', updated_at = now() WHERE id = $1",
    )
    .bind(body.order_id)
    .execute(state.db()?)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("payment_order_not_found".into()));
    }
    Ok(Json(json!({"status": "ok"})))
}

pub async fn get_payment_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    payment_config_response(state).await
}

/// Merchant-scoped compatibility alias. The platform endpoint above remains
/// platform-admin-only; keeping a separate handler prevents the alias from
/// accidentally widening the platform route's authorization.
pub async fn get_merchant_payment_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_merchant()?;
    payment_config_response(state).await
}

async fn payment_config_response(state: AppState) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT * FROM platform_payment_configs WHERE id = 1")
        .fetch_optional(state.db()?)
        .await?;
    Ok(Json(json!({"status": "ok", "data": row.map(row_to_json)})))
}

pub async fn save_payment_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
    let chain_type = body
        .get("chain_type")
        .and_then(Value::as_str)
        .unwrap_or("TRC20");
    let payment_address = body
        .get("payment_address")
        .and_then(Value::as_str)
        .unwrap_or("");
    let qr_image_path = body.get("qr_image_path").and_then(Value::as_str);
    let enabled = body.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let row = sqlx::query(
        r#"
        INSERT INTO platform_payment_configs (id, chain_type, payment_address, qr_image_path, enabled, updated_at)
        VALUES (1, $1, $2, $3, $4, now())
        ON CONFLICT (id) DO UPDATE SET chain_type = EXCLUDED.chain_type, payment_address = EXCLUDED.payment_address,
        qr_image_path = EXCLUDED.qr_image_path, enabled = EXCLUDED.enabled, updated_at = now()
        RETURNING *
        "#,
    )
    .bind(chain_type)
    .bind(payment_address)
    .bind(qr_image_path)
    .bind(enabled)
    .fetch_one(state.db()?)
    .await?;
    Ok(Json(json!({"status": "ok", "data": row_to_json(row)})))
}

pub async fn runtime_statuses(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    let pool = state
        .pool
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("runtime status requires PostgreSQL".into()))?;
    let rows = sqlx::query(
        "SELECT id::text AS merchant_id, tenant_id, bot_id, bot_username, status, runtime_version, config_version, feature_version FROM merchants ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    let registered = state.bot_registry.list_entries().await;
    let telemetry = state.worker_telemetry.snapshot();
    let in_process_worker_telemetry_available =
        state.worker_enabled && telemetry.global.scheduler_started_at.is_some();
    let durable_global = sqlx::query(
        "SELECT worker_id, started_at, last_seen_at, scheduler_started_at,
                last_poll_at, last_successful_poll_at, last_poll_error_at,
                total_polls, total_claimed_jobs, total_completed_jobs,
                total_failed_jobs, in_flight_jobs
         FROM platform.worker_runtime_heartbeats
         WHERE last_seen_at >= now() - interval '2 minutes'
         ORDER BY last_seen_at DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let durable_tenant_rows = sqlx::query(
        "SELECT DISTINCT ON (tenant_id)
                tenant_id, worker_id, last_seen_at, claimed_jobs,
                completed_jobs, failed_jobs, in_flight_jobs,
                last_claimed_at, last_completed_at, last_failed_at
         FROM platform.worker_tenant_heartbeats
         WHERE last_seen_at >= now() - interval '2 minutes'
         ORDER BY tenant_id, last_seen_at DESC",
    )
    .fetch_all(pool)
    .await?;
    let durable_tenant_telemetry: HashMap<String, Value> = durable_tenant_rows
        .into_iter()
        .map(|row| {
            let tenant_id: String = row.get("tenant_id");
            let value = json!({
                "tenant_id": &tenant_id,
                "worker_id": row.get::<String, _>("worker_id"),
                "last_seen_at": row.get::<DateTime<Utc>, _>("last_seen_at"),
                "claimed_jobs": row.get::<i64, _>("claimed_jobs"),
                "completed_jobs": row.get::<i64, _>("completed_jobs"),
                "failed_jobs": row.get::<i64, _>("failed_jobs"),
                "in_flight_jobs": row.get::<i64, _>("in_flight_jobs"),
                "last_claimed_at": row.get::<Option<DateTime<Utc>>, _>("last_claimed_at"),
                "last_completed_at": row.get::<Option<DateTime<Utc>>, _>("last_completed_at"),
                "last_failed_at": row.get::<Option<DateTime<Utc>>, _>("last_failed_at"),
            });
            (tenant_id, value)
        })
        .collect();
    let durable_worker_telemetry_available = durable_global.is_some();
    let worker_telemetry_available =
        in_process_worker_telemetry_available && durable_worker_telemetry_available;
    let durable_worker_telemetry = durable_global.as_ref().map(|row| {
        json!({
            "worker_id": row.get::<String, _>("worker_id"),
            "started_at": row.get::<DateTime<Utc>, _>("started_at"),
            "last_seen_at": row.get::<DateTime<Utc>, _>("last_seen_at"),
            "scheduler_started_at": row.get::<Option<DateTime<Utc>>, _>("scheduler_started_at"),
            "last_poll_at": row.get::<Option<DateTime<Utc>>, _>("last_poll_at"),
            "last_successful_poll_at": row.get::<Option<DateTime<Utc>>, _>("last_successful_poll_at"),
            "last_poll_error_at": row.get::<Option<DateTime<Utc>>, _>("last_poll_error_at"),
            "total_polls": row.get::<i64, _>("total_polls"),
            "total_claimed_jobs": row.get::<i64, _>("total_claimed_jobs"),
            "total_completed_jobs": row.get::<i64, _>("total_completed_jobs"),
            "total_failed_jobs": row.get::<i64, _>("total_failed_jobs"),
            "in_flight_jobs": row.get::<i64, _>("in_flight_jobs"),
        })
    }).unwrap_or(Value::Null);

    let data: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let merchant_id: String = row.get("merchant_id");
            let tenant_id: String = row.get("tenant_id");
            let bot_id: Option<i64> = row.get("bot_id");
            let bot_username: Option<String> = row.get("bot_username");
            let merchant_status: String = row.get("status");
            let durable_tenant_telemetry_available =
                durable_tenant_telemetry.contains_key(&tenant_id);
            let disabled = matches!(
                merchant_status.as_str(),
                "disabled" | "suspended" | "inactive" | "deleted"
            );
            let active = bot_id.is_some_and(|id| {
                registered
                    .iter()
                    .any(|entry| entry.bot_id == id && entry.tenant_id == tenant_id)
            });
            let (status, reason) = if disabled {
                ("disabled", "merchant_disabled")
            } else if active {
                ("running", "active_webhook_registry")
            } else if bot_id.is_none() {
                ("stopped", "bot_not_configured")
            } else {
                ("stopped", "bot_not_loaded")
            };

            json!({
                "merchant_id": merchant_id,
                "tenant_id": tenant_id,
                "bot_id": bot_id,
                "bot_username": bot_username,
                "merchant_status": merchant_status,
                "status": status,
                "reason": reason,
                "source": "rust_webhook_registry",
                "worker_enabled": state.worker_enabled,
                "in_process_worker_telemetry_available": in_process_worker_telemetry_available,
                "worker_telemetry_available": worker_telemetry_available,
                "worker_telemetry": telemetry.tenants.get(&tenant_id).cloned().unwrap_or_default(),
                "durable_worker_telemetry_available": durable_tenant_telemetry_available,
                "durable_worker_telemetry": durable_tenant_telemetry
                    .get(&tenant_id)
                    .cloned()
                    .unwrap_or(Value::Null),
                "runtime_version": row.get::<i32, _>("runtime_version"),
                "config_version": row.get::<i32, _>("config_version"),
                "feature_version": row.get::<i32, _>("feature_version"),
            })
        })
        .collect();

    let total = data.len();
    Ok(Json(json!({
        "status": "ok",
        "data": data,
        "total": total,
        "source": "rust_webhook_registry",
        "in_process_worker_telemetry_available": in_process_worker_telemetry_available,
        "worker_telemetry_available": worker_telemetry_available,
        "worker_telemetry": telemetry.global,
        "durable_worker_telemetry_available": durable_worker_telemetry_available,
        "durable_worker_telemetry_source": "postgres_worker_runtime_heartbeats",
        "durable_worker_telemetry": durable_worker_telemetry,
    })))
}

// POST /api/platform/subscriptions/downgrade - Run subscription downgrade for expired subscriptions
pub async fn run_subscription_downgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    // Find expired subscriptions and downgrade them
    let rows = sqlx::query(
        r#"
        UPDATE merchant_subscriptions
        SET status = 'expired', updated_at = now()
        WHERE end_at < NOW()
          AND status IN ('active', 'active_subscription', 'paid')
        RETURNING *
        "#,
    )
    .fetch_all(state.db()?)
    .await?;

    let total = rows.len();

    // Also update merchant plan_type to 'free' for merchants with expired subscriptions
    let merchant_updates = sqlx::query(
        r#"
        UPDATE merchants m
        SET plan_type = 'free',
            subscription_status = 'expired',
            updated_at = now()
        WHERE EXISTS (
            SELECT 1 FROM merchant_subscriptions s
            WHERE s.merchant_id = m.id
              AND s.end_at < NOW()
              AND s.status IN ('active', 'active_subscription', 'paid')
        )
        AND m.plan_type <> 'free'
        RETURNING *
        "#,
    )
    .fetch_all(state.db()?)
    .await?;

    let merchant_count = merchant_updates.len();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "subscriptions_downgraded": total,
            "merchants_downgraded": merchant_count,
        },
        "message": format!("Downgraded {} subscriptions and {} merchants", total, merchant_count)
    })))
}

pub fn parse_uuid_field(body: &Value, key: &str) -> ApiResult<Uuid> {
    body.get(key)
        .and_then(Value::as_str)
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| ApiError::BadRequest(format!("{key} invalid")))
}

pub fn rows_to_json(rows: Vec<sqlx::postgres::PgRow>) -> Vec<Value> {
    rows.into_iter().map(row_to_json).collect()
}

pub fn row_to_json(row: sqlx::postgres::PgRow) -> Value {
    let mut map = serde_json::Map::new();
    for col in row.columns() {
        let name = col.name();
        let type_name = col.type_info().name();
        let value = match type_name {
            "UUID" => row.try_get::<Uuid, _>(name).map(|v| json!(v.to_string())),
            "TEXT" | "VARCHAR" | "BPCHAR" => row.try_get::<String, _>(name).map(Value::String),
            "INT2" => row.try_get::<i16, _>(name).map(|v| json!(v)),
            "INT4" => row.try_get::<i32, _>(name).map(|v| json!(v)),
            "INT8" => row.try_get::<i64, _>(name).map(|v| json!(v)),
            "FLOAT4" => row.try_get::<f32, _>(name).map(|v| json!(v)),
            "FLOAT8" | "NUMERIC" => row.try_get::<f64, _>(name).map(|v| json!(v)),
            "BOOL" => row.try_get::<bool, _>(name).map(|v| json!(v)),
            "TIMESTAMPTZ" => row
                .try_get::<DateTime<Utc>, _>(name)
                .map(|v| json!(v.to_rfc3339())),
            "JSON" | "JSONB" => row.try_get::<Value, _>(name),
            _ => row.try_get::<String, _>(name).map(Value::String),
        }
        .unwrap_or(Value::Null);
        map.insert(name.to_string(), value);
    }
    Value::Object(map)
}
