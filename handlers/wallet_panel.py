# -*- coding: utf-8 -*-
"""
钱包监听地址：查看与取消绑定（用户与管理员共用逻辑）
"""
import logging
from html import escape

from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.ext import ContextTypes

import config
import database

logger = logging.getLogger(__name__)

MAX_ROWS = 25  # 单条消息内最多展示条数，避免超长


def _short_addr(addr: str) -> str:
    if len(addr) <= 18:
        return addr
    return f"{addr[:10]}…{addr[-6:]}"


async def build_wallet_keyboard(user_id: int, admin_view_all: bool = False):
    """
    admin_view_all=True 时列出全库活跃监听（仅应用层限制为管理员回调）。
    返回 (text_html, keyboard_rows)
    """
    if admin_view_all:
        monitors = await database.get_all_monitors_for_admin(active_only=True)
        title = "🛠 <b>全部活跃监听地址</b>\n\n"
        hint = "以下为系统中所有 <b>活跃</b> 监听；你可取消任意一条（测试清理）。\n\n"
    else:
        monitors = await database.get_monitors_by_user(user_id, active_only=True)
        title = "📋 <b>我的监听地址</b>\n\n"
        hint = "以下为当前正在推送的 USDT 链上监听；点「取消」即停止该地址通知。\n\n"

    if not monitors:
        if admin_view_all:
            text = "🛠 <b>全部活跃监听</b>\n\n📭 当前系统中没有活跃监听。"
            kb = [
                [InlineKeyboardButton("🔄 刷新", callback_data="wallet_admin_all")],
                [InlineKeyboardButton("🔙 返回主菜单", callback_data="back_to_start")],
            ]
        else:
            text = title + "📭 暂无监听中的地址。\n\n点击下方添加新地址。"
            kb = [
                [InlineKeyboardButton("👛 添加监听地址", callback_data="wallet_monitor")],
                [InlineKeyboardButton("🔙 返回主菜单", callback_data="back_to_start")],
            ]
        return text, kb

    lines = [title + hint]
    kb = []
    for m in monitors[:MAX_ROWS]:
        mid = m["monitor_id"]
        addr = m["address"]
        net = str(m.get("network", "")).upper()
        uid = m.get("user_id")
        if admin_view_all:
            lines.append(
                f"• <code>{net}</code> | 用户 <code>{uid}</code>\n  <code>{escape(addr)}</code>"
            )
        else:
            lines.append(f"• <code>{net}</code>\n  <code>{escape(addr)}</code>")
        label = f"❌ 取消 {net} {_short_addr(addr)}"
        if len(label) > 60:
            label = f"❌ 取消 #{mid}"
        cb = f"uw_a_{mid}" if admin_view_all else f"uw_u_{mid}"
        kb.append([InlineKeyboardButton(label, callback_data=cb)])

    if len(monitors) > MAX_ROWS:
        lines.append(f"\n… 共 {len(monitors)} 条，仅显示前 {MAX_ROWS} 条。")

    if admin_view_all:
        kb.append([InlineKeyboardButton("🔄 刷新列表", callback_data="wallet_admin_all")])
    kb.append([InlineKeyboardButton("👛 添加监听地址", callback_data="wallet_monitor")])
    kb.append([InlineKeyboardButton("🔙 返回主菜单", callback_data="back_to_start")])

    text = "\n".join(lines)
    return text, kb


async def my_wallets_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """命令 /wallets — 私聊展示我的监听列表"""
    if not update.message:
        return
    text, kb = await build_wallet_keyboard(update.effective_user.id, admin_view_all=False)
    await update.message.reply_html(text, reply_markup=InlineKeyboardMarkup(kb))


async def wallet_my_list_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    query = update.callback_query
    await query.answer()
    text, kb = await build_wallet_keyboard(query.from_user.id, admin_view_all=False)
    await query.edit_message_text(
        text, reply_markup=InlineKeyboardMarkup(kb), parse_mode="HTML"
    )


async def wallet_admin_all_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    query = update.callback_query
    if str(query.from_user.id) != str(config.ADMIN_ID):
        await query.answer("无权限", show_alert=True)
        return
    await query.answer()
    text, kb = await build_wallet_keyboard(query.from_user.id, admin_view_all=True)
    await query.edit_message_text(
        text, reply_markup=InlineKeyboardMarkup(kb), parse_mode="HTML"
    )


async def unwatch_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    query = update.callback_query
    parts = query.data.split("_")
    if len(parts) != 3 or parts[0] != "uw" or parts[1] not in ("u", "a"):
        await query.answer("数据无效", show_alert=True)
        return
    mode = parts[1]
    try:
        mid = int(parts[2])
    except ValueError:
        await query.answer("数据无效", show_alert=True)
        return
    if mode == "a" and str(query.from_user.id) != str(config.ADMIN_ID):
        await query.answer("无权限", show_alert=True)
        return
    ok = await database.deactivate_wallet_monitor(mid, query.from_user.id)
    if not ok:
        await query.answer("已取消或无权操作", show_alert=True)
        return
    await query.answer("已取消监听")
    admin_view = mode == "a"
    text, kb = await build_wallet_keyboard(
        query.from_user.id, admin_view_all=admin_view
    )
    await query.edit_message_text(
        text, reply_markup=InlineKeyboardMarkup(kb), parse_mode="HTML"
    )
