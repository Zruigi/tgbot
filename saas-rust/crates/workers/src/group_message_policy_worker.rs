use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::{PgPool, Row};
use tgbot_services::group_policy::{
    apply_group_message_policy, parse_custom_keywords, GroupMessagePolicyInput,
};
use tracing::{info, instrument};

use crate::job_queue::ClaimedJob;
use crate::scheduler::Worker;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GroupMessagePolicyJobKey {
    tenant_id: String,
    group_id: i64,
    telegram_message_id: i64,
}

pub struct GroupMessagePolicyWorker;

impl GroupMessagePolicyWorker {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Worker for GroupMessagePolicyWorker {
    fn job_type(&self) -> &str {
        "group_message_policy"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let key = parse_job_key(&claimed.job.payload)?;
        if claimed.job.tenant_id != key.tenant_id {
            return Err(anyhow!("group message policy job tenant mismatch"));
        }
        let row = sqlx::query(
            r#"
            SELECT message.user_id, message.username, message.content,
                   tracked.group_title, tracked.custom_keywords,
                   member.first_name, member.last_name
            FROM groups.group_messages message
            JOIN groups.tracked_groups tracked
              ON tracked.tenant_id = message.tenant_id
             AND tracked.group_id = message.group_id
            LEFT JOIN groups.group_members member
              ON member.tenant_id = message.tenant_id
             AND member.group_id = message.group_id
             AND member.user_id = message.user_id
            WHERE message.tenant_id = $1
              AND message.group_id = $2
              AND message.tg_msg_id = $3
              AND message.sender_type = 'user'
              AND tracked.is_monitored = true
              AND NULLIF(btrim(message.content), '') IS NOT NULL
            "#,
        )
        .bind(&key.tenant_id)
        .bind(key.group_id)
        .bind(key.telegram_message_id)
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            info!(
                tenant_id = key.tenant_id,
                group_id = key.group_id,
                telegram_message_id = key.telegram_message_id,
                "group message policy job is stale or no longer eligible"
            );
            return Ok(());
        };

        let group_title: String = row.try_get("group_title")?;
        let username: Option<String> = row.try_get("username")?;
        let first_name: Option<String> = row.try_get("first_name")?;
        let last_name: Option<String> = row.try_get("last_name")?;
        let content: String = row.try_get("content")?;
        let custom_keywords = parse_custom_keywords(
            row.try_get::<Option<String>, _>("custom_keywords")?
                .as_deref(),
        );
        let summary = apply_group_message_policy(
            pool,
            GroupMessagePolicyInput {
                tenant_id: &key.tenant_id,
                group_id: key.group_id,
                group_title: &group_title,
                telegram_message_id: key.telegram_message_id,
                user_id: row.try_get("user_id")?,
                username: username.as_deref(),
                first_name: first_name.as_deref(),
                last_name: last_name.as_deref(),
                content: &content,
                custom_keywords: &custom_keywords,
            },
        )
        .await?;

        info!(
            tenant_id = key.tenant_id,
            group_id = key.group_id,
            telegram_message_id = key.telegram_message_id,
            advertising = summary.is_advertising,
            lead_captured = summary.lead_captured,
            "group message policy job completed"
        );
        Ok(())
    }
}

fn parse_job_key(payload: &Value) -> Result<GroupMessagePolicyJobKey> {
    let tenant_id = payload
        .get("tenant_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing tenant_id in group message policy payload"))?;
    let group_id = payload
        .get("group_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("missing group_id in group message policy payload"))?;
    let telegram_message_id = payload
        .get("telegram_message_id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("missing telegram_message_id in group message policy payload"))?;

    Ok(GroupMessagePolicyJobKey {
        tenant_id: tenant_id.to_string(),
        group_id,
        telegram_message_id,
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
                "group_id": -10001,
                "telegram_message_id": 42
            }))
            .unwrap(),
            GroupMessagePolicyJobKey {
                tenant_id: "tenant-a".into(),
                group_id: -10001,
                telegram_message_id: 42,
            }
        );
        assert!(parse_job_key(&json!({
            "group_id": -10001,
            "telegram_message_id": 42
        }))
        .is_err());
    }
}
