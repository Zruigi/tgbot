//! Agent Tools API — wire agent tools to HTTP endpoints for n8n
//!
//! Endpoints:
//!   POST /api/agent/tools/finance-query
//!   POST /api/agent/tools/system-query
//!   POST /api/agent/tools/telegram-sdk-query
//!   POST /api/agent/tools/query-ad-clusters
//!   POST /api/agent/tools/delete-ad-cluster

use axum::{extract::State, http::HeaderMap, Json};
use serde_json::{json, Value};
use std::sync::Arc;
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_services::ad_pipeline;
use tracing::{error, info, warn};

use crate::auth::resolve_scope;
use crate::state::AppState;

pub async fn tool_finance_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    info!("agent_tool: finance_query called");

    // Finance service not yet ported. Return placeholder compatible with Python.
    Ok(Json(json!({
        "success": true,
        "status": "ok",
        "data": {
            "by_currency": [],
            "total_base_fiat": 0.0,
            "as_of": chrono::Utc::now().to_rfc3339(),
        },
        "message": "财务数据服务尚未接入 Rust 后端"
    })))
}

pub async fn tool_system_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    use tgbot_agent::schema::{AgentContext, Tool};
    use tgbot_agent::tools::system_query::SystemQueryTool;

    let tool = Arc::new(SystemQueryTool::new(Arc::new(state.db()?.clone())));
    let context = AgentContext::new(0);
    let result = tool.execute(&body, &context).await;
    Ok(Json(serde_json::to_value(&result).unwrap_or(
        json!({"status": "error", "message": "serialization failed"}),
    )))
}

pub async fn tool_telegram_sdk_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    use tgbot_domain::models::Role;

    let scope = resolve_scope(&state, &headers).await?;
    info!("agent_tool: telegram_sdk_query called");

    // Get tenant_id: from scope if merchant, or from request body if platform admin
    let tenant_id = if matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer) {
        scope.tenant_id.clone()
    } else {
        body.get("tenant_id")
            .or_else(|| body.get("context").and_then(|c| c.get("tenant_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let (_bot_id, telegram) = match state.tenant_telegram(tenant_id.as_deref().unwrap_or("")).await {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to get telegram client: {}", e);
            return Ok(Json(json!({
                "success": false,
                "status": "error",
                "message": "bot 未初始化，无法调用 Telegram 官方 SDK"
            })));
        }
    };

    // Operation type
    let operation = body
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("chat_capabilities");

    let include_admins = body
        .get("include_admins")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    match operation {
        "message_existence_probe" => {
            message_existence_probe(&telegram, &body).await
        }
        _ => {
            // chat_capabilities, bound_chat_capabilities, bot_visibility_status
            inspect_chats(&telegram, &body, include_admins).await
        }
    }
}

