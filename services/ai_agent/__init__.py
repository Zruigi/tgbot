# -*- coding: utf-8 -*-
"""
AI Agent 体系 — 内置 Skill 技能与安全约束
支持 Function Calling 调用 Web 系统功能（财务/广播/广告检测等）
"""

from .core import AIAgent, AgentContext, AgentResponse
from .safety import SafetyGuardrail
from .tools import (
    FinanceTool,
    BroadcastTool,
    ContentGeneratorTool,
    AdDetectorTool,
    SystemQueryTool,
)
from .prompts import (
    build_system_prompt,
    AGENT_SYSTEM_TEMPLATE,
    TOOL_INSTRUCTIONS,
)

__all__ = [
    'AIAgent',
    'AgentContext', 
    'AgentResponse',
    'SafetyGuardrail',
    'FinanceTool',
    'BroadcastTool',
    'ContentGeneratorTool',
    'AdDetectorTool',
    'SystemQueryTool',
    'build_system_prompt',
]
