// Finance routes - Financial summary, reconciliation, categories
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Column, Row, TypeInfo};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::scope::Scope;
use uuid::Uuid;

use crate::auth::resolve_scope;
use crate::state::AppState;
use crate::routes::monitor::{get_period_boundaries, row_to_json as base_row_to_json, rows_to_json};

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
pub struct FinanceSummaryQuery {
    pub currency: Option<String>,
    pub days: Option<i32>,
    pub period: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReconcileRequest {
    pub ledger_id: String,
    pub account_code: String,
    pub amount: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CalculatePnlRequest {
    pub buy_price: f64,
    pub sell_price: f64,
    pub quantity: f64,
    pub fee_rate: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateReportRequest {
    pub month: String,
    pub format: Option<String>, // pdf, excel
    pub transactions: Option<Vec<TransactionPnlItem>>,
}

#[derive(Debug, Deserialize)]
pub struct TransactionPnlItem {
    pub buy_price: f64,
    pub sell_price: f64,
    pub quantity: f64,
}

// ============================================================================
// Helper Functions
// ============================================================================

fn row_to_json(row: sqlx::postgres::PgRow) -> Value {
    base_row_to_json(row)
}

// ============================================================================
// API Endpoints
// ============================================================================

// GET /api/finance/summary - Get financial summary
pub async fn get_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FinanceSummaryQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let days = q.days.unwrap_or(30);
    let (start_ms, end_ms) = if let (Some(start), Some(end)) = (&q.start, &q.end) {
        // Parse date strings if provided
        (
            start.parse::<i64>().unwrap_or(0),
            end.parse::<i64>().unwrap_or_else(|_| chrono::Utc::now().timestamp_millis()),
        )
    } else {
        let period = q.period.as_deref().unwrap_or("month");
        get_period_boundaries(period, Some(days))
    };

    // Get summary by currency/token
    let rows = sqlx::query(
        r#"
        SELECT
            COALESCE(l.network, 'unknown') AS network,
            l.token_symbol,
            COUNT(1) AS tx_count,
            SUM(CASE WHEN l.direction = 'in' THEN l.amount ELSE 0 END) AS total_in,
            SUM(CASE WHEN l.direction = 'out' THEN l.amount ELSE 0 END) AS total_out,
            SUM(CASE WHEN l.reconciled_status = 'UNRECONCILED' THEN 1 ELSE 0 END) AS unreconciled_count,
            SUM(CASE WHEN l.reconciled_status = 'UNRECONCILED' THEN l.base_fiat_value ELSE 0 END) AS unreconciled_value
        FROM finance_ledgers l
        WHERE l.tenant_id = $1
          AND l.tx_timestamp >= $2
          AND l.tx_timestamp < $3
        GROUP BY COALESCE(l.network, 'unknown'), l.token_symbol
        ORDER BY network, token_symbol
        "#,
    )
    .bind(&tenant_id)
    .bind(start_ms)
    .bind(end_ms)
    .fetch_all(&state.pool)
    .await?;

    let by_token: Vec<Value> = rows
        .iter()
        .map(|row| {
            let network: String = row.try_get("network").unwrap_or_default();
            let token_symbol: String = row.try_get("token_symbol").unwrap_or_default();
            let tx_count: i64 = row.try_get("tx_count").unwrap_or(0);
            let total_in: f64 = row.try_get("total_in").unwrap_or(0.0);
            let total_out: f64 = row.try_get("total_out").unwrap_or(0.0);
            let unreconciled_count: i64 = row.try_get("unreconciled_count").unwrap_or(0);
            let unreconciled_value: f64 = row.try_get("unreconciled_value").unwrap_or(0.0);

            json!({
                "network": network,
                "token_symbol": token_symbol,
                "tx_count": tx_count,
                "inflow": total_in,
                "outflow": total_out,
                "profit": total_in - total_out,
                "unreconciled_count": unreconciled_count,
                "unreconciled_value": unreconciled_value
            })
        })
        .collect();

    // Calculate totals
    let total_inflow: f64 = by_token.iter()
        .filter_map(|v| v.get("inflow").and_then(|v| v.as_f64()))
        .sum();
    let total_outflow: f64 = by_token.iter()
        .filter_map(|v| v.get("outflow").and_then(|v| v.as_f64()))
        .sum();
    let total_unreconciled: i64 = by_token.iter()
        .filter_map(|v| v.get("unreconciled_count").and_then(|v| v.as_i64()))
        .sum();

    // Get daily flow (today)
    let today_start = chrono::Utc::now().date_naive()
        .and_hms_opt(0, 0, 0).unwrap()
        .and_utc();
    let today_end = today_start + chrono::Duration::days(1);
    let daily_row = sqlx::query(
        r#"
        SELECT
            COALESCE(SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END), 0) AS daily_in,
            COALESCE(SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END), 0) AS daily_out
        FROM finance_ledgers
        WHERE tenant_id = $1
          AND tx_timestamp >= $2
          AND tx_timestamp < $3
        "#,
    )
    .bind(&tenant_id)
    .bind(today_start.timestamp_millis())
    .bind(today_end.timestamp_millis())
    .fetch_one(&state.pool)
    .await?;

