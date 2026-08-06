use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tgbot_services::ad_pipeline;

pub struct QueryAdClustersTool {
    pool: Arc<PgPool>,
}

impl QueryAdClustersTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Tool for QueryAdClustersTool {
    fn name(&self) -> &str {
        "query_ad_clusters"
    }

    fn description(&self) -> &str {
        "查询当前租户已落库群消息中的广告聚合候选，不把没有数据误报为没有广告。"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "group_id": {
                    "type": "integer",
                    "description": "可选的群组或频道 ID"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "confirmed", "deleted", "all"],
                    "description": "广告聚合状态过滤"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "最多返回的聚合数量"
                },
                "window_minutes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 43200,
                    "description": "向前查询的时间窗口（分钟）"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let tenant_id = context
            .metadata
            .get("tenant_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(tenant_id) = tenant_id else {
            return ToolExecuteResult::fail(
                "query_ad_clusters requires an explicit tenant scope; refusing an unscoped query",
            );
        };

        let group_id = params
            .get("group_id")
            .and_then(Value::as_i64)
            .or_else(|| params.get("chat_id").and_then(Value::as_i64));
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        let limit = params
            .get("limit")
            .and_then(Value::as_i64)
            .unwrap_or(20)
            .clamp(1, 100);
        let window_minutes = params
            .get("window_minutes")
            .and_then(Value::as_i64)
            .unwrap_or(60)
            .clamp(1, 30 * 24 * 60);

        match ad_pipeline::query_clusters(
            &self.pool,
            Some(tenant_id),
            group_id,
            status,
            limit,
            window_minutes,
        )
        .await
        {
            Ok(data) => {
                let message = data
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| "广告聚合查询完成".to_string());
                ToolExecuteResult::ok_with_data(data, &message)
            }
            Err(error) => ToolExecuteResult::fail(&format!("广告聚合查询失败: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::AgentContext;
    use sqlx::postgres::PgPoolOptions;

    #[tokio::test]
    async fn rejects_an_unscoped_query_before_database_access() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/beta_unused")
            .expect("lazy pool should be constructible");
        let result = QueryAdClustersTool::new(Arc::new(pool))
            .execute(&serde_json::json!({}), &AgentContext::new(1))
            .await;

        assert!(!result.success);
        assert!(result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("explicit tenant scope")));
    }
}
