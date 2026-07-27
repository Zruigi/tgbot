// MiniApp routes - Bootstrap, Context, Logout/Revoke
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{FixedOffset, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::resolve_scope;
use crate::state::AppState;
use tgbot_domain::error::{ApiError, ApiResult};

#[derive(Clone)]
pub struct GrantStore(Arc<RwLock<HashMap<String, GrantData>>>);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GrantData {
    pub grant_hash: String,
    pub issued_for: String,
    pub role_hint: String,
    pub tenant_id: String,
    pub merchant_id: Option<i64>,
    pub entry_intent: String,
    pub expires_at: String,
    pub ua_fingerprint: String,
    pub proof_salt: String,
}

impl GrantStore {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    pub async fn insert(&self, grant: String, data: GrantData) {
        let mut store = self.0.write().await;
        store.insert(grant, data);
    }

    pub async fn get(&self, grant: &str) -> Option<GrantData> {
        let store = self.0.read().await;
        store.get(grant).cloned()
    }

    pub async fn remove(&self, grant: &str) -> Option<GrantData> {
        let mut store = self.0.write().await;
        store.remove(grant)
    }
}

impl Default for GrantStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
pub struct BootstrapRequest {
    #[serde(rename = "init_data")]
    pub init_data: Option<String>,
    pub otp: Option<String>,
    pub grant: Option<String>,
    #[serde(rename = "entry_intent")]
    pub entry_intent: Option<String>,
    #[serde(rename = "client_context")]
    pub client_context: Option<Value>,
    #[serde(rename = "tenant_hint")]
    pub tenant_hint: Option<String>,
    #[serde(rename = "merchant_id_hint")]
    pub merchant_id_hint: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct LogoutRevokeRequest {
    pub grant: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BusinessWindow {
    pub timezone: String,
    pub start: String,
    pub end: String,
    pub active: bool,
    pub now: String,
}

fn normalize_entry_intent(value: Option<&str>) -> String {
    let intent = value.unwrap_or("menu").to_lowercase();
    if !["menu", "admin_console", "merchant_console", "support"].contains(&intent.as_str()) {
        return "menu".to_string();
    }
    intent
}

fn normalize_role_hint(value: Option<&str>) -> String {
    let role = value.unwrap_or("platform_admin").to_lowercase();
    if !["platform_admin", "merchant_owner"].contains(&role.as_str()) {
        return "platform_admin".to_string();
    }
    role
}

fn identity_label(role: &str) -> &'static str {
    match role {
        "platform_admin" => "平台管理员",
        "merchant_owner" => "商户管理员",
        _ => "未注册用户",
    }
}

fn resolve_identity_scope(
    entry_intent: &str,
    is_platform_admin: bool,
    has_merchant: bool,
    last_scope: &str,
) -> (String, String) {
    let normalized_intent = normalize_entry_intent(Some(entry_intent));
    let normalized_last = normalize_role_hint(Some(last_scope));

    if normalized_intent == "merchant_console" {
        if has_merchant {
            return (
                "merchant_owner".to_string(),
                "merchant_console_entry".to_string(),
            );
        }
        if is_platform_admin {
            return (
                "platform_admin".to_string(),
                "merchant_entry_fallback_platform".to_string(),
            );
        }
    }

    if normalized_intent == "admin_console" {
        if is_platform_admin {
            return (
                "platform_admin".to_string(),
                "admin_console_entry".to_string(),
            );
        }
        if has_merchant {
            return (
                "merchant_owner".to_string(),
                "admin_entry_fallback_merchant".to_string(),
            );
        }
    }

    if is_platform_admin && has_merchant {
        return (
            normalized_last.clone(),
            "dual_identity_last_scope".to_string(),
        );
    }
    if is_platform_admin {
        return (
            "platform_admin".to_string(),
            "platform_identity".to_string(),
        );
    }
    if has_merchant {
        return (
            "merchant_owner".to_string(),
            "merchant_identity".to_string(),
        );
    }
    ("unregistered".to_string(), "unregistered".to_string())
}

fn build_landing_payload(
    role: &str,
    entry_intent: &str,
    business_window: &BusinessWindow,
) -> Value {
    let active = business_window.active;
    let mode = if active { "normal" } else { "after_hours" };

    json!({
        "route": match role {
            "unregistered" => "/onboarding",
            "merchant_owner" => "/saas",
            _ => "/saas"
        },
        "mode": mode,
        "reason": format!("{}_{}", entry_intent, role)
    })
}

fn get_business_window() -> BusinessWindow {
    // Default business window (9:00 - 22:00), evaluated in the product's
    // configured Asia/Shanghai timezone rather than returning a permanent
    // success value.
    let timezone = FixedOffset::east_opt(8 * 60 * 60).expect("valid timezone offset");
    let local_now = Utc::now().with_timezone(&timezone);
    let minute_of_day = local_now.hour() * 60 + local_now.minute();
    let start_minute = 9 * 60;
    let end_minute = 22 * 60;
    let active = start_minute <= minute_of_day && minute_of_day < end_minute;

    BusinessWindow {
        timezone: "Asia/Shanghai".to_string(),
        start: "09:00".to_string(),
        end: "22:00".to_string(),
        active,
        now: local_now.to_rfc3339(),
    }
}

// POST /api/miniapp/bootstrap - Bootstrap MiniApp session
pub async fn bootstrap(
    State(state): State<AppState>,
    Json(body): Json<BootstrapRequest>,
) -> ApiResult<Json<Value>> {
    let entry_intent = normalize_entry_intent(body.entry_intent.as_deref());

    // Extract hints from request
    let grant = body.grant.unwrap_or_default();
    let init_data = body.init_data.unwrap_or_default();
    let otp = body.otp.unwrap_or_default();

    let business_window = get_business_window();

    // Try to authenticate via init_data (TMA), OTP, or grant
    let mut token: Option<String> = None;
    let mut session: Option<Value> = None;

    if !init_data.is_empty() {
        // This compatibility bootstrap endpoint has no verified Telegram bot
        // identity in its request context.  Never turn arbitrary init_data into
        // a platform token; callers must use /api/login/tma, which performs the
        // HMAC check against a configured bot secret, or use a one-time grant.
        let landing = build_landing_payload("unregistered", &entry_intent, &business_window);
        return Ok(Json(json!({
            "status": "reopen_in_telegram_required",
            "message": "无法在 bootstrap 中验证 Telegram 身份，请重新通过已配置 Bot 登录",
            "reason": "tma_verification_required",
            "landing": landing,
            "renewal": {"rotated": false, "source": "init_data", "grant_expires_at": Value::Null},
            "ui_context": {
                "business_window": business_window,
                "entry_intent": entry_intent,
                "identity_label": "需要返回 Telegram"
            }
        })));
    } else if !otp.is_empty() {
        // Authenticate via OTP
        let otp_store = state.otp_store.clone();
        let now = chrono::Utc::now().timestamp() as f64;

        if let Some(otp_data) = otp_store.get(&otp).await {
            if otp_data.expire_at > now {
                otp_store.remove(&otp).await;
                let session_token = crate::routes::auth::issue_session_token(
                    &otp_data.role,
                    &otp_data.tenant_id,
                    otp_data.merchant_id,
                    otp_data.issued_for.as_deref(),
                );
                token = Some(session_token.clone());
                session = Some(json!({
                    "role": otp_data.role,
                    "tenant_id": otp_data.tenant_id,
                    "merchant_id": otp_data.merchant_id,
                    "actor_role": otp_data.role
                }));
            }
        }
    } else if !grant.is_empty() {
        // Authenticate via grant (entry proof cookie)
        let grant_store = state.grant_store.clone();
        if let Some(grant_data) = grant_store.get(&grant).await {
            let session_token = crate::routes::auth::issue_session_token(
                &grant_data.role_hint,
                &grant_data.tenant_id,
                grant_data.merchant_id,
                Some(&grant_data.issued_for),
            );
            token = Some(session_token.clone());
            session = Some(json!({
                "role": grant_data.role_hint,
                "tenant_id": grant_data.tenant_id,
                "merchant_id": grant_data.merchant_id,
            "actor_role": grant_data.role_hint
            }));
        }
    }

    // Check if we have a valid session
    if token.is_none() || session.is_none() {
        let landing = build_landing_payload("unregistered", &entry_intent, &business_window);
        return Ok(Json(json!({
            "status": "reopen_in_telegram_required",
            "message": "登录状态已失效，请返回 Telegram 重新打开",
            "reason": "bootstrap_auth_failed",
            "landing": landing,
            "renewal": {"rotated": false, "source": "none", "grant_expires_at": Value::Null},
            "ui_context": {
                "business_window": business_window,
                "entry_intent": entry_intent,
                "identity_label": "需要返回 Telegram"
            }
        })));
    }

    // Build landing page
    let session_data = session.as_ref().unwrap();
    let role = session_data
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("platform_admin");
    let landing = build_landing_payload(role, &entry_intent, &business_window);

    Ok(Json(json!({
        "status": "ok",
        "token": token,
        "session": session,
        "landing": landing,
        "renewal": {"rotated": !grant.is_empty(), "source": if !init_data.is_empty() { "init_data" } else if !otp.is_empty() { "otp" } else { "grant_proof" }, "grant_expires_at": Value::Null},
        "ui_context": {
            "business_window": business_window,
            "entry_intent": entry_intent,
            "identity_label": identity_label(role)
        }
    })))
}

// GET /api/miniapp/context - Get current session context for MiniApp
pub async fn context(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let business_window = get_business_window();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "session": {
                "role": scope.role.as_str(),
                "tenant_id": scope.tenant_id.unwrap_or_default(),
                "merchant_id": scope.merchant_id.map(|v| v.to_string()),
                "actor_role": scope.actor_role.as_str()
            },
            "business_window": business_window,
            "identity_label": identity_label(scope.role.as_str())
        }
    })))
}

// POST /api/miniapp/logout_revoke - Logout and revoke grant
pub async fn logout_revoke(
    State(state): State<AppState>,
    Json(body): Json<LogoutRevokeRequest>,
) -> ApiResult<Json<Value>> {
    // Revoke grant if provided
    if let Some(grant) = body.grant {
        if !grant.is_empty() {
            let grant_store = state.grant_store.clone();
            grant_store.remove(&grant).await;
        }
    }

    // In a full implementation, we would clear cookies here
    // But Axum responses don't directly support cookie deletion in the same way
    // The frontend should handle cookie clearing

    Ok(Json(json!({
        "status": "ok"
    })))
}
