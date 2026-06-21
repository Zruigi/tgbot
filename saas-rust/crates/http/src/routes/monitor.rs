// Monitor routes - Wallet monitoring, transaction tracking, PnL summary
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Column, Row, TypeInfo};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::scope::Scope;
use uuid::Uuid;

use crate::auth::resolve_scope;
use crate::state::AppState;

// Helper to convert Uuid to i64 (takes lower 64 bits)
fn uuid_to_i64(uuid: &Uuid) -> i64 {
    let bytes = uuid.as_bytes();
    let mut result = 0i64;
    for (i, &byte) in bytes.iter().take(8).enumerate() {
        result |= (byte as i64) << (i * 8);
    }
    result
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListMonitorsQuery {
    pub active_only: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub network: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMonitorRequest {
    pub address: String,
    pub network: String,
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMonitorRemarkRequest {
    pub remark: String,
}

#[derive(Debug, Deserialize)]
pub struct TxMetaRequest {
    pub income_reason: Option<String>,
    pub expense_reason: Option<String>,
    pub payer: Option<String>,
    pub payee: Option<String>,
    pub counterparty: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BatchTxMetaRequest {
    pub transactions: Vec<TxMetaItem>,
}

#[derive(Debug, Deserialize)]
pub struct TxMetaItem {
    pub tx_id: String,
    pub income_reason: Option<String>,
    pub expense_reason: Option<String>,
    pub payer: Option<String>,
    pub payee: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListTransactionsQuery {
    pub period: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub monitor_id: Option<i64>,
    pub direction: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub annotated_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct PnlSummaryQuery {
    pub period: Option<String>, // day, week, month
    pub days: Option<i32>,
}

// ============================================================================
// Helper Functions
// ============================================================================

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

pub fn rows_to_json(rows: Vec<sqlx::postgres::PgRow>) -> Vec<Value> {
    rows.into_iter().map(row_to_json).collect()
}

pub fn get_period_boundaries(period: &str, days: Option<i32>) -> (i64, i64) {
    let now = chrono::Utc::now();
    let (start, end) = match period {
        "day" => {
            let start = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
            let start = DateTime::<Utc>::from_naive_utc_and_offset(start, Utc);
            let end = start + chrono::Duration::days(1);
            (start.timestamp_millis(), end.timestamp_millis())
        }
        "week" => {
            let days_since_monday = now.weekday().num_days_from_monday() as i64;
            let start = (now - chrono::Duration::days(days_since_monday))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let start = DateTime::<Utc>::from_naive_utc_and_offset(start, Utc);
            let end = start + chrono::Duration::days(7);
            (start.timestamp_millis(), end.timestamp_millis())
        }
        "month" => {
            let start = now.date_naive().with_day(1).unwrap().and_hms_opt(0, 0, 0).unwrap();
            let start = DateTime::<Utc>::from_naive_utc_and_offset(start, Utc);
            let end = (now + chrono::Duration::days(32)).date_naive().with_day(1).unwrap().and_hms_opt(0, 0, 0).unwrap();
            let end = DateTime::<Utc>::from_naive_utc_and_offset(end, Utc);
            (start.timestamp_millis(), end.timestamp_millis())
        }
        _ => {
            let d = days.unwrap_or(7);
            let start = now - chrono::Duration::days(d as i64);
            let end = now;
            (start.timestamp_millis(), end.timestamp_millis())
        }
    };
    (start, end)
}

// ============================================================================
// API Endpoints
// ============================================================================

// GET /api/monitors - List all monitors
pub async fn list_monitors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListMonitorsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let active_only = q.active_only.unwrap_or(true);
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let rows = sqlx::query(
        r#"
        SELECT
            m.monitor_id,
            m.user_id,
            m.address,
            m.network,
            m.remark,
            m.risk_level,
            m.is_active,
            m.inflow_20,
            m.outflow_20,
            m.profit_20,
            m.created_at,
            m.updated_at,
            COALESCE(
                (SELECT SUM(CASE WHEN t.direction = 'in' THEN t.amount ELSE 0 END)
                 FROM monitor_transactions t
                 WHERE t.monitor_id = m.monitor_id
                   AND t.tx_timestamp >= extract(epoch from now() - interval '20 days') * 1000),
                0
            ) AS inflow_20_calc,
            COALESCE(
                (SELECT SUM(CASE WHEN t.direction = 'out' THEN t.amount ELSE 0 END)
                 FROM monitor_transactions t
                 WHERE t.monitor_id = m.monitor_id
                   AND t.tx_timestamp >= extract(epoch from now() - interval '20 days') * 1000),
                0
            ) AS outflow_20_calc
        FROM address_monitors m
        WHERE m.tenant_id = $1
          AND ($2::bool IS NULL OR m.is_active = $2)
          AND ($3::text IS NULL OR m.network = $3)
        ORDER BY m.created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(&tenant_id)
    .bind(active_only)
    .bind(q.network)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let total = rows.len();
    Ok(Json(json!({ "status": "ok", "data": rows_to_json(rows), "total": total })))
}

// POST /api/monitors - Create a new monitor
pub async fn create_monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateMonitorRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    if body.address.trim().is_empty() {
        return Err(ApiError::BadRequest("address is required".into()));
    }

    let network = if body.network.is_empty() {
        "TRC20".to_string()
    } else {
        body.network.to_uppercase()
    };
    let remark = body.remark.unwrap_or_default();

    // Use user_id from scope or default
    let user_id = scope.user_id.map(|u| uuid_to_i64(&u)).unwrap_or(0);

    let row = sqlx::query(
        r#"
        INSERT INTO address_monitors (user_id, tenant_id, address, network, remark, is_active)
        VALUES ($1, $2, $3, $4, $5, true)
        ON CONFLICT (address, network, tenant_id)
        DO UPDATE SET is_active = true, remark = EXCLUDED.remark, updated_at = now()
        RETURNING *
        "#,
    )
    .bind(user_id)
    .bind(&tenant_id)
    .bind(body.address.trim())
    .bind(&network)
    .bind(&remark)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(json!({ "status": "ok", "data": row_to_json(row) })))
}

// DELETE /api/monitors/{monitor_id} - Delete a monitor
pub async fn delete_monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(monitor_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    // Soft delete - set is_active = false
    let result = sqlx::query(
        "UPDATE address_monitors SET is_active = false, updated_at = now() WHERE monitor_id = $1 AND tenant_id = $2 RETURNING monitor_id",
    )
    .bind(monitor_id)
    .bind(&tenant_id)
    .fetch_optional(&state.pool)
    .await?;

    if result.is_none() {
        return Err(ApiError::NotFound("monitor_not_found".into()));
    }

    Ok(Json(json!({ "status": "ok" })))
}

// POST /api/monitors/{monitor_id}/remark - Update monitor remark
pub async fn update_monitor_remark(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(monitor_id): Path<i64>,
    Json(body): Json<UpdateMonitorRemarkRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let row = sqlx::query(
        "UPDATE address_monitors SET remark = $1, updated_at = now() WHERE monitor_id = $2 AND tenant_id = $3 RETURNING *",
    )
    .bind(&body.remark)
    .bind(monitor_id)
    .bind(&tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("monitor_not_found".into()))?;

    Ok(Json(json!({ "status": "ok", "data": row_to_json(row) })))
}

// GET /api/monitors/pnl_summary - Get PnL summary
pub async fn pnl_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PnlSummaryQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let period = q.period.as_deref().unwrap_or("week");
    let days = q.days.unwrap_or(7);
    let (start_ms, end_ms) = get_period_boundaries(period, Some(days));

    // Get inflow and outflow
    let row = sqlx::query(
        r#"
        SELECT
            COALESCE(SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END), 0) AS inflow,
            COALESCE(SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END), 0) AS outflow,
            COUNT(*) AS tx_count
        FROM monitor_transactions
        WHERE tenant_id = $1
          AND tx_timestamp >= $2
          AND tx_timestamp < $3
        "#,
    )
    .bind(&tenant_id)
    .bind(start_ms)
    .bind(end_ms)
    .fetch_one(&state.pool)
    .await?;

    let inflow: f64 = row.try_get("inflow")?;
    let outflow: f64 = row.try_get("outflow")?;
    let tx_count: i64 = row.try_get("tx_count")?;
    let profit = inflow - outflow;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "inflow": inflow,
            "outflow": outflow,
            "profit": profit,
            "tx_count": tx_count,
            "period": period,
            "start_ms": start_ms,
            "end_ms": end_ms
        }
    })))
}

