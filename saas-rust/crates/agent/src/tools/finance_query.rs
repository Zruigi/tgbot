// 财务数据查询与整理工具
// 对应 Python: services/ai_agent/tools.py :: FinanceTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use tracing::{error, info};

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
/// - onchain_api_test: TRC20/BSC 链上 API 测试
pub struct FinanceQueryTool;

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
                    "enum": ["summary", "unreconciled", "categories", "analysis", "onchain_api_test"],
                    "description": "操作类型：summary=财务摘要, unreconciled=未核销流水, categories=科目树, analysis=综合分析, onchain_api_test=链上API测试"
                },
                "month": {
                    "type": "string",
                    "description": "月份筛选（如 '2026-04'），不填则查全部"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: &Value, _context: &AgentContext) -> ToolExecuteResult {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("summary");

        match action {
            "summary" => self.exec_summary(params).await,
            "unreconciled" => self.exec_unreconciled(params).await,
            "categories" => self.exec_categories(params).await,
            "analysis" => self.exec_analysis(params).await,
            "onchain_api_test" => self.exec_onchain_api_test().await,
            _ => ToolExecuteResult::fail(&format!("未知操作: {}", action)),
        }
    }
}

impl FinanceQueryTool {
    /// 获取多币种财务摘要
    async fn exec_summary(&self, _params: &Value) -> ToolExecuteResult {
        // TODO: 接入 tgbot-services 的财务服务后替换为真实数据库查询
        // 当前返回占位数据，结构对齐 Python 版 FinanceTool.summary
        info!("FinanceQueryTool::exec_summary — 占位实现");

        let placeholder_data = serde_json::json!({
            "by_currency": [],
            "total_base_fiat": 0.0,
            "as_of": chrono::Utc::now().to_rfc3339(),
        });

        ToolExecuteResult::ok_with_data(
            placeholder_data,
            "已查询到 0 个币种的财务摘要（数据服务尚未接入）",
        )
    }

    /// 获取未核销流水
    async fn exec_unreconciled(&self, _params: &Value) -> ToolExecuteResult {
        // TODO: 接入 tgbot-services 后替换
        info!("FinanceQueryTool::exec_unreconciled — 占位实现");

        ToolExecuteResult::ok_with_data(
            serde_json::json!({
                "rows": [],
                "count": 0,
                "total_base_fiat_value": 0.0,
            }),
            "共 0 条未核销记录（数据服务尚未接入）",
        )
    }

    /// 获取科目树
    async fn exec_categories(&self) -> ToolExecuteResult {
        // TODO: 接入 tgbot-services 后替换
        info!("FinanceQueryTool::exec_categories — 占位实现");

        ToolExecuteResult::ok_with_data(
            serde_json::json!({
                "categories": [],
                "count": 0,
            }),
            "共 0 个科目分类（数据服务尚未接入）",
        )
    }

    /// 综合分析：汇总 + 未核销 + 建议
    async fn exec_analysis(&self, _params: &Value) -> ToolExecuteResult {
        // TODO: 接入 tgbot-services 后替换
        info!("FinanceQueryTool::exec_analysis — 占位实现");

        let analysis_text = build_analysis_text(&[], &[]);

        ToolExecuteResult::ok_with_data(
            serde_json::json!({
                "summary": [],
                "unreconciled_count": 0,
                "total_unreconciled_value": 0.0,
                "analysis_text": analysis_text,
            }),
            "综合分析已完成（数据服务尚未接入）",
        )
    }

    /// 测试 TRC20 / BSC API 连通性和凭证有效性
    async fn exec_onchain_api_test(&self) -> ToolExecuteResult {
        info!("FinanceQueryTool::exec_onchain_api_test — 执行链上 API 测试");

        let mut trc20_result = serde_json::json!({
            "configured": false,
            "source": "none",
            "ok": false,
            "detail": "未配置 TRC20 API Key"
        });

        let mut bsc_result = serde_json::json!({
            "configured": false,
            "source": "none",
            "ok": false,
            "detail": "未配置 BSC API Key"
        });

        // 尝试从环境变量读取 API Key
        let trc_key = std::env::var("TRONGRID_API_KEY")
            .or_else(|_| std::env::var("DMV_API_KEY"))
            .or_else(|_| std::env::var("DMV_TOKEN"))
            .unwrap_or_default();

        let bsc_key = std::env::var("BSCSCAN_API_KEY")
            .or_else(|_| std::env::var("BC_API_KEY"))
            .or_else(|_| std::env::var("BC_TOKEN"))
            .unwrap_or_default();

        // TRC20 测试
        if !trc_key.is_empty() {
            trc20_result["configured"] = serde_json::json!(true);
            trc20_result["source"] = serde_json::json!("env");

            match test_trc20_api(&trc_key).await {
                Ok(status) => {
                    trc20_result["ok"] = serde_json::json!(status.ok);
                    trc20_result["http_status"] = serde_json::json!(status.http_status);
                    trc20_result["detail"] = serde_json::json!(
                        if status.ok {
                            "TronGrid 连通正常"
                        } else {
                            &status.detail
                        }
                    );
                }
                Err(e) => {
                    trc20_result["ok"] = serde_json::json!(false);
                    trc20_result["detail"] = serde_json::json!(format!("请求异常: {}", e));
                }
            }
        }

        // BSC 测试
        if !bsc_key.is_empty() {
            bsc_result["configured"] = serde_json::json!(true);
            bsc_result["source"] = serde_json::json!("env");

            match test_bsc_api(&bsc_key).await {
                Ok(status) => {
                    bsc_result["ok"] = serde_json::json!(status.ok);
                    bsc_result["http_status"] = serde_json::json!(status.http_status);
                    if status.ok {
                        bsc_result["latest_block"] = serde_json::json!(status.latest_block);
                    }
                    bsc_result["detail"] = serde_json::json!(
                        if status.ok {
                            "BscScan 连通正常"
                        } else {
                            &status.detail
                        }
                    );
                }
                Err(e) => {
                    bsc_result["ok"] = serde_json::json!(false);
                    bsc_result["detail"] = serde_json::json!(format!("请求异常: {}", e));
                }
            }
        }

        let trc_ok = trc20_result["ok"].as_bool().unwrap_or(false);
        let bsc_ok = bsc_result["ok"].as_bool().unwrap_or(false);
        let ok_count = usize::from(trc_ok) + usize::from(bsc_ok);

        let summary = format!(
            "链上 API 测试完成：{}/2 可用 (TRC20={}, BSC={})",
            ok_count, trc_ok, bsc_ok
        );

        ToolExecuteResult::ok_with_data(
            serde_json::json!({
                "trc20": trc20_result,
                "bsc": bsc_result,
                "summary": summary,
            }),
            &summary,
        )
    }
}

