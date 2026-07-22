# -*- coding: utf-8 -*-
"""
AI Agent 类型定义与数据模型
"""

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Callable, Awaitable
from enum import Enum


class AgentRole(str, Enum):
    """Agent 角色类型"""
    ASSISTANT = "assistant"      # 通用助手
    FINANCE = "finance"          # 财务专家
    BROADCAST = "broadcast"      # 群发运营
    CONTENT = "content"          # 内容创作
    SECURITY = "security"        # 安全审查


class PermissionLevel(str, Enum):
    """操作权限级别"""
    READ_ONLY = "read_only"          # 只读查询
    SAFE_WRITE = "safe_write"        # 安全写入（可逆操作）
    DANGEROUS_WRITE = "dangerous"    # 危险写入（不可逆，需确认）


@dataclass
class ToolDefinition:
    """工具定义"""
    name: str                          # 工具名（Function Calling 用）
    description: str                   # 工具描述（LLM 用于判断是否调用）
    parameters: dict                   # JSON Schema 参数定义
    handler: Callable                  # 执行函数: async (params) -> dict
    permission: PermissionLevel       # 权限级别
    category: str                      # 分类（finance/broadcast/content/security/system）
    
    @property
    def schema_for_llm(self) -> dict:
        """转换为 LLM 可用的 function calling schema"""
        return {
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": self.parameters.get("properties", {}),
                    "required": self.parameters.get("required", []),
                }
            }
        }


@dataclass 
class AgentContext:
    """Agent 上下文（每次对话会话）"""
    user_id: int
    channel: str = "web_ai_chat"
    sandbox_id: str = "default"
    conversation_history: List[dict] = field(default_factory=list)
    active_role: AgentRole = AgentRole.ASSISTANT
    metadata: Dict[str, Any] = field(default_factory=dict)
    bot_instance: Any = None  # Bot 实例引用，用于工具执行


@dataclass
class AgentResponse:
    """Agent 响应结果"""
    text: str                           # 最终回复文本
    tool_calls_made: List[str] = field(default_factory=list)   # 本次调用的工具列表
    tool_results: Dict[str, Any] = field(default_factory=dict) # 工具执行结果
    is_action_taken: bool = False        # 是否执行了实际动作
    action_summary: str = ""            # 动作摘要
    requires_confirmation: bool = False  # 是否需要用户确认
    confidence: float = 1.0             # 置信度 0-1
    raw_response: Optional[dict] = None # 原始 LLM 响应
    ui_actions: List[Dict[str, Any]] = field(default_factory=list)
    job: Optional[Dict[str, Any]] = None
    
    def to_dict(self) -> dict:
        return {
            "text": self.text,
            "tool_calls_made": self.tool_calls_made,
            "tool_results": self.tool_results,
            "is_action_taken": self.is_action_taken,
            "action_summary": self.action_summary,
            "requires_confirmation": self.requires_confirmation,
            "confidence": self.confidence,
            "ui_actions": self.ui_actions,
            "job": self.job,
        }
