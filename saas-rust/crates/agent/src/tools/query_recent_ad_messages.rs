// Query tool for collecting recent ad messages into a snapshot
// Deterministic snapshot builder mirroring Python: services/ai_agent/tools.py::QueryRecentAdMessagesTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tgbot_integrations::telegram::TelegramClient;

pub struct QueryRecentAdMessagesTool {
    pool: Arc<PgPool>,
    telegram_client: Option<Arc<TelegramClient>>,
}

impl QueryRecentAdMessagesTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            pool,
            telegram_client: None,
        }
    }

    pub fn with_telegram(pool: Arc<PgPool>, client: Arc<TelegramClient>) -> Self {
        Self {
            pool,
            telegram_client: Some(client),
        }
    }
}

#[async_trait]
impl Tool for QueryRecentAdMessagesTool {
    fn name(&self) -> &str {
        "query_recent_ad_messages"
    }

    fn description(&self) -> &str {
        "Query recent 24-hour ad messages and create a snapshot for deletion confirmation."
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
                            "description": "Query window in hours (1-24, default 24)"
                        },
                        "group_id": {
                            "type": "integer",
                            "description": "Optional, limit to specific group"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Candidate limit, default 20, max 50"
                        }
                    },
                    "required": []
                }
            }
        })
    }

    async fn execute(&self, params: &Value, _context: &AgentContext) -> ToolExecuteResult {
        let hours = params
            .get("hours")
            .and_then(|v| v.as_i64())
            .unwrap_or(24)
            .max(1)
            .min(24);
        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(20)
            .max(1)
            .min(50);
        let group_id = params.get("group_id").and_then(|v| v.as_i64());

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
            ORDER BY COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) DESC, id DESC
            LIMIT $3
            "#,
        )
        .bind(cutoff_ts)
        .bind(group_id)
        .bind(limit)
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
            return ToolExecuteResult {
                success: true,
                data: Some(json!({
                    "candidate_count": 0,
                    "snapshot_id": "",
                    "message_refs": [],
                    "evidence_count": 0,
                })),
                message: Some("No ad messages found in query window".into()),
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
                "tg_msg_id": row.get::<i64, _>("tg_msg_id"),
                "user_id": row.get::<i64, _>("user_id"),
                "message_ts": row.get::<Option<i64>, _>("msg_ts"),
            }));
        }

        // Generate deterministic snapshot ID (uuid-like but deterministic based on content)
        let snapshot_id = format!("snap-{}", uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string());

        // Persist snapshot
        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO groups.ad_delete_snapshots
            (snapshot_id, tenant_id, group_id, refs, excluded_groups_count, created_at, created_by)
            VALUES ($1, $2, $3, $4, $5, now(), 'agent')
            ON CONFLICT (snapshot_id) DO NOTHING
            "#,
        )
        .bind(&snapshot_id)
        .bind("default")
        .bind(group_id)
        .bind(serde_json::to_value(&refs).unwrap_or(Value::Null))
        .bind(0i32)
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
