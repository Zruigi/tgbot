# -*- coding: utf-8 -*-
"""
AI 服务 - 对话、翻译、成本计算
"""
import logging
from typing import Optional, Dict, Any, List, Tuple
from datetime import datetime
from cachetools import TTLCache
import aiohttp
from aiohttp import ClientConnectorError

logger = logging.getLogger(__name__)


class AIService:
    """AI 对话服务"""
    
    def __init__(self, api_key: str, base_url: str):
        self.api_key = api_key
        self.base_url = base_url
        # AI 用量缓存：TTL 1小时，上限 500 条
        self._usage_cache: TTLCache = TTLCache(maxsize=500, ttl=3600)

    async def _resolve_runtime_config(self, requested_model: str = "") -> Dict[str, str]:
        """Resolve the active OpenAI-compatible LLM config.

        The web admin settings are the source of truth when present; env config is
        the fallback used by the development/demo runtime.
        """
        import config as bot_config

        async def _setting(key: str, default: str = "") -> str:
            try:
                import database
                value = await database.get_admin_setting(key, default)
                return str(value or default or "").strip()
            except Exception:
                return str(default or "").strip()

        async def _secret(key: str, default: str = "") -> str:
            try:
                from services.secret_manager import SecretManager
                value = await SecretManager.get_config(key, default=None)
                return str(value or "").strip()
            except Exception:
                return str(default or "").strip()

        lock_enabled = (await _setting("llm_model_lock_enabled", "false")).lower() in {"1", "true", "yes", "on"}
        locked_model = await _setting("llm_model_lock_target", "")
        configured_model = await _setting("llm_model", getattr(bot_config, "LLM_MODEL_NAME", ""))
        model = (locked_model if lock_enabled and locked_model else requested_model or configured_model or getattr(bot_config, "LLM_MODEL_NAME", "")).strip()
        if not model:
            model = "gemini-2.5-flash-lite"

        base_url = await _setting("llm_base_url", getattr(bot_config, "LLM_BASE_URL", ""))
        model_lower = model.lower()

        api_key = ""
        if "deepseek" in model_lower:
            api_key = (
                await _secret("DEEPSEEK_API_KEY")
                or await _setting("llm_api_key_deepseek", "")
                or getattr(bot_config, "DEEPSEEK_API_KEY", "")
                or await _secret("LLM_API_KEY")
                or await _setting("llm_api_key", "")
                or getattr(bot_config, "LLM_API_KEY", "")
            )
            if not base_url:
                base_url = "https://api.deepseek.com/v1"
        elif "gemini" in model_lower:
            api_key = (
                await _secret("GEMINI_API_KEY")
                or await _setting("llm_api_key_gemini", "")
                or getattr(bot_config, "GEMINI_API_KEY", "")
                or await _secret("LLM_API_KEY")
                or await _setting("llm_api_key", "")
                or getattr(bot_config, "LLM_API_KEY", "")
            )
        else:
            api_key = (
                await _secret("LLM_API_KEY")
                or await _setting("llm_api_key", "")
                or getattr(bot_config, "LLM_API_KEY", "")
                or self.api_key
            )

        if not base_url:
            base_url = self.base_url or "https://api.openai.com/v1"

        return {"api_key": str(api_key or "").strip(), "base_url": str(base_url or "").strip(), "model": model}

    @staticmethod
    def _chat_url(base_url: str) -> str:
        api_url = str(base_url or "").rstrip("/")
        if not api_url.endswith("/chat/completions"):
            api_url += "/chat/completions"
        return api_url

    @staticmethod
    def _is_local_gateway(base_url: str) -> bool:
        value = str(base_url or "").lower()
        return "localhost" in value or "127.0.0.1" in value or "::1" in value

    async def _direct_provider_fallback(self, runtime: Dict[str, str]) -> Optional[Dict[str, str]]:
        """Return a direct provider config when the local LiteLLM gateway is down."""
        import config as bot_config

        model = runtime.get("model") or ""
        lower = model.lower()
        if "gemini" in lower:
            api_key = str(getattr(bot_config, "GEMINI_API_KEY", "") or "").strip()
            if api_key:
                return {
                    "api_key": api_key,
                    "base_url": "https://generativelanguage.googleapis.com/v1beta/openai",
                    "model": model,
                }
        if "deepseek" in lower:
            api_key = str(getattr(bot_config, "DEEPSEEK_API_KEY", "") or "").strip()
            if api_key:
                return {
                    "api_key": api_key,
                    "base_url": "https://api.deepseek.com/v1",
                    "model": model,
                }
        default_llm = str(getattr(bot_config, "DEFAULT_LLM", "") or "").lower()
        if default_llm == "deepseek" and str(getattr(bot_config, "DEEPSEEK_API_KEY", "") or "").strip():
            return {
                "api_key": str(getattr(bot_config, "DEEPSEEK_API_KEY", "") or "").strip(),
                "base_url": "https://api.deepseek.com/v1",
                "model": "deepseek-chat",
            }
        return None

    async def chat_messages(
        self,
        messages: List[Dict[str, Any]],
        model: str = "",
        temperature: float = 0.3,
        timeout_seconds: float = 110.0,
    ) -> Tuple[str, Dict[str, Any]]:
        """Call the configured OpenAI-compatible LLM and return text + usage."""
        runtime = await self._resolve_runtime_config(model)
        api_key = runtime["api_key"]
        if not api_key:
            raise ValueError("Missing valid LLM API key")

        payload = {
            "model": runtime["model"],
            "messages": messages,
            "temperature": temperature,
            "max_tokens": 2048,
        }
        headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        }

        attempts = [runtime]
        if self._is_local_gateway(runtime["base_url"]):
            fallback = await self._direct_provider_fallback(runtime)
            if fallback:
                attempts.append(fallback)

        data = None
        last_error: Optional[Exception] = None
        timeout = aiohttp.ClientTimeout(total=timeout_seconds)
        for attempt in attempts:
            headers["Authorization"] = f"Bearer {attempt['api_key']}"
            payload["model"] = attempt["model"]
            logger.info("LLM chat request: model=%s base_url=%s", attempt["model"], attempt["base_url"])
            try:
                async with aiohttp.ClientSession(timeout=timeout) as session:
                    async with session.post(self._chat_url(attempt["base_url"]), headers=headers, json=payload) as response:
                        body = await response.text()
                        if response.status != 200:
                            raise RuntimeError(f"LLM API {response.status}: {body[:500]}")
                        data = await response.json()
                        break
            except (ClientConnectorError, OSError, RuntimeError) as exc:
                last_error = exc
                if attempt is attempts[-1]:
                    raise
                logger.warning("LLM gateway failed, retrying direct provider: %s", str(exc)[:200])

        if data is None:
            raise RuntimeError(f"LLM call failed: {last_error}")

        try:
            text = str(data["choices"][0]["message"].get("content") or "").strip()
        except (KeyError, IndexError, TypeError) as exc:
            raise RuntimeError("LLM response missing choices[0].message.content") from exc

        return text, dict(data.get("usage") or {})
    
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
        logger.info(f"AI 对话请求：user_id={user_id}, model={model}")
        text, usage = await self.chat_messages(
            messages=[
                {"role": "system", "content": "你是 Telegram 管理后台里的中文 AI 助手。直接回答用户问题，不要复读用户原文。"},
                {"role": "user", "content": message},
            ],
            model=model,
        )
        self.record_usage(user_id, int((usage or {}).get("total_tokens") or max(1, (len(message) + len(text)) // 4)), model)
        return text
    
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
