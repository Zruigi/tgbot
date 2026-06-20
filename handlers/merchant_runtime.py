# -*- coding: utf-8 -*-
"""
商户 Bot 运行时处理器。

仅挂载 tenant-safe 的商户能力：
- /start
- /console
- /features
- /profile
- /subscription
- /plans
- /buy <plan_code>
- /paid <order_id>
- /tx <order_id> <tx_hash>
- owner 专属控制台登录入口
"""
from __future__ import annotations

import logging
import re
from typing import Any, Dict

from telegram import BotCommand, InlineKeyboardButton, InlineKeyboardMarkup, MenuButtonWebApp, ReplyKeyboardMarkup, Update, WebAppInfo
from telegram.ext import Application, CallbackQueryHandler, CommandHandler, ContextTypes, MessageHandler, filters

from backend.app.services import (
    get_feature_gate_service,
    get_quota_service,
    get_subscription_lifecycle_service,
)
from backend.repositories import merchant_repo, subscription_repo, payment_repo
from services.i18n import (
    SUPPORTED_LOCALES,
    language_label,
    resolve_user_locale,
    set_user_locale,
    tr,
)
import web_admin

logger = logging.getLogger(__name__)

MERCHANT_RUNTIME_CALLBACK_PREFIX = "merchant_runtime:"


def _runtime_context(application: Application) -> Dict[str, Any]:
    return dict(application.bot_data.get('merchant_runtime') or {})


def build_merchant_runtime_menu_text_pattern() -> str:
    labels = set()
    for locale in SUPPORTED_LOCALES:
        labels.update({
            tr(locale, "buttons.open_merchant_console"),
            tr(locale, "buttons.current_features"),
            tr(locale, "buttons.profile"),
            tr(locale, "buttons.subscription_orders"),
            tr(locale, "buttons.plans"),
            tr(locale, "buttons.language"),
            tr(locale, "buttons.back_to_main"),
        })
        labels.update(language_label(code) for code in SUPPORTED_LOCALES)
    return r"^(?:" + "|".join(sorted(re.escape(item) for item in labels if item)) + r")$"


async def _refresh_runtime_context(application: Application) -> Dict[str, Any]:
    runtime_ctx = _runtime_context(application)
    merchant_id = int(runtime_ctx.get('merchant_id') or 0)
    if not merchant_id:
        return runtime_ctx

    merchant = await merchant_repo.get_merchant_by_id(merchant_id)
    if not merchant:
        return runtime_ctx

    updated = {
        **runtime_ctx,
        'tenant_id': str(merchant.get('tenant_id') or '').strip(),
        'merchant_id': int(merchant.get('id')),
        'owner_tg_user_id': int(merchant.get('owner_tg_user_id') or 0),
        'merchant_name': str(merchant.get('merchant_name') or '').strip(),
        'plan_type': str(merchant.get('plan_type') or 'free').strip() or 'free',
        'subscription_status': str(merchant.get('subscription_status') or 'unknown').strip() or 'unknown',
        'bot_username': str(merchant.get('bot_username') or '').strip(),
        'bot_id': int(merchant.get('bot_id')) if merchant.get('bot_id') is not None else None,
    }
    application.bot_data['merchant_runtime'] = updated
    return updated


def _is_owner(user_id: int, runtime_ctx: Dict[str, Any]) -> bool:
    try:
        return int(user_id) == int(runtime_ctx.get('owner_tg_user_id') or 0)
    except Exception:
        return False


async def _resolve_locale_from_update(update: Update) -> str:
    user = update.effective_user
    if not user:
        return "zh-CN"
    return await resolve_user_locale(user.id, getattr(user, "language_code", None))


def _merchant_language_keyboard(locale: str) -> InlineKeyboardMarkup:
    rows = []
    for code in SUPPORTED_LOCALES:
        prefix = "✅ " if code == locale else ""
        rows.append([
            InlineKeyboardButton(
                f"{prefix}{language_label(code)}",
                callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}lang_set:{code}",
            )
        ])
    rows.append([
        InlineKeyboardButton(
            tr(locale, "buttons.back_to_main"),
            callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}console",
        )
    ])
    return InlineKeyboardMarkup(rows)


def _merchant_language_reply_keyboard(locale: str) -> ReplyKeyboardMarkup:
    rows = [[language_label(code)] for code in SUPPORTED_LOCALES]
    rows.append([tr(locale, "buttons.back_to_main")])
    return ReplyKeyboardMarkup(
        rows,
        resize_keyboard=True,
        is_persistent=True,
        one_time_keyboard=False,
        input_field_placeholder=tr(locale, "common.language_title"),
    )


