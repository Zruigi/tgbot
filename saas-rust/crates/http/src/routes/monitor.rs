// Monitor routes - Wallet monitoring, transaction tracking, PnL summary
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{Column, Row, TypeInfo};
use tgbot_domain::error::{ApiError, ApiResult};
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
pub struct DeleteMonitorRequest {
    pub monitor_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct TxMetaRequest {
    pub income_reason: Option<String>,
    pub expense_reason: Option<String>,
    pub payer: Option<String>,
    pub payee: Option<String>,
    pub counterparty: Option<String>,
    pub counterparty_label: Option<String>,
    pub network: Option<String>,
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
    pub start: Option<String>,
    pub end: Option<String>,
    pub monitor_id: Option<i64>,
    pub direction: Option<String>,
    pub token_symbol: Option<String>,
    pub network: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub annotated_only: Option<bool>,
    pub exclude_dust: Option<bool>,
    pub dust_threshold: Option<f64>,
    pub sync_chain: Option<bool>,
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
            "FLOAT8" => row.try_get::<f64, _>(name).map(|v| json!(v)),
            "NUMERIC" => row
                .try_get::<rust_decimal::Decimal, _>(name)
                .map(|v| json!(v.to_string().parse::<f64>().unwrap_or_default())),
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
            let start = now
                .date_naive()
                .with_day(1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let start = DateTime::<Utc>::from_naive_utc_and_offset(start, Utc);
            let end = (now + chrono::Duration::days(32))
                .date_naive()
                .with_day(1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
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

fn parse_timestamp(value: &str) -> Option<i64> {
    value
        .parse::<i64>()
        .ok()
        .or_else(|| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|date| date.timestamp_millis())
        })
        .or_else(|| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date| date.and_utc().timestamp_millis())
        })
}

fn clean_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty() && !item.eq_ignore_ascii_case("all"))
        .map(str::to_string)
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

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
                   AND t.tenant_id = m.tenant_id
                   AND t.tx_timestamp >= extract(epoch from now() - interval '20 days') * 1000),
                0
            ) AS inflow_20_calc,
            COALESCE(
                (SELECT SUM(CASE WHEN t.direction = 'out' THEN t.amount ELSE 0 END)
                 FROM monitor_transactions t
                 WHERE t.monitor_id = m.monitor_id
                   AND t.tenant_id = m.tenant_id
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
    .fetch_all(state.db()?)
    .await?;

    let total = rows.len();
    Ok(Json(
        json!({ "status": "ok", "data": rows_to_json(rows), "total": total }),
    ))
}

// POST /api/monitors - Create a new monitor
pub async fn create_monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateMonitorRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

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
    .fetch_one(state.db()?)
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    // Soft delete - set is_active = false
    let result = sqlx::query(
        "UPDATE address_monitors SET is_active = false, updated_at = now() WHERE monitor_id = $1 AND tenant_id = $2 RETURNING monitor_id",
    )
    .bind(monitor_id)
    .bind(&tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    if result.is_none() {
        return Err(ApiError::NotFound("monitor_not_found".into()));
    }

    Ok(Json(json!({ "status": "ok" })))
}

pub async fn delete_monitor_body(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DeleteMonitorRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let result = sqlx::query(
        "UPDATE address_monitors SET is_active = false, updated_at = now() \
         WHERE monitor_id = $1 AND tenant_id = $2",
    )
    .bind(body.monitor_id)
    .bind(tenant_id)
    .execute(state.db()?)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("monitor_not_found".into()));
    }
    Ok(Json(json!({"status": "ok"})))
}

