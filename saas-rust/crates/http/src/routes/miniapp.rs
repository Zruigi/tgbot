// MiniApp routes - Bootstrap, Context, Logout/Revoke
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

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
            return ("merchant_owner".to_string(), "merchant_console_entry".to_string());
        }
        if is_platform_admin {
            return ("platform_admin".to_string(), "merchant_entry_fallback_platform".to_string());
        }
    }

    if normalized_intent == "admin_console" {
        if is_platform_admin {
            return ("platform_admin".to_string(), "admin_console_entry".to_string());
        }
        if has_merchant {
            return ("merchant_owner".to_string(), "admin_entry_fallback_merchant".to_string());
        }
    }

    if is_platform_admin && has_merchant {
        return (normalized_last.clone(), "dual_identity_last_scope".to_string());
    }
    if is_platform_admin {
        return ("platform_admin".to_string(), "platform_identity".to_string());
    }
    if has_merchant {
        return ("merchant_owner".to_string(), "merchant_identity".to_string());
    }
    ("unregistered".to_string(), "unregistered".to_string())
}

fn build_landing_payload(role: &str, entry_intent: &str, business_window: &BusinessWindow) -> Value {
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
    // Default business window (9:00 - 22:00)
    BusinessWindow {
        timezone: "Asia/Shanghai".to_string(),
        start: "09:00".to_string(),
        end: "22:00".to_string(),
        active: true, // Simplified - would check actual time
        now: chrono::Utc::now().to_rfc3339(),
    }
}

// POST /api/miniapp/bootstrap - Bootstrap MiniApp session
pub async fn bootstrap(
    State(state): State<AppState>,
    Json(body): Json<BootstrapRequest>,
) -> ApiResult<Json<Value>> {
    let entry_intent = normalize_entry_intent(body.entry_intent.as_deref());
    let client_context = body.client_context.unwrap_or(json!({}));

    // Extract hints from request
    let tenant_hint = body.tenant_hint.unwrap_or_default();
    let merchant_id_hint = body.merchant_id_hint;
    let grant = body.grant.unwrap_or_default();
    let init_data = body.init_data.unwrap_or_default();
    let otp = body.otp.unwrap_or_default();

    let business_window = get_business_window();

    // Try to authenticate via init_data (TMA), OTP, or grant
    let mut token: Option<String> = None;
    let mut session: Option<Value> = None;
    let mut proof_value: Option<String> = None;

    if !init_data.is_empty() {
        // Authenticate via TMA init_data
        // Simplified: extract user from init_data (full impl would verify HMAC)
        let (role, reason) = resolve_identity_scope(&entry_intent, true, false, "platform_admin");

        if role == "unregistered" {
            let landing = build_landing_payload(&role, &entry_intent, &business_window);
            return Ok(Json(json!({
                "status": "ok",
                "token": Value::Null,
                "session": {
                    "role": "unregistered",
                    "tenant_id": "",
                    "merchant_id": Value::Null,
                    "actor_role": "unregistered"
                },
                "landing": landing,
                "renewal": {"rotated": false, "source": "init_data", "grant_expires_at": Value::Null},
                "ui_context": {
                    "business_window": business_window,
                    "entry_intent": entry_intent,
                    "identity_label": identity_label("unregistered")
                }
            })));
        }

        // Issue session token
        let session_token = issue_miniapp_token(&role, "", None, Some("tma_user"));
        token = Some(session_token.clone());
        session = Some(json!({
            "role": role,
            "tenant_id": "",
            "merchant_id": Value::Null,
            "actor_role": role
        }));
    } else if !otp.is_empty() {
        // Authenticate via OTP
        let otp_store = state.otp_store.clone();
        let now = chrono::Utc::now().timestamp() as f64;

        if let Some(otp_data) = otp_store.get(&otp).await {
            if otp_data.expire_at > now {
                otp_store.remove(&otp).await;
                let session_token = issue_miniapp_token(
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
            let session_token = issue_miniapp_token(
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
            proof_value = Some(grant_data.proof_salt);
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
    let role = session_data.get("role").and_then(|v| v.as_str()).unwrap_or("platform_admin");
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
pub async fn context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
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

fn issue_miniapp_token(role: &str, tenant_id: &str, merchant_id: Option<i64>, issued_for: Option<&str>) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let payload = json!({
        "role": role,
        "actor_role": role,
        "tenant_id": tenant_id,
        "merchant_id": merchant_id,
        "issued_for": issued_for.unwrap_or(""),
        "iat": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    });

    let payload_json = serde_json::to_string(&payload).unwrap_or_default();
    let payload_b64 = base64_encode_miniapp(payload_json.as_bytes());

    // Simple signature
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    payload_b64.hash(&mut hasher);
    let signature = format!("{:016x}", hasher.finish());

    format!("{}.{}", payload_b64, signature)
}

fn base64_encode_miniapp(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }

    result
}