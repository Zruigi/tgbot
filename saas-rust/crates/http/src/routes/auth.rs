// Auth routes - Login, OTP, CSRF Token, TMA login
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::secret_crypto::SecretCipher;
use crate::state::AppState;
use tgbot_domain::error::{ApiError, ApiResult};

/// In-memory OTP store: {otp: {token, expire_at, role, tenant_id, merchant_id, issued_for}}
#[derive(Clone)]
pub struct OtpStore(Arc<RwLock<HashMap<String, OtpData>>>);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OtpData {
    pub token: String,
    pub expire_at: f64,
    pub role: String,
    pub tenant_id: String,
    pub merchant_id: Option<i64>,
    pub issued_for: Option<String>,
}

/// In-memory CSRF token store: {csrf_token: expire_timestamp}
#[derive(Clone)]
pub struct CsrfStore(Arc<RwLock<HashMap<String, f64>>>);

impl OtpStore {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    pub async fn insert(&self, otp: String, data: OtpData) {
        let mut store = self.0.write().await;
        store.insert(otp, data);
    }

    pub async fn get(&self, otp: &str) -> Option<OtpData> {
        let store = self.0.read().await;
        store.get(otp).cloned()
    }

    pub async fn remove(&self, otp: &str) -> Option<OtpData> {
        let mut store = self.0.write().await;
        store.remove(otp)
    }

    pub async fn cleanup(&self) {
        let now = chrono::Utc::now().timestamp() as f64;
        let mut store = self.0.write().await;
        store.retain(|_, v| v.expire_at > now);
    }
}

impl Default for OtpStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CsrfStore {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    pub async fn generate(&self) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as f64;

        // Cleanup expired tokens
        {
            let mut store = self.0.write().await;
            store.retain(|_, &mut v| v > now);
        }

        // Generate new token
        let token = generate_secure_token(32);
        let mut store = self.0.write().await;
        store.insert(token.clone(), now + 86400.0); // 24 hours TTL
        token
    }

    pub async fn get(&self, token: &str) -> bool {
        let now = chrono::Utc::now().timestamp() as f64;
        let store = self.0.read().await;
        store
            .get(token)
            .map(|&expire| expire > now)
            .unwrap_or(false)
    }
}

impl Default for CsrfStore {
    fn default() -> Self {
        Self::new()
    }
}

fn generate_secure_token(length: usize) -> String {
    use std::iter;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;

    iter::repeat_with(|| {
        rng_seed = rng_seed.wrapping_mul(1103515245).wrapping_add(12345);
        let idx = (rng_seed / 65536) as usize % CHARSET.len();
        CHARSET[idx] as char
    })
    .take(length)
    .collect()
}

fn build_session_token_payload(
    role: &str,
    tenant_id: &str,
    merchant_id: Option<i64>,
    issued_for: Option<&str>,
) -> Value {
    let issued_at = chrono::Utc::now().timestamp();
    json!({
        "role": role,
        "actor_role": role,
        "tenant_id": tenant_id,
        "merchant_id": merchant_id,
        "issued_for": issued_for.unwrap_or(""),
        "iat": issued_at,
        "exp": issued_at + 24 * 60 * 60
    })
}

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;

// Note: Exactly 32 bytes required for HMAC-SHA256 key
type HmacSha256 = Hmac<Sha256>;
const SESSION_SECRET_KEY: &[u8; 32] = b"01234567890123456789012345678901";

// ============== FIXED: 使用真正的 HMAC-SHA256 ==============
pub(crate) fn issue_session_token(
    role: &str,
    tenant_id: &str,
    merchant_id: Option<i64>,
    issued_for: Option<&str>,
) -> String {
    let payload = build_session_token_payload(role, tenant_id, merchant_id, issued_for);
    let payload_json = serde_json::to_string(&payload).unwrap_or_default();

    // 使用标准 base64 编码
    let payload_b64 = BASE64.encode(payload_json.as_bytes());

    // 使用真正的 HMAC-SHA256 进行签名
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(SESSION_SECRET_KEY).expect("HMAC can accept key");
    mac.update(payload_b64.as_bytes());
    let signature = BASE64.encode(mac.finalize().into_bytes());

    format!("{}.{}", payload_b64, signature)
}

