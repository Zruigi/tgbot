//! Data Verification - Verify data integrity after migration
//! This module provides comprehensive data verification by comparing
//! source SQLite counts with target PostgreSQL counts.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::sqlite_reader::{SqliteConfig, SqliteReader};

/// Comprehensive verification result for all migrated data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationVerificationReport {
    pub merchants: MerchantVerification,
    pub messages: MessageVerification,
    pub groups: GroupVerification,
    pub group_messages: GroupMessageVerification,
    pub users: UserVerification,
    pub admin_inbox: AdminInboxVerification,
    pub overall: OverallVerification,
}

/// Merchant migration verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerchantVerification {
    pub source_count: i64,
    pub target_count: i64,
    pub migrated_count: i64,
    pub is_valid: bool,
    pub details: String,
}

/// Message migration verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageVerification {
    pub source_count: i64,
    pub target_count: i64,
    pub is_valid: bool,
    pub details: String,
}

/// Group migration verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVerification {
    pub source_groups: i64,
    pub target_groups: i64,
    pub source_members: i64,
    pub target_members: i64,
    pub is_valid: bool,
    pub details: String,
}

/// Group message migration verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMessageVerification {
    pub source_count: i64,
    pub target_count: i64,
    pub is_valid: bool,
    pub details: String,
}

/// User migration verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserVerification {
    pub source_count: i64,
    pub target_count: i64,
    pub is_valid: bool,
    pub details: String,
}

/// Admin inbox migration verification (excluding virtual assistants)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminInboxVerification {
    pub source_count: i64,
    pub target_count: i64,
    pub virtual_assistant_excluded: i64,
    pub is_valid: bool,
    pub details: String,
}

/// Overall verification summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverallVerification {
    pub all_valid: bool,
    pub total_source: i64,
    pub total_target: i64,
    pub summary: String,
}

/// Verify all migrated data counts
pub async fn verify_all_counts(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<MigrationVerificationReport> {
    tracing::info!("Starting comprehensive migration verification");

    // Verify merchants
    let merchant_verification = verify_merchants(sqlite_config, pg_pool).await?;

    // Verify messages
    let message_verification = verify_messages(sqlite_config, pg_pool).await?;

    // Verify groups
    let group_verification = verify_groups(sqlite_config, pg_pool).await?;

    // Verify group messages
    let group_message_verification = verify_group_messages(sqlite_config, pg_pool).await?;

    // Verify users
    let user_verification = verify_users(sqlite_config, pg_pool).await?;

    // Verify admin inbox
    let admin_inbox_verification = verify_admin_inbox(sqlite_config, pg_pool).await?;

    // Calculate overall verification
    let total_source = merchant_verification.source_count
        + message_verification.source_count
        + group_verification.source_groups
        + group_verification.source_members
        + group_message_verification.source_count
        + user_verification.source_count
        + admin_inbox_verification.source_count;

    let total_target = merchant_verification.target_count
        + message_verification.target_count
        + group_verification.target_groups
        + group_verification.target_members
        + group_message_verification.target_count
        + user_verification.target_count
        + admin_inbox_verification.target_count;

    let all_valid = merchant_verification.is_valid
        && message_verification.is_valid
        && group_verification.is_valid
        && group_message_verification.is_valid
        && user_verification.is_valid
        && admin_inbox_verification.is_valid;

    let summary = if all_valid {
        format!(
            "All data verified successfully. Source: {}, Target: {}",
            total_source, total_target
        )
    } else {
        let mut issues = Vec::new();
        if !merchant_verification.is_valid {
            issues.push("merchants mismatch");
        }
        if !message_verification.is_valid {
            issues.push("messages mismatch");
        }
        if !group_verification.is_valid {
            issues.push("groups/members mismatch");
        }
        if !group_message_verification.is_valid {
            issues.push("group_messages mismatch");
        }
        if !user_verification.is_valid {
            issues.push("users mismatch");
        }
        if !admin_inbox_verification.is_valid {
            issues.push("admin_inbox mismatch");
        }
        format!(
            "Verification failed: {}. Source: {}, Target: {}",
            issues.join(", "),
            total_source,
            total_target
        )
    };

    let overall = OverallVerification {
        all_valid,
        total_source,
        total_target,
        summary,
    };

    let report = MigrationVerificationReport {
        merchants: merchant_verification,
        messages: message_verification,
        groups: group_verification,
        group_messages: group_message_verification,
        users: user_verification,
        admin_inbox: admin_inbox_verification,
        overall,
    };

    tracing::info!("Verification complete: {}", report.overall.summary);

    Ok(report)
}

/// Verify merchant migration counts
async fn verify_merchants(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<MerchantVerification> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_merchants = sqlite_reader.read_merchants()?;
    let source_count = source_merchants.len() as i64;

    let target_count: i64 = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM merchants")
        .fetch_one(pg_pool)
        .await?;

    let migrated_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM merchants WHERE legacy_merchant_id IS NOT NULL",
    )
    .fetch_one(pg_pool)
    .await?;

    let is_valid = source_count == target_count || migrated_count == source_count;

    let details = if is_valid {
        format!(
            "Source: {}, Target: {}, Migrated: {}",
            source_count, target_count, migrated_count
        )
    } else {
        format!(
            "MISMATCH - Source: {}, Target: {}, Migrated: {}",
            source_count, target_count, migrated_count
        )
    };

    tracing::info!("Merchant verification: {}", details);

    Ok(MerchantVerification {
        source_count,
        target_count,
        migrated_count,
        is_valid,
        details,
    })
}

