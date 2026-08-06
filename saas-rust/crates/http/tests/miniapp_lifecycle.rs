//! Disposable Mini App lifecycle coverage.
//!
//! The test is intentionally ignored by default because it exercises the
//! local PostgreSQL fixture, not a production database.  Run it with
//! `BETA_MINIAPP_TEST_DATABASE_URL` pointed at the disposable Beta database.

use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, HeaderValue},
    Json,
};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};
use tgbot_domain::{config::Config, error::ApiError, worker_telemetry::WorkerTelemetry};
use tgbot_http::{
    routes::{
        auth::{login_tma, TmaLoginRequest},
        miniapp::{
            bootstrap, context, logout_revoke, BootstrapRequest, GrantData, LogoutRevokeRequest,
        },
        telegram_webhook::BotRegistry,
    },
    secret_crypto::SecretCipher,
    state::AppState,
};
use tgbot_integrations::{
    litellm::{LiteLLMClient, LiteLLMConfig},
    supabase_rest::SupabaseRestClient,
};
use uuid::Uuid;

const FIXTURE_BOT_TOKEN: &str = "999000:miniapp-fixture-token";
const FIXTURE_SESSION_SECRET: &str = "miniapp-session-secret-012345678901234567";
const FIXTURE_ENCRYPTION_KEY: &str =
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

type HmacSha256 = Hmac<Sha256>;

fn fixture_config(database_url: &str) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".to_string(),
        database_url: database_url.to_string(),
        database_max_connections: 4,
        allow_rest_only: false,
        supabase_url: "http://127.0.0.1:1".to_string(),
        supabase_service_role_key: "disposable-fixture".to_string(),
        supabase_jwt_secret: String::new(),
        internal_saas_secret: String::new(),
        session_secret: FIXTURE_SESSION_SECRET.to_string(),
        secret_encryption_key: FIXTURE_ENCRYPTION_KEY.to_string(),
        web_admin_token: String::new(),
        bot_token: String::new(),
        telegram_auto_replies_enabled: false,
        telegram_commands_enabled: false,
        telegram_wallet_monitors_enabled: false,
        cors_allowed_origin: "*".to_string(),
    }
}

fn fixture_state(database_url: &str, pool: PgPool) -> AppState {
    let config = fixture_config(database_url);
    let llm = LiteLLMClient::new(LiteLLMConfig {
        base_url: "http://127.0.0.1:1".to_string(),
        api_key: "disposable-fixture".to_string(),
        default_model: "fixture-model".to_string(),
        timeout_secs: 1,
    })
    .expect("fixture LiteLLM client should build");

    AppState {
        config,
        pool: Some(pool),
        supabase: SupabaseRestClient::new("http://127.0.0.1:1", "disposable-fixture"),
        llm,
        otp_store: tgbot_http::routes::auth::OtpStore::new(),
        csrf_store: tgbot_http::routes::auth::CsrfStore::new(),
        grant_store: tgbot_http::routes::miniapp::GrantStore::new(),
        bot_registry: BotRegistry::new(),
        worker_enabled: false,
        worker_telemetry: WorkerTelemetry::default(),
        update_dispatcher: None,
    }
}

async fn fixture_pool() -> Result<(String, PgPool), Box<dyn Error>> {
    let database_url = std::env::var("BETA_MINIAPP_TEST_DATABASE_URL").map_err(|_| {
        "BETA_MINIAPP_TEST_DATABASE_URL is required; point it at the disposable local Beta database"
    })?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;
    Ok((database_url, pool))
}

fn fixture_ids() -> (Uuid, Uuid, String, i64, i64) {
    let user_id = Uuid::new_v4();
    let merchant_id = Uuid::new_v4();
    let tenant_id = format!("miniapp-fixture-{}", merchant_id.simple());
    let bytes = merchant_id.as_bytes();
    let legacy_id = (u64::from_be_bytes(bytes[..8].try_into().expect("UUID prefix"))
        % 1_000_000_000)
        .saturating_add(9_000_000_000) as i64;
    let tg_user_id = (u64::from_be_bytes(bytes[8..].try_into().expect("UUID suffix"))
        & 0x3fff_ffff_ffff_ffff) as i64;
    (user_id, merchant_id, tenant_id, legacy_id, tg_user_id)
}

