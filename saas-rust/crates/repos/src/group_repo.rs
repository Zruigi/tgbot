// Group repository — tracked groups CRUD
// Migrated from backend/repositories/group_repo.py

use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;

use super::row_to_json;

// ---------------------------------------------------------------------------
// List groups for a tenant (with optional filters)
// ---------------------------------------------------------------------------

/// Fetch tracked groups for a specific tenant.
/// If `is_monitored` is None, returns all groups. Otherwise filters by that value.
pub async fn list_groups(
    pool: &PgPool,
    tenant_id: &str,
    is_monitored: Option<bool>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Value>> {
    let rows = if let Some(monitored) = is_monitored {
        sqlx::query(
            r#"
            SELECT * FROM groups.tracked_groups
            WHERE tenant_id = $1 AND is_monitored = $2
            ORDER BY updated_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(tenant_id)
        .bind(monitored)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT * FROM groups.tracked_groups
            WHERE tenant_id = $1
            ORDER BY updated_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    };
    Ok(rows.into_iter().map(row_to_json).collect())
}

// ---------------------------------------------------------------------------
// Get a single group
// ---------------------------------------------------------------------------

/// Fetch a single tracked group by group_id.
pub async fn get_group(pool: &PgPool, group_id: i64, tenant_id: &str) -> Result<Option<Value>> {
    let row = sqlx::query(
        "SELECT * FROM groups.tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

// ---------------------------------------------------------------------------
// Create or update group config
// ---------------------------------------------------------------------------

/// Upsert group configuration (verify settings, keywords, etc.).
pub async fn upsert_group_config(
    pool: &PgPool,
    group_id: i64,
    group_title: &str,
    tenant_id: &str,
    is_monitored: Option<bool>,
    custom_keywords: Option<&str>,
    verify_enabled: Option<bool>,
    verify_type: Option<&str>,
    verify_timeout: Option<i32>,
) -> Result<Value> {
    let row = sqlx::query(
        r#"
        INSERT INTO groups.tracked_groups (
            group_id, group_title, tenant_id, is_monitored,
            custom_keywords, verify_enabled, verify_type, verify_timeout,
            first_seen, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now())
        ON CONFLICT (group_id) DO UPDATE SET
            group_title       = EXCLUDED.group_title,
            is_monitored      = COALESCE($4, groups.tracked_groups.is_monitored),
            custom_keywords   = COALESCE($5, groups.tracked_groups.custom_keywords),
            verify_enabled    = COALESCE($6, groups.tracked_groups.verify_enabled),
            verify_type       = COALESCE($7, groups.tracked_groups.verify_type),
            verify_timeout    = COALESCE($8, groups.tracked_groups.verify_timeout),
            updated_at        = now()
        RETURNING *
        "#,
    )
    .bind(group_id)
    .bind(group_title)
    .bind(tenant_id)
    .bind(is_monitored)
    .bind(custom_keywords)
    .bind(verify_enabled)
    .bind(verify_type)
    .bind(verify_timeout)
    .fetch_one(pool)
    .await?;
    Ok(row_to_json(row))
}

// ---------------------------------------------------------------------------
// Update group monitoring status
// ---------------------------------------------------------------------------

/// Enable or disable monitoring for a group.
pub async fn set_monitored(
    pool: &PgPool,
    group_id: i64,
    tenant_id: &str,
    is_monitored: bool,
) -> Result<Option<Value>> {
    let row = sqlx::query(
        r#"
        UPDATE groups.tracked_groups
        SET is_monitored = $3, updated_at = now()
        WHERE group_id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(group_id)
    .bind(tenant_id)
    .bind(is_monitored)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

// ---------------------------------------------------------------------------
// Delete a group (untrack)
// ---------------------------------------------------------------------------

/// Remove a group from tracking.
pub async fn delete_group(pool: &PgPool, group_id: i64, tenant_id: &str) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM groups.tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Group count for a tenant
// ---------------------------------------------------------------------------

/// Count total tracked groups for a tenant.
pub async fn count_groups(pool: &PgPool, tenant_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.tracked_groups WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?
    .unwrap_or(0);
    Ok(count)
}