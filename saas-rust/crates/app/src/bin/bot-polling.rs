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
//! This binary refuses to start unless TELEGRAM_POLLING_ENABLED=true is set
//! explicitly. This prevents accidental use with a production bot token.

use teloxide::prelude::*;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use tgbot_services::telegram_commands::{
    build_command_reply, parse_telegram_command, TelegramCommand,
};

#[derive(Debug, Clone, PartialEq)]
enum MessageAction {
    Command(TelegramCommand),
    UnknownCommand,
    Ignore,
}

#[derive(Clone)]
struct BotUsername(String);

fn is_polling_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn classify_message(text: &str, bot_username: &str) -> MessageAction {
    if let Some(command) = parse_telegram_command(text, bot_username) {
        if matches!(
            command,
            TelegramCommand::Wallets | TelegramCommand::WalletMonitor(_)
        ) {
            return MessageAction::UnknownCommand;
        }
        return MessageAction::Command(command);
    }

    let token = text.split_whitespace().next().unwrap_or_default();
    if !token.starts_with('/') {
        return MessageAction::Ignore;
    }

    if let Some((_, suffix)) = token.split_once('@') {
        let expected = bot_username.trim_start_matches('@');
        if suffix.is_empty() || !suffix.eq_ignore_ascii_case(expected) {
            return MessageAction::Ignore;
        }
    }

    MessageAction::UnknownCommand
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
    let polling_setting = std::env::var("TELEGRAM_POLLING_ENABLED").ok();
    let enabled = is_polling_enabled(polling_setting.as_deref());
    if !enabled {
        tracing::error!(
            "TELEGRAM_POLLING_ENABLED is not set to true. \
             The polling binary has no tenant isolation and must be explicitly enabled. \
             Set TELEGRAM_POLLING_ENABLED=true to proceed."
        );
        std::process::exit(1);
    }

    let token = std::env::var("BOT_TOKEN")
        .or_else(|_| std::env::var("TELOXIDE_TOKEN"))
        .map_err(|_| anyhow::anyhow!("BOT_TOKEN or TELOXIDE_TOKEN required"))?;

    tracing::info!("Starting polling bot...");
    let bot = Bot::new(&token);
    let me = bot.get_me().send().await?;
    let bot_username = me
        .username
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Telegram bot account has no username"))?;
    tracing::info!(bot_username = %bot_username, "Polling bot identity resolved");

    // Align the command menu and Mini App console button with the source
    // project (Python `post_init`): global `/menu`, admin-only commands, and
    // the MenuButtonWebApp console entry when FRONTEND_URL is a public URL.
    configure_bot_menu(&token).await;

    let handler = Update::filter_message().endpoint(message_handler);

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![BotUsername(bot_username)])
        .default_handler(|_upd| async move {})
        .error_handler(LoggingErrorHandler::with_custom_text("Bot error"))
        .build();

    tracing::info!("Polling started — press Ctrl+C to stop");
    dispatcher.dispatch().await;
    Ok(())
}

/// Align the bot's command menu and Mini App console button with the source
/// project (Python `post_init`):
///
/// - global command menu: `/menu` (Open interactive menu / 打开交互菜单)
/// - admin-only command menu (scope `chat` = `ADMIN_ID`): `/menu`, `/admin`,
///   `/status`, `/sync_potential`, `/clear`
/// - menu button: `MenuButtonWebApp` "控制台" pointing at
///   `{FRONTEND_URL}/miniapp?entry=admin_console` (only when `FRONTEND_URL`
///   is a public URL, mirroring the source project's guard)
///
/// Menu configuration is best-effort: failures are logged but never prevent
/// the polling bot from starting.
async fn configure_bot_menu(token: &str) {
    use serde_json::json;
    use tgbot_integrations::telegram::{BotCommandItem, TelegramClient, TelegramConfig};

    let Ok(client) = TelegramClient::new(TelegramConfig {
        bot_token: token.to_string(),
        api_base_url: std::env::var("TELEGRAM_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.telegram.org".into()),
        timeout_secs: 30,
    }) else {
        tracing::warn!("Failed to build Telegram client for menu setup");
        return;
    };

    // 1. Global command menu.
    let global_commands = vec![BotCommandItem {
        command: "menu".into(),
        description: "Open interactive menu / 打开交互菜单".into(),
    }];
    match client.set_my_commands(global_commands, None).await {
        Ok(()) => tracing::info!("Global command menu configured"),
        Err(error) => tracing::warn!(%error, "Failed to set global command menu"),
    }

    // 2. Admin-only command menu.
    let admin_id = std::env::var("ADMIN_ID")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok());
    if let Some(admin_id) = admin_id {
        let admin_commands = vec![
            BotCommandItem {
                command: "menu".into(),
                description: "🧭 打开交互菜单".into(),
            },
            BotCommandItem {
                command: "admin".into(),
                description: "⚙️ 启动机器人控制面板".into(),
            },
            BotCommandItem {
                command: "status".into(),
                description: "📊 查看监听与监控运行状态".into(),
            },
            BotCommandItem {
                command: "sync_potential".into(),
                description: "🔄 同步群组潜在用户".into(),
            },
            BotCommandItem {
                command: "clear".into(),
                description: "🧹 清理当前对话缓存".into(),
            },
        ];
        let scope = json!({ "type": "chat", "chat_id": admin_id });
        match client.set_my_commands(admin_commands, Some(scope)).await {
            Ok(()) => tracing::info!(admin_id, "Admin command menu configured"),
            Err(error) => tracing::warn!(%error, "Failed to set admin command menu"),
        }
    }

    // 3. Mini App console menu button (mirror source guard: public URL only).
    let frontend_url = std::env::var("FRONTEND_URL").unwrap_or_default();
    let frontend_url = frontend_url.trim_end_matches('/').to_string();
    if frontend_url.is_empty()
        || frontend_url.contains("127.0.0.1")
        || frontend_url.contains("localhost")
    {
        tracing::warn!("FRONTEND_URL 未配置或为本地地址，跳过 Menu Button 设置");
        return;
    }
    let miniapp_url = format!("{frontend_url}/miniapp?entry=admin_console");
    let menu_button = json!({
        "type": "web_app",
        "text": "控制台",
        "web_app": { "url": miniapp_url }
    });
    match client.set_chat_menu_button(None, menu_button.clone()).await {
        Ok(()) => tracing::info!(%miniapp_url, "Mini App menu button configured"),
        Err(error) => tracing::warn!(%error, "Failed to set menu button"),
    }
    if let Some(admin_id) = admin_id {
        match client.set_chat_menu_button(Some(admin_id), menu_button).await {
            Ok(()) => tracing::info!(admin_id, "Admin Mini App menu button configured"),
            Err(error) => tracing::warn!(%error, "Failed to set admin menu button"),
        }
    }
}

