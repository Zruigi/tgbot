# -*- coding: utf-8 -*-
"""
群组禁言提示消息辅助函数
用于在用户被禁言时发送包含解禁按钮的提示消息
"""
import logging
from telegram import InlineKeyboardButton, InlineKeyboardMarkup
from telegram.error import TelegramError
import database
import aiosqlite

logger = logging.getLogger(__name__)


async def send_mute_prompt(
    bot,
    group_id: int,
    user_id: int,
    display_name: str,
    bot_username: str,
    reason: str = "未验证或触发规则"
) -> int | None:
    """
    发送禁言提示消息，包含解禁按钮
    
    Args:
        bot: Telegram Bot 实例
        group_id: 群组 ID
        user_id: 被禁言用户 ID
        display_name: 用户显示名称
        bot_username: Bot 的 @username（用于生成 Deep Link）
        reason: 禁言原因
    
    Returns:
        消息 ID（用于后续删除），失败返回 None
    """
    try:
        mention = f'<a href="tg://user?id={user_id}">{display_name}</a>'
        
        # 生成 Deep Link：https://t.me/{bot_username}?start=unmute_{group_id}
        deep_link = f"https://t.me/{bot_username}?start=unmute_{group_id}"
        
        keyboard = InlineKeyboardMarkup([
            [InlineKeyboardButton("🔓 立即申请解禁", url=deep_link)]
        ])
        
        text = (
            f"⚠️ {mention} 你因{reason}被禁言。\n\n"
            f"点击下方按钮关注频道并完成验证，即可恢复发言权限。"
        )
        
        msg = await bot.send_message(
            chat_id=group_id,
            text=text,
            reply_markup=keyboard,
            parse_mode="HTML"
        )
        
        # 保存消息 ID 到数据库
        try:
            async with aiosqlite.connect(database.DB_NAME) as db:
                await db.execute(
                    "UPDATE group_members SET pending_verify_msg_id = ? WHERE group_id = ? AND user_id = ?",
                    (msg.message_id, group_id, user_id)
                )
                await db.commit()
            logger.info(f"[禁言提示] 已保存消息 ID msg_id={msg.message_id} user={user_id} group={group_id}")
        except Exception as e:
            logger.warning(f"[禁言提示] 保存消息 ID 失败: {e}")
        
        return msg.message_id
        
    except TelegramError as e:
        logger.error(f"[禁言提示] 发送消息失败 user={user_id} group={group_id}: {e}")
        return None
    except Exception as e:
        logger.error(f"[禁言提示] 未知错误: {e}")
        return None


async def delete_mute_prompt(bot, group_id: int, user_id: int) -> bool:
    """
    删除用户的禁言提示消息
    
    Args:
        bot: Telegram Bot 实例
        group_id: 群组 ID
        user_id: 用户 ID
    
    Returns:
        是否成功删除
    """
    try:
        async with aiosqlite.connect(database.DB_NAME) as db:
            async with db.execute(
                "SELECT pending_verify_msg_id FROM group_members WHERE group_id = ? AND user_id = ?",
                (group_id, user_id)
            ) as cursor:
                row = await cursor.fetchone()
                msg_id = row[0] if row and row[0] else None
        
        if not msg_id:
            logger.debug(f"[删除提示] 未找到消息 ID user={user_id} group={group_id}")
            return False
        
        try:
            await bot.delete_message(chat_id=group_id, message_id=msg_id)
            logger.info(f"[删除提示] 已删除消息 msg_id={msg_id} user={user_id} group={group_id}")
            
            # 清除数据库记录
            async with aiosqlite.connect(database.DB_NAME) as db:
                await db.execute(
                    "UPDATE group_members SET pending_verify_msg_id = NULL WHERE group_id = ? AND user_id = ?",
                    (group_id, user_id)
                )
                await db.commit()
            
            return True
        except TelegramError as e:
            logger.debug(f"[删除提示] 消息已被删除或不存在 msg_id={msg_id}: {e}")
            return False
            
    except Exception as e:
        logger.warning(f"[删除提示] 查询失败: {e}")
        return False
