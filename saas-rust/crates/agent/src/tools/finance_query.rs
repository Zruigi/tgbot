// 财务数据查询与整理工具
// 对应 Python: services/ai_agent/tools.py :: FinanceTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use chrono::Datelike;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::error;

// ---------------------------------------------------------------------------
// FinanceQueryTool
// ---------------------------------------------------------------------------

/// 财务数据查询与整理工具
///
/// 支持：
/// - summary: 多币种财务摘要
/// - unreconciled: 未核销流水
/// - categories: 科目树
/// - analysis: 综合分析（汇总 + 未核销 + 建议）
pub struct FinanceQueryTool {
    pool: Option<Arc<PgPool>>,
}

impl FinanceQueryTool {
    pub fn new() -> Self {
        Self { pool: None }
    }

    pub fn with_pool(pool: Arc<PgPool>) -> Self {
        Self { pool: Some(pool) }
    }
}

impl Default for FinanceQueryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FinanceQueryTool {
    fn name(&self) -> &str {
        "finance_query"
    }

    fn description(&self) -> &str {
        "查询和整理财务数据。支持：获取财务摘要、查看未核销流水、\
         获取科目树、生成报表摘要、测试 TRC20/BSC 链上 API。\
         可以自动整理财务数据并给出分析建议。"
    }

    fn category(&self) -> &str {
        "finance"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["summary", "unreconciled", "categories", "analysis"],
                    "description": "操作类型：summary=财务摘要, unreconciled=未核销流水, categories=科目树, analysis=综合分析"
                },
                "month": {
                    "type": "string",
                    "description": "月份筛选（如 '2026-04'），不填则查全部"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("summary");

        match action {
            "summary" => self.exec_summary(params, context).await,
            "unreconciled" => self.exec_unreconciled(params, context).await,
            "categories" => self.exec_categories(context).await,
            "analysis" => self.exec_analysis(params, context).await,
            _ => ToolExecuteResult::fail(&format!("未知操作: {}", action)),
        }
    }
}

impl FinanceQueryTool {
    /// 获取多币种财务摘要
    async fn exec_summary(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        match self.query_summary(params, context).await {
            Ok(data) => {
                let count = data
                    .get("by_currency")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                ToolExecuteResult::ok_with_data(data, &format!("已查询到 {count} 个币种的财务摘要"))
            }
            Err(error) => {
                error!(%error, "FinanceQueryTool summary failed");
                ToolExecuteResult::fail(&format!("财务摘要查询失败: {error}"))
            }
        }
    }

    /// 获取未核销流水
    async fn exec_unreconciled(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        match self.query_unreconciled(params, context).await {
            Ok(rows) => ToolExecuteResult::ok_with_data(
                Value::Array(rows.clone()),
                &format!("共 {} 条未核销记录", rows.len()),
            ),
            Err(error) => {
                error!(%error, "FinanceQueryTool unreconciled query failed");
                ToolExecuteResult::fail(&format!("未核销流水查询失败: {error}"))
            }
        }
    }

    /// 获取科目树
    async fn exec_categories(&self, context: &AgentContext) -> ToolExecuteResult {
        match self.query_categories(context).await {
            Ok(categories) => ToolExecuteResult::ok_with_data(
                Value::Array(categories.clone()),
                &format!("共 {} 个科目分类", categories.len()),
            ),
            Err(error) => {
                error!(%error, "FinanceQueryTool categories query failed");
                ToolExecuteResult::fail(&format!("科目查询失败: {error}"))
            }
        }
    }

    /// 综合分析：汇总 + 未核销 + 建议
    async fn exec_analysis(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let result = async {
            let summary = self.query_summary(params, context).await?;
            let unreconciled = self.query_unreconciled(params, context).await?;
            let summary_rows = summary
                .get("by_currency")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let total_unreconciled_value = unreconciled
                .iter()
                .filter_map(|row| row.get("base_fiat_value").and_then(Value::as_f64))
                .sum::<f64>();
            let analysis_text = build_analysis_text(&summary_rows, &unreconciled);
            Ok::<Value, String>(serde_json::json!({
                "summary": summary,
                "unreconciled_count": unreconciled.len(),
                "total_unreconciled_value": total_unreconciled_value,
                "analysis_text": analysis_text,
            }))
        }
        .await;

        match result {
            Ok(data) => ToolExecuteResult::ok_with_data(data, "综合分析已完成"),
            Err(error) => {
                error!(%error, "FinanceQueryTool analysis failed");
                ToolExecuteResult::fail(&format!("综合分析失败: {error}"))
            }
        }
    }

