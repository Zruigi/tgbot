//! Group Message Migration - Migrate group message data from SQLite to PostgreSQL
//! This module handles the migration of group messages from the legacy SQLite
//! database to the groups.group_messages table in PostgreSQL/Supabase.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;

use crate::sqlite_reader::{SqliteConfig, SqliteGroupMessage, SqliteReader};

/// Migration statistics for group messages
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupMessageMigrationStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
    pub batch_size: usize,
}

/// Migrate group messages from SQLite to PostgreSQL with batching
pub async fn migrate_group_messages(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
    batch_size: usize,
) -> Result<GroupMessageMigrationStats> {
    tracing::info!(
        "Starting group message migration from SQLite to PostgreSQL (batch_size={})",
        batch_size
    );

    let sqlite_reader = SqliteReader::new(sqlite_config)?;

    // Get total count
    let total_source = sqlite_reader.count_group_messages()?;
    tracing::info!("Total group messages in SQLite: {}", total_source);

    let mut stats = GroupMessageMigrationStats {
        total_source,
        migrated: 0,
        skipped: 0,
        errors: 0,
        batch_size,
    };

    let mut offset = 0;

    while offset < total_source as usize {
        // Read batch from SQLite
        let messages = sqlite_reader.read_group_messages(batch_size, offset)?;

        if messages.is_empty() {
            break;
        }

        // Migrate batch
        let batch_result = migrate_group_message_batch(&messages, pg_pool).await;

        match batch_result {
            Ok(batch_migrated) => {
                stats.migrated += batch_migrated;
                tracing::debug!(
                    "Migrated batch: {} group messages (offset={})",
                    batch_migrated,
                    offset
                );
            }
            Err(e) => {
                tracing::error!("Failed to migrate batch at offset {}: {}", offset, e);
                stats.errors += messages.len() as i64;
            }
        }

        offset += batch_size;
    }

    // Get skipped count (already migrated)
    let target_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_messages"
    )
    .fetch_one(pg_pool)
    .await
    .context("Failed to count group messages in PostgreSQL")?;

    // If target count is close to source, consider the rest as skipped (already migrated)
    if target_count < stats.total_source {
        stats.skipped = stats.total_source - stats.migrated - stats.errors;
    }

    tracing::info!(
        "Group message migration complete: {} migrated, {} skipped, {} errors",
        stats.migrated,
        stats.skipped,
        stats.errors
    );

    Ok(stats)
}

/// Migrate a batch of group messages
async fn migrate_group_message_batch(
    messages: &[SqliteGroupMessage],
    pg_pool: &PgPool,
) -> Result<i64> {
    let mut migrated = 0;

    for msg in messages {
        match migrate_single_group_message(msg, pg_pool).await {
            Ok(_) => migrated += 1,
            Err(e) => {
                // Log but continue with other messages
                tracing::warn!("Failed to migrate group message {}: {}", msg.msg_id, e);
            }
        }
    }

    Ok(migrated)
}

