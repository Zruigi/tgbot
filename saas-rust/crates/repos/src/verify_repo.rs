// Verify repository — verification sessions and stats
// Migrated from backend/repositories/verify_repo.py

use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;

use super::row_to_json;

// ---------------------------------------------------------------------------
// Verify sessions
// ---------------------------------------------------------------------------

/// Create a new verification session.
pub async fn create_session(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
    verify_type: &str,
    username: Option<&str>,
    first_name: Option<&str>,
) -> Result<Value> {
    let row = sqlx::query(
        r#"
        INSERT INTO groups.verify_sessions (
            group_id, user_id, username, first_name, verify_type,
            status, created_at, expires_at
        ) VALUES ($1, $2, $3, $4, $5, 'pending', now(), now() + interval '5 minutes')
        RETURNING *
        "#,
    )
    .bind(group_id)
    .bind(user_id)
    .bind(username)
    .bind(first_name)
    .bind(verify_type)
    .fetch_one(pool)
    .await?;
    Ok(row_to_json(row))
}

/// Get a pending session for a user in a group.
pub async fn get_pending_session(
    pool: &PgPool,
    group_id: i64,
    user_id: i64,
) -> Result<Option<Value>> {
    let row = sqlx::query(
        r#"
        SELECT * FROM groups.verify_sessions
        WHERE group_id = $1 AND user_id = $2 AND status = 'pending'
          AND expires_at > now()
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

/// Update session answer and status.
pub async fn answer_session(
    pool: &PgPool,
    session_id: i64,
    answer: &str,
    correct: bool,
) -> Result<Option<Value>> {
    let status = if correct { "passed" } else { "failed" };
    let row = sqlx::query(
        r#"
        UPDATE groups.verify_sessions
        SET answer = $2, status = $3, bot_msg_id = NULL
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(session_id)
    .bind(answer)
    .bind(status)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_json))
}

/// Expire pending sessions that have timed out.
pub async fn expire_sessions(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE groups.verify_sessions
        SET status = 'expired'
        WHERE status = 'pending' AND expires_at <= now()
        "#,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------
// Verify stats
// ---------------------------------------------------------------------------

/// Get verification statistics for a group or tenant.
pub async fn get_stats(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Option<i64>,
) -> Result<Value> {
    let total_verified: i64;
    let total_unverified: i64;
    let pending_sessions: i64;

    if let Some(gid) = group_id {
        total_verified = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND is_verified = true",
        )
        .bind(gid)
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;

        total_unverified = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND is_verified = false AND status = 'active'",
        )
        .bind(gid)
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;

        pending_sessions = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.verify_sessions WHERE group_id = $1 AND status = 'pending'",
        )
        .bind(gid)
        .fetch_one(pool)
        .await?;
    } else {
        total_verified = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE tenant_id = $1 AND is_verified = true",
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;

        total_unverified = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE tenant_id = $1 AND is_verified = false AND status = 'active'",
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;

        pending_sessions = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint FROM groups.verify_sessions vs
            WHERE vs.status = 'pending'
              AND vs.group_id IN (SELECT group_id FROM tracked_groups WHERE tenant_id = $1)
            "#,
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;
    }

    let verify_rate = if total_verified + total_unverified > 0 {
        (total_verified as f64 / (total_verified + total_unverified) as f64 * 100.0).round() / 100.0
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "total_verified": total_verified,
        "total_unverified": total_unverified,
        "pending_sessions": pending_sessions,
        "verify_rate": verify_rate,
    }))
}

// ---------------------------------------------------------------------------
// Verify growth (daily counts)
// ---------------------------------------------------------------------------

/// Get daily verification counts for the past N days.
pub async fn get_growth(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Option<i64>,
    days: i64,
) -> Result<Vec<Value>> {
    let rows = if let Some(gid) = group_id {
        sqlx::query(
            r#"
            SELECT
                DATE(verified_at) AS date,
                COUNT(*)::bigint AS verified_count
            FROM groups.group_members
            WHERE group_id = $1
              AND tenant_id = $2
              AND is_verified = true
              AND verified_at >= now() - ($3::bigint || ' days')::interval
            GROUP BY DATE(verified_at)
            ORDER BY DATE(verified_at) ASC
            "#,
        )
        .bind(gid)
        .bind(tenant_id)
        .bind(days)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT
                DATE(verified_at) AS date,
                COUNT(*)::bigint AS verified_count
            FROM groups.group_members
            WHERE tenant_id = $1
              AND is_verified = true
              AND verified_at >= now() - ($2::bigint || ' days')::interval
            GROUP BY DATE(verified_at)
            ORDER BY DATE(verified_at) ASC
            "#,
        )
        .bind(tenant_id)
        .bind(days)
        .fetch_all(pool)
        .await?
    };

    Ok(rows.into_iter().map(row_to_json).collect())
}

// ---------------------------------------------------------------------------
// Recent verifications
// ---------------------------------------------------------------------------

/// Get recently verified members.
pub async fn get_recent_verifications(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Option<i64>,
    days: i64,
    limit: i64,
) -> Result<Vec<Value>> {
    let rows = if let Some(gid) = group_id {
        sqlx::query(
            r#"
            SELECT * FROM groups.group_members
            WHERE group_id = $1
              AND tenant_id = $2
              AND is_verified = true
              AND verified_at >= now() - ($3::bigint || ' days')::interval
            ORDER BY verified_at DESC
            LIMIT $4
            "#,
        )
        .bind(gid)
        .bind(tenant_id)
        .bind(days)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT * FROM groups.group_members
            WHERE tenant_id = $1
              AND is_verified = true
              AND verified_at >= now() - ($2::bigint || ' days')::interval
            ORDER BY verified_at DESC
            LIMIT $3
            "#,
        )
        .bind(tenant_id)
        .bind(days)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    Ok(rows.into_iter().map(row_to_json).collect())
}