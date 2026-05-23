# -*- coding: utf-8 -*-
"""
商户自助接入对话流程
"""
import logging
import re

from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup, WebAppInfo
from telegram.ext import (
    ContextTypes,
    ConversationHandler,
    CommandHandler,
    MessageHandler,
    CallbackQueryHandler,
    filters,
)

from backend.repositories import merchant_repo, subscription_repo
from backend.app.services.merchant_onboarding_service import get_merchant_onboarding_service
from backend.app.services import get_subscription_lifecycle_service
from services.i18n import resolve_user_locale, tr


logger = logging.getLogger(__name__)

WAITING_MERCHANT_NAME = 1
WAITING_PLAN_SELECTION = 2


def build_register_merchant_text_pattern() -> str:
    labels = set()
    for locale in ("zh-CN", "en", "ja", "ru"):
        labels.add(tr(locale, "buttons.register_bot"))
    return r"^(?:" + "|".join(sorted(re.escape(item) for item in labels if item)) + r")$"


async def build_merchant_console_keyboard(user_id: int, locale: str = "zh-CN") -> InlineKeyboardMarkup:
    from handlers.start import _build_merchant_entry_urls

    miniapp_url, browser_url = await _build_merchant_entry_urls(user_id)
    return InlineKeyboardMarkup([
        [InlineKeyboardButton(tr(locale, "buttons.open_merchant_console"), web_app=WebAppInfo(url=miniapp_url))],
        [InlineKeyboardButton(tr(locale, "buttons.browser_open"), url=browser_url)],
        [InlineKeyboardButton(tr(locale, "buttons.back_to_main"), callback_data="back_to_start")],
    ])


async def start_merchant_onboarding(update: Update, context: ContextTypes.DEFAULT_TYPE):
    query = update.callback_query
    user = update.effective_user
    if query:
        await query.answer()
    if not user:
        return ConversationHandler.END
    locale = await resolve_user_locale(user.id, getattr(user, "language_code", None))

    merchant = await merchant_repo.get_merchant_by_owner(user.id)
    if merchant:
        try:
            text = (
                "🏪 <b>你已经有商户档案了</b>\n\n"
                "我直接带你进入商户控制台即可。"
            )
            keyboard = await build_merchant_console_keyboard(user.id, locale)
            if query:
                await query.edit_message_text(text, reply_markup=keyboard, parse_mode="HTML")
            else:
                await context.bot.send_message(chat_id=user.id, text=text, reply_markup=keyboard, parse_mode="HTML")
        except Exception as e:
            logger.error("existing merchant otp failed: %s", e, exc_info=True)
            if query:
                await query.edit_message_text("⚠️ 已检测到你的商户档案，但暂时无法生成控制台链接，请稍后重试。")
        return ConversationHandler.END

    text = (
        "🤖 <b>注册机器人</b>\n\n"
        "我会先为你创建一个商户档案，然后引导你去创建自己的 Telegram 机器人。\n\n"
        "你也可以直接打开 <a href=\"https://t.me/BotFather\">@BotFather</a> 提前创建机器人并获取 Token。\n\n"
        "请先发送你的项目名或商户名称。\n"
        "例如：<code>星海社区助手</code>\n\n"
        "发送 /cancel 可取消。"
    )
    if query:
        await query.edit_message_text(text, parse_mode="HTML")
    elif update.message:
        await update.message.reply_html(text)
    return WAITING_MERCHANT_NAME


