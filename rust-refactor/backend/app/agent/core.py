# -*- coding: utf-8 -*-
"""
AI Agent 核心引擎 - 多商户原生支持
"""
import json
import logging
import re
from typing import Any, Dict, List, Callable
from dataclasses import dataclass, field
from enum import Enum

logger = logging.getLogger(__name__)

class AgentRole(str, Enum):
    ADMIN = "admin"
    USER = "user"
    ASSISTANT = "assistant"

class PermissionLevel(str, Enum):
    READ_ONLY = "read_only"
    SAFE_WRITE = "safe_write"
    DANGEROUS_WRITE = "dangerous"

@dataclass
class AgentContext:
    user_id: int
    merchant_id: str
    tenant_id: str
    channel: str = "web_ai_chat"
    role: AgentRole = AgentRole.ASSISTANT
    permission: PermissionLevel = PermissionLevel.SAFE_WRITE
    conversation_history: List[dict] = field(default_factory=list)
    metadata: Dict[str, Any] = field(default_factory=dict)

@dataclass
class AgentResponse:
    text: str
    tool_calls_made: List[str] = field(default_factory=list)
    tool_results: Dict[str, Any] = field(default_factory=dict)
    is_action_taken: bool = False

class AIAgent:
    MAX_ROUNDS = 5

    def __init__(self, llm_call_fn: Callable = None, tool_executor_fn: Callable = None):
        self._llm_call = llm_call_fn
        self._tool_executor = tool_executor_fn

    async def process_message(self, user_message: str, ctx: AgentContext, model: str = "") -> AgentResponse:
        """处理用户消息"""
        if not user_message:
            return AgentResponse(text="消息不能为空")

        messages = [
            {"role": "system", "content": self._build_prompt(ctx)},
            {"role": "user", "content": user_message}
        ]

        for _ in range(self.MAX_ROUNDS):
            try:
                text, _ = await self._call_llm(messages, model or "deepseek-chat")
            except Exception as e:
                return AgentResponse(text=f"[LLM错误] {str(e)}")

            # 检查是否需要工具调用
            if "__TOOL_CALL__" not in str(text):
                return AgentResponse(text=str(text))

            # 解析工具调用
            tools = self._parse_tool_calls(str(text))
            for tool in tools:
                if self._tool_executor:
                    try:
                        result = await self._tool_executor(tool.get("name"), tool.get("arguments", {}), ctx)
                        messages.append({"role": "system", "content": f"Tool result: {json.dumps(result)}"})
                    except Exception as e:
                        logger.error(f"Tool error: {e}")

        return AgentResponse(text="[超时]")

    def _build_prompt(self, ctx: AgentContext) -> str:
        return f"""你是TGBot Pro AI助手。

当前上下文:
- 商户ID: {ctx.merchant_id}
- 租户ID: {ctx.tenant_id}
- 权限: {ctx.permission.value}

可用工具:
- ad_detect: 检测广告 (text, context)
- ad_register: 登记广告 (content, score, group_id)
- ad_delete: 删除广告 (message_id, group_id)
- ad_beautify: 美化广告 (original_content, target_style)
- ad_push_channel: 推送到频道 (content, channel_id)
- group_ban: 踢人/禁言 (user_id, group_id, duration)
- translate: 翻译 (text, target_lang)
- system_query: 系统查询 (query_type)

注意: 只操作当前商户数据, 广告>=40分视为广告"""

    async def _call_llm(self, messages: list, model: str) -> tuple:
        if not self._llm_call:
            return "[Agent] LLM服务未连接", {}
        return await self._llm_call(messages, model)

    def _parse_tool_calls(self, text: str) -> List[dict]:
        """解析工具调用"""
        match = re.search(r'__TOOL_CALL__(.+?)__END_TOOL_CALL__', text, re.DOTALL)
        if match:
            try:
                return [json.loads(match.group(1))]
            except json.JSONDecodeError:
                pass
        return []

def get_agent(llm_call_fn=None, tool_executor_fn=None) -> AIAgent:
    return AIAgent(llm_call_fn=llm_call_fn, tool_executor_fn=tool_executor_fn)