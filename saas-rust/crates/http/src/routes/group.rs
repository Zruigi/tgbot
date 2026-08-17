//! Group API — tracked groups listing and configuration
//!
//! Endpoints:
//!   GET  /api/groups            — list tracked groups for the tenant
//!   POST /api/groups/config     — update group config (verify, keywords, etc.)

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
pub struct GroupsQuery {
    pub is_monitored: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GroupConfigRequest {
    pub group_id: i64,
    pub is_monitored: Option<bool>,
    pub custom_keywords: Option<String>,
    pub delete_on_revoke: Option<bool>,
    pub revoke_sync_chat_id: Option<i64>,
    pub revoke_sync_mirror: Option<bool>,
    pub verify_enabled: Option<bool>,
    pub verify_type: Option<String>,
    pub verify_timeout: Option<i32>,
    pub verify_channel_id: Option<String>,
    pub verify_questions: Option<String>,
    pub verify_msg_template: Option<String>,
}

// ---------------------------------------------------------------------------
// GET /api/groups
// ---------------------------------------------------------------------------

pub async fn list_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<GroupsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let tenant_id = if matches!(
        scope.role,
        Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
    ) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    } else {
        // Platform admin can see all; use a sentinel to skip tenant filter
        ""
    };

    let rows = if tenant_id.is_empty() {
        // Platform admin: no tenant filter
        if let Some(is_monitored) = q.is_monitored {
            sqlx::query(
                r#"
                SELECT * FROM tracked_groups
                WHERE is_monitored = $1
                ORDER BY updated_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(is_monitored)
            .bind(limit)
            .bind(offset)
            .fetch_all(state.db()?)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT * FROM tracked_groups
                ORDER BY updated_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(state.db()?)
            .await?
        }
    } else {
        // Merchant-scoped: filter by tenant_id
        if let Some(is_monitored) = q.is_monitored {
            sqlx::query(
                r#"
                SELECT * FROM tracked_groups
                WHERE tenant_id = $1 AND is_monitored = $2
                ORDER BY updated_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(tenant_id)
            .bind(is_monitored)
            .bind(limit)
            .bind(offset)
            .fetch_all(state.db()?)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT * FROM tracked_groups
                WHERE tenant_id = $1
                ORDER BY updated_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(tenant_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(state.db()?)
            .await?
        }
    };

    let total = rows.len();
    Ok(Json(json!({
        "status": "ok",
        "data": rows.into_iter().map(row_to_json).collect::<Vec<_>>(),
        "total": total
    })))
}

// ---------------------------------------------------------------------------
// POST /api/groups/config
// ---------------------------------------------------------------------------

pub async fn update_group_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GroupConfigRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;

    // Tenant isolation: merchants can only update their own groups
    let tenant_id = if matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin) {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    } else if matches!(scope.role, Role::MerchantViewer) {
        return Err(ApiError::Forbidden);
    } else {
        // Platform admin: use the tenant_id from the group itself
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
        // Platform admin: no tenant filter
        sqlx::query(
            r#"
            UPDATE tracked_groups SET
                is_monitored       = COALESCE($2, is_monitored),
                custom_keywords    = COALESCE($3, custom_keywords),
                delete_on_revoke   = COALESCE($4, delete_on_revoke),
                revoke_sync_chat_id = COALESCE($5, revoke_sync_chat_id),
                revoke_sync_mirror = COALESCE($6, revoke_sync_mirror),
                verify_enabled     = COALESCE($7, verify_enabled),
                verify_type        = COALESCE($8, verify_type),
                verify_timeout     = COALESCE($9, verify_timeout),
                verify_channel_id  = COALESCE($10, verify_channel_id),
                verify_questions   = COALESCE($11, verify_questions),
                verify_msg_template = COALESCE($12, verify_msg_template),
                updated_at = now()
            WHERE group_id = $1
            RETURNING *
            "#,
        )
        .bind(body.group_id)
        .bind(body.is_monitored)
        .bind(body.custom_keywords)
        .bind(body.delete_on_revoke)
        .bind(body.revoke_sync_chat_id)
        .bind(body.revoke_sync_mirror)
        .bind(body.verify_enabled)
        .bind(body.verify_type)
        .bind(body.verify_timeout)
        .bind(body.verify_channel_id)
        .bind(body.verify_questions)
        .bind(body.verify_msg_template)
        .fetch_optional(state.db()?)
        .await?
    } else {
        // Merchant-scoped: must match tenant_id
        sqlx::query(
            r#"
            UPDATE tracked_groups SET
                is_monitored       = COALESCE($3, is_monitored),
                custom_keywords    = COALESCE($4, custom_keywords),
                delete_on_revoke   = COALESCE($5, delete_on_revoke),
                revoke_sync_chat_id = COALESCE($6, revoke_sync_chat_id),
                revoke_sync_mirror = COALESCE($7, revoke_sync_mirror),
                verify_enabled     = COALESCE($8, verify_enabled),
                verify_type        = COALESCE($9, verify_type),
                verify_timeout     = COALESCE($10, verify_timeout),
                verify_channel_id  = COALESCE($11, verify_channel_id),
                verify_questions   = COALESCE($12, verify_questions),
                verify_msg_template = COALESCE($13, verify_msg_template),
                updated_at = now()
            WHERE group_id = $2 AND tenant_id = $1
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(body.group_id)
        .bind(body.is_monitored)
        .bind(body.custom_keywords)
        .bind(body.delete_on_revoke)
        .bind(body.revoke_sync_chat_id)
        .bind(body.revoke_sync_mirror)
        .bind(body.verify_enabled)
        .bind(body.verify_type)
        .bind(body.verify_timeout)
        .bind(body.verify_channel_id)
        .bind(body.verify_questions)
        .bind(body.verify_msg_template)
        .fetch_optional(state.db()?)
        .await?
    };

    let row = row.ok_or_else(|| ApiError::NotFound("group_not_found".into()))?;

    Ok(Json(json!({
        "status": "ok",
        "data": row_to_json(row)
    })))
}
