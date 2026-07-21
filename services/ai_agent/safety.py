# -*- coding: utf-8 -*-
"""
AI Agent 安全约束层 — 防止越权、注入、误操作
"""

import re
import logging
import time
from typing import Any, Dict, List, Optional, Tuple
from dataclasses import dataclass

logger = logging.getLogger(__name__)


# ===== 危险操作白名单（需二次确认）=====
DANGEROUS_ACTIONS = {
    # 广播相关
    "send_broadcast",            # 兼容旧命名
    "send_broadcast_with_image", # 兼容旧命名
    "broadcast_message",         # 当前工具名
    # 数据修改
    "delete_data",              # 删除数据
    "delete_ad_message",        # 删除广告群消息
    "delete_ad_cluster",        # 批量删除广告聚类消息
    "delete_recent_ad_messages", # 删除最近时间窗的广告快照
    "ban_ad_senders",           # 封禁广告发送者
    "reverse_ledger",           # 红字冲销
    "reconcile_ledger",         # 核销账目
    "skill_manage",             # Skill 配置写操作
    # 用户管理
    "ban_user",                 # 封禁用户
}

# ===== 完全禁止的操作 =====
FORBIDDEN_PATTERNS = [
    r"(?i)(?:exec|eval|__import__|os\.system|subprocess)",     # 代码执行
    r"(?i)(?:DROP\s+TABLE|DELETE\s+FROM|TRUNCATE)",             # SQL 注入
    r"(?i)(?:rm\s+-rf|chmod|chown)\b",                         # 系统命令
]

# ===== 敏感信息脱敏模式 =====
SENSITIVE_PATTERNS = {
    r"\b\d{15,19}\b": "[银行卡号]",
    r"[Tt][1-9A-HJ-NP-Za-km-z]{33}": "[TRON地址]",
    r"0x[0-9a-fA-F]{40}": "[ETH地址]",
    r"\d{4}[-\s]?\d{4}[-\s]?\d{4}\s?\d{3,5}": "[手机号]",


}

# ===== 速率限制 =====
RATE_LIMITS = {
    "default": {"calls_per_minute": 30, "calls_per_hour": 200},
    "dangerous": {"calls_per_minute": 2, "calls_per_hour": 10},
}


@dataclass
class SafetyCheckResult:
    """安全检查结果"""
    passed: bool
    reason: str = ""
    risk_level: str = "safe"  # safe / warning / danger / forbidden
    needs_confirmation: bool = False
    sanitized_params: Optional[dict] = None
    wait_seconds: float = 0.0  # 速率限制等待时间


class SafetyGuardrail:
    """
    AI Agent 安全护栏
    多层防护：
    1. 模式匹配检测危险指令
    2. 权限分级校验
    3. 速率限制
    4. 输出脱敏
    5. 操作审计日志
    """

    def __init__(self):
        self._call_history: List[Tuple[float, str]] = []  # [(timestamp, action_type)]
        
        # 编译正则
        self._forbidden_patterns = [re.compile(p) for p in FORBIDDEN_PATTERNS]
        self._sensitive_patterns = {re.compile(k): v for k, v in SENSITIVE_PATTERNS.items()}

    async def check_tool_call(
        self,
        tool_name: str,
        params: Dict[str, Any],
        context_user_id: int,
    ) -> SafetyCheckResult:
        """
        对工具调用进行安全检查
        """
        # 1. 检查输入参数中是否有危险模式
        params_str = str(params)
        for pattern in self._forbidden_patterns:
            if pattern.search(params_str):
                logger.warning(f"安全拦截: 工具 {tool_name} 包含禁止模式")
                return SafetyCheckResult(
                    passed=False,
                    reason="请求内容包含不允许的指令模式",
                    risk_level="forbidden",
                )

        # 2. 判断是否为危险操作（需要确认）
        is_dangerous = any(
            d in tool_name or d.replace("_", "") in tool_name.replace("_", "")
            for d in DANGEROUS_ACTIONS
        )

        if is_dangerous:
            return SafetyCheckResult(
                passed=True,
                risk_level="danger",
                needs_confirmation=True,
                sanitized_params=params,
                reason=f"'{tool_name}' 属于重要操作，需要您确认后执行",
            )

        # 3. 速率限制检查
        limit_config = RATE_LIMITS["dangerous"] if is_dangerous else RATE_LIMITS["default"]
        wait_time = self._check_rate_limit(tool_name, limit_config)
        if wait_time > 0:
            return SafetyCheckResult(
                passed=False,
                reason=f"操作过于频繁，请等待 {wait_time:.0f} 秒后再试",
                risk_level="warning",
                wait_seconds=wait_time,
            )

        # 4. 参数基本校验
        try:
            sanitized = self._sanitize_params(params, tool_name)
        except Exception as e:
            return SafetyCheckResult(
                passed=False,
                reason=f"参数格式校验失败: {e}",
                risk_level="warning",
            )

        return SafetyCheckResult(
            passed=True,
            risk_level="safe" if not is_dangerous else "danger",
            needs_confirmation=False,
            sanitized_params=sanitized,
        )

    async def sanitize_output(self, text: str) -> str:
        """
        对输出文本进行敏感信息脱敏
        """
        result = text
        for pattern, replacement in self._sensitive_patterns.items():
            result = pattern.sub(replacement, result)
        return result

    def _sanitize_params(self, params: dict, tool_name: str) -> dict:
        """
        清理参数中的潜在风险
        """
        cleaned = {}
        for key, value in params.items():
            if isinstance(value, str):
                # 截断过长字符串（防注入长 payload）
                if len(value) > 5000:
                    value = value[:5000] + "... [已截断]"
                # 去除控制字符
                value = re.sub(r'[\x00-\x08\x0b\x0c\x0e-\x1f]', '', value)
            cleaned[key] = value
        
        return cleaned

    def _check_rate_limit(self, action_type: str, limits: dict) -> float:
        """
        返回需要等待的秒数，0 表示不限速
        """
        now = time.time()
        
        # 清理过期记录
        cutoff_1min = now - 60
        cutoff_1hour = now - 3600
        self._call_history = [
            (t, a) for t, a in self._call_history if t > cutoff_1hour
        ]
        
        # 统计
        recent_1min = [t for t, a in self._call_history if t > cutoff_1min]
        
        if len(recent_1min) >= limits["calls_per_minute"]:
            wait = recent_1min[0] + 60 - now
            return max(0, wait)
            
        if len(self._call_history) >= limits["calls_per_hour"]:
            wait = self._call_history[0] + 3600 - now
            return max(0, wait)
            
        # 记录本次调用
        self._call_history.append((now, action_type))
        return 0.0

    def log_action(self, tool_name: str, params: dict, result: dict, user_id: int):
        """
        记录操作审计日志
        """
        logger.info(
            f"Audit | user={user_id} | tool={tool_name} | "
            f"status={result.get('status','?')} | "
            f"summary={str(params)[:100]}"
        )