async fn insert_fixture(
    pool: &PgPool,
    user_id: Uuid,
    merchant_id: Uuid,
    tenant_id: &str,
    legacy_id: i64,
    tg_user_id: i64,
) -> Result<(), Box<dyn Error>> {
    let cipher = SecretCipher::from_encoded_key(FIXTURE_ENCRYPTION_KEY)
        .map_err(|error| format!("fixture cipher: {error}"))?;
    let secret_ref = cipher
        .encrypt(tenant_id, "TELEGRAM_BOT_TOKEN", FIXTURE_BOT_TOKEN)
        .map_err(|error| format!("fixture secret: {error}"))?;

    sqlx::query(
        "INSERT INTO auth.users (id, tg_user_id, username, first_name)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(tg_user_id)
    .bind("miniapp_fixture")
    .bind("Mini App Fixture")
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO merchants
           (id, legacy_merchant_id, tenant_id, owner_tg_user_id, merchant_name, status)
         VALUES ($1, $2, $3, $4, $5, 'active')",
    )
    .bind(merchant_id)
    .bind(legacy_id)
    .bind(tenant_id)
    .bind(tg_user_id)
    .bind("Mini App Fixture Merchant")
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO tenant_memberships (user_id, tenant_id, merchant_id, role)
         VALUES ($1, $2, $3, 'merchant_owner')",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(merchant_id)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO tenant_secret_refs (tenant_id, secret_key, secret_ref)
         VALUES ($1, 'TELEGRAM_BOT_TOKEN', $2)",
    )
    .bind(tenant_id)
    .bind(secret_ref)
    .execute(pool)
    .await?;

    Ok(())
}

async fn remove_fixture(
    pool: &PgPool,
    user_id: Uuid,
    merchant_id: Uuid,
    tenant_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM tenant_memberships WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM tenant_secret_refs WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM merchants WHERE id = $1")
        .bind(merchant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM auth.users WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

fn percent_encode_ascii(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn telegram_hash(data_check_string: &str, bot_token: &str) -> String {
    let mut secret_mac = HmacSha256::new_from_slice(b"WebAppData").expect("valid HMAC key");
    secret_mac.update(bot_token.as_bytes());
    let secret = secret_mac.finalize().into_bytes();

    let mut data_mac = HmacSha256::new_from_slice(&secret).expect("valid HMAC key");
    data_mac.update(data_check_string.as_bytes());
    hex::encode(data_mac.finalize().into_bytes())
}

fn build_init_data(tg_user_id: i64) -> String {
    let auth_date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    let query_id = "miniapp-fixture-query";
    let user_json = format!(r#"{{"id":{tg_user_id},"first_name":"Mini App"}}"#);
    let data_check_string = format!("auth_date={auth_date}\nquery_id={query_id}\nuser={user_json}");
    let hash = telegram_hash(&data_check_string, FIXTURE_BOT_TOKEN);
    format!(
        "auth_date={auth_date}&query_id={query_id}&user={}&hash={hash}",
        percent_encode_ascii(&user_json)
    )
}

fn bearer_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))
            .expect("fixture session token is a valid header value"),
    );
    headers
}

fn json_body(result: Result<Json<Value>, ApiError>) -> Value {
    result.expect("Mini App route should succeed").0
}