// POST /api/monitors/tx_meta - Batch update transaction metadata
pub async fn update_tx_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BatchTxMetaRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let mut updated = 0;
    for item in body.transactions {
        let tx_id = item.tx_id.trim();
        if tx_id.is_empty() {
            continue;
        }

        let result = sqlx::query(
            r#"
            UPDATE monitor_transactions
            SET income_reason = $1,
                expense_reason = $2,
                payer = $3,
                payee = $4,
                updated_at = now()
            WHERE tx_id = $5 AND tenant_id = $6
            "#,
        )
        .bind(item.income_reason.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()))
        .bind(item.expense_reason.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()))
        .bind(item.payer.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()))
        .bind(item.payee.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()))
        .bind(tx_id)
        .bind(&tenant_id)
        .execute(&state.pool)
        .await?;

        if result.rows_affected() > 0 {
            updated += 1;
        }
    }

    Ok(Json(json!({ "status": "ok", "updated": updated })))
}

// GET /api/monitors/export - Export transactions
pub async fn export_transactions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListTransactionsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let (start_ms, end_ms) = if let (Some(start), Some(end)) = (q.start_ms, q.end_ms) {
        (start, end)
    } else {
        let period = q.period.as_deref().unwrap_or("month");
        get_period_boundaries(period, None)
    };

    let limit = q.limit.unwrap_or(1000).clamp(1, 5000);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut sql = String::from(
        r#"
        SELECT t.*, m.address, m.network, m.remark AS monitor_remark
        FROM monitor_transactions t
        JOIN address_monitors m ON m.monitor_id = t.monitor_id
        WHERE t.tenant_id = $1
          AND t.tx_timestamp >= $2
          AND t.tx_timestamp < $3
        "#,
    );

    if let Some(ref monitor_id) = q.monitor_id {
        sql.push_str(&format!(" AND t.monitor_id = {}", monitor_id));
    }
    if let Some(ref direction) = q.direction {
        sql.push_str(&format!(" AND t.direction = '{}'", direction));
    }
    if q.annotated_only.unwrap_or(false) {
        sql.push_str(" AND (t.income_reason IS NOT NULL OR t.expense_reason IS NOT NULL OR t.payer IS NOT NULL OR t.payee IS NOT NULL)");
    }

    sql.push_str(" ORDER BY t.tx_timestamp DESC, t.tx_id DESC");
    sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

    let rows = sqlx::query(&sql)
        .bind(&tenant_id)
        .bind(start_ms)
        .bind(end_ms)
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(json!({ "status": "ok", "data": rows_to_json(rows), "count": rows.len() })))
}