    async fn query_summary(&self, params: &Value, context: &AgentContext) -> Result<Value, String> {
        let pool = self
            .pool
            .as_deref()
            .ok_or_else(|| "PostgreSQL 财务数据源未配置".to_string())?;
        let tenant_id = context_tenant_id(context)?;
        let (start_ms, end_ms) = month_bounds(params)?;
        let rows = sqlx::query(
            r#"
            SELECT COALESCE(network, 'unknown') AS network, token_symbol,
                   COUNT(1) AS tx_count,
                   COALESCE(SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END), 0)::double precision AS total_in,
                   COALESCE(SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END), 0)::double precision AS total_out,
                   COALESCE(SUM(CASE WHEN reconciled_status = 'UNRECONCILED' THEN 1 ELSE 0 END), 0)::bigint AS unreconciled_count
            FROM finance_ledgers
            WHERE tenant_id = $1
              AND ($2::bigint IS NULL OR tx_timestamp >= $2)
              AND ($3::bigint IS NULL OR tx_timestamp < $3)
            GROUP BY COALESCE(network, 'unknown'), token_symbol
            ORDER BY network, token_symbol
            "#,
        )
        .bind(tenant_id)
        .bind(start_ms)
        .bind(end_ms)
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;

        let by_currency: Vec<Value> = rows
            .iter()
            .map(|row| {
                let total_in = row.try_get::<f64, _>("total_in").unwrap_or(0.0);
                let total_out = row.try_get::<f64, _>("total_out").unwrap_or(0.0);
                serde_json::json!({
                    "network": row.try_get::<String, _>("network").unwrap_or_default(),
                    "currency": row.try_get::<String, _>("token_symbol").unwrap_or_default(),
                    "token_symbol": row.try_get::<String, _>("token_symbol").unwrap_or_default(),
                    "tx_count": row.try_get::<i64, _>("tx_count").unwrap_or(0),
                    "total_income": total_in,
                    "total_expense": total_out,
                    "balance": total_in - total_out,
                    "unreconciled_count": row.try_get::<i64, _>("unreconciled_count").unwrap_or(0),
                })
            })
            .collect();
        let total_unreconciled = by_currency
            .iter()
            .filter_map(|row| row.get("unreconciled_count").and_then(Value::as_i64))
            .sum::<i64>();

        Ok(serde_json::json!({
            "by_currency": by_currency,
            "total_unreconciled": total_unreconciled,
            "as_of": chrono::Utc::now().to_rfc3339(),
        }))
    }

    async fn query_unreconciled(
        &self,
        params: &Value,
        context: &AgentContext,
    ) -> Result<Vec<Value>, String> {
        let pool = self
            .pool
            .as_deref()
            .ok_or_else(|| "PostgreSQL 财务数据源未配置".to_string())?;
        let tenant_id = context_tenant_id(context)?;
        let (start_ms, end_ms) = month_bounds(params)?;
        let rows = sqlx::query(
            r#"
            SELECT ledger_id::text AS ledger_id, monitor_id, direction,
                   amount::double precision AS amount, token_symbol,
                   base_fiat_value::double precision AS base_fiat_value,
                   fiat_currency, exchange_rate::double precision AS exchange_rate,
                   counterparty, COALESCE(tx_id, tx_hash) AS tx_hash, tx_timestamp,
                   reconciled_status, COALESCE(network, 'unknown') AS network
            FROM finance_ledgers
            WHERE tenant_id = $1 AND reconciled_status = 'UNRECONCILED'
              AND ($2::bigint IS NULL OR tx_timestamp >= $2)
              AND ($3::bigint IS NULL OR tx_timestamp < $3)
            ORDER BY tx_timestamp DESC
            LIMIT 50
            "#,
        )
        .bind(tenant_id)
        .bind(start_ms)
        .bind(end_ms)
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;

        Ok(rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "ledger_id": row.try_get::<String, _>("ledger_id").unwrap_or_default(),
                    "monitor_id": row.try_get::<Option<i64>, _>("monitor_id").ok().flatten(),
                    "direction": row.try_get::<String, _>("direction").unwrap_or_default(),
                    "amount": row.try_get::<f64, _>("amount").unwrap_or(0.0),
                    "token_symbol": row.try_get::<String, _>("token_symbol").unwrap_or_default(),
                    "base_fiat_value": row.try_get::<f64, _>("base_fiat_value").unwrap_or(0.0),
                    "fiat_currency": row.try_get::<String, _>("fiat_currency").unwrap_or_default(),
                    "exchange_rate": row.try_get::<Option<f64>, _>("exchange_rate").ok().flatten(),
                    "counterparty": row.try_get::<Option<String>, _>("counterparty").ok().flatten(),
                    "tx_hash": row.try_get::<Option<String>, _>("tx_hash").ok().flatten(),
                    "tx_timestamp": row.try_get::<i64, _>("tx_timestamp").unwrap_or(0),
                    "reconciled_status": row.try_get::<String, _>("reconciled_status").unwrap_or_default(),
                    "network": row.try_get::<String, _>("network").unwrap_or_default(),
                })
            })
            .collect())
    }

    async fn query_categories(&self, context: &AgentContext) -> Result<Vec<Value>, String> {
        let pool = self
            .pool
            .as_deref()
            .ok_or_else(|| "PostgreSQL 财务数据源未配置".to_string())?;
        context_tenant_id(context)?;
        let rows = sqlx::query(
            "SELECT account_id, code, name, type, parent_id, description \
             FROM chart_of_accounts WHERE is_active = true ORDER BY code",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;
        let flat = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.try_get::<i64, _>("account_id").unwrap_or_default(),
                    "account_id": row.try_get::<i64, _>("account_id").unwrap_or_default(),
                    "code": row.try_get::<String, _>("code").unwrap_or_default(),
                    "name": row.try_get::<String, _>("name").unwrap_or_default(),
                    "type": row.try_get::<String, _>("type").unwrap_or_default(),
                    "parent_id": row.try_get::<Option<i64>, _>("parent_id").ok().flatten(),
                    "description": row.try_get::<Option<String>, _>("description").ok().flatten(),
                    "children": [],
                })
            })
            .collect::<Vec<_>>();
        Ok(build_category_tree(flat))
    }
}

