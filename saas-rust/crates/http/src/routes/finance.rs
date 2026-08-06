// Finance routes - Financial summary, reconciliation, categories
use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap},
    response::Response,
    Json,
};
use chrono::{DateTime, Datelike, NaiveDate};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::io::{Cursor, Write};
use tgbot_domain::error::{ApiError, ApiResult};
use uuid::Uuid;

use crate::auth::resolve_scope;
use crate::routes::monitor::{get_period_boundaries, rows_to_json};
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
pub struct FinanceSummaryQuery {
    pub currency: Option<String>,
    pub days: Option<i32>,
    pub period: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub monitor_id: Option<i64>,
    pub token_symbol: Option<String>,
    pub network: Option<String>,
    pub exclude_dust: Option<bool>,
    pub dust_threshold: Option<f64>,
    pub limit: Option<i64>,
    pub sync_chain: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub days: Option<i32>,
    pub action: Option<String>,
    pub target_table: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ReportDownloadQuery {
    pub month: Option<String>,
    pub format: Option<String>,
    pub period: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub monitor_id: Option<i64>,
    pub token_symbol: Option<String>,
    pub network: Option<String>,
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

fn resolve_time_bounds(
    period: Option<&str>,
    start: Option<&str>,
    end: Option<&str>,
    days: Option<i32>,
) -> (i64, i64) {
    match (
        start.and_then(parse_timestamp),
        end.and_then(parse_timestamp),
    ) {
        (Some(start_ms), Some(end_ms)) if start_ms < end_ms => (start_ms, end_ms),
        _ => get_period_boundaries(period.unwrap_or("month"), days),
    }
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

// GET /api/finance/summary - Get financial summary
pub async fn get_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FinanceSummaryQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    let days = q.days.unwrap_or(30);
    let (start_ms, end_ms) = resolve_time_bounds(
        q.period.as_deref(),
        q.start.as_deref(),
        q.end.as_deref(),
        Some(days),
    );
    let token_symbol = clean_filter(q.token_symbol.as_deref());
    let network = clean_filter(q.network.as_deref());
    let exclude_dust = q.exclude_dust.unwrap_or(false);
    let dust_threshold = q.dust_threshold.unwrap_or(1.0).max(0.0);

    // Get summary by currency/token
    let rows = sqlx::query(
        r#"
        SELECT
            COALESCE(l.network, 'unknown') AS network,
            l.token_symbol,
            COUNT(1) AS tx_count,
            SUM(CASE WHEN l.direction = 'in' THEN l.amount ELSE 0 END)::double precision AS total_in,
            SUM(CASE WHEN l.direction = 'out' THEN l.amount ELSE 0 END)::double precision AS total_out,
            SUM(CASE WHEN l.reconciled_status = 'UNRECONCILED' THEN 1 ELSE 0 END) AS unreconciled_count,
            SUM(CASE WHEN l.reconciled_status = 'UNRECONCILED' THEN l.base_fiat_value ELSE 0 END)::double precision AS unreconciled_value
        FROM finance_ledgers l
        WHERE l.tenant_id = $1
          AND l.tx_timestamp >= $2
          AND l.tx_timestamp < $3
          AND ($4::bigint IS NULL OR l.monitor_id = $4)
          AND ($5::text IS NULL OR UPPER(l.token_symbol) = UPPER($5))
          AND ($6::text IS NULL OR UPPER(l.network) = UPPER($6))
          AND (NOT $7 OR ABS(l.amount) >= $8)
        GROUP BY COALESCE(l.network, 'unknown'), l.token_symbol
        ORDER BY network, token_symbol
        "#,
    )
    .bind(&tenant_id)
    .bind(start_ms)
    .bind(end_ms)
    .bind(q.monitor_id)
    .bind(&token_symbol)
    .bind(&network)
    .bind(exclude_dust)
    .bind(dust_threshold)
    .fetch_all(state.db()?)
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
    let total_inflow: f64 = by_token
        .iter()
        .filter_map(|v| v.get("inflow").and_then(|v| v.as_f64()))
        .sum();
    let total_outflow: f64 = by_token
        .iter()
        .filter_map(|v| v.get("outflow").and_then(|v| v.as_f64()))
        .sum();
    let total_unreconciled: i64 = by_token
        .iter()
        .filter_map(|v| v.get("unreconciled_count").and_then(|v| v.as_i64()))
        .sum();

    // Get daily flow (today)
    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let today_end = today_start + chrono::Duration::days(1);
    let daily_row = sqlx::query(
        r#"
        SELECT
            COALESCE(SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END), 0)::double precision AS daily_in,
            COALESCE(SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END), 0)::double precision AS daily_out
        FROM finance_ledgers
        WHERE tenant_id = $1
          AND tx_timestamp >= $2
          AND tx_timestamp < $3
        "#,
    )
    .bind(&tenant_id)
    .bind(today_start.timestamp_millis())
    .bind(today_end.timestamp_millis())
    .fetch_one(state.db()?)
    .await?;