/// Verify message migration counts
async fn verify_messages(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<MessageVerification> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_count = sqlite_reader.count_messages()?;

    let target_count: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM chat.messages")
            .fetch_one(pg_pool)
            .await?;

    // Allow some tolerance for messages (they may have been partially migrated before)
    let is_valid = target_count >= source_count - (source_count / 10); // Allow 10% difference

    let details = if is_valid {
        format!(
            "Source: {}, Target: {} (within acceptable range)",
            source_count, target_count
        )
    } else {
        format!(
            "MISMATCH - Source: {}, Target: {}",
            source_count, target_count
        )
    };

    tracing::info!("Message verification: {}", details);

    Ok(MessageVerification {
        source_count,
        target_count,
        is_valid,
        details,
    })
}

/// Verify group and member migration counts
async fn verify_groups(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<GroupVerification> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;

    let source_groups = sqlite_reader.read_groups()?.len() as i64;
    let source_members = sqlite_reader.read_all_group_members()?.len() as i64;

    let target_groups: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM groups.tracked_groups")
            .fetch_one(pg_pool)
            .await?;

    let target_members: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM groups.group_members")
            .fetch_one(pg_pool)
            .await?;

    let is_valid = source_groups == target_groups && source_members == target_members;

    let details = if is_valid {
        format!(
            "Groups: {}/{}, Members: {}/{}",
            source_groups, target_groups, source_members, target_members
        )
    } else {
        format!(
            "MISMATCH - Groups: {}/{}, Members: {}/{}",
            source_groups, target_groups, source_members, target_members
        )
    };

    tracing::info!("Group verification: {}", details);

    Ok(GroupVerification {
        source_groups,
        target_groups,
        source_members,
        target_members,
        is_valid,
        details,
    })
}

/// Verify group message migration counts
async fn verify_group_messages(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<GroupMessageVerification> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_count = sqlite_reader.count_group_messages()?;

    let target_count: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM groups.group_messages")
            .fetch_one(pg_pool)
            .await?;

    let is_valid = source_count == target_count;

    let details = if is_valid {
        format!("Source: {}, Target: {}", source_count, target_count)
    } else {
        format!(
            "MISMATCH - Source: {}, Target: {}",
            source_count, target_count
        )
    };

    tracing::info!("Group message verification: {}", details);

    Ok(GroupMessageVerification {
        source_count,
        target_count,
        is_valid,
        details,
    })
}

/// Verify user migration counts
async fn verify_users(sqlite_config: &SqliteConfig, pg_pool: &PgPool) -> Result<UserVerification> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_count = sqlite_reader.count_users()?;

    let target_count: i64 = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM chat.users")
        .fetch_one(pg_pool)
        .await?;

    let is_valid = source_count == target_count;

    let details = if is_valid {
        format!("Source: {}, Target: {}", source_count, target_count)
    } else {
        format!(
            "MISMATCH - Source: {}, Target: {}",
            source_count, target_count
        )
    };

    tracing::info!("User verification: {}", details);

    Ok(UserVerification {
        source_count,
        target_count,
        is_valid,
        details,
    })
}