// POST /api/monitors/{monitor_id}/remark - Update monitor remark
pub async fn update_monitor_remark(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(monitor_id): Path<i64>,
    Json(body): Json<UpdateMonitorRemarkRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let row = sqlx::query(
        "UPDATE address_monitors SET remark = $1, updated_at = now() WHERE monitor_id = $2 AND tenant_id = $3 RETURNING *",
    )
    .bind(&body.remark)
    .bind(monitor_id)
    .bind(&tenant_id)
    .fetch_optional(state.db()?)
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let period = q.period.as_deref().unwrap_or("week");
    let days = q.days.unwrap_or(7);
    let (start_ms, end_ms) = get_period_boundaries(period, Some(days));

    // Get inflow and outflow
    let row = sqlx::query(
        r#"
        SELECT
            COALESCE(SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END), 0)::double precision AS inflow,
            COALESCE(SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END), 0)::double precision AS outflow,
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
    .fetch_one(state.db()?)
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

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
        .bind(
            item.income_reason
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
        )
        .bind(
            item.expense_reason
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
        )
        .bind(
            item.payer
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
        )
        .bind(
            item.payee
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
        )
        .bind(tx_id)
        .bind(&tenant_id)
        .execute(state.db()?)
        .await?;

        if result.rows_affected() > 0 {
            updated += 1;
        }
    }

    Ok(Json(json!({ "status": "ok", "updated": updated })))
}

pub async fn update_transaction_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tx_id): Path<String>,
    Json(body): Json<TxMetaRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let tx_id = tx_id.trim();
    if tx_id.is_empty() {
        return Err(ApiError::BadRequest("tx_id is required".into()));
    }

    let clean = |value: Option<String>| {
        value
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
    };
    let income_reason = clean(body.income_reason);
    let expense_reason = clean(body.expense_reason);
    let payer = clean(body.payer);
    let payee = clean(body.payee);
    let counterparty = clean(body.counterparty);
    let network = clean(body.network).map(|item| item.to_uppercase());
    let counterparty_label = clean(body.counterparty_label);

    let mut transaction = state.db()?.begin().await?;
    let row = sqlx::query(
        r#"
        UPDATE monitor_transactions
        SET income_reason = COALESCE($1, income_reason),
            expense_reason = COALESCE($2, expense_reason),
            payer = COALESCE($3, payer),
            payee = COALESCE($4, payee),
            counterparty = COALESCE($5, counterparty),
            network = COALESCE($6, network),
            updated_at = now()
        WHERE tx_id = $7 AND tenant_id = $8
        RETURNING counterparty, network
        "#,
    )
    .bind(income_reason)
    .bind(expense_reason)
    .bind(payer)
    .bind(payee)
    .bind(counterparty)
    .bind(network)
    .bind(tx_id)
    .bind(&tenant_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| ApiError::NotFound("transaction_not_found".into()))?;

    if let Some(label) = counterparty_label {
        let row_counterparty: Option<String> = row.try_get("counterparty")?;
        let row_network: String = row.try_get("network")?;
        let row_counterparty = row_counterparty
            .filter(|item| !item.trim().is_empty())
            .ok_or_else(|| ApiError::BadRequest("counterparty is required for label".into()))?;

        sqlx::query(
            r#"
            INSERT INTO monitor_counterparty_labels (tenant_id, counterparty, network, label)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (tenant_id, counterparty, network)
            DO UPDATE SET label = EXCLUDED.label, updated_at = now()
            "#,
        )
        .bind(&tenant_id)
        .bind(row_counterparty)
        .bind(row_network)
        .bind(label)
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;
    Ok(Json(json!({ "status": "ok", "updated": 1 })))
}

