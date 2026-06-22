//! Group Verify Config API — verification configuration and stats
//!
//! Endpoints:
//!   GET  /api/group_verify_config         — get verification config for groups
//!   POST /api/group_verify_config         — update verification config
//!   GET  /api/verify/stats                — verification statistics
//!   GET  /api/verify/growth               — verification growth data

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;

use crate::auth::resolve_scope;
use crate::state::AppState;

use super::platform::row_to_json;

// ---------------------------------------------------------------------------
// Query / body structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct VerifyConfigQuery {
    pub group_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyConfigUpdateRequest {
    pub group_id: i64,
    pub verify_enabled: Option<bool>,
    pub verify_type: Option<String>,
    pub verify_timeout: Option<i32>,
    pub verify_channel_id: Option<String>,
    pub verify_questions: Option<String>,
    pub verify_msg_template: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyStatsQuery {
    pub group_id: Option<i64>,
    pub days: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyGrowthQuery {
    pub group_id: Option<i64>,
    pub days: Option<i64>,
}

// ---------------------------------------------------------------------------
// GET /api/group_verify_config
// ---------------------------------------------------------------------------

pub async fn get_verify_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<VerifyConfigQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let tenant_id = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    } else {
        ""
    };

    let rows = if let Some(group_id) = q.group_id {
        if tenant_id.is_empty() {
            sqlx::query(
                r#"
                SELECT group_id, group_title, verify_enabled, verify_type,
                       verify_timeout, verify_channel_id, verify_questions,
                       verify_msg_template, tenant_id
                FROM groups.tracked_groups
                WHERE group_id = $1
                "#,
            )
            .bind(group_id)
            .fetch_all(&state.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT group_id, group_title, verify_enabled, verify_type,
                       verify_timeout, verify_channel_id, verify_questions,
                       verify_msg_template, tenant_id
                FROM groups.tracked_groups
                WHERE group_id = $1 AND tenant_id = $2
                "#,
            )
            .bind(group_id)
            .bind(tenant_id)
            .fetch_all(&state.pool)
            .await?
        }
    } else if tenant_id.is_empty() {
        // Platform admin: all groups with verify_enabled
        sqlx::query(
            r#"
            SELECT group_id, group_title, verify_enabled, verify_type,
                   verify_timeout, verify_channel_id, verify_questions,
                   verify_msg_template, tenant_id
            FROM groups.tracked_groups
            WHERE verify_enabled = true
            ORDER BY group_title
            "#,
        )
        .fetch_all(&state.pool)
        .await?
    } else {
        // Merchant-scoped: all groups with verify_enabled for this tenant
        sqlx::query(
            r#"
            SELECT group_id, group_title, verify_enabled, verify_type,
                   verify_timeout, verify_channel_id, verify_questions,
                   verify_msg_template, tenant_id
            FROM groups.tracked_groups
            WHERE tenant_id = $1 AND verify_enabled = true
            ORDER BY group_title
            "#,
        )
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await?
    };

    let total = rows.len();
    Ok(Json(json!({
        "status": "ok",
        "data": rows.into_iter().map(row_to_json).collect::<Vec<_>>(),
        "total": total
    })))
}

// ---------------------------------------------------------------------------
// POST /api/group_verify_config
// ---------------------------------------------------------------------------

