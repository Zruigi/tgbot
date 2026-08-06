//! Small Tables Migration - subscription_plans, address_monitors, monitor_transactions
//! 列名经实测 SQLite snapshot 与 PG 目标 schema 双向核对(2026-07-26)
//! SQLite "plans" → PG "subscription_plans"

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;

use crate::migration_audit::MigrationFailure;
use crate::sqlite_reader::SqliteConfig;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SmallTablesMigrationStats {
    pub plans: TableStats,
    pub address_monitors: TableStats,
    pub monitor_transactions: TableStats,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TableStats {
    pub total_source: i64,
    pub migrated: i64,
    pub skipped: i64,
    pub errors: i64,
    pub failures: Vec<MigrationFailure>,
}

pub async fn migrate_small_tables(
    sqlite_config: &SqliteConfig,
    pg_pool: &PgPool,
) -> Result<SmallTablesMigrationStats> {
    tracing::info!("Starting small tables migration");

    let conn = rusqlite::Connection::open(&sqlite_config.db_path)
        .context("Failed to open SQLite database for small tables")?;

    let plans = migrate_plans(&conn, pg_pool).await?;
    let address_monitors = migrate_address_monitors(&conn, pg_pool).await?;
    let monitor_transactions = migrate_monitor_transactions(&conn, pg_pool).await?;

    let stats = SmallTablesMigrationStats {
        plans,
        address_monitors,
        monitor_transactions,
    };

    tracing::info!(
        "Small tables: plans({}/{} migrated), address_monitors({}/{}), monitor_transactions({}/{})",
        stats.plans.migrated,
        stats.plans.total_source,
        stats.address_monitors.migrated,
        stats.address_monitors.total_source,
        stats.monitor_transactions.migrated,
        stats.monitor_transactions.total_source,
    );

    Ok(stats)
}

// ---------------------------------------------------------------------------
// plans (SQLite) → subscription_plans (PG)
// SQLite 列: id plan_code plan_name price_usdt duration_days features_json
//           status created_at updated_at
// PG 列:   plan_code plan_name price_usdt duration_days features_json
//           status created_at updated_at  (PK plan_code)
// ---------------------------------------------------------------------------

async fn migrate_plans(conn: &rusqlite::Connection, pg_pool: &PgPool) -> Result<TableStats> {
    let mut stmt = conn.prepare(
        "SELECT plan_code, plan_name, price_usdt, duration_days, features_json, status, created_at, updated_at
         FROM plans ORDER BY id",
    )?;

    let rows: Vec<(
        String,
        String,
        Option<f64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let total_source = rows.len() as i64;
    let mut stats = TableStats {
        total_source,
        migrated: 0,
        skipped: 0,
        errors: 0,
        failures: Vec::new(),
    };

    for (
        plan_code,
        plan_name,
        price_usdt,
        duration_days,
        features_json,
        status,
        created_at,
        updated_at,
    ) in rows
    {
        let created = created_at.as_deref().and_then(|s| parse_timestamp(s).ok());
        let updated = updated_at.as_deref().and_then(|s| parse_timestamp(s).ok());

        let result = sqlx::query(
            r#"INSERT INTO subscription_plans (plan_code, plan_name, price_usdt, duration_days, features_json, status, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7, $8)
               ON CONFLICT (plan_code) DO UPDATE SET
                   plan_name = EXCLUDED.plan_name,
                   price_usdt = COALESCE(EXCLUDED.price_usdt, subscription_plans.price_usdt),
                   duration_days = COALESCE(EXCLUDED.duration_days, subscription_plans.duration_days),
                   features_json = COALESCE(EXCLUDED.features_json, subscription_plans.features_json),
                   status = COALESCE(EXCLUDED.status, subscription_plans.status),
                   updated_at = EXCLUDED.updated_at"#,
        )
        .bind(&plan_code)
        .bind(&plan_name)
        .bind(price_usdt)
        .bind(duration_days)
        .bind(&features_json)
        .bind(&status)
        .bind(created)
        .bind(updated)
        .execute(pg_pool)
        .await;

        match result {
            Ok(_) => stats.migrated += 1,
            Err(e) => {
                stats.errors += 1;
                tracing::warn!("plan {} failed: {}", plan_code, e);
                stats.failures.push(MigrationFailure::new(
                    "plans",
                    format!("plan_code={plan_code}"),
                    e.to_string(),
                ));
            }
        }
    }
    Ok(stats)
}

// ---------------------------------------------------------------------------
// address_monitors (SQLite → PG)
// SQLite 列: monitor_id user_id address network last_tx_timestamp is_active
//           remark risk_level risk_note created_at owner_type tenant_id
// PG 列:   monitor_id user_id tenant_id address network remark risk_level
//           risk_note is_active last_tx_timestamp inflow_20 outflow_20
//           profit_20 created_at updated_at  (PK monitor_id)
// ---------------------------------------------------------------------------

async fn migrate_address_monitors(
    conn: &rusqlite::Connection,
    pg_pool: &PgPool,
) -> Result<TableStats> {
    let mut stmt = conn.prepare(
        "SELECT monitor_id, user_id, address, network, last_tx_timestamp, is_active,
                remark, risk_level, risk_note, created_at, owner_type, tenant_id
         FROM address_monitors ORDER BY monitor_id",
    )?;

    let rows: Vec<(
        i64,
        Option<i64>,
        String,
        Option<String>,
        Option<i64>,
        Option<bool>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let total_source = rows.len() as i64;
    let mut stats = TableStats {
        total_source,
        migrated: 0,
        skipped: 0,
        errors: 0,
        failures: Vec::new(),
    };

    for (
        monitor_id,
        user_id,
        address,
        network,
        last_tx_ts,
        is_active,
        remark,
        risk_level,
        risk_note,
        created_at,
        _owner_type,
        tenant_id,
    ) in rows
    {
        let created = created_at.as_deref().and_then(|s| parse_timestamp(s).ok());
        let tenant = tenant_id.unwrap_or_else(|| "default".to_string());

        let result = sqlx::query(
            r#"INSERT INTO address_monitors (monitor_id, user_id, tenant_id, address, network,
                   remark, risk_level, risk_note, is_active, last_tx_timestamp, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, COALESCE($11, now()))
               ON CONFLICT (monitor_id) DO UPDATE SET
                   address = EXCLUDED.address,
                   network = COALESCE(EXCLUDED.network, address_monitors.network),
                   is_active = COALESCE(EXCLUDED.is_active, address_monitors.is_active),
                   last_tx_timestamp = COALESCE(EXCLUDED.last_tx_timestamp, address_monitors.last_tx_timestamp),
                   updated_at = now()"#,
        )
        .bind(monitor_id)
        .bind(user_id)
        .bind(&tenant)
        .bind(&address)
        .bind(&network)
        .bind(&remark)
        .bind(&risk_level)
        .bind(&risk_note)
        .bind(is_active)
        .bind(last_tx_ts)
        .bind(created)
        .execute(pg_pool)
        .await;

        match result {
            Ok(_) => stats.migrated += 1,
            Err(e) => {
                stats.errors += 1;
                tracing::warn!("address_monitor {} failed: {}", monitor_id, e);
                stats.failures.push(MigrationFailure::new(
                    "address_monitors",
                    format!("tenant_id={tenant},monitor_id={monitor_id}"),
                    e.to_string(),
                ));
            }
        }
    }
    Ok(stats)
}

// ---------------------------------------------------------------------------
// monitor_transactions (SQLite → PG)
// SQLite 列: tx_id monitor_id user_id address network direction amount
//           token_symbol tx_timestamp counterparty risk_level risk_note
//           created_at income_reason expense_reason payer payee tenant_id
// PG 列:   tx_id monitor_id user_id tenant_id address network direction
//           amount token_symbol tx_timestamp counterparty risk_level
//           risk_note income_reason expense_reason payer payee
//           is_energy_fee created_at updated_at  (PK tenant_id, tx_id)
// ---------------------------------------------------------------------------

async fn migrate_monitor_transactions(
    conn: &rusqlite::Connection,
    pg_pool: &PgPool,
) -> Result<TableStats> {
    let mut stmt = conn.prepare(
        "SELECT tx_id, monitor_id, user_id, address, network, direction, amount,
                token_symbol, tx_timestamp, counterparty, risk_level, risk_note,
                created_at, income_reason, expense_reason, payer, payee, tenant_id
         FROM monitor_transactions ORDER BY tx_id",
    )?;

    let rows: Vec<(
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
                r.get(14)?,
                r.get(15)?,
                r.get(16)?,
                r.get(17)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let total_source = rows.len() as i64;
    let mut stats = TableStats {
        total_source,
        migrated: 0,
        skipped: 0,
        errors: 0,
        failures: Vec::new(),
    };

    for (
        tx_id,
        monitor_id,
        user_id,
        address,
        network,
        direction,
        amount,
        token_symbol,
        tx_timestamp,
        counterparty,
        risk_level,
        risk_note,
        created_at,
        income_reason,
        expense_reason,
        payer,
        payee,
        tenant_id,
    ) in rows
    {
        let created = created_at.as_deref().and_then(|s| parse_timestamp(s).ok());
        let tenant = tenant_id.unwrap_or_else(|| "default".to_string());
        // tx_timestamp 是 unix 时间戳(秒), 转为 timestamptz
        // PG tx_timestamp 列是 bigint(unix 秒),直接绑定 i64
        let tx_ts = tx_timestamp;

        let result = sqlx::query(
            r#"INSERT INTO monitor_transactions (tx_id, monitor_id, user_id, tenant_id, address,
                   network, direction, amount, token_symbol, tx_timestamp, counterparty,
                   risk_level, risk_note, income_reason, expense_reason, payer, payee, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
               ON CONFLICT (tenant_id, tx_id) DO UPDATE SET
                   risk_level = COALESCE(EXCLUDED.risk_level, monitor_transactions.risk_level),
                   risk_note = COALESCE(EXCLUDED.risk_note, monitor_transactions.risk_note),
                   updated_at = now()"#,
        )
        .bind(&tx_id)
        .bind(monitor_id)
        .bind(user_id)
        .bind(&tenant)
        .bind(&address)
        .bind(&network)
        .bind(&direction)
        .bind(amount)
        .bind(&token_symbol)
        .bind(tx_ts)
        .bind(&counterparty)
        .bind(&risk_level)
        .bind(&risk_note)
        .bind(&income_reason)
        .bind(&expense_reason)
        .bind(&payer)
        .bind(&payee)
        .bind(created)
        .execute(pg_pool)
        .await;

        match result {
            Ok(_) => stats.migrated += 1,
            Err(e) => {
                stats.errors += 1;
                tracing::warn!("monitor_transaction {} failed: {}", tx_id, e);
                stats.failures.push(MigrationFailure::new(
                    "monitor_transactions",
                    format!("tenant_id={tenant},tx_id={tx_id}"),
                    e.to_string(),
                ));
            }
        }
    }
    Ok(stats)
}

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
    Err(anyhow::anyhow!("Unparseable timestamp: {}", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_accounting() {
        let s = TableStats {
            total_source: 10,
            migrated: 9,
            skipped: 1,
            errors: 0,
            failures: Vec::new(),
        };
        assert_eq!(s.total_source, s.migrated + s.skipped + s.errors);
    }

    #[test]
    fn parse_various_timestamps() {
        assert!(parse_timestamp("2026-01-15 10:30:00").is_ok());
        assert!(parse_timestamp("2026-01-15T10:30:00Z").is_ok());
        assert!(parse_timestamp("2026-01-15").is_ok());
        assert!(parse_timestamp("not-a-date").is_err());
    }
}
