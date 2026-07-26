//! Standalone Telegram bot — polling mode (development/testing only).
//!
//! # Security
//!
//! This binary uses a single BOT_TOKEN from environment. It has NO tenant
//! isolation and NO webhook security. It is intended ONLY for local
//! development and integration testing.
//!
//! # Gate
//!
//! This binary refuses to start unless TELEGRAM_COMMANDS_ENABLED=true is set
//! explicitly. This prevents accidental use with a production bot token.

use teloxide::prelude::*;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use tgbot_services::telegram_commands::{build_command_reply, parse_telegram_command};

/// Commands recognised by the polling binary.
///
/// Only `/start` and `/help` are supported — matching the shared
/// `tgbot_services::telegram_commands::parse_telegram_command` parser.
#[derive(Debug, Clone, PartialEq)]
enum Command {
    Start,
    Help,
}

fn parse_commands(text: &str) -> Option<Command> {
    let bot_username = std::env::var("BOT_USERNAME").unwrap_or_default();
    match parse_telegram_command(text, &bot_username) {
        Some(tgbot_services::telegram_commands::TelegramCommand::Start(_)) => Some(Command::Start),
        Some(tgbot_services::telegram_commands::TelegramCommand::Help) => Some(Command::Help),
        None => None,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,teloxide=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // === Fail-closed gate ===
    // This polling binary has no tenant isolation.  It MUST NOT be run against
    // a production bot token unless the operator explicitly opts in.
    let enabled = std::env::var("TELEGRAM_COMMANDS_ENABLED")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
    if !enabled {
        tracing::error!(
            "TELEGRAM_COMMANDS_ENABLED is not set to true. \
             The polling binary has no tenant isolation and must be explicitly enabled. \
             Set TELEGRAM_COMMANDS_ENABLED=true to proceed."
        );
        std::process::exit(1);
    }

    let token = std::env::var("BOT_TOKEN")
        .or_else(|_| std::env::var("TELOXIDE_TOKEN"))
        .map_err(|_| anyhow::anyhow!("BOT_TOKEN or TELOXIDE_TOKEN required"))?;

    tracing::info!("Starting polling bot...");
    let bot = Bot::new(&token);

    let me = bot.get_me().send().await?;
    let username = me.username.as_deref().unwrap_or("unknown");
    tracing::info!("Bot @{} is online!", username);

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter(|msg: Message| {
                    msg.text()
                        .and_then(|text| parse_commands(text))
                        .is_some()
                })
                .endpoint(command_handler),
        )
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.text().is_some())
                .endpoint(fallback_message),
        );

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .default_handler(|_upd| async move {})
        .error_handler(LoggingErrorHandler::with_custom_text("Bot error"))
        .build();

    tracing::info!("Polling started — press Ctrl+C to stop");
    dispatcher.dispatch().await;
    Ok(())
}

async fn command_handler(bot: Bot, msg: Message) -> Result<(), teloxide::RequestError> {
    let Some(text) = msg.text() else {
        return Ok(());
    };
    let Some(command) = parse_commands(text) else {
        return Ok(());
    };
    let first_name = msg
        .from()
        .map(|u| u.first_name.clone())
        .unwrap_or_else(|| "朋友".into());

    let shared_cmd = match command {
        Command::Start => tgbot_services::telegram_commands::TelegramCommand::Start(None),
        Command::Help => tgbot_services::telegram_commands::TelegramCommand::Help,
    };
    let reply = build_command_reply(&shared_cmd, &first_name);

    bot.send_message(msg.chat.id, reply.text).send().await?;
    Ok(())
}

async fn fallback_message(bot: Bot, msg: Message) -> Result<(), teloxide::RequestError> {
    if let Some(text) = msg.text() {
        if text.starts_with('/') {
            bot.send_message(msg.chat.id, "未知命令。请输入 /help 查看可用命令。")
                .send()
                .await?;
        }
        // Non-command text is silently ignored in polling mode.
    }
    Ok(())
}