    let daily_in: f64 = daily_row.try_get("daily_in").unwrap_or(0.0);
    let daily_out: f64 = daily_row.try_get("daily_out").unwrap_or(0.0);
    let daily_flow = daily_in - daily_out;

    // Calculate success rate (reconciled / total)
    let total_tx: i64 = by_token.iter()
        .filter_map(|v| v.get("tx_count").and_then(|v| v.as_i64()))
        .sum();
    let success_rate = if total_tx > 0 {
        ((total_tx - total_unreconciled) as f64 / total_tx as f64) * 100.0
    } else {
        100.0
    };

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "total_unreconciled": total_unreconciled,
            "total_inflow": total_inflow,
            "total_outflow": total_outflow,
            "daily_flow": daily_flow,
            "success_rate": success_rate,
            "tx_count": total_tx,
            "period_start_ms": start_ms,
            "period_end_ms": end_ms,
            "by_token": by_token
        }
    })))
}

// POST /api/finance/reconcile - Reconcile a ledger entry
pub async fn reconcile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ReconcileRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    // Validate ledger_id
    let ledger_uuid = Uuid::parse_str(&body.ledger_id)
        .map_err(|_| ApiError::BadRequest("invalid_ledger_id".into()))?;

    // Get the ledger entry
    let ledger_row = sqlx::query(
        "SELECT ledger_id, user_id, reconciled_status FROM finance_ledgers WHERE ledger_id = $1 AND tenant_id = $2",
    )
    .bind(ledger_uuid)
    .bind(&tenant_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("ledger_not_found".into()))?;

    let ledger_user_id: i64 = ledger_row.try_get("user_id")?;
    let current_status: String = ledger_row.try_get("reconciled_status")?;

    if current_status == "RECONCILED" {
        return Ok(Json(json!({ "status": "ok", "message": "already_reconciled" })));
    }

    // Validate account code
    let account_row = sqlx::query(
        "SELECT account_id FROM chart_of_accounts WHERE code = $1 AND is_active = true",
    )
    .bind(&body.account_code)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::BadRequest("invalid_account_code".into()))?;

    let account_id: i64 = account_row.try_get("account_id")?;

    // Perform reconciliation in a transaction
    let mut tx = state.pool.begin().await?;

    // Update the ledger
    sqlx::query(
        "UPDATE finance_ledgers SET account_id = $1, reconciled_status = 'RECONCILED', updated_at = now() WHERE ledger_id = $2",
    )
    .bind(account_id)
    .bind(ledger_uuid)
    .execute(&mut *tx)
    .await?;

    // Insert audit log
    let admin_userid = scope.user_id.map(|u| uuid_to_i64(&u)).unwrap_or(0);
    sqlx::query(
        r#"
        INSERT INTO finance_audit_logs (tenant_id, admin_userid, action, target_table, target_id, reason)
        VALUES ($1, $2, 'RECONCILE', 'finance_ledgers', $3, $4)
        "#,
    )
    .bind(&tenant_id)
    .bind(admin_userid)
    .bind(&body.ledger_id)
    .bind(format!("Reconciled to account {}", body.account_code))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(json!({ "status": "ok" })))
}

