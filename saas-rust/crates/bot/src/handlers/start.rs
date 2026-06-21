//! /start 命令处理 - v3.3 规范
//! - 双轨菜单（管理员 / 普通用户）
//! - 双列排版
//! - OTP 动态链接（不暴露明文 Token）
//! - Deep Linking 自助解禁：unmute_{group_id} 参数处理

use teloxide::{
    types::{Update, User, ChatPermissions, WebAppInfo, InlineKeyboardButton, InlineKeyboardMarkup, Message, CallbackQuery},
    dispatching::{UpdateWithCx, StorageDoor},
    Bot, HandlerExt,
};
use crate::telegram_client::TelegramClient;
use crate::runtime_registry::RuntimeRegistry;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn, error};

// ---------------------------------------------------------------------------
// 频率限制缓存：{user_id: last_verify_time}，TTL 60秒，上限 1000 条
// ---------------------------------------------------------------------------
lazy_static::lazy_static! {
    static ref UNMUTE_RATE_LIMIT: Mutex<lru::LruCache<i64, Instant>> =
        Mutex::new(lru::LruCache::new(1000));
}

// ---------------------------------------------------------------------------
// i18n 支持（简化版，后续可扩展为完整的 i18n 系统）
// ---------------------------------------------------------------------------
const SUPPORTED_LOCALES: [&str; 2] = ["en", "zh-CN"];

fn language_label(code: &str) -> &'static str {
    match code {
        "en" => "English",
        "zh-CN" => "中文",
        _ => code,
    }
}

// ---------------------------------------------------------------------------
// 管理员判断
// ---------------------------------------------------------------------------
fn is_admin(user_id: i64) -> bool {
    // 从配置读取管理员 ID 列表
    let admin_ids = std::env::var("ADMIN_TG_IDS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect::<Vec<_>>();
    admin_ids.contains(&user_id)
}

// ---------------------------------------------------------------------------
// 菜单构建
// ---------------------------------------------------------------------------
fn main_menu_action_map(locale: &str, has_merchant_console: bool, is_admin: bool) -> HashMap<String, String> {
    let mut mapping = HashMap::new();
    mapping.insert("语言设置".to_string(), "lang_menu".to_string());
    mapping.insert("联系客服".to_string(), "contact_admin".to_string());
    mapping.insert("现货价格".to_string(), "spot_price".to_string());
    mapping.insert("合约价格".to_string(), "futures_price".to_string());
    mapping.insert("钱包监控".to_string(), "wallet_monitor".to_string());
    mapping.insert("我的钱包".to_string(), "wallet_my_list".to_string());
    mapping.insert("邮箱设置".to_string(), "email_settings".to_string());
    mapping.insert("返回主菜单".to_string(), "back_to_start".to_string());

    if has_merchant_console {
        mapping.insert("商户后台".to_string(), "merchant_web_otp".to_string());
    } else {
        mapping.insert("注册商户".to_string(), "register_merchant".to_string());
    }

    if is_admin {
        mapping.insert("管理后台".to_string(), "admin_web_otp".to_string());
        mapping.insert("系统设置".to_string(), "admin_settings".to_string());
    }
    mapping
}

fn admin_keyboard(locale: &str) -> InlineKeyboardMarkup {
    let rows = vec![
        vec![InlineKeyboardButton::callback("管理后台", "admin_web_otp")],
        vec![InlineKeyboardButton::callback("商户后台", "merchant_web_otp")],
        vec![
            InlineKeyboardButton::callback("系统设置", "admin_settings"),
            InlineKeyboardButton::callback("现货价格", "spot_price"),
        ],
        vec![
            InlineKeyboardButton::callback("合约价格", "futures_price"),
            InlineKeyboardButton::callback("钱包监控", "wallet_monitor"),
        ],
        vec![
            InlineKeyboardButton::callback("我的钱包", "wallet_my_list"),
            InlineKeyboardButton::callback("邮箱设置", "email_settings"),
        ],
        vec![InlineKeyboardButton::callback("语言设置", "lang_menu")],
    ];
    InlineKeyboardMarkup::new(rows)
}

fn user_keyboard(locale: &str, has_merchant_console: bool) -> InlineKeyboardMarkup {
    let mut rows = vec![
        vec![InlineKeyboardButton::callback("联系客服", "contact_admin")],
    ];

    if has_merchant_console {
        rows.push(vec![InlineKeyboardButton::callback("商户后台", "merchant_web_otp")]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback("注册商户", "register_merchant")]);
    }

    rows.extend(vec![
        vec![
            InlineKeyboardButton::callback("现货价格", "spot_price"),
            InlineKeyboardButton::callback("合约价格", "futures_price"),
        ],
        vec![
            InlineKeyboardButton::callback("钱包监控", "wallet_monitor"),
            InlineKeyboardButton::callback("我的钱包", "wallet_my_list"),
        ],
        vec![InlineKeyboardButton::callback("邮箱设置", "email_settings")],
        vec![InlineKeyboardButton::callback("语言设置", "lang_menu")],
    ]);

    InlineKeyboardMarkup::new(rows)
}