    let daily_in: f64 = daily_row.try_get("daily_in").unwrap_or(0.0);
    let daily_out: f64 = daily_row.try_get("daily_out").unwrap_or(0.0);
    let daily_flow = daily_in - daily_out;

    // Calculate success rate (reconciled / total)
    let total_tx: i64 = by_token
        .iter()
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
    let (tenant_id, _merchant_id) = scope.require_merchant()?;

    // Validate ledger_id
    let ledger_uuid = Uuid::parse_str(&body.ledger_id)
        .map_err(|_| ApiError::BadRequest("invalid_ledger_id".into()))?;

    // Get the ledger entry
    let ledger_row = sqlx::query(
        "SELECT ledger_id, user_id, reconciled_status FROM finance_ledgers WHERE ledger_id = $1 AND tenant_id = $2",
    )
    .bind(ledger_uuid)
    .bind(&tenant_id)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::NotFound("ledger_not_found".into()))?;

    let _ledger_user_id: i64 = ledger_row.try_get("user_id")?;
    let current_status: String = ledger_row.try_get("reconciled_status")?;

    if current_status == "RECONCILED" {
        return Ok(Json(
            json!({ "status": "ok", "message": "already_reconciled" }),
        ));
    }

    // Validate account code
    let account_row = sqlx::query(
        "SELECT account_id FROM chart_of_accounts WHERE code = $1 AND is_active = true",
    )
    .bind(&body.account_code)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::BadRequest("invalid_account_code".into()))?;

    let account_id: i64 = account_row.try_get("account_id")?;

    // Perform reconciliation in a transaction
    let mut tx = state.db()?.begin().await?;

    // Update the ledger
    sqlx::query(
        "UPDATE finance_ledgers SET account_id = $1, reconciled_status = 'RECONCILED', updated_at = now() WHERE ledger_id = $2 AND tenant_id = $3",
    )
    .bind(account_id)
    .bind(ledger_uuid)
    .bind(&tenant_id)
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
    .fetch_all(state.db()?)
    .await?;

    // Build tree structure
    let mut node_map: std::collections::HashMap<i64, Value> = std::collections::HashMap::new();
    let mut child_ids: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    let mut root_ids = Vec::new();

    for row in &rows {
        let account_id: i64 = row.try_get("account_id")?;
        let code: String = row.try_get("code")?;
        let name: String = row.try_get("name")?;
        let account_type: String = row.try_get("type")?;
        let parent_id: Option<i64> = row.try_get("parent_id")?;
        let description: Option<String> = row.try_get("description")?;

        let node = json!({
            "id": account_id.to_string(),
            "account_id": account_id,
            "code": code,
            "name": name,
            "type": account_type,
            "parent_id": parent_id,
            "description": description,
            "children": Vec::<Value>::new()
        });

        node_map.insert(account_id, node);
        if let Some(parent_id) = parent_id {
            child_ids.entry(parent_id).or_default().push(account_id);
        } else {
            root_ids.push(account_id);
        }
    }

    fn build_category_tree(
        account_id: i64,
        nodes: &std::collections::HashMap<i64, Value>,
        children_by_parent: &std::collections::HashMap<i64, Vec<i64>>,
    ) -> Option<Value> {
        let mut node = nodes.get(&account_id)?.clone();
        let children = children_by_parent
            .get(&account_id)
            .into_iter()
            .flatten()
            .filter_map(|child_id| build_category_tree(*child_id, nodes, children_by_parent))
            .collect();
        if let Some(object) = node.as_object_mut() {
            object.insert("children".to_string(), Value::Array(children));
        }
        Some(node)
    }
    let roots: Vec<Value> = root_ids
        .into_iter()
        .filter_map(|account_id| build_category_tree(account_id, &node_map, &child_ids))
        .collect();

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
    let pnl_rate = if cost > 0.0 {
        (pnl / cost) * 100.0
    } else {
        0.0
    };

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
    let (_tenant_id, _merchant_id) = scope.require_merchant()?;

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

