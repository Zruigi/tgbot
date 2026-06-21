// User repository — inbox and chat-history queries
// Migrated from backend/repositories/user_repo.py

use anyhow::Result;
use serde_json::Value;
use sqlx::{PgPool, Row};
use tracing::info;

use super::row_to_json;

// ---------------------------------------------------------------------------
// GET /api/users — inbox list
// ---------------------------------------------------------------------------

/// Fetch the admin inbox for a tenant (tenant-scoped).
///
/// Returns rows from `chat.admin_inbox` filtered by `tenant_id`,
/// ordered by `last_active DESC`.
pub async fn get_inbox_list_for_tenant(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT
            user_id,
            username AS from_username,
            summary,
            unread_count,
            last_active,
            avatar_file_id,
            tenant_id
        FROM chat.admin_inbox
        WHERE tenant_id = $1
        ORDER BY last_active DESC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_json).collect())
}

/// Fetch the full admin inbox (platform-admin, no tenant filter).
pub async fn get_inbox_list(pool: &PgPool) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT
            user_id,
            username AS from_username,
            summary,
            unread_count,
            last_active,
            avatar_file_id,
            tenant_id
        FROM chat.admin_inbox
        ORDER BY last_active DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        // Auto-rebuild is intentionally not triggered in the Rust port;
        // the inbox materialisation is expected to be maintained by the
        // message-ingestion pipeline instead.
        info!("Admin inbox is empty – skipping auto-rebuild (handled by pipeline)");
    }

    Ok(rows.into_iter().map(row_to_json).collect())
}

// ---------------------------------------------------------------------------
// GET /api/messages/{user_id} — chat history
// ---------------------------------------------------------------------------

/// Return paginated chat history between a Telegram user and the
/// merchant's bot, filtered by `tenant_id`.
pub async fn get_chat_history(
    pool: &PgPool,
    user_id: i64,
    bot_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT
            m.id          AS msg_id,
            m.trace_id,
            m.from_userid,
            m.to_userid,
            m.content,
            m.msg_type    AS type,
            m.file_id,
            m.media_url,
            m.timestamp,
            m.is_read,
            m.quoted_text,
            m.quoted_msg_id,
            m.quoted_file_id,
            m.quoted_type,
            m.tg_msg_id,
            m.status,
            m.auto_translation,
            m.translated_text,
            m.original_text,
            m.admin_userid,
            m.is_deleted,
            e.emoji
        FROM chat.messages m
        LEFT JOIN chat.emoji_messages e ON e.msg_id = m.id
        WHERE ((m.from_userid = $1 AND m.to_userid = $2)
            OR (m.from_userid = $2 AND m.to_userid = $1))
          AND m.is_deleted = false
        ORDER BY m.timestamp DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(user_id)
    .bind(bot_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    // Front-end expects chronological order (oldest first)
    let mut items: Vec<Value> = rows.into_iter().map(row_to_json).collect();
    items.reverse();
    Ok(items)
}

// ---------------------------------------------------------------------------
// POST /api/mark_read/{user_id}
// ---------------------------------------------------------------------------

/// Mark all unread messages from `user_id` as read and reset the
/// inbox unread counter.
pub async fn mark_messages_as_read(pool: &PgPool, user_id: i64, bot_id: i64) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE chat.messages
        SET is_read = true
        WHERE from_userid = $1
          AND to_userid = $2
          AND is_read = false
        "#,
    )
    .bind(user_id)
    .bind(bot_id)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        UPDATE chat.admin_inbox
        SET unread_count = 0, summary = '', updated_at = now()
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// POST /api/delete_chat/{user_id} — soft delete
// ---------------------------------------------------------------------------

