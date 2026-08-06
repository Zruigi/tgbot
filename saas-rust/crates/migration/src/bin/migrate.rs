//! SQLite to PostgreSQL migration tool for tgbot

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use std::env;
use tgbot_migration::{
    migrate_group_messages::migrate_group_messages,
    migrate_groups::migrate_groups,
    migrate_merchants::migrate_merchants,
    migrate_messages::migrate_messages,
    migrate_small_tables::migrate_small_tables,
    migrate_users::migrate_users,
    migration_audit::MigrationFailure,
    sqlite_reader::SqliteConfig,
    verify_counts::{verify_all_counts, MigrationVerificationReport},
};
use uuid::Uuid;

#[derive(Clone, Copy, serde::Serialize)]
struct MigrationPhaseStats<'a> {
    name: &'a str,
    #[serde(rename = "source_count")]
    total_source: i64,
    #[serde(rename = "migrated_count")]
    migrated: i64,
    skipped: i64,
    errors: i64,
    excluded: i64,
}

fn validate_phase_stats(phases: &[MigrationPhaseStats<'_>]) -> Result<()> {
    let issues = phases
        .iter()
        .filter_map(|phase| {
            let accounted = phase.migrated + phase.skipped + phase.errors + phase.excluded;
            if phase.skipped == 0 && phase.errors == 0 && accounted == phase.total_source {
                None
            } else {
                Some(format!(
                    "{}: source={}, migrated={}, skipped={}, errors={}, excluded={}, accounted={}",
                    phase.name,
                    phase.total_source,
                    phase.migrated,
                    phase.skipped,
                    phase.errors,
                    phase.excluded,
                    accounted
                ))
            }
        })
        .collect::<Vec<_>>();

    if issues.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "migration has unaccepted or unaccounted source rows: {}; export a row-level audit and obtain an explicit disposition before accepting the migration",
            issues.join("; ")
        ))
    }
}

fn migration_requires_disposition(
    failures: &[MigrationFailure],
    phase_stats_valid: bool,
    verification_valid: bool,
) -> bool {
    !failures.is_empty() || !phase_stats_valid || !verification_valid
}

#[derive(serde::Serialize)]
struct MigrationAuditPayload<'a> {
    schema_version: u32,
    generated_at: String,
    run_id: String,
    operator: String,
    status: &'static str,
    verification: &'a MigrationVerificationReport,
    phase_stats: &'a [MigrationPhaseStats<'a>],
    records: &'a [MigrationFailure],
}

#[derive(serde::Serialize)]
struct MigrationAuditReport<'a> {
    #[serde(flatten)]
    payload: MigrationAuditPayload<'a>,
    checksum_sha256: String,
}