// GET /api/monitors/export - Export transactions
pub async fn export_transactions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListTransactionsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let parsed_start = q
        .start_ms
        .or_else(|| q.start.as_deref().and_then(parse_timestamp));
    let parsed_end = q
        .end_ms
        .or_else(|| q.end.as_deref().and_then(parse_timestamp));
    let (start_ms, end_ms) = if let (Some(start), Some(end)) = (parsed_start, parsed_end) {
        (start, end)
    } else {
        let period = q.period.as_deref().unwrap_or("month");
        get_period_boundaries(period, None)
    };

    let limit = q.limit.unwrap_or(1000).clamp(1, 5000);
    let offset = q.offset.unwrap_or(0).max(0);
    let direction = clean_filter(q.direction.as_deref());
    if direction
        .as_deref()
        .is_some_and(|value| value != "in" && value != "out")
    {
        return Err(ApiError::BadRequest("direction must be in or out".into()));
    }
    let token_symbol = clean_filter(q.token_symbol.as_deref());
    let network = clean_filter(q.network.as_deref());
    let annotated_only = q.annotated_only.unwrap_or(false);
    let exclude_dust = q.exclude_dust.unwrap_or(false);
    let dust_threshold = q.dust_threshold.unwrap_or(1.0).max(0.0);

    let rows = sqlx::query(
        r#"
        SELECT t.*, m.remark AS monitor_remark,
               labels.label AS counterparty_label,
               COALESCE((
                   SELECT COUNT(*) FROM monitor_transactions related
                   WHERE related.tenant_id = t.tenant_id
                     AND related.counterparty = t.counterparty
                     AND related.network = t.network
               ), 0) AS counterparty_tx_count
        FROM monitor_transactions t
        JOIN address_monitors m
          ON m.monitor_id = t.monitor_id
         AND m.tenant_id = t.tenant_id
        LEFT JOIN monitor_counterparty_labels labels
          ON labels.tenant_id = t.tenant_id
         AND labels.counterparty = t.counterparty
         AND labels.network = t.network
        WHERE t.tenant_id = $1
          AND t.tx_timestamp >= $2
          AND t.tx_timestamp < $3
          AND ($4::bigint IS NULL OR t.monitor_id = $4)
          AND ($5::text IS NULL OR t.direction = $5)
          AND ($6::text IS NULL OR UPPER(t.token_symbol) = UPPER($6))
          AND ($7::text IS NULL OR UPPER(t.network) = UPPER($7))
          AND (NOT $8 OR t.income_reason IS NOT NULL OR t.expense_reason IS NOT NULL OR t.payer IS NOT NULL OR t.payee IS NOT NULL)
          AND (NOT $9 OR ABS(t.amount) >= $10)
        ORDER BY t.tx_timestamp DESC, t.tx_id DESC
        LIMIT $11 OFFSET $12
        "#,
    )
    .bind(&tenant_id)
    .bind(start_ms)
    .bind(end_ms)
    .bind(q.monitor_id)
    .bind(direction)
    .bind(token_symbol)
    .bind(network)
    .bind(annotated_only)
    .bind(exclude_dust)
    .bind(dust_threshold)
    .bind(limit)
    .bind(offset)
    .fetch_all(state.db()?)
    .await?;

    let count = rows.len();
    Ok(Json(
        json!({ "status": "ok", "data": rows_to_json(rows), "count": count }),
    ))
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let now = chrono::Utc::now();
    let year_start = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        now.date_naive()
            .with_ordinal(1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        Utc,
    );
    let start_ms = year_start.timestamp_millis();
    let end_ms = now.timestamp_millis();

    // Get monthly breakdown
    let rows = sqlx::query(
        r#"
        SELECT
            date_trunc('month', to_timestamp(tx_timestamp / 1000)) AS month,
            COALESCE(SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END), 0)::double precision AS inflow,
            COALESCE(SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END), 0)::double precision AS outflow,
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
    .fetch_all(state.db()?)
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

// ============================================================================
// Missing Endpoints — Phase 9 additions
// ============================================================================

/// POST /api/monitors/{monitor_id}/claim — Claim a user address as admin monitor
pub async fn claim_monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(monitor_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _) = scope.require_merchant()?;

    let result = sqlx::query(
        "UPDATE address_monitors SET is_active = true, updated_at = now() \
         WHERE monitor_id = $1 AND tenant_id = $2 AND is_active = false \
         RETURNING monitor_id",
    )
    .bind(monitor_id)
    .bind(&tenant_id)
    .fetch_optional(state.db()?)
    .await?;

    match result {
        Some(_) => Ok(Json(json!({"status": "ok"}))),
        None => Err(ApiError::NotFound("monitor_not_found".into())),
    }
}

