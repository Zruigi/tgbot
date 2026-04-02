# -*- coding: utf-8 -*-
"""
入群验证模块
支持三种验证类型：
  1. captcha  - 数字验证码
  2. quiz     - 答题（inline keyboard）
  3. channel  - 关注频道验证

新特性（v4.7）：
  - 可点击的名字 Mention: <a href="tg://user?id={user_id}">{name}</a>
  - 消息自毁机制：验证通过后删除欢迎消息 + 3 秒后自删提示
  - 超时踢人使用 context.job_queue.run_once，同时删除欢迎消息
  - pending_verify_msg_id 持久化到 group_members 表
  - 改进默认验证文案模板
"""
import asyncio
import logging
import random
from html import escape
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup, ChatPermissions, ChatMemberUpdated
from telegram.ext import ContextTypes, MessageHandler, filters, CallbackQueryHandler, ChatMemberHandler
from telegram.error import TelegramError
import database
import aiosqlite
from config import DB_NAME

logger = logging.getLogger(__name__)
_verifying_users: set = set()  # {(group_id, user_id)}


# -----------------------------------------------------------------
# 基础工具函数
# -----------------------------------------------------------------

def _make_captcha() -> str:
    return str(random.randint(100000, 999999))


def _mention_html(user_id: int, name: str) -> str:
    """生成可点击的用户名 HTML Mention，即便用户没有 @username 也可点开资料卡"""
    return f'<a href="tg://user?id={user_id}">{escape(name)}</a>'


async def _mute_user(bot, chat_id, user_id):
    logger.info("[验证] 尝试禁言 chat=%s user=%s", chat_id, user_id)
    try:
        await bot.restrict_chat_member(
            chat_id=chat_id, user_id=user_id,
            permissions=ChatPermissions(
                can_send_messages=False, can_send_polls=False,
                can_send_other_messages=False, can_add_web_page_previews=False
            )
        )
        logger.info("[验证] 禁言成功 chat=%s user=%s", chat_id, user_id)
    except TelegramError as e:
        logger.warning("禁言失败 user=%s: %s", user_id, e)


async def _unmute_user(bot, chat_id, user_id):
    try:
        await bot.restrict_chat_member(
            chat_id=chat_id, user_id=user_id,
            permissions=ChatPermissions(
                can_send_messages=True, can_send_polls=True,
                can_send_other_messages=True, can_add_web_page_previews=True,
                can_change_info=False, can_invite_users=True, can_pin_messages=False
            )
        )
    except TelegramError as e:
        logger.warning("解除禁言失败 user=%s: %s", user_id, e)


async def _kick_user(bot, chat_id, user_id):
    """踢出用户（ban + 立即 unban），允许以后重新加入，不是永久封禁"""
    try:
        await bot.ban_chat_member(chat_id=chat_id, user_id=user_id)
        await asyncio.sleep(1)
        await bot.unban_chat_member(chat_id=chat_id, user_id=user_id)
    except TelegramError as e:
        logger.warning("踢出失败 user=%s: %s", user_id, e)


async def _check_channel_membership(bot, user_id, channel_id) -> bool:
    try:
        member = await bot.get_chat_member(chat_id=channel_id, user_id=user_id)
        logger.info("[验证] 频道成员状态 user=%s channel=%s status=%s", user_id, channel_id, member.status)
        return member.status in ('member', 'administrator', 'creator')
    except TelegramError as e:
        logger.warning("[验证] 检测频道成员失败 user=%s channel=%s: %s", user_id, channel_id, e)
        return False


async def _auto_delete(bot, chat_id, message_id, delay):
    """延迟 delay 秒后删除消息"""
    await asyncio.sleep(delay)
    try:
        await bot.delete_message(chat_id=chat_id, message_id=message_id)
    except Exception:
        pass


# -----------------------------------------------------------------
# pending_verify_msg_id 持久化（存入 group_members 表）
# -----------------------------------------------------------------

