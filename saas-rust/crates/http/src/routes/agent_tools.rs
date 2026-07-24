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
use tgbot_domain::error::{ApiResult};

use crate::auth::resolve_scope;
use crate::state::AppState;

pub async fn tool_finance_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    tracing::info!("agent_tool: finance_query called");
    Ok(Json(json!({
        "status": "ok",
        "tool": "finance_query",
        "input": body,
        "result": {
            "status": "stub",
            "message": "真实数据需接入 finance 仓库层"
        }
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
    use std::sync::Arc;

    let tool = Arc::new(SystemQueryTool::new(Arc::new(state.db()?.clone())));
    let context = AgentContext::new(0);
    let result = tool.execute(&body, &context).await;
    Ok(Json(serde_json::to_value(&result).unwrap_or(json!({"status": "error", "message": "serialization failed"}))))
}

pub async fn tool_telegram_sdk_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    tracing::info!("agent_tool: telegram_sdk_query called");
    Ok(Json(json!({
        "status": "ok",
        "tool": "telegram_sdk_query",
        "input": body,
        "result": {
            "status": "stub",
            "message": "真实数据需接入 Telegram SDK 查询仓库"
        }
    })))
}

pub async fn tool_query_ad_clusters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    tracing::info!("agent_tool: query_ad_clusters called");
    Ok(Json(json!({"status": "ok", "tool": "query_ad_clusters", "data": [], "input": body})))
}

pub async fn tool_delete_ad_cluster(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &_headers).await?;
    let cluster_id = body.get("cluster_id").and_then(|v| v.as_str()).unwrap_or("unknown");
    tracing::warn!("agent_tool: delete_ad_cluster called for cluster_id={}", cluster_id);
    Ok(Json(json!({
        "status": "ok",
        "tool": "delete_ad_cluster",
        "cluster_id": cluster_id,
        "action": "audit_logged_stub"
    })))
}
