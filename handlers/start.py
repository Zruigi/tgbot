# -*- coding: utf-8 -*-
"""
/start 命令处理，提供主菜单
"""
import logging

from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.ext import ContextTypes
import config

logger = logging.getLogger(__name__)


async def restart_bot_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """仅管理员：优雅停止 Application 并由 main 中 os.execl 拉起新进程（便于本地测试）。"""
    user = update.effective_user
    if not user or not update.message:
        return
    if str(user.id) != str(config.ADMIN_ID):
        await update.message.reply_text("❌ 仅管理员可使用此命令。")
        return
    await update.message.reply_text("🔄 正在重启 Bot，请稍候…")
    config.RESTART_REQUESTED = True
    context.application.stop_running()


async def start(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理 /start 命令，显示主菜单"""
    user = update.effective_user
    if not user:
        return

    logger.info(f"Checking access for user {user.id} (Config ADMIN_ID: {config.ADMIN_ID})")

    if str(user.id) == str(config.ADMIN_ID):
        # 管理员菜单
        keyboard = [
            [InlineKeyboardButton("🌐 Web 管理后台", callback_data="admin_dashboard"),
             InlineKeyboardButton("⚙️ 系统设置", callback_data="adm:main")],
            [InlineKeyboardButton("📈 现货行情", callback_data="spot_price"),
             InlineKeyboardButton("📉 合约行情", callback_data="futures_price")],
            [InlineKeyboardButton("👛 监听钱包地址", callback_data="wallet_monitor"),
             InlineKeyboardButton("📋 我的监听列表", callback_data="wallet_my_list")],
            [InlineKeyboardButton("📧 邮箱设置", callback_data="email_settings")],
        ]
        text = (
            f"👋 欢迎回来，{user.mention_html()}\n\n"
            "<b>管理员控制台</b> — 请选择功能："
        )
    else:
        # 普通用户菜单
        keyboard = [
            [InlineKeyboardButton("💬 联系人工客服", callback_data="contact_admin")],
            [InlineKeyboardButton("📈 现货行情", callback_data="spot_price"),
             InlineKeyboardButton("📉 合约行情", callback_data="futures_price")],
            [InlineKeyboardButton("👛 监听钱包地址", callback_data="wallet_monitor"),
             InlineKeyboardButton("📋 我的监听列表", callback_data="wallet_my_list")],
            [InlineKeyboardButton("📧 邮箱设置", callback_data="email_settings")],
        ]
        text = (
            f"👋 你好，{user.mention_html()}！\n\n"
            "我是智能助手，可以帮你：\n"
            "• 📊 查看实时币价行情\n"
            "• 👛 监听 USDT 钱包变动\n"
            "• 💬 联系人工客服\n\n"
            "<i>也可以直接发消息给我，我会转达给客服。</i>"
        )

    reply_markup = InlineKeyboardMarkup(keyboard)

    # 智能判断：是新消息还是回调按钮点击
    if update.message:
        await update.message.reply_html(text, reply_markup=reply_markup)
    elif update.callback_query:
        try:
            await update.callback_query.edit_message_text(text, reply_markup=reply_markup, parse_mode="HTML")
        except Exception as e:
            if update.callback_query and update.callback_query.message:
                await update.callback_query.message.reply_html(text, reply_markup=reply_markup)
            else:
                await context.bot.send_message(chat_id=user.id, text=text, reply_markup=reply_markup, parse_mode="HTML")

async def button_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理主菜单按钮点击（未实现的按钮）"""
    query = update.callback_query
    if not query:
        return
        
    await query.answer()
    
    data = query.data
    if not data:
        return
        
    if data == "cs_mode_test":
        await query.edit_message_text(
            "🎧 <b>双向客服模式已激活 (TG端测试)</b>\n\n"
            "现在你可以直接回复任何转发给你的消息，我会实时同步到 Web 端并转达给用户。\n\n"
            "<b>测试步骤：</b>\n"
            "1. 请让一个测试账号给机器人发消息\n"
            "2. 你在这里收到消息后，直接<b>回复</b>该消息\n"
            "3. 检查 Web 端是否同步显示了你的回复\n\n"
            "<i>提示：如果你在这里看不到同步，请确保你的 .env 中 ADMIN_ID 设置正确。</i>",
            parse_mode="HTML"
        )
    elif data == "admin_dashboard":
        # 2026 规格：提供带免登 Token 的 Web 链接
        admin_link = f"{config.WEB_BASE_URL}/?token={config.WEB_ADMIN_TOKEN}"
        text = (
            "🚀 <b>Web 管理端已准备就绪</b>\n\n"
            "你可以点击下方按钮免密登录后台管理系统，实时处理客户消息、监控链上交易及查看潜在用户。\n\n"
            "<i>提示：该链接包含管理凭证，请勿转发给他人。</i>"
        )
        keyboard = [
            [InlineKeyboardButton("🌐 立即进入管理后台", url=admin_link)],
            [InlineKeyboardButton("⬅️ 返回主菜单", callback_data="back_to_start")]
        ]
        await query.edit_message_text(text, reply_markup=InlineKeyboardMarkup(keyboard), parse_mode="HTML")
    elif data == "contact_admin":
        await query.edit_message_text("你可以直接在这个聊天框发送任何消息、图片或文件，我会实时转达给管理员。")
    elif data == "debug_id":
        if query.from_user:
            await query.answer(f"你的当前 ID 是: {query.from_user.id}", show_alert=True)
    elif data == "restart_bot":
        if query.from_user and str(query.from_user.id) != str(config.ADMIN_ID):
            await query.answer("无权限", show_alert=True)
            return
        await query.answer("正在重启…")
        await query.edit_message_text("🔄 正在重启 Bot，请稍候…")
        config.RESTART_REQUESTED = True
        context.application.stop_running()
    elif data == "back_to_start":
        await start(update, context)
    else:
        # 其他未处理的按钮逻辑，不再直接返回开发中，而是静默忽略或返回提示
        await query.answer("该功能正在维护中...", show_alert=True)