async def _save_pending_msg_id(group_id: int, user_id: int, msg_id: int) -> None:
    try:
        async with aiosqlite.connect(DB_NAME) as db:
            await db.execute(
                "UPDATE group_members SET pending_verify_msg_id = ? WHERE group_id = ? AND user_id = ?",
                (msg_id, group_id, user_id)
            )
            await db.commit()
    except Exception as e:
        logger.warning("[验证] 保存 pending_verify_msg_id 失败: %s", e)


async def _get_pending_msg_id(group_id: int, user_id: int) -> int | None:
    try:
        async with aiosqlite.connect(DB_NAME) as db:
            async with db.execute(
                "SELECT pending_verify_msg_id FROM group_members WHERE group_id = ? AND user_id = ?",
                (group_id, user_id)
            ) as cursor:
                row = await cursor.fetchone()
                return row[0] if row and row[0] else None
    except Exception as e:
        logger.warning("[验证] 读取 pending_verify_msg_id 失败: %s", e)
        return None


async def _clear_pending_msg_id(group_id: int, user_id: int) -> None:
    try:
        async with aiosqlite.connect(DB_NAME) as db:
            await db.execute(
                "UPDATE group_members SET pending_verify_msg_id = NULL WHERE group_id = ? AND user_id = ?",
                (group_id, user_id)
            )
            await db.commit()
    except Exception:
        pass

# -----------------------------------------------------------------
# Job Queue 超时回调
# -----------------------------------------------------------------

async def _timeout_kick_job(context: ContextTypes.DEFAULT_TYPE) -> None:
    """
    Job Queue 回调：超时未验证 -> 踢出 + 删除欢迎消息，确保群内不留痕迹。
    job.data = {"group_id": int, "user_id": int, "session_id": int, "bot_msg_id": int}
    """
    data = context.job.data
    group_id = data["group_id"]
    user_id  = data["user_id"]
    session_id = data["session_id"]
    bot_msg_id = data["bot_msg_id"]

    session = await database.get_pending_verify_session(group_id, user_id)
    if not session or session['status'] != 'pending':
        logger.info("[验证超时] 用户 %s 已完成验证，跳过踢出", user_id)
        return

    await database.update_verify_session_status(session_id, 'expired')
    _verifying_users.discard((group_id, user_id))

    # 删除欢迎/验证消息（不留痕迹）
    try:
        await context.bot.delete_message(chat_id=group_id, message_id=bot_msg_id)
    except Exception:
        pass
    stored_msg_id = await _get_pending_msg_id(group_id, user_id)
    if stored_msg_id and stored_msg_id != bot_msg_id:
        try:
            await context.bot.delete_message(chat_id=group_id, message_id=stored_msg_id)
        except Exception:
            pass
    await _clear_pending_msg_id(group_id, user_id)

    await _kick_user(context.bot, group_id, user_id)
    logger.info("[验证超时] 已踢出用户 %s（群 %s）", user_id, group_id)

    try:
        tip = await context.bot.send_message(
            chat_id=group_id, text="\u23f0 验证超时，已移出未验证用户。"
        )
        asyncio.create_task(_auto_delete(context.bot, group_id, tip.message_id, 8))
    except Exception:
        pass


def _schedule_timeout_kick(
    context: ContextTypes.DEFAULT_TYPE,
    group_id: int, user_id: int,
    session_id: int, bot_msg_id: int, timeout: int,
) -> None:
    """
    使用 context.job_queue.run_once 注册超时任务。
    若 job_queue 不可用则回退到 asyncio.create_task。
    """
    if not context.job_queue:
        logger.warning("[验证] job_queue 不可用，回退到 asyncio.create_task")
        asyncio.create_task(
            _legacy_timeout_kick(context.bot, group_id, user_id, session_id, bot_msg_id, timeout)
        )
        return

    job_name = f"verify_timeout_{group_id}_{user_id}"
    for job in (context.job_queue.get_jobs_by_name(job_name) or []):
        job.schedule_removal()

    context.job_queue.run_once(
        _timeout_kick_job,
        when=timeout,
        data={"group_id": group_id, "user_id": user_id,
              "session_id": session_id, "bot_msg_id": bot_msg_id},
        name=job_name,
    )
    logger.info("[验证] 已注册超时任务 job=%s timeout=%ss", job_name, timeout)


