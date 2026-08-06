// Config API - System configuration management
use axum::{extract::State, http::HeaderMap, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;
use tgbot_domain::error::{ApiError, ApiResult};

use crate::auth::resolve_scope;
use crate::state::AppState;

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SaveConfigRequest {
    pub llm_api_mode: Option<String>,
    pub llm_model_type: Option<String>,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_boolish")]
    pub llm_model_lock_enabled: Option<bool>,
    pub llm_model_lock_target: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_boolish")]
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
    matches!(
        value.trim().to_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn deserialize_optional_boolish<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(value)),
        Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(Some(true)),
            "0" | "false" | "no" | "off" => Ok(Some(false)),
            _ => Err(serde::de::Error::custom("expected a boolean value")),
        },
        _ => Err(serde::de::Error::custom("expected a boolean value")),
    }
}

fn validate_setting<'a>(field: &str, value: &'a str, max_chars: usize) -> ApiResult<&'a str> {
    let value = value.trim();
    if value.chars().count() > max_chars {
        return Err(ApiError::BadRequest(format!("{field} is too long")));
    }
    Ok(value)
}

fn validate_http_url(field: &str, value: &str) -> ApiResult<()> {
    if !(value.starts_with("https://") || value.starts_with("http://")) {
        return Err(ApiError::BadRequest(format!(
            "{field} must use http:// or https://"
        )));
    }
    Ok(())
}

