//! AI API routes — chat, ad detection, pipeline mode, usage
//!
//! Endpoints:
//!   POST /api/ai/chat             — AI chat with n8n/frontend compatibility
//!   POST /api/ai/detect_ad        — Ad detection
//!   GET  /api/ai/pipeline_mode    — Get AI pipeline mode
//!   POST /api/ai/pipeline_mode   — Set AI pipeline mode
//!   GET  /api/openai/models       — List available models (stub)
//!   POST /api/openai/chat/completions — OpenAI-compatible chat (stub)

use axum::{extract::State, http::HeaderMap, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use tgbot_domain::error::{ApiError, ApiResult};

use crate::auth::resolve_scope;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AiChatRequest {
    pub content: Option<String>,
    pub user_id: Option<i64>,
    pub model: Option<String>,
    pub feature_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PipelineModeRequest {
    pub mode: String,
}

// ---------------------------------------------------------------------------
// POST /api/ai/chat
// ---------------------------------------------------------------------------

pub async fn ai_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AiChatRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().unwrap_or("default");

    let content = body.content.as_deref().unwrap_or("").trim();
    if content.is_empty() {
        return Err(ApiError::BadRequest("input text is required".into()));
    }

    let user_id = body.user_id.unwrap_or(0);
    let model = body.model.as_deref().unwrap_or("gemini-2.5-flash-lite");

    // TODO: Integrate with actual AI service from tgbot-agent crate
    // For now, provide a backward-compatible stub response
    tracing::info!(
        "ai_chat: user_id={}, model={}, tenant_id={}, content_len={}",
        user_id, model, tenant_id, content.len()
    );

    let reply = format!("[AI] 已收到您的消息。此功能当前为占位实现。");

    // Stub usage logging
    let _ = sqlx::query(
        "INSERT INTO ai_usage_log (channel, user_id, tenant_id, feature_key, result_status, endpoint, model, prompt_tokens, completion_tokens, tokens_used, cost) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
    )
    .bind("web_chat")
    .bind(user_id)
    .bind(tenant_id)
    .bind(body.feature_key.as_deref().unwrap_or("general"))
    .bind("success")
    .bind("chat")
    .bind(model)
    .bind((content.len() / 4) as i64)
    .bind((reply.len() / 4) as i64)
    .bind(((content.len() + reply.len()) / 4) as i64)
    .bind(0.0001f64)
    .execute(state.db()?)
    .await;

    Ok(Json(json!({
        "status": "ok",
        "reply": reply,
        "data": { "reply": reply },
    })))
}

// ---------------------------------------------------------------------------
// POST /api/ai/detect_ad
// ---------------------------------------------------------------------------

pub async fn detect_ad(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    // TODO: Integrate with actual ad detection from tgbot-agent crate
    // For now, return a neutral result
    tracing::info!("detect_ad: stub called");

    Ok(Json(json!({
        "status": "ok",
        "is_ad": false,
        "score": 0,
        "confidence": "low",
        "findings": [],
    })))
}

// ---------------------------------------------------------------------------
// GET /api/ai/pipeline_mode
// ---------------------------------------------------------------------------

pub async fn get_pipeline_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().unwrap_or("default");

    let mode: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT value FROM platform.admin_settings WHERE key = 'ai_pipeline_mode'"
    )
    .fetch_optional(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "data": mode.unwrap_or_else(|| "llm".to_string()),
    })))
}

// ---------------------------------------------------------------------------
// POST /api/ai/pipeline_mode
// ---------------------------------------------------------------------------

pub async fn set_pipeline_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PipelineModeRequest>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    let valid_modes = ["llm", "hybrid", "rule"];
    if !valid_modes.contains(&body.mode.as_str()) {
        return Err(ApiError::BadRequest(
            format!("mode must be one of: {:?}", valid_modes).into(),
        ));
    }

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value) VALUES ('ai_pipeline_mode', $1) \
         ON CONFLICT (key) DO UPDATE SET value = $1"
    )
    .bind(&body.mode)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "data": body.mode,
    })))
}

// ---------------------------------------------------------------------------
// GET /api/openai/models
// ---------------------------------------------------------------------------

pub async fn list_openai_models(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;

    Ok(Json(json!({
        "status": "ok",
        "data": [
            { "id": "gemini-2.5-flash-lite", "object": "model", "created": 1700000000, "owned_by": "google" },
            { "id": "gemini-2.5-pro", "object": "model", "created": 1700000000, "owned_by": "google" },
            { "id": "gpt-4o-mini", "object": "model", "created": 1700000000, "owned_by": "openai" },
        ],
    })))
}

// ---------------------------------------------------------------------------
// POST /api/openai/chat/completions
// ---------------------------------------------------------------------------

pub async fn openai_chat_completions(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;

    tracing::info!("openai_chat_completions: stub called");

    Ok(Json(json!({
        "id": "chatcmpl-xxx",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "此功能为占位实现，尚未接入真实 AI 服务。" },
            "finish_reason": "stop",
        }],
    })))
}

// ---------------------------------------------------------------------------
// POST /api/openai/exec
// ---------------------------------------------------------------------------

pub async fn openai_exec(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;

    Ok(Json(json!({
        "status": "ok",
        "result": "exec stub"
    })))
}

// ---------------------------------------------------------------------------
// AI Skills API — frontend chat/skills compatibility
// ---------------------------------------------------------------------------

pub async fn list_skills(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": [], "total": 0})))
}

pub async fn upsert_skill(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": null})))
}

pub async fn toggle_skill(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok"})))
}

pub async fn import_builtin_skills(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": []})))
}

pub async fn list_discovered_skills(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": []})))
}

// ---------------------------------------------------------------------------
// POST /api/ad-pipeline/events/group-message — n8n AdCluster compatibility
// ---------------------------------------------------------------------------

pub async fn ad_pipeline_group_message(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": null})))
}

// ---------------------------------------------------------------------------
// POST /api/ad-pipeline/backfill — n8n AdCluster compatibility
// ---------------------------------------------------------------------------

pub async fn ad_pipeline_backfill(
    State(_state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": {"backfilled": 0}})))
}

// ---------------------------------------------------------------------------
// GET /api/advertisements — n8n AdCrud compatibility
// ---------------------------------------------------------------------------

pub async fn list_advertisements(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": [], "total": 0})))
}

pub async fn get_advertisement(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;
    Ok(Json(json!({"status": "ok", "data": null})))
}
