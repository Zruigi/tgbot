use sqlx::Row;
//! Message repository — message CRUD and search
//! Migrated from backend/repositories/message_repo.py

use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::row_to_json;

// ---------------------------------------------------------------------------
// Save a message
// ---------------------------------------------------------------------------

/// Insert a new message into `chat.messages` and update the admin inbox
/// summary if the message is directed to the bot (inbox notification).
pub async fn save_message(
    pool: &PgPool,
    from_userid: i64,
    to_userid: i64,
    content: Option<&str>,
    msg_type: &str,
    file_id: Option<&str>,
    media_url: Option<&str>,
    is_read: bool,
    tg_msg_id: Option<i64>,
    tenant_id: &str,
    reply_to_msg_id: Option<i64>,
) -> Result<i64> {
    let trace_id = Uuid::new_v4().to_string();

    // Resolve quoted fields when reply_to_msg_id is provided
    let (quoted_text, quoted_file_id, quoted_type): (Option<String>, Option<String>, Option<String>) =
        if let Some(qid) = reply_to_msg_id {
            let row = sqlx::query(
                "SELECT content, file_id, msg_type FROM chat.messages WHERE id = $1",
            )
            .bind(qid)
            .fetch_optional(pool)
            .await?;
            match row {
                Some(r) => (
                    r.try_get("content").ok().flatten(),
                    r.try_get("file_id").ok().flatten(),
                    r.try_get("msg_type").ok().flatten(),
                ),
                None => (None, None, None),
            }
        } else {
            (None, None, None)
        };

    let row = sqlx::query(
        r#"
        INSERT INTO chat.messages (
            trace_id, from_userid, to_userid, content, msg_type,
            file_id, media_url, is_read, tg_msg_id,
            quoted_text, quoted_msg_id, quoted_file_id, quoted_type,
            status, tenant_id
        ) VALUES (
            $1, $2, $3, $4, $5,
            $6, $7, $8, $9,
            $10, $11, $12, $13,
            'sent', $14
        )
        RETURNING id
        "#,
    )
    .bind(&trace_id)
    .bind(from_userid)
    .bind(to_userid)
    .bind(content)
    .bind(msg_type)
    .bind(file_id)
    .bind(media_url)
    .bind(is_read)
    .bind(tg_msg_id)
    .bind(&quoted_text)
    .bind(reply_to_msg_id)
    .bind(&quoted_file_id)
    .bind(quoted_type.unwrap_or_else(|| "text".into()))
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    let msg_id: i64 = row.try_get("id")?;

    // Update admin inbox if the message is from a user to the bot
    if !is_read {
        update_inbox_on_new_message(pool, from_userid, content, msg_type, tenant_id).await?;
    }

    Ok(msg_id)
}

/// Update the admin inbox summary when a new unread message arrives.
async fn update_inbox_on_new_message(
    pool: &PgPool,
    from_userid: i64,
    content: Option<&str>,
    msg_type: &str,
    tenant_id: &str,
) -> Result<()> {
    let display = match content {
        Some(c) if !c.is_empty() => {
            if c.len() > 20 {
                format!("{}...", &c[..20])
            } else {
                c.to_string()
            }
        }
        _ => format!("[{msg_type}]"),
    };

    let now = chrono::Utc::now();
    let time_str = now.format("%H:%M").to_string();
    let summary_line = format!("{time_str}: {display}");
    let summary = if summary_line.len() > 500 {
        format!("{}...", &summary_line[..497])
    } else {
        summary_line
    };

    // Count all unread from this user
    let unread: i64 = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM chat.messages
        WHERE from_userid = $1 AND is_read = false AND is_deleted = false
        "#,
    )
    .bind(from_userid)
    .fetch_one(pool)
    .await?
    .unwrap_or(0);

    sqlx::query(
        r#"
        INSERT INTO chat.admin_inbox (user_id, username, summary, unread_count, last_active, tenant_id)
        VALUES ($1, 'User ' || $1::text, $2, $3, now(), $4)
        ON CONFLICT (user_id) DO UPDATE SET
            summary      = EXCLUDED.summary,
            unread_count = EXCLUDED.unread_count,
            last_active  = now(),
            updated_at   = now()
        "#,
    )
    .bind(from_userid)
    .bind(&summary)
    .bind(unread)
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Delete single message
// ---------------------------------------------------------------------------

