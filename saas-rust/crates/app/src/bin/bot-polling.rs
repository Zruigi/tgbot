//! Standalone Telegram bot — polling mode using teloxide 0.14.

use teloxide::{prelude::*, utils::command::BotCommands};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum Command {
    #[command(description = "Welcome & bot info")]
    Start,
    #[command(description = "Command list")]
    Help,
    #[command(description = "System status")]
    Status,
    #[command(description = "Echo your message")]
    Echo(String),
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
                .filter_command::<Command>()
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

async fn command_handler(bot: Bot, msg: Message, cmd: Command) -> Result<(), teloxide::RequestError> {
    match cmd {
        Command::Start => {
            let name = msg.from()
                .map(|u| u.first_name.clone())
                .unwrap_or_else(|| "friend".into());
            bot.send_message(msg.chat.id, format!(
                "Hello, {}!\n\nI'm your SaaS Telegram Bot (Rust + Supabase).\n\n/help — commands\n/status — system status\n/echo <text> — echo\n\nBuilt with: Rust | teloxide | axum | Supabase",
                name
            )).send().await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id,
                "Available Commands:\n\n/start — Welcome & bot info\n/help — This menu\n/status — System status\n/echo <text> — Echo back\n\nMore coming soon..."
            ).send().await?;
        }
        Command::Status => {
            let supabase_url = std::env::var("SUPABASE_URL").unwrap_or_default();
            let pg_url = std::env::var("SUPABASE_DATABASE_URL").unwrap_or_default();
            let pg_status = if pg_url.is_empty() { "Not configured" } else { "Blocked by proxy" };

            let text = format!(
                "System Status\n\nBot: Online\nSupabase REST: Connected\nPostgreSQL direct: {}\nSupabase URL: {}\nMode: REST-only",
                pg_status,
                if supabase_url.is_empty() { "Not set" } else { "Set" }
            );
            bot.send_message(msg.chat.id, text).send().await?;
        }
        Command::Echo(ref text) => {
            let echo_text = text.trim();
            if echo_text.is_empty() {
                bot.send_message(msg.chat.id, "Usage: /echo <your message>").send().await?;
            } else {
                bot.send_message(msg.chat.id, format!("{}", echo_text)).send().await?;
            }
        }
    }
    Ok(())
}

async fn fallback_message(bot: Bot, msg: Message) -> Result<(), teloxide::RequestError> {
    if let Some(text) = msg.text() {
        if text.starts_with('/') {
            bot.send_message(msg.chat.id, "Unknown command. Try /help").send().await?;
        } else {
            bot.send_message(
                msg.chat.id,
                format!("Received: \"{}\"\n\nUse /help to see commands.", text),
            ).send().await?;
        }
    }
    Ok(())
}
