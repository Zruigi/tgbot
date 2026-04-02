# -*- coding: utf-8 -*-
"""
频道关注验证处理器 - 检查用户是否关注了频道
"""

import logging
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.ext import ContextTypes

logger = logging.getLogger(__name__)

async def check_channel_subscription(bot, user_id: int, channel_id: int) -> bool:
    """
    检查用户是否关注了频道
    
    Args:
        bot: Telegram bot 实例
        user_id: 用户 ID
        channel_id: 频道 ID
    
    Returns:
        True 如果用户是频道成员，False 否则
    """
    # 频道/匿名主体不做个人关注校验，避免误判
    if int(user_id) <= 0:
        return True

    try:
        member = await bot.get_chat_member(chat_id=channel_id, user_id=user_id)
        # 检查成员状态
        return member.status in ['member', 'administrator', 'creator']
    except Exception as e:
        logger.error(f"检查频道成员失败: {e}")
        return False


async def handle_subscribe_button(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """
    处理"回补订阅解开禁言"按钮点击
    
    Args:
        update: Telegram update 对象
        context: 上下文对象
    """
    query = update.callback_query
    user_id = query.from_user.id
    
    try:
        # 从 callback_data 中提取群组 ID 和频道 ID
        # 格式: subscribe_{group_id}_{channel_id}
        parts = query.data.split('_')
        if len(parts) < 3:
            await query.answer("参数错误", show_alert=True)
            return
        
        group_id = int(parts[1])
        channel_id = int(parts[2])
        
        # 检查用户是否关注了频道
        is_subscribed = await check_channel_subscription(context.bot, user_id, channel_id)
        
        if is_subscribed:
            # 用户已关注，执行解禁
            await context.bot.restrict_chat_member(
                chat_id=group_id,
                user_id=user_id,
                permissions=None  # None 表示恢复所有权限
            )
            
            # 发送成功消息
            await query.answer("✅ 验证成功！您已被解禁", show_alert=True)
            
            # 删除提醒消息（如果有）
            try:
                await context.bot.delete_message(
                    chat_id=group_id,
                    message_id=query.message.message_id
                )
            except Exception as e:
                logger.warning(f"删除消息失败: {e}")
            
            logger.info(f"用户 {user_id} 已验证并解禁（群组 {group_id}）")
        else:
            # 用户未关注，显示关注链接
            channel_link = f"https://t.me/{channel_id}" if str(channel_id).startswith('-100') else f"https://t.me/c/{str(channel_id)[4:]}"
            
            keyboard = InlineKeyboardMarkup([
                [InlineKeyboardButton("📢 关注频道", url=channel_link)],
                [InlineKeyboardButton("✅ 已关注，重新验证", callback_data=f"subscribe_{group_id}_{channel_id}")]
            ])
            
            await query.edit_message_text(
                text=f"""❌ 您还未关注频道

请先关注我们的频道，然后点击"已关注，重新验证"按钮。

📢 频道链接: {channel_link}""",
                reply_markup=keyboard
            )
            
            await query.answer("请先关注频道", show_alert=False)
    
    except Exception as e:
        logger.error(f"处理订阅按钮失败: {e}")
        await query.answer("处理失败，请稍后重试", show_alert=True)


async def send_channel_reminder_message(bot, group_id: int, user_id: int, username: str, channel_id: int, channel_link: str) -> int:
    """
    发送频道关注提醒消息
    
    Args:
        bot: Telegram bot 实例
        group_id: 群组 ID
        user_id: 用户 ID
        username: 用户名（用于 @提醒）
        channel_id: 频道 ID
        channel_link: 频道链接
    
    Returns:
        消息 ID
    """
    try:
        keyboard = InlineKeyboardMarkup([
            [InlineKeyboardButton("📢 直接关注", url=channel_link)],
            [InlineKeyboardButton("✅ 回补订阅解开禁言", callback_data=f"subscribe_{group_id}_{channel_id}")]
        ])
        
        # 使用用户名进行 @提醒（如果有用户名）
        mention = f"@{username}" if username else f"<a href=\"tg://user?id={user_id}\">用户</a>"
        
        text = f"""🔔 {mention}，请关注我们的频道以获得最新信息！

📢 频道链接: {channel_link}
🤖 或点击下方按钮: 回补订阅解开禁言

⏰ 此消息将在12小时后自动删除"""
        
        msg = await bot.send_message(
            chat_id=group_id,
            text=text,
            parse_mode='HTML',
            reply_markup=keyboard
        )
        
        logger.info(f"已发送提醒消息给用户 {username or user_id}（群组 {group_id}，消息 ID {msg.message_id}）")
        return msg.message_id
    
    except Exception as e:
        logger.error(f"发送提醒消息失败: {e}")
        raise


async def handle_start_command_with_channel_check(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """
    处理 /start 命令，检查用户是否关注了频道
    
    如果用户未关注频道，显示"回补订阅解开禁言"按钮
    """
    user = update.effective_user
    
    try:
        # 从 context.args 中获取参数
        if not context.args:
            # 无参数，显示欢迎消息
            await update.message.reply_text(
                "👋 欢迎使用本机器人！\n\n"
                "请关注我们的频道以获得最新信息。"
            )
            return
        
        # 解析参数
        param = context.args[0]
        
        # 检查是否是频道验证参数
        if param.startswith('channel_'):
            # 格式: channel_{group_id}_{channel_id}
            parts = param.split('_')
            if len(parts) >= 3:
                group_id = int(parts[1])
                channel_id = int(parts[2])
                
                # 检查用户是否关注了频道
                is_subscribed = await check_channel_subscription(context.bot, user.id, channel_id)
                
                if is_subscribed:
                    # 用户已关注，执行解禁
                    await context.bot.restrict_chat_member(
                        chat_id=group_id,
                        user_id=user.id,
                        permissions=None
                    )
                    
                    await update.message.reply_text(
                        "✅ 验证成功！您已被解禁。\n\n"
                        "感谢您关注我们的频道！"
                    )
                    
                    logger.info(f"用户 {user.id} 通过 Deep Link 验证并解禁（群组 {group_id}）")
                else:
                    # 用户未关注，显示关注链接
                    channel_link = f"https://t.me/{channel_id}" if str(channel_id).startswith('-100') else f"https://t.me/c/{str(channel_id)[4:]}"
                    
                    keyboard = InlineKeyboardMarkup([
                        [InlineKeyboardButton("📢 关注频道", url=channel_link)],
                        [InlineKeyboardButton("✅ 已关注，重新验证", callback_data=f"subscribe_{group_id}_{channel_id}")]
                    ])
                    
                    await update.message.reply_text(
                        f"""❌ 您还未关注频道

请先关注我们的频道，然后点击"已关注，重新验证"按钮。

📢 频道链接: {channel_link}""",
                        reply_markup=keyboard
                    )
    
    except Exception as e:
        logger.error(f"处理 /start 命令失败: {e}")
        await update.message.reply_text("处理失败，请稍后重试")
