use anyhow::{anyhow, Context, Result};
use sqlx::postgres::PgPoolOptions;
use std::env;
use tgbot_http::secret_crypto::SecretCipher;
use tgbot_integrations::telegram::{TelegramClient, TelegramConfig};
use uuid::Uuid;

fn enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn required(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        return Err(anyhow!("{name} must not be empty"));
    }
    Ok(value)
}

fn bot_id_from_token(token: &str) -> Result<i64> {
    let (id, secret) = token
        .split_once(':')
        .ok_or_else(|| anyhow!("BOT_TOKEN has invalid format"))?;
    let id = id
        .parse::<i64>()
        .map_err(|_| anyhow!("BOT_TOKEN has invalid id"))?;
    if id <= 0 || secret.len() < 10 {
        return Err(anyhow!("BOT_TOKEN has invalid format"));
    }
    Ok(id)
}

#[tokio::main]
async fn main() -> Result<()> {
    if !enabled(env::var("BETA_TEST_BOT_ENABLED").ok().as_deref()) {
        return Err(anyhow!(
            "BETA_TEST_BOT_ENABLED=true is required for the isolated test Bot"
        ));
    }

    let token = required("BOT_TOKEN")?;
    let bot_id = bot_id_from_token(&token)?;
    let tenant_id = env::var("BETA_TEST_TENANT_ID")
        .unwrap_or_else(|_| "beta-real-test-bot".to_string())
        .trim()
        .to_string();
    if tenant_id.is_empty() {
        return Err(anyhow!("BETA_TEST_TENANT_ID must not be empty"));
    }
    let merchant_id = env::var("BETA_TEST_MERCHANT_ID")
        .unwrap_or_else(|_| "77777777-7777-4777-8777-777777777777".to_string());
    let merchant_id = Uuid::parse_str(merchant_id.trim())
        .map_err(|_| anyhow!("BETA_TEST_MERCHANT_ID must be a UUID"))?;
    let encryption_key = required("TGBOT_SECRET_ENCRYPTION_KEY")?;
    let database_url = required("DATABASE_URL")?;
    let api_base_url = env::var("TELEGRAM_API_BASE_URL")
        .unwrap_or_else(|_| "https://api.telegram.org".to_string());

    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: token.clone(),
        api_base_url,
        timeout_secs: 15,
    })?;
    let bot = telegram
        .get_me()
        .await
        .context("isolated test Bot getMe failed")?;
    if bot.id != bot_id || !bot.is_bot {
        return Err(anyhow!("isolated test Bot identity check failed"));
    }

    let cipher = SecretCipher::from_encoded_key(&encryption_key).map_err(anyhow::Error::msg)?;
    let encrypted_ref = cipher
        .encrypt(&tenant_id, "TELEGRAM_BOT_TOKEN", &token)
        .map_err(anyhow::Error::msg)?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .context("failed to connect to the isolated Beta database")?;
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO merchants \
         (id, tenant_id, merchant_name, status, onboarding_completed, bot_id, bot_username) \
         VALUES ($1, $2, 'Beta Real Test Bot', 'active', true, $3, $4) \
         ON CONFLICT (tenant_id) DO UPDATE SET \
           status = 'active', onboarding_completed = true, bot_id = EXCLUDED.bot_id, \
           bot_username = EXCLUDED.bot_username, updated_at = now()",
    )
    .bind(merchant_id)
    .bind(&tenant_id)
    .bind(bot.id)
    .bind(bot.username.as_deref())
    .execute(&mut *transaction)
    .await
    .context("failed to seed the isolated Beta merchant")?;
    sqlx::query(
        "INSERT INTO tenant_secret_refs (tenant_id, secret_key, secret_ref, updated_at) \
         VALUES ($1, 'TELEGRAM_BOT_TOKEN', $2, now()) \
         ON CONFLICT (tenant_id, secret_key) DO UPDATE SET \
           secret_ref = EXCLUDED.secret_ref, updated_at = now()",
    )
    .bind(&tenant_id)
    .bind(encrypted_ref)
    .execute(&mut *transaction)
    .await
    .context("failed to seed the isolated Telegram secret reference")?;
    transaction.commit().await?;
    pool.close().await;

    println!(
        "Seeded isolated Beta Bot tenant={} bot_id={} username={}",
        tenant_id,
        bot.id,
        bot.username.as_deref().unwrap_or("<none>")
    );
    Ok(())
}
