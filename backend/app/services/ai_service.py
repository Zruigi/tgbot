# -*- coding: utf-8 -*-
"""
AI 服务 - 对话、翻译、成本计算
"""
import logging
from typing import Optional, Dict, Any
from datetime import datetime
from cachetools import TTLCache

logger = logging.getLogger(__name__)


class AIService:
    """AI 对话服务"""
    
    def __init__(self, api_key: str, base_url: str):
        self.api_key = api_key
        self.base_url = base_url
        # AI 用量缓存：TTL 1小时，上限 500 条
        self._usage_cache: TTLCache = TTLCache(maxsize=500, ttl=3600)
    
    async def chat(
        self,
        user_id: int,
        message: str,
        model: str = 'gpt-3.5-turbo'
    ) -> str:
        """
        AI 对话
        
        Args:
            user_id: 用户 ID
            message: 用户消息
            model: AI 模型
        
        Returns:
            AI 回复的文本
        """
        # TODO: 实现实际的 OpenAI API 调用
        logger.info(f"AI 对话请求：user_id={user_id}, model={model}")
        
        # 模拟响应
        return f"[AI] 收到您的消息：{message}"
    
    async def translate(
        self,
        text: str,
        target_lang: str = 'zh'
    ) -> str:
        """
        翻译文本
        
        Args:
            text: 待翻译文本
            target_lang: 目标语言
        
        Returns:
            翻译后的文本
        """
        logger.info(f"翻译请求：target_lang={target_lang}")
        
        # TODO: 实现实际的翻译 API 调用
        return f"[翻译] {text}"
    
    def calculate_cost(self, tokens: int, model: str) -> float:
        """
        计算 API 调用成本
        
        Args:
            tokens: 使用的 token 数
            model: 模型名称
        
        Returns:
            成本（美元）
        """
        # OpenAI 定价（每 1000 tokens）
        pricing = {
            'gpt-3.5-turbo': 0.002,
            'gpt-4': 0.03,
            'gpt-4-32k': 0.06,
        }
        
        price_per_1k = pricing.get(model, 0.002)
        return (tokens / 1000) * price_per_1k
    
    def record_usage(self, user_id: int, tokens: int, model: str) -> None:
        """记录 AI 使用情况"""
        if user_id not in self._usage_cache:
            self._usage_cache[user_id] = {
                'total_tokens': 0,
                'total_cost': 0.0,
                'last_used': None
            }
        
        self._usage_cache[user_id]['total_tokens'] += tokens
        self._usage_cache[user_id]['total_cost'] += self.calculate_cost(tokens, model)
        self._usage_cache[user_id]['last_used'] = datetime.now()
    
    def get_usage(self, user_id: int) -> Dict[str, Any]:
        """获取用户使用情况"""
        return self._usage_cache.get(user_id, {
            'total_tokens': 0,
            'total_cost': 0.0,
            'last_used': None
        })


# 全局实例（延迟初始化）
_ai_service: Optional[AIService] = None


def get_ai_service() -> AIService:
    """获取 AI 服务实例"""
    global _ai_service
    if _ai_service is None:
        from backend.app.core.config import config
        _ai_service = AIService(
            api_key=config.OPENAI_API_KEY,
            base_url=config.OPENAI_BASE_URL
        )
    return _ai_service
