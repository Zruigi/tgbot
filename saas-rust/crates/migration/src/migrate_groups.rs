//! Group Migration - Migrate group and member data from SQLite to PostgreSQL
//! This module handles the migration of group tracking data from the legacy
//! SQLite database to the groups.* tables in PostgreSQL/Supabase.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;

use crate::sqlite_reader::{SqliteConfig, SqliteGroup, SqliteGroupMember, SqliteReader};

/// Migration statistics for groups
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupMigrationStats {
    pub groups: GroupStats,
    pub members: GroupMemberStats,
}

/// Statistics for tracked groups migration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
}

/// Statistics for group members migration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupMemberStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
}

/// Migrate groups from SQLite to PostgreSQL
pub async fn migrate_groups(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<GroupMigrationStats> {
    tracing::info!("Starting group migration from SQLite to PostgreSQL");

    let sqlite_reader = SqliteReader::new(sqlite_config)?;

    // Migrate groups
    let source_groups = sqlite_reader.read_groups()?;
    let group_stats = migrate_group_list(&source_groups, pg_pool).await?;

    // Migrate group members
    let source_members = sqlite_reader.read_all_group_members()?;
    let member_stats = migrate_member_list(&source_members, pg_pool).await?;

    let stats = GroupMigrationStats {
        groups: group_stats,
        members: member_stats,
    };

    tracing::info!(
        "Group migration complete: groups({} migrated, {} skipped, {} errors), members({} migrated, {} skipped, {} errors)",
        stats.groups.migrated,
        stats.groups.skipped,
        stats.groups.errors,
        stats.members.migrated,
        stats.members.skipped,
        stats.members.errors
    );

    Ok(stats)
}

/// Migrate a list of groups
async fn migrate_group_list(groups: &[SqliteGroup], pg_pool: &PgPool) -> Result<GroupStats> {
    let mut stats = GroupStats {
        total_source: groups.len() as i64,
        migrated: 0,
        skipped: 0,
        errors: 0,
    };

    for group in groups {
        match migrate_single_group(group, pg_pool).await {
            Ok(_) => {
                stats.migrated += 1;
                tracing::debug!("Migrated group: {}", group.group_id);
            }
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("duplicate key") || error_str.contains("unique constraint") {
                    stats.skipped += 1;
                } else {
                    stats.errors += 1;
                    tracing::error!("Failed to migrate group {}: {}", group.group_id, e);
                }
            }
        }
    }

    Ok(stats)
}

/// Migrate a single group
async fn migrate_single_group(sqlite_group: &SqliteGroup, pg_pool: &PgPool) -> Result<()> {
    let first_seen = parse_timestamp(&sqlite_group.first_seen)?;
    let updated_at = parse_timestamp(&sqlite_group.updated_at)?;
    let verify_type = sqlite_group.verify_type.clone();
    let verify_timeout = sqlite_group.verify_timeout;

    sqlx::query(
        r#"
        INSERT INTO groups.tracked_groups (
            group_id,
            group_title,
            tenant_id,
            is_monitored,
            custom_keywords,
            first_seen,
            updated_at,
            delete_on_revoke,
            revoke_sync_chat_id,
            verify_enabled,
            verify_type,
            verify_timeout,
            verify_channel_id,
            verify_questions
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        ON CONFLICT (group_id) DO UPDATE SET
            group_title = COALESCE(EXCLUDED.group_title, tracked_groups.group_title),
            tenant_id = COALESCE(EXCLUDED.tenant_id, tracked_groups.tenant_id),
            is_monitored = COALESCE(EXCLUDED.is_monitored, tracked_groups.is_monitored),
            custom_keywords = COALESCE(EXCLUDED.custom_keywords, tracked_groups.custom_keywords),
            updated_at = EXCLUDED.updated_at,
            delete_on_revoke = COALESCE(EXCLUDED.delete_on_revoke, tracked_groups.delete_on_revoke),
            revoke_sync_chat_id = COALESCE(EXCLUDED.revoke_sync_chat_id, tracked_groups.revoke_sync_chat_id),
            verify_enabled = COALESCE(EXCLUDED.verify_enabled, tracked_groups.verify_enabled),
            verify_type = COALESCE(EXCLUDED.verify_type, tracked_groups.verify_type),
            verify_timeout = COALESCE(EXCLUDED.verify_timeout, tracked_groups.verify_timeout),
            verify_channel_id = COALESCE(EXCLUDED.verify_channel_id, tracked_groups.verify_channel_id),
            verify_questions = COALESCE(EXCLUDED.verify_questions, tracked_groups.verify_questions)
        "#
    )
    .bind(sqlite_group.group_id)
    .bind(&sqlite_group.group_title)
    .bind(&sqlite_group.tenant_id)
    .bind(sqlite_group.is_monitored)
    .bind(&sqlite_group.custom_keywords)
    .bind(first_seen)
    .bind(updated_at)
    .bind(sqlite_group.delete_on_revoke)
    .bind(sqlite_group.revoke_sync_chat_id)
    .bind(sqlite_group.verify_enabled)
    .bind(&verify_type)
    .bind(verify_timeout)
    .bind(&sqlite_group.verify_channel_id)
    .bind(&sqlite_group.verify_questions)
    .execute(pg_pool)
    .await
    .context("Failed to insert group")?;

    Ok(())
}

/// Migrate a list of group members
async fn migrate_member_list(
    members: &[SqliteGroupMember],
    pg_pool: &PgPool,
) -> Result<GroupMemberStats> {
    let mut stats = GroupMemberStats {
        total_source: members.len() as i64,
        migrated: 0,
        skipped: 0,
        errors: 0,
    };

    for member in members {
        match migrate_single_member(member, pg_pool).await {
            Ok(_) => {
                stats.migrated += 1;
            }
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("duplicate key") || error_str.contains("unique constraint") {
                    stats.skipped += 1;
                } else {
                    stats.errors += 1;
                    tracing::debug!("Failed to migrate member {}: {}", member.id, e);
                }
            }
        }
    }

    Ok(stats)
}

