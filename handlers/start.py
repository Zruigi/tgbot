# -*- coding: utf-8 -*-
"""
/start 命令处理 — v3.3 规范
- 双轨菜单（管理员 / 普通用户）
- 双列排版
- OTP 动态链接（不暴露明文 Token）
- Deep Linking 自助解禁：unmute_{group_id} 参数处理
"""
import logging
import time
from datetime import datetime, timedelta

from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup, ChatPermissions
from telegram.ext import ContextTypes
from telegram.error import TelegramError
import config
import web_admin
import database

logger = logging.getLogger(__name__)

# 频率限制缓存：{user_id: last_verify_time}
_unmute_rate_limit = {}


def _is_admin(user_id: int) -> bool:
    """判断是否为管理员"""
    return user_id in config.ADMIN_TG_IDS


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



def _admin_keyboard(user_id: int = 0) -> InlineKeyboardMarkup:
    """管理员专属菜单（双列排版）— Web后台按钮直接使用 url 跳转"""
    # 生成 OTP + 一次性加密入口路由，降低固定登录入口暴露与重放风险
    try:
        otp = web_admin.generate_otp()
        url = f"{config.FRONTEND_URL}/?otp={otp}"
        logger.info(f"Final Login URL Sent to TG: {url}")
        web_btn = InlineKeyboardButton("🌐 Web 管理后台", url=url)
    except Exception as e:
        logger.warning(f"生成 OTP 失败，降级为 callback: {e}")
        web_btn = InlineKeyboardButton("🌐 Web 管理后台", callback_data="admin_web_otp")

    keyboard = [
        # 首行：Web 后台独占一行（重要入口，url 直接跳转）
        [web_btn],
        # 双列行
        [InlineKeyboardButton("⚙️ 系统设置", callback_data="admin_settings"),
         InlineKeyboardButton("📈 现货行情", callback_data="spot_price")],
        [InlineKeyboardButton("📉 合约行情", callback_data="futures_price"),
         InlineKeyboardButton("👛 监听钱包", callback_data="wallet_monitor")],
        [InlineKeyboardButton("📋 我的列表", callback_data="wallet_my_list"),
         InlineKeyboardButton("📧 邮箱设置", callback_data="email_settings")],
    ]
    return InlineKeyboardMarkup(keyboard)


def _user_keyboard() -> InlineKeyboardMarkup:
    """普通用户菜单（双列排版，客服置顶整行）"""
    keyboard = [
        # 首行：客服独占
        [InlineKeyboardButton("🎧 联系人工客服", callback_data="contact_admin")],
        # 双列行
        [InlineKeyboardButton("📈 现货行情", callback_data="spot_price"),
         InlineKeyboardButton("📉 合约行情", callback_data="futures_price")],
        [InlineKeyboardButton("👛 监听钱包", callback_data="wallet_monitor"),
         InlineKeyboardButton("📋 我的列表", callback_data="wallet_my_list")],
        [InlineKeyboardButton("📧 邮箱设置", callback_data="email_settings")],
    ]
    return InlineKeyboardMarkup(keyboard)


async def start(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理 /start 命令，显示主菜单或处理 Deep Linking 参数"""
    user = update.effective_user
    if not user:
        return

    # 解析 Deep Linking 参数
    args = context.args or []
    if args and args[0].startswith("unmute_"):
        await _handle_unmute_deeplink(update, context, user.id, args[0])
        return

    is_admin = _is_admin(user.id)
    keyboard = _admin_keyboard(user.id) if is_admin else _user_keyboard()

    role_tag = "👑 管理员" if is_admin else "👤 用户"
    text = (
        f"你好，{user.mention_html()} {role_tag}\n"
        f"UserID: <code>{user.id}</code>\n\n"
        "请选择功能："
    )

    if update.message:
        await update.message.reply_html(text, reply_markup=keyboard)
    elif update.callback_query:
        try:
            await update.callback_query.edit_message_text(
                text, reply_markup=keyboard, parse_mode="HTML"
            )
        except Exception:
            chat_id = update.callback_query.message.chat_id if update.callback_query.message else user.id
            await context.bot.send_message(
                chat_id=chat_id, text=text,
                reply_markup=keyboard, parse_mode="HTML"
            )


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

    if data == "admin_web_otp":
        if not is_admin:
            await query.answer("⛔ 无权限", show_alert=True)
            return
        try:
            # 使用 FRONTEND_URL（指向 5173 Vite 前端），而非后端 8080
            frontend_url = config.FRONTEND_URL
            if not frontend_url or '127.0.0.1' in frontend_url or 'localhost' in frontend_url:
                await query.answer(
                    "⚠️ 暂时无法生成登录链接\n请配置 FRONTEND_URL 环境变量指向前端隧道地址",
                    show_alert=True
                )
                return
            otp = web_admin.generate_otp()
            if not otp:
                await query.answer("⚠️ OTP 生成失败，请重试", show_alert=True)
                return
            otp_url = f"{frontend_url}/?otp={otp}"
            logger.info(f"Final Login URL Sent to TG: {otp_url}")
            if not otp_url or not otp_url.startswith('http'):
                await query.answer("⚠️ 登录链接异常，请检查 FRONTEND_URL 配置", show_alert=True)
                return
            text = (
                "🚀 <b>Web 管理后台</b>\n\n"
                "点击下方按钮一键免密登录后台。\n"
                "⚠️ 此链接 <b>5 分钟内有效</b>，仅限单次使用，请勿转发。"
            )
            kb = InlineKeyboardMarkup([
                [InlineKeyboardButton("🌐 立即进入管理后台", url=otp_url)],
                [InlineKeyboardButton("⬅️ 返回主菜单", callback_data="back_to_start")],
            ])
            try:
                await query.edit_message_text(text, reply_markup=kb, parse_mode="HTML")
            except Exception:
                await context.bot.send_message(
                    chat_id=user_id, text=text, reply_markup=kb, parse_mode="HTML"
                )
        except Exception as e:
            logger.error(f"生成 OTP 链接失败: {e}")
            await query.answer("⚠️ 暂时无法生成登录链接，请检查后端配置", show_alert=True)

    elif data == "contact_admin":
        await query.edit_message_text(
            "🎧 <b>人工客服</b>\n\n"
            "你可以直接在此聊天框发送消息、图片或文件，我会实时转达给客服人员。",
            parse_mode="HTML"
        )

    elif data == "admin_settings":
        if not is_admin:
            await query.answer("⛔ 无权限", show_alert=True)
            return
        await query.answer("系统设置请在 Web 后台操作。", show_alert=True)

    elif data == "restart_bot":
        if not is_admin:
            await query.answer("⛔ 无权限", show_alert=True)
            return
        await query.edit_message_text("🔄 正在重启 Bot，请稍候…")
        config.RESTART_REQUESTED = True
        context.application.stop_running()

    elif data == "back_to_start":
        await start(update, context)

    else:
        await query.answer("该功能正在维护中…", show_alert=True)