fn context_tenant_id(context: &AgentContext) -> Result<&str, String> {
    context
        .metadata
        .get("tenant_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "AgentContext 缺少 tenant_id".to_string())
}

fn month_bounds(params: &Value) -> Result<(Option<i64>, Option<i64>), String> {
    let Some(month) = params
        .get("month")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok((None, None));
    };
    let start_date = chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .map_err(|_| "month 必须使用 YYYY-MM 格式".to_string())?;
    let (next_year, next_month) = if start_date.month() == 12 {
        (start_date.year() + 1, 1)
    } else {
        (start_date.year(), start_date.month() + 1)
    };
    let end_date = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .ok_or_else(|| "month 超出有效范围".to_string())?;
    let start = start_date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "month 起始时间无效".to_string())?
        .and_utc()
        .timestamp_millis();
    let end = end_date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "month 结束时间无效".to_string())?
        .and_utc()
        .timestamp_millis();
    Ok((Some(start), Some(end)))
}

fn build_category_tree(rows: Vec<Value>) -> Vec<Value> {
    let mut nodes = std::collections::HashMap::<i64, Value>::new();
    let mut children = std::collections::HashMap::<i64, Vec<i64>>::new();
    let mut roots = Vec::new();
    for row in rows {
        let Some(id) = row.get("account_id").and_then(Value::as_i64) else {
            continue;
        };
        if let Some(parent_id) = row.get("parent_id").and_then(Value::as_i64) {
            children.entry(parent_id).or_default().push(id);
        } else {
            roots.push(id);
        }
        nodes.insert(id, row);
    }

    fn build_node(
        id: i64,
        nodes: &std::collections::HashMap<i64, Value>,
        children: &std::collections::HashMap<i64, Vec<i64>>,
    ) -> Option<Value> {
        let mut node = nodes.get(&id)?.clone();
        let nested = children
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|child_id| build_node(*child_id, nodes, children))
            .collect::<Vec<_>>();
        node.as_object_mut()?
            .insert("children".to_string(), Value::Array(nested));
        Some(node)
    }

    roots
        .into_iter()
        .filter_map(|id| build_node(id, &nodes, &children))
        .collect()
}