fn mask_secret(value: &str, prefix: usize, suffix: usize) -> String {
    if value.is_empty() {
        return String::new();
    }
    let characters: Vec<char> = value.chars().collect();
    if characters.len() <= prefix.saturating_add(suffix) {
        return "...".to_string();
    }
    format!(
        "{}...{}",
        characters[..prefix].iter().collect::<String>(),
        characters[characters.len() - suffix..]
            .iter()
            .collect::<String>()
    )
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

    let rows = sqlx::query("SELECT key, value FROM platform.admin_settings WHERE key = ANY($1)")
        .bind(vec![
            "llm_api_mode",
            "llm_model_type",
            "llm_base_url",
            "llm_api_key",
            "llm_model",
            "llm_model_lock_enabled",
            "llm_model_lock_target",
            "global_auto_translate",
            "translation_engine",
        ])
        .fetch_all(state.db()?)
        .await?;
    let settings: HashMap<String, String> = rows
        .iter()
        .map(|row| Ok((row.try_get("key")?, row.try_get("value")?)))
        .collect::<Result<_, sqlx::Error>>()?;
    let setting = |key: &str, default: String| settings.get(key).cloned().unwrap_or(default);
    let llm_api_mode = setting("llm_api_mode", "rented".into());
    let llm_model_type = setting("llm_model_type", "openai".into());
    let llm_base_url = setting(
        "llm_base_url",
        std::env::var("LLM_BASE_URL").unwrap_or_default(),
    );
    let llm_api_key_configured = settings
        .get("llm_api_key")
        .filter(|value| !value.is_empty())
        .is_some()
        || std::env::var("LLM_API_KEY")
            .map(|value| !value.is_empty())
            .unwrap_or(false);
    let llm_model = setting("llm_model", String::new());
    let llm_model_lock_enabled = is_truthy(&setting("llm_model_lock_enabled", "false".into()));
    let llm_model_lock_target = setting("llm_model_lock_target", "gemini-3.1-pro-preview".into());
    let global_auto_translate = is_truthy(&setting("global_auto_translate", "false".into()));
    let translation_engine = setting("translation_engine", "llm".into());

    // Get bot token masked
    let bot_token_masked = mask_secret(&state.config.bot_token, 0, 0);

    // Get admin_id from environment
    let admin_id: i64 = std::env::var("ADMIN_ID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "llm_api_mode": llm_api_mode,
            "llm_model_type": llm_model_type,
            "llm_base_url": llm_base_url,
            "llm_api_key": "",
            "llm_api_key_configured": llm_api_key_configured,
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

#[cfg(test)]
mod tests {
    use super::mask_secret;

    #[test]
    fn mask_secret_never_returns_short_secret() {
        assert_eq!(mask_secret("short", 8, 4), "...");
        assert_eq!(mask_secret("", 8, 4), "");
        assert_eq!(mask_secret("1234567890:secret", 0, 0), "...");
    }

    #[test]
    fn mask_secret_preserves_only_requested_edges_for_long_values() {
        assert_eq!(mask_secret("abcdefghijklmnop", 2, 2), "ab...op");
    }
}

// POST /api/config - Save system configuration
pub async fn save_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SaveConfigRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    let SaveConfigRequest {
        llm_api_mode,
        llm_model_type,
        llm_base_url,
        llm_api_key,
        llm_model,
        llm_model_lock_enabled,
        llm_model_lock_target,
        global_auto_translate,
        translation_engine,
    } = body;
    let mut settings: Vec<(&str, String)> = Vec::new();

    if let Some(value) = llm_api_mode {
        let value = validate_setting("llm_api_mode", &value, 20)?;
        if !matches!(value, "rented" | "own") {
            return Err(ApiError::BadRequest("invalid llm_api_mode".into()));
        }
        settings.push(("llm_api_mode", value.into()));
    }
    if let Some(value) = llm_model_type {
        let value = validate_setting("llm_model_type", &value, 30)?;
        if !matches!(value, "openai" | "deepseek" | "gemini") {
            return Err(ApiError::BadRequest("invalid llm_model_type".into()));
        }
        settings.push(("llm_model_type", value.into()));
    }
    if let Some(value) = llm_base_url {
        let value = validate_setting("llm_base_url", &value, 2_048)?;
        if !value.is_empty() {
            validate_http_url("llm_base_url", value)?;
        }
        settings.push(("llm_base_url", value.into()));
    }
    if let Some(value) = llm_api_key {
        let value = validate_setting("llm_api_key", &value, 8_192)?;
        if !value.is_empty() {
            settings.push(("llm_api_key", value.into()));
        }
    }
    if let Some(value) = llm_model {
        let value = validate_setting("llm_model", &value, 200)?;
        if !value.is_empty() {
            settings.push(("llm_model", value.into()));
        }
    }

    let requested_lock_target = llm_model_lock_target
        .as_deref()
        .map(|value| validate_setting("llm_model_lock_target", value, 200))
        .transpose()?
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(value) = &requested_lock_target {
        settings.push(("llm_model_lock_target", value.clone()));
    }
    if let Some(enabled) = llm_model_lock_enabled {
        settings.push(("llm_model_lock_enabled", enabled.to_string()));
        if enabled {
            let lock_target = match requested_lock_target {
                Some(value) => value,
                None => sqlx::query_scalar::<_, String>(
                    "SELECT value FROM platform.admin_settings WHERE key = 'llm_model_lock_target'",
                )
                .fetch_optional(state.db()?)
                .await?
                .unwrap_or_else(|| "gemini-3.1-pro-preview".into()),
            };
            settings.push(("llm_model", lock_target));
        }
    }
    if let Some(enabled) = global_auto_translate {
        settings.push(("global_auto_translate", enabled.to_string()));
    }
    if let Some(value) = translation_engine {
        let value = validate_setting("translation_engine", &value, 50)?;
        if !value.is_empty() {
            settings.push(("translation_engine", value.into()));
        }
    }

    let mut transaction = state.db()?.begin().await?;
    for (key, value) in &settings {
        sqlx::query(
            "INSERT INTO platform.admin_settings (key, value, updated_at) VALUES ($1, $2, now()) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;

    Ok(Json(
        json!({ "status": "ok", "data": {"updated": settings.len()} }),
    ))
}

// POST /api/config/vertex - Vertex AI one-click configuration
pub async fn vertex_oneclick_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VertexOneClickRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    let vertex_api_key = validate_setting("vertex_api_key", &body.vertex_api_key, 8_192)?;
    if vertex_api_key.is_empty() {
        return Err(ApiError::BadRequest("vertex_api_key required".into()));
    }
    let oneapi_base_url = body
        .oneapi_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("https://generativelanguage.googleapis.com/v1beta")
        .trim_end_matches('/')
        .to_string();
    validate_http_url("oneapi_base_url", &oneapi_base_url)?;
    let requested_model = body
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("gemini-2.5-flash-lite")
        .to_string();
    validate_setting("model", &requested_model, 200)?;
    let lock_enabled = sqlx::query_scalar::<_, String>(
        "SELECT value FROM platform.admin_settings WHERE key = 'llm_model_lock_enabled'",
    )
    .fetch_optional(state.db()?)
    .await?
    .map(|value| is_truthy(&value))
    .unwrap_or(false);
    let model = if lock_enabled {
        sqlx::query_scalar::<_, String>(
            "SELECT value FROM platform.admin_settings WHERE key = 'llm_model_lock_target'",
        )
        .fetch_optional(state.db()?)
        .await?
        .unwrap_or_else(|| "gemini-3.1-pro-preview".into())
    } else {
        requested_model
    };
    let relay_api_key = body
        .relay_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "sk-{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )
        });
    validate_setting("relay_api_key", &relay_api_key, 8_192)?;

    // Build URLs
    let model_infer_url = format!("{oneapi_base_url}/models/{model}:generateContent");
    let model_list_url = format!("{oneapi_base_url}/models");
    let openai_chat_url = format!("{oneapi_base_url}/chat/completions");

    // Save all configs
    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value, updated_at) VALUES ('vertex_api_key', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(vertex_api_key)
    .execute(state.db()?)
    .await?;

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value, updated_at) VALUES ('oneapi_base_url', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&oneapi_base_url)
    .execute(state.db()?)
    .await?;

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value, updated_at) VALUES ('relay_api_key', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&relay_api_key)
    .execute(state.db()?)
    .await?;

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value, updated_at) VALUES ('llm_model', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&model)
    .execute(state.db()?)
    .await?;

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value, updated_at) VALUES ('llm_base_url', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&openai_chat_url)
    .execute(state.db()?)
    .await?;

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value, updated_at) VALUES ('llm_api_key', $1, now()) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&relay_api_key)
    .execute(state.db()?)
    .await?;

    // Mask keys for response
    let relay_masked = mask_secret(&relay_api_key, 8, 4);
    let vertex_masked = mask_secret(vertex_api_key, 4, 4);

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "oneapi_base_url": oneapi_base_url,
            "model_infer_url": model_infer_url,
            "model_list_url": model_list_url,
            "openai_chat_url": openai_chat_url,
            "relay_api_key": "",
            "relay_api_key_configured": true,
            "relay_api_key_masked": relay_masked,
            "model": model,
            "vertex_api_key_masked": vertex_masked,
        },
        "message": "Vertex AI configuration generated and saved"
    })))
}

// GET /api/config/gemini_models - Get available Gemini models
pub async fn get_gemini_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;
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
