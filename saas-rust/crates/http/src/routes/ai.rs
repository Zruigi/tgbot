//! AI API routes — chat, ad detection, pipeline mode, usage
//!
//! Endpoints:
//!   POST /api/ai/chat             — AI chat with n8n/frontend compatibility
//!   POST /api/ai/detect_ad        — Ad detection
//!   GET  /api/ai/pipeline_mode    — Get AI pipeline mode
//!   POST /api/ai/pipeline_mode   — Set AI pipeline mode
//!   GET  /api/openai/models       — List available models
//!   POST /api/openai/chat/completions — OpenAI-compatible chat

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_integrations::litellm::{ChatCompletionRequest, ChatMessage};

use crate::auth::resolve_scope;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AiChatRequest {
    #[serde(alias = "input", alias = "message")]
    pub content: Option<String>,
    #[serde(alias = "userId")]
    pub user_id: Option<i64>,
    pub model: Option<String>,
    pub feature_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PipelineModeRequest {
    pub mode: String,
    #[serde(alias = "tenantId")]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PipelineModeQuery {
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillListQuery {
    #[serde(default, deserialize_with = "deserialize_truthy")]
    pub include_builtin: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSkillRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub prompt_template: String,
    pub source: Option<String>,
    pub is_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ToggleSkillRequest {
    pub name: String,
    pub is_enabled: bool,
}

// ---------------------------------------------------------------------------
// POST /api/ai/chat — Structured Intent Router
// Calls LLM in JSON mode (temperature 0, 15s timeout) to analyze user intent
// Routes to ad query tool if needed, otherwise returns direct reply
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct IntentAnalysis {
    need_tool: bool,
    tool_name: Option<String>, // fixed to "query_ad_clusters"
    group_id: Option<String>,  // advisory only, overridden by system chat_id
    reply_content: String,     // direct reply or waiting prompt
}

pub async fn ai_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AiChatRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = scope.tenant_id.as_deref().unwrap_or("default");

    let content = body.content.as_deref().unwrap_or("").trim();
    if content.is_empty() {
        return Ok(Json(json!({
            "status": "ok",
            "reply": "请输入消息内容",
            "tool_calls_made": [],
            "tool_results": {},
            "data": { "reply": "请输入消息内容" },
        })));
    }

    let user_id = body.user_id.unwrap_or(0);
    let real_chat_id = body.user_id;

    // Input truncation (500 chars, SPEC-2.2b)
    let truncated_input = content.chars().take(500).collect::<String>();

    // Structured intent analysis via LLM (JSON mode, temperature 0, 15s timeout)
    // System prompt guides LLM to output JSON with specific structure
    let intent_prompt = format!(
        r#"分析用户意图，返回 JSON 对象：
{{"need_tool": false|true, "tool_name": "query_ad_clusters"|null, "group_id": "群ID"|null, "reply_content": "直接回复或等待提示"}}

用户消息: {}"#,
        truncated_input
    );

    // Structured intent analysis: LLM in JSON mode, temperature 0, 15s timeout
    let intent_request = ChatCompletionRequest {
        model: "gemini-2.5-flash-lite".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: intent_prompt,
        }],
        temperature: Some(0.0), // Deterministic output (temperature 0, spec-2.2b)
        max_tokens: None,
        response_format: None,
        extra: Value::Null,
    };

    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        state.llm.chat_completion(&intent_request),
    )
    .await
    {
        Ok(Ok(resp)) => resp,
        Ok(Err(_)) | Err(_) => {
            // Timeout or LLM error → degrade gracefully
            return Ok(Json(json!({
                "status": "ok",
                "reply": "抱歉，处理过程中出现了问题。请重新尝试。",
                "tool_calls_made": [],
                "tool_results": {},
                "data": { "reply": "抱歉，处理过程中出现了问题。请重新尝试。" },
            })));
        }
    };

    let reply_text = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default();

    // Parse IntentAnalysis JSON (fail-closed on parse error)
    let intent: IntentAnalysis = match serde_json::from_str(&reply_text) {
        Ok(parsed) => parsed,
        Err(_) => {
            // JSON parse failure → friendly degraded reply, no tool execution
            return Ok(Json(json!({
                "status": "ok",
                "reply": "抱歉，我在处理您的请求时遇到了问题。请尝试用更清晰的方式重新表述。",
                "tool_calls_made": [],
                "tool_results": {},
                "data": { "reply": "抱歉，我在处理您的请求时遇到了问题。请尝试用更清晰的方式重新表述。" },
            })));
        }
    };

    // Log usage
    let prompt_tokens = (truncated_input.len() / 4) as i64;
    let completion_tokens = (reply_text.len() / 4) as i64;
    let total_tokens = prompt_tokens + completion_tokens;
    let _ = sqlx::query(
        "INSERT INTO ai_usage_logs (channel, user_id, tenant_id, feature_key, result_status, endpoint, model, prompt_tokens, completion_tokens, total_tokens, estimated_cost_usd) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
    )
    .bind("web_chat")
    .bind(user_id)
    .bind(tenant_id)
    .bind(body.feature_key.as_deref().unwrap_or("general"))
    .bind("success")
    .bind("ai_chat")
    .bind("gemini-2.5-flash-lite")
    .bind(prompt_tokens)
    .bind(completion_tokens)
    .bind(total_tokens)
    .bind(0.0f64)
    .execute(state.db()?)
    .await;

    // Route based on intent analysis
    if !intent.need_tool {
        // Direct reply, no tool execution
        return Ok(Json(json!({
            "status": "ok",
            "reply": intent.reply_content,
            "tool_calls_made": [],
            "tool_results": {},
            "data": { "reply": intent.reply_content },
        })));
    }

    // Tool routing: if tool_name specified and is "query_ad_clusters" → prepare to call
    // (Actual tool execution would happen in n8n TgbotAiAgent.js)
    let effective_tool_name = intent
        .tool_name
        .as_deref()
        .unwrap_or("query_ad_clusters");

    // Override group_id with real system chat_id (prompt injection defense)
    let effective_group_id = real_chat_id
        .or_else(|| intent.group_id.as_ref().and_then(|s| s.parse::<i64>().ok()));

    // Return structured response with tool_calls_made (for n8n consumption)
    Ok(Json(json!({
        "status": "ok",
        "reply": intent.reply_content,
        "tool_calls_made": [effective_tool_name],
        "tool_results": {},
        "data": {
            "reply": intent.reply_content,
            "tool_name": effective_tool_name,
            "group_id": effective_group_id,
        },
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

    let text = body
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if text.is_empty() {
        return Err(ApiError::BadRequest("text is required".into()));
    }
    let context_type = body
        .get("context_type")
        .and_then(Value::as_str)
        .unwrap_or("group_message");
    let result = tgbot_agent::ad_detection::detect_advertising_signals(text, context_type);

    Ok(Json(json!({
        "status": "ok",
        "is_ad": result.is_advertising,
        "is_advertising": result.is_advertising,
        "score": result.ad_score,
        "ad_score": result.ad_score,
        "confidence": result.confidence,
        "risk_level": result.risk_level,
        "findings": result.findings,
        "matched_keywords": result.matched_keywords,
        "url_count": result.url_count,
        "context_type": result.context_type,
        "recommendation": result.recommendation,
    })))
}

// ---------------------------------------------------------------------------
// GET /api/ai/pipeline_mode
// ---------------------------------------------------------------------------

pub async fn get_pipeline_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PipelineModeQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = if scope.role == Role::PlatformAdmin {
        query.tenant_id.as_deref().unwrap_or("platform").trim()
    } else {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    };
    let tenant_id = if tenant_id.is_empty() {
        "platform"
    } else {
        tenant_id
    };
    let setting_key = pipeline_setting_key(tenant_id);

    let mode: Option<String> =
        sqlx::query_scalar::<_, String>("SELECT value FROM platform.admin_settings WHERE key = $1")
            .bind(&setting_key)
            .fetch_optional(state.db()?)
            .await?;
    let mode = mode.unwrap_or_else(|| "agent".to_string());

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "tenantId": tenant_id,
            "mode": mode,
            "modes": ["agent", "legacy"],
        },
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
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    let valid_modes = ["agent", "legacy"];
    if !valid_modes.contains(&body.mode.as_str()) {
        return Err(ApiError::BadRequest(
            format!("mode must be one of: {:?}", valid_modes).into(),
        ));
    }
    let tenant_id = body.tenant_id.as_deref().unwrap_or("platform").trim();
    let tenant_id = if tenant_id.is_empty() {
        "platform"
    } else {
        tenant_id
    };
    let setting_key = pipeline_setting_key(tenant_id);

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value) VALUES ($1, $2) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&setting_key)
    .bind(&body.mode)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "data": { "tenantId": tenant_id, "mode": body.mode },
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
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("gemini-2.5-flash-lite");
    let messages: Vec<ChatMessage> = serde_json::from_value(
        body.get("messages")
            .cloned()
            .ok_or_else(|| ApiError::BadRequest("messages is required".into()))?,
    )
    .map_err(|error| ApiError::BadRequest(format!("invalid messages: {error}")))?;
    if messages.is_empty() {
        return Err(ApiError::BadRequest("messages must not be empty".into()));
    }
    let mut request = ChatCompletionRequest::simple(model, messages);
    request.temperature = body.get("temperature").and_then(Value::as_f64);
    request.max_tokens = body
        .get("max_tokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let response = state
        .llm
        .chat_completion(&request)
        .await
        .map_err(ApiError::Anyhow)?;
    let choices: Vec<Value> = response
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index.unwrap_or(0),
                "message": {
                    "role": choice.message.role.as_deref().unwrap_or("assistant"),
                    "content": choice.message.content,
                    "tool_calls": choice.message.tool_calls,
                },
                "finish_reason": choice.finish_reason,
            })
        })
        .collect();
    let usage = response.usage.as_ref().map(|usage| {
        json!({
            "prompt_tokens": usage.prompt_tokens.unwrap_or(0),
            "completion_tokens": usage.completion_tokens.unwrap_or(0),
            "total_tokens": usage.total_tokens.unwrap_or(0),
        })
    });
    Ok(Json(json!({
        "id": response.id.unwrap_or_else(|| format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis())),
        "object": response.object.unwrap_or_else(|| "chat.completion".into()),
        "created": response.created.unwrap_or_else(|| chrono::Utc::now().timestamp()),
        "model": response.model.unwrap_or_else(|| model.into()),
        "choices": choices,
        "usage": usage,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/openai/exec
// ---------------------------------------------------------------------------

pub async fn openai_exec(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    let input = body
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if input.is_empty() {
        return Err(ApiError::BadRequest("input is required".into()));
    }
    let model = body.get("model").and_then(Value::as_str);
    let sandbox_id = body
        .get("sandbox_id")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let response = state
        .llm
        .chat(
            vec![ChatMessage {
                role: "user".into(),
                content: input.into(),
            }],
            model,
        )
        .await
        .map_err(ApiError::Anyhow)?;
    let output = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .ok_or_else(|| ApiError::Anyhow(anyhow::anyhow!("LLM returned no content")))?;
    Ok(Json(json!({
        "status": "ok",
        "output": output,
        "output_text": output,
        "result": output,
        "model": response.model.unwrap_or_else(|| model.unwrap_or("gemini-2.5-flash-lite").into()),
        "sandbox_id": sandbox_id,
        "usage": {
            "total_tokens": response.usage.and_then(|usage| usage.total_tokens).unwrap_or(0)
        }
    })))
}

// ---------------------------------------------------------------------------
// AI Skills API — frontend chat/skills compatibility
// ---------------------------------------------------------------------------

pub async fn list_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SkillListQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = skill_tenant_id(&scope);
    let rows = sqlx::query(
        "SELECT id, name, description, prompt_template, source, is_enabled, created_at, updated_at \
         FROM tenant_saas.ai_custom_skills \
         WHERE tenant_id = $1 AND ($2 OR source != 'builtin_fs') \
         ORDER BY updated_at DESC, id DESC LIMIT 100",
    )
    .bind(tenant_id)
    .bind(query.include_builtin)
    .fetch_all(state.db()?)
    .await?;
    let skills: Vec<Value> = rows.iter().map(skill_row_json).collect::<Result<_, _>>()?;
    Ok(Json(
        json!({"status": "ok", "data": skills, "total": skills.len()}),
    ))
}

pub async fn upsert_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertSkillRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_skill_write(&scope)?;
    let tenant_id = skill_tenant_id(&scope);
    let name = validate_skill_text("name", &body.name, 100, false)?;
    let description = validate_skill_text("description", &body.description, 2_000, true)?;
    let prompt_template =
        validate_skill_text("prompt_template", &body.prompt_template, 20_000, true)?;
    let source = match body.source.as_deref().unwrap_or("custom").trim() {
        "custom" => "custom",
        _ => return Err(ApiError::BadRequest("source must be custom".into())),
    };
    let row = sqlx::query(
        "INSERT INTO tenant_saas.ai_custom_skills \
         (tenant_id, name, description, prompt_template, source, is_enabled) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (tenant_id, name) DO UPDATE SET \
           description = EXCLUDED.description, prompt_template = EXCLUDED.prompt_template, \
           source = EXCLUDED.source, is_enabled = EXCLUDED.is_enabled, updated_at = now() \
         RETURNING id, name, description, prompt_template, source, is_enabled, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(name)
    .bind(description)
    .bind(prompt_template)
    .bind(source)
    .bind(body.is_enabled.unwrap_or(true))
    .fetch_one(state.db()?)
    .await?;
    Ok(Json(json!({"status": "ok", "data": skill_row_json(&row)?})))
}

pub async fn toggle_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ToggleSkillRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_skill_write(&scope)?;
    let tenant_id = skill_tenant_id(&scope);
    let name = validate_skill_text("name", &body.name, 100, false)?;
    let row = sqlx::query(
        "UPDATE tenant_saas.ai_custom_skills SET is_enabled = $1, updated_at = now() \
         WHERE tenant_id = $2 AND name = $3 \
         RETURNING id, name, description, prompt_template, source, is_enabled, created_at, updated_at",
    )
    .bind(body.is_enabled)
    .bind(tenant_id)
    .bind(name)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::NotFound("skill not found".into()))?;
    Ok(Json(json!({"status": "ok", "data": skill_row_json(&row)?})))
}

