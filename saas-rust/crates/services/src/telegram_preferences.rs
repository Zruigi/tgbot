use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

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

// ---------------------------------------------------------------------------
// 主菜单 / 控制台入口（对齐 Python handlers/start.py 的卡片菜单）
// ---------------------------------------------------------------------------

/// 查询 Telegram 用户在控制台场景下的角色：是否平台管理员、是否商户 owner
pub async fn user_console_roles(pool: &PgPool, tg_user_id: i64) -> Result<(bool, bool)> {
    let is_platform_admin = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS( \
             SELECT 1 FROM auth.users u \
             JOIN auth.saas_user_roles r ON r.user_id = u.id \
             WHERE u.tg_user_id = $1 AND r.role = 'platform_admin' \
         )",
    )
    .bind(tg_user_id)
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    let has_merchant = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS( \
             SELECT 1 FROM merchants \
             WHERE owner_tg_user_id = $1 AND status IN ('active', 'active_free') \
         )",
    )
    .bind(tg_user_id)
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    Ok((is_platform_admin, has_merchant))
}

/// 查询该 Telegram 用户拥有的商户：(tenant_id, legacy_merchant_id)
pub async fn merchant_console_by_owner(
    pool: &PgPool,
    tg_user_id: i64,
) -> Result<Option<(String, Option<i64>)>> {
    let row = sqlx::query(
        "SELECT tenant_id, legacy_merchant_id FROM merchants \
         WHERE owner_tg_user_id = $1 AND status IN ('active', 'active_free') \
         ORDER BY created_at LIMIT 1",
    )
    .bind(tg_user_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some((
        row.try_get::<String, _>("tenant_id")?,
        row.try_get::<Option<i64>, _>("legacy_merchant_id")?,
    )))
}

/// 构建 /start 主菜单（对齐 Python _admin_keyboard/_user_keyboard）
///
/// 管理员额外显示 Web 管理后台；普通用户显示联系人工客服；
/// 商户 owner 显示商户控制台，否则显示注册机器人入口（点击后提示未绑定）。
pub fn build_full_start_menu(
    is_platform_admin: bool,
    has_merchant_console: bool,
    first_name: &str,
    user_id: i64,
    locale: &str,
) -> (String, Option<Value>) {
    let locale = normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE);
    let role_tag = if is_platform_admin {
        "👑 管理员"
    } else if has_merchant_console {
        "🏪 商户"
    } else {
        "👤 用户"
    };
    let text = if locale == "en" {
        format!(
            "Hello, {first_name} {role_tag}\nUserID: <code>{user_id}</code>\n\nPlease select a function:"
        )
    } else {
        format!(
            "你好，{first_name} {role_tag}\nUserID: <code>{user_id}</code>\n\n请选择功能："
        )
    };

    let mut rows: Vec<Vec<Value>> = Vec::new();
    if is_platform_admin {
        rows.push(vec![json!({
            "text": "🌐 Web 管理后台",
            "callback_data": "admin_web_otp",
        })]);
    }
    rows.push(vec![json!({
        "text": "🎧 联系人工客服",
        "callback_data": "contact_admin",
    })]);
    rows.push(vec![json!({
        "text": if has_merchant_console { "🏪 商户控制台" } else { "🤖 注册机器人" },
        "callback_data": "merchant_web_otp",
    })]);
    rows.push(vec![
        json!({
            "text": "👛 监听钱包",
            "callback_data": "wallet_monitor",
        }),
        json!({
            "text": "📋 我的列表",
            "callback_data": "wallet_my_list",
        }),
    ]);
    rows.push(vec![json!({
        "text": "🌐 语言选择",
        "callback_data": "lang_menu",
    })]);

    (text, Some(json!({ "inline_keyboard": rows })))
}

/// 构建控制台入口卡片（web_app 打开 Mini App + 浏览器兜底 OTP + 返回主菜单）
pub fn build_console_entry_card(
    locale: &str,
    miniapp_url: &str,
    browser_url: &str,
    is_merchant: bool,
) -> (String, Option<Value>) {
    let _ = locale;
    let text = if is_merchant {
        "🏪 商户控制台\n\nMini App 主入口会优先使用 Telegram 身份自动登录。\n浏览器兜底入口仍保留 OTP，并在同设备内支持无感续签。"
    } else {
        "🚀 Web 管理后台\n\nMini App 主入口会优先使用 Telegram 身份自动登录。\n浏览器兜底入口仍保留 OTP，并在同设备内支持无感续签。"
    };
    let open_label = if is_merchant {
        "🏪 打开商户控制台"
    } else {
        "🚀 打开管理后台"
    };
    let keyboard = json!({
        "inline_keyboard": [
            [{ "text": open_label, "web_app": { "url": miniapp_url } }],
            [{ "text": "🌐 浏览器打开", "url": browser_url }],
            [{ "text": "⬅️ 返回主菜单", "callback_data": "back_to_start" }],
        ]
    });
    (text.to_string(), Some(keyboard))
}

/// 构建控制台入口 URL：(miniapp_url, browser_url)。
/// browser_url 额外带 otp，浏览器打开后 LoginView 自动登录（无需手工输 token）。
pub fn console_entry_urls(
    frontend_url: &str,
    entry_intent: &str,
    grant: &str,
    otp: Option<&str>,
    tenant_id: Option<&str>,
    merchant_id: Option<i64>,
) -> (String, String) {
    let base = frontend_url.trim().trim_end_matches('/');
    let mut params = Vec::new();
    params.push(("entry".to_string(), entry_intent.to_string()));
    params.push(("grant".to_string(), grant.to_string()));
    if let Some(tenant_id) = tenant_id.filter(|value| !value.trim().is_empty()) {
        params.push(("tenant".to_string(), tenant_id.trim().to_string()));
    }
    if let Some(merchant_id) = merchant_id.filter(|value| *value > 0) {
        params.push(("merchant_id".to_string(), merchant_id.to_string()));
    }
    let miniapp_url = build_miniapp_url(base, &params);
    let mut browser_params = params.clone();
    if let Some(otp) = otp.filter(|value| !value.trim().is_empty()) {
        browser_params.push(("otp".to_string(), otp.trim().to_string()));
    }
    let browser_url = build_miniapp_url(base, &browser_params);
    (miniapp_url, browser_url)
}

fn build_miniapp_url(base: &str, params: &[(String, String)]) -> String {
    if base.is_empty() {
        return String::new();
    }
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}/miniapp?{query}")
}

/// 极简 urlencode（对齐 Python urllib.parse.quote 的主要场景）
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
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