// ---------------------------------------------------------------------------
// 分析文本生成
// ---------------------------------------------------------------------------

/// 构建分析文本供 LLM 参考（对齐 Python 版 _build_analysis_text）
fn build_analysis_text(summary: &[Value], unreconciled: &[Value]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("财务分析报告\n".to_string());
    lines.push("=".repeat(30));

    for s in summary {
        let currency = s.get("currency").and_then(|v| v.as_str()).unwrap_or("?");
        let total_in = s
            .get("total_income")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let total_out = s
            .get("total_expense")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let balance = s.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0);

        lines.push(format!(
            "\n{}: 收入={:.2} | 支出={:.2} | 净额={:.2}",
            currency, total_in, total_out, balance
        ));
    }

    let total_unreconciled: f64 = unreconciled
        .iter()
        .map(|r| {
            r.get("base_fiat_value")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        })
        .sum();

    lines.push(format!(
        "\n待核销金额: ${:.2} ({}笔)",
        total_unreconciled,
        unreconciled.len()
    ));

    if unreconciled.len() > 10 {
        lines.push("建议尽快处理待核销流水，避免积压影响账目准确性".into());
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AgentContext, Tool};

    #[test]
    fn test_name_and_category() {
        let tool = FinanceQueryTool::new();
        assert_eq!(tool.name(), "finance_query");
        assert_eq!(tool.category(), "finance");
    }

    #[test]
    fn test_parameters_schema() {
        let tool = FinanceQueryTool::new();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("month").is_some());
        let actions = props["action"]["enum"].as_array().unwrap();
        assert!(!actions.iter().any(|action| action == "onchain_api_test"));
    }

    #[tokio::test]
    async fn test_exec_summary() {
        let tool = FinanceQueryTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "summary"});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("数据源未配置"));
    }

    #[tokio::test]
    async fn test_exec_unreconciled() {
        let tool = FinanceQueryTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "unreconciled"});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("数据源未配置"));
    }

    #[tokio::test]
    async fn test_exec_categories() {
        let tool = FinanceQueryTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "categories"});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("数据源未配置"));
    }

    #[tokio::test]
    async fn test_exec_analysis() {
        let tool = FinanceQueryTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "analysis"});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("数据源未配置"));
    }

    #[tokio::test]
    async fn test_exec_unknown_action() {
        let tool = FinanceQueryTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "unknown_action"});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("未知操作"));
    }

    #[tokio::test]
    async fn test_removed_global_credential_action_fails_closed() {
        let tool = FinanceQueryTool::new();
        let ctx = AgentContext::new(1);
        let result = tool
            .execute(&serde_json::json!({"action": "onchain_api_test"}), &ctx)
            .await;
        assert!(!result.success);
        assert!(!result.action_taken);
        assert!(result.message.unwrap().contains("未知操作"));
    }

    #[test]
    fn test_build_analysis_text() {
        let summary = vec![serde_json::json!({
            "currency": "USDT",
            "total_income": 1000.0,
            "total_expense": 600.0,
            "balance": 400.0,
        })];
        let unreconciled: Vec<Value> = vec![];
        let text = build_analysis_text(&summary, &unreconciled);
        assert!(text.contains("USDT"));
        assert!(text.contains("1000"));
    }
}
