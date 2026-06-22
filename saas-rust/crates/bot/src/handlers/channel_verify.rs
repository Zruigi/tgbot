//! Channel Verify handler – migrated from handlers/channel_verify.py
//!
//! Verifies whether a user has subscribed to a required channel.
//! Used for "join to unban" flows in group chats.

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode, MessageId, UserId, ChatMemberKind};
use teloxide::payloads::SendMessageSetters;
use crate::handlers::HandlerResult;

// ---------------------------------------------------------------------------
// Check channel subscription
// ---------------------------------------------------------------------------

/// Check if a user is a member of a given channel.
/// Returns true if the user is a member, administrator, or creator.
pub async fn check_channel_subscription(
    bot: &Bot,
    user_id: i64,
    channel_id: i64,
) -> bool {
    // Skip checks for non-positive user IDs (channels/anonymous)
    if user_id <= 0 {
        return true;
    }

    match bot.get_chat_member(teloxide::types::Recipient::Id(teloxide::types::ChatId(channel_id)), teloxide::types::UserId(user_id.try_into().unwrap())).await {
        Ok(member) => {
            matches!(
                member.kind,
                ChatMemberKind::Owner(_)
                    | ChatMemberKind::Administrator(_)
                    | ChatMemberKind::Member
            )
        }
        Err(e) => {
            tracing::error!("Failed to check channel member: {}", e);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Handle subscribe button callback
// ---------------------------------------------------------------------------

/// Process the "subscribe button" callback to verify and lift restrictions.
/// Callback data format: `subscribe_{group_id}_{channel_id}`
pub async fn handle_subscribe_button(
    bot: &Bot,
    query: CallbackQuery,
) -> HandlerResult<()> {
    let chat_id = match &query.message {
        Some(m) => m.chat().id.0,
        None => return Ok(()),
    };
    let message_id = query.message.as_ref().map(|m| m.id().0 as i32);
    let user_id = query.from.id.0 as i64;

    let data = match &query.data {
        Some(d) => d.clone(),
        None => return Ok(()),
    };

    let parts: Vec<&str> = data.split('_').collect();
    if parts.len() < 3 {
        bot.answer_callback_query(&query.id)
            .text("参数错误")
            .show_alert(true)
            .await?;
        return Ok(());
    }

    let group_id: i64 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => {
            bot.answer_callback_query(&query.id)
                .text("参数错误")
                .show_alert(true)
                .await?;
            return Ok(());
        }
    };

    let channel_id: i64 = match parts[2].parse() {
        Ok(v) => v,
        Err(_) => {
            bot.answer_callback_query(&query.id)
                .text("参数错误")
                .show_alert(true)
                .await?;
            return Ok(());
        }
    };

    // Check if user has subscribed
    let is_subscribed = check_channel_subscription(bot, user_id, channel_id).await;

    if is_subscribed {
        // User has subscribed – lift restrictions
        bot.restrict_chat_member(
            teloxide::types::ChatId(group_id),
            UserId(user_id.try_into().unwrap()),
            teloxide::types::ChatPermissions::all(),
        )
        .await?;

        // Confirm success
        bot.answer_callback_query(&query.id)
            .text("✅ 验证成功！您已被解禁")
            .show_alert(true)
            .await?;

        // Optionally delete the reminder message
        if let Some(mid) = message_id {
            let _ = bot.delete_message(teloxide::types::ChatId(group_id), MessageId(mid)).await;
        }

        tracing::info!(
            "User {} verified and unrestricted (group {})",
            UserId(user_id.try_into().unwrap()),
            group_id
        );
    } else {
        // User has not subscribed – show channel link
        let channel_link = build_channel_link(channel_id);

        let keyboard = InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::url(
                "📢 关注频道",
                channel_link.parse().unwrap(),
            )],
            vec![InlineKeyboardButton::callback(
                "✅ 已关注，重新验证",
                format!("subscribe_{group_id}_{channel_id}"),
            )],
        ]);

        if let Some(mid) = message_id {
            let text = format!(
                "❌ 您还未关注频道\n\n\
                 请先关注我们的频道，然后点击\"已关注，重新验证\"按钮。\n\n\
                 📢 频道链接: {}",
                channel_link
            );
            bot.edit_message_text(teloxide::types::ChatId(group_id), MessageId(mid), &text)
                .reply_markup(keyboard)
                .await?;
        }

        bot.answer_callback_query(&query.id)
            .text("请先关注频道")
            .show_alert(false)
            .await?;
    }

    Ok(())
}