fn language_keyboard(current_locale: &str) -> InlineKeyboardMarkup {
    let mut rows = vec![];
    for code in SUPPORTED_LOCALES.iter() {
        let prefix = if *code == current_locale { "✅ " } else { "" };
        let label = format!("{}{}", prefix, language_label(code));
        rows.push(vec![InlineKeyboardButton::callback(
            &label,
            &format!("lang_set:{}", code),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback("返回主菜单", "back_to_start")]);
    InlineKeyboardMarkup::new(rows)
}

// ---------------------------------------------------------------------------
// /start 命令入口
// ---------------------------------------------------------------------------
pub async fn start_command(
    bot: Bot,
    msg: Message,
    args: Vec<String>,
    telegram_client: Arc<TelegramClient>,
) -> Result<()> {
    let user = msg.from().ok_or_else(|| anyhow::anyhow!("No user found"))?;
    let user_id = user.id.0;

    // 解析 Deep Linking 参数
    if let Some(first_arg) = args.first() {
        if first_arg.starts_with("unmute_") {
            handle_unmute_deeplink(&bot, user_id, first_arg, telegram_client).await?;
            return Ok(());
        }
    }

    let is_admin_user = is_admin(user_id);

    // 检查商户后台权限（后续从数据库查询）
    let has_merchant_console = false; // TODO: 从 merchant_repo 查询

    // 构建菜单
    let keyboard = if is_admin_user {
        admin_keyboard("zh-CN")
    } else {
        user_keyboard("zh-CN", has_merchant_console)
    };

    // 构建主菜单文本
    let role_tag = if is_admin_user {
        "管理员"
    } else if has_merchant_console {
        "商户"
    } else {
        "用户"
    };

    let text = format!(
        "欢迎 {}\n角色: {}\nID: {}\n\n请选择功能:",
        user.full_name().unwrap_or_else(|| "用户".to_string()),
        role_tag,
        user_id
    );

    bot.send_message(msg.chat.id, text)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// 处理 unmute_{group_id} Deep Linking 参数
// ---------------------------------------------------------------------------
async fn handle_unmute_deeplink(
    bot: &Bot,
    user_id: i64,
    param: &str,
    telegram_client: Arc<TelegramClient>,
) -> Result<()> {
    // 1. 解析 group_id
    let group_id = param
        .trim_start_matches("unmute_")
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("无效的解禁链接参数"))?;

    // 2. 频率限制检查（1分钟内只能点击一次）
    let mut rate_limit = UNMUTE_RATE_LIMIT.lock().await;
    if let Some(last_time) = rate_limit.get(&user_id) {
        let elapsed = last_time.elapsed();
        if elapsed < Duration::from_secs(60) {
            let remaining = 60 - elapsed.as_secs();
            bot.send_message(
                teloxide::types::ChatId(user_id),
                format!("⏱️ 请稍候 {} 秒后再试。（防止爆破 API）", remaining),
            )
            .await?;
            return Ok(());
        }
    }
    rate_limit.put(user_id, Instant::now());
    drop(rate_limit);

    // 3. 获取群组验证配置（从数据库查询，后续实现）
    let verify_channel_id: Option<i64> = None; // TODO: 从 database.get_group_verify_config 查询

    if let Some(channel_id) = verify_channel_id {
        // 4. 检查用户是否已关注频道
        if let Ok(member) = bot.get_chat_member(teloxide::types::ChatId(channel_id), teloxide::types::UserId(user_id)).await {
            let is_member = matches!(
                member.kind,
                teloxide::types::ChatMemberKind::Owner(_)
                | teloxide::types::ChatMemberKind::Administrator(_)
                | teloxide::types::ChatMemberKind::Member(_)
            );

            if is_member {
                // 已关注：直接执行解禁
                execute_unmute(bot, user_id, group_id).await?;
            } else {
                // 未关注：发送频道链接 + 验证按钮
                let channel_link = format!("https://t.me/{}", channel_id);
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::url("📢 前往关注频道", &channel_link)],
                    vec![InlineKeyboardButton::callback(
                        "✅ 我已关注，点击验证",
                        &format!("unmute_verify:{}:{}", group_id, channel_id),
                    )],
                ]);

                bot.send_message(
                    teloxide::types::ChatId(user_id),
                    "📢 <b>需要关注频道才能解禁</b>\n\n请先关注下方频道，然后点击 [我已关注，点击验证] 按钮完成验证。\n\n验证成功后，你将恢复在群组中的发言权限。"
                )
                .reply_markup(keyboard)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
            }
        }
    } else {
        bot.send_message(
            teloxide::types::ChatId(user_id),
            "❌ 该群组未配置验证频道，无法解禁。"
        )
        .await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 执行解禁操作
// ---------------------------------------------------------------------------
async fn execute_unmute(bot: &Bot, user_id: i64, group_id: i64) -> Result<()> {
    // 1. 检查 Bot 权限
    let bot_user_id = bot.id();
    if let Ok(member) = bot.get_chat_member(teloxide::types::ChatId(group_id), bot_user_id).await {
        let can_restrict = matches!(
            member.kind,
            teloxide::types::ChatMemberKind::Owner(_)
            | teloxide::types::ChatMemberKind::Administrator(_)
        );

        if !can_restrict {
            bot.send_message(
                teloxide::types::ChatId(user_id),
                "❌ Bot 在该群组中权限不足，无法解禁。请联系群管理员。"
            )
            .await?;
            return Ok(());
        }
    }

    // 2. 恢复用户发言权限
    bot.restrict_chat_member(
        teloxide::types::ChatId(group_id),
        teloxide::types::UserId(user_id),
        ChatPermissions {
            can_send_messages: Some(true),
            can_send_polls: Some(true),
            can_send_other_messages: Some(true),
            can_add_web_page_previews: Some(true),
            can_change_info: Some(false),
            can_invite_users: Some(true),
            can_pin_messages: Some(false),
        },
    )
    .await?;

    info!("[解禁] 已恢复用户发言权限 user={} group={}", user_id, group_id);

    // 3. 更新数据库（后续实现）
    // await database.set_member_verified(group_id, user_id);

    // 4. 发送成功提示
    bot.send_message(
        teloxide::types::ChatId(user_id),
        "✅ <b>解禁成功！</b>\n\n你已恢复在群组中的发言权限，现在可以回到群里发言了。\n\n感谢你的关注与支持！"
    )
    .parse_mode(teloxide::types::ParseMode::Html)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// 按钮回调处理
// ---------------------------------------------------------------------------
pub async fn button_callback(
    bot: Bot,
    callback: CallbackQuery,
) -> Result<()> {
    let data = match &callback.data {
        Some(d) => d,
        None => return Ok(()),
    };

    let user = callback.from;
    let user_id = user.id.0;
    let is_admin_user = is_admin(user_id);

    // 处理解禁验证回调
    if data.starts_with("unmute_verify:") {
        let parts: Vec<&str> = data.split(':').collect();
        if parts.len() >= 3 {
            let group_id: i64 = parts[1].parse().unwrap_or(0);
            let verify_channel_id: i64 = parts[2].parse().unwrap_or(0);

            if let Ok(member) = bot.get_chat_member(
                teloxide::types::ChatId(verify_channel_id),
                teloxide::types::UserId(user_id)
            ).await {
                let is_member = matches!(
                    member.kind,
                    teloxide::types::ChatMemberKind::Owner(_)
                    | teloxide::types::ChatMemberKind::Administrator(_)
                    | teloxide::types::ChatMemberKind::Member(_)
                );

                if is_member {
                    callback.answer("⏳ 正在处理解禁请求，请稍候...").await?;
                    execute_unmute(&bot, user_id, group_id).await?;
                    return Ok(());
                }
            }

            callback.answer("❌ 未检测到你的关注，请先关注频道后再试").await?;
        }
        return Ok(());
    }

    // 语言菜单
    if data == "lang_menu" {
        let text = "语言设置\n\n当前语言: 中文";
        if let Some(msg) = &callback.message {
            bot.edit_message_text(msg.chat.id, msg.id, text)
                .reply_markup(language_keyboard("zh-CN"))
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        callback.answer("").await?;
        return Ok(());
    }

    // 设置语言
    if let Some(locale) = data.strip_prefix("lang_set:") {
        let text = format!("语言已设置为: {}", language_label(locale));
        callback.answer(&text).await?;

        // 返回主菜单
        let has_merchant_console = false;
        let keyboard = if is_admin_user {
            admin_keyboard(locale)
        } else {
            user_keyboard(locale, has_merchant_console)
        };

        if let Some(msg) = &callback.message {
            bot.edit_message_text(msg.chat.id, msg.id, "请选择功能:")
                .reply_markup(keyboard)
                .await?;
        }
        return Ok(());
    }

    // 管理后台 OTP
    if data == "admin_web_otp" {
        if !is_admin_user {
            callback.answer("权限不足").await?;
            return Ok(());
        }

        // TODO: 生成 OTP 链接
        let text = "管理后台\n\n(OTP 链接生成功能开发中)";
        if let Some(msg) = &callback.message {
            bot.edit_message_text(msg.chat.id, msg.id, text).await?;
        }
        callback.answer("").await?;
        return Ok(());
    }

    // 商户后台 OTP
    if data == "merchant_web_otp" {
        // TODO: 检查商户绑定状态并生成 OTP
        let text = "商户后台\n\n(OTP 链接生成功能开发中)";
        if let Some(msg) = &callback.message {
            bot.edit_message_text(msg.chat.id, msg.id, text).await?;
        }
        callback.answer("").await?;
        return Ok(());
    }

    // 联系客服
    if data == "contact_admin" {
        let text = "联系客服\n\n请直接发送消息，客服会尽快回复您。";
        if let Some(msg) = &callback.message {
            bot.edit_message_text(msg.chat.id, msg.id, text)
                .reply_markup(InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback("返回主菜单", "back_to_start")],
                ]))
                .await?;
        }
        callback.answer("").await?;
        return Ok(());
    }

    // 返回主菜单
    if data == "back_to_start" {
        let has_merchant_console = false;
        let keyboard = if is_admin_user {
            admin_keyboard("zh-CN")
        } else {
            user_keyboard("zh-CN", has_merchant_console)
        };

        if let Some(msg) = &callback.message {
            bot.edit_message_text(msg.chat.id, msg.id, "请选择功能:")
                .reply_markup(keyboard)
                .await?;
        }
        callback.answer("").await?;
        return Ok(());
    }

    // 系统设置（仅提示）
    if data == "admin_settings" {
        callback.answer("系统设置请在 Web 后台操作").await?;
        return Ok(());
    }

    // 注册商户
    if data == "register_merchant" {
        // TODO: 跳转商户注册流程
        let text = "商户注册\n\n(商户注册功能开发中)";
        if let Some(msg) = &callback.message {
            bot.edit_message_text(msg.chat.id, msg.id, text).await?;
        }
        callback.answer("").await?;
        return Ok(());
    }

    callback.answer("功能开发中").await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 主菜单消息处理（用户点击菜单按钮后发送的文本消息）
// ---------------------------------------------------------------------------
pub async fn main_menu_message_handler(
    bot: Bot,
    msg: Message,
    telegram_client: Arc<TelegramClient>,
) -> Result<()> {
    let user = match msg.from() {
        Some(u) => u,
        None => return Ok(()),
    };

    let user_id = user.id.0;
    let text = msg.text().unwrap_or("").trim().to_string();
    let is_admin_user = is_admin(user_id);
    let has_merchant_console = false;

    // 检查是否是语言选项
    for code in SUPPORTED_LOCALES.iter() {
        if text == language_label(code) {
            let response = format!("语言已设置为: {}", language_label(code));
            bot.send_message(msg.chat.id, response).await?;
            return Ok(());
        }
    }

    // 动作映射
    let action = main_menu_action_map("zh-CN", has_merchant_console, is_admin_user)
        .get(&text)
        .cloned();

    match action.as_deref() {
        Some("lang_menu") => {
            let text = "语言设置\n\n当前语言: 中文";
            bot.send_message(msg.chat.id, text)
                .reply_markup(language_keyboard("zh-CN"))
                .await?;
        }
        Some("back_to_start") => {
            start_command(bot, msg, vec![], telegram_client).await?;
        }
        Some("contact_admin") => {
            let text = "联系客服\n\n请直接发送消息，客服会尽快回复您。";
            bot.send_message(msg.chat.id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        Some("admin_web_otp") => {
            if !is_admin_user {
                bot.send_message(msg.chat.id, "❌ 仅管理员可使用此功能").await?;
                return Ok(());
            }
            // TODO: 生成 OTP 链接
            bot.send_message(msg.chat.id, "管理后台\n\n(OTP 链接生成功能开发中)").await?;
        }
        Some("merchant_web_otp") => {
            // TODO: 检查商户绑定并生成 OTP
            bot.send_message(msg.chat.id, "商户后台\n\n(OTP 链接生成功能开发中)").await?;
        }
        Some("admin_settings") => {
            bot.send_message(msg.chat.id, "系统设置请在 Web 后台操作").await?;
        }
        _ => {
            // 未匹配到菜单动作，静默忽略
        }
    }

    Ok(())
}