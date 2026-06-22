//! Alert Setting handler – migrated from handlers/alert_setting.py
//!
//! Provides email binding/unbinding for price alerts.
//! Users can bind an email address to receive notifications.

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode, MessageId};
use teloxide::payloads::SendMessageSetters;

use crate::handlers::HandlerResult;

// ---------------------------------------------------------------------------
// Conversation state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSettingState {
    WaitingEmail,
}

// ---------------------------------------------------------------------------
// Email validation (RFC 5322 simplified)
// ---------------------------------------------------------------------------

static EMAIL_REGEX: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+$").unwrap());

fn is_valid_email(email: &str) -> bool {
    EMAIL_REGEX.is_match(email)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Start email settings flow – display current binding and offer actions.
pub async fn email_settings_start(
    bot: &Bot,
    chat_id: ChatId,
    message_id: Option<i32>,
    user_id: i64,
    pool: &sqlx::PgPool,
) -> HandlerResult<Option<AlertSettingState>> {
    let current_email = get_user_email(pool, user_id).await;

    let text = format!(
        "📧 <b>邮箱设置</b>\n\n\
         当前绑定邮箱：<code>{}</code>\n\n\
         请选择操作：",
        if current_email.is_empty() {
            "未绑定"
        } else {
            &current_email
        }
    );

    let mut keyboard_buttons = vec![vec![InlineKeyboardButton::callback(
        "🔗 绑定/修改邮箱",
        "bind_email",
    )]];

    if !current_email.is_empty() {
        keyboard_buttons.push(vec![InlineKeyboardButton::callback(
            "❌ 解绑邮箱",
            "unbind_email",
        )]);
    }
    keyboard_buttons.push(vec![InlineKeyboardButton::callback(
        "🔙 取消",
        "cancel_email_setting",
    )]);

    let keyboard = InlineKeyboardMarkup::new(keyboard_buttons);

    if let Some(mid) = message_id {
        bot.edit_message_text(chat_id, MessageId(mid), &text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    } else {
        bot.send_message(chat_id, &text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Callback handlers
// ---------------------------------------------------------------------------

/// Prompt user to input email.
pub async fn bind_email_prompt(
    bot: &Bot,
    chat_id: ChatId,
    message_id: i32,
) -> HandlerResult<AlertSettingState> {
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "🔙 取消",
        "cancel_email_setting",
    )]]);

    bot.edit_message_text(
        chat_id,
        MessageId(message_id),
        "请输入你要绑定的邮箱地址：\n(例如: example@gmail.com)",
    )
    .reply_markup(keyboard)
    .await?;

    Ok(AlertSettingState::WaitingEmail)
}

/// Unbind email from user account.
pub async fn unbind_email(
    bot: &Bot,
    chat_id: ChatId,
    message_id: i32,
    user_id: i64,
    pool: &sqlx::PgPool,
) -> HandlerResult<()> {
    set_user_email(pool, user_id, None).await;

    bot.edit_message_text(chat_id, MessageId(message_id), "✅ 邮箱已成功解绑。")
        .await?;
    Ok(())
}

/// Cancel email setting flow.
pub async fn cancel_email_setting(
    bot: &Bot,
    chat_id: ChatId,
    message_id: Option<i32>,
) -> HandlerResult<()> {
    if let Some(mid) = message_id {
        bot.edit_message_text(chat_id, MessageId(mid), "已取消邮箱设置。")
            .await?;
    } else {
        bot.send_message(chat_id, "已取消邮箱设置。").await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Text message handler: receive email
// ---------------------------------------------------------------------------

/// Process user-submitted email address, validate, and persist.
pub async fn receive_email(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    email: &str,
    pool: &sqlx::PgPool,
) -> HandlerResult<Option<AlertSettingState>> {
    let email = email.trim();

    if !is_valid_email(email) {
        let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            "🔙 取消",
            "cancel_email_setting",
        )]]);
        bot.send_message(
            chat_id,
            "❌ 邮箱格式不正确，请重新输入：",
        )
        .reply_markup(keyboard)
        .await?;
        return Ok(Some(AlertSettingState::WaitingEmail));
    }

    // Save to DB
    set_user_email(pool, user_id, Some(email)).await;

    bot.send_message(chat_id, format!("✅ 成功绑定邮箱：{email}"))
        .await?;

    Ok(None)
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

/// Get user's email from the database.
async fn get_user_email(pool: &sqlx::PgPool, user_id: i64) -> String {
    let result: Option<String> = sqlx::query_scalar(
        "SELECT email FROM users WHERE tg_user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten();

    result.unwrap_or_default()
}

/// Set user's email. Pass `None` to clear the email.
pub async fn set_user_email(pool: &sqlx::PgPool, user_id: i64, email: Option<&str>) {
    let _ = sqlx::query(
        "UPDATE users SET email = $1, updated_at = now() WHERE tg_user_id = $2",
    )
    .bind(email)
    .bind(user_id)
    .execute(pool)
    .await;
}