// ============== FIXED: 使用真正的 HMAC-SHA256 验证 ==============
pub(crate) fn verify_session_token(token: &str) -> Option<Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return None;
    }

    let payload_b64 = parts[0];
    let signature_b64 = parts[1];

    // 验证 HMAC-SHA256 签名
    let mut mac =
        <HmacSha256 as hmac::Mac>::new_from_slice(SESSION_SECRET_KEY).expect("HMAC can accept key");
    mac.update(payload_b64.as_bytes());
    let expected_sig = BASE64.encode(mac.finalize().into_bytes());

    // 使用恒定时间比较防止时序攻击
    if !constant_time_compare(signature_b64, &expected_sig) {
        return None;
    }

    // Decode payload
    let decoded = BASE64.decode(payload_b64).ok()?;
    let json_str = String::from_utf8(decoded).ok()?;
    let payload: Value = serde_json::from_str(&json_str).ok()?;
    let exp = payload.get("exp").and_then(Value::as_i64)?;
    if exp <= chrono::Utc::now().timestamp() {
        return None;
    }
    Some(payload)
}

// 恒定时间比较 - 防止时序攻击
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut result = 0u8;
    for i in 0..a_bytes.len() {
        result |= a_bytes[i] ^ b_bytes[i];
    }
    result == 0
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OtpLoginRequest {
    pub otp: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TmaLoginRequest {
    #[serde(rename = "init_data")]
    pub init_data: Option<String>,
    #[serde(rename = "client_context")]
    pub client_context: Option<Value>,
    #[serde(rename = "tenant_hint")]
    pub tenant_hint: Option<String>,
    #[serde(rename = "merchant_id_hint")]
    pub merchant_id_hint: Option<i64>,
    pub grant: Option<String>,
}

// POST /api/login - Token login, returns OTP and CSRF token
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> ApiResult<Json<Value>> {
    let token = body.token.unwrap_or_default();

    // Check against WEB_ADMIN_TOKEN or BOT_TOKEN
    let valid = !token.is_empty()
        && (token == state.config.web_admin_token || token == state.config.bot_token);

    if !valid {
        return Err(ApiError::Unauthorized);
    }

    // Generate OTP (for compatibility, we issue session token directly)
    let otp = generate_secure_token(12);
    let now = chrono::Utc::now().timestamp() as f64;
    let expire_at = now + 300.0; // 5 minutes TTL

    // Store OTP with session token
    let session_token = issue_session_token("platform_admin", "", None, Some("admin"));
    let otp_data = OtpData {
        token: session_token,
        expire_at,
        role: "platform_admin".to_string(),
        tenant_id: "".to_string(),
        merchant_id: None,
        issued_for: Some("admin".to_string()),
    };

    let otp_store = state.otp_store.clone();
    otp_store.insert(otp.clone(), otp_data).await;

    // Generate CSRF token
    let csrf_store = state.csrf_store.clone();
    let csrf_token = csrf_store.generate().await;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "otp": otp,
            "csrf_token": csrf_token
        }
    })))
}

// POST /api/login/otp - OTP login
pub async fn login_otp(
    State(state): State<AppState>,
    Json(body): Json<OtpLoginRequest>,
) -> ApiResult<Json<Value>> {
    let otp = body.otp.unwrap_or_default();

    if otp.is_empty() {
        return Err(ApiError::BadRequest("otp is required".to_string()));
    }

    let otp_store = state.otp_store.clone();
    let now = chrono::Utc::now().timestamp() as f64;

    // Check OTP store
    if let Some(otp_data) = otp_store.get(&otp).await {
        if otp_data.expire_at > now {
            // OTP valid, consume it
            otp_store.remove(&otp).await;

            // Session token is already in OTP data
            let session_token = &otp_data.token;

            // Generate CSRF token
            let csrf_store = state.csrf_store.clone();
            let csrf_token = csrf_store.generate().await;

            return Ok(Json(json!({
                "status": "ok",
                "data": {
                    "token": session_token,
                    "csrf_token": csrf_token,
                    "session": {
                        "role": otp_data.role,
                        "tenant_id": otp_data.tenant_id,
                        "merchant_id": otp_data.merchant_id,
                        "actor_role": otp_data.role,
                        "issued_for": otp_data.issued_for
                    }
                }
            })));
        }
    }

    // Check database for persisted OTP (future: integrate with database)
    // For now, reject invalid/expired OTP
    Err(ApiError::Unauthorized)
}

