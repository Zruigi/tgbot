// Config API - System configuration management
use axum::{
    extract::State,
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::scope::Scope;

use crate::auth::resolve_scope;
use crate::state::AppState;

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemConfig {
    pub llm_base_url: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub llm_model_lock_enabled: bool,
    pub llm_model_lock_target: String,
    pub global_auto_translate: bool,
    pub translation_engine: String,
    pub bot_token_masked: String,
    pub admin_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SaveConfigRequest {
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub llm_model_lock_enabled: Option<bool>,
    pub llm_model_lock_target: Option<String>,
    pub global_auto_translate: Option<bool>,
    pub translation_engine: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VertexOneClickRequest {
    pub vertex_api_key: String,
    pub oneapi_base_url: Option<String>,
    pub relay_api_key: Option<String>,
    pub model: Option<String>,
}

// ============================================================================
// Helper Functions
// ============================================================================

fn is_truthy(value: &str) -> bool {
    matches!(value.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

// ============================================================================
// API Endpoints
// ============================================================================

// GET /api/config - Get system configuration
pub async fn get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    let get_setting = |key: &str, default: &str| -> String {
        // Query from admin_settings table
        std::sync::Mutex::new(default).lock().unwrap().clone()
    };

    // Get settings from database
    let llm_base_url = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM admin_settings WHERE key = 'llm_base_url'",
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .unwrap_or_else(|| std::env::var("LLM_BASE_URL").unwrap_or_default());

    let llm_api_key = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM admin_settings WHERE key = 'llm_api_key'",
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .unwrap_or_else(|| std::env::var("LLM_API_KEY").unwrap_or_default());

    let llm_model = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM admin_settings WHERE key = 'llm_model'",
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .unwrap_or_default();

    let llm_model_lock_enabled = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM admin_settings WHERE key = 'llm_model_lock_enabled'",
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .map(|v| is_truthy(&v))
    .unwrap_or(false);

    let llm_model_lock_target = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM admin_settings WHERE key = 'llm_model_lock_target'",
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .unwrap_or_else(|| "gemini-3.1-pro-preview".to_string());

    let global_auto_translate = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM admin_settings WHERE key = 'global_auto_translate'",
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .map(|v| is_truthy(&v))
    .unwrap_or(false);

    let translation_engine = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM admin_settings WHERE key = 'translation_engine'",
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .unwrap_or_else(|| "llm".to_string());

    // Get bot token masked
    let bot_token = &state.config.bot_token;
    let bot_token_masked = if bot_token.len() > 10 {
        format!("{}...", &bot_token[..10])
    } else {
        bot_token.clone()
    };

    // Get admin_id from environment
    let admin_id: i64 = std::env::var("ADMIN_ID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "llm_base_url": llm_base_url,
            "llm_api_key": llm_api_key,
            "llm_model": llm_model,
            "llm_model_lock_enabled": llm_model_lock_enabled,
            "llm_model_lock_target": llm_model_lock_target,
            "global_auto_translate": global_auto_translate,
            "translation_engine": translation_engine,
            "bot_token_masked": bot_token_masked,
            "admin_id": admin_id,
        }
    })))
}

