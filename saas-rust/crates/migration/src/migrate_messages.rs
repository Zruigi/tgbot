//! Message Migration - Migrate message data from SQLite to PostgreSQL
//! This module handles the migration of private chat messages from the legacy
//! SQLite database to the chat.messages table in PostgreSQL/Supabase.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;

use crate::migration_audit::MigrationFailure;
use crate::sqlite_reader::{SqliteConfig, SqliteMessage, SqliteReader};

/// Migration statistics for messages
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageMigrationStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
    pub batch_size: usize,
    pub failures: Vec<MigrationFailure>,
}

/// Migrate messages from SQLite to PostgreSQL with batching
pub async fn migrate_messages(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
    batch_size: usize,
) -> Result<MessageMigrationStats> {
    tracing::info!(
        "Starting message migration from SQLite to PostgreSQL (batch_size={})",
        batch_size
    );

    let sqlite_reader = SqliteReader::new(sqlite_config)?;

    // Get total count
    let total_source = sqlite_reader.count_messages()?;
    tracing::info!("Total messages in SQLite: {}", total_source);

    let mut stats = MessageMigrationStats {
        total_source,
        migrated: 0,
        skipped: 0,
        errors: 0,
        batch_size,
        failures: Vec::new(),
    };

    let mut offset = 0;

    while offset < total_source as usize {
        // Read batch from SQLite
        let messages = sqlite_reader.read_messages(batch_size, offset)?;

        if messages.is_empty() {
            break;
        }

        // Migrate batch
        let batch_result = migrate_message_batch(&messages, pg_pool).await;

        match batch_result {
            Ok((batch_migrated, batch_failures)) => {
                stats.migrated += batch_migrated;
                stats.errors += batch_failures.len() as i64;
                stats.failures.extend(batch_failures);
                tracing::debug!(
                    "Migrated batch: {} messages (offset={})",
                    batch_migrated,
                    offset
                );
            }
            Err(e) => {
                tracing::error!("Failed to migrate batch at offset {}: {}", offset, e);
                stats.errors += messages.len() as i64;
                let error = e.to_string();
                stats.failures.extend(messages.iter().map(|message| {
                    MigrationFailure::new("messages", message_source_key(message), error.clone())
                }));
            }
        }

        offset += batch_size;
    }

    tracing::info!(
        "Message migration complete: {} migrated, {} skipped, {} errors",
        stats.migrated,
        stats.skipped,
        stats.errors
    );

    Ok(stats)
}

/// Migrate a batch of messages
async fn migrate_message_batch(
    messages: &[SqliteMessage],
    pg_pool: &PgPool,
) -> Result<(i64, Vec<MigrationFailure>)> {
    let mut migrated = 0;
    let mut failures = Vec::new();

    for msg in messages {
        match migrate_single_message(msg, pg_pool).await {
            Ok(_) => migrated += 1,
            Err(e) => {
                tracing::warn!("Failed to migrate message {}: {}", msg.msg_id, e);
                failures.push(MigrationFailure::new(
                    "messages",
                    message_source_key(msg),
                    e.to_string(),
                ));
            }
        }
    }

    Ok((migrated, failures))
}

fn message_source_key(message: &SqliteMessage) -> String {
    format!("tenant_id={}::msg_id={}", message.tenant_id, message.msg_id)
}

/// Migrate a single message from SQLite to PostgreSQL
async fn migrate_single_message(sqlite_msg: &SqliteMessage, pg_pool: &PgPool) -> Result<()> {
    // Generate trace_id if not present
    let trace_id = normalized_trace_id(
        sqlite_msg.trace_id.as_deref(),
        &sqlite_msg.tenant_id,
        sqlite_msg.msg_id,
    );

    // Parse timestamp
    let timestamp = parse_timestamp(&sqlite_msg.timestamp)?;
    let quoted_type = sqlite_msg
        .quoted_type
        .clone()
        .unwrap_or_else(|| "text".to_string());
    let status = sqlite_msg
        .status
        .clone()
        .unwrap_or_else(|| "sent".to_string());
    let msg_type = sqlite_msg.msg_type.clone();

    // Insert message into PostgreSQL
    sqlx::query(
        r#"
        INSERT INTO chat.messages (
            trace_id,
            from_userid,
            to_userid,
            content,
            msg_type,
            file_id,
            media_url,
            timestamp,
            is_read,
            quoted_text,
            quoted_msg_id,
            quoted_file_id,
            quoted_type,
            tg_msg_id,
            status,
            auto_translation,
            translated_text,
            original_text,
            admin_userid,
            is_deleted,
            tenant_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)
        ON CONFLICT (trace_id) DO UPDATE SET
            from_userid = EXCLUDED.from_userid,
            to_userid = EXCLUDED.to_userid,
            content = COALESCE(EXCLUDED.content, chat.messages.content),
            is_read = EXCLUDED.is_read,
            tg_msg_id = COALESCE(EXCLUDED.tg_msg_id, chat.messages.tg_msg_id),
            status = EXCLUDED.status
        "#
    )
    .bind(&trace_id)
    .bind(sqlite_msg.from_userid)
    .bind(sqlite_msg.to_userid)
    .bind(&sqlite_msg.content)
    .bind(&msg_type)
    .bind(&sqlite_msg.file_id)
    .bind(&sqlite_msg.media_url)
    .bind(timestamp)
    .bind(sqlite_msg.is_read)
    .bind(&sqlite_msg.quoted_text)
    .bind(sqlite_msg.quoted_msg_id)
    .bind(&sqlite_msg.quoted_file_id)
    .bind(&quoted_type)
    .bind(sqlite_msg.tg_msg_id)
    .bind(&status)
    .bind(&sqlite_msg.auto_translation)
    .bind(&sqlite_msg.translated_text)
    .bind(&sqlite_msg.original_text)
    .bind(&sqlite_msg.admin_userid)
    .bind(sqlite_msg.is_deleted)
    .bind(&sqlite_msg.tenant_id)
    .execute(pg_pool)
    .await
    .context("Failed to insert message")?;

    // Migrate emoji reactions if any (from emoji_messages table)
    // Note: This would require reading from the emoji_messages table in SQLite
    // For now, we'll skip emoji migration

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

fn normalized_trace_id(trace_id: Option<&str>, tenant_id: &str, message_id: i64) -> String {
    trace_id
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("legacy-message:{tenant_id}:{message_id}"))
}

/// Verify message migration by comparing counts
pub async fn verify_message_migration(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<MessageVerificationResult> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_count = sqlite_reader.count_messages()?;

    let target_count: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM chat.messages")
            .fetch_one(pg_pool)
            .await
            .context("Failed to count messages in PostgreSQL")?;

    let result = MessageVerificationResult {
        source_count,
        target_count,
        is_valid: source_count == target_count,
    };

    tracing::info!(
        "Message verification: source={}, target={}, valid={}",
        result.source_count,
        result.target_count,
        result.is_valid
    );

    Ok(result)
}

/// Message migration verification result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageVerificationResult {
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
    }

    #[test]
    fn test_normalized_trace_id_preserves_existing_value() {
        assert_eq!(
            normalized_trace_id(Some("trace-1"), "tenant-a", 42),
            "trace-1"
        );
    }

    #[test]
    fn test_normalized_trace_id_is_stable_for_missing_value() {
        assert_eq!(
            normalized_trace_id(None, "tenant-a", 42),
            "legacy-message:tenant-a:42"
        );
        assert_eq!(
            normalized_trace_id(Some("  "), "tenant-a", 42),
            "legacy-message:tenant-a:42"
        );
    }
}
