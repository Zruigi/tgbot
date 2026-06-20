# -*- coding: utf-8 -*-
"""
/start 命令处理 — v3.3 规范
- 双轨菜单（管理员 / 普通用户）
- 双列排版
- OTP 动态链接（不暴露明文 Token）
- Deep Linking 自助解禁：unmute_{group_id} 参数处理
"""
import logging
import os
import re
import time
from typing import Dict
from urllib.parse import urlencode

from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup, ChatPermissions, WebAppInfo, ReplyKeyboardMarkup
from telegram.ext import ContextTypes
from telegram.error import TelegramError
import config
import web_admin
import database
from backend.repositories import merchant_repo
from cachetools import TTLCache
from services.i18n import (
    SUPPORTED_LOCALES,
    language_label,
    resolve_user_locale,
    set_user_locale,
    tr,
)

logger = logging.getLogger(__name__)

# 频率限制缓存：{user_id: last_verify_time}，TTL 60秒，上限 1000 条
_unmute_rate_limit = TTLCache(maxsize=1000, ttl=60)


def _main_menu_action_map(locale: str, has_merchant_console: bool, is_admin: bool) -> Dict[str, str]:
    mapping = {
        tr(locale, "buttons.language"): "lang_menu",
        tr(locale, "buttons.contact_admin"): "contact_admin",
        tr(locale, "buttons.spot_price"): "spot_price",
        tr(locale, "buttons.futures_price"): "futures_price",
        tr(locale, "buttons.wallet_monitor"): "wallet_monitor",
        tr(locale, "buttons.wallet_list"): "wallet_my_list",
        tr(locale, "buttons.email_settings"): "email_settings",
        tr(locale, "buttons.back_to_main"): "back_to_start",
    }
    if has_merchant_console:
        mapping[tr(locale, "buttons.merchant_console")] = "merchant_web_otp"
    else:
        mapping[tr(locale, "buttons.register_bot")] = "register_merchant"
    if is_admin:
        mapping[tr(locale, "buttons.web_admin")] = "admin_web_otp"
        mapping[tr(locale, "buttons.system_settings")] = "admin_settings"
    return mapping


def build_main_menu_text_pattern() -> str:
    labels = set()
    for locale_code in SUPPORTED_LOCALES:
        direct_keys = {
            tr(locale_code, "buttons.language"),
            tr(locale_code, "buttons.contact_admin"),
            tr(locale_code, "buttons.wallet_list"),
            tr(locale_code, "buttons.back_to_main"),
            tr(locale_code, "buttons.web_admin"),
            tr(locale_code, "buttons.merchant_console"),
            tr(locale_code, "buttons.system_settings"),
        }
        labels.update(direct_keys)
        labels.update(language_label(code) for code in SUPPORTED_LOCALES)
    escaped = sorted(re.escape(label) for label in labels if label)
    return r"^(?:" + "|".join(escaped) + r")$"


def _is_admin(user_id: int) -> bool:
    """判断是否为管理员"""
    return user_id in config.ADMIN_TG_IDS


def _resolve_frontend_url_for_otp() -> str:
    """
    运行时解析 OTP 登录基础 URL。
    优先级：
    1) .env 中的 FRONTEND_URL
    2) .env 中的 WEB_BASE_URL
    3) 进程内缓存的 config.FRONTEND_URL
    """
    env_file = os.path.join(config.BASE_DIR, ".env")
    latest_frontend = ""
    latest_web_base = ""

    try:
        if os.path.exists(env_file):
            with open(env_file, "r", encoding="utf-8", errors="ignore") as f:
                for raw_line in f:
                    line = raw_line.strip()
                    if not line or line.startswith("#") or "=" not in line:
                        continue
                    key, value = line.split("=", 1)
                    key = key.strip()
                    value = value.strip().strip("'\"").rstrip("/")
                    if not value:
                        continue
                    if key == "FRONTEND_URL":
                        latest_frontend = value
                    elif key == "WEB_BASE_URL":
                        latest_web_base = value
    except Exception as e:
        logger.warning(f"读取 .env 域名失败，回退到 config.FRONTEND_URL: {e}")

    return (
        latest_frontend
        or latest_web_base
        or (config.FRONTEND_URL or "").strip().rstrip("/")
    )