/// Build a Telegram channel link from the channel ID.
fn build_channel_link(channel_id: i64) -> String {
    // -100xxx... format for private channels
    if channel_id < 0 {
        format!("https://t.me/c/{}", channel_id.to_string().trim_start_matches("-100"))
    } else {
        format!("https://t.me/{}", channel_id)
    }
}

// ---------------------------------------------------------------------------
// Send channel reminder message
// ---------------------------------------------------------------------------

/// Send a channel subscription reminder to a user in a group.
/// Returns the message ID if successful.
pub async fn send_channel_reminder_message(
    bot: &Bot,
    group_id: i64,
    user_id: i64,
    username: Option<&str>,
    channel_id: i64,
    channel_link: &str,
) -> HandlerResult<i32> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::url(
            "📢 直接关注",
            channel_link.parse().unwrap_or_else(|_| "https://t.me".parse().unwrap()),
        )],
        vec![InlineKeyboardButton::callback(
            "✅ 回补订阅解开禁言",
            format!("subscribe_{group_id}_{channel_id}"),
        )],
    ]);

    // Build user mention
    let mention = match username {
        Some(name) if !name.is_empty() => format!("@{}", name),
        _ => format!("<a href=\"tg://user?id={}\">用户</a>", user_id),
    };

    let text = format!(
        "🔔 {mention}，请关注我们的频道以获得最新信息！\n\n\
         📢 频道链接: {channel_link}\n\
         🤖 或点击下方按钮: 回补订阅解开禁言\n\n\
         ⏰ 此消息将在12小时后自动删除",
        mention = mention,
        channel_link = channel_link
    );

    let msg = bot
        .send_message(teloxide::types::ChatId(group_id), text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    tracing::info!(
        "Sent reminder message to user {} (group {}, message {})",
        username.unwrap_or(&user_id.to_string()),
        group_id,
        msg.id.0
    );

    Ok(msg.id.0 as i32)
}

// ---------------------------------------------------------------------------
// Deep-link /start with channel verification
// ---------------------------------------------------------------------------

/// Handle /start with channel verification parameter.
/// Format: /start channel_{group_id}_{channel_id}
pub async fn handle_start_with_channel_check(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    args: &[String],
) -> HandlerResult<()> {
    if args.is_empty() {
        // No argument – just welcome
        bot.send_message(
            chat_id,
            "👋 欢迎使用本机器人！\n\n请关注我们的频道以获得最新信息。",
        )
        .await?;
        return Ok(());
    }

    let param = &args[0];

    if let Some(stripped) = param.strip_prefix("channel_") {
        let parts: Vec<&str> = stripped.split('_').collect();
        if parts.len() >= 2 {
            let group_id: i64 = match parts[0].parse() {
                Ok(v) => v,
                Err(_) => {
                    bot.send_message(chat_id, "处理失败，请稍后重试").await?;
                    return Ok(());
                }
            };
            let channel_id: i64 = match parts[1].parse() {
                Ok(v) => v,
                Err(_) => {
                    bot.send_message(chat_id, "处理失败，请稍后重试").await?;
                    return Ok(());
                }
            };

            // Check subscription
            let is_subscribed = check_channel_subscription(bot, user_id, channel_id).await;

            if is_subscribed {
                // Lift restrictions
                bot.restrict_chat_member(
                    teloxide::types::ChatId(group_id),
                    UserId(user_id.try_into().unwrap()),
                    teloxide::types::ChatPermissions::all(),
                )
                .await?;

                bot.send_message(
                    chat_id,
                    "✅ 验证成功！您已被解禁。\n\n感谢您关注我们的频道！",
                )
                .await?;

                tracing::info!(
                    "User {} verified via deep link and unrestricted (group {})",
                    UserId(user_id.try_into().unwrap()),
                    group_id
                );
            } else {
                // Show channel link
                let channel_link = build_channel_link(channel_id);

                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::url(
                        "📢 关注频道",
                        channel_link.parse().unwrap_or_else(|_| "https://t.me".parse().unwrap()),
                    )],
                    vec![InlineKeyboardButton::callback(
                        "✅ 已关注，重新验证",
                        format!("subscribe_{group_id}_{channel_id}"),
                    )],
                ]);

                bot.send_message(
                    chat_id,
                    format!(
                        "❌ 您还未关注频道\n\n\
                         请先关注我们的频道，然后点击\"已关注，重新验证\"按钮。\n\n\
                         📢 频道链接: {}",
                        channel_link
                    ),
                )
                .reply_markup(keyboard)
                .await?;
            }
        }
    }

    Ok(())
}