pub async fn update_verify_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VerifyConfigUpdateRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    // Only owner/admin can update
    if matches!(scope.role, Role::MerchantViewer) {
        return Err(ApiError::Forbidden);
    }

    let tenant_id = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin
    ) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    } else {
        ""
    };

    // Validate verify_type if provided
    if let Some(ref vtype) = body.verify_type {
        let valid = matches!(vtype.as_str(), "captcha" | "question" | "math" | "none");
        if !valid {
            return Err(ApiError::BadRequest(
                "verify_type must be one of: captcha, question, math, none".into(),
            ));
        }
    }

    // Validate verify_timeout if provided
    if let Some(timeout) = body.verify_timeout {
        if timeout < 30 || timeout > 600 {
            return Err(ApiError::BadRequest(
                "verify_timeout must be between 30 and 600 seconds".into(),
            ));
        }
    }

    let row = if tenant_id.is_empty() {
        // Platform admin
        sqlx::query(
            r#"
            UPDATE groups.tracked_groups SET
                verify_enabled     = COALESCE($2, verify_enabled),
                verify_type        = COALESCE($3, verify_type),
                verify_timeout     = COALESCE($4, verify_timeout),
                verify_channel_id  = COALESCE($5, verify_channel_id),
                verify_questions   = COALESCE($6, verify_questions),
                verify_msg_template = COALESCE($7, verify_msg_template),
                updated_at = now()
            WHERE group_id = $1
            RETURNING *
            "#,
        )
        .bind(body.group_id)
        .bind(body.verify_enabled)
        .bind(body.verify_type)
        .bind(body.verify_timeout)
        .bind(body.verify_channel_id)
        .bind(body.verify_questions)
        .bind(body.verify_msg_template)
        .fetch_optional(&state.pool)
        .await?
    } else {
        // Merchant-scoped
        sqlx::query(
            r#"
            UPDATE groups.tracked_groups SET
                verify_enabled     = COALESCE($3, verify_enabled),
                verify_type        = COALESCE($4, verify_type),
                verify_timeout     = COALESCE($5, verify_timeout),
                verify_channel_id  = COALESCE($6, verify_channel_id),
                verify_questions   = COALESCE($7, verify_questions),
                verify_msg_template = COALESCE($8, verify_msg_template),
                updated_at = now()
            WHERE group_id = $2 AND tenant_id = $1
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(body.group_id)
        .bind(body.verify_enabled)
        .bind(body.verify_type)
        .bind(body.verify_timeout)
        .bind(body.verify_channel_id)
        .bind(body.verify_questions)
        .bind(body.verify_msg_template)
        .fetch_optional(&state.pool)
        .await?
    };

    let row = row.ok_or_else(|| ApiError::NotFound("group_not_found".into()))?;

    Ok(Json(json!({
        "status": "ok",
        "data": row_to_json(row)
    })))
}

// ---------------------------------------------------------------------------
// GET /api/verify/stats
// ---------------------------------------------------------------------------