async def receive_merchant_name(update: Update, context: ContextTypes.DEFAULT_TYPE):
    if not update.message or not update.message.text or not update.effective_user:
        return WAITING_MERCHANT_NAME

    user = update.effective_user
    locale = await resolve_user_locale(user.id, getattr(user, "language_code", None))
    merchant_name = update.message.text.strip()
    if len(merchant_name) < 2:
        await update.message.reply_text("❌ 名称太短了，请至少输入 2 个字。")
        return WAITING_MERCHANT_NAME
    if len(merchant_name) > 40:
        await update.message.reply_text("❌ 名称太长了，请控制在 40 个字以内。")
        return WAITING_MERCHANT_NAME

    merchant = await merchant_repo.get_merchant_by_owner(user.id)
    if merchant is None:
        try:
            merchant = await get_merchant_onboarding_service().create_merchant(
                owner_tg_user_id=user.id,
                merchant_name=merchant_name,
                bot_token=None,
            )
        except Exception as e:
            logger.error("create merchant failed: %s", e, exc_info=True)
            await update.message.reply_text("⚠️ 商户档案创建失败，请稍后再试。")
            return ConversationHandler.END

    # Show plan selection
    try:
        plans = await subscription_repo.list_plans(active_only=True)
    except Exception as e:
        logger.error("list plans failed: %s", e, exc_info=True)
        plans = []

    if not plans:
        # No plans available, skip to success message (original flow)
        try:
            keyboard = await build_merchant_console_keyboard(user.id, locale)
        except Exception as e:
            logger.error("build merchant otp failed: %s", e, exc_info=True)
            await update.message.reply_text("⚠️ 商户档案已创建，但暂时无法生成控制台链接，请稍后再试。")
            return ConversationHandler.END

        text = (
            "✅ <b>商户档案已创建</b>\n\n"
            f"商户名称：<code>{merchant.get('merchant_name') or merchant_name}</code>\n"
            f"租户标识：<code>{merchant.get('tenant_id') or '-'}</code>\n\n"
            "接下来这样做：\n"
            "1. 点击 <a href=\"https://t.me/BotFather\">@BotFather</a> 创建你的机器人\n"
            "2. 复制 BotFather 返回的 Bot Token\n"
            "3. 点击下方按钮进入商户控制台\n"
            "4. 在控制台里提交 Bot Token 完成接入\n"
        )
        await update.message.reply_html(text, reply_markup=keyboard)
        return ConversationHandler.END

    # Build plan selection keyboard
    rows = []
    for item in plans:
        plan_code = str(item.get('plan_code') or '').strip()
        if not plan_code:
            continue
        price = float(item.get('price_usdt') or 0)
        label = f"{item.get('plan_name') or plan_code} · {price} USDT / {item.get('duration_days') or 30}天"
        rows.append([InlineKeyboardButton(label, callback_data=f"select_plan:{plan_code}")])

    rows.append([InlineKeyboardButton("⏭️ 暂时使用免费套餐", callback_data="select_plan:free")])

    plan_keyboard = InlineKeyboardMarkup(rows)

    text = (
        "✅ <b>商户档案已创建</b>\n\n"
        f"商户名称：<code>{merchant.get('merchant_name') or merchant_name}</code>\n"
        f"租户标识：<code>{merchant.get('tenant_id') or '-'}</code>\n\n"
        "请选择一个套餐开始使用：\n"
        "（付费套餐需要完成支付后才能激活）"
    )
    await update.message.reply_html(text, reply_markup=plan_keyboard)
    return WAITING_PLAN_SELECTION


