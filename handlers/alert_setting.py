# -*- coding: utf-8 -*-
"""
提醒设置与邮箱绑定处理
"""
import re
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.ext import (
    ContextTypes,
    ConversationHandler,
    CommandHandler,
    MessageHandler,
    CallbackQueryHandler,
    filters,
)
from database import create_or_update_user, get_user

# 状态定义
WAITING_EMAIL = 1

# 邮箱正则
EMAIL_REGEX = re.compile(r"^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+$")

async def email_settings_start(update: Update, context: ContextTypes.DEFAULT_TYPE) -> int:
    """开始邮箱设置流程"""
    user_id = update.effective_user.id
    user = await get_user(user_id)
    
    current_email = user['email'] if user and user['email'] else "未绑定"
    
    text = f"📧 <b>邮箱设置</b>\n\n当前绑定邮箱：<code>{current_email}</code>\n\n请选择操作："
    
    keyboard = [
        [InlineKeyboardButton("🔗 绑定/修改邮箱", callback_data="bind_email")],
    ]
    if current_email != "未绑定":
        keyboard.append([InlineKeyboardButton("❌ 解绑邮箱", callback_data="unbind_email")])
    keyboard.append([InlineKeyboardButton("🔙 取消", callback_data="cancel_email_setting")])
    
    reply_markup = InlineKeyboardMarkup(keyboard)
    
    if update.callback_query:
        await update.callback_query.answer()
        await update.callback_query.edit_message_text(text, reply_markup=reply_markup, parse_mode="HTML")
    else:
        await update.message.reply_html(text, reply_markup=reply_markup)
        
    return ConversationHandler.END

async def bind_email_prompt(update: Update, context: ContextTypes.DEFAULT_TYPE) -> int:
    """提示用户输入邮箱"""
    query = update.callback_query
    await query.answer()
    
    keyboard = [[InlineKeyboardButton("🔙 取消", callback_data="cancel_email_setting")]]
    reply_markup = InlineKeyboardMarkup(keyboard)
    
    await query.edit_message_text(
        "请输入你要绑定的邮箱地址：\n(例如: example@gmail.com)",
        reply_markup=reply_markup
    )
    return WAITING_EMAIL

async def receive_email(update: Update, context: ContextTypes.DEFAULT_TYPE) -> int:
    """接收并验证邮箱"""
    email = update.message.text.strip()
    user_id = update.effective_user.id
    
    if not EMAIL_REGEX.match(email):
        keyboard = [[InlineKeyboardButton("🔙 取消", callback_data="cancel_email_setting")]]
        reply_markup = InlineKeyboardMarkup(keyboard)
        await update.message.reply_text(
            "❌ 邮箱格式不正确，请重新输入：",
            reply_markup=reply_markup
        )
        return WAITING_EMAIL
        
    # 保存到数据库
    await create_or_update_user(user_id, email)
    
    await update.message.reply_text(f"✅ 成功绑定邮箱：{email}")
    return ConversationHandler.END

async def unbind_email(update: Update, context: ContextTypes.DEFAULT_TYPE) -> int:
    """解绑邮箱"""
    query = update.callback_query
    await query.answer()
    user_id = update.effective_user.id
    
    await create_or_update_user(user_id, None)
    await query.edit_message_text("✅ 邮箱已成功解绑。")
    return ConversationHandler.END

async def cancel_email_setting(update: Update, context: ContextTypes.DEFAULT_TYPE) -> int:
    """取消设置"""
    query = update.callback_query
    await query.answer()
    await query.edit_message_text("已取消邮箱设置。")
    return ConversationHandler.END

# 导出 ConversationHandler 供 main.py 注册
email_conv_handler = ConversationHandler(
    entry_points=[
        CommandHandler("email", email_settings_start),
        CallbackQueryHandler(email_settings_start, pattern="^email_settings$")
    ],
    states={
        WAITING_EMAIL: [
            MessageHandler(filters.TEXT & ~filters.COMMAND, receive_email)
        ],
    },
    fallbacks=[
        CallbackQueryHandler(bind_email_prompt, pattern="^bind_email$"),
        CallbackQueryHandler(unbind_email, pattern="^unbind_email$"),
        CallbackQueryHandler(cancel_email_setting, pattern="^cancel_email_setting$"),
        CommandHandler("cancel", cancel_email_setting)
    ],
)