def _build_admin_browser_url(frontend_url: str, *, otp: str) -> str:
    params = {'otp': otp}
    return f"{frontend_url.rstrip('/')}/admin?{urlencode(params)}"


async def _handle_unmute_deeplink(update: Update, context: ContextTypes.DEFAULT_TYPE, user_id: int, param: str) -> None:
    """
    处理 unmute_{group_id} Deep Linking 参数
    流程：
    1. 解析 group_id
    2. 检查频率限制（1分钟内只能验证一次）
    3. 检查用户是否已关注 verify_channel_id
    4. 如果未关注：发送频道链接 + [我已关注，点击验证] 按钮
    5. 如果已关注：解禁 + 更新数据库 + 删除禁言提示消息
    """
    try:
        group_id = int(param.replace("unmute_", ""))
    except (ValueError, IndexError):
        await context.bot.send_message(
            chat_id=user_id,
            text="❌ 无效的解禁链接参数。"
        )
        return

    # 1. 频率限制检查（1分钟内只能点击一次）
    now = time.time()
    if user_id in _unmute_rate_limit:
        last_time = _unmute_rate_limit[user_id]
        if now - last_time < 60:
            remaining = int(60 - (now - last_time))
            await context.bot.send_message(
                chat_id=user_id,
                text=f"⏱️ 请稍候 {remaining} 秒后再试。（防止爆破 API）"
            )
            return
    _unmute_rate_limit[user_id] = now

    # 2. 获取群组验证配置
    try:
        verify_config = await database.get_group_verify_config(group_id)
    except Exception as e:
        logger.error(f"[解禁] 获取群组配置失败 group={group_id}: {e}")
        await context.bot.send_message(
            chat_id=user_id,
            text="❌ 无法获取群组配置，请稍后重试。"
        )
        return

    verify_channel_id = verify_config.get("verify_channel_id")
    if not verify_channel_id:
        await context.bot.send_message(
            chat_id=user_id,
            text="❌ 该群组未配置验证频道，无法解禁。"
        )
        return

    # 3. 检查用户是否已关注频道
    try:
        member = await context.bot.get_chat_member(chat_id=verify_channel_id, user_id=user_id)
        is_member = member.status in ('member', 'administrator', 'creator')
    except TelegramError as e:
        logger.warning(f"[解禁] 检测频道成员失败 user={user_id} channel={verify_channel_id}: {e}")
        is_member = False

    if not is_member:
        # 未关注：发送频道链接 + 验证按钮
        try:
            channel_info = await context.bot.get_chat(verify_channel_id)
            channel_link = f"https://t.me/{channel_info.username}" if channel_info.username else str(verify_channel_id)
        except Exception:
            channel_link = str(verify_channel_id)

        keyboard = InlineKeyboardMarkup([
            [InlineKeyboardButton("📢 前往关注频道", url=channel_link)],
            [InlineKeyboardButton("✅ 我已关注，点击验证", callback_data=f"unmute_verify:{group_id}:{verify_channel_id}")]
        ])
        await context.bot.send_message(
            chat_id=user_id,
            text=(
                "📢 <b>需要关注频道才能解禁</b>\n\n"
                "请先关注下方频道，然后点击 [我已关注，点击验证] 按钮完成验证。\n\n"
                "验证成功后，你将恢复在群组中的发言权限。"
            ),
            reply_markup=keyboard,
            parse_mode="HTML"
        )
        return

    # 已关注：直接执行解禁
    await _execute_unmute(context.bot, user_id, group_id)