/// Verify admin inbox migration counts (excluding virtual assistants)
async fn verify_admin_inbox(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<AdminInboxVerification> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_raw = sqlite_reader.read_admin_inbox()?;

    // Count virtual assistant IDs to exclude
    let virtual_assistant_ids = [0i64, -99999i64, 900000001i64];
    let mut virtual_assistant_excluded = 0i64;
    let mut source_count = 0i64;

    for entry in &source_raw {
        if virtual_assistant_ids.contains(&entry.from_userid) {
            virtual_assistant_excluded += 1;
        } else {
            source_count += 1;
        }
    }

    let target_count: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM chat.admin_inbox")
            .fetch_one(pg_pool)
            .await?;

    let is_valid = source_count == target_count;

    let details = if is_valid {
        format!(
            "Source: {} (excluded {} virtual assistants), Target: {}",
            source_count, virtual_assistant_excluded, target_count
        )
    } else {
        format!(
            "MISMATCH - Source: {} (excluded {} virtual assistants), Target: {}",
            source_count, virtual_assistant_excluded, target_count
        )
    };

    tracing::info!("Admin inbox verification: {}", details);

    Ok(AdminInboxVerification {
        source_count,
        target_count,
        virtual_assistant_excluded,
        is_valid,
        details,
    })
}

/// Quick count verification (without full verification report)
pub async fn quick_verify(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<QuickVerifyResult> {
    let sqlite_reader = SqliteReader::new(sqlite_config)?;

    let sqlite_counts = sqlite_reader.get_table_counts()?;

    let merchants: i64 = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM merchants")
        .fetch_one(pg_pool)
        .await?;

    let messages: i64 = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM chat.messages")
        .fetch_one(pg_pool)
        .await?;

    let groups: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM groups.tracked_groups")
            .fetch_one(pg_pool)
            .await?;

    let group_members: i64 =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM groups.group_members")
            .fetch_one(pg_pool)
            .await?;

    Ok(QuickVerifyResult {
        source_merchants: sqlite_counts.merchants,
        target_merchants: merchants,
        source_messages: sqlite_counts.messages,
        target_messages: messages,
        source_groups: sqlite_counts.groups,
        target_groups: groups,
        source_group_members: sqlite_counts.group_members,
        target_group_members: group_members,
    })
}

/// Quick verification result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickVerifyResult {
    pub source_merchants: i64,
    pub target_merchants: i64,
    pub source_messages: i64,
    pub target_messages: i64,
    pub source_groups: i64,
    pub target_groups: i64,
    pub source_group_members: i64,
    pub target_group_members: i64,
}

impl QuickVerifyResult {
    pub fn is_healthy(&self) -> bool {
        // Allow some tolerance for messages
        let msg_valid = self.target_messages >= self.source_messages - (self.source_messages / 10);

        self.target_merchants >= self.source_merchants
            && msg_valid
            && self.target_groups >= self.source_groups
            && self.target_group_members >= self.source_group_members
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quick_verify_healthy() {
        let result = QuickVerifyResult {
            source_merchants: 10,
            target_merchants: 10,
            source_messages: 100,
            target_messages: 100,
            source_groups: 5,
            target_groups: 5,
            source_group_members: 50,
            target_group_members: 50,
        };
        assert!(result.is_healthy());
    }

    #[test]
    fn test_quick_verify_healthy_with_tolerance() {
        let result = QuickVerifyResult {
            source_merchants: 10,
            target_merchants: 10,
            source_messages: 100,
            target_messages: 95, // 5% difference, within 10% tolerance
            source_groups: 5,
            target_groups: 5,
            source_group_members: 50,
            target_group_members: 50,
        };
        assert!(result.is_healthy());
    }

    #[test]
    fn test_quick_verify_unhealthy() {
        let result = QuickVerifyResult {
            source_merchants: 10,
            target_merchants: 5, // Missing merchants
            source_messages: 100,
            target_messages: 50,
            source_groups: 5,
            target_groups: 5,
            source_group_members: 50,
            target_group_members: 50,
        };
        assert!(!result.is_healthy());
    }
}
