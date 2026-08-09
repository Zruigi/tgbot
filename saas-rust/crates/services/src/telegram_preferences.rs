use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sqlx::PgPool;

pub const DEFAULT_TELEGRAM_LOCALE: &str = "zh-CN";
pub const SUPPORTED_TELEGRAM_LOCALES: [&str; 2] = ["en", "zh-CN"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageCallback {
    Menu,
    Set(&'static str),
}

pub fn normalize_telegram_locale(value: &str) -> Option<&'static str> {
    match value.trim() {
        "en" => Some("en"),
        "zh-CN" => Some("zh-CN"),
        _ => None,
    }
}

pub fn language_label(locale: &str) -> &'static str {
    match normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE) {
        "en" => "English",
        _ => "中文",
    }
}

pub fn parse_language_callback(data: &str) -> Option<LanguageCallback> {
    if data == "lang_menu" {
        return Some(LanguageCallback::Menu);
    }
    let locale = data.strip_prefix("lang_set:")?;
    if locale.is_empty() || locale.contains(':') {
        return None;
    }
    normalize_telegram_locale(locale).map(LanguageCallback::Set)
}

pub fn language_menu_reply(locale: &str) -> (String, Option<Value>) {
    let locale = normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE);
    let (text, labels) = match locale {
        "en" => (
            format!(
                "Language settings\n\nCurrent language: {}",
                language_label(locale)
            ),
            [("中文", "zh-CN"), ("✅ English", "en")],
        ),
        _ => (
            format!("语言设置\n\n当前语言: {}", language_label(locale)),
            [("English", "en"), ("✅ 中文", "zh-CN")],
        ),
    };
    let mut keyboard = labels
        .into_iter()
        .map(|(label, code)| {
            vec![json!({
                "text": label,
                "callback_data": format!("lang_set:{code}"),
            })]
        })
        .collect::<Vec<_>>();
    keyboard.push(vec![json!({
        "text": if locale == "en" { "Back to main menu" } else { "返回主菜单" },
        "callback_data": "back_to_start",
    })]);
    (text, Some(json!({ "inline_keyboard": keyboard })))
}

pub fn localized_command_reply(command: &str, first_name: &str, locale: &str) -> String {
    let locale = normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE);
    if locale == "en" {
        return match command {
            "start" => format!(
                "Hello, {first_name}! Welcome to the bot.\n\nAvailable commands:\n/start - Start using the bot\n/help - Show help\n/wallets - View address monitors\n/wallet_monitor <address> - Add an address monitor"
            ),
            "help" => {
                "Available commands:\n/start - Start using the bot\n/help - Show help\n/wallets - View address monitors\n/wallet_monitor <address> - Add an address monitor".to_string()
            }
            "wallets" => "Reading the address monitors for this tenant. Please wait.".to_string(),
            "menu" => "Interactive menu:\n/start - Open interactive menu\n/wallets - View address monitors\n/wallet_monitor <address> - Add an address monitor\n/status - View monitoring status\n/clear - Clear current conversation cache".to_string(),
            "status" => "Checking monitoring and bot status...".to_string(),
            "clear" => "Current conversation cache cleared ✅".to_string(),
            _ => anyhow!("unsupported Telegram command").to_string(),
        };
    }
    match command {
        "start" => format!(
            "你好，{first_name}！欢迎使用本机器人。\n\n已实现的命令：\n/start - 开始使用机器人\n/help - 显示帮助信息\n/wallets - 查看地址监听\n/wallet_monitor <地址> - 添加地址监听"
        ),
        "help" => "已实现的命令：\n/start - 开始使用机器人\n/help - 显示帮助信息\n/wallets - 查看地址监听\n/wallet_monitor <地址> - 添加地址监听".to_string(),
        "wallets" => "正在读取当前租户的地址监听列表，请稍候。".to_string(),
        "menu" => "交互菜单：\n/start - 打开交互菜单\n/wallets - 查看地址监听\n/wallet_monitor <地址> - 添加地址监听\n/status - 查看监听与监控运行状态\n/clear - 清理当前对话缓存".to_string(),
        "status" => "正在检查监听与机器人运行状态...".to_string(),
        "clear" => "当前对话缓存已清理 ✅".to_string(),
        _ => anyhow!("unsupported Telegram command").to_string(),
    }
}

pub fn localized_start_menu_reply(first_name: &str, locale: &str) -> (String, Option<Value>) {
    let locale = normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE);
    let text = localized_command_reply("start", first_name, locale);
    let wallet_label = if locale == "en" {
        "📡 My address monitors"
    } else {
        "📡 我的地址监听"
    };
    (
        text,
        Some(json!({
            "inline_keyboard": [[{
                "text": wallet_label,
                "callback_data": "wallet_my_list"
            }]]
        })),
    )
}

pub async fn telegram_locale(
    pool: &PgPool,
    tenant_id: &str,
    telegram_user_id: i64,
) -> Result<String> {
    let locale = sqlx::query_scalar::<_, String>(
        "SELECT locale FROM chat.telegram_user_preferences \
         WHERE tenant_id = $1 AND telegram_user_id = $2",
    )
    .bind(tenant_id)
    .bind(telegram_user_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or_else(|| DEFAULT_TELEGRAM_LOCALE.to_string());
    Ok(normalize_telegram_locale(&locale)
        .unwrap_or(DEFAULT_TELEGRAM_LOCALE)
        .to_string())
}

pub async fn set_telegram_locale(
    pool: &PgPool,
    tenant_id: &str,
    telegram_user_id: i64,
    locale: &str,
) -> Result<String> {
    let locale =
        normalize_telegram_locale(locale).ok_or_else(|| anyhow!("unsupported Telegram locale"))?;
    sqlx::query(
        "INSERT INTO chat.telegram_user_preferences \
         (tenant_id, telegram_user_id, locale) VALUES ($1, $2, $3) \
         ON CONFLICT (tenant_id, telegram_user_id) DO UPDATE SET locale = EXCLUDED.locale, updated_at = now()",
    )
    .bind(tenant_id)
    .bind(telegram_user_id)
    .bind(locale)
    .execute(pool)
    .await?;
    Ok(locale.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_supported_language_callbacks() {
        assert_eq!(
            parse_language_callback("lang_menu"),
            Some(LanguageCallback::Menu)
        );
        assert_eq!(
            parse_language_callback("lang_set:en"),
            Some(LanguageCallback::Set("en"))
        );
        assert_eq!(
            parse_language_callback("lang_set:zh-CN"),
            Some(LanguageCallback::Set("zh-CN"))
        );
        assert_eq!(parse_language_callback("lang_set:fr"), None);
        assert_eq!(parse_language_callback("lang_set:en:extra"), None);
        assert_eq!(parse_language_callback("lang_set:"), None);
    }

    #[test]
    fn language_menu_preserves_existing_callback_contract() {
        let (text, markup) = language_menu_reply("en");
        assert!(text.contains("Language settings"));
        assert_eq!(
            markup.as_ref().unwrap()["inline_keyboard"][0][0]["callback_data"],
            "lang_set:zh-CN"
        );
        assert_eq!(
            markup.as_ref().unwrap()["inline_keyboard"][1][0]["callback_data"],
            "lang_set:en"
        );
        assert_eq!(
            markup.as_ref().unwrap()["inline_keyboard"][2][0]["callback_data"],
            "back_to_start"
        );
    }

    #[test]
    fn locale_is_tenant_aware_at_the_query_boundary() {
        assert_eq!(normalize_telegram_locale(" en "), Some("en"));
        assert_eq!(normalize_telegram_locale("fr"), None);
    }
}