async def _execute_unmute(bot, user_id: int, group_id: int) -> None:
    """
    执行解禁操作：
    1. 检查 Bot 是否有管理员权限
    2. 恢复用户发言权限
    3. 更新数据库 is_verified = True
    4. 删除禁言提示消息
    5. 发送成功提示
    """
    # 1. 检查 Bot 权限
    try:
        bot_member = await bot.get_chat_member(chat_id=group_id, user_id=bot.id)
        if not bot_member.can_restrict_members:
            await bot.send_message(
                chat_id=user_id,
                text="❌ Bot 在该群组中权限不足，无法解禁。请联系群管理员。"
            )
            logger.warning(f"[解禁] Bot 权限不足 group={group_id}")
            return
    except TelegramError as e:
        logger.error(f"[解禁] 检查 Bot 权限失败 group={group_id}: {e}")
        await bot.send_message(
            chat_id=user_id,
            text="❌ 无法验证 Bot 权限，请稍后重试。"
        )
        return

    # 2. 恢复用户发言权限
    try:
        await bot.restrict_chat_member(
            chat_id=group_id,
            user_id=user_id,
            permissions=ChatPermissions(
                can_send_messages=True,
                can_send_polls=True,
                can_send_other_messages=True,
                can_add_web_page_previews=True,
                can_change_info=False,
                can_invite_users=True,
                can_pin_messages=False
            )
        )
        logger.info(f"[解禁] 已恢复用户发言权限 user={user_id} group={group_id}")
    except TelegramError as e:
        logger.error(f"[解禁] 恢复权限失败 user={user_id} group={group_id}: {e}")
        await bot.send_message(
            chat_id=user_id,
            text="❌ 解禁失败，请稍后重试或联系管理员。"
        )
        return

    # 3. 更新数据库
    try:
        await database.set_member_verified(group_id, user_id)
        logger.info(f"[解禁] 已标记用户为已验证 user={user_id} group={group_id}")
    except Exception as e:
        logger.warning(f"[解禁] 更新数据库失败: {e}")

    # 4. 尝试删除禁言提示消息（从 group_members 表读取 pending_verify_msg_id）
    try:
        async with database.aiosqlite.connect(database.DB_NAME) as db:
            async with db.execute(
                "SELECT pending_verify_msg_id FROM group_members WHERE group_id = ? AND user_id = ?",
                (group_id, user_id)
            ) as cursor:
                row = await cursor.fetchone()
                msg_id = row[0] if row and row[0] else None
        
        if msg_id:
            try:
                await bot.delete_message(chat_id=group_id, message_id=msg_id)
                logger.info(f"[解禁] 已删除禁言提示消息 msg_id={msg_id}")
            except TelegramError:
                logger.debug(f"[解禁] 删除消息失败（可能已被删除）msg_id={msg_id}")
    except Exception as e:
        logger.warning(f"[解禁] 查询消息 ID 失败: {e}")

    # 5. 发送成功提示
    await bot.send_message(
        chat_id=user_id,
        text=(
            "✅ <b>解禁成功！</b>\n\n"
            "你已恢复在群组中的发言权限，现在可以回到群里发言了。\n\n"
            "感谢你的关注与支持！"
        ),
        parse_mode="HTML"
    )





