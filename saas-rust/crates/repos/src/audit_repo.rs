// Audit repository — tenant audit logs
// Migrated from backend/repositories/audit_repo.py

use anyhow::Result;
use sqlx::PgPool;
use tracing::warn;

/// Log a tenant-isolation violation event (non-blocking).
pub async fn log_tenant_isolation_event(
    pool: &PgPool,
    tenant_id: &str,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    _operator_role: &str,
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
        "system",
        None,
        Some(resource_type),
        Some(resource_id),
        Some(&payload.to_string()),
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

/// Create an audit log entry.
async fn do_log_event(
    pool: &PgPool,
    tenant_id: &str,
    action: &str,
    operator_role: &str,
    operator_id: Option<&str>,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
    payload_json: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO tenant_audit_logs (
            tenant_id,
            operator_role,
            operator_id,
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
    .bind(operator_id)
    .bind(action)
    .bind(resource_type.as_ref())
    .bind(resource_id.as_ref())
    .bind(payload_json)
    .execute(pool)
    .await?;
    Ok(())
}