// POST /api/login/tma - Telegram Mini App login
pub async fn login_tma(
    State(state): State<AppState>,
    Json(body): Json<TmaLoginRequest>,
) -> ApiResult<Json<Value>> {
    let init_data = body.init_data.clone().unwrap_or_default();
    let _client_context = body.client_context.clone();
    let tenant_hint = body.tenant_hint.clone().unwrap_or_default();
    let merchant_id_hint = body.merchant_id_hint;
    let _grant = body.grant.clone();

    if init_data.is_empty() {
        return Err(ApiError::BadRequest("init_data is required".to_string()));
    }

    // ============== FIXED: 验证 Telegram init_data HMAC 签名 ==============
    // Telegram init_data format: query_id=xxx&user=xxx&auth_date=xxx&hash=xxx
    // hash = HMAC-SHA256(data, bot_token)
    let parsed = parse_telegram_init_data(&init_data);

    let hash = parsed
        .get("hash")
        .ok_or_else(|| ApiError::BadRequest("init_data missing hash".to_string()))?;

    // 从数据库获取 bot_token (简化版 - 实际应从商户配置获取)
    let bot_token = get_bot_token_for_tma(&state, &tenant_hint, merchant_id_hint).await?;

    // 验证 HMAC-SHA256 签名
    let data_check_string = build_data_check_string(&parsed);
    let expected_hash = compute_telegram_hash(&data_check_string, &bot_token);

    if !constant_time_compare(hash, &expected_hash) {
        tracing::warn!("TMA login failed: invalid HMAC signature");
        return Err(ApiError::Unauthorized);
    }

    // 从 init_data 提取用户信息。Telegram 的 user.id 是后续租户归属
    // 校验的唯一可信用户标识；没有它不能把一个已签名的 payload 映射成
    // merchant_owner 或 platform_admin。
    let auth_date = parsed.get("auth_date").and_then(|v| v.parse::<i64>().ok());
    let user_json: Option<String> = match parsed.get("user") {
        Some(v) => Some(v.to_string()),
        None => None,
    };
    let user_json_ref: Option<&str> = user_json.as_deref();
    let telegram_user_id = user_json_ref
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|user| user.get("id").and_then(Value::as_i64))
        .ok_or(ApiError::Unauthorized)?;

    // 检查 auth_date 是否在合理范围内 (24小时内)
    if let Some(ts) = auth_date {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if now.saturating_sub(ts) > 86400 || ts > now.saturating_add(300) {
            return Err(ApiError::BadRequest("init_data expired".to_string()));
        }
    }

    // tenant_hint 是客户端输入，不能单独决定会话角色。平台会话必须对应
    // 已登记的 platform_admin；商户会话必须对应该租户的 owner/member。
    let (role, tenant_id) = if tenant_hint.trim().is_empty() {
        let is_platform_admin: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1
                 FROM auth.users u
                 JOIN auth.saas_user_roles r ON r.user_id = u.id
                 WHERE u.tg_user_id = $1 AND r.role = 'platform_admin'
             )",
        )
        .bind(telegram_user_id)
        .fetch_one(state.db()?)
        .await?;
        if !is_platform_admin {
            return Err(ApiError::Unauthorized);
        }
        ("platform_admin".to_string(), String::new())
    } else {
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
        .bind(telegram_user_id)
        .bind(tenant_hint.trim())
        .fetch_optional(state.db()?)
        .await?;

        let role = match member_role.as_deref() {
            Some("merchant_owner") | Some("merchant_admin") | Some("merchant_viewer") => {
                member_role.unwrap()
            }
            _ => return Err(ApiError::Unauthorized),
        };
        (role, tenant_hint.trim().to_string())
    };

    let session_token = issue_session_token(&role, &tenant_id, merchant_id_hint, user_json_ref);

    // Generate CSRF token
    let csrf_store = state.csrf_store.clone();
    let csrf_token = csrf_store.generate().await;

    Ok(Json(json!({
        "status": "ok",
        "token": session_token,
        "csrf_token": csrf_token,
        "session": {
            "role": role,
            "tenant_id": tenant_id,
            "merchant_id": merchant_id_hint,
            "issued_for": user_json
        }
    })))
}

// 解析 Telegram init_data 为 key-value 对
fn parse_telegram_init_data(init_data: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in init_data.split('&') {
        let parts: Vec<&str> = pair.splitn(2, '=').collect();
        if parts.len() == 2 {
            map.insert(urlencoding_decode(parts[0]), urlencoding_decode(parts[1]));
        }
    }
    map
}

// URL 解码
fn urlencoding_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