async def _build_merchant_entry_urls(owner_tg_user_id: int) -> tuple[str, str]:
    merchant = await merchant_repo.get_merchant_by_owner(int(owner_tg_user_id))
    if not merchant:
        raise ValueError('merchant_not_found')

    frontend_url = _resolve_frontend_url_for_otp()
    if not frontend_url or '127.0.0.1' in frontend_url or 'localhost' in frontend_url:
        raise ValueError(f'前端域名不可用: {frontend_url}')

    grant_meta = await web_admin.generate_entry_grant(
        issued_for=str(owner_tg_user_id),
        role_hint='merchant_owner',
        tenant_id=str(merchant.get('tenant_id') or ''),
        merchant_id=int(merchant.get('id')),
        entry_intent='merchant_console',
    )
    otp = await web_admin.generate_otp(
        role='merchant_owner',
        tenant_id=str(merchant.get('tenant_id') or ''),
        merchant_id=int(merchant.get('id')),
        issued_for=str(owner_tg_user_id),
    )
    miniapp_url = web_admin.build_miniapp_url(
        entry_intent='merchant_console',
        grant=grant_meta['grant'],
        tenant_hint=str(merchant.get('tenant_id') or ''),
        merchant_id_hint=int(merchant.get('id')),
    )
    browser_url = web_admin.build_miniapp_url(
        entry_intent='merchant_console',
        grant=grant_meta['grant'],
        otp=otp,
        tenant_hint=str(merchant.get('tenant_id') or ''),
        merchant_id_hint=int(merchant.get('id')),
    )
    return miniapp_url, browser_url


async def _build_admin_entry_urls(user_id: int) -> tuple[str, str]:
    frontend_url = _resolve_frontend_url_for_otp()
    if not frontend_url or '127.0.0.1' in frontend_url or 'localhost' in frontend_url:
        raise ValueError(f'前端域名不可用: {frontend_url}')

    grant_meta = await web_admin.generate_entry_grant(
        issued_for=str(user_id),
        role_hint='platform_admin',
        entry_intent='admin_console',
    )
    otp = await web_admin.generate_otp(
        role='platform_admin',
        issued_for=str(user_id),
    )
    miniapp_url = web_admin.build_miniapp_url(
        entry_intent='admin_console',
        grant=grant_meta['grant'],
    )
    browser_url = web_admin.build_miniapp_url(
        entry_intent='admin_console',
        grant=grant_meta['grant'],
        otp=otp,
    )
    return miniapp_url, browser_url


def _language_keyboard(locale: str) -> InlineKeyboardMarkup:
    rows = []
    current = locale
    for code in SUPPORTED_LOCALES:
        prefix = "✅ " if code == current else ""
        rows.append([InlineKeyboardButton(f"{prefix}{language_label(code)}", callback_data=f"lang_set:{code}")])
    rows.append([InlineKeyboardButton(tr(locale, "buttons.back_to_main"), callback_data="back_to_start")])
    return InlineKeyboardMarkup(rows)


def _language_reply_keyboard(locale: str) -> ReplyKeyboardMarkup:
    rows = [[language_label(code)] for code in SUPPORTED_LOCALES]
    rows.append([tr(locale, "buttons.back_to_main")])
    return ReplyKeyboardMarkup(
        rows,
        resize_keyboard=True,
        is_persistent=True,
        one_time_keyboard=False,
        input_field_placeholder=tr(locale, "common.language_title"),
    )


def _main_reply_keyboard(locale: str, is_admin: bool, has_merchant_console: bool) -> ReplyKeyboardMarkup:
    rows = [
        [tr(locale, "buttons.contact_admin")],
    ]
    if is_admin:
        rows.append([tr(locale, "buttons.web_admin")])
        rows.append([tr(locale, "buttons.merchant_console")])
    elif has_merchant_console:
        rows.append([tr(locale, "buttons.merchant_console")])
    else:
        rows.append([tr(locale, "buttons.register_bot")])

    rows.extend([
        [tr(locale, "buttons.spot_price"), tr(locale, "buttons.futures_price")],
        [tr(locale, "buttons.wallet_monitor"), tr(locale, "buttons.wallet_list")],
        [tr(locale, "buttons.email_settings"), tr(locale, "buttons.language")],
    ])
    if is_admin:
        rows.append([tr(locale, "buttons.system_settings")])

    return ReplyKeyboardMarkup(
        rows,
        resize_keyboard=True,
        is_persistent=True,
        one_time_keyboard=False,
        input_field_placeholder=tr(locale, "start.main_menu", user="", role_tag="", user_id="").split("\n")[-1],
    )