async def _legacy_timeout_kick(
    bot, group_id: int, user_id: int,
    session_id: int, bot_msg_id: int, timeout: int,
) -> None:
    """job_queue 不可用时的兜底：asyncio.sleep 实现"""
    await asyncio.sleep(timeout)
    session = await database.get_pending_verify_session(group_id, user_id)
    if not session or session['status'] != 'pending':
        return
    await database.update_verify_session_status(session_id, 'expired')
    _verifying_users.discard((group_id, user_id))
    try:
        await bot.delete_message(chat_id=group_id, message_id=bot_msg_id)
    except Exception:
        pass
    stored = await _get_pending_msg_id(group_id, user_id)
    if stored and stored != bot_msg_id:
        try:
            await bot.delete_message(chat_id=group_id, message_id=stored)
        except Exception:
            pass
    await _clear_pending_msg_id(group_id, user_id)
    await _kick_user(bot, group_id, user_id)
    try:
        tip = await bot.send_message(chat_id=group_id, text="\u23f0 验证超时，已移出未验证用户。")
        asyncio.create_task(_auto_delete(bot, group_id, tip.message_id, 8))
    except Exception:
        pass


def _cancel_timeout_job(
    context: ContextTypes.DEFAULT_TYPE, group_id: int, user_id: int,
) -> None:
    """验证通过后取消超时踢人任务"""
    if not context.job_queue:
        return
    job_name = f"verify_timeout_{group_id}_{user_id}"
    for job in (context.job_queue.get_jobs_by_name(job_name) or []):
        job.schedule_removal()
    logger.info("[验证] 已取消超时任务 job=%s", job_name)

# -----------------------------------------------------------------
# 验证成功统一处理
# -----------------------------------------------------------------

async def _on_verify_success(
    bot,
    context: ContextTypes.DEFAULT_TYPE,
    group_id: int,
    user_id: int,
    display_name: str,
    bot_msg_id: int | None = None,
) -> None:
    """
    验证成功统一处理：
    1. 取消 Job Queue 超时任务
    2. 解除禁言
    3. 删除欢迎/验证消息（确保群内不留痕迹）
    4. 发送简短确认提示，3 秒后自动删除
    5. 清除 DB 中的 pending_verify_msg_id
    """
    _cancel_timeout_job(context, group_id, user_id)
    _verifying_users.discard((group_id, user_id))
    await _unmute_user(bot, group_id, user_id)

    msg_to_delete = bot_msg_id or await _get_pending_msg_id(group_id, user_id)
    if msg_to_delete:
        try:
            await bot.delete_message(chat_id=group_id, message_id=msg_to_delete)
        except Exception:
            pass
    await _clear_pending_msg_id(group_id, user_id)

    mention = _mention_html(user_id, display_name)
    try:
        confirm = await bot.send_message(
            chat_id=group_id,
            text=f"\u2705 {mention} \u9a8c\u8bc1\u901a\u8fc7\uff0c\u6b22\u8fce\u52a0\u5165\uff01",
            parse_mode='HTML',
        )
        asyncio.create_task(_auto_delete(bot, group_id, confirm.message_id, 3))
    except Exception as e:
        logger.warning("[验证] 发送确认提示失败: %s", e)


# -----------------------------------------------------------------
# 各验证类型启动函数
# -----------------------------------------------------------------

async def _start_captcha_verify(bot, context, group_id, user_id, name, timeout, member):
    code = _make_captcha()
    mention = _mention_html(user_id, name)
    text = (
        f"\U0001f44b \u6b22\u8fce {mention} \u52a0\u5165\uff01\n\n"
        f"\u8bf7\u5728 <b>{timeout}</b> \u79d2\u5185\u53d1\u9001\u4ee5\u4e0b\u9a8c\u8bc1\u7801\uff0c\u5426\u5219\u5c06\u88ab\u79fb\u51fa\u7fa4\u7ec4\u3002\n\n"
        f"\U0001f510 \u9a8c\u8bc1\u7801\uff1a<code>{code}</code>"
    )
    msg = await bot.send_message(chat_id=group_id, text=text, parse_mode='HTML')
    session_id = await database.create_verify_session(
        group_id=group_id, user_id=user_id,
        username=member.username or '', first_name=member.first_name or '',
        verify_type='captcha', answer=code,
        bot_msg_id=msg.message_id, timeout_secs=timeout
    )
    await _save_pending_msg_id(group_id, user_id, msg.message_id)
    _schedule_timeout_kick(context, group_id, user_id, session_id, msg.message_id, timeout)


