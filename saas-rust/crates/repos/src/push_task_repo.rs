use sqlx::Row;
//! Push task repository – CRUD for push_tasks table.
use anyhow::Result;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::row_to_json;

/// Create a new push task.
pub async fn create_push_task(
    pool: &PgPool,
    tenant_id: &str,
    merchant_id: Option<Uuid>,
    task_name: &str,
    content: &str,
    image_file_id: Option<&str>,
    do_translate: bool,
    target_chat_ids: &[i64],
    start_at: &str,
    interval_hours: i32,
    status: &str,
    next_run_at: &str,
) -> Result<i64> {
    let row = sqlx::query(
        r#"
        INSERT INTO push_tasks (
            tenant_id, merchant_id, task_name, content, image_file_id, do_translate,
            target_chat_ids, start_at, interval_hours, status, next_run_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8::timestamptz, $9, $10, $11::timestamptz)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(merchant_id)
    .bind(task_name)
    .bind(content)
    .bind(image_file_id)
    .bind(do_translate)
    .bind(serde_json::to_value(target_chat_ids)?)
    .bind(start_at)
    .bind(interval_hours)
    .bind(status)
    .bind(next_run_at)
    .fetch_one(pool)
    .await?;
    let id: i64 = row.try_get("id")?;
    Ok(id)
}

/// List push tasks, optionally filtered by tenant and status.
pub async fn list_push_tasks(
    pool: &PgPool,
    tenant_id: Option<&str>,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Value>> {
    let rows = match (tenant_id, status) {
        (Some(tid), Some(st)) => {
            sqlx::query(
                r#"
                SELECT * FROM push_tasks
                WHERE tenant_id = $1 AND status = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(tid)
            .bind(st)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(tid), None) => {
            sqlx::query(
                r#"
                SELECT * FROM push_tasks
                WHERE tenant_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(tid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, Some(st)) => {
            sqlx::query(
                r#"
                SELECT * FROM push_tasks
                WHERE status = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(st)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            sqlx::query(
                r#"
                SELECT * FROM push_tasks
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows.into_iter().map(row_to_json).collect())
}

/// Update the status of a push task (paused / running).
pub async fn update_push_task_status(
    pool: &PgPool,
    task_id: i64,
    status: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE push_tasks SET status = $1, updated_at = now() WHERE id = $2",
    )
    .bind(status)
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Get a single push task by id.
pub async fn get_push_task(pool: &PgPool, task_id: i64) -> Result<Option<Value>> {
    let row = sqlx::query("SELECT * FROM push_tasks WHERE id = $1")
        .bind(task_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(row_to_json))
}

/// Record the result of a push task run and update next_run_at.
pub async fn update_push_task_run_result(
    pool: &PgPool,
    task_id: i64,
    last_run_at: &str,
    next_run_at: &str,
    summary: &Value,
    retry_count: i32,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE push_tasks
        SET last_run_at     = $1::timestamptz,
            next_run_at     = $2::timestamptz,
            last_run_summary = $3,
            retry_count     = $4,
            updated_at      = now()
        WHERE id = $5
        "#,
    )
    .bind(last_run_at)
    .bind(next_run_at)
    .bind(summary)
    .bind(retry_count)
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Delete a push task.
pub async fn delete_push_task(pool: &PgPool, task_id: i64) -> Result<bool> {
    let result = sqlx::query("DELETE FROM push_tasks WHERE id = $1")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
