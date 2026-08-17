// Delete tool for consuming ad message snapshots
// Consumes ONLY stored snapshots created by query_recent_ad_messages
// Corresponding Python: services/ai_agent/tools.py::DeleteRecentAdMessagesTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tgbot_integrations::telegram::TelegramClient;

pub struct DeleteRecentAdMessagesTool {
    pool: Arc<PgPool>,
    telegram_client: Option<Arc<TelegramClient>>,
}

impl DeleteRecentAdMessagesTool {
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
impl Tool for DeleteRecentAdMessagesTool {
    fn name(&self) -> &str {
        "delete_recent_ad_messages"
    }

    fn description(&self) -> &str {
        "Delete ad messages from a confirmed snapshot. Must be called with snapshot_id from query_recent_ad_messages."
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
                        "snapshot_id": {
                            "type": "string",
                            "description": "Snapshot ID from query_recent_ad_messages"
                        },
                        "confirm": {
                            "type": "boolean",
                            "description": "Must be true to execute actual deletion"
                        }
                    },
                    "required": ["snapshot_id"]
                }
            }
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let snapshot_id = match params.get("snapshot_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolExecuteResult {
                    success: false,
                    data: None,
                    message: Some("snapshot_id required".into()),
                    action_taken: false,
                    needs_confirmation: false,
                    requires_llm_generation: false,
                    generated_content: None,
                };
            }
        };

        let confirm = params
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 平台 admin 会话（ai.rs fallback "platform"）→ 视为 default 租户，与头像/财务回退策略一致
        let tenant_id: String = context
            .metadata
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|v| {
                if v == "platform" {
                    "default".to_string()
                } else {
                    v.to_string()
                }
            })
            .unwrap_or_else(|| "default".to_string());

        // Load snapshot with TENANT SCOPING (fix #3)
        let snapshot_row = match sqlx::query(
            "SELECT snapshot_id, tenant_id, refs, cutoff_ts, created_at, consumed_at FROM groups.ad_delete_snapshots WHERE snapshot_id = $1 AND tenant_id = $2"
        )
        .bind(snapshot_id)
        .bind(&tenant_id)
        .fetch_optional(self.pool.as_ref())
        .await
        {
            Ok(Some(row)) => row,
            Ok(None) => {
                return ToolExecuteResult {
                    success: false,
                    data: None,
                    message: Some(format!("Snapshot not found or belongs to different tenant: {}", snapshot_id)),
                    action_taken: false,
                    needs_confirmation: false,
                    requires_llm_generation: false,
                    generated_content: None,
                };
            }
            Err(e) => {
                return ToolExecuteResult {
                    success: false,
                    data: None,
                    message: Some(format!("Snapshot query failed: {}", e)),
                    action_taken: false,
                    needs_confirmation: false,
                    requires_llm_generation: false,
                    generated_content: None,
                };
            }
        };

        let created_at: chrono::DateTime<Utc> = snapshot_row.get("created_at");
        let consumed_at: Option<chrono::DateTime<Utc>> = snapshot_row.get("consumed_at");
        let refs_json: Value = snapshot_row.get("refs");
        // Query window lower bound stored at snapshot creation (may be NULL on legacy rows)
        let cutoff_ts: Option<i64> = snapshot_row.get("cutoff_ts");

        // Check if already consumed
        if consumed_at.is_some() {
            return ToolExecuteResult {
                success: false,
                data: None,
                message: Some("Snapshot already consumed (reuse prevention)".into()),
                action_taken: false,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            };
        }

        // Check TTL (15 minutes)
        let now = Utc::now();
        let age_secs = (now - created_at).num_seconds();
        if age_secs > 15 * 60 {
            return ToolExecuteResult {
                success: false,
                data: None,
                message: Some("Snapshot expired (>15 minutes old), please re-query".into()),
                action_taken: false,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            };
        }

        // KILL-SWITCH SUPREMACY (fix #2): compute dry_run correctly
        let ad_delete_enabled = tgbot_services::ad_pipeline::ad_delete_enabled();
        let dry_run = tgbot_services::ad_pipeline::effective_dry_run(!confirm, ad_delete_enabled);

        // FAIL-CLOSED: if no telegram client and not in dry_run, refuse deletion (fix #1)
        if !dry_run && self.telegram_client.is_none() {
            return ToolExecuteResult {
                success: false,
                data: Some(json!({
                    "deleted": 0,
                    "skipped": 0,
                    "errors": 0,
                    "dry_run": true,
                })),
                message: Some("Bot client unavailable, cannot execute deletion".into()),
                action_taken: false,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            };
        }

        let refs_array = match refs_json.as_array() {
            Some(arr) => arr,
            None => {
                return ToolExecuteResult {
                    success: false,
                    data: None,
                    message: Some("refs must be array".into()),
                    action_taken: false,
                    needs_confirmation: false,
                    requires_llm_generation: false,
                    generated_content: None,
                };
            }
        };

        // Re-verify and delete each row
        let mut deleted = 0i64;
        let mut skipped = 0i64;
        let mut error_details: Vec<Value> = Vec::new();
        // Upper bound: snapshot creation time (tolerate +60s clock skew)
        let snapshot_upper_ts = created_at.timestamp() + 60;

        for ref_item in refs_array {
            let msg_id = match ref_item.get("msg_id").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            let group_id = match ref_item.get("group_id").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            let tg_msg_id = match ref_item.get("tg_msg_id").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            let user_id = ref_item
                .get("user_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            // Re-verify row exists and matches criteria.
            // Time window semantics (mirrors Python): message_ts must fall inside
            // [snapshot cutoff, snapshot created_at]. NULL cutoff (legacy rows)
            // disables the lower bound; upper bound always enforced.
            let row_exists = match sqlx::query(
                r#"
                SELECT 1 FROM groups.group_messages
                WHERE id = $1
                  AND group_id = $2
                  AND tg_msg_id = $3
                  AND user_id = $4
                  AND (is_advertising = true OR ad_score >= 40)
                  AND NOT COALESCE(is_deleted, false)
                  AND ($5::bigint IS NULL OR COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) >= $5)
                  AND COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) <= $6
                LIMIT 1
                "#,
            )
            .bind(msg_id)
            .bind(group_id)
            .bind(tg_msg_id)
            .bind(user_id)
            .bind(cutoff_ts)
            .bind(snapshot_upper_ts)
            .fetch_optional(self.pool.as_ref())
            .await
            {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(_) => false,
            };

            if !row_exists {
                skipped += 1;
                error_details.push(json!({
                    "msg_id": msg_id,
                    "reason": "re_verification_failed"
                }));
                continue;
            }

            // In dry-run mode: skip claim and deletions, just count
            if dry_run {
                deleted += 1;
                continue;
            }

            // REAL EXECUTION: Claim with preemption
            let claimed = match sqlx::query(
                r#"
                UPDATE groups.group_messages
                SET ad_action_status = 'deleting'
                WHERE id = $1
                  AND COALESCE(ad_action_status, 'none') IN ('none', '')
                "#,
            )
            .bind(msg_id)
            .execute(self.pool.as_ref())
            .await
            {
                Ok(result) => result.rows_affected() > 0,
                Err(_) => false,
            };

            if !claimed {
                skipped += 1;
                error_details.push(json!({
                    "msg_id": msg_id,
                    "reason": "already_claimed_or_deleted"
                }));
                continue;
            }

            // REAL TELEGRAM DELETION (fix #1): call delete_message FIRST, then soft-delete
            let telegram_ok = if let Some(client) = self.telegram_client.as_ref() {
                match client.delete_message(group_id, tg_msg_id).await {
                    Ok(_) => true,
                    Err(e) => {
                        // Telegram failure: RESET claim so retry can happen (fix #1)
                        let _ = sqlx::query(
                            "UPDATE groups.group_messages SET ad_action_status = 'none' WHERE id = $1 AND ad_action_status = 'deleting'"
                        )
                        .bind(msg_id)
                        .execute(self.pool.as_ref())
                        .await;

                        error_details.push(json!({
                            "msg_id": msg_id,
                            "reason": format!("telegram_delete_failed: {}", e)
                        }));
                        false
                    }
                }
            } else {
                // Should not reach here (caught earlier), but be safe
                error_details.push(json!({
                    "msg_id": msg_id,
                    "reason": "bot_client_unavailable"
                }));
                false
            };

            if !telegram_ok {
                skipped += 1;
                continue;
            }

            // Soft-delete in DB: only on Telegram success
            match sqlx::query(
                r#"
                UPDATE groups.group_messages
                SET is_deleted = true,
                    deleted_at = now(),
                    ad_action_status = 'deleted'
                WHERE id = $1
                "#,
            )
            .bind(msg_id)
            .execute(self.pool.as_ref())
            .await
            {
                Ok(result) if result.rows_affected() > 0 => {
                    deleted += 1;
                }
                _ => {
                    skipped += 1;
                    error_details.push(json!({
                        "msg_id": msg_id,
                        "reason": "soft_delete_failed"
                    }));
                }
            }
        }

        // AUDIT LOGGING (fix #5): record batch action
        if !dry_run && (deleted > 0 || error_details.len() > 0) {
            let actor = context
                .metadata
                .get("user_id")
                .and_then(|v| v.as_i64())
                .map(|id| id.to_string())
                .unwrap_or_else(|| "agent".to_string());

            let _ = sqlx::query(
                r#"
                INSERT INTO platform.audit_logs
                (tenant_id, actor, action, resource_type, resource_id, payload_json, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, now())
                "#,
            )
            .bind(&tenant_id)
            .bind(&actor)
            .bind("delete_recent_ad_messages")
            .bind("group_messages")
            .bind(snapshot_id)
            .bind(format!(
                r#"{{"deleted":{}, "skipped":{}, "errors":{}}}"#,
                deleted,
                skipped,
                error_details.len()
            ))
            .execute(self.pool.as_ref())
            .await
            .ok();
        }

        // Mark snapshot consumed only on real execution
        if !dry_run {
            let _ = sqlx::query(
                "UPDATE groups.ad_delete_snapshots SET consumed_at = now() WHERE snapshot_id = $1",
            )
            .bind(snapshot_id)
            .execute(self.pool.as_ref())
            .await;
        }

        ToolExecuteResult {
            success: error_details.is_empty(),
            data: Some(json!({
                "deleted": deleted,
                "skipped": skipped,
                "errors": error_details.len(),
                "error_details": error_details,
                "action_taken": deleted > 0 && !dry_run,
                "dry_run": dry_run,
            })),
            message: Some(if dry_run {
                format!(
                    "预计删除 {} 条广告消息（{} 条超 Telegram 48 小时删除时限将跳过）",
                    deleted, skipped
                )
            } else if deleted > 0 {
                format!(
                    "已删除 {} 条广告消息；{} 条因超过 Telegram 48 小时删除时限跳过（本地记录保留）。",
                    deleted, skipped
                )
            } else {
                format!(
                    "未能删除：{} 条广告消息均超过 Telegram 48 小时删除时限（bot 只能删除 48 小时内的消息），已跳过。",
                    skipped
                )
            }),
            action_taken: deleted > 0 && !dry_run,
            needs_confirmation: false,
            requires_llm_generation: false,
            generated_content: None,
        }
    }
}
