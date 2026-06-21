//! Broadcast target repository – CRUD for broadcast_targets table.
use anyhow::Result;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::row_to_json;

/// Upsert a broadcast target (insert or update on conflict by chat_id).
pub async fn upsert_broadcast_target(
    pool: &PgPool,
    chat_id: i64,
    chat_title: &str,
    chat_type: &str,
    chat_username: Option<&str>,
    can_broadcast: bool,
    tenant_id: &str,
    member_count: i32,
) -> Result<Value> {
    let row = sqlx::query(
        r#"
        INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, tenant_id, member_count, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        ON CONFLICT (chat_id) DO UPDATE SET
            chat_title    = EXCLUDED.chat_title,
            chat_type     = EXCLUDED.chat_type,
            chat_username = EXCLUDED.chat_username,
            can_broadcast = EXCLUDED.can_broadcast,
            tenant_id     = EXCLUDED.tenant_id,
            member_count  = EXCLUDED.member_count,
            updated_at    = now()
        RETURNING *
        "#,
    )
    .bind(chat_id)
    .bind(chat_title)
    .bind(chat_type)
    .bind(chat_username)
    .bind(can_broadcast)
    .bind(tenant_id)
    .bind(member_count)
    .fetch_one(pool)
    .await?;
    Ok(row_to_json(row))
}

/// List broadcast targets with optional filters.
pub async fn list_broadcast_targets(
    pool: &PgPool,
    tenant_id: &str,
    chat_types: Option<&[String]>,
    enabled_only: bool,
) -> Result<Vec<Value>> {
    let rows = if let Some(types) = chat_types {
        if enabled_only {
            sqlx::query(
                r#"
                SELECT * FROM broadcast_targets
                WHERE tenant_id = $1 AND can_broadcast = true AND chat_type = ANY($2)
                ORDER BY updated_at DESC
                "#,
            )
            .bind(tenant_id)
            .bind(types)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT * FROM broadcast_targets
                WHERE tenant_id = $1 AND chat_type = ANY($2)
                ORDER BY updated_at DESC
                "#,
            )
            .bind(tenant_id)
            .bind(types)
            .fetch_all(pool)
            .await?
        }
    } else if enabled_only {
        sqlx::query(
            r#"
            SELECT * FROM broadcast_targets
            WHERE tenant_id = $1 AND can_broadcast = true
            ORDER BY updated_at DESC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT * FROM broadcast_targets
            WHERE tenant_id = $1
            ORDER BY updated_at DESC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows.into_iter().map(row_to_json).collect())
}

/// Update whether a target can participate in broadcasts.
pub async fn set_broadcast_target_permission(
    pool: &PgPool,
    chat_id: i64,
    can_broadcast: bool,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE broadcast_targets SET can_broadcast = $1, updated_at = now() WHERE chat_id = $2",
    )
    .bind(can_broadcast)
    .bind(chat_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Delete a broadcast target.
pub async fn delete_broadcast_target(pool: &PgPool, chat_id: i64) -> Result<bool> {
    let result = sqlx::query("DELETE FROM broadcast_targets WHERE chat_id = $1")
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