def _admin_keyboard(locale: str, user_id: int = 0) -> InlineKeyboardMarkup:
    """管理员专属菜单（双列排版）— Web后台入口始终点击后动态生成 OTP 链接"""
    keyboard = [
        [InlineKeyboardButton(tr(locale, "buttons.web_admin"), callback_data="admin_web_otp")],
        [InlineKeyboardButton(tr(locale, "buttons.merchant_console"), callback_data="merchant_web_otp")],
        [InlineKeyboardButton(tr(locale, "buttons.system_settings"), callback_data="admin_settings"),
         InlineKeyboardButton(tr(locale, "buttons.spot_price"), callback_data="spot_price")],
        [InlineKeyboardButton(tr(locale, "buttons.futures_price"), callback_data="futures_price"),
         InlineKeyboardButton(tr(locale, "buttons.wallet_monitor"), callback_data="wallet_monitor")],
        [InlineKeyboardButton(tr(locale, "buttons.wallet_list"), callback_data="wallet_my_list"),
         InlineKeyboardButton(tr(locale, "buttons.email_settings"), callback_data="email_settings")],
        [InlineKeyboardButton(tr(locale, "buttons.language"), callback_data="lang_menu")],
    ]
    return InlineKeyboardMarkup(keyboard)



def _user_keyboard(locale: str, has_merchant_console: bool = False) -> InlineKeyboardMarkup:
    """普通用户菜单（双列排版，客服置顶整行）"""
    keyboard = [
        [InlineKeyboardButton(tr(locale, "buttons.contact_admin"), callback_data="contact_admin")],
    ]
    if has_merchant_console:
        keyboard.append([InlineKeyboardButton(tr(locale, "buttons.merchant_console"), callback_data="merchant_web_otp")])
    else:
        keyboard.append([InlineKeyboardButton(tr(locale, "buttons.register_bot"), callback_data="register_merchant")])
    keyboard.extend([
        [InlineKeyboardButton(tr(locale, "buttons.spot_price"), callback_data="spot_price"),
         InlineKeyboardButton(tr(locale, "buttons.futures_price"), callback_data="futures_price")],
        [InlineKeyboardButton(tr(locale, "buttons.wallet_monitor"), callback_data="wallet_monitor"),
         InlineKeyboardButton(tr(locale, "buttons.wallet_list"), callback_data="wallet_my_list")],
        [InlineKeyboardButton(tr(locale, "buttons.email_settings"), callback_data="email_settings")],
        [InlineKeyboardButton(tr(locale, "buttons.language"), callback_data="lang_menu")],
    ])
    return InlineKeyboardMarkup(keyboard)


