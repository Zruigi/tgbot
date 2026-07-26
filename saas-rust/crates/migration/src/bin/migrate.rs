//! SQLite to PostgreSQL migration tool for tgbot

use anyhow::{anyhow, Context, Result};
use std::env;
use sqlx::postgres::PgPoolOptions;
use tgbot_migration::{
    sqlite_reader::SqliteConfig,
    migrate_merchants::migrate_merchants,
    migrate_messages::migrate_messages,
    migrate_groups::migrate_groups,
    migrate_group_messages::migrate_group_messages,
    migrate_users::migrate_users,
    migrate_small_tables::migrate_small_tables,
    verify_counts::verify_all_counts,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Get environment variables
    let sqlite_path = env::var("SQLITE_DB_PATH")
        .unwrap_or_else(|_| "/home/ubuntu/tgbot-rust-beta/../tgbot/crypto_bot.sqlite3".to_string());

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

    // Run migrations
    tracing::info!("Starting data migration...");

    // 1. Migrate merchants
    tracing::info!("\n=== Phase 1: Merchants ===");
    let merchant_stats = migrate_merchants(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate merchants")?;
    tracing::info!("Merchants: {} migrated, {} skipped, {} errors",
        merchant_stats.migrated, merchant_stats.skipped, merchant_stats.errors);

    // 2. Migrate messages (private chat messages)
    tracing::info!("\n=== Phase 2: Private Chat Messages ===");
    let message_stats = migrate_messages(&sqlite_config, &pg_pool, 100)
        .await
        .context("Failed to migrate messages")?;
    tracing::info!("Messages: {} migrated, {} skipped, {} errors",
        message_stats.migrated, message_stats.skipped, message_stats.errors);

    // 3. Migrate groups and members
    tracing::info!("\n=== Phase 3: Groups and Members ===");
    let group_stats = migrate_groups(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate groups")?;
    tracing::info!("Groups: {} migrated, {} skipped, {} errors",
        group_stats.groups.migrated, group_stats.groups.skipped, group_stats.groups.errors);
    tracing::info!("Members: {} migrated, {} skipped, {} errors",
        group_stats.members.migrated, group_stats.members.skipped, group_stats.members.errors);

    // 4. Migrate group messages
    tracing::info!("\n=== Phase 4: Group Messages ===");
    let group_msg_stats = migrate_group_messages(&sqlite_config, &pg_pool, 500)
        .await
        .context("Failed to migrate group messages")?;
    tracing::info!("Group Messages: {} migrated, {} skipped, {} errors",
        group_msg_stats.migrated, group_msg_stats.skipped, group_msg_stats.errors);

    // 5. Migrate users and admin inbox
    tracing::info!("\n=== Phase 5: Users and Admin Inbox ===");
    let user_stats = migrate_users(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate users")?;
    tracing::info!("Users: {} migrated, {} skipped, {} errors",
        user_stats.users.migrated, user_stats.users.skipped, user_stats.users.errors);
    tracing::info!("Admin Inbox: {} migrated, {} skipped, {} errors, {} virtual_assistant_excluded",
        user_stats.admin_inbox.migrated, user_stats.admin_inbox.skipped,
        user_stats.admin_inbox.errors, user_stats.admin_inbox.virtual_assistant_excluded);

    // 6. Migrate small tables (plans, address_monitors, monitor_transactions)
    tracing::info!("\n=== Phase 6: Small Tables ===");
    let small_stats = migrate_small_tables(&sqlite_config, &pg_pool)
        .await
        .context("Failed to migrate small tables")?;
    tracing::info!("Plans: {} migrated, {} skipped, {} errors",
        small_stats.plans.migrated, small_stats.plans.skipped, small_stats.plans.errors);
    tracing::info!("Address Monitors: {} migrated, {} skipped, {} errors",
        small_stats.address_monitors.migrated, small_stats.address_monitors.skipped, small_stats.address_monitors.errors);
    tracing::info!("Monitor Transactions: {} migrated, {} skipped, {} errors",
        small_stats.monitor_transactions.migrated, small_stats.monitor_transactions.skipped, small_stats.monitor_transactions.errors);

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
    tracing::info!("\nOverall: {}", verification.overall.summary);

    if verification.overall.all_valid {
        tracing::info!("\n✓ Migration completed successfully!");
        Ok(())
    } else {
        Err(anyhow!("Migration verification failed"))
    }
}
