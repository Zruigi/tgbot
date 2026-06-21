//! Merchant Migration - Migrate merchant data from SQLite to PostgreSQL
//! This module handles the migration of merchant data from the legacy SQLite
//! database to the SaaS PostgreSQL/Supabase database.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::sqlite_reader::{SqliteConfig, SqliteMerchant, SqliteReader};

/// Migration statistics for merchants
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MerchantMigrationStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
    pub details: Vec<MerchantMigrationDetail>,
}

/// Detailed migration result for each merchant
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MerchantMigrationDetail {
    pub legacy_id: i64,
    pub tenant_id: String,
    pub status: String,
    pub error: Option<String>,
}

/// Migrate merchants from SQLite to PostgreSQL
pub async fn migrate_merchants(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<MerchantMigrationStats> {
    tracing::info!("Starting merchant migration from SQLite to PostgreSQL");

    // Read merchants from SQLite
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_merchants = sqlite_reader.read_merchants()?;

    let mut stats = MerchantMigrationStats {
        total_source: source_merchants.len() as i64,
        migrated: 0,
        skipped: 0,
        errors: 0,
        details: Vec::new(),
    };

    for merchant in source_merchants {
        let result = migrate_single_merchant(&merchant, pg_pool).await;

        match result {
            Ok(_) => {
                stats.migrated += 1;
                stats.details.push(MerchantMigrationDetail {
                    legacy_id: merchant.id,
                    tenant_id: merchant.tenant_id.clone(),
                    status: "migrated".to_string(),
                    error: None,
                });
                tracing::debug!("Migrated merchant: {} ({})", merchant.merchant_name, merchant.tenant_id);
            }
            Err(e) => {
                // Check if it's a duplicate key error (already migrated)
                let error_str = e.to_string();
                if error_str.contains("duplicate key") || error_str.contains("unique constraint") {
                    stats.skipped += 1;
                    stats.details.push(MerchantMigrationDetail {
                        legacy_id: merchant.id,
                        tenant_id: merchant.tenant_id.clone(),
                        status: "skipped_duplicate".to_string(),
                        error: Some("Already exists in target".to_string()),
                    });
                } else {
                    stats.errors += 1;
                    stats.details.push(MerchantMigrationDetail {
                        legacy_id: merchant.id,
                        tenant_id: merchant.tenant_id.clone(),
                        status: "error".to_string(),
                        error: Some(e.to_string()),
                    });
                    tracing::error!("Failed to migrate merchant {}: {}", merchant.id, e);
                }
            }
        }
    }

    tracing::info!(
        "Merchant migration complete: {} migrated, {} skipped, {} errors",
        stats.migrated,
        stats.skipped,
        stats.errors
    );

    Ok(stats)
}

/// Migrate a single merchant from SQLite to PostgreSQL
async fn migrate_single_merchant(
    sqlite_merchant: &SqliteMerchant,
    pg_pool: &PgPool,
) -> Result<()> {
    // Parse timestamps
    let created_at = parse_timestamp(&sqlite_merchant.created_at)?;
    let updated_at = parse_timestamp(&sqlite_merchant.updated_at)?;
    let subscription_expires_at = sqlite_merchant.subscription_expires_at
        .as_ref()
        .and_then(|s| parse_timestamp(s).ok());

    // Insert or update merchant in PostgreSQL
    sqlx::query(
        r#"
        INSERT INTO merchants (
            legacy_merchant_id,
            tenant_id,
            owner_tg_user_id,
            owner_username,
            merchant_name,
            bot_id,
            bot_username,
            status,
            plan_type,
            subscription_status,
            subscription_expires_at,
            onboarding_completed,
            runtime_version,
            config_version,
            feature_version,
            created_at,
            updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        ON CONFLICT (legacy_merchant_id) DO UPDATE SET
            tenant_id = EXCLUDED.tenant_id,
            owner_tg_user_id = EXCLUDED.owner_tg_user_id,
            owner_username = EXCLUDED.owner_username,
            merchant_name = EXCLUDED.merchant_name,
            bot_id = EXCLUDED.bot_id,
            bot_username = EXCLUDED.bot_username,
            status = EXCLUDED.status,
            plan_type = EXCLUDED.plan_type,
            subscription_status = EXCLUDED.subscription_status,
            subscription_expires_at = EXCLUDED.subscription_expires_at,
            onboarding_completed = EXCLUDED.onboarding_completed,
            runtime_version = EXCLUDED.runtime_version,
            config_version = EXCLUDED.config_version,
            feature_version = EXCLUDED.feature_version,
            updated_at = EXCLUDED.updated_at
        "#
    )
    .bind(sqlite_merchant.id)
    .bind(&sqlite_merchant.tenant_id)
    .bind(sqlite_merchant.owner_tg_user_id)
    .bind(&sqlite_merchant.merchant_name) // owner_username not in source
    .bind(&sqlite_merchant.merchant_name)
    .bind(sqlite_merchant.bot_id)
    .bind(&sqlite_merchant.bot_username)
    .bind(&sqlite_merchant.status)
    .bind(&sqlite_merchant.plan_type)
    .bind(&sqlite_merchant.subscription_status)
    .bind(subscription_expires_at)
    .bind(sqlite_merchant.onboarding_completed)
    .bind(sqlite_merchant.runtime_version)
    .bind(sqlite_merchant.config_version)
    .bind(sqlite_merchant.feature_version)
    .bind(created_at)
    .bind(updated_at)
    .execute(pg_pool)
    .await
    .context("Failed to insert merchant")?;

    Ok(())
}

/// Parse SQLite timestamp string to DateTime<Utc>
fn parse_timestamp(s: &str) -> Result<DateTime<Utc>> {
    // Try various common formats
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

    // Try parsing as date only
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let datetime = date.and_hms_opt(0, 0, 0).unwrap();
        return Ok(DateTime::from_naive_utc_and_offset(datetime, Utc));
    }

    Err(anyhow::anyhow!("Failed to parse timestamp: {}", s))
}

/// Verify merchant migration by comparing counts
pub async fn verify_merchant_migration(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<MerchantVerificationResult> {
    // Get source count from SQLite
    let sqlite_reader = SqliteReader::new(sqlite_config)?;
    let source_merchants = sqlite_reader.read_merchants()?;
    let source_count = source_merchants.len() as i64;

    // Get target count from PostgreSQL
    let target_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM merchants"
    )
    .fetch_one(pg_pool)
    .await
    .context("Failed to count merchants in PostgreSQL")?
    .unwrap_or(0);

    // Get count of merchants with legacy_id (successfully migrated)
    let migrated_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM merchants WHERE legacy_merchant_id IS NOT NULL"
    )
    .fetch_one(pg_pool)
    .await
    .context("Failed to count migrated merchants in PostgreSQL")?
    .unwrap_or(0);

    let result = MerchantVerificationResult {
        source_count,
        target_count,
        migrated_count,
        is_valid: source_count == target_count || migrated_count == source_count,
    };

    tracing::info!(
        "Merchant verification: source={}, target={}, migrated={}, valid={}",
        result.source_count,
        result.target_count,
        result.migrated_count,
        result.is_valid
    );

    Ok(result)
}

/// Merchant migration verification result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MerchantVerificationResult {
    pub source_count: i64,
    pub target_count: i64,
    pub migrated_count: i64,
    pub is_valid: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp() {
        let result = parse_timestamp("2026-01-15 10:30:00");
        assert!(result.is_ok());

        let result = parse_timestamp("2026-01-15T10:30:00");
        assert!(result.is_ok());
    }
}