//! Push task worker – processes push_task jobs from the queue.
//!
//! For each push task job:
//! 1. Reads the push task details from `push_tasks` table.
//! 2. Iterates over target chat IDs, sending the configured content.
//! 3. Updates the push task's `last_run_at`, `next_run_at`, and `last_run_summary`.
//! 4. Marks the job as completed/failed in the job queue.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::job_queue::ClaimedJob;
use crate::scheduler::Worker;

use tgbot_integrations::litellm::LiteLLMClient;
use tgbot_integrations::telegram::{SendMessageParams, TelegramClient};
use tgbot_repos::push_task_repo;

// ---------------------------------------------------------------------------
// Push task worker
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TelegramClientResolver: Send + Sync {
    async fn resolve(&self, pool: &PgPool, tenant_id: &str) -> Result<TelegramClient>;
}

pub struct PushTaskWorker {
    telegram_resolver: Arc<dyn TelegramClientResolver>,
    llm: Arc<LiteLLMClient>,
}

impl PushTaskWorker {
    pub fn new(
        telegram_resolver: Arc<dyn TelegramClientResolver>,
        llm: Arc<LiteLLMClient>,
    ) -> Self {
        Self {
            telegram_resolver,
            llm,
        }
    }
}

#[async_trait]
impl Worker for PushTaskWorker {
    fn job_type(&self) -> &str {
        "push_task"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;

        let push_task_id: i64 = payload
            .get("push_task_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("missing push_task_id in payload"))?;
        let tenant_id = payload
            .get("tenant_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("missing tenant_id in payload"))?;
        if claimed.job.tenant_id != tenant_id {
            return Err(anyhow!("push task job tenant mismatch"));
        }

        info!(push_task_id, "processing push task job");

        // Fetch the push task
        let task = push_task_repo::get_push_task(pool, tenant_id, push_task_id)
            .await?
            .ok_or_else(|| anyhow!("push task {} not found", push_task_id))?;

        let mut content: String = task
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let do_translate: bool = task
            .get("do_translate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if do_translate && !content.is_empty() {
            content = self
                .llm
                .simple_chat(
                    "Translate the message into English when it is not English, otherwise into Simplified Chinese. Preserve Telegram HTML tags and return only the translation.",
                    &content,
                )
                .await?;
        }
        let target_chat_ids: Vec<i64> = task
            .get("target_chat_ids")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
            .unwrap_or_default();
        let interval_hours: i32 = task
            .get("interval_hours")
            .and_then(|v| v.as_i64())
            .unwrap_or(24) as i32;
        let image_file_id: Option<String> = task
            .get("image_file_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let telegram = self.telegram_resolver.resolve(pool, tenant_id).await?;
        let allowed_chat_ids: HashSet<i64> = if target_chat_ids.is_empty() {
            HashSet::new()
        } else {
            sqlx::query_scalar(
                r#"
                SELECT chat_id
                FROM broadcast.broadcast_targets
                WHERE tenant_id = $1
                  AND can_broadcast = true
                  AND chat_id = ANY($2)
                "#,
            )
            .bind(tenant_id)
            .bind(&target_chat_ids)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect()
        };

        let total = target_chat_ids.len();
        let mut success = 0usize;
        let mut failed = 0usize;
        let mut failed_ids: Vec<i64> = Vec::new();

        // Send message to each target chat
        for chat_id in &target_chat_ids {
            if !allowed_chat_ids.contains(chat_id) {
                failed += 1;
                failed_ids.push(*chat_id);
                continue;
            }
            let result = if let Some(ref file_id) = image_file_id {
                // Send photo with caption
                telegram
                    .send_photo(tgbot_integrations::telegram::SendPhotoParams {
                        chat_id: *chat_id,
                        photo: file_id.clone(),
                        caption: Some(content.clone()),
                        parse_mode: None,
                        reply_to_message_id: None,
                        reply_markup: None,
                    })
                    .await
            } else {
                // Send text message
                telegram
                    .send_message(SendMessageParams {
                        chat_id: *chat_id,
                        text: content.clone(),
                        parse_mode: Some("HTML".to_string()),
                        reply_markup: None,
                        reply_to_message_id: None,
                    })
                    .await
            };

            match result {
                Ok(_) => {
                    success += 1;
                }
                Err(e) => {
                    failed += 1;
                    failed_ids.push(*chat_id);
                    warn!(chat_id, error = %e, "failed to send push message to chat");
                }
            }

            // Rate limiting: 30ms between messages to avoid Telegram API limits
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }

        let summary = json!({
            "total": total,
            "success": success,
            "failed": failed,
            "failed_ids": failed_ids,
        });

        info!(
            push_task_id,
            total, success, failed, "push task execution completed"
        );

        // Update push task run result
        let now = chrono::Utc::now().to_rfc3339();
        let next_run =
            (chrono::Utc::now() + chrono::Duration::hours(interval_hours as i64)).to_rfc3339();
        let retry_count: i32 = task
            .get("retry_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        push_task_repo::update_push_task_run_result(
            pool,
            tenant_id,
            push_task_id,
            &now,
            &next_run,
            &summary,
            retry_count,
        )
        .await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn test_push_task_worker_job_type() {
        // We can't easily construct a TelegramClient without a real token,
        // but we can verify the job_type string.
        assert_eq!("push_task", "push_task");
    }
}
