use anyhow::{anyhow, Context, Result};
use std::{
    env, fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tgbot_http::routes::telegram_webhook::derive_webhook_secret;
use tgbot_integrations::telegram::{GetUpdatesParams, TelegramClient, TelegramConfig};
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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

fn webhook_url(template: &str, bot_id: i64) -> String {
    template.replace("{bot_id}", &bot_id.to_string())
}

fn write_heartbeat(path: Option<&str>) -> Result<()> {
    let Some(path) = path.filter(|path| !path.trim().is_empty()) else {
        return Ok(());
    };
    let temporary_path = format!("{path}.tmp.{}", std::process::id());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    fs::write(&temporary_path, format!("{timestamp}\n"))?;
    fs::rename(temporary_path, path)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tgbot_integrations=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    if !enabled(env::var("BETA_TEST_BOT_ENABLED").ok().as_deref()) {
        return Err(anyhow!(
            "BETA_TEST_BOT_ENABLED=true is required for the isolated test Bot"
        ));
    }
    let token = required("BOT_TOKEN")?;
    let master_key = required("TGBOT_SECRET_ENCRYPTION_KEY")?;
    let telegram = TelegramClient::new(TelegramConfig {
        bot_token: token.clone(),
        api_base_url: env::var("TELEGRAM_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.telegram.org".to_string()),
        timeout_secs: 40,
    })?;
    let bot = telegram.get_me().await.context("test Bot getMe failed")?;
    let webhook_template = env::var("BETA_LOCAL_WEBHOOK_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18090/telegram/webhook/{bot_id}".to_string());
    let webhook_url = webhook_url(&webhook_template, bot.id);
    let webhook_secret =
        derive_webhook_secret(&master_key, bot.id, &token).map_err(anyhow::Error::msg)?;
    let heartbeat_path = env::var("BETA_BRIDGE_HEARTBEAT_FILE").ok();
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(30))
        .build()?;

    if enabled(env::var("BETA_CLEAR_WEBHOOK").ok().as_deref()) {
        telegram
            .delete_webhook()
            .await
            .context("failed to clear the test Bot webhook")?;
    }

    info!(
        bot_id = bot.id,
        bot_username = bot.username.as_deref().unwrap_or("<none>"),
        webhook_url,
        "Beta real Bot bridge started"
    );
    if let Err(error) = write_heartbeat(heartbeat_path.as_deref()) {
        warn!(error = %error, "Unable to write Beta real Bot bridge heartbeat");
    }

    let mut offset = None;
    loop {
        let updates = match telegram
            .get_updates(GetUpdatesParams {
                offset,
                limit: Some(100),
                timeout: Some(25),
            })
            .await
        {
            Ok(updates) => updates,
            Err(error) => {
                warn!(error = %error, "Telegram getUpdates failed; retrying");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        if let Err(error) = write_heartbeat(heartbeat_path.as_deref()) {
            warn!(error = %error, "Unable to update Beta real Bot bridge heartbeat");
        }

        for update in updates {
            let update_id = update.update_id;
            let response = http
                .post(&webhook_url)
                .header("x-telegram-bot-api-secret-token", &webhook_secret)
                .json(&update)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    offset = Some(update_id + 1);
                    info!(update_id, "Forwarded Telegram update to local Beta");
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    let body = body.chars().take(240).collect::<String>();
                    warn!(update_id, %status, body, "Local Beta rejected Telegram update");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    break;
                }
                Err(error) => {
                    warn!(update_id, error = %error, "Local Beta webhook request failed");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    break;
                }
            }
        }
    }
}
