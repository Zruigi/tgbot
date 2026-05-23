# -*- coding: utf-8 -*-
"""
广播投递服务 - 消息发送、速率限制
"""
import asyncio
import logging
from typing import List, Dict, Any, Optional
from datetime import datetime

logger = logging.getLogger(__name__)


class BroadcastService:
    """广播投递服务"""
    
    def __init__(self, bot_instance=None):
        self.bot = bot_instance
        self._rate_limit_delay = 0.1  # 每条消息间隔 100ms
    
    async def send_to_users(
        self,
        user_ids: List[int],
        message: str,
        batch_size: int = 10
    ) -> Dict[str, Any]:
        """
        批量发送消息给用户
        
        Args:
            user_ids: 用户 ID 列表
            message: 消息内容
            batch_size: 批次大小
        
        Returns:
            发送结果统计
        """
        total = len(user_ids)
        success = 0
        failed = 0
        errors = []
        
        logger.info(f"开始广播投递：total={total}, batch_size={batch_size}")
        
        for i in range(0, total, batch_size):
            batch = user_ids[i:i + batch_size]
            
            for user_id in batch:
                try:
                    await self._send_message(user_id, message)
                    success += 1
                    
                    # 速率限制
                    await asyncio.sleep(self._rate_limit_delay)
                    
                except Exception as e:
                    failed += 1
                    errors.append({
                        'user_id': user_id,
                        'error': str(e)
                    })
                    logger.error(f"广播投递失败：user_id={user_id}, error={e}")
            
            # 批次间暂停
            if i + batch_size < total:
                await asyncio.sleep(1)
        
        result = {
            'total': total,
            'success': success,
            'failed': failed,
            'errors': errors[:10],  # 只保留前 10 个错误
            'completed_at': datetime.now().isoformat()
        }
        
        logger.info(f"广播投递完成：success={success}, failed={failed}")
        return result
    
    async def _send_message(self, user_id: int, message: str) -> bool:
        """发送单条消息"""
        if self.bot is None:
            raise RuntimeError("Bot instance not initialized")
        
        await self.bot.send_message(chat_id=user_id, text=message)
        return True
    
    async def send_with_retry(
        self,
        user_id: int,
        message: str,
        max_retries: int = 3
    ) -> bool:
        """发送消息（带重试）"""
        for attempt in range(max_retries):
            try:
                await self._send_message(user_id, message)
                return True
            except Exception as e:
                logger.warning(f"发送失败 (尝试 {attempt + 1}/{max_retries}): {e}")
                if attempt < max_retries - 1:
                    await asyncio.sleep(2 ** attempt)  # 指数退避
        
        return False


# 全局实例
_broadcast_service: Optional[BroadcastService] = None


def get_broadcast_service(bot_instance=None) -> BroadcastService:
    """获取广播服务实例"""
    global _broadcast_service
    if _broadcast_service is None:
        _broadcast_service = BroadcastService(bot_instance)
    elif bot_instance is not None:
        _broadcast_service.bot = bot_instance
    return _broadcast_service