#[tokio::test]
#[ignore = "requires an explicit disposable PostgreSQL fixture"]
async fn tma_hmac_session_context_and_grant_logout_lifecycle() -> Result<(), Box<dyn Error>> {
    let (database_url, pool) = fixture_pool().await?;
    let (user_id, merchant_id, tenant_id, legacy_id, tg_user_id) = fixture_ids();
    insert_fixture(
        &pool,
        user_id,
        merchant_id,
        &tenant_id,
        legacy_id,
        tg_user_id,
    )
    .await?;

    let state = fixture_state(&database_url, pool.clone());
    let init_data = build_init_data(tg_user_id);

    // The exact Telegram Web App HMAC must be accepted; a one-byte mutation
    // must be rejected before any session is issued.
    let mut invalid_init_data = init_data.clone();
    invalid_init_data.push('0');
    let invalid = login_tma(
        State(state.clone()),
        Json(TmaLoginRequest {
            init_data: Some(invalid_init_data),
            client_context: None,
            tenant_hint: Some(tenant_id.clone()),
            merchant_id_hint: Some(legacy_id),
            grant: None,
        }),
    )
    .await
    .expect_err("mutated init_data must fail HMAC validation");
    assert!(matches!(invalid, ApiError::Unauthorized));

    let login = json_body(
        login_tma(
            State(state.clone()),
            Json(TmaLoginRequest {
                init_data: Some(init_data.clone()),
                client_context: None,
                tenant_hint: Some(tenant_id.clone()),
                merchant_id_hint: Some(legacy_id),
                grant: None,
            }),
        )
        .await,
    );
    assert_eq!(login["status"], "ok");
    assert_eq!(login["session"]["role"], "merchant_owner");
    let login_token = login["token"].as_str().expect("login token");

    let login_context = json_body(context(State(state.clone()), bearer_headers(login_token)).await);
    assert_eq!(login_context["status"], "ok");
    assert_eq!(login_context["data"]["session"]["role"], "merchant_owner");
    assert_eq!(login_context["data"]["session"]["tenant_id"], tenant_id);

    // Bootstrap intentionally does not trust init_data directly.  The TMA
    // HMAC endpoint above is the authenticated entry point; bootstrap accepts
    // the resulting short-lived grant/session path below.
    let init_bootstrap = json_body(
        bootstrap(
            State(state.clone()),
            Json(BootstrapRequest {
                init_data: Some(init_data),
                otp: None,
                grant: None,
                entry_intent: Some("merchant_console".to_string()),
                client_context: None,
                tenant_hint: Some(tenant_id.clone()),
                merchant_id_hint: Some(legacy_id),
            }),
        )
        .await,
    );
    assert_eq!(init_bootstrap["status"], "reopen_in_telegram_required");
    assert_eq!(init_bootstrap["reason"], "tma_verification_required");

    let grant = format!("miniapp-fixture-grant-{}", merchant_id.simple());
    state
        .grant_store
        .insert(
            grant.clone(),
            GrantData {
                grant_hash: "fixture-grant-hash".to_string(),
                issued_for: tg_user_id.to_string(),
                role_hint: "merchant_owner".to_string(),
                tenant_id: tenant_id.clone(),
                merchant_id: Some(legacy_id),
                entry_intent: "merchant_console".to_string(),
                expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
                ua_fingerprint: "fixture-ua".to_string(),
                proof_salt: "fixture-proof-salt".to_string(),
            },
        )
        .await;

    let grant_bootstrap = json_body(
        bootstrap(
            State(state.clone()),
            Json(BootstrapRequest {
                init_data: None,
                otp: None,
                grant: Some(grant.clone()),
                entry_intent: Some("merchant_console".to_string()),
                client_context: None,
                tenant_hint: None,
                merchant_id_hint: None,
            }),
        )
        .await,
    );
    assert_eq!(grant_bootstrap["status"], "ok");
    assert_eq!(grant_bootstrap["renewal"]["rotated"], true);
    assert_eq!(grant_bootstrap["renewal"]["source"], "grant_proof");
    let grant_token = grant_bootstrap["token"].as_str().expect("grant token");
    let grant_context = json_body(context(State(state.clone()), bearer_headers(grant_token)).await);
    assert_eq!(grant_context["data"]["session"]["role"], "merchant_owner");
    assert_eq!(grant_context["data"]["session"]["tenant_id"], tenant_id);

    let logout = json_body(
        logout_revoke(
            State(state.clone()),
            Json(LogoutRevokeRequest {
                grant: Some(grant.clone()),
            }),
        )
        .await,
    );
    assert_eq!(logout["status"], "ok");
    assert!(state.grant_store.get(&grant).await.is_none());

    let revoked_bootstrap = json_body(
        bootstrap(
            State(state.clone()),
            Json(BootstrapRequest {
                init_data: None,
                otp: None,
                grant: Some(grant),
                entry_intent: None,
                client_context: None,
                tenant_hint: None,
                merchant_id_hint: None,
            }),
        )
        .await,
    );
    assert_eq!(revoked_bootstrap["status"], "reopen_in_telegram_required");
    assert_eq!(revoked_bootstrap["reason"], "bootstrap_auth_failed");

    // Expiry is checked independently of logout/revocation.  A malformed or
    // expired grant must never mint another session.
    let expired_grant = format!("miniapp-expired-grant-{}", merchant_id.simple());
    state
        .grant_store
        .insert(
            expired_grant.clone(),
            GrantData {
                grant_hash: "expired-grant-hash".to_string(),
                issued_for: tg_user_id.to_string(),
                role_hint: "merchant_owner".to_string(),
                tenant_id: tenant_id.clone(),
                merchant_id: Some(legacy_id),
                entry_intent: "menu".to_string(),
                expires_at: (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339(),
                ua_fingerprint: "fixture-ua".to_string(),
                proof_salt: "fixture-proof-salt".to_string(),
            },
        )
        .await;
    let expired_bootstrap = json_body(
        bootstrap(
            State(state.clone()),
            Json(BootstrapRequest {
                init_data: None,
                otp: None,
                grant: Some(expired_grant),
                entry_intent: None,
                client_context: None,
                tenant_hint: None,
                merchant_id_hint: None,
            }),
        )
        .await,
    );
    assert_eq!(expired_bootstrap["status"], "reopen_in_telegram_required");
    assert_eq!(expired_bootstrap["reason"], "bootstrap_auth_failed");

    remove_fixture(&pool, user_id, merchant_id, &tenant_id).await?;
    pool.close().await;
    Ok(())
}
