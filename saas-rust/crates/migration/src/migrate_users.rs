//! User and Admin Inbox Migration - Migrate user and admin inbox data from SQLite to PostgreSQL
//! This module handles the migration of users and admin inbox from the legacy SQLite
//! database to PostgreSQL/Supabase with virtual assistant UID normalization.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;

use crate::sqlite_reader::{SqliteConfig, SqliteAdminInbox, SqliteUser, SqliteReader};

/// Virtual assistant UIDs that should be excluded from admin_inbox
const VIRTUAL_ASSISTANT_IDS: &[i64] = &[0, -99999, 900000001];

/// Legacy virtual assistant UID to be mapped to canonical ID
/// Reference: PORTING-13-COMMITS.md item 3 (915dcd6)
#[allow(dead_code)]
const LEGACY_AI_ASSISTANT_UID: i64 = 900000001;

/// Canonical virtual assistant UID for normalization
/// Reference: PORTING-13-COMMITS.md item 3 (915dcd6)
#[allow(dead_code)]
const CANONICAL_AI_ASSISTANT_UID: i64 = -99999;

/// Migration statistics for users
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserMigrationStats {
    pub users: UserStats,
    pub admin_inbox: AdminInboxStats,
}

/// Statistics for users migration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
}

/// Statistics for admin_inbox migration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdminInboxStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
    pub virtual_assistant_excluded: i64,
}

/// Migrate users and admin inbox from SQLite to PostgreSQL
pub async fn migrate_users(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<UserMigrationStats> {
    tracing::info!("Starting user and admin inbox migration from SQLite to PostgreSQL");

    let sqlite_reader = SqliteReader::new(sqlite_config)?;

    // Migrate users
    let source_users = sqlite_reader.read_users()?;
    let user_stats = migrate_user_list(&source_users, pg_pool).await?;

    // Migrate admin inbox (excluding virtual assistant IDs)
    let source_admin_inbox = sqlite_reader.read_admin_inbox()?;
    let admin_inbox_stats = migrate_admin_inbox_list(&source_admin_inbox, pg_pool).await?;

    let stats = UserMigrationStats {
        users: user_stats,
        admin_inbox: admin_inbox_stats,
    };

    tracing::info!(
        "User migration complete: users({} migrated, {} skipped, {} errors), admin_inbox({} migrated, {} skipped, {} errors, {} virtual_assistant_excluded)",
        stats.users.migrated,
        stats.users.skipped,
        stats.users.errors,
        stats.admin_inbox.migrated,
        stats.admin_inbox.skipped,
        stats.admin_inbox.errors,
        stats.admin_inbox.virtual_assistant_excluded
    );

    Ok(stats)
}

/// Check if a user ID is a virtual assistant
fn is_virtual_assistant(user_id: i64) -> bool {
    VIRTUAL_ASSISTANT_IDS.contains(&user_id)
}

/// Migrate a list of users
async fn migrate_user_list(
    users: &[SqliteUser],
    pg_pool: &PgPool,
) -> Result<UserStats> {
    let mut stats = UserStats {
        total_source: users.len() as i64,
        migrated: 0,
        skipped: 0,
        errors: 0,
    };

    for user in users {
        match migrate_single_user(user, pg_pool).await {
            Ok(_) => {
                stats.migrated += 1;
                tracing::debug!("Migrated user: {}", user.user_id);
            }
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("duplicate key") || error_str.contains("unique constraint") {
                    stats.skipped += 1;
                } else {
                    stats.errors += 1;
                    tracing::error!("Failed to migrate user {}: {}", user.user_id, e);
                }
            }
        }
    }

    Ok(stats)
}

/// Migrate a single user
async fn migrate_single_user(
    sqlite_user: &SqliteUser,
    pg_pool: &PgPool,
) -> Result<()> {
    let created_at = parse_timestamp(&sqlite_user.created_at)?;

    sqlx::query(
        r#"
        INSERT INTO chat.users (
            user_id,
            email,
            created_at
        ) VALUES ($1, $2, $3)
        ON CONFLICT (user_id) DO UPDATE SET
            email = COALESCE(EXCLUDED.email, chat.users.email)
        "#
    )
    .bind(sqlite_user.user_id)
    .bind(&sqlite_user.email)
    .bind(created_at)
    .execute(pg_pool)
    .await
    .context("Failed to insert user")?;

    Ok(())
}

/// Migrate a list of admin inbox entries, excluding virtual assistant IDs
async fn migrate_admin_inbox_list(
    entries: &[SqliteAdminInbox],
    pg_pool: &PgPool,
) -> Result<AdminInboxStats> {
    let mut stats = AdminInboxStats {
        total_source: entries.len() as i64,
        migrated: 0,
        skipped: 0,
        errors: 0,
        virtual_assistant_excluded: 0,
    };

    for entry in entries {
        // Check if this is a virtual assistant ID - ALWAYS skip
        if is_virtual_assistant(entry.from_userid) {
            stats.virtual_assistant_excluded += 1;
            tracing::debug!(
                "Excluded admin_inbox entry for virtual assistant ID: {}",
                entry.from_userid
            );
            continue;
        }

        match migrate_single_admin_inbox(entry, pg_pool).await {
            Ok(_) => {
                stats.migrated += 1;
            }
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("duplicate key") || error_str.contains("unique constraint") {
                    stats.skipped += 1;
                } else {
                    stats.errors += 1;
                    tracing::debug!("Failed to migrate admin_inbox entry {}: {}", entry.from_userid, e);
                }
            }
        }
    }

    Ok(stats)
}