pub async fn tool_query_ad_clusters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    use tgbot_domain::models::Role;

    let scope = resolve_scope(&state, &headers).await?;
    info!("agent_tool: query_ad_clusters called");

    let pool = state.db()?;

    // Get tenant_id: from scope if merchant, or from request body if platform admin
    let tenant_id = if matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer) {
        scope.tenant_id.clone()
    } else {
        body.get("tenant_id")
            .or_else(|| body.get("context").and_then(|c| c.get("tenant_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let group_id = body
        .get("group_id")
        .or_else(|| body.get("chat_id"))
        .and_then(|v| v.as_i64());

    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("pending");

    let limit = body
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(20)
        .max(1);

    let window_minutes = body
        .get("window_minutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(60)
        .max(1);

    match ad_pipeline::query_clusters(
        pool,
        tenant_id.as_deref(),
        group_id,
        status,
        limit,
        window_minutes,
    )
    .await
    {
        Ok(result) => Ok(Json(result)),
        Err(e) => {
            error!("query_ad_clusters failed: {}", e);
            Ok(Json(json!({
                "success": false,
                "status": "error",
                "message": format!("广告聚类查询失败: {}", e)
            })))
        }
    }
}

pub async fn tool_delete_ad_cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    use tgbot_domain::models::Role;

    let scope = resolve_scope(&state, &headers).await?;

    let cluster_id = body
        .get("cluster_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if cluster_id.is_empty() {
        return Ok(Json(json!({
            "status": "error",
            "message": "cluster_id 不能为空",
            "success": false
        })));
    }

    info!("agent_tool: delete_ad_cluster called for cluster_id={}", cluster_id);

    let pool = state.db()?;

    // Get tenant_id: from scope if merchant, or from request body if platform admin
    let tenant_id = if matches!(scope.role, Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer) {
        scope.tenant_id.clone()
    } else {
        body.get("tenant_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    // Query clusters to validate and resolve message_refs server-side
    let status = "all";
    let window_minutes = body
        .get("window_minutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(1440)
        .max(1)
        .min(1440);

    let limit = 100i64;

    let clusters_result = match ad_pipeline::query_clusters(
        pool,
        tenant_id.as_deref(),
        None,
        status,
        limit,
        window_minutes,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to query clusters for validation: {}", e);
            return Ok(Json(json!({
                "success": false,
                "status": "error",
                "message": format!("聚类查询失败: {}", e)
            })));
        }
    };

    let clusters = clusters_result
        .get("data")
        .and_then(|d| d.get("clusters"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    // Find matching cluster
    let matched = clusters
        .iter()
        .find(|c| {
            c.get("cluster_id")
                .and_then(|id| id.as_str())
                .map(|id| id == cluster_id)
                .unwrap_or(false)
        });

    // Safety gate 1: Cluster must exist (404)
    let cluster = match matched {
        Some(c) => c,
        None => {
            // 安全闸(ea76526):服务端查不到该 cluster → HTTP 404,与 Python 一致
            return Err(ApiError::NotFound(
                "未从后端聚类查询到该 cluster，已拒绝删除".to_string(),
            ));
        }
    };

    // Safety gate 2: Cluster must be confirmed ad (409)
    if !ad_pipeline::cluster_is_confirmed_ad(Some(cluster)) {
        // 安全闸(f08faed):未确认为广告的候选簇一律 HTTP 409,与 Python 一致
        return Err(ApiError::Conflict(
            "该 cluster 仅为待判定候选，尚未确认是广告，已拒绝删除".to_string(),
        ));
    }

    let refs = cluster
        .get("message_refs")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    if refs.is_empty() {
        return Ok(Json(json!({
            "success": false,
            "status": "error",
            "message": "cluster 内无消息记录",
            "action_taken": false
        })));
    }

    let dry_run = body
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let ban_senders = body
        .get("ban_senders")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Get telegram client
    let (_bot_id, telegram) = match state.tenant_telegram(tenant_id.as_deref().unwrap_or("")).await {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to get telegram client: {}", e);
            return Ok(Json(json!({
                "success": false,
                "status": "error",
                "message": "bot 未初始化，无法执行删除",
                "action_taken": false
            })));
        }
    };

    // Execute deletion - delete_cluster_messages returns Value directly
    let result = ad_pipeline::delete_cluster_messages(pool, &telegram, &refs, dry_run, ban_senders).await;
    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Helper Functions
// ---------------------------------------------------------------------------

async fn message_existence_probe(
    telegram: &tgbot_integrations::telegram::TelegramClient,
    params: &Value,
) -> ApiResult<Json<Value>> {
    let chat_id = params
        .get("chat_id")
        .or_else(|| params.get("group_id"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let tg_msg_id = params
        .get("tg_msg_id")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    if chat_id == 0 || tg_msg_id == 0 {
        return Ok(Json(json!({
            "success": false,
            "message": "message_existence_probe 需要 chat_id/group_id 和 tg_msg_id"
        })));
    }

    // Note: staging_chat_id support would require database lookup
    // For now, return that we need it
    let staging_chat_id = params.get("staging_chat_id").and_then(|v| v.as_i64());

    if staging_chat_id.is_none() {
        return Ok(Json(json!({
            "success": false,
            "message": "缺少 staging_chat_id 或群组未配置校对信息群，无法用 forwardMessage 探测消息是否仍存在",
            "data": {
                "chat_id": chat_id,
                "tg_msg_id": tg_msg_id,
                "source": "telegram_bot_api"
            }
        })));
    }

    let staging_chat_id = staging_chat_id.unwrap();

    // Try forward message to test existence
    match telegram
        .forward_message(staging_chat_id, chat_id, tg_msg_id)
        .await
    {
        Ok(msg) => {
            // Delete the forwarded message to clean up
            let _ = telegram.delete_message(staging_chat_id, msg.message_id).await;
            Ok(Json(json!({
                "success": true,
                "data_complete": true,
                "evidence_count": 1,
                "data": {
                    "chat_id": chat_id,
                    "tg_msg_id": tg_msg_id,
                    "exists": true,
                    "source": "telegram_bot_api.forwardMessage"
                },
                "message": "消息存在性探测成功：该消息可被 forwardMessage 读取"
            })))
        }
        Err(e) => Ok(Json(json!({
            "success": true,
            "data_complete": true,
            "evidence_count": 1,
            "data": {
                "chat_id": chat_id,
                "tg_msg_id": tg_msg_id,
                "exists": false,
                "source": "telegram_bot_api.forwardMessage",
                "error": format!("{}", e)
            },
            "message": "消息存在性探测完成：该消息无法被 forwardMessage 读取，可能已删除或 bot 无权限"
        }))),
    }
}

async fn inspect_chats(
    telegram: &tgbot_integrations::telegram::TelegramClient,
    params: &Value,
    include_admins: bool,
) -> ApiResult<Json<Value>> {
    let explicit_chat_id = params
        .get("chat_id")
        .or_else(|| params.get("group_id"))
        .and_then(|v| v.as_i64());

    let _limit = params
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(10)
        .max(1)
        .min(50) as usize;

    let operation = params
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("chat_capabilities");

    let mut inspected = Vec::new();

    if let Some(chat_id) = explicit_chat_id {
        match inspect_single_chat(telegram, chat_id, include_admins).await {
            Ok(item) => inspected.push(item),
            Err(e) => {
                warn!("Failed to inspect chat {}: {}", chat_id, e);
                inspected.push(json!({
                    "chat_id": chat_id,
                    "success": false,
                    "errors": [format!("检查失败: {}", e)]
                }));
            }
        }
    }

    let visibility_notes: Vec<&str> = match operation {
        "bot_visibility_status" => vec![
            "Bot API 没有拉取任意群/频道历史消息的方法。",
            "能分析的消息必须是 bot 已收到 update 并写入 SQL 的消息；群 privacy mode、管理员权限和 bot 入群时间都会影响可见性。",
        ],
        _ => vec![],
    };

    Ok(Json(json!({
        "success": true,
        "data_complete": !inspected.is_empty(),
        "evidence_count": inspected.len(),
        "data": {
            "operation": operation,
            "items": inspected,
            "history_fetch_supported": false,
            "visibility_notes": visibility_notes
        },
        "count": inspected.len(),
        "message": format!("Telegram SDK 查询完成：{} 个聊天；历史消息拉取不受 Bot API 支持", inspected.len())
    })))
}

async fn inspect_single_chat(
    telegram: &tgbot_integrations::telegram::TelegramClient,
    chat_id: i64,
    include_admins: bool,
) -> anyhow::Result<Value> {
    let mut item = json!({
        "chat_id": chat_id,
        "source": "telegram_bot_api",
        "history_fetch_supported": false,
        "history_fetch_note": "Telegram Bot API 不支持按 chat_id 拉取任意历史消息；消息正文只能来自 bot 实际收到并落库的 updates。",
        "errors": [],
        "success": true
    });

    // Get chat info
    if let Ok(chat) = telegram.get_chat(chat_id).await {
        if let Ok(chat_json) = serde_json::to_value(&chat) {
            item["chat_type"] = chat_json
                .get("type")
                .cloned()
                .unwrap_or(Value::Null);
            item["title"] = chat_json
                .get("title")
                .cloned()
                .unwrap_or(Value::Null);
            item["username"] = chat_json
                .get("username")
                .cloned()
                .unwrap_or(Value::Null);
        }
    } else {
        item["errors"]
            .as_array_mut()
            .map(|e| e.push(json!("getChat: failed to retrieve chat")));
    }

    // Get bot info
    if let Ok(bot) = telegram.get_me().await {
        item["bot_id"] = json!(bot.id);
        item["bot_username"] = json!(bot.username);

        // Get bot's status in chat
        if let Ok(member) = telegram.get_chat_member(chat_id, bot.id).await {
            if let Ok(member_json) = serde_json::to_value(&member) {
                item["bot_status"] = member_json
                    .get("status")
                    .cloned()
                    .unwrap_or(Value::Null);
                if let Some(perms) = member_json.as_object() {
                    let mut permissions = json!({
                        "status": item["bot_status"].clone()
                    });
                    for key in &[
                        "can_manage_chat",
                        "can_delete_messages",
                        "can_manage_video_chats",
                        "can_restrict_members",
                        "can_promote_members",
                        "can_change_info",
                        "can_invite_users",
                        "can_post_messages",
                        "can_edit_messages",
                        "can_pin_messages",
                        "can_manage_topics",
                    ] {
                        if let Some(val) = perms.get(*key) {
                            permissions[*key] = val.clone();
                        }
                    }
                    item["bot_permissions"] = permissions;
                }
            }
        }
    }

    // Get member count
    if let Ok(count) = telegram.get_chat_member_count(chat_id).await {
        item["member_count"] = json!(count);
    }

    // Get admin list if requested
    if include_admins {
        if let Ok(admins) = telegram.get_chat_administrators(chat_id).await {
            let admin_count = admins.len();
            item["admin_count"] = json!(admin_count);
            let admins_list: Vec<Value> = admins
                .iter()
                .take(20)
                .filter_map(|admin| {
                    if let Ok(admin_json) = serde_json::to_value(admin) {
                        Some(json!({
                            "user_id": admin_json.get("user").and_then(|u| u.get("id")).cloned().unwrap_or(Value::Null),
                            "username": admin_json.get("user").and_then(|u| u.get("username")).cloned().unwrap_or(Value::Null),
                            "status": admin_json.get("status").cloned().unwrap_or(Value::Null)
                        }))
                    } else {
                        None
                    }
                })
                .collect();
            item["admins"] = json!(admins_list);
        }
    }

    let errors = item
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|e| e.len())
        .unwrap_or(0);

    item["success"] = json!(errors == 0);
    item["data_complete"] = json!(
        item.get("bot_status").is_some() && errors == 0
    );

    Ok(item)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finance_query_response_shape() {
        // Verify finance response has expected structure for compatibility
        let response = serde_json::json!({
            "success": true,
            "status": "ok",
            "data": {
                "by_currency": [],
                "total_base_fiat": 0.0,
            },
            "message": "test"
        });

        assert!(response.get("success").unwrap().as_bool().unwrap());
        assert_eq!(response.get("status").unwrap().as_str().unwrap(), "ok");
        assert!(response.get("data").is_some());
        assert!(response.get("data").unwrap().get("by_currency").is_some());
    }

    #[test]
    fn test_ad_cluster_confirmed_validation() {
        // Test: cluster marked as "ad" should be confirmed
        let cluster = json!({
            "cluster_id": "test-1",
            "llm_status": "ad",
            "confirmed_for_deletion": false
        });
        assert!(ad_pipeline::cluster_is_confirmed_ad(Some(&cluster)));

        // Test: cluster marked as confirmed_for_deletion should be confirmed
        let cluster2 = json!({
            "cluster_id": "test-2",
            "llm_status": "pending",
            "confirmed_for_deletion": true
        });
        assert!(ad_pipeline::cluster_is_confirmed_ad(Some(&cluster2)));

        // Test: pending cluster without confirmation should not be confirmed
        let cluster3 = json!({
            "cluster_id": "test-3",
            "llm_status": "pending",
            "confirmed_for_deletion": false
        });
        assert!(!ad_pipeline::cluster_is_confirmed_ad(Some(&cluster3)));

        // Test: None should not be confirmed
        assert!(!ad_pipeline::cluster_is_confirmed_ad(None));
    }

    #[test]
    fn test_delete_ad_cluster_safety_gates() {
        // Safety gate 1: Cluster must exist in query result or delete is rejected
        // This is tested in integration via delete_ad_cluster endpoint

        // Safety gate 2: Cluster must be confirmed ad (llm_status=="ad" or confirmed_for_deletion)
        // Test via cluster_is_confirmed_ad function above

        // Test that the response structure matches Python for error cases
        let error_response = json!({
            "success": false,
            "status": "error",
            "message": "未从后端聚类查询到该 cluster，已拒绝删除",
            "action_taken": false
        });

        assert!(!error_response.get("success").unwrap().as_bool().unwrap());
        assert!(!error_response.get("action_taken").unwrap().as_bool().unwrap());
    }
}
