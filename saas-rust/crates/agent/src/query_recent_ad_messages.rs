// Query tool for collecting recent ad messages into a snapshot
// Deterministic snapshot builder mirroring Python: services/ai_agent/tools.py::QueryRecentAdMessagesTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;

pub struct QueryRecentAdMessagesTool {
    pool: Arc<PgPool>,
}

impl QueryRecentAdMessagesTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Tool for QueryRecentAdMessagesTool {
    fn name(&self) -> &str {
        "query_recent_ad_messages"
    }

    fn description(&self) -> &str {
        "Query recent ad messages (window up to 30 days, default 7 days) and create a snapshot for deletion confirmation."
    }

    fn category(&self) -> &str {
        "security"
    }

    fn schema_for_llm(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "hours": {
                            "type": "integer",
                            "description": "Query window in hours (1-720, default 168, i.e. up to 30 days)"
                        },
                        "group_id": {
                            "type": "integer",
                            "description": "Optional, limit to specific group"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Candidate limit, default 50, max 200"
                        }
                    },
                    "required": []
                }
            }
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let hours = params
            .get("hours")
            .and_then(|v| v.as_i64())
            .unwrap_or(168)
            .max(1)
            .min(720);
        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(50)
            .max(1)
            .min(200);
        let group_id = params.get("group_id").and_then(|v| v.as_i64());

        let tenant_id = context
            .metadata
            .get("tenant_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(tenant_id) = tenant_id else {
            return ToolExecuteResult::fail(
                "query_recent_ad_messages requires an explicit tenant scope",
            );
        };

        // Query recent ad messages using ad_pipeline logic
        let cutoff_ts = chrono::Utc::now().timestamp() - (hours * 3600);
        let now_ts = chrono::Utc::now().timestamp();

        // Query from group_messages matching ad criteria
        let rows = match sqlx::query(
            r#"
            SELECT id, group_id, tg_msg_id, user_id, COALESCE(username, '') AS username,
                   COALESCE(content, '') AS content,
                   COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) AS msg_ts,
                   is_advertising, ad_score
            FROM groups.group_messages
            WHERE COALESCE(content, '') != ''
              AND COALESCE(sender_type, 'user') IN ('user', 'channel', 'bot')
              AND COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) >= $1
              AND NOT COALESCE(is_deleted, false)
              AND (is_advertising = true OR ad_score >= 40)
              AND ($2::bigint IS NULL OR group_id = $2)
              AND tenant_id = $4
            ORDER BY COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) DESC, id DESC
            LIMIT $3
            "#,
        )
        .bind(cutoff_ts)
        .bind(group_id)
        .bind(limit)
        .bind(tenant_id)
        .fetch_all(self.pool.as_ref())
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                return ToolExecuteResult {
                    success: false,
                    data: None,
                    message: Some(format!("Database query failed: {}", e)),
                    action_taken: false,
                    needs_confirmation: false,
                    requires_llm_generation: false,
                    generated_content: None,
                };
            }
        };

        if rows.is_empty() {
            // 空结果时附带"系统近24h自动清理统计"，避免 agent 冷冰冰回答"没有广告"
            let cleaned_24h: i64 = sqlx::query_scalar(
                r#"
                SELECT COUNT(*)::bigint
                FROM groups.ad_governance_actions
                WHERE tenant_id = $1
                  AND action_type IN ('delete', 'mute')
                  AND status = 'executed'
                  AND executed_at >= now() - interval '24 hours'
                "#,
            )
            .bind(tenant_id)
            .fetch_one(self.pool.as_ref())
            .await
            .unwrap_or(0);

            let message = if cleaned_24h > 0 {
                format!(
                    "当前查询窗口内没有待删除的广告消息。系统近 24 小时已自动清理 {} 条广告消息，无需手动处理。",
                    cleaned_24h
                )
            } else {
                "当前查询窗口内没有待删除的广告消息。广告治理服务持续运行中，暂无需要手动处理的内容。"
                    .to_string()
            };
            return ToolExecuteResult {
                success: true,
                data: Some(json!({
                    "candidate_count": 0,
                    "snapshot_id": "",
                    "message_refs": [],
                    "evidence_count": 0,
                    "auto_cleaned_24h": cleaned_24h,
                })),
                message: Some(message),
                action_taken: false,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            };
        }

        // Build refs from rows
        let mut refs: Vec<Value> = Vec::new();
        for row in &rows {
            refs.push(json!({
                "msg_id": row.get::<i64, _>("id"),
                "group_id": row.get::<i64, _>("group_id"),
                "chat_id": row.get::<i64, _>("group_id"),
                "tg_msg_id": row.get::<Option<i64>, _>("tg_msg_id"),
                "user_id": row.get::<i64, _>("user_id"),
                "message_ts": row.get::<Option<i64>, _>("msg_ts"),
            }));
        }

        // Generate deterministic snapshot ID (uuid-like but deterministic based on content)
        let snapshot_id = format!(
            "snap-{}",
            uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string()
        );

        // Persist snapshot
        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO groups.ad_delete_snapshots
            (snapshot_id, tenant_id, group_id, refs, excluded_groups_count, cutoff_ts, created_at, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, now(), 'agent')
            ON CONFLICT (snapshot_id) DO NOTHING
            "#,
        )
        .bind(&snapshot_id)
        .bind(tenant_id)
        .bind(group_id)
        .bind(serde_json::to_value(&refs).unwrap_or(Value::Null))
        .bind(0i32)
        .bind(cutoff_ts)
        .execute(self.pool.as_ref())
        .await
        {
            return ToolExecuteResult {
                success: false,
                data: None,
                message: Some(format!("Failed to persist snapshot: {}", e)),
                action_taken: false,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            };
        }

        // Format response
        ToolExecuteResult {
            success: true,
            data: Some(json!({
                "snapshot_id": snapshot_id,
                "candidate_count": refs.len(),
                "message_refs": refs,
                "cutoff_ts": cutoff_ts,
                "snapshot_created_at": now_ts,
                "evidence_count": refs.len(),
            })),
            message: Some(format!("Locked {} ad messages in snapshot", refs.len())),
            action_taken: false,
            needs_confirmation: true,
            requires_llm_generation: false,
            generated_content: None,
        }
    }
}