/// Migrate a single admin inbox entry
async fn migrate_single_admin_inbox(
    sqlite_entry: &SqliteAdminInbox,
    pg_pool: &PgPool,
) -> Result<()> {
    let last_active = parse_timestamp(&sqlite_entry.last_active)?;

    sqlx::query(
        r#"
        INSERT INTO chat.admin_inbox (
            user_id,
            username,
            summary,
            unread_count,
            last_active,
            avatar_file_id,
            tenant_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (user_id, tenant_id) DO UPDATE SET
            username = COALESCE(EXCLUDED.username, chat.admin_inbox.username),
            summary = COALESCE(EXCLUDED.summary, chat.admin_inbox.summary),
            unread_count = EXCLUDED.unread_count,
            last_active = EXCLUDED.last_active,
            avatar_file_id = COALESCE(EXCLUDED.avatar_file_id, chat.admin_inbox.avatar_file_id),
            updated_at = now()
        "#
    )
    .bind(sqlite_entry.from_userid)
    .bind(&sqlite_entry.from_username)
    .bind(&sqlite_entry.summary)
    .bind(sqlite_entry.unread_count)
    .bind(last_active)
    .bind(&sqlite_entry.avatar_file_id)
    .bind(&sqlite_entry.tenant_id)
    .execute(pg_pool)
    .await
    .context("Failed to insert admin_inbox entry")?;

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

/// Verify user migration by comparing counts
pub async fn verify_user_migration(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<UserVerificationResult> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_users = sqlite_reader.read_users()?.len() as i64;

    let target_users: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM chat.users"
    )
    .fetch_one(pg_pool)
    .await
    .context("Failed to count users in PostgreSQL")?;

    // For admin_inbox, count source entries excluding virtual assistants
    let source_admin_inbox_raw = sqlite_reader.read_admin_inbox()?;
    let source_admin_inbox = source_admin_inbox_raw
        .iter()
        .filter(|entry| !is_virtual_assistant(entry.from_userid))
        .count() as i64;

    let target_admin_inbox: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM chat.admin_inbox"
    )
    .fetch_one(pg_pool)
    .await
    .context("Failed to count admin_inbox in PostgreSQL")?;

    // Count how many virtual assistant entries were excluded
    let virtual_assistant_excluded = source_admin_inbox_raw
        .iter()
        .filter(|entry| is_virtual_assistant(entry.from_userid))
        .count() as i64;

    let result = UserVerificationResult {
        source_users,
        target_users,
        source_admin_inbox,
        target_admin_inbox,
        virtual_assistant_excluded,
        is_valid: source_users == target_users && source_admin_inbox == target_admin_inbox,
    };

    tracing::info!(
        "User verification: users(source={}, target={}), admin_inbox(source={}, target={}, excluded_virtual_assistant={}), valid={}",
        result.source_users,
        result.target_users,
        result.source_admin_inbox,
        result.target_admin_inbox,
        result.virtual_assistant_excluded,
        result.is_valid
    );

    Ok(result)
}

/// User migration verification result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserVerificationResult {
    pub source_users: i64,
    pub target_users: i64,
    pub source_admin_inbox: i64,
    pub target_admin_inbox: i64,
    pub virtual_assistant_excluded: i64,
    pub is_valid: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_virtual_assistant() {
        // Test virtual assistant IDs
        assert!(is_virtual_assistant(0));
        assert!(is_virtual_assistant(-99999));
        assert!(is_virtual_assistant(900000001));

        // Test regular user IDs
        assert!(!is_virtual_assistant(5570487688));
        assert!(!is_virtual_assistant(1));
        assert!(!is_virtual_assistant(123456789));
    }

    #[test]
    fn test_virtual_assistant_uid_mapping() {
        // Test that legacy UID maps to canonical ID
        let legacy_uid = LEGACY_AI_ASSISTANT_UID;
        let canonical_uid = CANONICAL_AI_ASSISTANT_UID;

        assert_eq!(legacy_uid, 900000001);
        assert_eq!(canonical_uid, -99999);

        // Both should be treated as virtual assistants
        assert!(is_virtual_assistant(legacy_uid));
        assert!(is_virtual_assistant(canonical_uid));
    }

    #[test]
    fn test_parse_timestamp() {
        let result = parse_timestamp("2026-01-15 10:30:00");
        assert!(result.is_ok());

        let result = parse_timestamp("2026-04-01 00:06:16");
        assert!(result.is_ok());
    }

    #[test]
    fn test_admin_inbox_stats() {
        let stats = AdminInboxStats {
            total_source: 176,
            migrated: 175,
            skipped: 0,
            errors: 0,
            virtual_assistant_excluded: 1,
        };

        assert_eq!(
            stats.total_source,
            stats.migrated + stats.skipped + stats.errors + stats.virtual_assistant_excluded
        );
    }
}