pub async fn import_builtin_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_skill_write(&scope)?;
    let tenant_id = skill_tenant_id(&scope);
    let discovered = discover_project_skills()?;
    let mut imported = 0u64;
    for skill in &discovered {
        imported += sqlx::query(
            "INSERT INTO tenant_saas.ai_custom_skills \
             (tenant_id, name, description, prompt_template, source, is_enabled) \
             VALUES ($1, $2, $3, $4, 'builtin_fs', true) \
             ON CONFLICT (tenant_id, name) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(
            skill
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .bind(
            skill
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .bind(format!(
            "引用项目内技能: {}",
            skill
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ))
        .execute(state.db()?)
        .await?
        .rows_affected();
    }
    Ok(Json(json!({
        "status": "ok",
        "data": {"imported": imported, "detected": discovered.len()},
        "message": format!("已导入 {}/{} 个本地 Skills", imported, discovered.len()),
    })))
}

pub async fn list_discovered_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    let skills = discover_project_skills()?;
    Ok(Json(
        json!({"status": "ok", "data": skills, "total": skills.len()}),
    ))
}

fn deserialize_truthy<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn pipeline_setting_key(tenant_id: &str) -> String {
    if tenant_id == "platform" {
        "ai_pipeline_mode".to_string()
    } else {
        format!("ai_pipeline_mode:{tenant_id}")
    }
}

