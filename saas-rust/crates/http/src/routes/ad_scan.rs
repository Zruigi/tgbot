//! Ad scan HTTP endpoints.
//!
//! Endpoints:
//! - GET  /api/agent/ad-scan/groups          → list_ad_scan_groups
//! - POST /api/agent/ad-scans                → create_ad_scan
//! - GET  /api/agent/ad-scans/{job_id}       → get_ad_scan
//! - GET  /api/agent/ad-scans/{job_id}/clusters → get_ad_scan_clusters
//! - POST /api/agent/ad-scans/{job_id}/confirm  → confirm_ad_scan

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_services::ad_governance::AdGovernanceService;

use crate::auth::resolve_scope;
use crate::state::AppState;

fn map_ad_scan_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if message.contains("not found") || message.contains("access denied") {
        ApiError::NotFound("ad_scan_not_found".into())
    } else {
        ApiError::Anyhow(error)
    }
}

fn require_ad_scan_write(role: &Role) -> ApiResult<()> {
    if matches!(
        role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::PlatformAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

// ============================================================================
// Request/Response Models
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAdScanRequest {
    pub group_id: i64,
    pub requested_by: Option<String>,
    pub dry_run: Option<bool>,
    pub activate_policy: Option<bool>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfirmAdScanRequest {
    pub dry_run: Option<bool>,
    pub activate_policy: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdScanClustersQuery {
    pub tier: Option<String>,
    pub limit: Option<i32>,
}

// ============================================================================
// Route Handlers
// ============================================================================

/// GET /api/agent/ad-scan/groups
/// List groups available for ad scanning (tenant-scoped).
pub async fn list_ad_scan_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _) = scope.require_merchant()?;

    let service = AdGovernanceService::new(Arc::new(state.db()?.clone()));
    let groups = service.list_groups(&tenant_id).await?;

    Ok(Json(json!({
        "status": "ok",
        "data": groups,
        "count": groups.len(),
    })))
}

/// POST /api/agent/ad-scans
/// Create a new ad scan job.
/// SAFETY: First scan is always forced dry_run=true; activation requires explicit confirm.
pub async fn create_ad_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateAdScanRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_ad_scan_write(&scope.role)?;
    let (tenant_id, _) = scope.require_merchant()?;

    // Validate group_id
    if payload.group_id == 0 {
        return Err(ApiError::BadRequest("group_id required".into()));
    }

    let requested_by = payload
        .requested_by
        .unwrap_or_else(|| scope.role.as_str().to_string());
    let dry_run = payload.dry_run.unwrap_or(true);
    let activate_policy = payload.activate_policy.unwrap_or(false);
    let source = payload.source.unwrap_or_else(|| "agent".to_string());

    let service = AdGovernanceService::new(Arc::new(state.db()?.clone()));
    let job = service
        .create_job(
            &tenant_id,
            payload.group_id,
            &requested_by,
            None,
            dry_run,
            activate_policy,
            &source,
        )
        .await
        .map_err(map_ad_scan_error)?;

    Ok(Json(json!({
        "status": "ok",
        "job": job,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ad_scan_writes_require_owner_or_admin_scope() {
        assert!(require_ad_scan_write(&Role::PlatformAdmin).is_ok());
        assert!(require_ad_scan_write(&Role::MerchantOwner).is_ok());
        assert!(require_ad_scan_write(&Role::MerchantAdmin).is_ok());
        assert!(require_ad_scan_write(&Role::MerchantViewer).is_err());
    }
}

/// GET /api/agent/ad-scans/{job_id}
/// Get scan job status and results (tenant-scoped).
pub async fn get_ad_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _) = scope.require_merchant()?;

    let service = AdGovernanceService::new(Arc::new(state.db()?.clone()));
    let job = service
        .get_job(&tenant_id, &job_id)
        .await
        .map_err(map_ad_scan_error)?;

    Ok(Json(json!({
        "status": "ok",
        "job": job,
    })))
}

/// GET /api/agent/ad-scans/{job_id}/clusters
/// Get governance clusters for a scan job (optional tier filter).
pub async fn get_ad_scan_clusters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Query(query): Query<AdScanClustersQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let (tenant_id, _) = scope.require_merchant()?;

    let tier = query.tier.as_deref();
    let limit = query.limit.unwrap_or(100);

    let service = AdGovernanceService::new(Arc::new(state.db()?.clone()));
    let clusters = service
        .get_clusters(&tenant_id, &job_id, tier, limit)
        .await
        .map_err(map_ad_scan_error)?;

    Ok(Json(json!({
        "status": "ok",
        "data": clusters,
        "count": clusters.len(),
    })))
}

/// POST /api/agent/ad-scans/{job_id}/confirm
/// Confirm and start a scan job execution.
/// SAFETY: This is the explicit activation gate. Requires explicit confirm to proceed.
pub async fn confirm_ad_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Json(payload): Json<ConfirmAdScanRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_ad_scan_write(&scope.role)?;
    let (tenant_id, _) = scope.require_merchant()?;

    let dry_run = payload.dry_run.unwrap_or(true);
    let activate_policy = payload.activate_policy.unwrap_or(false);

    let service = AdGovernanceService::new(Arc::new(state.db()?.clone()));

    // Configure job with confirmed settings
    let _configured = service
        .configure_job(&tenant_id, &job_id, dry_run, activate_policy)
        .await
        .map_err(map_ad_scan_error)?;

    // Start the job (transitions to queued → worker picks it up)
    let job = service
        .start_job(&tenant_id, &job_id)
        .await
        .map_err(map_ad_scan_error)?;

    Ok(Json(json!({
        "status": "ok",
        "job": job,
    })))
}
