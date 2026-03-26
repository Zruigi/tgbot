# -*- coding: utf-8 -*-
"""
\u5165\u7fa4\u9a8c\u8bc1\u6a21\u5757
\u652f\u6301\u4e09\u79cd\u9a8c\u8bc1\u7c7b\u578b\uff1a
  1. captcha  - \u6570\u5b57\u9a8c\u8bc1\u7801
  2. quiz     - \u7b54\u9898\uff08inline keyboard\uff09
  3. channel  - \u5173\u6ce8\u9891\u9053\u9a8c\u8bc1
"""
import asyncio
import logging
import random
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup, ChatPermissions, ChatMemberUpdated
from telegram.ext import ContextTypes, MessageHandler, filters, CallbackQueryHandler, ChatMemberHandler
from telegram.error import TelegramError
import database

logger = logging.getLogger(__name__)
_verifying_users: set = set()  # (group_id, user_id)


def _make_captcha() -> str:
    return str(random.randint(100000, 999999))


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
        logger.warning("\u7981\u8a00\u5931\u8d25 user=%s: %s", user_id, e)


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
        logger.warning("\u89e3\u9664\u7981\u8a00\u5931\u8d25 user=%s: %s", user_id, e)


async def _kick_user(bot, chat_id, user_id):
    try:
        await bot.ban_chat_member(chat_id=chat_id, user_id=user_id)
        await asyncio.sleep(1)
        await bot.unban_chat_member(chat_id=chat_id, user_id=user_id)
    except TelegramError as e:
        logger.warning("\u8e22\u51fa\u5931\u8d25 user=%s: %s", user_id, e)


async def _check_channel_membership(bot, user_id, channel_id) -> bool:
    try:
        member = await bot.get_chat_member(chat_id=channel_id, user_id=user_id)
        logger.info("[验证] 频道成员状态 user=%s channel=%s status=%s", user_id, channel_id, member.status)
        return member.status in ('member', 'administrator', 'creator')
    except TelegramError as e:
        logger.warning("[验证] 检测频道成员失败 user=%s channel=%s: %s", user_id, channel_id, e)
        return False


async def _auto_delete(bot, chat_id, message_id, delay):
    await asyncio.sleep(delay)
    try:
        await bot.delete_message(chat_id=chat_id, message_id=message_id)
    except Exception:
        pass


async def _timeout_kick(bot, group_id, user_id, session_id, bot_msg_id, timeout):
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
    await _kick_user(bot, group_id, user_id)
    try:
        tip = await bot.send_message(chat_id=group_id, text="\u23f0 \u9a8c\u8bc1\u8d85\u65f6\uff0c\u5df2\u79fb\u51fa\u672a\u9a8c\u8bc1\u7528\u6237\u3002")
        asyncio.create_task(_auto_delete(bot, group_id, tip.message_id, 8))
    except Exception:
        pass


async def _start_captcha_verify(bot, group_id, user_id, name, timeout, member):
    code = _make_captcha()
    text = (
        f"👋 \u6b22\u8fce <b>{name}</b> \u52a0\u5165\uff01\n\n"
        f"\u8bf7\u5728 <b>{timeout}\u79d2</b> \u5185\u53d1\u9001\u4ee5\u4e0b\u9a8c\u8bc1\u7801\uff0c\u5426\u5219\u5c06\u88ab\u79fb\u51fa\u7fa4\u7ec4\u3002\n\n"
        f"\U0001f510 \u9a8c\u8bc1\u7801\uff1a<code>{code}</code>"
    )
    msg = await bot.send_message(chat_id=group_id, text=text, parse_mode='HTML')
    session_id = await database.create_verify_session(
        group_id=group_id, user_id=user_id,
        username=member.username or '', first_name=member.first_name or '',
        verify_type='captcha', answer=code,
        bot_msg_id=msg.message_id, timeout_secs=timeout
    )
    asyncio.create_task(_timeout_kick(bot, group_id, user_id, session_id, msg.message_id, timeout))