async fn message_handler(
    bot: Bot,
    msg: Message,
    bot_username: BotUsername,
) -> Result<(), teloxide::RequestError> {
    let Some(text) = msg.text() else {
        return Ok(());
    };
    match classify_message(text, &bot_username.0) {
        MessageAction::Command(command) => {
            let first_name = msg
                .from
                .as_ref()
                .map(|user| user.first_name.clone())
                .unwrap_or_else(|| "朋友".into());
            let reply = build_command_reply(&command, &first_name);
            bot.send_message(msg.chat.id, reply.text).send().await?;
        }
        MessageAction::UnknownCommand => {
            bot.send_message(msg.chat.id, "未知命令。请输入 /help 查看可用命令。")
                .send()
                .await?;
        }
        MessageAction::Ignore => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_disabled_by_default() {
        assert!(!is_polling_enabled(None));
    }

    #[test]
    fn test_gate_true_enables() {
        for v in &["1", "true", "TRUE", "yes", "on", "  true  "] {
            assert!(
                is_polling_enabled(Some(v)),
                "value {v:?} should enable the gate"
            );
        }
    }

    #[test]
    fn test_gate_false_disables() {
        for v in &["0", "false", "no", "off", "random", ""] {
            assert!(
                !is_polling_enabled(Some(v)),
                "value {v:?} should not enable the gate"
            );
        }
    }

    #[test]
    fn test_parse_commands_start_without_payload() {
        assert_eq!(
            classify_message("/start", "MyTestBot"),
            MessageAction::Command(TelegramCommand::Start(None))
        );
    }

    #[test]
    fn test_parse_commands_start_with_payload() {
        assert_eq!(
            classify_message("/start ref_abc", "MyTestBot"),
            MessageAction::Command(TelegramCommand::Start(Some("ref_abc".into())))
        );
    }

    #[test]
    fn test_polling_rejects_tenant_wallet_menu() {
        assert_eq!(
            classify_message("/wallets", "MyTestBot"),
            MessageAction::UnknownCommand
        );
    }

    #[test]
    fn test_parse_commands_start_with_bot_suffix_and_payload() {
        assert_eq!(
            classify_message("/start@MyTestBot ref_xyz", "MyTestBot"),
            MessageAction::Command(TelegramCommand::Start(Some("ref_xyz".into())))
        );
    }

    #[test]
    fn test_parse_commands_help() {
        assert_eq!(
            classify_message("/help", "MyTestBot"),
            MessageAction::Command(TelegramCommand::Help)
        );
    }

    #[test]
    fn test_parse_commands_help_with_bot_suffix() {
        assert_eq!(
            classify_message("/help@MyTestBot", "MyTestBot"),
            MessageAction::Command(TelegramCommand::Help)
        );
    }

    #[test]
    fn test_parse_commands_wrong_bot_suffix() {
        assert_eq!(
            classify_message("/start@WrongBot", "MyTestBot"),
            MessageAction::Ignore
        );
    }

    #[test]
    fn test_parse_commands_help_wrong_bot_suffix() {
        assert_eq!(
            classify_message("/help@WrongBot", "MyTestBot"),
            MessageAction::Ignore
        );
    }

    #[test]
    fn test_parse_commands_plain_text() {
        assert_eq!(
            classify_message("hello world", "MyTestBot"),
            MessageAction::Ignore
        );
    }

    #[test]
    fn test_parse_commands_unknown_command() {
        assert_eq!(
            classify_message("/unknown", "MyTestBot"),
            MessageAction::UnknownCommand
        );

        assert_eq!(
            classify_message("/unknown@MyTestBot", "MyTestBot"),
            MessageAction::UnknownCommand
        );

        assert_eq!(
            classify_message("/unknown@OtherBot", "MyTestBot"),
            MessageAction::Ignore
        );
    }

    #[test]
    fn test_parse_commands_empty() {
        assert_eq!(classify_message("", "MyTestBot"), MessageAction::Ignore);
    }
}