pub async fn get_unreconciled(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FinanceSummaryQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let (start_ms, end_ms) =
        if query.period.is_some() || query.start.is_some() || query.end.is_some() {
            resolve_time_bounds(
                query.period.as_deref(),
                query.start.as_deref(),
                query.end.as_deref(),
                query.days,
            )
        } else {
            (0, chrono::Utc::now().timestamp_millis() + 86_400_000)
        };
    let token_symbol = clean_filter(query.token_symbol.as_deref());
    let network = clean_filter(query.network.as_deref());
    let limit = query.limit.unwrap_or(500).clamp(1, 1000);

    let rows = sqlx::query(
        r#"
        SELECT l.ledger_id, l.monitor_id, l.direction, l.amount, l.token_symbol,
               l.base_fiat_value, l.fiat_currency, l.exchange_rate, l.counterparty,
               l.tx_id, l.tx_hash, l.tx_timestamp, l.created_at, l.reconciled_status,
               COALESCE(m.network, l.network, 'unknown') AS network,
               a.code, a.name
        FROM finance_ledgers l
        LEFT JOIN address_monitors m ON m.monitor_id = l.monitor_id AND m.tenant_id = l.tenant_id
        LEFT JOIN chart_of_accounts a ON a.account_id = l.account_id
        WHERE l.tenant_id = $1
          AND l.reconciled_status = 'UNRECONCILED'
          AND l.tx_timestamp >= $2
          AND l.tx_timestamp < $3
          AND ($4::bigint IS NULL OR l.monitor_id = $4)
          AND ($5::text IS NULL OR UPPER(l.token_symbol) = UPPER($5))
          AND ($6::text IS NULL OR UPPER(l.network) = UPPER($6))
        ORDER BY l.tx_timestamp DESC
        LIMIT $7
        "#,
    )
    .bind(&tenant_id)
    .bind(start_ms)
    .bind(end_ms)
    .bind(query.monitor_id)
    .bind(token_symbol)
    .bind(network)
    .bind(limit)
    .fetch_all(state.db()?)
    .await?;

    let count = rows.len();
    Ok(Json(json!({
        "status": "ok",
        "data": rows_to_json(rows),
        "count": count
    })))
}

pub async fn get_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let rows = sqlx::query(
        r#"
        SELECT token_symbol
        FROM (
            SELECT DISTINCT token_symbol FROM finance_ledgers WHERE tenant_id = $1
            UNION
            SELECT DISTINCT token_symbol FROM monitor_transactions WHERE tenant_id = $1
        ) tokens
        WHERE token_symbol IS NOT NULL AND token_symbol <> ''
        ORDER BY token_symbol
        "#,
    )
    .bind(&tenant_id)
    .fetch_all(state.db()?)
    .await?;
    let tokens: Vec<String> = rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>("token_symbol").ok())
        .collect();

    Ok(Json(json!({ "status": "ok", "data": tokens })))
}

pub async fn get_audit_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuditLogQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let days = query.days.unwrap_or(30).clamp(1, 365);
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let action = clean_filter(query.action.as_deref());
    let target_table = clean_filter(query.target_table.as_deref());

    let rows = sqlx::query(
        r#"
        SELECT audit_id, timestamp, admin_userid, action, target_table, target_id,
               reason, ip_address, before_state, after_state
        FROM finance_audit_logs
        WHERE tenant_id = $1
          AND timestamp >= now() - make_interval(days => $2)
          AND ($3::text IS NULL OR action = $3)
          AND ($4::text IS NULL OR target_table = $4)
        ORDER BY timestamp DESC
        LIMIT $5
        "#,
    )
    .bind(&tenant_id)
    .bind(days)
    .bind(action)
    .bind(target_table)
    .bind(limit)
    .fetch_all(state.db()?)
    .await?;

    Ok(Json(json!({ "status": "ok", "data": rows_to_json(rows) })))
}

