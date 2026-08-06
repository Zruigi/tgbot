// Audit repository — tenant audit logs
// Migrated from backend/repositories/audit_repo.py

use anyhow::Result;
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

/// Log a tenant-isolation violation event (non-blocking).
pub async fn log_tenant_isolation_event(
    pool: &PgPool,
    tenant_id: &str,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    operator_role: &str,
    request_path: &str,
) -> Result<()> {
    let payload = serde_json::json!({
        "request_path": request_path,
    });

    // Don't fail the request if audit logging fails
    if let Err(e) = do_log_event(
        pool,
        tenant_id,
        action,
        operator_role,
        None,
        Some(resource_type),
        Some(resource_id),
        Some(payload),
    )
    .await
    {
        warn!(
            "[TenantIsolation] Audit log failed: action={}, tenant_id={}, error={}",
            action, tenant_id, e
        );
    }
    Ok(())
}

/// Record an unsupported Telegram callback without making the webhook fail.
pub async fn log_unsupported_telegram_callback(
    pool: &PgPool,
    tenant_id: &str,
    callback_id: &str,
    callback_data: &str,
    callback_chat_id: Option<i64>,
    telegram_user_id: Option<i64>,
) -> Result<()> {
    let payload = json!({
        "request_path": "telegram/callback_query",
        "callback_data": callback_data,
        "callback_chat_id": callback_chat_id,
        "telegram_user_id": telegram_user_id,
    });

    if let Err(error) = do_log_event(
        pool,
        tenant_id,
        "unsupported_telegram_callback",
        "telegram_bot",
        None,
        Some("telegram_callback"),
        Some(callback_id),
        Some(payload),
    )
    .await
    {
        warn!(
            tenant_id,
            callback_id,
            error = %error,
            "Unsupported Telegram callback audit log failed"
        );
    }
    Ok(())
}

/// Create an audit log entry.
async fn do_log_event(
    pool: &PgPool,
    tenant_id: &str,
    action: &str,
    operator_role: &str,
    actor_user_id: Option<Uuid>,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
    payload_json: Option<Value>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO tenant_audit_logs (
            tenant_id,
            operator_role,
            actor_user_id,
            action,
            resource_type,
            resource_id,
            payload,
            created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(tenant_id)
    .bind(operator_role)
    .bind(actor_user_id)
    .bind(action)
    .bind(resource_type.as_ref())
    .bind(resource_id.as_ref())
    .bind(payload_json)
    .execute(pool)
    .await?;
    Ok(())
}