// GET /api/monitors/transactions - List transactions
pub async fn list_transactions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListTransactionsQuery>,
) -> ApiResult<Json<Value>> {
    // Same as export_transactions but with different default limit
    export_transactions(State(state), headers, Query(q)).await
}

// GET /api/monitors/year_summary - Get year summary
pub async fn year_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let now = chrono::Utc::now();
    let year_start = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        now.date_naive().with_ordinal(1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
        Utc,
    );
    let start_ms = year_start.timestamp_millis();
    let end_ms = now.timestamp_millis();

    // Get monthly breakdown
    let rows = sqlx::query(
        r#"
        SELECT
            date_trunc('month', to_timestamp(tx_timestamp / 1000)) AS month,
            COALESCE(SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END), 0) AS inflow,
            COALESCE(SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END), 0) AS outflow,
            COUNT(*) AS tx_count
        FROM monitor_transactions
        WHERE tenant_id = $1
          AND tx_timestamp >= $2
          AND tx_timestamp < $3
        GROUP BY date_trunc('month', to_timestamp(tx_timestamp / 1000))
        ORDER BY month
        "#,
    )
    .bind(&tenant_id)
    .bind(start_ms)
    .bind(end_ms)
    .fetch_all(&state.pool)
    .await?;

    let monthly_data: Vec<Value> = rows
        .iter()
        .map(|row| {
            let month: DateTime<Utc> = row.try_get("month").unwrap_or(now);
            let inflow: f64 = row.try_get("inflow").unwrap_or(0.0);
            let outflow: f64 = row.try_get("outflow").unwrap_or(0.0);
            let tx_count: i64 = row.try_get("tx_count").unwrap_or(0);
            json!({
                "month": month.format("%Y-%m").to_string(),
                "inflow": inflow,
                "outflow": outflow,
                "profit": inflow - outflow,
                "tx_count": tx_count
            })
        })
        .collect();

    Ok(Json(json!({ "status": "ok", "data": monthly_data })))
}