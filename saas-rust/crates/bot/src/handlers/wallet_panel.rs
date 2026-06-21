//! Wallet Panel handler – migrated from handlers/wallet_panel.py
//!
//! Displays a user's watched wallet addresses with inline keyboards
//! for adding new addresses or cancelling existing monitors.
//! Admin users can view all monitors system-wide.

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

use crate::handlers::HandlerResult;

/// Maximum number of monitor rows to display per message.
const MAX_ROWS: usize = 25;

// ---------------------------------------------------------------------------
// Monitor record (read from DB)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MonitorRecord {
    pub monitor_id: i64,
    pub address: String,
    pub network: String,
    pub user_id: Option<i64>,
}

/// Shorten an address for display.
fn short_addr(addr: &str) -> &str {
    if addr.len() <= 18 {
        addr
    } else if addr.len() > 16 {
        // Return the full string; we'll format separately
        addr
    } else {
        addr
    }
}

fn format_short_addr(addr: &str) -> String {
    if addr.len() <= 18 {
        addr.to_string()
    } else {
        format!("{}…{}", &addr[..10], &addr[addr.len() - 6..])
    }
}

// ---------------------------------------------------------------------------
// Keyboard builder
// ---------------------------------------------------------------------------

/// Build the wallet list text and keyboard rows.
/// When `admin_view_all` is true, lists all active monitors (admin only).
pub async fn build_wallet_keyboard(
    pool: &sqlx::PgPool,
    user_id: i64,
    admin_view_all: bool,
) -> (String, Vec<Vec<InlineKeyboardButton>>) {
    let monitors = if admin_view_all {
        get_all_monitors_for_admin(pool, true).await
    } else {
        get_monitors_by_user(pool, user_id, true).await
    };

    let (title, hint) = if admin_view_all {
        (
            "🛠 <b>全部活跃监听地址</b>\n\n",
            "以下为系统中所有 <b>活跃</b> 监听；你可取消任意一条（测试清理）。\n\n",
        )
    } else {
        (
            "📋 <b>我的监听地址</b>\n\n",
            "以下为当前正在推送的 USDT 链上监听；点「取消」即停止该地址通知。\n\n",
        )
    };

    if monitors.is_empty() {
        if admin_view_all {
            let text = "🛠 <b>全部活跃监听</b>\n\n📭 当前系统中没有活跃监听。";
            let kb = vec![
                vec![InlineKeyboardButton::callback("🔄 刷新", "wallet_admin_all")],
                vec![InlineKeyboardButton::callback(
                    "🔙 返回主菜单",
                    "back_to_start",
                )],
            ];
            return (text.to_string(), kb);
        } else {
            let text = format!("{title}📭 暂无监听中的地址。\n\n点击下方添加新地址。");
            let kb = vec![
                vec![InlineKeyboardButton::callback(
                    "👛 添加监听地址",
                    "wallet_monitor",
                )],
                vec![InlineKeyboardButton::callback(
                    "🔙 返回主菜单",
                    "back_to_start",
                )],
            ];
            return (text, kb);
        }
    }

    let mut lines = vec![format!("{title}{hint}")];
    let mut kb = Vec::new();

    for m in monitors.iter().take(MAX_ROWS) {
        let net = m.network.to_uppercase();
        if admin_view_all {
            lines.push(format!(
                "• <code>{net}</code> | 用户 <code>{}</code>\n  <code>{}</code>",
                m.user_id.unwrap_or(0),
                html_escape(&m.address),
            ));
        } else {
            lines.push(format!("• <code>{net}</code>\n  <code>{}</code>", html_escape(&m.address)));
        }

        let short = format_short_addr(&m.address);
        let mut label = format!("❌ 取消 {net} {short}");
        if label.len() > 60 {
            label = format!("❌ 取消 #{}", m.monitor_id);
        }

        let cb = if admin_view_all {
            format!("uw_a_{}", m.monitor_id)
        } else {
            format!("uw_u_{}", m.monitor_id)
        };
        kb.push(vec![InlineKeyboardButton::callback(label, cb)]);
    }

    if monitors.len() > MAX_ROWS {
        lines.push(format!("\n… 共 {} 条，仅显示前 {MAX_ROWS} 条。", monitors.len()));
    }

    if admin_view_all {
        kb.push(vec![InlineKeyboardButton::callback(
            "🔄 刷新列表",
            "wallet_admin_all",
        )]);
    }
    kb.push(vec![InlineKeyboardButton::callback(
        "👛 添加监听地址",
        "wallet_monitor",
    )]);
    kb.push(vec![InlineKeyboardButton::callback(
        "🔙 返回主菜单",
        "back_to_start",
    )]);

    (lines.join("\n"), kb)
}

/// Minimal HTML entity escaping for text inside `<code>` tags.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Command: /wallets
// ---------------------------------------------------------------------------