/// Soft-delete a single message by id (must belong to the given user pair).
pub async fn delete_single_message(pool: &PgPool, user_id: i64, msg_id: i64) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE chat.messages
        SET is_deleted = true
        WHERE id = $1
          AND (from_userid = $2 OR to_userid = $2)
        "#,
    )
    .bind(msg_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Search messages
// ---------------------------------------------------------------------------

/// Search private-chat and group messages, with tenant isolation.
pub async fn search_messages(
    pool: &PgPool,
    keyword: &str,
    group_id: Option<i64>,
    user_id: Option<i64>,
    limit: i64,
    tenant_id: Option<&str>,
    bot_id: Option<i64>,
) -> Result<Vec<Value>> {
    let pattern = format!("%{keyword}%");

    // If searching within a specific group
    if let Some(gid) = group_id {
        return search_in_group(pool, gid, &pattern, tenant_id, limit).await;
    }

    // If searching within a specific user's conversation
    if let Some(uid) = user_id {
        return search_in_user_messages(pool, uid, &pattern, tenant_id, bot_id, limit)
            .await;
    }

    // Global search: private + group
    search_all(pool, &pattern, tenant_id, bot_id, limit).await
}

async fn search_in_group(
    pool: &PgPool,
    group_id: i64,
    pattern: &str,
    tenant_id: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>> {
    let rows = if let Some(tid) = tenant_id {
        sqlx::query(
            r#"
            SELECT gm.*, u.username, u.first_name
            FROM groups.group_messages gm
            LEFT JOIN groups.group_members u ON gm.user_id = u.user_id AND gm.group_id = u.group_id
            WHERE gm.group_id = $1
              AND gm.content LIKE $2
              AND gm.group_id IN (SELECT group_id FROM groups.tracked_groups WHERE tenant_id = $3)
            ORDER BY gm.timestamp DESC
            LIMIT $4
            "#,
        )
        .bind(group_id)
        .bind(pattern)
        .bind(tid)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT gm.*, u.username, u.first_name
            FROM groups.group_messages gm
            LEFT JOIN groups.group_members u ON gm.user_id = u.user_id AND gm.group_id = u.group_id
            WHERE gm.group_id = $1
              AND gm.content LIKE $2
            ORDER BY gm.timestamp DESC
            LIMIT $3
            "#,
        )
        .bind(group_id)
        .bind(pattern)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    Ok(rows.into_iter().map(row_to_json).collect())
}

async fn search_in_user_messages(
    pool: &PgPool,
    user_id: i64,
    pattern: &str,
    tenant_id: Option<&str>,
    bot_id: Option<i64>,
    limit: i64,
) -> Result<Vec<Value>> {
    let rows = if let (Some(_tid), Some(bid)) = (tenant_id, bot_id) {
        sqlx::query(
            r#"
            SELECT m.*, 'private' AS source_type
            FROM chat.messages m
            WHERE (m.from_userid = $1 OR m.to_userid = $1)
              AND m.content LIKE $2
              AND m.is_deleted = false
              AND ((m.from_userid = $3 OR m.to_userid = $3))
            ORDER BY m.timestamp DESC
            LIMIT $4
            "#,
        )
        .bind(user_id)
        .bind(pattern)
        .bind(bid)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT m.*, 'private' AS source_type
            FROM chat.messages m
            WHERE (m.from_userid = $1 OR m.to_userid = $1)
              AND m.content LIKE $2
              AND m.is_deleted = false
            ORDER BY m.timestamp DESC
            LIMIT $3
            "#,
        )
        .bind(user_id)
        .bind(pattern)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    Ok(rows.into_iter().map(row_to_json).collect())
}

async fn search_all(
    pool: &PgPool,
    pattern: &str,
    tenant_id: Option<&str>,
    bot_id: Option<i64>,
    limit: i64,
) -> Result<Vec<Value>> {
    let mut results = Vec::new();

    // Private messages
    let private_rows = if let (Some(_tid), Some(bid)) = (tenant_id, bot_id) {
        sqlx::query(
            r#"
            SELECT m.*, 'private' AS source_type
            FROM chat.messages m
            WHERE m.content LIKE $1
              AND m.is_deleted = false
              AND (m.from_userid = $2 OR m.to_userid = $2)
            ORDER BY m.timestamp DESC
            LIMIT $3
            "#,
        )
        .bind(pattern)
        .bind(bid)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT m.*, 'private' AS source_type
            FROM chat.messages m
            WHERE m.content LIKE $1
              AND m.is_deleted = false
            ORDER BY m.timestamp DESC
            LIMIT $2
            "#,
        )
        .bind(pattern)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    for row in private_rows {
        let mut v = row_to_json(row);
        if let Some(obj) = v.as_object_mut() {
            obj.insert("user_id".into(), obj.get("from_userid").cloned().unwrap_or(Value::Null));
        }
        results.push(v);
    }

    // Group messages
    let group_rows = if let Some(tid) = tenant_id {
        sqlx::query(
            r#"
            SELECT gm.*, u.username, u.first_name, 'group' AS source_type
            FROM groups.group_messages gm
            LEFT JOIN groups.group_members u ON gm.user_id = u.user_id AND gm.group_id = u.group_id
            WHERE gm.content LIKE $1
              AND gm.group_id IN (SELECT group_id FROM groups.tracked_groups WHERE tenant_id = $2)
            ORDER BY gm.timestamp DESC
            LIMIT $3
            "#,
        )
        .bind(pattern)
        .bind(tid)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT gm.*, u.username, u.first_name, 'group' AS source_type
            FROM groups.group_messages gm
            LEFT JOIN groups.group_members u ON gm.user_id = u.user_id AND gm.group_id = u.group_id
            WHERE gm.content LIKE $1
            ORDER BY gm.timestamp DESC
            LIMIT $2
            "#,
        )
        .bind(pattern)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    for row in group_rows {
        results.push(row_to_json(row));
    }

    // Sort by timestamp desc, then truncate
    results.sort_by(|a, b| {
        let ta = a.get("timestamp").and_then(Value::as_str).unwrap_or("");
        let tb = b.get("timestamp").and_then(Value::as_str).unwrap_or("");
        tb.cmp(ta)
    });
    results.truncate(limit as usize);

    Ok(results)
}