def _merchant_reply_keyboard(locale: str) -> ReplyKeyboardMarkup:
    rows = [
        [tr(locale, "buttons.open_merchant_console")],
        [tr(locale, "buttons.current_features"), tr(locale, "buttons.profile")],
        [tr(locale, "buttons.subscription_orders"), tr(locale, "buttons.plans")],
        [tr(locale, "buttons.language")],
    ]
    return ReplyKeyboardMarkup(
        rows,
        resize_keyboard=True,
        is_persistent=True,
        one_time_keyboard=False,
        input_field_placeholder=tr(locale, "merchant_runtime.console_text"),
    )


async def _build_console_keyboard(runtime_ctx: Dict[str, Any], locale: str) -> InlineKeyboardMarkup | None:
    owner_tg_user_id = int(runtime_ctx.get('owner_tg_user_id') or 0)
    if not owner_tg_user_id:
        return None

    try:
        from handlers.start import _build_merchant_entry_urls

        miniapp_url, browser_url = await _build_merchant_entry_urls(owner_tg_user_id)
    except Exception as e:
        logger.warning("merchant runtime failed to build console otp url: %s", e)
        return None

    return InlineKeyboardMarkup([
        [InlineKeyboardButton(tr(locale, "buttons.open_merchant_console"), web_app=WebAppInfo(url=miniapp_url))],
        [InlineKeyboardButton(tr(locale, "buttons.browser_open"), url=browser_url)],
        [InlineKeyboardButton(tr(locale, "buttons.current_features"), callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}features")],
        [InlineKeyboardButton(tr(locale, "buttons.profile"), callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}profile")],
        [InlineKeyboardButton(tr(locale, "buttons.subscription_orders"), callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}subscription")],
        [InlineKeyboardButton(tr(locale, "buttons.plans"), callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}plans")],
        [InlineKeyboardButton(tr(locale, "buttons.language"), callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}lang_menu")],
    ])


def _build_plans_keyboard(plans: list[Dict[str, Any]], locale: str) -> InlineKeyboardMarkup | None:
    rows = []
    for item in plans[:8]:
        plan_code = str(item.get('plan_code') or '').strip()
        if not plan_code:
            continue
        rows.append([
            InlineKeyboardButton(
                f"{tr(locale, 'buttons.plans')} {item.get('plan_name') or plan_code} · {item.get('price_usdt') or 0} USDT",
                callback_data=f"{MERCHANT_RUNTIME_CALLBACK_PREFIX}buy:{plan_code}",
            )
        ])
    return InlineKeyboardMarkup(rows) if rows else None


async def _format_feature_summary(runtime_ctx: Dict[str, Any], locale: str) -> str:
    tenant_id = str(runtime_ctx.get('tenant_id') or '').strip()
    merchant_name = str(runtime_ctx.get('merchant_name') or '商户机器人').strip() or '商户机器人'
    plan_type = str(runtime_ctx.get('plan_type') or 'free').strip() or 'free'
    subscription_status = str(runtime_ctx.get('subscription_status') or 'unknown').strip() or 'unknown'
    bot_username = str(runtime_ctx.get('bot_username') or '').strip()

    feature_map = await get_feature_gate_service().get_feature_map(tenant_id)
    quota = await get_quota_service().get_quota(tenant_id)
    enabled = sorted([key for key, value in (feature_map or {}).items() if bool(value)])

    feature_lines = '\n'.join([f"- <code>{item}</code>" for item in enabled[:20]]) if enabled else "- -"
    if len(enabled) > 20:
        feature_lines += f"\n- ... 共 {len(enabled)} 项"

    daily_quota = quota.get('daily_quota') if isinstance(quota, dict) else None
    monthly_quota = quota.get('monthly_quota') if isinstance(quota, dict) else None
    daily_used = quota.get('daily_used') if isinstance(quota, dict) else None
    monthly_used = quota.get('monthly_used') if isinstance(quota, dict) else None

    username_line = f"@{bot_username}" if bot_username else '-'
    return tr(
        locale,
        "merchant_runtime.features_title",
        merchant_name=merchant_name,
        bot_username=username_line,
        tenant_id=tenant_id or '-',
        plan_type=plan_type,
        subscription_status=subscription_status,
        feature_lines=feature_lines,
        daily_used=daily_used if daily_used is not None else 0,
        daily_quota=daily_quota if daily_quota is not None else 0,
        monthly_used=monthly_used if monthly_used is not None else 0,
        monthly_quota=monthly_quota if monthly_quota is not None else 0,
    )


async def _format_profile_summary(runtime_ctx: Dict[str, Any], locale: str) -> str:
    merchant_id = int(runtime_ctx.get('merchant_id') or 0)
    merchant = await merchant_repo.get_merchant_by_id(merchant_id) if merchant_id else None
    if not merchant:
        return "当前未找到商户资料。"

    return tr(
        locale,
        "merchant_runtime.profile_title",
        merchant_name=merchant.get('merchant_name') or '-',
        tenant_id=merchant.get('tenant_id') or '-',
        merchant_id=merchant.get('id') or '-',
        status=merchant.get('status') or '-',
        plan_type=merchant.get('plan_type') or '-',
        subscription_status=merchant.get('subscription_status') or '-',
        bot_username=f"@{merchant.get('bot_username') or '-'}",
    )


async def _format_subscription_summary(runtime_ctx: Dict[str, Any], locale: str) -> str:
    merchant_id = int(runtime_ctx.get('merchant_id') or 0)
    if not merchant_id:
        return "当前无订阅信息。"

    subscription = await subscription_repo.get_active_subscription(merchant_id)
    latest_order = await payment_repo.get_latest_open_payment_order(merchant_id)

    lines = [tr(locale, "merchant_runtime.subscription_title")]
    if subscription:
        lines.extend([
            f"- <code>{subscription.get('plan_code') or '-'}</code>",
            f"- <code>{subscription.get('status') or '-'}</code>",
            f"- <code>{subscription.get('end_at') or '-'}</code>",
        ])
    else:
        lines.append(tr(locale, "merchant_runtime.no_subscription"))

    if latest_order:
        lines.extend([
            "",
            tr(locale, "merchant_runtime.latest_order"),
            f"- 订单号：<code>{latest_order.get('id') or '-'}</code>",
            f"- 套餐：<code>{latest_order.get('plan_code') or '-'}</code>",
            f"- 状态：<code>{latest_order.get('status') or '-'}</code>",
            f"- 唯一应付金额：<code>{latest_order.get('amount_usdt') or '-'}</code> USDT",
            f"- 收款地址：<code>{latest_order.get('payment_address') or '-'}</code>",
            f"- 过期：<code>{latest_order.get('expire_at') or '-'}</code>",
            "",
            tr(locale, "merchant_runtime.next_steps"),
            "1. 直接按唯一金额向该地址付款（不依赖二维码）",
            f"2. 付款后发送 <code>/paid {latest_order.get('id') or ''}</code>",
            "3. 只有平台明确要求时，再用 <code>/tx 订单ID tx_hash</code> 补链上证明",
        ])
    else:
        lines.extend([
            "",
            tr(locale, "merchant_runtime.next_steps"),
            "- 先用 <code>/plans</code> 查看套餐",
            "- 再用 <code>/buy plan_code</code> 或直接点套餐按钮下单",
        ])
    return '\n'.join(lines)


async def _format_plans_summary(locale: str) -> tuple[str, list[Dict[str, Any]]]:
    plans = await subscription_repo.list_plans(active_only=True)
    if not plans:
        return "当前没有可用套餐。", []
    lines = [tr(locale, "merchant_runtime.plans_title")]
    for item in plans[:10]:
        lines.append(
            f"- <code>{item.get('plan_code') or '-'}</code> · {item.get('plan_name') or '-'} · {item.get('price_usdt') or 0} USDT / {item.get('duration_days') or 0} 天"
        )
    lines.append(f"\n{tr(locale, 'merchant_runtime.buy_usage')}")
    return '\n'.join(lines), plans


async def _reply_owner_only(update: Update, text: str, *, reply_markup: InlineKeyboardMarkup | None = None) -> None:
    if update.message:
        await update.message.reply_html(text, reply_markup=reply_markup)
    elif update.callback_query:
        await update.callback_query.answer()
        await update.callback_query.message.reply_html(text, reply_markup=reply_markup)


async def _create_order_and_format(runtime_ctx: Dict[str, Any], plan_code: str, locale: str) -> str:
    merchant_id = int(runtime_ctx.get('merchant_id') or 0)
    order = await get_subscription_lifecycle_service().create_payment_order(merchant_id, plan_code)
    return tr(
        locale,
        "merchant_runtime.order_created",
        order_id=order.get('id') or '-',
        plan_code=order.get('plan_code') or '-',
        amount_usdt=order.get('amount_usdt') or '-',
        payment_address=order.get('payment_address') or '-',
        expire_at=order.get('expire_at') or '-',
    )


async def merchant_start(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user:
        return
    locale = await _resolve_locale_from_update(update)

    merchant_name = str(runtime_ctx.get('merchant_name') or '商户机器人').strip() or '商户机器人'
    owner_view = _is_owner(user.id, runtime_ctx)

    if owner_view:
        text = tr(locale, "merchant_runtime.owner_welcome", user=user.mention_html(), merchant_name=merchant_name)
        reply_markup = await _build_console_keyboard(runtime_ctx, locale)
        reply_keyboard = _merchant_reply_keyboard(locale)
    else:
        text = tr(locale, "merchant_runtime.guest_welcome", merchant_name=merchant_name)
        reply_markup = None
        reply_keyboard = None

    if update.message:
        await update.message.reply_html(text, reply_markup=reply_keyboard or reply_markup)
    elif update.callback_query:
        await update.callback_query.answer()
        if reply_markup:
            await update.callback_query.message.reply_html(text, reply_markup=reply_markup)
        if reply_keyboard:
            await update.callback_query.message.reply_html(text, reply_markup=reply_keyboard)


async def merchant_console_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user or not update.message:
        return
    locale = await _resolve_locale_from_update(update)

    if not _is_owner(user.id, runtime_ctx):
        await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_console"))
        return

    reply_markup = await _build_console_keyboard(runtime_ctx, locale)
    if not reply_markup:
        await update.message.reply_text(tr(locale, "start.link_unavailable"))
        return

    await update.message.reply_html(tr(locale, "merchant_runtime.console_text"), reply_markup=reply_markup)


async def merchant_features_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user:
        return
    locale = await _resolve_locale_from_update(update)

    if not _is_owner(user.id, runtime_ctx):
        if update.message:
            await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_features"))
        elif update.callback_query:
            await update.callback_query.answer(tr(locale, "merchant_runtime.owner_only_features"), show_alert=True)
        return

    text = await _format_feature_summary(runtime_ctx, locale)
    reply_markup = await _build_console_keyboard(runtime_ctx, locale)
    await _reply_owner_only(update, text, reply_markup=reply_markup)


async def merchant_profile_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user:
        return
    locale = await _resolve_locale_from_update(update)
    if not _is_owner(user.id, runtime_ctx):
        if update.message:
            await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_profile"))
        elif update.callback_query:
            await update.callback_query.answer(tr(locale, "merchant_runtime.owner_only_profile"), show_alert=True)
        return
    text = await _format_profile_summary(runtime_ctx, locale)
    await _reply_owner_only(update, text)


async def merchant_subscription_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user:
        return
    locale = await _resolve_locale_from_update(update)
    if not _is_owner(user.id, runtime_ctx):
        if update.message:
            await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_subscription"))
        elif update.callback_query:
            await update.callback_query.answer(tr(locale, "merchant_runtime.owner_only_subscription"), show_alert=True)
        return
    text = await _format_subscription_summary(runtime_ctx, locale)
    await _reply_owner_only(update, text)


async def merchant_plans_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user:
        return
    locale = await _resolve_locale_from_update(update)
    if not _is_owner(user.id, runtime_ctx):
        if update.message:
            await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_plans"))
        elif update.callback_query:
            await update.callback_query.answer(tr(locale, "merchant_runtime.owner_only_plans"), show_alert=True)
        return
    text, plans = await _format_plans_summary(locale)
    await _reply_owner_only(update, text, reply_markup=_build_plans_keyboard(plans, locale))


async def merchant_buy_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user or not update.message:
        return
    locale = await _resolve_locale_from_update(update)
    if not _is_owner(user.id, runtime_ctx):
        await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_buy"))
        return
    if not context.args:
        await update.message.reply_html(tr(locale, "merchant_runtime.buy_usage"))
        return

    plan_code = str(context.args[0] or '').strip()
    try:
        text = await _create_order_and_format(runtime_ctx, plan_code, locale)
    except ValueError as e:
        await update.message.reply_text(str(e))
        return
    except Exception as e:
        logger.error("merchant buy command failed: %s", e, exc_info=True)
        await update.message.reply_text("创建支付订单失败，请稍后重试。")
        return

    await update.message.reply_html(text)


async def merchant_paid_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user or not update.message:
        return
    locale = await _resolve_locale_from_update(update)
    if not _is_owner(user.id, runtime_ctx):
        await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_buy"))
        return
    if not context.args:
        await update.message.reply_html(tr(locale, "merchant_runtime.paid_usage"))
        return
    try:
        order_id = int(context.args[0])
    except Exception:
        await update.message.reply_text("order_id 无效")
        return

    merchant_id = int(runtime_ctx.get('merchant_id') or 0)
    order = await payment_repo.get_payment_order(order_id)
    if not order or int(order.get('merchant_id') or 0) != merchant_id:
        await update.message.reply_text("订单不存在或不属于当前商户")
        return

    ok = await get_subscription_lifecycle_service().mark_order_paid(order_id)
    if ok:
        refreshed = await payment_repo.get_payment_order(order_id)
        await update.message.reply_html(
            "\n".join([
                "<b>已提交付款确认</b>",
                f"- 订单号：<code>{order_id}</code>",
                f"- 当前状态：<code>{(refreshed or {}).get('status') or 'paid_pending_review'}</code>",
                "- 平台会按订单金额和付款情况审核",
                "- 如平台没有要求，不需要再提交 tx_hash",
            ])
        )
    else:
        await update.message.reply_text("提交失败，请检查订单状态。")


async def merchant_tx_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    runtime_ctx = await _refresh_runtime_context(context.application)
    user = update.effective_user
    if not user or not update.message:
        return
    locale = await _resolve_locale_from_update(update)
    if not _is_owner(user.id, runtime_ctx):
        await update.message.reply_text(tr(locale, "merchant_runtime.owner_only_tx"))
        return
    if len(context.args) < 2:
        await update.message.reply_html(tr(locale, "merchant_runtime.tx_usage"))
        return

    try:
        order_id = int(context.args[0])
    except Exception:
        await update.message.reply_text("order_id 无效")
        return
    tx_hash = str(context.args[1] or '').strip()
    if not tx_hash:
        await update.message.reply_text("tx_hash 不能为空")
        return

    merchant_id = int(runtime_ctx.get('merchant_id') or 0)
    order = await payment_repo.get_payment_order(order_id)
    if not order or int(order.get('merchant_id') or 0) != merchant_id:
        await update.message.reply_text("订单不存在或不属于当前商户")
        return

    ok = await get_subscription_lifecycle_service().submit_tx_hash(order_id, tx_hash)
    await update.message.reply_text("tx_hash 已提交，等待平台审核。仅在平台要求补充链上证明时再使用该步骤。" if ok else "提交失败，请检查订单状态。")


async def merchant_runtime_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    query = update.callback_query
    if not query or not query.data:
        return

    runtime_ctx = await _refresh_runtime_context(context.application)
    locale = await _resolve_locale_from_update(update)
    if not _is_owner(query.from_user.id, runtime_ctx):
        await query.answer(tr(locale, "merchant_runtime.owner_only_console"), show_alert=True)
        return

    action = query.data.removeprefix(MERCHANT_RUNTIME_CALLBACK_PREFIX)
    if action == 'lang_menu':
        await query.answer()
        await query.message.reply_text(
            f"{tr(locale, 'common.language_title')}\n\n{tr(locale, 'common.language_current', language=language_label(locale))}",
            reply_markup=_merchant_language_keyboard(locale),
        )
        return
    if action.startswith('lang_set:'):
        next_locale = await set_user_locale(query.from_user.id, action.split(':', 1)[1])
        await query.answer(tr(next_locale, "common.language_saved", language=language_label(next_locale)))
        await merchant_start(update, context)
        return
    if action == 'features':
        await merchant_features_command(update, context)
        return
    if action == 'profile':
        await merchant_profile_command(update, context)
        return
    if action == 'subscription':
        await merchant_subscription_command(update, context)
        return
    if action == 'plans':
        await merchant_plans_command(update, context)
        return
    if action == 'console':
        reply_markup = await _build_console_keyboard(runtime_ctx, locale)
        await query.answer()
        if reply_markup:
            await query.message.reply_html(tr(locale, "merchant_runtime.console_text"), reply_markup=reply_markup)
        else:
            await query.message.reply_text(tr(locale, "start.link_unavailable"))
        return
    if action.startswith('buy:'):
        plan_code = action.split(':', 1)[1].strip()
        try:
            text = await _create_order_and_format(runtime_ctx, plan_code, locale)
            await query.answer("订单已创建")
            await query.message.reply_html(text)
        except ValueError as e:
            await query.answer(str(e), show_alert=True)
        except Exception as e:
            logger.error("merchant buy callback failed: %s", e, exc_info=True)
            await query.answer("创建订单失败", show_alert=True)
        return

    await query.answer()


async def merchant_runtime_menu_message_handler(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    if not update.message or not update.effective_user:
        return

    runtime_ctx = await _refresh_runtime_context(context.application)
    locale = await _resolve_locale_from_update(update)
    text = (update.message.text or "").strip()

    if not _is_owner(update.effective_user.id, runtime_ctx):
        return

    for code in SUPPORTED_LOCALES:
        if text == language_label(code):
            await set_user_locale(update.effective_user.id, code)
            await update.message.reply_text(
                tr(code, "common.language_saved", language=language_label(code)),
                reply_markup=_merchant_reply_keyboard(code),
            )
            return

    if text == tr(locale, "buttons.language"):
        await update.message.reply_text(
            f"{tr(locale, 'common.language_title')}\n\n{tr(locale, 'common.language_current', language=language_label(locale))}",
            reply_markup=_merchant_language_reply_keyboard(locale),
        )
        return
    if text == tr(locale, "buttons.back_to_main"):
        await merchant_start(update, context)
        return
    if text == tr(locale, "buttons.open_merchant_console"):
        await merchant_console_command(update, context)
        return
    if text == tr(locale, "buttons.current_features"):
        await merchant_features_command(update, context)
        return
    if text == tr(locale, "buttons.profile"):
        await merchant_profile_command(update, context)
        return
    if text == tr(locale, "buttons.subscription_orders"):
        await merchant_subscription_command(update, context)
        return
    if text == tr(locale, "buttons.plans"):
        await merchant_plans_command(update, context)
        return


def register_merchant_runtime_handlers(application: Application) -> None:
    application.add_handler(CommandHandler("menu", merchant_start))
    application.add_handler(CommandHandler("start", merchant_start))
    application.add_handler(CommandHandler("console", merchant_console_command))
    application.add_handler(CommandHandler("features", merchant_features_command))
    application.add_handler(CommandHandler("profile", merchant_profile_command))
    application.add_handler(CommandHandler("subscription", merchant_subscription_command))
    application.add_handler(CommandHandler("plans", merchant_plans_command))
    application.add_handler(CommandHandler("buy", merchant_buy_command))
    application.add_handler(CommandHandler("paid", merchant_paid_command))
    application.add_handler(CommandHandler("tx", merchant_tx_command))
    application.add_handler(CallbackQueryHandler(merchant_runtime_callback, pattern=rf"^{MERCHANT_RUNTIME_CALLBACK_PREFIX}"))
    application.add_handler(
        MessageHandler(filters.TEXT & ~filters.COMMAND & filters.Regex(build_merchant_runtime_menu_text_pattern()), merchant_runtime_menu_message_handler)
    )


async def setup_merchant_runtime_entrypoints(application: Application) -> None:
    runtime_ctx = _runtime_context(application)
    tenant_id = str(runtime_ctx.get('tenant_id') or '').strip()
    merchant_id = int(runtime_ctx.get('merchant_id') or 0)

    try:
        await application.bot.set_my_commands([
            BotCommand("menu", "Open menu / 打开菜单"),
            BotCommand("console", "Merchant console / 控制台"),
            BotCommand("profile", "Profile / 商户资料"),
            BotCommand("subscription", "Subscription / 订阅"),
            BotCommand("plans", "Plans / 套餐"),
        ])
    except Exception as e:
        logger.warning("failed to set merchant runtime commands tenant=%s: %s", tenant_id, e)

    try:
        miniapp_url = web_admin.build_miniapp_url(
            entry_intent='merchant_console',
            tenant_hint=tenant_id,
            merchant_id_hint=merchant_id or None,
        )
        await application.bot.set_chat_menu_button(
            menu_button=MenuButtonWebApp(
                text="🏪 Console",
                web_app=WebAppInfo(url=miniapp_url),
            )
        )
    except Exception as e:
        logger.warning("failed to set merchant runtime menu button tenant=%s: %s", tenant_id, e)