fn skill_tenant_id(scope: &tgbot_domain::scope::Scope) -> &str {
    scope.tenant_id.as_deref().unwrap_or("platform")
}

fn require_skill_write(scope: &tgbot_domain::scope::Scope) -> ApiResult<()> {
    if matches!(
        scope.role,
        Role::PlatformAdmin | Role::MerchantOwner | Role::MerchantAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn validate_skill_text<'a>(
    field: &str,
    value: &'a str,
    max_chars: usize,
    allow_empty: bool,
) -> ApiResult<&'a str> {
    let value = value.trim();
    if !allow_empty && value.is_empty() {
        return Err(ApiError::BadRequest(format!("{field} is required")));
    }
    if value.chars().count() > max_chars {
        return Err(ApiError::BadRequest(format!("{field} is too long")));
    }
    Ok(value)
}

fn skill_row_json(row: &sqlx::postgres::PgRow) -> Result<Value, sqlx::Error> {
    Ok(json!({
        "id": row.try_get::<i64, _>("id")?,
        "name": row.try_get::<String, _>("name")?,
        "description": row.try_get::<String, _>("description")?,
        "prompt_template": row.try_get::<String, _>("prompt_template")?,
        "source": row.try_get::<String, _>("source")?,
        "is_enabled": row.try_get::<bool, _>("is_enabled")?,
        "created_at": row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")?,
        "updated_at": row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")?,
    }))
}

