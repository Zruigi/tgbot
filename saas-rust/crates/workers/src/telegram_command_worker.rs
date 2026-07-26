use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tgbot_integrations::telegram::SendMessageParams;
use tgbot_repos::message_repo;
use tgbot_services::telegram_commands::{build_command_reply, parse_telegram_command};
use tracing::{info, instrument};

use crate::job_queue::ClaimedJob;
use crate::push_task_worker::TelegramClientResolver;
use crate::scheduler::Worker;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramCommandJobKey {
    tenant_id: String,
    inbound_message_id: i64,
}

pub struct TelegramCommandWorker {
    telegram_resolver: Arc<dyn TelegramClientResolver>,
}

impl TelegramCommandWorker {
    pub fn new(telegram_resolver: Arc<dyn TelegramClientResolver>) -> Self {
        Self { telegram_resolver }
    }
}

#[async_trait]
impl Worker for TelegramCommandWorker {
    fn job_type(&self) -> &str {
        "telegram_command_reply"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let key = parse_job_key(&claimed.job.payload)?;
        if claimed.job.tenant_id != key.tenant_id {
            bail!("telegram command job tenant mismatch");
        }

        let row = sqlx::query(
            r#"
            SELECT inbound.from_userid AS user_id,
                   inbound.to_userid AS bot_id,
                   inbound.content,
                   inbound.tg_msg_id,
                   merchant.bot_username,
                   sender.first_name
            FROM chat.messages inbound
            JOIN merchants merchant
              ON merchant.tenant_id = inbound.tenant_id
             AND merchant.bot_id = inbound.to_userid
             AND merchant.status IN ('active', 'active_free')
            LEFT JOIN chat.users sender
              ON sender.user_id = inbound.from_userid
            WHERE inbound.id = $2
              AND inbound.tenant_id = $1
              AND inbound.is_read = false
              AND inbound.is_deleted = false
              AND inbound.from_userid > 0
              AND inbound.to_userid > 0
              AND inbound.from_userid <> inbound.to_userid
              AND inbound.msg_type = 'text'
              AND inbound.tg_msg_id IS NOT NULL
              AND NULLIF(btrim(inbound.content), '') IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1
                  FROM chat.messages outbound
                  WHERE outbound.tenant_id = inbound.tenant_id
                    AND outbound.from_userid = inbound.to_userid
                    AND outbound.to_userid = inbound.from_userid
                    AND outbound.quoted_msg_id = inbound.tg_msg_id
                    AND outbound.is_read = true
                    AND outbound.is_deleted = false
              )
            "#,
        )
        .bind(&key.tenant_id)
        .bind(key.inbound_message_id)
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            info!(
                tenant_id = key.tenant_id,
                inbound_message_id = key.inbound_message_id,
                "telegram command reply job is stale or already delivered"
            );
            return Ok(());
        };

        let content: String = row.try_get("content")?;
        let bot_username: Option<String> = row.try_get("bot_username")?;
        let Some(command) = parse_telegram_command(&content, bot_username.as_deref().unwrap_or(""))
        else {
            return Ok(());
        };
        let first_name: Option<String> = row.try_get("first_name")?;
        let reply = build_command_reply(&command, first_name.as_deref().unwrap_or("朋友"));
        let user_id: i64 = row.try_get("user_id")?;
        let bot_id: i64 = row.try_get("bot_id")?;
        let telegram_message_id: i64 = row.try_get("tg_msg_id")?;
        let telegram = self
            .telegram_resolver
            .resolve(pool, &key.tenant_id)
            .await
            .context("failed to resolve Telegram command client")?;
        let sent = telegram
            .send_message(SendMessageParams {
                chat_id: user_id,
                text: reply.text.clone(),
                parse_mode: None,
                reply_markup: None,
                reply_to_message_id: Some(telegram_message_id),
            })
            .await
            .context("failed to deliver Telegram command reply")?;

        message_repo::save_message(
            pool,
            bot_id,
            user_id,
            Some(&reply.text),
            "text",
            None,
            None,
            true,
            Some(sent.message_id),
            &key.tenant_id,
            Some(telegram_message_id),
        )
        .await
        .context("failed to persist Telegram command reply")?;

        info!(
            tenant_id = key.tenant_id,
            inbound_message_id = key.inbound_message_id,
            "telegram command reply delivered"
        );
        Ok(())
    }
}

fn parse_job_key(payload: &Value) -> Result<TelegramCommandJobKey> {
    let tenant_id = payload
        .get("tenant_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing tenant_id in telegram command payload"))?;
    let inbound_message_id = payload
        .get("inbound_message_id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("missing inbound_message_id in telegram command payload"))?;

    Ok(TelegramCommandJobKey {
        tenant_id: tenant_id.to_string(),
        inbound_message_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_tenant_scoped_job_key() {
        assert_eq!(
            parse_job_key(&json!({
                "tenant_id": "tenant-a",
                "inbound_message_id": 42
            }))
            .unwrap(),
            TelegramCommandJobKey {
                tenant_id: "tenant-a".into(),
                inbound_message_id: 42,
            }
        );
        assert!(parse_job_key(&json!({
            "tenant_id": "tenant-a",
            "inbound_message_id": 0
        }))
        .is_err());
        assert!(parse_job_key(&json!({"inbound_message_id": 42})).is_err());
    }
}
