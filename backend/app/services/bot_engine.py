# -*- coding: utf-8 -*-
"""
Bot 引擎服务 - Telegram Bot 操作封装
"""
import logging
from typing import Optional, List, Dict, Any
from telegram import Bot

logger = logging.getLogger(__name__)


class BotEngine:
    """Bot 操作封装"""
    
    def __init__(self, bot: Optional[Bot] = None):
        self.bot = bot
    
    def set_bot(self, bot: Bot) -> None:
        """设置 Bot 实例"""
        self.bot = bot
    
    async def send_message(
        self,
        chat_id: int,
        text: str,
        reply_to_message_id: Optional[int] = None,
        parse_mode: Optional[str] = None,
    ) -> bool:
        """发送文本消息"""
        if not self.bot:
            logger.error("Bot 实例未初始化")
            return False
        
        try:
            await self.bot.send_message(
                chat_id=chat_id,
                text=text,
                reply_to_message_id=reply_to_message_id,
                parse_mode=parse_mode,
            )
            return True
        except Exception as e:
            logger.error(f"发送消息失败：{e}")
            return False
    
    async def send_photo(
        self,
        chat_id: int,
        photo: Any,
        caption: Optional[str] = None,
        reply_to_message_id: Optional[int] = None,
    ) -> bool:
        """发送图片"""
        if not self.bot:
            logger.error("Bot 实例未初始化")
            return False
        
        try:
            await self.bot.send_photo(
                chat_id=chat_id,
                photo=photo,
                caption=caption,
                reply_to_message_id=reply_to_message_id,
            )
            return True
        except Exception as e:
            logger.error(f"发送图片失败：{e}")
            return False
    
    async def get_chat_members(self, chat_id: int) -> List[Dict[str, Any]]:
        """获取群成员列表"""
        if not self.bot:
            logger.error("Bot 实例未初始化")
            return []
        
        try:
            members = await self.bot.get_chat_members(chat_id)
            return [
                {
                    'user_id': m.user.id,
                    'username': m.user.username,
                    'first_name': m.user.first_name,
                    'last_name': m.user.last_name,
                    'is_bot': m.user.is_bot,
                    'status': getattr(m, 'status', 'member'),
                }
                for m in members
            ]
        except Exception as e:
            logger.error(f"获取群成员失败：{e}")
            return []
    
    async def get_chat_administrators(self, chat_id: int) -> List[Dict[str, Any]]:
        """获取群管理员列表"""
        if not self.bot:
            logger.error("Bot 实例未初始化")
            return []
        
        try:
            admins = await self.bot.get_chat_administrators(chat_id)
            return [
                {
                    'user_id': a.user.id,
                    'username': a.user.username,
                    'first_name': a.user.first_name,
                    'last_name': a.user.last_name,
                    'is_bot': a.user.is_bot,
                    'status': getattr(a, 'status', 'administrator'),
                }
                for a in admins
            ]
        except Exception as e:
            logger.error(f"获取管理员失败：{e}")
            return []
    
    async def kick_chat_member(self, chat_id: int, user_id: int) -> bool:
        """踢出群成员"""
        if not self.bot:
            logger.error("Bot 实例未初始化")
            return False
        
        try:
            await self.bot.kick_chat_member(chat_id=chat_id, user_id=user_id)
            return True
        except Exception as e:
            logger.error(f"踢出成员失败：{e}")
            return False
    
    async def ban_chat_member(self, chat_id: int, user_id: int) -> bool:
        """禁止群成员发言"""
        if not self.bot:
            logger.error("Bot 实例未初始化")
            return False
        
        try:
            await self.bot.ban_chat_member(chat_id=chat_id, user_id=user_id)
            return True
        except Exception as e:
            logger.error(f"禁止发言失败：{e}")
            return False


# 全局单例
_bot_engine = BotEngine()


def get_bot_engine() -> BotEngine:
    """获取 Bot 引擎"""
    return _bot_engine


def init_bot_engine(bot: Bot) -> None:
    """初始化 Bot 引擎"""
    _bot_engine.set_bot(bot)