fn discover_project_skills() -> ApiResult<Vec<Value>> {
    let configured_root = std::env::var_os("TGBOT_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|error| ApiError::Anyhow(error.into()))?);
    let project_root = configured_root
        .canonicalize()
        .map_err(|error| ApiError::Anyhow(error.into()))?;
    let configured_dirs = std::env::var("AI_SKILL_LOCAL_DIRS").unwrap_or_else(|_| "skills".into());
    let mut skills = BTreeMap::new();

    for relative in configured_dirs
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let path = Path::new(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|component| component == Component::ParentDir)
        {
            continue;
        }
        let candidate = project_root.join(path);
        let Ok(candidate) = candidate.canonicalize() else {
            continue;
        };
        if !candidate.starts_with(&project_root) || !candidate.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(candidate) else {
            continue;
        };
        for entry in entries.flatten() {
            let skill_directory = entry.path();
            let skill_file = skill_directory.join("SKILL.md");
            if !skill_directory.is_dir() || !skill_file.is_file() {
                continue;
            }
            let Some(name) = skill_directory.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 100 {
                continue;
            }
            let description = read_skill_description(&skill_file);
            skills.entry(name.to_string()).or_insert_with(|| {
                json!({
                    "name": name,
                    "description": description,
                    "source": "builtin_scan",
                })
            });
        }
    }

    Ok(skills.into_values().collect())
}

fn read_skill_description(path: &Path) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return "项目内技能模板".into();
    };
    content
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("description")
                .then(|| value.trim().trim_matches(['\'', '"']).to_string())
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "项目内技能模板".into())
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