/// Handle the /wallets command – show user's own monitor list.
pub async fn my_wallets_command(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    pool: &sqlx::PgPool,
) -> HandlerResult<()> {
    let (text, kb) = build_wallet_keyboard(pool, user_id, false).await;
    bot.send_message(chat_id, &text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(kb))
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Callback: wallet_my_list
// ---------------------------------------------------------------------------

/// Show the user's own wallet monitor list (callback version).
pub async fn wallet_my_list_callback(
    bot: &Bot,
    chat_id: ChatId,
    message_id: i32,
    user_id: i64,
    pool: &sqlx::PgPool,
) -> HandlerResult<()> {
    let (text, kb) = build_wallet_keyboard(pool, user_id, false).await;
    bot.edit_message_text(chat_id, message_id, &text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(kb))
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Callback: wallet_admin_all
// ---------------------------------------------------------------------------

/// Show all active monitors (admin only).
pub async fn wallet_admin_all_callback(
    bot: &Bot,
    chat_id: ChatId,
    message_id: i32,
    user_id: i64,
    admin_id: i64,
    pool: &sqlx::PgPool,
) -> HandlerResult<()> {
    if user_id != admin_id {
        bot.answer_callback_query(
            &teloxide::types::CallbackQuery::default(), // placeholder; real usage passes query id
        )
        .text("无权限")
        .show_alert(true)
        .await
        .ok();
        // In production, the caller handles the callback answer before invoking this.
        return Ok(());
    }

    let (text, kb) = build_wallet_keyboard(pool, user_id, true).await;
    bot.edit_message_text(chat_id, message_id, &text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(kb))
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Callback: unwatch (uw_u_{mid} or uw_a_{mid})
// ---------------------------------------------------------------------------

/// Handle unwatch callback to deactivate a wallet monitor.
pub async fn unwatch_callback(
    bot: &Bot,
    chat_id: ChatId,
    message_id: i32,
    user_id: i64,
    admin_id: i64,
    callback_data: &str,
    pool: &sqlx::PgPool,
) -> HandlerResult<()> {
    let parts: Vec<&str> = callback_data.split('_').collect();
    if parts.len() != 3 || parts[0] != "uw" {
        return Ok(());
    }

    let mode = parts[1];
    if mode != "u" && mode != "a" {
        return Ok(());
    }

    let mid: i64 = match parts[2].parse() {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    // Admin mode requires admin privileges
    if mode == "a" && user_id != admin_id {
        return Ok(());
    }

    let ok = deactivate_wallet_monitor(pool, mid, user_id).await;
    if !ok {
        return Ok(());
    }

    let admin_view = mode == "a";
    let (text, kb) = build_wallet_keyboard(pool, user_id, admin_view).await;
    bot.edit_message_text(chat_id, message_id, &text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(kb))
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// DB queries
// ---------------------------------------------------------------------------

/// Get all active monitors for a specific user.
async fn get_monitors_by_user(
    pool: &sqlx::PgPool,
    user_id: i64,
    active_only: bool,
) -> Vec<MonitorRecord> {
    let query = if active_only {
        "SELECT monitor_id, address, network, user_id FROM wallet_monitors WHERE user_id = $1 AND active = true ORDER BY created_at DESC"
    } else {
        "SELECT monitor_id, address, network, user_id FROM wallet_monitors WHERE user_id = $1 ORDER BY created_at DESC"
    };

    let rows = sqlx::query(query)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    rows.iter()
        .map(|row| MonitorRecord {
            monitor_id: row.try_get("monitor_id").unwrap_or(0),
            address: row.try_get("address").unwrap_or_default(),
            network: row.try_get("network").unwrap_or_default(),
            user_id: row.try_get("user_id").ok(),
        })
        .collect()
}

/// Get all active monitors (admin view).
async fn get_all_monitors_for_admin(
    pool: &sqlx::PgPool,
    active_only: bool,
) -> Vec<MonitorRecord> {
    let query = if active_only {
        "SELECT monitor_id, address, network, user_id FROM wallet_monitors WHERE active = true ORDER BY created_at DESC LIMIT 100"
    } else {
        "SELECT monitor_id, address, network, user_id FROM wallet_monitors ORDER BY created_at DESC LIMIT 100"
    };

    let rows = sqlx::query(query)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    rows.iter()
        .map(|row| MonitorRecord {
            monitor_id: row.try_get("monitor_id").unwrap_or(0),
            address: row.try_get("address").unwrap_or_default(),
            network: row.try_get("network").unwrap_or_default(),
            user_id: row.try_get("user_id").ok(),
        })
        .collect()
}

/// Deactivate a wallet monitor. Returns true if the operation succeeded.
pub async fn deactivate_wallet_monitor(
    pool: &sqlx::PgPool,
    monitor_id: i64,
    user_id: i64,
) -> bool {
    let result = sqlx::query(
        "UPDATE wallet_monitors SET active = false, updated_at = now() WHERE monitor_id = $1 AND user_id = $2 AND active = true",
    )
    .bind(monitor_id)
    .bind(user_id)
    .execute(pool)
    .await;

    match result {
        Ok(r) => r.rows_affected() > 0,
        Err(e) => {
            tracing::error!("Failed to deactivate monitor {monitor_id}: {e}");
            false
        }
    }
}