// ---------------------------------------------------------------------------
// 链上 API 测试辅助
// ---------------------------------------------------------------------------

struct ApiTestStatus {
    ok: bool,
    http_status: u16,
    detail: String,
    latest_block: Option<String>,
}

async fn test_trc20_api(api_key: &str) -> Result<ApiTestStatus, anyhow::Error> {
    let url = "https://api.trongrid.io/v1/contracts/TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t/events?limit=1";

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;

    let resp = client
        .get(url)
        .header("TRON-PRO-API-KEY", api_key)
        .send()
        .await?;

    let http_status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let is_json = content_type.starts_with("application/json");
    let body: Value = if is_json {
        resp.json().await.unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    let ok = http_status == 200 && body.get("success").and_then(|v| v.as_bool()).unwrap_or(true);

    let detail = if ok {
        "TronGrid 连通正常".to_string()
    } else {
        body.get("error")
            .or_else(|| body.get("message"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("HTTP {}", http_status))
    };

    Ok(ApiTestStatus {
        ok,
        http_status,
        detail,
        latest_block: None,
    })
}

async fn test_bsc_api(api_key: &str) -> Result<ApiTestStatus, anyhow::Error> {
    let url = format!(
        "https://api.bscscan.com/api?module=proxy&action=eth_blockNumber&apikey={}",
        api_key
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;

    let resp = client.get(&url).send().await?;

    let http_status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let is_json = content_type.starts_with("application/json");
    let body: Value = if is_json {
        resp.json().await.unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    let bsc_msg = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let bsc_result = body.get("result").and_then(|v| v.as_str());
    let ok = http_status == 200
        && bsc_result.is_some()
        && !bsc_msg.contains("invalid");

    let detail = if ok {
        "BscScan 连通正常".to_string()
    } else {
        if !bsc_msg.is_empty() {
            bsc_msg
        } else {
            let s = body.to_string();
            s.chars().take(120).collect()
        }
    };

    Ok(ApiTestStatus {
        ok,
        http_status,
        detail,
        latest_block: bsc_result.map(|s| s.to_string()),
    })
}

// ---------------------------------------------------------------------------
// 分析文本生成
// ---------------------------------------------------------------------------

/// 构建分析文本供 LLM 参考（对齐 Python 版 _build_analysis_text）
fn build_analysis_text(
    summary: &[Value],
    unreconciled: &[Value],
) -> String {
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
        let balance = s
            .get("balance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

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
        "\n待核销金额: ${:,.2} ({}笔)",
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
    use crate::core::Tool;
    use crate::schema::AgentContext;

    #[test]
    fn test_name_and_category() {
        let tool = FinanceQueryTool;
        assert_eq!(tool.name(), "finance_query");
        assert_eq!(tool.category(), "finance");
    }

    #[test]
    fn test_parameters_schema() {
        let tool = FinanceQueryTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("month").is_some());
    }

    #[tokio::test]
    async fn test_exec_summary() {
        let tool = FinanceQueryTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "summary"});
        let result = tool.execute(&params, &ctx).await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_exec_unreconciled() {
        let tool = FinanceQueryTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "unreconciled"});
        let result = tool.execute(&params, &ctx).await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_exec_categories() {
        let tool = FinanceQueryTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "categories"});
        let result = tool.execute(&params, &ctx).await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_exec_analysis() {
        let tool = FinanceQueryTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "analysis"});
        let result = tool.execute(&params, &ctx).await;
        assert!(result.success);
        let data = result.data.unwrap();
        assert!(data.get("analysis_text").is_some());
    }

    #[tokio::test]
    async fn test_exec_unknown_action() {
        let tool = FinanceQueryTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"action": "unknown_action"});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
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
