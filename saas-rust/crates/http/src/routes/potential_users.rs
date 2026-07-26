//! Potential Users API — lead management for merchant onboarding
//!
//! Endpoints:
//!   GET  /api/potential_users                 — list potential users (leads)
//!   GET  /api/potential_users/tags            — get available profile tags
//!   POST /api/potential_users/sync            — sync potential users from groups
//!   POST /api/potential_users/{user_id}/promote  — promote a lead to tracked member

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_services::group_policy::parse_custom_keywords;

use crate::auth::resolve_scope;
use crate::group_message_policy::{apply_group_message_policy, GroupMessagePolicyInput};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PotentialUsersQuery {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub keyword: Option<String>,
    pub tag: Option<String>,
}

// ---------------------------------------------------------------------------
// GET /api/potential_users
// ---------------------------------------------------------------------------

pub async fn list_potential_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PotentialUsersQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let offset = q.offset.unwrap_or(0).max(0);
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let keyword = normalize_filter(q.keyword.as_deref());
    let tag = normalize_filter(q.tag.as_deref());

    let rows = sqlx::query(
        "SELECT user_id, username, first_name, last_name, message_text, keywords, profile_tags, \
                group_id, group_title, captured_at, created_at, COUNT(*) OVER()::bigint AS total \
         FROM tenant_saas.potential_users \
         WHERE tenant_id = $1 \
           AND ($2::text IS NULL OR profile_tags ILIKE '%' || $2 || '%') \
           AND ($3::text IS NULL OR username ILIKE '%' || $3 || '%' \
                OR first_name ILIKE '%' || $3 || '%' \
                OR last_name ILIKE '%' || $3 || '%' \
                OR keywords ILIKE '%' || $3 || '%' \
                OR message_text ILIKE '%' || $3 || '%') \
         ORDER BY captured_at DESC, user_id LIMIT $4 OFFSET $5",
    )
    .bind(tenant_id)
    .bind(tag)
    .bind(keyword)
    .bind(limit)
    .bind(offset)
    .fetch_all(state.db()?)
    .await?;

    let users: Vec<Value> = rows.iter().map(|r| {
        json!({
            "user_id": r.try_get::<i64, _>("user_id").unwrap_or(0),
            "username": r.try_get::<Option<String>, _>("username").unwrap_or(None),
            "first_name": r.try_get::<Option<String>, _>("first_name").unwrap_or(None),
            "last_name": r.try_get::<Option<String>, _>("last_name").unwrap_or(None),
            "message_text": r.try_get::<Option<String>, _>("message_text").unwrap_or(None),
            "keywords": r.try_get::<Option<String>, _>("keywords").unwrap_or(None),
            "profile_tags": r.try_get::<Option<String>, _>("profile_tags").unwrap_or(None),
            "group_id": r.try_get::<i64, _>("group_id").unwrap_or(0),
            "group_title": r.try_get::<Option<String>, _>("group_title").unwrap_or(None),
            "captured_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("captured_at").unwrap_or(None),
            "created_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("created_at").unwrap_or(None),
        })
    }).collect();
    let total = rows
        .first()
        .and_then(|row| row.try_get::<i64, _>("total").ok())
        .unwrap_or(0);

    Ok(Json(json!({"status": "ok", "data": users, "total": total})))
}

// ---------------------------------------------------------------------------
// GET /api/potential_users/tags
// ---------------------------------------------------------------------------

pub async fn get_available_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let rows = sqlx::query(
        "SELECT DISTINCT unnest(string_to_array(profile_tags, ',')) AS tag \
         FROM tenant_saas.potential_users \
         WHERE tenant_id = $1 AND profile_tags IS NOT NULL AND profile_tags != '' \
         ORDER BY tag",
    )
    .bind(tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let tags: Vec<String> = rows
        .iter()
        .filter_map(|r| r.try_get::<String, _>("tag").ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    Ok(Json(json!({"status": "ok", "data": tags})))
}

// ---------------------------------------------------------------------------
// POST /api/potential_users/sync
// ---------------------------------------------------------------------------

pub async fn sync_potential_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    let rows = sqlx::query(
        r#"
        SELECT message.group_id, tracked.group_title, message.tg_msg_id, message.user_id,
               message.username, member.first_name, member.last_name, message.content,
               tracked.custom_keywords
        FROM groups.group_messages message
        JOIN groups.tracked_groups tracked
          ON tracked.group_id = message.group_id AND tracked.tenant_id = message.tenant_id
        LEFT JOIN groups.group_members member
          ON member.group_id = message.group_id
         AND member.user_id = message.user_id
         AND member.tenant_id = message.tenant_id
        WHERE message.tenant_id = $1
          AND tracked.is_monitored = true
          AND message.sender_type = 'user'
          AND message.tg_msg_id IS NOT NULL
          AND NULLIF(btrim(message.content), '') IS NOT NULL
        ORDER BY message.timestamp DESC
        LIMIT 2000
        "#,
    )
    .bind(tenant_id)
    .fetch_all(state.db()?)
    .await?;

    let mut matched = 0_u64;
    for row in &rows {
        let group_title: String = row.try_get("group_title")?;
        let username: Option<String> = row.try_get("username")?;
        let first_name: Option<String> = row.try_get("first_name")?;
        let last_name: Option<String> = row.try_get("last_name")?;
        let content: String = row.try_get("content")?;
        let custom_keywords = parse_custom_keywords(
            row.try_get::<Option<String>, _>("custom_keywords")?
                .as_deref(),
        );
        let summary = apply_group_message_policy(
            state.db()?,
            GroupMessagePolicyInput {
                tenant_id,
                group_id: row.try_get("group_id")?,
                group_title: &group_title,
                telegram_message_id: row.try_get("tg_msg_id")?,
                user_id: row.try_get("user_id")?,
                username: username.as_deref(),
                first_name: first_name.as_deref(),
                last_name: last_name.as_deref(),
                content: &content,
                custom_keywords: &custom_keywords,
            },
        )
        .await?;
        if summary.lead_captured {
            matched += 1;
        }
    }
    tracing::info!(
        processed = rows.len(),
        matched,
        "potential user sync completed"
    );

    Ok(Json(json!({
        "status": "ok",
        "processed": rows.len(),
        "synced": matched,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/potential_users/{user_id}/promote
// ---------------------------------------------------------------------------

pub async fn promote_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_write_access(&scope)?;
    let tenant_id = scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?;

    // Remove from potential users table
    let deleted = sqlx::query(
        "DELETE FROM tenant_saas.potential_users WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .execute(state.db()?)
    .await?
    .rows_affected();

    Ok(Json(json!({
        "status": if deleted > 0 { "ok" } else { "error" },
        "user_id": user_id,
    })))
}

fn normalize_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn require_write_access(scope: &tgbot_domain::scope::Scope) -> ApiResult<()> {
    if matches!(
        scope.role,
        Role::PlatformAdmin | Role::MerchantOwner | Role::MerchantAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