#[derive(Debug)]
struct ReportRow {
    transaction_id: String,
    timestamp: i64,
    direction: String,
    amount: f64,
    token_symbol: String,
    network: String,
    counterparty: String,
    reconciled_status: String,
}

fn month_bounds(month: &str) -> Option<(i64, i64)> {
    let start = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").ok()?;
    let (next_year, next_month) = if start.month() == 12 {
        (start.year() + 1, 1)
    } else {
        (start.year(), start.month() + 1)
    };
    let end = NaiveDate::from_ymd_opt(next_year, next_month, 1)?;
    Some((
        start.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis(),
        end.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis(),
    ))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn spreadsheet_column(mut index: usize) -> String {
    let mut name = String::new();
    loop {
        name.insert(0, (b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name
}

fn build_xlsx(headers: &[&str], rows: &[Vec<String>]) -> anyhow::Result<Vec<u8>> {
    let mut sheet = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (row_index, row) in std::iter::once(headers.iter().map(|item| item.to_string()).collect())
        .chain(rows.iter().cloned())
        .enumerate()
    {
        sheet.push_str(&format!("<row r=\"{}\">", row_index + 1));
        for (column_index, value) in row.iter().enumerate() {
            let reference = format!("{}{}", spreadsheet_column(column_index), row_index + 1);
            sheet.push_str(&format!(
                "<c r=\"{reference}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
                xml_escape(value)
            ));
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");

    let cursor = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(cursor);
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let files = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Finance" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
    ];
    for (name, contents) in files {
        archive.start_file(name, options)?;
        archive.write_all(contents.as_bytes())?;
    }
    archive.start_file("xl/worksheets/sheet1.xml", options)?;
    archive.write_all(sheet.as_bytes())?;
    Ok(archive.finish()?.into_inner())
}

fn pdf_escape(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                '?'
            }
        })
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn build_pdf(month: &str, rows: &[ReportRow]) -> Vec<u8> {
    let total_inflow: f64 = rows
        .iter()
        .filter(|row| row.direction == "in")
        .map(|row| row.amount)
        .sum();
    let total_outflow: f64 = rows
        .iter()
        .filter(|row| row.direction == "out")
        .map(|row| row.amount)
        .sum();
    let mut lines = vec![
        format!("On-chain Finance Report {month}"),
        format!("Transactions: {}", rows.len()),
        format!("Total inflow: {total_inflow:.8}"),
        format!("Total outflow: {total_outflow:.8}"),
        String::new(),
        "Timestamp | Direction | Amount | Token | Network | Transaction".to_string(),
    ];
    lines.extend(rows.iter().take(48).map(|row| {
        format!(
            "{} | {} | {:.8} | {} | {} | {}",
            row.timestamp,
            row.direction,
            row.amount,
            row.token_symbol,
            row.network,
            row.transaction_id
        )
    }));

    let mut stream = String::from("BT\n/F1 8 Tf\n40 800 Td\n11 TL\n");
    for line in lines {
        stream.push_str(&format!("({}) Tj\nT*\n", pdf_escape(&line)));
    }
    stream.push_str("ET\n");

    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}endstream", stream.len(), stream),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );
    pdf
}

pub async fn download_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReportDownloadQuery>,
) -> ApiResult<Response> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _merchant_id) = scope.require_merchant()?;
    let month = query
        .month
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m").to_string());
    let (start_ms, end_ms) = month_bounds(&month).unwrap_or_else(|| {
        resolve_time_bounds(
            query.period.as_deref(),
            query.start.as_deref(),
            query.end.as_deref(),
            Some(30),
        )
    });
    let token_symbol = clean_filter(query.token_symbol.as_deref());
    let network = clean_filter(query.network.as_deref());
    let rows = sqlx::query(
        r#"
        SELECT COALESCE(l.tx_id, l.tx_hash, l.ledger_id::text) AS transaction_id,
               l.tx_timestamp, l.direction, l.amount::double precision AS amount, l.token_symbol, l.network,
               COALESCE(l.counterparty, '') AS counterparty, l.reconciled_status
        FROM finance_ledgers l
        WHERE l.tenant_id = $1
          AND l.tx_timestamp >= $2
          AND l.tx_timestamp < $3
          AND ($4::bigint IS NULL OR l.monitor_id = $4)
          AND ($5::text IS NULL OR UPPER(l.token_symbol) = UPPER($5))
          AND ($6::text IS NULL OR UPPER(l.network) = UPPER($6))
        ORDER BY l.tx_timestamp DESC
        LIMIT 5000
        "#,
    )
    .bind(&tenant_id)
    .bind(start_ms)
    .bind(end_ms)
    .bind(query.monitor_id)
    .bind(token_symbol)
    .bind(network)
    .fetch_all(state.db()?)
    .await?;
    let report_rows: Vec<ReportRow> = rows
        .iter()
        .map(|row| ReportRow {
            transaction_id: row.try_get("transaction_id").unwrap_or_default(),
            timestamp: row.try_get("tx_timestamp").unwrap_or_default(),
            direction: row.try_get("direction").unwrap_or_default(),
            amount: row.try_get("amount").unwrap_or_default(),
            token_symbol: row.try_get("token_symbol").unwrap_or_default(),
            network: row.try_get("network").unwrap_or_default(),
            counterparty: row.try_get("counterparty").unwrap_or_default(),
            reconciled_status: row.try_get("reconciled_status").unwrap_or_default(),
        })
        .collect();

    let format = query.format.as_deref().unwrap_or("pdf").to_lowercase();
    let (contents, content_type, extension) = match format.as_str() {
        "excel" | "xlsx" => {
            let rows: Vec<Vec<String>> = report_rows
                .iter()
                .map(|row| {
                    vec![
                        row.transaction_id.clone(),
                        row.timestamp.to_string(),
                        row.direction.clone(),
                        row.amount.to_string(),
                        row.token_symbol.clone(),
                        row.network.clone(),
                        row.counterparty.clone(),
                        row.reconciled_status.clone(),
                    ]
                })
                .collect();
            (
                build_xlsx(
                    &[
                        "transaction_id",
                        "timestamp",
                        "direction",
                        "amount",
                        "token_symbol",
                        "network",
                        "counterparty",
                        "reconciled_status",
                    ],
                    &rows,
                )
                .map_err(ApiError::Anyhow)?,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "xlsx",
            )
        }
        "pdf" => (build_pdf(&month, &report_rows), "application/pdf", "pdf"),
        _ => return Err(ApiError::BadRequest("format must be pdf or excel".into())),
    };
    let filename = format!("onchain-finance-{month}.{extension}");

    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from(contents))
        .map_err(|error| ApiError::Anyhow(error.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn parses_supported_timestamps() {
        assert_eq!(parse_timestamp("1711929600000"), Some(1_711_929_600_000));
        assert_eq!(parse_timestamp("2024-04-01"), Some(1_711_929_600_000));
        assert_eq!(
            parse_timestamp("2024-04-01T00:00:00Z"),
            Some(1_711_929_600_000)
        );
    }

    #[test]
    fn calculates_month_boundaries() {
        let (start, end) = month_bounds("2024-12").unwrap();
        assert_eq!(start, 1_733_011_200_000);
        assert_eq!(end, 1_735_689_600_000);
        assert!(month_bounds("2024-13").is_none());
    }

    #[test]
    fn generates_valid_xlsx_archive() {
        let bytes = build_xlsx(
            &["transaction_id", "amount"],
            &[vec!["tx-1".into(), "12.5".into()]],
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut sheet = String::new();
        archive
            .by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        assert!(sheet.contains("transaction_id"));
        assert!(sheet.contains("tx-1"));
    }

    #[test]
    fn generates_valid_pdf_envelope() {
        let bytes = build_pdf("2024-04", &[]);
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));
    }
}
