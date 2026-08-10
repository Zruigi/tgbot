// MiniApp routes - Bootstrap, Context, Logout/Revoke
use axum::{extract::State, http::HeaderMap, Json};
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

fn identity_label(role: &str) -> &'static str {
    match role {
        "platform_admin" => "平台管理员",
        "merchant_owner" => "商户管理员",
        _ => "未注册用户",
    }
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
        // Authenticate via Telegram Mini App init_data (HMAC-verified).
        // This is the same protocol as /api/login/tma, but with the relaxed
        // bootstrap semantics from the Python source: an unknown Telegram user
        // resolves to role "unregistered" (guide to onboarding) instead of an
        // immediate 401.
        use crate::routes::auth::{
            build_data_check_string, compute_telegram_hash, get_bot_token_for_tma,
            parse_telegram_init_data,
        };
        let parsed = parse_telegram_init_data(&init_data);
        let received_hash = parsed.get("hash").cloned().unwrap_or_default();
        if !received_hash.is_empty() {
            // Resolve the signing bot token: tenant_hint/merchant_id_hint when
            // present (merchant-scoped), otherwise the configured bot token.
            let tenant_hint = body.tenant_hint.clone().unwrap_or_default();
            let merchant_id_hint = body.merchant_id_hint;
            match get_bot_token_for_tma(&state, &tenant_hint, merchant_id_hint).await {
                Ok(bot_token) => {
                    let data_check_string = build_data_check_string(&parsed);
                    let expected_hash = compute_telegram_hash(&data_check_string, &bot_token);
                    if expected_hash == received_hash {
                        // Extract Telegram user id from init_data.user
                        let user_json = parsed.get("user").cloned().unwrap_or_default();
                        let telegram_user_id = serde_json::from_str::<Value>(&user_json)
                            .ok()
                            .and_then(|u| u.get("id").and_then(Value::as_i64));
                        if let Some(tg_user_id) = telegram_user_id {
                            // auth_date expiry check (24h window, same as login_tma)
                            let auth_date = parsed
                                .get("auth_date")
                                .and_then(|v| v.parse::<i64>().ok());
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            let expired = auth_date.is_some_and(|ts| {
                                now.saturating_sub(ts) > 86400 || ts > now.saturating_add(300)
                            });
                            if !expired {
                                // Resolve role: platform_admin if in auth.users
                                // with platform_admin role; merchant_owner/admin/
                                // viewer if owner/member of the hinted tenant;
                                // otherwise unregistered.
                                let is_platform_admin: bool = sqlx::query_scalar(
                                    "SELECT EXISTS(
                                         SELECT 1
                                         FROM auth.users u
                                         JOIN auth.saas_user_roles r ON r.user_id = u.id
                                         WHERE u.tg_user_id = $1 AND r.role = 'platform_admin'
                                     )",
                                )
                                .bind(tg_user_id)
                                .fetch_one(state.db()?)
                                .await
                                .unwrap_or(false);

                                let (role, tenant_id, merchant_id) =
                                    if is_platform_admin {
                                        ("platform_admin".to_string(), String::new(), None)
                                    } else if !tenant_hint.trim().is_empty() {
                                        // Merchant-scoped lookup
                                        let member_role: Option<String> = sqlx::query_scalar(
                                            "SELECT CASE
                                                 WHEN m.owner_tg_user_id = $1 THEN 'merchant_owner'
                                                 ELSE tm.role
                                             END
                                             FROM merchants m
                                             LEFT JOIN auth.users u ON u.tg_user_id = $1
                                             LEFT JOIN tenant_memberships tm
                                               ON tm.user_id = u.id AND tm.tenant_id = m.tenant_id
                                             WHERE m.tenant_id = $2
                                               AND m.status IN ('active', 'active_free')
                                               AND (m.owner_tg_user_id = $1 OR tm.user_id IS NOT NULL)
                                             ORDER BY CASE WHEN m.owner_tg_user_id = $1 THEN 0 ELSE 1 END
                                             LIMIT 1",
                                        )
                                        .bind(tg_user_id)
                                        .bind(tenant_hint.trim())
                                        .fetch_optional(state.db()?)
                                        .await
                                        .ok()
                                        .flatten();

                                        match member_role.as_deref() {
                                            Some(r @ ("merchant_owner" | "merchant_admin"
                                            | "merchant_viewer")) => (
                                                r.to_string(),
                                                tenant_hint.trim().to_string(),
                                                merchant_id_hint,
                                            ),
                                            _ => (
                                                "unregistered".to_string(),
                                                String::new(),
                                                None,
                                            ),
                                        }
                                    } else {
                                        // No tenant hint and not platform admin:
                                        // look up any merchant owned by this user.
                                        let owned: Option<String> = sqlx::query_scalar(
                                            "SELECT tenant_id FROM merchants
                                             WHERE owner_tg_user_id = $1
                                               AND status IN ('active', 'active_free')
                                             LIMIT 1",
                                        )
                                        .bind(tg_user_id)
                                        .fetch_optional(state.db()?)
                                        .await
                                        .ok()
                                        .flatten();

                                        match owned {
                                            Some(owned_tenant) => (
                                                "merchant_owner".to_string(),
                                                owned_tenant,
                                                merchant_id_hint,
                                            ),
                                            None => (
                                                "unregistered".to_string(),
                                                String::new(),
                                                None,
                                            ),
                                        }
                                    };

                                if role != "unregistered" {
                                    let session_token = crate::routes::auth::issue_session_token(
                                        &state.config.session_secret,
                                        &role,
                                        &tenant_id,
                                        merchant_id,
                                        Some(&user_json),
                                        24 * 3600,
                                    )
                                    .ok_or_else(|| {
                                        ApiError::ServiceUnavailable(
                                            "session signing unavailable".into(),
                                        )
                                    })?;
                                    token = Some(session_token.clone());
                                    session = Some(json!({
                                        "role": role,
                                        "tenant_id": tenant_id,
                                        "merchant_id": merchant_id,
                                        "actor_role": role
                                    }));
                                } else {
                                    // Unregistered: signal onboarding with
                                    // status ok + session.role=unregistered
                                    // (Python bootstrap semantics).
                                    let landing = build_landing_payload(
                                        "unregistered",
                                        &entry_intent,
                                        &business_window,
                                    );
                                    return Ok(Json(json!({
                                        "status": "ok",
                                        "token": Value::Null,
                                        "session": {
                                            "role": "unregistered",
                                            "tenant_id": "",
                                            "merchant_id": Value::Null,
                                            "actor_role": "unregistered",
                                        },
                                        "landing": landing,
                                        "renewal": {"rotated": false, "source": "init_data", "grant_expires_at": Value::Null},
                                        "ui_context": {
                                            "business_window": business_window,
                                            "entry_intent": entry_intent,
                                            "identity_label": "未注册"
                                        }
                                    })));
                                }
                            }
                        }
                    } else {
                        tracing::warn!("MiniApp bootstrap TMA HMAC verification failed");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "MiniApp bootstrap bot token resolution failed");
                }
            }
        }
    } else if !otp.is_empty() {
        // Authenticate via OTP
        let otp_store = state.otp_store.clone();
        let now = chrono::Utc::now().timestamp() as f64;

        if let Some(otp_data) = otp_store.get(&otp).await {
            if otp_data.expire_at > now {
                otp_store.remove(&otp).await;
                let session_token = crate::routes::auth::issue_session_token(
                    &state.config.session_secret,
                    &otp_data.role,
                    &otp_data.tenant_id,
                    otp_data.merchant_id,
                    otp_data.issued_for.as_deref(),
                    24 * 3600,
                )
                .ok_or_else(|| {
                    ApiError::ServiceUnavailable("session signing unavailable".into())
                })?;
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
                &state.config.session_secret,
                &grant_data.role_hint,
                &grant_data.tenant_id,
                grant_data.merchant_id,
                Some(&grant_data.issued_for),
                24 * 3600,
            )
            .ok_or_else(|| ApiError::ServiceUnavailable("session signing unavailable".into()))?;
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
