# -*- coding: utf-8 -*-
"""
提醒消息自动清理任务
"""

import asyncio
import logging
from datetime import datetime
import database

logger = logging.getLogger(__name__)

async def cleanup_expired_reminders(bot) -> None:
    """
    定时清理过期的提醒消息
    每小时执行一次
    """
    while True:
        try:
            await asyncio.sleep(3600)  # 每小时执行一次
            
            logger.info("开始清理过期的提醒消息...")
            
            # 获取过期的提醒记录
            expired_reminders = await database.get_expired_reminders()
            
            if not expired_reminders:
                logger.info("没有过期的提醒消息")
                continue
            
            logger.info(f"找到 {len(expired_reminders)} 条过期提醒消息")
            
            deleted_count = 0
            for reminder in expired_reminders:
                try:
                    user_id = int(reminder.get('user_id') or 0)
                    # 频道/匿名主体提醒不执行删除，直接标记已处理
                    if user_id <= 0:
                        await database.mark_reminder_deleted(reminder['id'])
                        logger.info(f"跳过非用户提醒记录 id={reminder['id']} user_id={user_id}")
                        continue

                    # 删除 Telegram 消息
                    await bot.delete_message(
                        chat_id=reminder['group_id'],
                        message_id=reminder['message_id']
                    )
                    
                    # 标记为已删除
                    await database.mark_reminder_deleted(reminder['id'])
                    
                    deleted_count += 1
                    logger.info(f"已删除提醒消息 {reminder['message_id']}（群组 {reminder['group_id']}）")
                
                except Exception as e:
                    logger.error(f"删除提醒消息失败: {e}")
                    # 继续处理下一条
                    continue
            
            logger.info(f"本次清理完成，共删除 {deleted_count} 条消息")
        
        except asyncio.CancelledError:
            logger.info("提醒消息清理任务已停止")
            break
        except Exception as e:
            logger.error(f"清理提醒消息时出错: {e}")
            # 继续运行，等待下一次执行


async def auto_unmute_on_channel_subscribe(bot, user_id: int, group_id: int) -> None:
    """
    用户关注频道后自动解禁
    
    Args:
        bot: Telegram bot 实例
        user_id: 用户 ID
        group_id: 群组 ID
    """
    try:
        # 获取用户的提醒记录
        reminder = await database.get_reminder_by_user_group(user_id, group_id)
        
        if not reminder:
            logger.warning(f"未找到用户 {user_id} 在群组 {group_id} 的提醒记录")
            return
        
        # 解禁用户
        await bot.restrict_chat_member(
            chat_id=group_id,
            user_id=user_id,
            permissions=None  # None 表示恢复所有权限
        )
        
        # 删除提醒消息
        try:
            await bot.delete_message(
                chat_id=group_id,
                message_id=reminder['message_id']
            )
        except Exception as e:
            logger.warning(f"删除提醒消息失败: {e}")
        
        # 标记提醒为已删除
        await database.mark_reminder_deleted(reminder['id'])
        
        logger.info(f"用户 {user_id} 已自动解禁（群组 {group_id}）")
    
    except Exception as e:
        logger.error(f"自动解禁失败: {e}")