async def _start_quiz_verify(bot, group_id, user_id, name, timeout, member, questions):
    if not questions:
        await _start_captcha_verify(bot, group_id, user_id, name, timeout, member)
        return
    q = random.choice(questions)
    question_text = q.get('question', '\u8bf7\u56de\u7b54\u95ee\u9898')
    options = q.get('options', [])
    correct_idx = q.get('correct', 0)
    keyboard = [
        [InlineKeyboardButton(opt, callback_data=f"verify_quiz:{group_id}:{user_id}:{i}:{correct_idx}")]
        for i, opt in enumerate(options)
    ]
    text = (
        f"\U0001f44b \u6b22\u8fce <b>{name}</b> \u52a0\u5165\uff01\n\n"
        f"\u8bf7\u5728 <b>{timeout}\u79d2</b> \u5185\u56de\u7b54\u4ee5\u4e0b\u95ee\u9898\u5b8c\u6210\u9a8c\u8bc1\u3002\n\n"
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
    asyncio.create_task(_timeout_kick(bot, group_id, user_id, session_id, msg.message_id, timeout))


async def _start_channel_verify(bot, group_id, user_id, name, timeout, member, channel_id):
    if not channel_id:
        await _start_captcha_verify(bot, group_id, user_id, name, timeout, member)
        return
    keyboard = [[InlineKeyboardButton(
        "\u2705 \u6211\u5df2\u5173\u6ce8\uff0c\u70b9\u51fb\u9a8c\u8bc1",
        callback_data=f"verify_channel:{group_id}:{user_id}:{channel_id}"
    )]]
    text = (
        f"\U0001f44b \u6b22\u8fce <b>{name}</b> \u52a0\u5165\uff01\n\n"
        f"\u8bf7\u5148\u5173\u6ce8\u9891\u9053\uff0c\u7136\u540e\u70b9\u51fb\u4e0b\u65b9\u6309\u9215\u5b8c\u6210\u9a8c\u8bc1\u3002\n"
        f"\u672a\u5728 <b>{timeout}\u79d2</b> \u5185\u5b8c\u6210\u5c06\u88ab\u79fb\u51fa\u7fa4\u7ec4\u3002\n\n"
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
    asyncio.create_task(_timeout_kick(bot, group_id, user_id, session_id, msg.message_id, timeout))


async def handle_new_member_verify(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    message = update.message
    if not message or not message.new_chat_members:
        return
    group_id = message.chat.id
    config = await database.get_group_verify_config(group_id)

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
            logger.error("\u4fdd\u5b58\u6210\u5458\u5931\u8d25: %s", e)

        if not config.get('verify_enabled'):
            continue

        key = (group_id, user_id)
        if key in _verifying_users:
            continue
        _verifying_users.add(key)

        verify_type = config.get('verify_type', 'captcha')
        timeout = config.get('verify_timeout', 120)
        name = member.first_name or member.username or str(user_id)
        await _mute_user(context.bot, group_id, user_id)

        try:
            if verify_type == 'quiz':
                await _start_quiz_verify(
                    context.bot, group_id, user_id, name, timeout, member,
                    config.get('verify_questions') or []
                )
            elif verify_type == 'channel':
                await _start_channel_verify(
                    context.bot, group_id, user_id, name, timeout, member,
                    config.get('verify_channel_id', '')
                )
            else:
                await _start_captcha_verify(context.bot, group_id, user_id, name, timeout, member)
        except Exception as e:
            logger.error("\u542f\u52a8\u9a8c\u8bc1\u5931\u8d25 user=%s: %s", user_id, e)
            _verifying_users.discard(key)


async def handle_verify_message(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """\u5904\u7406\u9a8c\u8bc1\u7801\u6587\u5b57\u56de\u590d"""
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
        _verifying_users.discard((group_id, user_id))
        await _unmute_user(context.bot, group_id, user_id)
        try:
            await context.bot.delete_message(chat_id=group_id, message_id=session['bot_msg_id'])
            await message.delete()
        except Exception:
            pass
        confirm = await context.bot.send_message(
            chat_id=group_id,
            text=f"\u2705 {message.from_user.first_name} \u9a8c\u8bc1\u901a\u8fc7\uff0c\u6b22\u8fce\u5165\u7fa4\uff01"
        )
        asyncio.create_task(_auto_delete(context.bot, group_id, confirm.message_id, 10))
    else:
        try:
            await message.delete()
        except Exception:
            pass


async def handle_verify_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """\u5904\u7406\u7b54\u9898/\u9891\u9053\u9a8c\u8bc1\u56de\u8c03"""
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
        if chosen == correct:
            await database.update_verify_session_status(session['session_id'], 'passed')
            _verifying_users.discard((group_id, user_id))
            await _unmute_user(context.bot, group_id, user_id)
            try:
                await query.message.delete()
            except Exception:
                pass
            confirm = await context.bot.send_message(
                chat_id=group_id,
                text=f"\u2705 {query.from_user.first_name} \u7b54\u9898\u6b63\u786e\uff0c\u9a8c\u8bc1\u901a\u8fc7\uff0c\u6b22\u8fce\u5165\u7fa4\uff01"
            )
            asyncio.create_task(_auto_delete(context.bot, group_id, confirm.message_id, 10))
        else:
            await database.update_verify_session_status(session['session_id'], 'failed')
            _verifying_users.discard((group_id, user_id))
            try:
                await query.message.delete()
            except Exception:
                pass
            await _kick_user(context.bot, group_id, user_id)
            tip = await context.bot.send_message(
                chat_id=group_id,
                text=f"\u274c {query.from_user.first_name} \u7b54\u9898\u9519\u8bef\uff0c\u5df2\u79fb\u51fa\u7fa4\u7ec4\u3002"
            )
            asyncio.create_task(_auto_delete(context.bot, group_id, tip.message_id, 10))

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
        if is_member:
            await database.update_verify_session_status(session['session_id'], 'passed')
            _verifying_users.discard((group_id, user_id))
            await _unmute_user(context.bot, group_id, user_id)
            try:
                await query.message.delete()
            except Exception:
                pass
            confirm = await context.bot.send_message(
                chat_id=group_id,
                text=f"✅ {query.from_user.first_name} 已关注频道，验证通过，欢迎入群！"
            )
            asyncio.create_task(_auto_delete(context.bot, group_id, confirm.message_id, 10))
        else:
            await query.answer("❌ 未检测到关注，请先关注频道后再点击验证", show_alert=True)


async def handle_chat_member_updated(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理超级群新成员加入（ChatMemberUpdated 事件）"""
    result: ChatMemberUpdated = update.chat_member
    if not result:
        logger.warning("[验证] chat_member 为空")
        return
    old_status = result.old_chat_member.status
    new_status = result.new_chat_member.status
    logger.info("[验证] ChatMemberUpdated: group=%s user=%s %s->%s", result.chat.id, result.new_chat_member.user.id, old_status, new_status)
    # 只处理「加入」动作：left/kicked → member/restricted
    joined = old_status in ('left', 'kicked') and new_status in ('member', 'restricted')
    if not joined:
        logger.info("[验证] 非加入事件，跳过")
        return
    member = result.new_chat_member.user
    if member.is_bot:
        return
    group_id = result.chat.id
    user_id = member.id
    config = await database.get_group_verify_config(group_id)
    logger.info("[验证] 群%s 验证配置: enabled=%s type=%s", group_id, config.get('verify_enabled'), config.get('verify_type'))
    try:
        await database.save_group_member(
            user_id=user_id, username=member.username or f'User{user_id}',
            first_name=member.first_name, last_name=member.last_name,
            group_id=group_id, group_title=result.chat.title,
        )
    except Exception as e:
        logger.error('保存成员失败: %s', e)
    if not config.get('verify_enabled'):
        return
    key = (group_id, user_id)
    if key in _verifying_users:
        return
    _verifying_users.add(key)
    verify_type = config.get('verify_type', 'captcha')
    timeout = config.get('verify_timeout', 120)
    name = member.first_name or member.username or str(user_id)
    await _mute_user(context.bot, group_id, user_id)
    try:
        if verify_type == 'quiz':
            await _start_quiz_verify(
                context.bot, group_id, user_id, name, timeout, member,
                config.get('verify_questions') or []
            )
        elif verify_type == 'channel':
            await _start_channel_verify(
                context.bot, group_id, user_id, name, timeout, member,
                config.get('verify_channel_id', '')
            )
        else:
            await _start_captcha_verify(context.bot, group_id, user_id, name, timeout, member)
    except Exception as e:
        logger.error('启动验证失败 user=%s: %s', user_id, e)
        _verifying_users.discard(key)


# Handler 注册对象
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
 