fn write_migration_audit_report(
    failures: &[MigrationFailure],
    verification: &MigrationVerificationReport,
    phase_stats: &[MigrationPhaseStats<'_>],
) -> Result<()> {
    let report_path = env::var("MIGRATION_AUDIT_PATH")
        .context("migration failures require MIGRATION_AUDIT_PATH for row-level audit export")?;
    let operator = env::var("MIGRATION_AUDIT_OPERATOR")
        .context("migration failures require MIGRATION_AUDIT_OPERATOR for audit ownership")?;
    let run_id = env::var("MIGRATION_RUN_ID").unwrap_or_else(|_| Uuid::new_v4().to_string());
    let payload = MigrationAuditPayload {
        schema_version: 1,
        generated_at: chrono::Utc::now().to_rfc3339(),
        run_id,
        operator,
        status: "requires_disposition",
        verification,
        phase_stats,
        records: failures,
    };
    let payload_bytes =
        serde_json::to_vec(&payload).context("failed to serialize audit payload")?;
    let checksum_sha256 = hex::encode(Sha256::digest(&payload_bytes));
    let report = MigrationAuditReport {
        payload,
        checksum_sha256,
    };
    let report_bytes =
        serde_json::to_vec_pretty(&report).context("failed to serialize migration audit report")?;

    std::fs::write(&report_path, report_bytes)
        .with_context(|| format!("failed to write migration audit report to {report_path}"))?;
    tracing::warn!(
        path = %report_path,
        failure_count = failures.len(),
        "migration row-level audit report written; explicit disposition is still required"
    );
    Ok(())
}

fn attach_failure_counts(
    failure: MigrationFailure,
    verification: &MigrationVerificationReport,
) -> MigrationFailure {
    let counts = match failure.source_table.as_str() {
        "merchants" => Some((
            verification.merchants.source_count,
            verification.merchants.target_count,
        )),
        "messages" => Some((
            verification.messages.source_count,
            verification.messages.target_count,
        )),
        "groups" => Some((
            verification.groups.source_groups,
            verification.groups.target_groups,
        )),
        "group_members" => Some((
            verification.groups.source_members,
            verification.groups.target_members,
        )),
        "group_messages" => Some((
            verification.group_messages.source_count,
            verification.group_messages.target_count,
        )),
        "users" => Some((
            verification.users.source_count,
            verification.users.target_count,
        )),
        "admin_inbox" => Some((
            verification.admin_inbox.source_count,
            verification.admin_inbox.target_count,
        )),
        "plans" => Some((
            verification.small_tables.plans.source_count,
            verification.small_tables.plans.target_count,
        )),
        "address_monitors" => Some((
            verification.small_tables.address_monitors.source_count,
            verification.small_tables.address_monitors.target_count,
        )),
        "monitor_transactions" => Some((
            verification.small_tables.monitor_transactions.source_count,
            verification.small_tables.monitor_transactions.target_count,
        )),
        _ => None,
    };
    match counts {
        Some((source_count, target_count)) => failure.with_counts(source_count, target_count),
        None => failure,
    }
}

async fn audit_migrator_role(pool: &sqlx::PgPool, dedicated_role: bool) -> Result<()> {
    let row = sqlx::query(
        "SELECT current_user AS role_name, rolsuper, rolcreaterole, rolcreatedb, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .context("failed to inspect PostgreSQL migration role")?;
    let role_name: String = row.try_get("role_name")?;
    let is_super: bool = row.try_get("rolsuper")?;
    let can_create_role: bool = row.try_get("rolcreaterole")?;
    let can_create_db: bool = row.try_get("rolcreatedb")?;
    let bypass_rls: bool = row.try_get("rolbypassrls")?;

    if dedicated_role {
        if !bypass_rls {
            return Err(anyhow!(
                "MIGRATOR_DATABASE_URL role {role_name} must have BYPASSRLS"
            ));
        }
        if is_super || can_create_role || can_create_db {
            return Err(anyhow!(
                "migration role {role_name} is over-privileged (superuser/create-role/create-db)"
            ));
        }
        tracing::info!(
            role = %role_name,
            bypass_rls,
            "migration role preflight passed; dedicated role is not superuser"
        );
    } else {
        tracing::warn!(
            role = %role_name,
            bypass_rls,
            is_super,
            can_create_role,
            can_create_db,
            "using DATABASE_URL for migration; dedicated BYPASSRLS role was not supplied"
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let sqlite_path = env::var("SQLITE_DB_PATH")
        .map(|path| path.trim().to_owned())
        .map_err(|_| {
            anyhow!("SQLITE_DB_PATH is required; refusing an implicit source database path")
        })?;
    if sqlite_path.is_empty() {
        return Err(anyhow!("SQLITE_DB_PATH must not be empty"));
    }

    // Use MIGRATOR_DATABASE_URL (with BYPASSRLS) if available, otherwise fall back to DATABASE_URL
    let pg_url = env::var("MIGRATOR_DATABASE_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .context("Neither MIGRATOR_DATABASE_URL nor DATABASE_URL environment variable set")?;

    let is_migrator_role = env::var("MIGRATOR_DATABASE_URL").is_ok();

    // Create SQLite config
    let sqlite_config = SqliteConfig::new(&sqlite_path);

    tracing::info!("SQLite database: {}", sqlite_path);
    if is_migrator_role {
        tracing::info!("PostgreSQL connection: using MIGRATOR_DATABASE_URL (BYPASSRLS role)");
    } else {
        tracing::info!("PostgreSQL connection: using DATABASE_URL (RLS-subject role)");
    }

    // Create PostgreSQL connection pool with higher timeout for large migrations
    let pg_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&pg_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    tracing::info!("Connected to PostgreSQL database");
    audit_migrator_role(&pg_pool, is_migrator_role).await?;

    // Run migrations
    tracing::info!("Starting data migration...");

    // 1. Migrate merchants
    tracing::info!("\n=== Phase 1: Merchants ===");
    let merchant_stats = migrate_merchants(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate merchants")?;
    tracing::info!(
        "Merchants: {} migrated, {} skipped, {} errors",
        merchant_stats.migrated,
        merchant_stats.skipped,
        merchant_stats.errors
    );

    // 2. Migrate messages (private chat messages)
    tracing::info!("\n=== Phase 2: Private Chat Messages ===");
    let message_stats = migrate_messages(&sqlite_config, &pg_pool, 100)
        .await
        .context("Failed to migrate messages")?;
    tracing::info!(
        "Messages: {} migrated, {} skipped, {} errors",
        message_stats.migrated,
        message_stats.skipped,
        message_stats.errors
    );

    // 3. Migrate groups and members
    tracing::info!("\n=== Phase 3: Groups and Members ===");
    let group_stats = migrate_groups(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate groups")?;
    tracing::info!(
        "Groups: {} migrated, {} skipped, {} errors",
        group_stats.groups.migrated,
        group_stats.groups.skipped,
        group_stats.groups.errors
    );
    tracing::info!(
        "Members: {} migrated, {} skipped, {} errors",
        group_stats.members.migrated,
        group_stats.members.skipped,
        group_stats.members.errors
    );

    // 4. Migrate group messages
    tracing::info!("\n=== Phase 4: Group Messages ===");
    let group_msg_stats = migrate_group_messages(&sqlite_config, &pg_pool, 500)
        .await
        .context("Failed to migrate group messages")?;
    tracing::info!(
        "Group Messages: {} migrated, {} skipped, {} errors",
        group_msg_stats.migrated,
        group_msg_stats.skipped,
        group_msg_stats.errors
    );

    // 5. Migrate users and admin inbox
    tracing::info!("\n=== Phase 5: Users and Admin Inbox ===");
    let user_stats = migrate_users(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate users")?;
    tracing::info!(
        "Users: {} migrated, {} skipped, {} errors",
        user_stats.users.migrated,
        user_stats.users.skipped,
        user_stats.users.errors
    );
    tracing::info!(
        "Admin Inbox: {} migrated, {} skipped, {} errors, {} virtual_assistant_excluded",
        user_stats.admin_inbox.migrated,
        user_stats.admin_inbox.skipped,
        user_stats.admin_inbox.errors,
        user_stats.admin_inbox.virtual_assistant_excluded
    );

    // 6. Migrate small tables (plans, address_monitors, monitor_transactions)
    tracing::info!("\n=== Phase 6: Small Tables ===");
    let small_stats = migrate_small_tables(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate small tables")?;
    tracing::info!(
        "Plans: {} migrated, {} skipped, {} errors",
        small_stats.plans.migrated,
        small_stats.plans.skipped,
        small_stats.plans.errors
    );
    tracing::info!(
        "Address Monitors: {} migrated, {} skipped, {} errors",
        small_stats.address_monitors.migrated,
        small_stats.address_monitors.skipped,
        small_stats.address_monitors.errors
    );
    tracing::info!(
        "Monitor Transactions: {} migrated, {} skipped, {} errors",
        small_stats.monitor_transactions.migrated,
        small_stats.monitor_transactions.skipped,
        small_stats.monitor_transactions.errors
    );

    let row_level_failures = merchant_stats
        .failures
        .iter()
        .chain(message_stats.failures.iter())
        .chain(group_stats.groups.failures.iter())
        .chain(group_stats.members.failures.iter())
        .chain(group_msg_stats.failures.iter())
        .chain(user_stats.users.failures.iter())
        .chain(user_stats.admin_inbox.failures.iter())
        .chain(small_stats.plans.failures.iter())
        .chain(small_stats.address_monitors.failures.iter())
        .chain(small_stats.monitor_transactions.failures.iter())
        .cloned()
        .collect::<Vec<_>>();
    // Verify data integrity
    tracing::info!("\n=== Verification Phase ===");
    let verification = verify_all_counts(&sqlite_config, &pg_pool)
        .await
        .context("Failed to verify migration")?;

    // Print verification results
    tracing::info!("\n=== VERIFICATION RESULTS ===");
    tracing::info!("Merchants: {}", verification.merchants.details);
    tracing::info!("Messages: {}", verification.messages.details);
    tracing::info!("Groups: {}", verification.groups.details);
    tracing::info!("Group Messages: {}", verification.group_messages.details);
    tracing::info!("Users: {}", verification.users.details);
    tracing::info!("Admin Inbox: {}", verification.admin_inbox.details);
    tracing::info!("Small Tables: {}", verification.small_tables.details);
    tracing::info!("\nOverall: {}", verification.overall.summary);

    let phase_stats = [
        MigrationPhaseStats {
            name: "merchants",
            total_source: merchant_stats.total_source,
            migrated: merchant_stats.migrated,
            skipped: merchant_stats.skipped,
            errors: merchant_stats.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "messages",
            total_source: message_stats.total_source,
            migrated: message_stats.migrated,
            skipped: message_stats.skipped,
            errors: message_stats.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "groups",
            total_source: group_stats.groups.total_source,
            migrated: group_stats.groups.migrated,
            skipped: group_stats.groups.skipped,
            errors: group_stats.groups.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "group_members",
            total_source: group_stats.members.total_source,
            migrated: group_stats.members.migrated,
            skipped: group_stats.members.skipped,
            errors: group_stats.members.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "group_messages",
            total_source: group_msg_stats.total_source,
            migrated: group_msg_stats.migrated,
            skipped: group_msg_stats.skipped,
            errors: group_msg_stats.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "users",
            total_source: user_stats.users.total_source,
            migrated: user_stats.users.migrated,
            skipped: user_stats.users.skipped,
            errors: user_stats.users.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "admin_inbox",
            total_source: user_stats.admin_inbox.total_source,
            migrated: user_stats.admin_inbox.migrated,
            skipped: user_stats.admin_inbox.skipped,
            errors: user_stats.admin_inbox.errors,
            excluded: user_stats.admin_inbox.virtual_assistant_excluded,
        },
        MigrationPhaseStats {
            name: "plans",
            total_source: small_stats.plans.total_source,
            migrated: small_stats.plans.migrated,
            skipped: small_stats.plans.skipped,
            errors: small_stats.plans.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "address_monitors",
            total_source: small_stats.address_monitors.total_source,
            migrated: small_stats.address_monitors.migrated,
            skipped: small_stats.address_monitors.skipped,
            errors: small_stats.address_monitors.errors,
            excluded: 0,
        },
        MigrationPhaseStats {
            name: "monitor_transactions",
            total_source: small_stats.monitor_transactions.total_source,
            migrated: small_stats.monitor_transactions.migrated,
            skipped: small_stats.monitor_transactions.skipped,
            errors: small_stats.monitor_transactions.errors,
            excluded: 0,
        },
    ];
    let row_level_failures = row_level_failures
        .into_iter()
        .map(|failure| attach_failure_counts(failure, &verification))
        .collect::<Vec<_>>();
    let phase_validation = validate_phase_stats(&phase_stats);
    let requires_disposition = migration_requires_disposition(
        &row_level_failures,
        phase_validation.is_ok(),
        verification.overall.all_valid,
    );
    if requires_disposition {
        // Also emit an audit when aggregate accounting or verification finds a
        // gap without a row-level converter error. The phase stats preserve the
        // exact unresolved boundary for an authorized source-snapshot audit.
        write_migration_audit_report(&row_level_failures, &verification, &phase_stats)?;
    }
    phase_validation?;

    if verification.overall.all_valid {
        tracing::info!("\n✓ Migration completed successfully!");
        Ok(())
    } else {
        Err(anyhow!("Migration verification failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_stats_accept_complete_migration_and_explicit_exclusion() {
        let phases = [
            MigrationPhaseStats {
                name: "messages",
                total_source: 10,
                migrated: 10,
                skipped: 0,
                errors: 0,
                excluded: 0,
            },
            MigrationPhaseStats {
                name: "admin_inbox",
                total_source: 4,
                migrated: 3,
                skipped: 0,
                errors: 0,
                excluded: 1,
            },
        ];

        assert!(validate_phase_stats(&phases).is_ok());
    }

    #[test]
    fn phase_stats_reject_skips_errors_and_hidden_gaps() {
        for phase in [
            MigrationPhaseStats {
                name: "skipped",
                total_source: 10,
                migrated: 9,
                skipped: 1,
                errors: 0,
                excluded: 0,
            },
            MigrationPhaseStats {
                name: "errors",
                total_source: 10,
                migrated: 9,
                skipped: 0,
                errors: 1,
                excluded: 0,
            },
            MigrationPhaseStats {
                name: "hidden-gap",
                total_source: 10,
                migrated: 9,
                skipped: 0,
                errors: 0,
                excluded: 0,
            },
        ] {
            assert!(validate_phase_stats(&[phase]).is_err());
        }
    }

    #[test]
    fn disposition_audit_covers_aggregate_gaps_without_row_failures() {
        assert!(migration_requires_disposition(&[], false, true));
        assert!(migration_requires_disposition(&[], true, false));
        assert!(!migration_requires_disposition(&[], true, true));
    }
}
