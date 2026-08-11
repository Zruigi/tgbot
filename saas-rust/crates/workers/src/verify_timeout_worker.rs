//! Verify timeout worker – 入群验证超时踢出（对齐 Python join_verify.py 120s 超时）。
//!
//! start_group_verify 发题时入队 `verify_timeout_kick`（run_at = now()+120s）。
//! 到点后检查 group_members.pending_verify_msg_id：
//!   - 仍 pending → 踢出（ban + unban）+ 删除验证消息 + 清 pending
//!   - 已通过（pending 已清）→ 跳过

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::job_queue::ClaimedJob;
use crate::push_task_worker::TelegramClientResolver;
use crate::scheduler::Worker;

use tgbot_integrations::telegram::TelegramClient;

pub struct VerifyTimeoutWorker {
    telegram_resolver: Arc<dyn TelegramClientResolver>,
}

impl VerifyTimeoutWorker {
    pub fn new(telegram_resolver: Arc<dyn TelegramClientResolver>) -> Self {
        Self { telegram_resolver }
    }
}

#[async_trait]
impl Worker for VerifyTimeoutWorker {
    fn job_type(&self) -> &str {
        "verify_timeout_kick"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;
        let group_id = payload
            .get("group_id")
            .and_then(Value::as_i64)
            .context("verify_timeout payload missing group_id")?;
        let user_id = payload
            .get("user_id")
            .and_then(Value::as_i64)
            .context("verify_timeout payload missing user_id")?;
        let msg_id = payload.get("msg_id").and_then(Value::as_i64);
        let tenant_id = &claimed.job.tenant_id;

        // 仍 pending？已通过则 pending_verify_msg_id 为 NULL（回调清空），跳过
        let pending: Option<i64> = sqlx::query_scalar(
            "SELECT pending_verify_msg_id FROM groups.group_members \
             WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(group_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .context("failed to load pending verify state")?;
        let Some(_pending) = pending else {
            info!(group_id, user_id, "Verify already passed, skip timeout kick");
            return Ok(());
        };

        let telegram = self
            .telegram_resolver
            .resolve(pool, tenant_id)
            .await
            .context("failed to resolve telegram client for verify timeout")?;

        // 踢出（ban + unban，可再次入群，对齐 Python _kick_user）
        if let Err(error) = telegram.ban_chat_member(group_id, user_id).await {
            warn!(%error, group_id, user_id, "timeout kick ban failed");
        }
        if let Err(error) = telegram.unban_chat_member(group_id, user_id).await {
            warn!(%error, group_id, user_id, "timeout kick unban failed");
        }
        if let Some(msg_id) = msg_id {
            let _ = telegram.delete_message(group_id, msg_id).await;
        }

        // 清 pending + 状态
        sqlx::query(
            "UPDATE groups.group_members \
             SET pending_verify_msg_id = NULL, status = 'left', updated_at = now() \
             WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(group_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(pool)
        .await
        .context("failed to clear pending verify state")?;

        info!(group_id, user_id, "Verify timeout: kicked user");
        Ok(())
    }
}