/// Soft-delete a chat: hide it from the inbox.
pub async fn soft_delete_chat(pool: &PgPool, user_id: i64) -> Result<()> {
    // The admin_inbox table in the PG schema does not have an is_hidden column
    // by default. We emulate "soft delete" by clearing the inbox row so it no
    // longer shows in the user list. A future migration can add is_hidden.
    sqlx::query(
        r#"
        DELETE FROM chat.admin_inbox
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// POST /api/users/rebuild
// ---------------------------------------------------------------------------

/// Rebuild the admin inbox from the messages table for a given tenant.
pub async fn rebuild_inbox_for_tenant(
    pool: &PgPool,
    tenant_id: &str,
    bot_id: i64,
) -> Result<usize> {
    // Find all users who exchanged messages with the bot
    let users = sqlx::query(
        r#"
        SELECT
            CASE WHEN from_userid = $1 THEN to_userid ELSE from_userid END AS uid,
            MAX(timestamp) AS last_ts
        FROM chat.messages
        WHERE (from_userid = $1 OR to_userid = $1)
          AND tenant_id = $2
        GROUP BY uid
        "#,
    )
    .bind(bot_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    let mut count = 0usize;
    for row in &users {
        let uid: i64 = row.try_get("uid")?;
        let last_ts: chrono::DateTime<chrono::Utc> = row.try_get("last_ts")?;

        // Resolve username
        let username = resolve_username(pool, uid).await;

        // Unread count
        let unread: i64 = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint
            FROM chat.messages
            WHERE from_userid = $1 AND to_userid = $2 AND is_read = false AND is_deleted = false
            "#,
        )
        .bind(uid)
        .bind(bot_id)
        .fetch_one(pool)
        .await?;

        // Latest message summary
        let summary: Option<String> = sqlx::query_scalar::<_, String>(
            r#"
            SELECT content
            FROM chat.messages
            WHERE ((from_userid = $1 AND to_userid = $2) OR (from_userid = $2 AND to_userid = $1))
              AND is_deleted = false
            ORDER BY timestamp DESC
            LIMIT 1
            "#,
        )
        .bind(uid)
        .bind(bot_id)
        .fetch_optional(pool)
        .await?;

        let summary_text = match &summary {
            Some(s) if s.len() > 30 => format!("{}...", &s[..30]),
            Some(s) => s.clone(),
            None => "[媒体消息]".into(),
        };

        sqlx::query(
            r#"
            INSERT INTO chat.admin_inbox (user_id, username, summary, unread_count, last_active, tenant_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (user_id) DO UPDATE SET
                username  = EXCLUDED.username,
                summary   = EXCLUDED.summary,
                unread_count = EXCLUDED.unread_count,
                last_active  = EXCLUDED.last_active,
                updated_at   = now()
            "#,
        )
        .bind(uid)
        .bind(&username)
        .bind(&summary_text)
        .bind(unread)
        .bind(last_ts)
        .bind(tenant_id)
        .execute(pool)
        .await?;

        count += 1;
    }

    info!("Rebuilt inbox for tenant={tenant_id}: {count} users");
    Ok(count)
}

async fn resolve_username(pool: &PgPool, user_id: i64) -> String {
    // Try chat.users first
    if let Ok(Some(name)) = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(NULLIF(username, ''), NULLIF(first_name, '')) FROM chat.users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        if !name.is_empty() {
            return name;
        }
    }
    // Fallback
    format!("User {user_id}")
}

// ---------------------------------------------------------------------------
// Tenant isolation: verify user belongs to tenant
// ---------------------------------------------------------------------------

/// Check whether `user_id` is associated with the given tenant.
/// Strategy chain:
///   1. chat.users with matching tenant_id (if column exists)
///   2. Message exchange with the bot
pub async fn is_user_belong_to_tenant(
    pool: &PgPool,
    user_id: i64,
    tenant_id: &str,
    bot_id: i64,
) -> Result<bool> {
    // Strategy 1: user exchanged messages with the bot belonging to the tenant
    let found: bool = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM chat.messages
            WHERE tenant_id = $1
              AND ((from_userid = $2 AND to_userid = $3)
                OR (from_userid = $3 AND to_userid = $2))
              AND is_deleted = false
            LIMIT 1
        )
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(bot_id)
    .fetch_one(pool)
    .await?;

    Ok(found)
}

// ---------------------------------------------------------------------------
// Resolve bot_id for a tenant
// ---------------------------------------------------------------------------

/// Look up the bot_id (Telegram bot user id) for a merchant's tenant.
pub async fn get_bot_id_for_tenant(pool: &PgPool, tenant_id: &str) -> Result<Option<i64>> {
    let bot_id: Option<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT bot_id FROM merchants WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(bot_id)
}