async def receive_plan_selection(update: Update, context: ContextTypes.DEFAULT_TYPE):
    query = update.callback_query
    if not query or not query.data:
        return WAITING_PLAN_SELECTION

    await query.answer()

    user = update.effective_user
    if not user:
        return ConversationHandler.END
    locale = await resolve_user_locale(user.id, getattr(user, "language_code", None))

    data = query.data  # format: "select_plan:plan_code" or "select_plan:free"
    parts = data.split(":", 1)
    if len(parts) < 2:
        return WAITING_PLAN_SELECTION

    plan_code = parts[1].strip()
    merchant = await merchant_repo.get_merchant_by_owner(user.id)
    if not merchant:
        await query.edit_message_text("⚠️ 商户档案未找到，请重新注册。")
        return ConversationHandler.END

    merchant_id = int(merchant.get('id') or 0)

    if plan_code == 'free':
        # User chose free plan — update merchant status and show success message
        try:
            await merchant_repo.update_merchant(merchant_id, {
                'status': 'active_free',
                'plan_type': 'free',
                'subscription_status': 'active',
                'onboarding_completed': 1,
            })

            logger.info(f"Merchant {merchant_id} selected free plan, status updated to active_free")
        except Exception as e:
            logger.error(f"Failed to update merchant status for free plan: {e}", exc_info=True)
        
        try:
            keyboard = await build_merchant_console_keyboard(user.id, locale)
        except Exception as e:
            logger.error("build merchant otp failed: %s", e, exc_info=True)
            keyboard = None

        text = (
            "✅ <b>商户档案已创建</b>\n\n"
            f"商户名称：<code>{merchant.get('merchant_name') or '-'}</code>\n"
            f"租户标识：<code>{merchant.get('tenant_id') or '-'}</code>\n"
            f"当前套餐：<b>免费套餐</b>\n\n"
            "接下来这样做：\n"
            "1. 点击 @BotFather 创建你的机器人\n"
            "2. 复制 BotFather 返回的 Bot Token\n"
            "3. 点击下方按钮进入商户控制台\n"
            "4. 在控制台里提交 Bot Token 完成接入\n"
        )
        if keyboard:
            await query.edit_message_text(text, reply_markup=keyboard, parse_mode="HTML")
        else:
            await query.edit_message_text(text + "\n⚠️ 控制台链接暂不可用，请稍后再试。", parse_mode="HTML")
        return ConversationHandler.END

    # User chose a paid plan — create payment order
    try:
        order = await get_subscription_lifecycle_service().create_payment_order(merchant_id, plan_code)
    except ValueError as e:
        error_msg = str(e)
        if error_msg == 'plan_not_found':
            await query.edit_message_text("⚠️ 该套餐不存在，请重新选择。")
        elif error_msg == 'payment_config_not_found':
            await query.edit_message_text("⚠️ 支付配置未就绪，请先使用免费套餐，稍后在控制台购买。")
        else:
            await query.edit_message_text(f"⚠️ 创建订单失败：{error_msg}")
        return ConversationHandler.END
    except Exception as e:
        logger.error("create payment order failed: %s", e, exc_info=True)
        await query.edit_message_text("⚠️ 创建支付订单失败，请稍后在商户控制台购买套餐。")
        return ConversationHandler.END

    # Show payment info
    try:
        keyboard = await build_merchant_console_keyboard(user.id, locale)
    except Exception as e:
        logger.error("build merchant otp failed: %s", e, exc_info=True)
        keyboard = None

    payment_text = (
        "💳 <b>支付订单已创建</b>\n\n"
        f"商户名称：<code>{merchant.get('merchant_name') or '-'}</code>\n"
        f"租户标识：<code>{merchant.get('tenant_id') or '-'}</code>\n\n"
        f"套餐：<code>{order.get('plan_code') or '-'}</code>\n"
        f"唯一应付金额：<code>{order.get('amount_usdt') or '-'}</code> USDT\n"
        f"收款地址：<code>{order.get('payment_address') or '-'}</code>\n"
        f"过期时间：<code>{order.get('expire_at') or '-'}</code>\n\n"
        "📌 请直接按唯一金额向该地址付款（不依赖二维码）。\n"
        f"📌 付款后在商户控制台标记已付款，或使用你的独立Bot发送 <code>/paid {order.get('id', '')}</code>。\n\n"
        "接下来这样做：\n"
        "1. 点击 @BotFather 创建你的机器人\n"
        "2. 复制 BotFather 返回的 Bot Token\n"
        "3. 点击下方按钮进入商户控制台\n"
        "4. 在控制台里提交 Bot Token 完成接入\n"
    )

    if keyboard:
        await query.edit_message_text(payment_text, reply_markup=keyboard, parse_mode="HTML")
    else:
        await query.edit_message_text(payment_text + "\n⚠️ 控制台链接暂不可用，请稍后再试。", parse_mode="HTML")

    return ConversationHandler.END


async def cancel_merchant_onboarding(update: Update, context: ContextTypes.DEFAULT_TYPE):
    cmd = (update.message.text or '').strip().lower() if update.message else ''
    if cmd.startswith('/start'):
        from handlers.start import start
        await start(update, context)
    elif cmd.startswith('/admin'):
        from handlers.admin_panel import admin_panel
        await admin_panel(update, context)
    elif update.message:
        await update.message.reply_text("已取消注册流程。")
    elif update.callback_query:
        await update.callback_query.answer("已取消", show_alert=True)
    return ConversationHandler.END


merchant_onboarding_handler = ConversationHandler(
    entry_points=[
        CallbackQueryHandler(start_merchant_onboarding, pattern=r"^register_merchant$"),
        MessageHandler(filters.TEXT & ~filters.COMMAND & filters.Regex(build_register_merchant_text_pattern()), start_merchant_onboarding),
    ],
    states={
        WAITING_MERCHANT_NAME: [MessageHandler(filters.TEXT & ~filters.COMMAND, receive_merchant_name)],
        WAITING_PLAN_SELECTION: [CallbackQueryHandler(receive_plan_selection, pattern=r"^select_plan:")],
    },
    fallbacks=[
        CommandHandler("cancel", cancel_merchant_onboarding),
        CommandHandler("start", cancel_merchant_onboarding),
        CommandHandler("admin", cancel_merchant_onboarding),
    ],
    per_chat=True,
    per_user=True,
    per_message=False,
)