async def start(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理 /start 命令，显示主菜单或处理 Deep Linking 参数"""
    user = update.effective_user
    if not user:
        return
    locale = await resolve_user_locale(user.id, getattr(user, "language_code", None))

    # 解析 Deep Linking 参数
    args = context.args or []
    if args and args[0].startswith("unmute_"):
        await _handle_unmute_deeplink(update, context, user.id, args[0])
        return

    is_admin = _is_admin(user.id)
    merchant = await merchant_repo.get_merchant_by_owner(user.id)
    has_merchant_console = merchant is not None
    keyboard = _admin_keyboard(locale, user.id) if is_admin else _user_keyboard(locale, has_merchant_console)
    reply_keyboard = _main_reply_keyboard(locale, is_admin, has_merchant_console)

    role_tag = tr(locale, "start.role.admin") if is_admin else (tr(locale, "start.role.merchant") if has_merchant_console else tr(locale, "start.role.user"))
    text = tr(locale, "start.main_menu", user=user.mention_html(), role_tag=role_tag, user_id=user.id)

    if update.message:
        await update.message.reply_html(text, reply_markup=reply_keyboard)
    elif update.callback_query:
        chat_id = update.callback_query.message.chat_id if update.callback_query.message else user.id
        try:
            await update.callback_query.edit_message_text(
                text, reply_markup=keyboard, parse_mode="HTML"
            )
        except Exception:
            pass
        await context.bot.send_message(
            chat_id=chat_id,
            text=text,
            reply_markup=reply_keyboard,
            parse_mode="HTML"
        )


async def show_language_menu(update: Update, context: ContextTypes.DEFAULT_TYPE, locale: str) -> None:
    text = (
        f"{tr(locale, 'common.language_title')}\n\n"
        f"{tr(locale, 'common.language_current', language=language_label(locale))}"
    )
    if update.message:
        await update.message.reply_text(text, reply_markup=_language_reply_keyboard(locale))
    elif update.callback_query:
        try:
            await update.callback_query.edit_message_text(text, reply_markup=_language_keyboard(locale))
        except Exception:
            pass
        chat_id = update.callback_query.message.chat_id if update.callback_query.message else update.effective_user.id
        await context.bot.send_message(chat_id=chat_id, text=text, reply_markup=_language_reply_keyboard(locale))


async def main_menu_message_handler(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    if not update.message or not update.effective_user:
        return

    user = update.effective_user
    locale = await resolve_user_locale(user.id, getattr(user, "language_code", None))
    is_admin = _is_admin(user.id)
    merchant = await merchant_repo.get_merchant_by_owner(user.id)
    has_merchant_console = merchant is not None
    text = (update.message.text or "").strip()

    for code in SUPPORTED_LOCALES:
        if text == language_label(code):
            await set_user_locale(user.id, code)
            await update.message.reply_text(
                tr(code, "common.language_saved", language=language_label(code)),
                reply_markup=_main_reply_keyboard(code, is_admin, has_merchant_console),
            )
            return

    action = _main_menu_action_map(locale, has_merchant_console, is_admin).get(text)
    if not action:
        return

    if action == "lang_menu":
        await show_language_menu(update, context, locale)
        return
    if action == "back_to_start":
        await start(update, context)
        return
    if action == "wallet_my_list":
        from handlers.wallet_panel import my_wallets_command
        await my_wallets_command(update, context)
        return
    if action == "admin_settings":
        await update.message.reply_text(tr(locale, "common.admin_settings_web_only"))
        return
    if action == "admin_web_otp":
        if not is_admin:
            await update.message.reply_text(tr(locale, "common.no_permission"))
            return
        try:
            miniapp_url, browser_url = await _build_admin_entry_urls(user.id)
            text = tr(locale, "start.admin_portal_text")
            kb = InlineKeyboardMarkup([
                [InlineKeyboardButton(tr(locale, "buttons.open_admin_console"), web_app=WebAppInfo(url=miniapp_url))],
                [InlineKeyboardButton(tr(locale, "buttons.browser_open"), url=browser_url)],
                [InlineKeyboardButton(tr(locale, "buttons.back_to_main"), callback_data="back_to_start")],
            ])
            await update.message.reply_html(text, reply_markup=kb)
        except Exception as e:
            logger.error(f"[main_menu] 生成管理员入口失败: {e}", exc_info=True)
            await update.message.reply_text(tr(locale, "start.link_unavailable"))
        return
    if action == "merchant_web_otp":
        try:
            miniapp_url, browser_url = await _build_merchant_entry_urls(user.id)
            text = tr(locale, "start.merchant_portal_text")
            kb = InlineKeyboardMarkup([
                [InlineKeyboardButton(tr(locale, "buttons.open_merchant_console"), web_app=WebAppInfo(url=miniapp_url))],
                [InlineKeyboardButton(tr(locale, "buttons.browser_open"), url=browser_url)],
                [InlineKeyboardButton(tr(locale, "buttons.back_to_main"), callback_data="back_to_start")],
            ])
            await update.message.reply_html(text, reply_markup=kb)
        except ValueError as e:
            if str(e) == "merchant_not_found":
                await update.message.reply_text(tr(locale, "start.merchant_not_bound"))
            else:
                await update.message.reply_text(tr(locale, "start.link_unavailable"))
        except Exception as e:
            logger.error(f"[main_menu] 生成商户入口失败: {e}", exc_info=True)
            await update.message.reply_text(tr(locale, "start.link_unavailable"))
        return
    if action == "contact_admin":
        await update.message.reply_html(tr(locale, "start.contact_admin_text"))
        return


async def restart_bot_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """仅管理员：优雅停止 Application（由 main.py 中 os.execl 拉起新进程）"""
    user = update.effective_user
    if not user or not update.message:
        return
    if not _is_admin(user.id):
        await update.message.reply_text("❌ 仅管理员可使用此命令。")
        return
    await update.message.reply_text("🔄 正在重启 Bot，请稍候…")
    config.RESTART_REQUESTED = True
    context.application.stop_running()


async def button_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理主菜单按钮回调"""
    query = update.callback_query
    if not query or not query.from_user:
        return
    await query.answer()

    data = query.data
    if not data:
        return

    user_id = query.from_user.id
    is_admin = _is_admin(user_id)
    locale = await resolve_user_locale(user_id, getattr(query.from_user, "language_code", None))

    # 处理解禁验证回调
    if data.startswith("unmute_verify:"):
        parts = data.split(":")
        try:
            group_id = int(parts[1])
            verify_channel_id = parts[2]
        except (IndexError, ValueError):
            await query.answer("❌ 参数错误", show_alert=True)
            return

        # 再次检查用户是否已关注频道
        try:
            member = await context.bot.get_chat_member(chat_id=verify_channel_id, user_id=user_id)
            is_member = member.status in ('member', 'administrator', 'creator')
        except TelegramError:
            is_member = False

        if not is_member:
            await query.answer("❌ 未检测到你的关注，请先关注频道后再试", show_alert=True)
            return

        # 执行解禁
        await query.edit_message_text("⏳ 正在处理解禁请求，请稍候...")
        await _execute_unmute(context.bot, user_id, group_id)
        return

    if data == "lang_menu":
        await show_language_menu(update, context, locale)
        return

    if data.startswith("lang_set:"):
        selected_locale = await set_user_locale(user_id, data.split(":", 1)[1])
        await query.answer(tr(selected_locale, "common.language_saved", language=language_label(selected_locale)))
        await start(update, context)
        return

    if data == "admin_web_otp":
        if not is_admin:
            await query.answer(tr(locale, "common.no_permission"), show_alert=True)
            return
        try:
            miniapp_url, browser_url = await _build_admin_entry_urls(user_id)
            logger.info(f"[admin_web_otp] generated urls user_id=%s miniapp=%s", user_id, miniapp_url)
            text = tr(locale, "start.admin_portal_text")
            kb = InlineKeyboardMarkup([
                [InlineKeyboardButton(tr(locale, "buttons.open_admin_console"), web_app=WebAppInfo(url=miniapp_url))],
                [InlineKeyboardButton(tr(locale, "buttons.browser_open"), url=browser_url)],
                [InlineKeyboardButton(tr(locale, "buttons.back_to_main"), callback_data="back_to_start")],
            ])
            sent_ok = False
            try:
                await query.edit_message_text(text, reply_markup=kb, parse_mode="HTML")
                sent_ok = True
                logger.info(f"[admin_web_otp] edit_message_text sent user_id={user_id}")
            except Exception as edit_err:
                logger.warning(f"[admin_web_otp] edit_message_text failed user_id={user_id}: {edit_err}")
            if not sent_ok:
                try:
                    await context.bot.send_message(
                        chat_id=user_id,
                        text=text,
                        reply_markup=kb,
                        parse_mode="HTML"
                    )
                    sent_ok = True
                    logger.info(f"[admin_web_otp] fallback send_message sent user_id={user_id}")
                except Exception as send_err:
                    logger.error(f"[admin_web_otp] fallback send_message failed user_id={user_id}: {send_err}")
                    raise
        except Exception as e:
            logger.error(f"生成 OTP 链接失败: {e}", exc_info=True)
            await query.answer(tr(locale, "start.link_unavailable"), show_alert=True)

    elif data == "merchant_web_otp":
        try:
            miniapp_url, browser_url = await _build_merchant_entry_urls(user_id)
            logger.info(f"[merchant_web_otp] generated urls user_id=%s miniapp=%s", user_id, miniapp_url)
            text = tr(locale, "start.merchant_portal_text")
            kb = InlineKeyboardMarkup([
                [InlineKeyboardButton(tr(locale, "buttons.open_merchant_console"), web_app=WebAppInfo(url=miniapp_url))],
                [InlineKeyboardButton(tr(locale, "buttons.browser_open"), url=browser_url)],
                [InlineKeyboardButton(tr(locale, "buttons.back_to_main"), callback_data="back_to_start")],
            ])
            sent_ok = False
            try:
                await query.edit_message_text(text, reply_markup=kb, parse_mode="HTML")
                sent_ok = True
                logger.info(f"[merchant_web_otp] edit_message_text sent user_id={user_id}")
            except Exception as edit_err:
                logger.warning(f"[merchant_web_otp] edit_message_text failed user_id={user_id}: {edit_err}")
            if not sent_ok:
                try:
                    await context.bot.send_message(chat_id=user_id, text=text, reply_markup=kb, parse_mode="HTML")
                    logger.info(f"[merchant_web_otp] fallback send_message sent user_id={user_id}")
                except Exception as send_err:
                    logger.error(f"[merchant_web_otp] fallback send_message failed user_id={user_id}: {send_err}")
                    raise
        except ValueError as e:
            if str(e) == 'merchant_not_found':
                await query.answer(tr(locale, "start.merchant_not_bound"), show_alert=True)
            else:
                await query.answer(tr(locale, "start.link_unavailable"), show_alert=True)
            logger.warning(f"[merchant_web_otp] value error user_id={user_id}: {e}")
        except Exception as e:
            logger.error(f"生成商户 OTP 链接失败: {e}", exc_info=True)
            await query.answer(tr(locale, "start.link_unavailable"), show_alert=True)

    elif data == "register_merchant":
        await start_merchant_onboarding(update, context)

    elif data == "contact_admin":
        text = tr(locale, "start.contact_admin_text")
        kb = InlineKeyboardMarkup([
            [InlineKeyboardButton(tr(locale, "buttons.back_to_main"), callback_data="back_to_start")],
        ])
        sent_ok = False
        try:
            await query.edit_message_text(text, reply_markup=kb, parse_mode="HTML")
            sent_ok = True
        except Exception as edit_err:
            logger.warning(f"[contact_admin] edit_message_text failed user_id={user_id}: {edit_err}")
        if not sent_ok:
            try:
                await context.bot.send_message(
                    chat_id=user_id,
                    text=text,
                    reply_markup=kb,
                    parse_mode="HTML",
                )
            except Exception as send_err:
                logger.error(f"[contact_admin] send_message failed user_id={user_id}: {send_err}")
                await query.answer("⚠️ 暂时无法打开客服入口，请稍后重试", show_alert=True)


    elif data == "admin_settings":
        if not is_admin:
            await query.answer(tr(locale, "common.no_permission"), show_alert=True)
            return
        await query.answer(tr(locale, "common.admin_settings_web_only"), show_alert=True)

    elif data == "restart_bot":
        if not is_admin:
            await query.answer(tr(locale, "common.no_permission"), show_alert=True)
            return
        await query.edit_message_text("🔄 正在重启 Bot，请稍候…")
        config.RESTART_REQUESTED = True
        context.application.stop_running()

    elif data == "back_to_start":
        await start(update, context)

    else:
        await query.answer(tr(locale, "common.feature_maintaining"), show_alert=True)