async def _start_quiz_verify(bot, context, group_id, user_id, name, timeout, member, questions):
    if not questions:
        await _start_captcha_verify(bot, context, group_id, user_id, name, timeout, member)
        return
    q = random.choice(questions)
    question_text = q.get('question', '\u8bf7\u56de\u7b54\u95ee\u9898')
    options = q.get('options', [])
    correct_idx = q.get('correct', 0)
    keyboard = [
        [InlineKeyboardButton(opt, callback_data=f"verify_quiz:{group_id}:{user_id}:{i}:{correct_idx}")]
        for i, opt in enumerate(options)
    ]
    mention = _mention_html(user_id, name)
    text = (
        f"\U0001f44b \u6b22\u8fce {mention} \u52a0\u5165\uff01\n\n"
        f"\u8bf7\u5728 <b>{timeout}</b> \u79d2\u5185\u56de\u7b54\u4ee5\u4e0b\u95ee\u9898\u5b8c\u6210\u9a8c\u8bc1\u3002\n\n"
        f"\u2753 {question_text}"
    )
    msg = await bot.send_message(
        chat_id=group_id, text=text, parse_mode='HTML',
        reply_markup=InlineKeyboardMarkup(keyboard)
    )
    session_id = await database.create_verify_session(
        group_id=group_id, user_id=user_id,
        username=member.username or '', first_name=member.first_name or '',
        verify_type='quiz', answer=str(correct_idx),
        bot_msg_id=msg.message_id, timeout_secs=timeout
    )
    await _save_pending_msg_id(group_id, user_id, msg.message_id)
    _schedule_timeout_kick(context, group_id, user_id, session_id, msg.message_id, timeout)


async def _start_channel_verify(bot, context, group_id, user_id, name, timeout, member, channel_id, template=None):
    if not channel_id:
        await _start_captcha_verify(bot, context, group_id, user_id, name, timeout, member)
        return
    keyboard = [[InlineKeyboardButton(
        "\u2705 \u6211\u5df2\u5173\u6ce8\uff0c\u70b9\u51fb\u9a8c\u8bc1",
        callback_data=f"verify_channel:{group_id}:{user_id}:{channel_id}"
    )]]
    mention = _mention_html(user_id, name)
    if template:
        text = (
            template
            .replace('{mention}', mention)
            .replace('{name}', escape(name))
            .replace('{user_id}', str(user_id))
            .replace('{timeout}', str(timeout))
            .replace('{channel_link}', str(channel_id))
        )
    else:
        text = (
            f"\U0001f44b \u6b22\u8fce {mention} \u52a0\u5165\uff01\n\n"
            f"\u8bf7\u5148\u5173\u6ce8\u9891\u9053\uff0c\u7136\u540e\u70b9\u51fb\u4e0b\u65b9\u6309\u9215\u5b8c\u6210\u9a8c\u8bc1\u3002\n"
            f"\u672a\u5728 <b>{timeout}</b> \u79d2\u5185\u5b8c\u6210\u5c06\u88ab\u79fb\u51fa\u7fa4\u7ec4\u3002\n\n"
            f"\U0001f4e2 \u9891\u9053\uff1a{channel_id}"
        )
    msg = await bot.send_message(
        chat_id=group_id, text=text, parse_mode='HTML',
        reply_markup=InlineKeyboardMarkup(keyboard)
    )
    session_id = await database.create_verify_session(
        group_id=group_id, user_id=user_id,
        username=member.username or '', first_name=member.first_name or '',
        verify_type='channel', answer=channel_id,
        bot_msg_id=msg.message_id, timeout_secs=timeout
    )
    await _save_pending_msg_id(group_id, user_id, msg.message_id)
    _schedule_timeout_kick(context, group_id, user_id, session_id, msg.message_id, timeout)

