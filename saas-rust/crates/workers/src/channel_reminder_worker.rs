use anyhow::Result;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{info, warn};

use crate::push_task_worker::TelegramClientResolver;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChannelReminderCleanupSummary {
    pub claimed: usize,
    pub deleted: usize,
    pub failed: usize,
}

pub async fn cleanup_expired_channel_reminders(
    pool: &PgPool,
    telegram_resolver: Arc<dyn TelegramClientResolver>,
    batch_size: i64,
) -> Result<ChannelReminderCleanupSummary> {
    let rows = sqlx::query(
        r#"
        WITH candidates AS (
            SELECT id
            FROM groups.channel_reminders
            WHERE is_deleted = false
              AND expires_at <= now()
              AND (delete_retry_at IS NULL OR delete_retry_at <= now())
              AND (delete_claimed_at IS NULL OR delete_claimed_at < now() - interval '5 minutes')
            ORDER BY expires_at, id
            FOR UPDATE SKIP LOCKED
            LIMIT $1
        )
        UPDATE groups.channel_reminders AS reminder
        SET delete_claimed_at = now(),
            delete_attempts = reminder.delete_attempts + 1
        FROM candidates
        WHERE reminder.id = candidates.id
        RETURNING reminder.id, reminder.group_id, reminder.message_id, reminder.tenant_id
        "#,
    )
    .bind(batch_size.clamp(1, 500))
    .fetch_all(pool)
    .await?;

    let mut summary = ChannelReminderCleanupSummary {
        claimed: rows.len(),
        ..ChannelReminderCleanupSummary::default()
    };
    for row in rows {
        let reminder_id: i64 = row.try_get("id")?;
        let group_id: i64 = row.try_get("group_id")?;
        let message_id: i64 = row.try_get("message_id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let result = match telegram_resolver.resolve(pool, &tenant_id).await {
            Ok(telegram) => telegram.delete_message(group_id, message_id).await,
            Err(error) => Err(error),
        };

        match result {
            Ok(true) => {
                sqlx::query(
                    "UPDATE groups.channel_reminders \
                     SET is_deleted = true, delete_claimed_at = NULL, delete_retry_at = NULL, \
                         last_delete_error = NULL \
                     WHERE id = $1 AND tenant_id = $2",
                )
                .bind(reminder_id)
                .bind(&tenant_id)
                .execute(pool)
                .await?;
                summary.deleted += 1;
            }
            Ok(false) => {
                release_failed_claim(pool, reminder_id, &tenant_id, "Telegram returned false")
                    .await?;
                summary.failed += 1;
            }
            Err(error) => {
                let error_message = error.to_string().chars().take(500).collect::<String>();
                release_failed_claim(pool, reminder_id, &tenant_id, &error_message).await?;
                summary.failed += 1;
                warn!(
                    reminder_id,
                    group_id, tenant_id, "failed to delete expired channel reminder"
                );
            }
        }
    }

    if summary.claimed > 0 {
        info!(
            claimed = summary.claimed,
            deleted = summary.deleted,
            failed = summary.failed,
            "expired channel reminder cleanup completed"
        );
    }
    Ok(summary)
}

async fn release_failed_claim(
    pool: &PgPool,
    reminder_id: i64,
    tenant_id: &str,
    error_message: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE groups.channel_reminders \
         SET delete_claimed_at = NULL, last_delete_error = $3, \
             delete_retry_at = now() + make_interval(secs => LEAST(3600, \
                 (power(2, LEAST(delete_attempts, 10))::integer * 30))) \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(reminder_id)
    .bind(tenant_id)
    .bind(error_message)
    .execute(pool)
    .await?;
    Ok(())
}