// POST /api/config - Save system configuration
pub async fn save_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SaveConfigRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    // Save each setting to database
    if let Some(llm_base_url) = body.llm_base_url {
        sqlx::query(
            "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_base_url', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(llm_base_url)
        .execute(&state.pool)
        .await?;
    }

    if let Some(llm_api_key) = body.llm_api_key {
        sqlx::query(
            "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_api_key', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(llm_api_key)
        .execute(&state.pool)
        .await?;
    }

    if let Some(llm_model) = body.llm_model {
        if !llm_model.trim().is_empty() {
            sqlx::query(
                "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_model', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            )
            .bind(llm_model.trim())
            .execute(&state.pool)
            .await?;
        }
    }

    if let Some(llm_model_lock_target) = body.llm_model_lock_target {
        if !llm_model_lock_target.trim().is_empty() {
            sqlx::query(
                "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_model_lock_target', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            )
            .bind(llm_model_lock_target.trim())
            .execute(&state.pool)
            .await?;
        }
    }

    if let Some(llm_model_lock_enabled) = body.llm_model_lock_enabled {
        let lock_enabled_str = if llm_model_lock_enabled { "true" } else { "false" };
        sqlx::query(
            "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_model_lock_enabled', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(lock_enabled_str)
        .execute(&state.pool)
        .await?;

        // If lock is enabled, also update llm_model to the lock target
        if llm_model_lock_enabled {
            let lock_target = sqlx::query_scalar::<_, Option<String>>(
                "SELECT value FROM admin_settings WHERE key = 'llm_model_lock_target'",
            )
            .fetch_optional(&state.pool)
            .await?
            .flatten()
            .unwrap_or_else(|| "gemini-3.1-pro-preview".to_string());

            sqlx::query(
                "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_model', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            )
            .bind(lock_target)
            .execute(&state.pool)
            .await?;
        }
    }

    if let Some(global_auto_translate) = body.global_auto_translate {
        let flag_str = if global_auto_translate { "true" } else { "false" };
        sqlx::query(
            "INSERT INTO admin_settings (key, value, updated_at) VALUES ('global_auto_translate', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(flag_str)
        .execute(&state.pool)
        .await?;
    }

    if let Some(translation_engine) = body.translation_engine {
        sqlx::query(
            "INSERT INTO admin_settings (key, value, updated_at) VALUES ('translation_engine', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(translation_engine)
        .execute(&state.pool)
        .await?;
    }

    Ok(Json(json!({ "status": "ok" })))
}

// POST /api/config/vertex - Vertex AI one-click configuration
pub async fn vertex_oneclick_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VertexOneClickRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    if body.vertex_api_key.trim().is_empty() {
        return Err(ApiError::BadRequest("vertex_api_key required".into()));
    }

    let oneapi_base_url = body.oneapi_base_url.unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
    let model = body.model.unwrap_or_else(|| "gemini-2.5-flash-lite".to_string());

    // Generate relay_api_key if not provided
    let relay_api_key = body.relay_api_key.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("sk-{:x}{:x}", timestamp, rand_u64())
    });

    fn rand_u64() -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::SystemTime;
        let mut hasher = DefaultHasher::new();
        SystemTime::now().hash(&mut hasher);
        hasher.finish()
    }

    // Build URLs
    let model_infer_url = format!("{}/models/{}:generateContent", oneapi_base_url.trim_end_matches('/'), model);
    let model_list_url = format!("{}/models", oneapi_base_url.trim_end_matches('/'));
    let openai_chat_url = format!("{}/chat/completions", oneapi_base_url.trim_end_matches('/'));

    // Save all configs
    sqlx::query(
        "INSERT INTO admin_settings (key, value, updated_at) VALUES ('vertex_api_key', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&body.vertex_api_key)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO admin_settings (key, value, updated_at) VALUES ('oneapi_base_url', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&oneapi_base_url)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO admin_settings (key, value, updated_at) VALUES ('relay_api_key', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&relay_api_key)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_model', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&model)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_base_url', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&openai_chat_url)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO admin_settings (key, value, updated_at) VALUES ('llm_api_key', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&relay_api_key)
    .execute(&state.pool)
    .await?;

    // Mask keys for response
    let relay_masked = if relay_api_key.len() > 12 {
        format!("{}...{}", &relay_api_key[..8], &relay_api_key[relay_api_key.len()-4..])
    } else {
        relay_api_key.clone()
    };

    let vertex_masked = if body.vertex_api_key.len() > 8 {
        format!("{}...{}", &body.vertex_api_key[..4], &body.vertex_api_key[body.vertex_api_key.len()-4..])
    } else {
        body.vertex_api_key.clone()
    };

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "oneapi_base_url": oneapi_base_url,
            "model_infer_url": model_infer_url,
            "model_list_url": model_list_url,
            "openai_chat_url": openai_chat_url,
            "relay_api_key": relay_api_key,
            "relay_api_key_masked": relay_masked,
            "model": model,
            "vertex_api_key_masked": vertex_masked,
        },
        "message": "Vertex AI configuration generated and saved"
    })))
}

// GET /api/config/gemini_models - Get available Gemini models
pub async fn get_gemini_models(
    State(_state): State<AppState>,
    _headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let models = vec![
        // Gemini 3.1
        "gemini-3.1-pro-preview",
        // Gemini 2.5
        "gemini-2.5-pro",
        "gemini-2.5-flash",
        "gemini-2.5-flash-lite",
        // Gemini 2.0
        "gemini-2.0-flash",
        "gemini-2.0-flash-lite",
        "gemini-2.0-flash-exp",
        // Gemini 1.5
        "gemini-1.5-pro",
        "gemini-1.5-pro-latest",
        "gemini-1.5-flash",
        "gemini-1.5-flash-latest",
        "gemini-1.5-flash-8b",
        // Legacy
        "gemini-pro",
        "gemini-pro-vision",
    ];

    Ok(Json(json!({
        "status": "ok",
        "data": models
    })))
}