// 构建 data_check_string (用于计算 hash)
fn build_data_check_string(data: &std::collections::HashMap<String, String>) -> String {
    let mut pairs: Vec<String> = data
        .iter()
        .filter(|(k, _)| *k != "hash")
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    pairs.sort();
    pairs.join("\n")
}

// 计算 Telegram HMAC-SHA256 hash
fn compute_telegram_hash(data_check_string: &str, bot_token: &str) -> String {
    // Telegram Web Apps derive a per-bot secret first, then sign the sorted
    // data-check string with that secret. Using the raw bot token here would
    // reject valid Mini App payloads and is not the Telegram protocol.
    let mut secret_mac =
        <HmacSha256 as hmac::Mac>::new_from_slice(b"WebAppData").expect("HMAC can accept key");
    secret_mac.update(bot_token.as_bytes());
    let secret = secret_mac.finalize().into_bytes();

    let mut mac =
        <HmacSha256 as hmac::Mac>::new_from_slice(&secret).expect("HMAC can accept derived key");
    mac.update(data_check_string.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

// 从数据库获取 TMA 登录用的 bot_token
async fn get_bot_token_for_tma(
    state: &AppState,
    tenant_hint: &str,
    merchant_id_hint: Option<i64>,
) -> ApiResult<String> {
    let tenant_id = tenant_hint.trim();
    if tenant_id.is_empty() {
        // 平台登录可以使用专门的 TMA token；缺失时必须拒绝，不能回退到
        // 可预测的开发 token。
        let token = std::env::var("TMA_BOT_TOKEN")
            .or_else(|_| std::env::var("BOT_TOKEN"))
            .unwrap_or_else(|_| state.config.bot_token.clone());
        if token.trim().is_empty() {
            tracing::warn!("No bot_token configured for TMA login");
            return Err(ApiError::Unauthorized);
        }
        return Ok(token);
    }

    // 商户 token 必须从该 tenant 的加密 secret ref 解密；禁止使用全局
    // BOT_TOKEN，否则任意 tenant_hint 都可能借用另一租户的签名密钥。
    let row = sqlx::query(
        "SELECT secrets.secret_ref
         FROM merchants m
         JOIN tenant_secret_refs secrets
           ON secrets.tenant_id = m.tenant_id
          AND secrets.secret_key = 'TELEGRAM_BOT_TOKEN'
         WHERE m.tenant_id = $1
           AND ($2::bigint IS NULL OR m.legacy_merchant_id = $2)
           AND m.status IN ('active', 'active_free')",
    )
    .bind(tenant_id)
    .bind(merchant_id_hint)
    .fetch_optional(state.db()?)
    .await?
    .ok_or(ApiError::Unauthorized)?;
    let secret_ref: String = row.try_get("secret_ref")?;
    let cipher = SecretCipher::from_encoded_key(&state.config.secret_encryption_key)
        .map_err(ApiError::BadRequest)?;
    let token = cipher
        .decrypt(tenant_id, "TELEGRAM_BOT_TOKEN", &secret_ref)
        .map_err(ApiError::BadRequest)?;
    if token.trim().is_empty() {
        return Err(ApiError::Unauthorized);
    }
    Ok(token)
}

// GET /api/csrf-token - Get CSRF token
pub async fn csrf_token(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let csrf_store = state.csrf_store.clone();
    let csrf_token = csrf_store.generate().await;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "csrf_token": csrf_token
        }
    })))
}

// Internal function to generate OTP (used by admin)
pub async fn generate_otp_internal(
    state: &AppState,
    role: String,
    tenant_id: String,
    merchant_id: Option<i64>,
    issued_for: Option<String>,
    ttl_seconds: i64,
) -> String {
    let otp = generate_secure_token(12);
    let now = chrono::Utc::now().timestamp() as f64;
    let expire_at = now + ttl_seconds as f64;

    let session_token = issue_session_token(&role, &tenant_id, merchant_id, issued_for.as_deref());

    let otp_data = OtpData {
        token: session_token,
        expire_at,
        role: role.clone(),
        tenant_id,
        merchant_id,
        issued_for,
    };

    let otp_store = state.otp_store.clone();
    otp_store.insert(otp.clone(), otp_data).await;

    otp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_web_app_hash_uses_web_app_data_derived_secret() {
        let data = "auth_date=1700000000\nquery_id=AAH\nuser={\"id\":42}";
        assert_eq!(
            compute_telegram_hash(data, "123456:ABCdef"),
            "183ee3a9badd18521e50eba6626fc62a3323833eb552589b9d6817a9068dfbc2"
        );
    }
}