# -----------------------------------------------------------------
# 新成员入群处理（MessageHandler + ChatMemberHandler 两路）
# -----------------------------------------------------------------

async def _dispatch_verify(context, bot, group_id, user_id, name, timeout, member, config):
    """根据配置派发到对应验证类型"""
    verify_type = config.get('verify_type', 'captcha')
    if verify_type == 'quiz':
        await _start_quiz_verify(
            bot, context, group_id, user_id, name, timeout, member,
            config.get('verify_questions') or []
        )
    elif verify_type == 'channel':
        await _start_channel_verify(
            bot, context, group_id, user_id, name, timeout, member,
            config.get('verify_channel_id', ''),
            template=config.get('verify_msg_template'),
        )
    else:
        await _start_captcha_verify(bot, context, group_id, user_id, name, timeout, member)


async def handle_new_member_verify(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    message = update.message
    if not message or not message.new_chat_members:
        return
    group_id = message.chat.id
    cfg = await database.get_group_verify_config(group_id)

    for member in message.new_chat_members:
        if member.is_bot:
            continue
        user_id = member.id
        try:
            await database.save_group_member(
                user_id=user_id, username=member.username or f"User{user_id}",
                first_name=member.first_name, last_name=member.last_name,
                group_id=group_id, group_title=message.chat.title,
            )
        except Exception as e:
            logger.error("保存成员失败: %s", e)

        if not cfg.get('verify_enabled'):
            continue
        key = (group_id, user_id)
        if key in _verifying_users:
            continue
        _verifying_users.add(key)
        timeout = cfg.get('verify_timeout', 120)
        name = member.first_name or member.username or str(user_id)
        await _mute_user(context.bot, group_id, user_id)
        try:
            await _dispatch_verify(context, context.bot, group_id, user_id, name, timeout, member, cfg)
        except Exception as e:
            logger.error("启动验证失败 user=%s: %s", user_id, e)
            _verifying_users.discard(key)


async def handle_chat_member_updated(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理超级群新成员加入（ChatMemberUpdated 事件）"""
    result: ChatMemberUpdated = update.chat_member
    if not result:
        return
    old_status = result.old_chat_member.status
    new_status = result.new_chat_member.status
    logger.info("[验证] ChatMemberUpdated: group=%s user=%s %s->%s",
                result.chat.id, result.new_chat_member.user.id, old_status, new_status)
    joined = old_status in ('left', 'kicked') and new_status in ('member', 'restricted')
    if not joined:
        return
    member = result.new_chat_member.user
    if member.is_bot:
        return
    group_id = result.chat.id
    user_id = member.id
    cfg = await database.get_group_verify_config(group_id)
    try:
        await database.save_group_member(
            user_id=user_id, username=member.username or f'User{user_id}',
            first_name=member.first_name, last_name=member.last_name,
            group_id=group_id, group_title=result.chat.title,
        )
    except Exception as e:
        logger.error('保存成员失败: %s', e)
    if not cfg.get('verify_enabled'):
        return
    key = (group_id, user_id)
    if key in _verifying_users:
        return
    _verifying_users.add(key)
    timeout = cfg.get('verify_timeout', 120)
    name = member.first_name or member.username or str(user_id)
    await _mute_user(context.bot, group_id, user_id)
    try:
        await _dispatch_verify(context, context.bot, group_id, user_id, name, timeout, member, cfg)
    except Exception as e:
        logger.error('启动验证失败 user=%s: %s', user_id, e)
        _verifying_users.discard(key)


# -----------------------------------------------------------------
# 验证码文字回复处理
# -----------------------------------------------------------------

async def handle_verify_message(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理验证码文字回复"""
    message = update.message
    if not message or not message.text:
        return
    group_id = message.chat.id
    user_id = message.from_user.id
    if (group_id, user_id) not in _verifying_users:
        return
    session = await database.get_pending_verify_session(group_id, user_id)
    if not session or session['verify_type'] != 'captcha':
        return
    if message.text.strip() == session['answer']:
        await database.update_verify_session_status(session['session_id'], 'passed')
        try:
            await message.delete()
        except Exception:
            pass
        await _on_verify_success(
            context.bot, context, group_id, user_id,
            message.from_user.first_name or str(user_id),
            bot_msg_id=session['bot_msg_id'],
        )
    else:
        try:
            await message.delete()
        except Exception:
            pass


# -----------------------------------------------------------------
# Inline 按钮回调（quiz / channel）
# -----------------------------------------------------------------

async def handle_verify_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理答题/频道验证回调"""
    query = update.callback_query
    await query.answer()
    data = query.data

    if data.startswith('verify_quiz:'):
        _, group_id, user_id, chosen, correct = data.split(':')
        group_id, user_id, chosen, correct = int(group_id), int(user_id), int(chosen), int(correct)
        if query.from_user.id != user_id:
            await query.answer("\u274c \u8fd9\u4e0d\u662f\u7ed9\u4f60\u7684\u9a8c\u8bc1\u9898", show_alert=True)
            return
        session = await database.get_pending_verify_session(group_id, user_id)
        if not session:
            return
        display = query.from_user.first_name or str(user_id)
        if chosen == correct:
            await database.update_verify_session_status(session['session_id'], 'passed')
            await _on_verify_success(
                context.bot, context, group_id, user_id, display,
                bot_msg_id=query.message.message_id,
            )
        else:
            await database.update_verify_session_status(session['session_id'], 'failed')
            _cancel_timeout_job(context, group_id, user_id)
            _verifying_users.discard((group_id, user_id))
            try:
                await query.message.delete()
            except Exception:
                pass
            await _clear_pending_msg_id(group_id, user_id)
            await _kick_user(context.bot, group_id, user_id)
            mention = _mention_html(user_id, display)
            try:
                tip = await context.bot.send_message(
                    chat_id=group_id,
                    text=f"\u274c {mention} \u7b54\u9898\u9519\u8bef\uff0c\u5df2\u79fb\u51fa\u7fa4\u7ec4\u3002",
                    parse_mode='HTML',
                )
                asyncio.create_task(_auto_delete(context.bot, group_id, tip.message_id, 10))
            except Exception:
                pass

    elif data.startswith('verify_channel:'):
        parts = data.split(':', 3)
        group_id, user_id = int(parts[1]), int(parts[2])
        channel_id = parts[3]
        if query.from_user.id != user_id:
            await query.answer("\u274c \u8fd9\u4e0d\u662f\u7ed9\u4f60\u7684\u9a8c\u8bc1", show_alert=True)
            return
        session = await database.get_pending_verify_session(group_id, user_id)
        if not session:
            return
        is_member = await _check_channel_membership(context.bot, user_id, channel_id)
        logger.info("[验证] 频道关注检测结果 user=%s channel=%s is_member=%s", user_id, channel_id, is_member)
        display = query.from_user.first_name or str(user_id)
        if is_member:
            await database.update_verify_session_status(session['session_id'], 'passed')
            await database.set_member_verified(group_id, user_id)
            await _on_verify_success(
                context.bot, context, group_id, user_id, display,
                bot_msg_id=query.message.message_id,
            )
        else:
            await query.answer("\u274c \u672a\u68c0\u6d4b\u5230\u5173\u6ce8\uff0c\u8bf7\u5148\u5173\u6ce8\u9891\u9053\u540e\u518d\u70b9\u51fb\u9a8c\u8bc1", show_alert=True)


# -----------------------------------------------------------------
# Handler 注册对象
# -----------------------------------------------------------------

new_member_verify_handler = MessageHandler(
    filters.StatusUpdate.NEW_CHAT_MEMBERS,
    handle_new_member_verify,
)

chat_member_verify_handler = ChatMemberHandler(
    handle_chat_member_updated,
    ChatMemberHandler.CHAT_MEMBER,
)

verify_message_handler = MessageHandler(
    filters.TEXT & filters.ChatType.GROUPS & ~filters.COMMAND,
    handle_verify_message,
)

verify_callback_handler = CallbackQueryHandler(
    handle_verify_callback,
    pattern=r'^verify_(quiz|channel):',
)