pub async fn verify_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<VerifyStatsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let days = q.days.unwrap_or(7).clamp(1, 90);

    let tenant_id = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    } else {
        ""
    };

    // Total verified members
    let total_verified: i64 = if let Some(group_id) = q.group_id {
        if tenant_id.is_empty() {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND is_verified = true",
            )
            .bind(group_id)
            .fetch_one(&state.pool)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND is_verified = true",
            )
            .bind(group_id)
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?
        }
    } else if tenant_id.is_empty() {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE is_verified = true",
        )
        .fetch_one(&state.pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE tenant_id = $1 AND is_verified = true",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?
    }
    .unwrap_or(0);

    // Total unverified members
    let total_unverified: i64 = if let Some(group_id) = q.group_id {
        if tenant_id.is_empty() {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND is_verified = false AND status = 'active'",
            )
            .bind(group_id)
            .fetch_one(&state.pool)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)::bigint FROM groups.group_members WHERE group_id = $1 AND tenant_id = $2 AND is_verified = false AND status = 'active'",
            )
            .bind(group_id)
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?
        }
    } else if tenant_id.is_empty() {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE is_verified = false AND status = 'active'",
        )
        .fetch_one(&state.pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.group_members WHERE tenant_id = $1 AND is_verified = false AND status = 'active'",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?
    }
    .unwrap_or(0);

    // Recent verifications (within the time window)
    let recently_verified: i64 = if let Some(group_id) = q.group_id {
        if tenant_id.is_empty() {
            sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*)::bigint FROM groups.group_members
                WHERE group_id = $1 AND is_verified = true
                  AND verified_at >= now() - ($2::bigint || ' days')::interval
                "#,
            )
            .bind(group_id)
            .bind(days)
            .fetch_one(&state.pool)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*)::bigint FROM groups.group_members
                WHERE group_id = $1 AND tenant_id = $2 AND is_verified = true
                  AND verified_at >= now() - ($3::bigint || ' days')::interval
                "#,
            )
            .bind(group_id)
            .bind(tenant_id)
            .bind(days)
            .fetch_one(&state.pool)
            .await?
        }
    } else if tenant_id.is_empty() {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint FROM groups.group_members
            WHERE is_verified = true
              AND verified_at >= now() - ($1::bigint || ' days')::interval
            "#,
        )
        .bind(days)
        .fetch_one(&state.pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint FROM groups.group_members
            WHERE tenant_id = $1 AND is_verified = true
              AND verified_at >= now() - ($2::bigint || ' days')::interval
            "#,
        )
        .bind(tenant_id)
        .bind(days)
        .fetch_one(&state.pool)
        .await?
    }
    .unwrap_or(0);

    // Pending verify sessions
    let pending_sessions: i64 = if let Some(group_id) = q.group_id {
        if tenant_id.is_empty() {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)::bigint FROM groups.verify_sessions WHERE group_id = $1 AND status = 'pending'",
            )
            .bind(group_id)
            .fetch_one(&state.pool)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*)::bigint FROM groups.verify_sessions vs
                WHERE vs.group_id = $1 AND vs.status = 'pending'
                  AND vs.group_id IN (SELECT group_id FROM groups.tracked_groups WHERE tenant_id = $2)
                "#,
            )
            .bind(group_id)
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?
        }
    } else if tenant_id.is_empty() {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.verify_sessions WHERE status = 'pending'",
        )
        .fetch_one(&state.pool)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint FROM groups.verify_sessions vs
            WHERE vs.status = 'pending'
              AND vs.group_id IN (SELECT group_id FROM groups.tracked_groups WHERE tenant_id = $1)
            "#,
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?
    }
    .unwrap_or(0);

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "days": days,
            "total_verified": total_verified,
            "total_unverified": total_unverified,
            "recently_verified": recently_verified,
            "pending_sessions": pending_sessions,
            "verify_rate": if total_verified + total_unverified > 0 {
                (total_verified as f64 / (total_verified + total_unverified) as f64 * 100.0).round() / 100.0
            } else {
                0.0
            }
        }
    })))
}

// ---------------------------------------------------------------------------
// GET /api/verify/growth
// ---------------------------------------------------------------------------

pub async fn verify_growth(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<VerifyGrowthQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let days = q.days.unwrap_or(30).clamp(1, 90);

    let tenant_id = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    } else {
        ""
    };

    // Daily verification counts
    let rows = if let Some(group_id) = q.group_id {
        if tenant_id.is_empty() {
            sqlx::query(
                r#"
                SELECT
                    DATE(verified_at) AS date,
                    COUNT(*)::bigint AS verified_count
                FROM groups.group_members
                WHERE group_id = $1
                  AND is_verified = true
                  AND verified_at >= now() - ($2::bigint || ' days')::interval
                GROUP BY DATE(verified_at)
                ORDER BY DATE(verified_at) ASC
                "#,
            )
            .bind(group_id)
            .bind(days)
            .fetch_all(&state.pool)
            .await?
        } else {
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
            .bind(group_id)
            .bind(tenant_id)
            .bind(days)
            .fetch_all(&state.pool)
            .await?
        }
    } else if tenant_id.is_empty() {
        sqlx::query(
            r#"
            SELECT
                DATE(verified_at) AS date,
                COUNT(*)::bigint AS verified_count
            FROM groups.group_members
            WHERE is_verified = true
              AND verified_at >= now() - ($1::bigint || ' days')::interval
            GROUP BY DATE(verified_at)
            ORDER BY DATE(verified_at) ASC
            "#,
        )
        .bind(days)
        .fetch_all(&state.pool)
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
        .fetch_all(&state.pool)
        .await?
    };

    let items: Vec<Value> = rows.into_iter().map(row_to_json).collect();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "days": days,
            "items": items,
            "total": items.len(),
        }
    })))
}