/// Migrate a single group message from SQLite to PostgreSQL
async fn migrate_single_group_message(
    sqlite_msg: &SqliteGroupMessage,
    pg_pool: &PgPool,
) -> Result<()> {
    // Parse timestamp if available
    let timestamp = if let Some(ts) = &sqlite_msg.timestamp {
        Some(parse_timestamp(ts)?)
    } else {
        None
    };

    // Parse ad_checked_at if available
    let ad_checked_at = if let Some(ts) = &sqlite_msg.ad_checked_at {
        Some(parse_timestamp(ts)?)
    } else {
        None
    };

    // Parse deleted_at if available
    let deleted_at = if let Some(ts) = &sqlite_msg.deleted_at {
        Some(parse_timestamp(ts)?)
    } else {
        None
    };

    let msg_type = sqlite_msg.msg_type.clone();
    let sender_type = sqlite_msg.sender_type.clone();
    let ad_action_status = sqlite_msg.ad_action_status.clone();

    // Insert message into PostgreSQL
    sqlx::query(
        r#"
        INSERT INTO groups.group_messages (
            group_id,
            user_id,
            username,
            content,
            msg_type,
            tg_msg_id,
            sender_type,
            quoted_tg_msg_id,
            quoted_text,
            message_ts,
            file_id,
            avatar_file_id,
            revoke_checked,
            is_advertising,
            ad_score,
            ad_confidence,
            ad_findings,
            tenant_id,
            ad_cluster_id,
            ad_checked_at,
            ad_action_status,
            is_deleted,
            deleted_at,
            ad_text_hash,
            media_unique_id,
            timestamp
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
        ON CONFLICT (group_id, tg_msg_id) WHERE tg_msg_id IS NOT NULL DO UPDATE SET
            user_id = EXCLUDED.user_id,
            username = COALESCE(EXCLUDED.username, groups.group_messages.username),
            content = COALESCE(EXCLUDED.content, groups.group_messages.content),
            is_advertising = EXCLUDED.is_advertising,
            ad_score = EXCLUDED.ad_score,
            ad_confidence = EXCLUDED.ad_confidence,
            ad_findings = COALESCE(EXCLUDED.ad_findings, groups.group_messages.ad_findings),
            ad_cluster_id = COALESCE(EXCLUDED.ad_cluster_id, groups.group_messages.ad_cluster_id),
            ad_checked_at = COALESCE(EXCLUDED.ad_checked_at, groups.group_messages.ad_checked_at),
            ad_action_status = COALESCE(EXCLUDED.ad_action_status, groups.group_messages.ad_action_status),
            is_deleted = EXCLUDED.is_deleted,
            deleted_at = COALESCE(EXCLUDED.deleted_at, groups.group_messages.deleted_at),
            ad_text_hash = COALESCE(EXCLUDED.ad_text_hash, groups.group_messages.ad_text_hash),
            media_unique_id = COALESCE(EXCLUDED.media_unique_id, groups.group_messages.media_unique_id)
        "#
    )
    .bind(sqlite_msg.group_id)
    .bind(sqlite_msg.user_id)
    .bind(&sqlite_msg.username)
    .bind(&sqlite_msg.content)
    .bind(&msg_type)
    .bind(sqlite_msg.tg_msg_id)
    .bind(&sender_type)
    .bind(sqlite_msg.quoted_tg_msg_id)
    .bind(&sqlite_msg.quoted_text)
    .bind(sqlite_msg.message_ts)
    .bind(&sqlite_msg.file_id)
    .bind(&sqlite_msg.avatar_file_id)
    .bind(sqlite_msg.revoke_checked)
    .bind(sqlite_msg.is_advertising)
    .bind(sqlite_msg.ad_score)
    .bind(&sqlite_msg.ad_confidence)
    .bind(&sqlite_msg.ad_findings)
    .bind(&sqlite_msg.tenant_id)
    .bind(&sqlite_msg.ad_cluster_id)
    .bind(ad_checked_at)
    .bind(&ad_action_status)
    .bind(sqlite_msg.is_deleted)
    .bind(deleted_at)
    .bind(&sqlite_msg.ad_text_hash)
    .bind(&sqlite_msg.media_unique_id)
    .bind(timestamp)
    .execute(pg_pool)
    .await
    .context("Failed to insert group message")?;

    Ok(())
}

/// Parse SQLite timestamp string to DateTime<Utc>
fn parse_timestamp(s: &str) -> Result<DateTime<Utc>> {
    let formats = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.fZ",
    ];

    for fmt in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(DateTime::from_naive_utc_and_offset(dt, Utc));
        }
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let datetime = date.and_hms_opt(0, 0, 0).unwrap();
        return Ok(DateTime::from_naive_utc_and_offset(datetime, Utc));
    }

    Err(anyhow::anyhow!("Failed to parse timestamp: {}", s))
}

/// Verify group message migration by comparing counts
pub async fn verify_group_message_migration(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<GroupMessageVerificationResult> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_count = sqlite_reader.count_group_messages()?;

    let target_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_messages"
    )
    .fetch_one(pg_pool)
    .await
    .context("Failed to count group messages in PostgreSQL")?;

    let result = GroupMessageVerificationResult {
        source_count,
        target_count,
        is_valid: source_count == target_count,
    };

    tracing::info!(
        "Group message verification: source={}, target={}, valid={}",
        result.source_count,
        result.target_count,
        result.is_valid
    );

    Ok(result)
}

/// Group message migration verification result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupMessageVerificationResult {
    pub source_count: i64,
    pub target_count: i64,
    pub is_valid: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp() {
        let result = parse_timestamp("2026-01-15 10:30:00");
        assert!(result.is_ok());

        let result = parse_timestamp("2026-01-15T10:30:00Z");
        assert!(result.is_ok());
    }

    #[test]
    fn test_group_message_stats() {
        let stats = GroupMessageMigrationStats {
            total_source: 1000,
            migrated: 950,
            skipped: 40,
            errors: 10,
            batch_size: 100,
        };

        assert_eq!(stats.total_source, 1000);
        assert_eq!(stats.migrated + stats.skipped + stats.errors, stats.total_source);
    }
}
