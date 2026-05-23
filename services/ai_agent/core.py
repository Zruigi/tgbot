# -*- coding: utf-8 -*-
"""
AI Agent 核心引擎 — 对话管理 + Function Calling 循环 + 安全约束集成
"""

import json
import logging
import asyncio
from typing import Any, Dict, List, Optional

from .schema import (
    AgentContext,
    AgentResponse,
    ToolDefinition,
    PermissionLevel,
    AgentRole,
)
from .safety import SafetyGuardrail, SafetyCheckResult
from .tools import get_tool, get_all_tools, get_tools_for_llm, TOOL_REGISTRY

logger = logging.getLogger(__name__)

# 最大循环次数（防无限递归）
MAX_TOOL_CALL_ROUNDS = 5


class AIAgent:
    """
    AI Agent 核心引擎
    
    功能：
    1. 维护对话上下文
    2. 解析 LLM 的 function calling 指令
    3. 安全检查后执行工具
    4. 将结果反馈给 LLM 形成最终回复
    """

    def __init__(self, llm_call_fn=None):
        """
        Args:
            llm_call_fn: LLM 调用函数，签名为:
                async fn(messages: list, model: str, tools: list) -> (text: str, tool_calls: list)
        """
        self._llm_call = llm_call_fn
        self.safety = SafetyGuardrail()
        self._tools_loaded = True

    @property
    def available_tools(self) -> List[str]:
        return [t.name for t in get_all_tools()]

    async def process_message(
        self,
        user_message: str,
        ctx: AgentContext,
        model: str = "",
    ) -> AgentResponse:
        """
        处理一条用户消息，返回 Agent 响应
        
        这是主入口函数：
        1. 构建含 system prompt + tool definitions 的 messages
        2. 调用 LLM
        3. 如果 LLM 返回 tool_calls → 安全检查 → 执行 → 结果回传 LLM → 直到无 tool_calls
        4. 返回最终回复
        """
        # 构建初始消息
        system_prompt = self._build_system_prompt(ctx)
        
        messages = [{"role": "system", "content": system_prompt}]
        messages.extend(ctx.conversation_history)
        messages.append({"role": "user", "content": user_message})

        # 获取工具 schemas
        tool_schemas = get_tools_for_llm()

        # Function Calling 循环
        round_num = 0
        tool_calls_made = []
        tool_results = {}
        action_taken = False
        needs_confirmation = False
        final_text = ""

        while round_num < MAX_TOOL_CALL_ROUNDS:
            round_num += 1
            
            try:
                # 调用 LLM
                answer, raw_tool_calls = await self._call_llm(
                    messages=messages,
                    model=model or "gemini-2.5-flash-lite",
                    tools=tool_schemas,
                    round_num=round_num,
                )

                # 如果没有 tool_calls，这就是最终回复
                if not raw_tool_calls:
                    final_text = answer
                    break

                # 处理 tool_calls
                messages.append({"role": "assistant", "content": answer or "", "tool_calls": []})

                all_results = []
                has_dangerous = False

                for tc in raw_tool_calls:
                    tool_name = tc.get("function", {}).get("name", "")
                    args_str = tc.get("function", {}).get("arguments", "{}")
                    
                    try:
                        args = json.loads(args_str) if isinstance(args_str, str) else args_str
                    except Exception:
                        args = {}

                    tool_instance = get_tool(tool_name)
                    if not tool_instance:
                        all_results.append({
                            "tool_name": tool_name,
                            "error": f"未知工具: {tool_name}",
                        })
                        continue

                    tool_calls_made.append(tool_name)

                    # 安全检查
                    safety_result: SafetyCheckResult = await self.safety.check_tool_call(
                        tool_name=tool_name,
                        params=args,
                        context_user_id=ctx.user_id,
                    )

                    if not safety_result.passed:
                        all_results.append({
                            "tool_name": tool_name,
                            "error": safety_result.reason,
                            "blocked": True,
                        })
                        continue

                    if safety_result.needs_confirmation:
                        # 危险操作先挂起，不在此轮执行
                        has_dangerous = True
                        needs_confirmation = True
                        exec_params = safety_result.sanitized_params or args
                        pending_payload = {
                            "needs_confirmation": True,
                            "_params": exec_params,
                            "message": "操作待确认",
                            "success": True,
                            "action_taken": False,
                        }
                        tool_results[tool_name] = pending_payload
                        all_results.append({
                            "tool_name": tool_name,
                            "result": pending_payload,
                            "success": True,
                            "blocked": False,
                            "needs_confirmation": True,
                        })
                        continue

                    # 执行工具
                    try:
                        exec_params = safety_result.sanitized_params or args
                        result = await tool_instance.execute(exec_params, context=ctx)

                        # ★ 关键修复：检查执行结果是否失败
                        # 如果 success=False，不走确认流程，直接记录错误
                        if not result.get("success", True):
                            all_results.append({
                                "tool_name": tool_name,
                                "success": False,
                                "error": result.get("message", "工具执行失败"),
                                "action_taken": False,
                            })
                            # 不设置 has_dangerous / needs_confirmation
                            # 执行失败的操作不需要用户确认
                            continue

                        if result.get("action_taken"):
                            action_taken = True
                        
                        # 如果是内容生成工具的特殊请求
                        if result.get("requires_llm_generation"):
                            gen_prompt = result["data"].get("generate_prompt", "")
                            if gen_prompt and self._llm_call:
                                generated_text, _ = await self._call_llm(
                                    messages=[{"role": "user", "content": gen_prompt}],
                                    model=model,
                                    tools=[],
                                )
                                result["generated_content"] = generated_text
                                result.pop("requires_llm_generation", None)

                        # ★ 只有执行成功且需要确认时才走确认流程
                        # 注意：needs_confirmation 来自 safety_result，表示安全检查认为需要确认
                        # 但只有工具执行成功才有意义让用户确认
                        tool_needs_confirm = bool(result.get("needs_confirmation")) and result.get("success", True)

                        all_results.append({
                            "tool_name": tool_name,
                            "result": result,
                            "success": True,
                            "blocked": False,
                            "needs_confirmation": tool_needs_confirm,
                        })

                        # 保存参数，便于确认后重放
                        stored_result = dict(result)
                        stored_result["_params"] = exec_params
                        stored_result["needs_confirmation"] = tool_needs_confirm
                        tool_results[tool_name] = stored_result

                        # 记录审计日志
                        self.safety.log_action(
                            tool_name=tool_name,
                            params=args,
                            result=result,
                            user_id=ctx.user_id,
                        )

                    except Exception as e:
                        logger.error(f"工具 {tool_name} 执行异常: {e}", exc_info=True)
                        all_results.append({
                            "tool_name": tool_name,
                            "error": str(e),
                            "success": False,
                        })

                # 将工具结果加入 messages，让 LLM 总结回复
                # ★ 降级模式：使用 user/system role 而非 tool role（避免 LLM 不识别）
                results_summary = self._format_tool_results(all_results)
                
                if has_dangerous:
                    messages.append({
                        "role": "user",
                        "content": f"[系统提示: 工具返回结果]\n{results_summary}\n\n请根据以上结果生成回复。如果有危险操作待确认，请提醒用户确认。",
                    })
                else:
                    messages.append({
                        "role": "user", 
                        "content": f"[工具执行结果]\n{results_summary}\n\n请基于以上结果简洁回复用户。",
                    })

                # 如果有危险操作需要确认，提前退出让用户确认
                if has_dangerous:
                    final_text = results_summary + "\n\n⚠️ 以上操作需要您确认后才能执行。请回复「确认」继续，或取消操作。"
                    break

            except Exception as e:
                logger.error(f"AIAgent process_message 第{round_num}轮异常: {e}", exc_info=True)
                final_text = f"抱歉，处理过程中出现了问题：{str(e)[:200]}"
                break

        if not final_text:
            final_text = "我已收到您的请求，但未能生成合适的回复。"

        # 输出脱敏
        final_text = await self.safety.sanitize_output(final_text)

        # 更新对话历史
        ctx.conversation_history.append({"role": "user", "content": user_message})
        ctx.conversation_history.append({"role": "assistant", "content": final_text})

        # 保持历史在合理范围内
        max_history = 40
        if len(ctx.conversation_history) > max_history:
            ctx.conversation_history = ctx.conversation_history[-max_history:]

        return AgentResponse(
            text=final_text,
            tool_calls_made=tool_calls_made,
            tool_results=tool_results,
            is_action_taken=action_taken,
            requires_confirmation=needs_confirmation,
            action_summary=", ".join(tool_calls_made) if tool_calls_made else "",
            confidence=0.9 if not needs_confirmation else 0.6,
        )

    async def confirm_and_execute(
        self,
        pending_response: AgentResponse,
        ctx: AgentContext,
    ) -> AgentResponse:
        """
        用户确认后执行之前被暂停的危险操作
        """
        if not pending_response.requires_confirmation:
            return pending_response

        # 重新执行被暂停的操作（跳过安全确认步骤）
        executed_results = {}
        action_taken = False
        has_failure = False
        failure_messages = []

        for tool_name, prev_result in pending_response.tool_results.items():
            if prev_result.get("needs_confirmation"):
                tool_instance = get_tool(tool_name)
                if tool_instance:
                    try:
                        # 从 metadata 中恢复参数
                        result = await tool_instance.execute(prev_result.get("_params", {}), context=ctx)
                        executed_results[tool_name] = result
                        
                        # ★ 关键修复：检查执行结果
                        if not result.get("success", True):
                            has_failure = True
                            failure_messages.append(f"{tool_name}: {result.get('message', '执行失败')}")
                        elif result.get("action_taken"):
                            action_taken = True
                    except Exception as e:
                        executed_results[tool_name] = {"error": str(e), "success": False}
                        has_failure = True
                        failure_messages.append(f"{tool_name}: {str(e)}")

        # ★ 根据执行结果生成不同的回复文本
        if has_failure:
            result_text = f"⚠️ 操作执行完成，但有部分失败:\n" + "\n".join(f"- {msg}" for msg in failure_messages)
        else:
            result_text = "✅ 操作已执行完毕。"

        return AgentResponse(
            text=f"{result_text}\n{pending_response.text}",
            tool_calls_made=pending_response.tool_calls_made,
            tool_results=executed_results,
            is_action_taken=action_taken,
            requires_confirmation=False,
            action_summary=pending_response.action_summary,
            confidence=1.0 if not has_failure else 0.7,
        )

    def _match_tool_from_text(self, text: str, tools: list) -> Optional[str]:
        """根据用户文本做启发式工具匹配（仅作为 LLM 未显式返回 tool_call 时的兜底）"""
        if not text:
            return None

        normalized = text.lower()
        keyword_map = {
            "finance_query": [
                "查财务", "财务摘要", "财务查询", "财务报表", "财务分析",
                "查账目", "查流水", "看账目", "看流水",
                "未核销", "待核销", "核销流水",
                "对账单", "对个账",
            ],
            "broadcast_message": [
                "群发消息", "群发通知", "发送广播", "给所有人发",
                "群发公告", "批量发送",
            ],
            "detect_advertising": [
                "检测广告", "是不是广告", "广告检测", "垃圾信息检测",
                "是否广告", "鉴别广告",
            ],
            "generate_content": [
                "帮我写", "生成文案", "写一段", "生成内容",
                "帮我生成", "写个文案", "写公告",
            ],
            "system_query": [
                "系统状态", "系统概览", "有多少群", "群组统计",
                "系统信息", "活跃群组",
                "最近消息", "最近有谁", "谁给我发", "谁给我发送", "最近发消息",
                "广告消息", "群广告", "疑似广告",
            ],
            "skill_manage": [
                "新增skill", "增加skill", "添加skill", "技能管理",
                "列出技能", "查看技能", "更新技能", "导入技能",
                "新增技能", "添加技能",
            ],
        }

        best_match = None
        best_score = 0
        for tool in tools:
            fn = tool.get("function", {})
            name = fn.get("name", "")
            desc = (fn.get("description", "") or "").lower()
            score = 0

            for keyword in keyword_map.get(name, []):
                if keyword in normalized:
                    score += 8

            if name and name in normalized:
                score += 6

            if score == 0:
                for token in normalized.split():
                    if token and token in desc:
                        score += 1

            if score > best_score:
                best_score = score
                best_match = name

        if best_match == "system_query" and best_score >= 8:
            return best_match
        return best_match if best_score >= 16 else None

    def _build_fallback_args(self, tool_name: str, user_text: str) -> dict:
        """当模型未返回参数时，基于用户原文构造一组可执行参数。"""
        text = (user_text or "").strip()
        if not text:
            return {}

        if tool_name == "broadcast_message":
            content = text
            for kw in ("群发消息", "群发通知", "发送广播", "给所有人发", "群发公告", "批量发送", "测试"):
                content = content.replace(kw, "")
            content = content.strip(" ：:，,。.!！？?") or text
            return {"content": content, "target_type": "users"}

        if tool_name == "finance_query":
            if any(k in text for k in ("未核销", "待核销")):
                return {"action": "unreconciled"}
            if any(k in text for k in ("科目", "分类")):
                return {"action": "categories"}
            if any(k in text.lower() for k in ("trc20", "bsc")) and "测试" in text:
                return {"action": "onchain_api_test"}
            return {"action": "analysis"}

        if tool_name == "detect_advertising":
            return {"text": text, "context_type": "private_message"}

        if tool_name == "generate_content":
            return {"topic": text, "style": "professional", "length": "medium", "purpose": "announcement"}

        if tool_name == "system_query":
            lower = text.lower()
            if any(k in text for k in ("最近有谁", "谁给我发", "谁给我发送", "最近发消息", "谁发了消息")):
                return {"query_type": "recent_senders", "limit": 10}
            if "最近消息" in text:
                return {"query_type": "recent_messages", "limit": 10}
            if any(k in text for k in ("广告消息", "群广告", "疑似广告", "广告群消息")):
                return {"query_type": "group_ad_messages", "limit": 20}
            if "潜在用户" in text:
                return {"query_type": "potential_users", "limit": 10}
            if any(k in text for k in ("监听", "地址列表", "监控地址")):
                return {"query_type": "monitors_list", "limit": 10}
            if any(k in text for k in ("群", "群组")):
                return {"query_type": "active_groups", "limit": 10}
            if "统计" in text or "概览" in text or "状态" in text:
                return {"query_type": "stats_overview"}
            return {"query_type": "stats_overview"}

        if tool_name == "skill_manage":
            lower = text.lower()
            if "导入" in text:
                return {"action": "import_builtin"}
            if any(k in lower for k in ("新增", "添加", "增加")) and "skill" in lower:
                return {"action": "add"}
            if "启用" in text:
                return {"action": "enable"}
            if any(k in text for k in ("停用", "禁用")):
                return {"action": "disable"}
            if any(k in text for k in ("更新", "修改")):
                return {"action": "update"}
            return {"action": "list"}

        return {}


    async def _call_llm(
        self,
        messages: list,
        model: str,
        tools: list = None,
        round_num: int = 1,
    ) -> tuple:
        """
        调用 LLM 并解析响应
        返回: (answer_text, tool_calls_list)

        注意: 当前 LLM 调用链路不支持原生 function calling。
        工具调用通过文本关键词匹配实现（降级模式）。
        """
        if not self._llm_call:
            return "[Agent] LLM 服务未连接，无法生成回复", []

        try:
            # 标准调用（不传 tools——当前 LLM 不支持 function calling）
            text, usage = await self._llm_call(
                messages=messages,
                model=model or "gemini-2.5-flash-lite",
            )

            # 尝试从返回文本中解析 function calling（显式标记模式）
            tool_calls = []
            if isinstance(text, str) and text.startswith("__TOOL_CALL__"):
                try:
                    tool_data = json.loads(text[len("__TOOL_CALL__"):])
                    tool_calls = tool_data if isinstance(tool_data, list) else [tool_data]
                except Exception as e:
                    logger.warning(f"Tool call JSON 解析失败，当作纯文本: {e}")

            # 兜底：当模型没按约定返回 tool_call 时，用“最新用户消息”做启发式匹配，
            # 避免被模型自述能力文本误导到 finance_query。
            if tools and not tool_calls and round_num == 1:
                latest_user_text = ""
                for message in reversed(messages):
                    if message.get("role") == "user":
                        latest_user_text = str(message.get("content", "") or "")
                        break

                # 只在消息长度足够时才进行启发式匹配，过短的消息（如"你好"）跳过
                matched_tool_name = None
                if len(latest_user_text.strip()) >= 4:
                    matched_tool_name = self._match_tool_from_text(latest_user_text, tools)
                if matched_tool_name:
                    fallback_args = self._build_fallback_args(matched_tool_name, latest_user_text)
                    tool_calls = [{
                        "function": {
                            "name": matched_tool_name,
                            "arguments": json.dumps(fallback_args, ensure_ascii=False),
                        }
                    }]

            return text, tool_calls

        except Exception as e:
            logger.error(f"LLM 调用异常: {e}")
            import traceback
            logger.error(traceback.format_exc())
            return f"[LLM 错误] {str(e)[:200]}", []

    def _build_system_prompt(self, ctx: AgentContext) -> str:
        """构建系统提示词"""
        from .prompts import build_system_prompt
        return build_system_prompt(ctx)

    def _format_tool_results(self, all_results: List[dict]) -> str:
        """将工具结果格式化为 LLM 可读的总结"""
        parts = []
        for r in all_results:
            name = r["tool_name"]
            if r.get("blocked"):
                parts.append(f"- [{name}] ❌ 已拦截: {r.get('error', '安全原因')}")
            elif r.get("error"):
                # ★ 改进：区分执行失败和执行出错
                if r.get("success") is False:
                    parts.append(f"- [{name}] ❌ 执行失败: {r.get('error')}")
                else:
                    parts.append(f"- [{name}] ⚠️ 执行出错: {r.get('error')}")
            elif r.get("success") is False:
                # ★ 新增：处理 success=False 但没有 error 字段的情况
                parts.append(f"- [{name}] ❌ 执行失败")
            else:
                result = r.get("result", {})
                msg = result.get("message", "完成")
                data_preview = str(result.get("data", ""))[:200]
                conf_note = " ⚠️ 需确认" if r.get("needs_confirmation") else ""
                parts.append(f"- [{name}] ✅ {msg}{conf_note}\n  数据: {data_preview}")
        
        return "工具执行结果:\n" + "\n".join(parts)


# ===== 全局实例 =====
_global_agent: Optional[AIAgent] = None

def get_agent(llm_call_fn=None) -> AIAgent:
    """获取全局 Agent 实例（单例）"""
    global _global_agent
    if _global_agent is None:
        _global_agent = AIAgent(llm_call_fn=llm_call_fn)
    elif llm_call_fn:
        _global_agent._llm_call = llm_call_fn
    return _global_agent
