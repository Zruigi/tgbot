// Member repository — group members CRUD
// Migrated from backend/repositories/member_repo.py

use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;

use super::row_to_json;

// ---------------------------------------------------------------------------
// List members for a group
// ---------------------------------------------------------------------------

/// Fetch group members with optional filters.
pub async fn list_members(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Option<i64>,
    status: Option<&str>,
    is_verified: Option<bool>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT * FROM groups.group_members
        WHERE tenant_id = $1
          AND ($2::bigint IS NULL OR group_id = $2)
          AND ($3::text IS NULL OR status = $3)
          AND ($4::boolean IS NULL OR is_verified = $4)
        ORDER BY joined_at DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(tenant_id)
    .bind(group_id)
    .bind(status)
    .bind(is_verified)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_json).collect())
}

// ---------------------------------------------------------------------------
// Get a single member
// ---------------------------------------------------------------------------

/// Fetch a single member by group_id and user_id.
pub async fn get_member(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
    tenant_id: &str,
) -> Result<Option<Value>> {
    let row = sqlx::query(
        "SELECT * FROM groups.group_members WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

// ---------------------------------------------------------------------------
// Create or update member
// ---------------------------------------------------------------------------

/// Upsert a group member record.
pub async fn upsert_member(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
    tenant_id: &str,
    username: Option<&str>,
    first_name: Option<&str>,
    last_name: Option<&str>,
    group_title: Option<&str>,
) -> Result<Value> {
    let row = sqlx::query(
        r#"
        INSERT INTO groups.group_members (
            group_id, user_id, tenant_id, username, first_name, last_name,
            group_title, status, joined_at, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', now(), now(), now())
        ON CONFLICT (group_id, user_id) DO UPDATE SET
            username    = COALESCE($4, groups.group_members.username),
            first_name  = COALESCE($5, groups.group_members.first_name),
            last_name   = COALESCE($6, groups.group_members.last_name),
            group_title = COALESCE($7, groups.group_members.group_title),
            updated_at  = now()
        RETURNING *
        "#,
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(username)
    .bind(first_name)
    .bind(last_name)
    .bind(group_title)
    .fetch_one(pool)
    .await?;
    Ok(row_to_json(row))
}

// ---------------------------------------------------------------------------
// Update member status (kick, ban, mute, unmute, verify, unverify)
// ---------------------------------------------------------------------------

/// Update member status.
pub async fn update_member_status(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
    tenant_id: &str,
    status: &str,
) -> Result<Option<Value>> {
    let row = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET status = $4, updated_at = now()
        WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(status)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

// ---------------------------------------------------------------------------
// Verify / unverify member
// ---------------------------------------------------------------------------

/// Mark a member as verified.
pub async fn verify_member(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
    tenant_id: &str,
) -> Result<Option<Value>> {
    let row = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET is_verified = true, verified_at = now(), status = 'active', updated_at = now()
        WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

/// Mark a member as unverified.
pub async fn unverify_member(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
    tenant_id: &str,
) -> Result<Option<Value>> {
    let row = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET is_verified = false, verified_at = NULL, updated_at = now()
        WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Batch update member status.
pub async fn batch_update_status(
    pool: &PgPool,
    group_id: i64,
    tenant_id: &str,
    user_ids: &[i64],
    status: &str,
) -> Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET status = $4, updated_at = now()
        WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)
        "#,
    )
    .bind(group_id)
    .bind(tenant_id)
    .bind(user_ids)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Batch verify members.
pub async fn batch_verify(
    pool: &PgPool,
    group_id: i64,
    tenant_id: &str,
    user_ids: &[i64],
) -> Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE groups.group_members
        SET is_verified = true, verified_at = now(), status = 'active', updated_at = now()
        WHERE group_id = $1 AND tenant_id = $2 AND user_id = ANY($3)
        "#,
    )
    .bind(group_id)
    .bind(tenant_id)
    .bind(user_ids)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------
// Member counts
// ---------------------------------------------------------------------------

/// Count members in a group.
pub async fn count_members(pool: &PgPool, group_id: i64, tenant_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Count verified members in a group.
pub async fn count_verified_members(pool: &PgPool, group_id: i64, tenant_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND is_verified = true",
    )
    .bind(group_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Get members by user_id across all groups
// ---------------------------------------------------------------------------

/// Fetch all member records for a given user_id (across groups).
pub async fn get_user_members(
    pool: &PgPool,
    user_id: i64,
    tenant_id: &str,
) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        "SELECT * FROM groups.group_members WHERE user_id = $1 AND tenant_id = $2 ORDER BY joined_at DESC",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_json).collect())
}

// ---------------------------------------------------------------------------
// Delete member
// ---------------------------------------------------------------------------

/// Delete a member record.
pub async fn delete_member(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
    tenant_id: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM groups.group_members WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}