/// POST /api/monitors/tx_meta/batch — Batch update transaction metadata
pub async fn batch_update_tx_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BatchTxMetaRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let mut updated = 0usize;
    for tx in &body.transactions {
        let result = sqlx::query(
            "UPDATE monitor_transactions SET \
             income_reason = COALESCE($2, income_reason), \
             expense_reason = COALESCE($3, expense_reason), \
             payer = COALESCE($4, payer), \
             payee = COALESCE($5, payee), \
             updated_at = now() \
             WHERE tx_id = $1 AND tenant_id = $6",
        )
        .bind(&tx.tx_id)
        .bind(&tx.income_reason)
        .bind(&tx.expense_reason)
        .bind(&tx.payer)
        .bind(&tx.payee)
        .bind(&tenant_id)
        .execute(state.db()?)
        .await?;

        if result.rows_affected() > 0 {
            updated += 1;
        }
    }

    Ok(Json(json!({"status": "ok", "updated": updated})))
}

/// GET /api/monitors/counterparty_aliases — List counterparty labels
pub async fn list_counterparty_aliases(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let rows = sqlx::query(
        "SELECT id, counterparty, network, label, created_at, updated_at \
         FROM monitor_counterparty_labels WHERE tenant_id = $1 ORDER BY label ASC",
    )
    .bind(&tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let aliases: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<i64, _>("id").unwrap_or(0),
                "counterparty": r.try_get::<String, _>("counterparty").unwrap_or_default(),
                "network": r.try_get::<String, _>("network").unwrap_or_default(),
                "label": r.try_get::<String, _>("label").unwrap_or_default(),
            })
        })
        .collect();

    Ok(Json(json!({"status": "ok", "data": aliases})))
}

/// POST /api/monitors/counterparty_aliases — Upsert a counterparty label
pub async fn upsert_counterparty_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let counterparty = body
        .get("counterparty")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("counterparty is required".into()))?;
    let label = body
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("label is required".into()))?;
    let network = body
        .get("network")
        .and_then(|v| v.as_str())
        .unwrap_or("TRC20");

    sqlx::query(
        "INSERT INTO monitor_counterparty_labels (tenant_id, counterparty, network, label) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (tenant_id, counterparty, network) \
         DO UPDATE SET label = EXCLUDED.label, updated_at = now()",
    )
    .bind(&tenant_id)
    .bind(counterparty)
    .bind(network)
    .bind(label)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({"status": "ok"})))
}

/// DELETE /api/monitors/counterparty_aliases/{alias_id} — Delete a label
pub async fn delete_counterparty_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(alias_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let result =
        sqlx::query("DELETE FROM monitor_counterparty_labels WHERE id = $1 AND tenant_id = $2")
            .bind(alias_id)
            .bind(&tenant_id)
            .execute(state.db()?)
            .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("alias_not_found".into()));
    }
    Ok(Json(json!({"status": "ok"})))
}

/// GET /api/monitors/user_queries — List user-query addresses (non-admin)
pub async fn list_user_query_monitors(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let rows = sqlx::query(
        "SELECT m.monitor_id, m.address, m.network, m.remark, m.is_active, \
                COALESCE(m.inflow_20, 0) as inflow_20, \
                COALESCE(m.outflow_20, 0) as outflow_20, \
                COALESCE(m.profit_20, 0) as profit_20 \
         FROM address_monitors m \
         WHERE m.tenant_id = $1 AND m.is_active = true \
         ORDER BY m.updated_at DESC",
    )
    .bind(&tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let monitors: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "monitor_id": r.try_get::<i64, _>("monitor_id").unwrap_or(0),
                "address": r.try_get::<String, _>("address").unwrap_or_default(),
                "network": r.try_get::<String, _>("network").unwrap_or_default(),
                "remark": r.try_get::<Option<String>, _>("remark").unwrap_or(None),
                "inflow_20": r.try_get::<f64, _>("inflow_20").unwrap_or(0.0),
                "outflow_20": r.try_get::<f64, _>("outflow_20").unwrap_or(0.0),
                "profit_20": r.try_get::<f64, _>("profit_20").unwrap_or(0.0),
            })
        })
        .collect();

    Ok(Json(json!({"status": "ok", "data": monitors})))
}