/// Migrate a single group member
async fn migrate_single_member(sqlite_member: &SqliteGroupMember, pg_pool: &PgPool) -> Result<()> {
    let joined_at = parse_timestamp(&sqlite_member.joined_at)?;
    let verified_at = sqlite_member
        .verified_at
        .as_ref()
        .and_then(|s| parse_timestamp(s).ok());

    sqlx::query(
        r#"
        INSERT INTO groups.group_members (
            user_id,
            username,
            first_name,
            last_name,
            group_id,
            group_title,
            tenant_id,
            status,
            joined_at,
            is_verified,
            verified_at,
            avatar_file_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (group_id, user_id) DO UPDATE SET
            username = COALESCE(EXCLUDED.username, groups.group_members.username),
            first_name = COALESCE(EXCLUDED.first_name, groups.group_members.first_name),
            last_name = COALESCE(EXCLUDED.last_name, groups.group_members.last_name),
            group_title = COALESCE(EXCLUDED.group_title, groups.group_members.group_title),
            status = COALESCE(EXCLUDED.status, groups.group_members.status),
            is_verified = COALESCE(EXCLUDED.is_verified, groups.group_members.is_verified),
            verified_at = COALESCE(EXCLUDED.verified_at, groups.group_members.verified_at),
            avatar_file_id = COALESCE(EXCLUDED.avatar_file_id, groups.group_members.avatar_file_id)
        "#,
    )
    .bind(sqlite_member.user_id)
    .bind(&sqlite_member.username)
    .bind(&sqlite_member.first_name)
    .bind(&sqlite_member.last_name)
    .bind(sqlite_member.group_id)
    .bind(&sqlite_member.group_title)
    .bind(&sqlite_member.tenant_id)
    .bind(&sqlite_member.status)
    .bind(joined_at)
    .bind(sqlite_member.is_verified)
    .bind(verified_at)
    .bind(&sqlite_member.avatar_file_id)
    .execute(pg_pool)
    .await
    .context("Failed to insert group member")?;

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

/// Verify group migration by comparing counts
pub async fn verify_group_migration(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<GroupVerificationResult> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_groups = sqlite_reader.read_groups()?.len() as i64;
    let source_members = sqlite_reader.read_all_group_members()?.len() as i64;

    let target_groups: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM groups.tracked_groups")
            .fetch_one(pg_pool)
            .await
            .context("Failed to count groups in PostgreSQL")?;

    let target_members: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM groups.group_members")
            .fetch_one(pg_pool)
            .await
            .context("Failed to count group members in PostgreSQL")?;

    let result = GroupVerificationResult {
        source_groups,
        target_groups,
        source_members,
        target_members,
        is_valid: source_groups == target_groups && source_members == target_members,
    };

    tracing::info!(
        "Group verification: groups(source={}, target={}), members(source={}, target={}), valid={}",
        result.source_groups,
        result.target_groups,
        result.source_members,
        result.target_members,
        result.is_valid
    );

    Ok(result)
}

/// Group migration verification result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GroupVerificationResult {
    pub source_groups: i64,
    pub target_groups: i64,
    pub source_members: i64,
    pub target_members: i64,
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
}