// GET /api/finance/categories - Get chart of accounts
pub async fn get_categories(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    let rows = sqlx::query(
        "SELECT account_id, code, name, type, parent_id, description, is_active FROM chart_of_accounts WHERE is_active = true ORDER BY code",
    )
    .fetch_all(&state.pool)
    .await?;

    // Build tree structure
    let mut node_map: std::collections::HashMap<i64, Value> = std::collections::HashMap::new();
    let mut roots: Vec<Value> = Vec::new();

    for row in &rows {
        let account_id: i64 = row.try_get("account_id")?;
        let code: String = row.try_get("code")?;
        let name: String = row.try_get("name")?;
        let account_type: String = row.try_get("type")?;
        let parent_id: Option<i64> = row.try_get("parent_id")?;
        let description: Option<String> = row.try_get("description")?;

        let node = json!({
            "account_id": account_id,
            "code": code,
            "name": name,
            "type": account_type,
            "parent_id": parent_id,
            "description": description,
            "children": Vec::<Value>::new()
        });

        node_map.insert(account_id, node);
    }

    // Build tree
    for row in &rows {
        let account_id: i64 = row.try_get("account_id")?;
        let parent_id: Option<i64> = row.try_get("parent_id")?;

        if let Some(pid) = parent_id {
            // First, get the child node (if exists)
            let child_node = node_map.get(&account_id).cloned();
            if let Some(child) = child_node {
                // Then get the parent node separately
                if let Some(parent_node) = node_map.get_mut(&pid) {
                    if let Some(children) = parent_node.get_mut("children").and_then(|v| v.as_array_mut()) {
                        children.push(child);
                    }
                }
            }
        } else {
            if let Some(node) = node_map.get(&account_id) {
                roots.push(node.clone());
            }
        }
    }

    Ok(Json(json!({ "status": "ok", "data": roots })))
}

// POST /api/finance/calculate_pnl - Calculate PnL
pub async fn calculate_pnl(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CalculatePnlRequest>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    let buy_price = body.buy_price;
    let sell_price = body.sell_price;
    let quantity = body.quantity;
    let fee_rate = body.fee_rate.unwrap_or(0.001);

    let cost = buy_price * quantity;
    let revenue = sell_price * quantity;
    let fee = (cost + revenue) * fee_rate;
    let pnl = revenue - cost - fee;
    let pnl_rate = if cost > 0.0 { (pnl / cost) * 100.0 } else { 0.0 };

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "cost": cost,
            "revenue": revenue,
            "fee": fee,
            "pnl": pnl,
            "pnl_rate": pnl_rate
        }
    })))
}

// POST /api/finance/report - Generate financial report
pub async fn generate_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GenerateReportRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, merchant_id) = scope.require_merchant()?;

    let transactions = body.transactions.unwrap_or_default();

    // Calculate summary
    let mut total_cost = 0.0;
    let mut total_revenue = 0.0;
    let mut total_fee = 0.0;
    let mut profitable_count = 0;
    let mut losing_count = 0;
    let fee_rate = 0.001; // Default fee rate

    for tx in &transactions {
        let cost = tx.buy_price * tx.quantity;
        let revenue = tx.sell_price * tx.quantity;
        let fee = (cost + revenue) * fee_rate;
        let pnl = revenue - cost - fee;

        total_cost += cost;
        total_revenue += revenue;
        total_fee += fee;

        if pnl > 0.0 {
            profitable_count += 1;
        } else {
            losing_count += 1;
        }
    }

    let total_pnl = total_revenue - total_cost - total_fee;
    let total_transactions = transactions.len();
    let win_rate = if total_transactions > 0 {
        (profitable_count as f64 / total_transactions as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "total_transactions": total_transactions,
            "total_cost": total_cost,
            "total_revenue": total_revenue,
            "total_fee": total_fee,
            "total_pnl": total_pnl,
            "win_rate": win_rate,
            "profitable_count": profitable_count,
            "losing_count": losing_count
        }
    })))
}