# -*- coding: utf-8 -*-
"""
AI Agent 核心引擎 — 对话管理 + Function Calling 循环 + 安全约束集成
"""

import json
import logging
import asyncio
import re
from typing import Any, Dict, List, Optional

import config

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
        routed_user_message = self._resolve_contextual_command(user_message, ctx)
        
        messages = [{"role": "system", "content": system_prompt}]
        messages.extend(ctx.conversation_history)
        # 获取工具 schemas
        tool_schemas = get_tools_for_llm()
        from .prompts import build_tool_use_prompt
        messages.append({
            "role": "user",
            "content": build_tool_use_prompt(routed_user_message[:2000], tool_schemas),
        })

        # Function Calling 循环
        round_num = 0
        tool_calls_made = []
        tool_results = {}
        action_taken = False
        needs_confirmation = False
        final_text = ""

        # 删除最近广告是高风险操作：用 Python 确定性生成候选快照，
        # 不让 LLM 自行构造 message_id 或扩大删除范围。
        prepared_delete = await self._prepare_recent_ad_deletion(routed_user_message, ctx)
        if prepared_delete is not None:
            ctx.conversation_history.append({"role": "user", "content": user_message})
            ctx.conversation_history.append({"role": "assistant", "content": prepared_delete.text})
            if len(ctx.conversation_history) > 40:
                ctx.conversation_history = ctx.conversation_history[-40:]
            return prepared_delete

        preflight_tool_calls = self._build_preflight_tool_calls(routed_user_message, tool_schemas)
        if preflight_tool_calls:
            all_results = []
            for tc in preflight_tool_calls:
                tool_name = tc.get("function", {}).get("name", "")
                args_str = tc.get("function", {}).get("arguments", "{}")
                try:
                    args = json.loads(args_str) if isinstance(args_str, str) else args_str
                except Exception:
                    args = {}

                tool_instance = get_tool(tool_name)
                if not tool_instance:
                    error_result = {"success": False, "message": f"未知工具: {tool_name}"}
                    tool_results[tool_name or "unknown_tool"] = error_result
                    all_results.append({"tool_name": tool_name, "error": error_result["message"], "success": False})
                    continue

                tool_calls_made.append(tool_name)
                safety_result: SafetyCheckResult = await self.safety.check_tool_call(
                    tool_name=tool_name,
                    params=args,
                    context_user_id=ctx.user_id,
                )
                if not safety_result.passed or safety_result.needs_confirmation:
                    error_result = {
                        "success": False,
                        "message": safety_result.reason or "该工具不能预执行",
                        "blocked": True,
                    }
                    tool_results[tool_name] = error_result
                    all_results.append({
                        "tool_name": tool_name,
                        "error": error_result["message"],
                        "blocked": True,
                        "success": False,
                    })
                    continue

                try:
                    exec_params = safety_result.sanitized_params or args
                    result = await tool_instance.execute(exec_params, context=ctx)
                    if result.get("action_taken"):
                        action_taken = True
                    stored_result = dict(result)
                    stored_result["_params"] = exec_params
                    tool_results[tool_name] = stored_result
                    all_results.append({
                        "tool_name": tool_name,
                        "result": result,
                        "success": bool(result.get("success", True)),
                        "blocked": False,
                    })
                    self.safety.log_action(
                        tool_name=tool_name,
                        params=args,
                        result=result,
                        user_id=ctx.user_id,
                    )
                except Exception as e:
                    logger.error(f"预执行工具 {tool_name} 异常: {e}", exc_info=True)
                    tool_results[tool_name] = {"success": False, "message": str(e)}
                    all_results.append({"tool_name": tool_name, "error": str(e), "success": False})

            results_summary = self._format_tool_results(all_results)
            messages.append({
                "role": "user",
                "content": (
                    f"[工具执行结果]\n{results_summary}\n\n"
                    "请基于以上真实工具结果直接回答用户，不要说你还需要查询。"
                    "如果证据状态里 analysis_allowed=false、data_complete=false 或 evidence_count=0，"
                    "必须明确说明当前没有足够消息证据，不能回答“未发现广告/没有异常”。"
                ),
            })
            if "query_ad_clusters" in tool_results:
                final_text = self._format_ad_cluster_result(tool_results["query_ad_clusters"])
            elif "query_recent_ad_messages" in tool_results:
                final_text = self._format_recent_ad_result(tool_results["query_recent_ad_messages"])
            elif (
                "system_query" in tool_results
                and "telegram_sdk_query" in tool_results
                and ((tool_results["system_query"].get("_params") or {}).get("query_type") == "bound_chats")
            ):
                final_text = self._format_verified_bound_chats(
                    tool_results["system_query"], tool_results["telegram_sdk_query"]
                )
            elif "system_query" in tool_results:
                final_text = self._format_system_query_result(tool_results["system_query"])
            elif "telegram_sdk_query" in tool_results:
                final_text = self._format_telegram_sdk_result(tool_results["telegram_sdk_query"])
            elif "finance_query" in tool_results:
                final_text = self._format_finance_result(tool_results["finance_query"])
            elif "detect_advertising" in tool_results:
                final_text = self._format_ad_detection_result(tool_results["detect_advertising"])
            else:
                final_text, _ = await self._call_llm(
                    messages=messages,
                    model=model or "gemini-2.5-flash-lite",
                    tools=[],
                    round_num=2,
                )

        while round_num < MAX_TOOL_CALL_ROUNDS:
            if final_text:
                break
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
                # OpenAI-compatible gateways reject an empty tool_calls array.
                # We use user-role tool result summaries below, so no native
                # tool call metadata is needed here.
                messages.append({"role": "assistant", "content": answer or ""})

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
                        tool_results[tool_name or "unknown_tool"] = {
                            "success": False,
                            "message": f"未知工具: {tool_name}",
                        }
                        all_results.append({
                            "tool_name": tool_name,
                            "error": f"未知工具: {tool_name}",
                            "success": False,
                        })
                        continue

                    tool_calls_made.append(tool_name)

                    # Recent-ad deletion must only come from the deterministic
                    # query snapshot prepared above. LLM-generated refs, including
                    # an empty argument object, are never accepted for confirmation.
                    if tool_name == "delete_recent_ad_messages":
                        blocked_message = "必须先由后端查询并锁定精确广告快照，当前删除请求已拒绝"
                        tool_results[tool_name] = {
                            "success": False,
                            "message": blocked_message,
                            "blocked": True,
                        }
                        all_results.append({
                            "tool_name": tool_name,
                            "error": blocked_message,
                            "blocked": True,
                            "success": False,
                        })
                        continue

                    # 安全检查
                    safety_result: SafetyCheckResult = await self.safety.check_tool_call(
                        tool_name=tool_name,
                        params=args,
                        context_user_id=ctx.user_id,
                    )

                    if not safety_result.passed:
                        tool_results[tool_name] = {
                            "success": False,
                            "message": safety_result.reason,
                            "blocked": True,
                        }
                        all_results.append({
                            "tool_name": tool_name,
                            "error": safety_result.reason,
                            "blocked": True,
                            "success": False,
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
                            failure_result = dict(result)
                            failure_result["_params"] = exec_params
                            failure_result["success"] = False
                            tool_results[tool_name] = failure_result
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
                        tool_results[tool_name] = {
                            "success": False,
                            "message": str(e),
                        }
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
                        "content": (
                            f"[工具执行结果]\n{results_summary}\n\n"
                            "请基于以上结果简洁回复用户。"
                            "如果证据状态显示没有消息证据或不允许分析，必须说明数据不可用，不能下安全结论。"
                        ),
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
            if tool_calls_made:
                final_text = "工具调用已达到上限或未返回最终结论，请缩小查询范围后重试。"
            else:
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
                        if result.get("action_taken"):
                            action_taken = True

                        # ★ 关键修复：检查执行结果
                        if not result.get("success", True):
                            has_failure = True
                            failure_messages.append(f"{tool_name}: {result.get('message', '执行失败')}")
                    except Exception as e:
                        executed_results[tool_name] = {"error": str(e), "success": False}
                        has_failure = True
                        failure_messages.append(f"{tool_name}: {str(e)}")

        # 确认后的回复只描述本次执行结果，避免再次拼接旧的“请确认”提示。
        result_messages = [
            str(result.get("message") or "").strip()
            for result in executed_results.values()
            if isinstance(result, dict) and str(result.get("message") or "").strip()
        ]
        if has_failure and action_taken:
            result_text = "操作已部分执行，但有失败项：\n" + "\n".join(f"- {msg}" for msg in failure_messages)
        elif has_failure:
            result_text = "操作未执行：\n" + "\n".join(f"- {msg}" for msg in failure_messages)
        else:
            result_text = "操作已执行完毕。"
            if result_messages:
                result_text += "\n" + "\n".join(f"- {msg}" for msg in result_messages)

        return AgentResponse(
            text=result_text,
            tool_calls_made=pending_response.tool_calls_made,
            tool_results=executed_results,
            is_action_taken=action_taken,
            requires_confirmation=False,
            action_summary=pending_response.action_summary,
            confidence=1.0 if not has_failure else 0.7,
        )

    @staticmethod
    def _is_recent_ad_delete_request(text: str) -> bool:
        normalized = re.sub(r"\s+", "", str(text or "").lower())
        has_delete = any(word in normalized for word in ("删除", "清理", "移除", "删掉", "执行删除"))
        has_ad = any(word in normalized for word in ("广告", "垃圾信息", "牛皮癣"))
        is_negated = any(word in normalized for word in ("不要删除", "别删除", "禁止删除", "无需删除", "不需要删除"))
        is_question = normalized.startswith(("为什么", "为何", "怎么会"))
        requests_single = "一条" in normalized and not any(word in normalized for word in ("最近", "24小时", "小时内", "今天"))
        return has_delete and has_ad and not is_negated and not is_question and not requests_single

    async def _prepare_recent_ad_deletion(
        self,
        user_text: str,
        ctx: AgentContext,
    ) -> Optional[AgentResponse]:
        if not self._is_recent_ad_delete_request(user_text):
            return None

        hours_match = re.search(r"(\d{1,3})\s*(?:小时|h)", user_text, re.I)
        hours = max(1, min(int(hours_match.group(1)) if hours_match else 24, 24))
        count_match = re.search(r"(?:最近\s*)?(\d{1,2})\s*条", user_text, re.I)
        requested_limit = max(1, min(int(count_match.group(1)) if count_match else 50, 50))
        group_match = re.search(r"(?:group_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", user_text, re.I)
        query_params: Dict[str, Any] = {
            "hours": hours,
            "limit": requested_limit,
            "sample_size": 5,
        }
        include_sender_messages = any(
            phrase in re.sub(r"\s+", "", user_text)
            for phrase in (
                "发布者的所有消息", "发布者所有消息", "发送者的所有消息",
                "发送者所有消息", "该用户所有消息", "他们的所有消息",
            )
        )
        if include_sender_messages:
            query_params["include_sender_messages"] = True
        if group_match:
            query_params["group_id"] = int(group_match.group(1))
        elif isinstance(ctx.metadata, dict):
            context_group_id = ctx.metadata.get("group_id") or ctx.metadata.get("chat_id")
            if str(context_group_id or "").strip():
                try:
                    query_params["group_id"] = int(context_group_id)
                except Exception:
                    pass

        query_tool = get_tool("query_recent_ad_messages")
        if not query_tool:
            return AgentResponse(
                text="最近广告查询工具未加载，未执行任何删除。",
                tool_calls_made=["query_recent_ad_messages"],
                tool_results={"query_recent_ad_messages": {"success": False, "message": "tool_not_loaded"}},
                confidence=0.0,
            )

        try:
            query_result = await query_tool.execute(query_params, context=ctx)
        except Exception as exc:
            logger.error("生成最近广告删除快照失败: %s", exc, exc_info=True)
            return AgentResponse(
                text=f"查询最近 {hours} 小时广告失败，未执行任何删除。",
                tool_calls_made=["query_recent_ad_messages"],
                tool_results={"query_recent_ad_messages": {"success": False, "message": str(exc)}},
                confidence=0.0,
            )

        data = query_result.get("data") or {}
        refs = data.get("candidate_refs") or []
        if not query_result.get("success", True) or not refs:
            return AgentResponse(
                text=self._format_recent_ad_result(query_result),
                tool_calls_made=["query_recent_ad_messages"],
                tool_results={"query_recent_ad_messages": query_result},
                confidence=0.9 if query_result.get("success", True) else 0.2,
            )

        delete_params = {
            "message_refs": refs,
            "cutoff_ts": int(data.get("cutoff_ts") or 0),
            "snapshot_created_at": int(data.get("snapshot_created_at") or 0),
            "hours": hours,
            "include_sender_messages": include_sender_messages,
            "confirmed_sender_ids": data.get("confirmed_sender_ids") or [],
        }
        pending_delete = {
            "success": True,
            "needs_confirmation": True,
            "action_taken": False,
            "_params": delete_params,
            "message": f"已锁定 {len(refs)} 条候选广告，等待确认删除",
            "data": {
                "candidate_count": len(refs),
                "latest_recorded_at": data.get("latest_recorded_at"),
                "earliest_recorded_at": data.get("earliest_recorded_at"),
                "random_sample_times": data.get("random_sample_times") or [],
                "delete_enabled": bool(getattr(config, "AD_DELETE_ENABLED", False)),
            },
        }
        samples = "、".join(data.get("random_sample_times") or []) or "无"
        delete_switch_note = (
            "真实删除开关已开启。"
            if pending_delete["data"]["delete_enabled"] else
            "当前 AD_DELETE_ENABLED 未开启，即使确认也会被后端阻止。"
        )
        permission_excluded_count = int(data.get("permission_excluded_count") or 0)
        permission_note = (
            f"另有 {permission_excluded_count} 条位于机器人无删除权限的群，未纳入本次快照。\n"
            if permission_excluded_count else ""
        )
        sender_note = (
            f"已按明确指令额外纳入这些广告发送者的 {int(data.get('sender_expanded_count') or 0)} 条同时间窗消息。\n"
            if include_sender_messages else ""
        )
        text = (
            f"已查询最近 {hours} 小时广告记录，安全快照锁定 {len(refs)} 条。\n"
            f"最新时间：{data.get('latest_recorded_at') or '无'}\n"
            f"最早时间：{data.get('earliest_recorded_at') or '无'}\n"
            f"随机时间样本：{samples}\n\n"
            f"{permission_note}"
            f"{sender_note}"
            "已排除管理员、bot、频道转发和低分记录；单次最多50条，快照15分钟后失效。\n"
            f"{delete_switch_note}\n"
            "请回复「确认」执行这份精确快照，其他回复均不会删除。"
        )
        return AgentResponse(
            text=text,
            tool_calls_made=["query_recent_ad_messages", "delete_recent_ad_messages"],
            tool_results={
                "query_recent_ad_messages": query_result,
                "delete_recent_ad_messages": pending_delete,
            },
            is_action_taken=False,
            requires_confirmation=True,
            action_summary=f"待确认删除最近{hours}小时广告 {len(refs)} 条",
            confidence=1.0,
        )

    @staticmethod
    def _format_recent_ad_result(result: Dict[str, Any]) -> str:
        if not result.get("success", True):
            return f"最近广告记录查询失败：{result.get('message') or '未知错误'}。未执行任何删除。"
        data = result.get("data") or {}
        count = int(data.get("snapshot_count") or 0)
        hours = int(data.get("hours") or 24)
        if not count:
            return result.get("limitation") or f"最近 {hours} 小时没有可安全处理的已标记用户广告。"
        samples = "、".join(data.get("random_sample_times") or []) or "无"
        deletable_count = int(data.get("deletable_count", count) or 0)
        permission_note = ""
        if deletable_count < count:
            permission_note = f"\n其中 {deletable_count} 条具备删除权限，{count - deletable_count} 条因机器人群权限不足被排除。"
        return (
            f"最近 {hours} 小时共找到 {data.get('total_eligible_count', count)} 条合格广告，"
            f"本次快照返回 {count} 条。\n"
            f"最新时间：{data.get('latest_recorded_at') or '无'}\n"
            f"最早时间：{data.get('earliest_recorded_at') or '无'}\n"
            f"随机时间样本：{samples}{permission_note}"
        )

    @staticmethod
    def _format_ad_cluster_result(result: Dict[str, Any]) -> str:
        if not result.get("success", True):
            return f"广告聚合查询失败：{result.get('message') or '未知错误'}"
        evidence_count = int(result.get("evidence_count") or 0)
        data = result.get("data") or {}
        clusters = data.get("clusters") or []
        if evidence_count <= 0 or result.get("analysis_allowed") is False:
            return result.get("limitation") or "当前没有 bot 已收到并落库的消息证据，不能判断群内没有广告。"
        if not clusters:
            return (
                f"已检查当前时间窗内 {evidence_count} 条已落库消息，未形成疑似广告聚类。"
                "这不代表群聊全部历史中没有广告。"
            )
        return f"已检查 {evidence_count} 条已落库消息，发现 {len(clusters)} 个疑似广告聚类。"

    @staticmethod
    def _format_system_query_result(result: Dict[str, Any]) -> str:
        if not result.get("success", True):
            return f"系统查询失败：{result.get('message') or '未知错误'}"
        data = result.get("data")
        if isinstance(data, dict) and "total_count" in data:
            groups = data.get("groups") or []
            channels = data.get("channels") or []
            broadcast_only = data.get("broadcast_only_groups") or []
            lines = [
                f"当前绑定群组 {int(data.get('group_count') or 0)} 个，频道 {int(data.get('channel_count') or 0)} 个。"
            ]
            if broadcast_only:
                lines.append(f"另有仅广播目标群 {int(data.get('broadcast_only_group_count') or 0)} 个。")
            if groups:
                lines.append("群组：" + "、".join(
                    f"{item.get('title') or item.get('chat_title') or '未命名'} ({item.get('chat_id')})"
                    for item in groups[:20]
                ))
            if channels:
                lines.append("频道：" + "、".join(
                    f"{item.get('title') or item.get('chat_title') or '未命名'} ({item.get('chat_id')})"
                    for item in channels[:20]
                ))
            if broadcast_only:
                lines.append("仅广播目标群：" + "、".join(
                    f"{item.get('title') or item.get('chat_title') or '未命名'} ({item.get('chat_id')})"
                    for item in broadcast_only[:20]
                ))
            return "\n".join(lines)

        if isinstance(data, list):
            evidence_count = int(result.get("evidence_count") or result.get("count") or len(data))
            if evidence_count <= 0:
                return result.get("limitation") or result.get("message") or "当前没有可用的已落库数据。"
            previews = []
            for item in data[:10]:
                if not isinstance(item, dict):
                    continue
                title = item.get("group_title") or item.get("title") or item.get("username") or item.get("user_id") or "记录"
                content = str(item.get("content") or "").strip().replace("\n", " ")[:80]
                previews.append(f"{title}: {content}" if content else str(title))
            heading = result.get("message") or f"查询到 {evidence_count} 条记录"
            return heading + ("\n" + "\n".join(f"- {item}" for item in previews) if previews else "")

        if isinstance(data, dict):
            summary_items = [
                f"{key}={value}" for key, value in data.items()
                if isinstance(value, (str, int, float, bool))
            ]
            return (result.get("message") or "系统查询完成") + ("\n" + "，".join(summary_items) if summary_items else "")
        return result.get("message") or "系统查询完成。"

    @staticmethod
    def _format_ad_detection_result(result: Dict[str, Any]) -> str:
        if not result.get("success", True):
            return f"广告检测失败：{result.get('message') or '未知错误'}"
        data = result.get("data") or {}
        findings = data.get("findings") or []
        verdict = "检测为广告" if data.get("is_advertising") else "未达到广告判定阈值"
        text = f"{verdict}，评分 {int(data.get('ad_score') or 0)}/100，置信度 {data.get('confidence') or 'low'}。"
        if findings:
            text += "\n依据：" + "；".join(str(item) for item in findings[:8])
        return text

    @staticmethod
    def _format_finance_result(result: Dict[str, Any]) -> str:
        if not result.get("success", True):
            return f"财务查询失败：{result.get('message') or '未知错误'}"
        data = result.get("data")
        if isinstance(data, dict) and "summary" in data:
            summary = data.get("summary") or {}
            rows = summary.get("by_currency", []) if isinstance(summary, dict) else []
            return (
                f"财务综合查询完成：{len(rows)} 个币种，"
                f"待核销 {int(data.get('unreconciled_count') or 0)} 条。"
            )
        if isinstance(data, dict) and "by_currency" in data:
            rows = data.get("by_currency") or []
            return (
                f"财务摘要：{len(rows)} 个币种，"
                f"待核销 {int(data.get('total_unreconciled') or 0)} 条。"
            )
        if isinstance(data, list):
            return result.get("message") or f"财务查询完成，共 {len(data)} 条记录。"
        return result.get("message") or "财务查询完成。"

    @staticmethod
    def _telegram_capability_rows(result: Dict[str, Any]) -> tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
        data = result.get("data") or {}
        items = data.get("items") if isinstance(data, dict) else None
        if not isinstance(items, list):
            items = [data] if isinstance(data, dict) and data.get("chat_id") else []
        verified = [item for item in items if item.get("success") and item.get("chat_type")]
        unavailable = [item for item in items if item not in verified]
        return verified, unavailable

    @classmethod
    def _format_telegram_sdk_result(cls, result: Dict[str, Any]) -> str:
        if not result.get("success", True):
            return f"Telegram SDK 查询失败：{result.get('message') or '未知错误'}"
        verified, unavailable = cls._telegram_capability_rows(result)
        if not verified:
            return "Telegram 官方 API 没有验证到当前 Bot 可访问的群或频道。"

        deletable = []
        non_deletable = []
        for item in verified:
            permissions = item.get("bot_permissions") or {}
            status = str(item.get("bot_status") or permissions.get("status") or "unknown")
            can_delete = status == "creator" or (
                status == "administrator" and bool(permissions.get("can_delete_messages"))
            )
            label = f"{item.get('title') or '未命名'} ({item.get('chat_id')})"
            (deletable if can_delete else non_deletable).append(f"{label} [{status}]")

        lines = [
            f"Telegram 官方 API 已验证 {len(verified)} 个可访问聊天："
            f"{len(deletable)} 个可删除消息，{len(non_deletable)} 个不可删除。"
        ]
        if deletable:
            lines.append("可删除：" + "、".join(deletable))
        if non_deletable:
            lines.append("不可删除：" + "、".join(non_deletable))
        if unavailable:
            lines.append(f"另有 {len(unavailable)} 条数据库绑定记录未通过 Telegram 验证，不计入可用聊天。")
        return "\n".join(lines)

    @classmethod
    def _format_verified_bound_chats(cls, system_result: Dict[str, Any], sdk_result: Dict[str, Any]) -> str:
        verified, unavailable = cls._telegram_capability_rows(sdk_result)
        if not verified and not unavailable:
            return (
                cls._format_system_query_result(system_result)
                + "\nTelegram SDK 本次未返回可核验记录，以上数量仅来自数据库绑定表。"
            )
        groups = [item for item in verified if item.get("chat_type") in {"group", "supergroup"}]
        channels = [item for item in verified if item.get("chat_type") == "channel"]
        lines = [f"Telegram 官方 API 当前验证到群组 {len(groups)} 个，频道 {len(channels)} 个。"]
        if groups:
            lines.append("群组：")
            lines.extend(
                f"{index}. {item.get('title') or '未命名'} ({item.get('chat_id')})"
                for index, item in enumerate(groups, 1)
            )
        if channels:
            lines.append("频道：")
            lines.extend(
                f"{index}. {item.get('title') or '未命名'} ({item.get('chat_id')})"
                for index, item in enumerate(channels, 1)
            )
        db_data = system_result.get("data") or {}
        db_total = int(db_data.get("group_count") or 0) + int(db_data.get("channel_count") or 0)
        if unavailable or db_total > len(verified):
            lines.append(
                f"数据库绑定表共有 {db_total} 条群/频道记录，其中 {len(unavailable)} 条未通过官方验证，"
                "可能是测试、失效或迁移后的旧记录，未计入上述数量。"
            )
        return "\n".join(lines)

    @staticmethod
    def _latest_assistant_text(ctx: AgentContext) -> str:
        for message in reversed(ctx.conversation_history or []):
            if message.get("role") == "assistant":
                return str(message.get("content") or "")
        return ""

    @classmethod
    def _resolve_contextual_command(cls, user_text: str, ctx: AgentContext) -> str:
        """Resolve terse follow-ups against the last verified Agent response."""
        text = str(user_text or "").strip()
        normalized = re.sub(r"\s+", "", text.lower())
        previous = cls._latest_assistant_text(ctx)

        if normalized in {"删除", "删掉", "执行删除", "清理", "处理掉"} and any(
            marker in previous for marker in ("条合格广告", "具备删除权限", "广告记录")
        ):
            return "删除最近24小时的广告"

        ordinal_match = re.search(r"第\s*(\d{1,2}|一|二|三|四|五|六|七|八|九|十)\s*个", text)
        terse_ordinal = normalized in {"第一个", "第二个", "第三个", "第四个", "第五个"}
        wants_group_messages = bool(ordinal_match) and (
            terse_ordinal or any(word in normalized for word in ("群", "消息", "查看", "打开", "看看"))
        )
        if wants_group_messages and "群组：" in previous:
            token = ordinal_match.group(1)
            chinese_numbers = {
                "一": 1, "二": 2, "三": 3, "四": 4, "五": 5,
                "六": 6, "七": 7, "八": 8, "九": 9, "十": 10,
            }
            index = chinese_numbers.get(token, int(token) if token.isdigit() else 0)
            group_section = previous.split("群组：", 1)[-1].split("频道：", 1)[0]
            group_ids = [int(value) for value in re.findall(r"\((-?\d{5,})\)", group_section)]
            if 1 <= index <= len(group_ids):
                return f"查看群 {group_ids[index - 1]} 最近群消息"

        return text

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
                "检测广告", "广告检测", "这条是不是广告", "这段是不是广告", "鉴别这条", "检测这条消息",
            ],
            "query_ad_clusters": [
                "垃圾信息检测", "是否有广告", "是否存在广告",
                "有没有广告", "群广告", "疑似广告", "广告消息", "灰黑产",
                "打广告", "发广告", "谁在打广告", "哪些人打广告", "有哪些人打广告",
                "清理广告", "处理广告",
            ],
            "query_recent_ad_messages": [
                "检查广告", "广告检查", "查看广告", "最近24小时广告", "最近广告记录", "广告记录时间",
                "随机抽取广告", "随机广告时间", "今天的广告记录",
            ],
            "delete_recent_ad_messages": [
                "删除最近24小时的广告", "执行删除最近", "清理最近广告",
            ],
            "delete_ad_message": [
                "删除一条广告", "删除最近广告", "删除广告消息",
                "识别广告并删除一条", "删掉这条广告",
            ],
            "delete_ad_cluster": [
                "删除cluster", "删除 cluster", "delete_ad_cluster", "删除广告cluster",
                "删除广告聚类", "批量删除广告",
            ],
            "generate_content": [
                "帮我写", "生成文案", "写一段", "生成内容",
                "帮我生成", "写个文案", "写公告",
            ],
            "system_query": [
                "系统状态", "系统概览", "有多少群", "群组统计",
                "有几个群", "几个群", "有多少频道", "有几个频道", "几个频道",
                "绑定群", "绑定频道", "群列表", "频道列表", "机器人绑定", "查看群聊", "查询群聊", "我的群聊",
                "系统信息", "活跃群组", "群消息", "群聊消息", "频道消息",
                "最近消息", "最近有谁", "谁给我发", "谁给我发送", "最近发消息",
                "广告消息", "群广告", "疑似广告", "打广告", "发广告", "灰黑产",
                "哪些人打广告", "有哪些人打广告", "谁在打广告",
            ],
            "telegram_sdk_query": [
                "机器人权限", "bot权限", "删除权限", "群权限", "频道权限",
                "机器人能不能删除", "为什么删除失败", "telegram sdk",
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

        if best_match in {
            "finance_query", "system_query", "telegram_sdk_query", "detect_advertising",
            "query_ad_clusters", "query_recent_ad_messages",
        } and best_score >= 8:
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
            if "摘要" in text:
                return {"action": "summary"}
            if any(k in text for k in ("未核销", "待核销")):
                return {"action": "unreconciled"}
            if any(k in text for k in ("科目", "分类")):
                return {"action": "categories"}
            if any(k in text.lower() for k in ("trc20", "bsc")) and "测试" in text:
                return {"action": "onchain_api_test"}
            return {"action": "analysis"}

        if tool_name == "detect_advertising":
            return {"text": text, "context_type": "private_message"}

        if tool_name == "query_ad_clusters":
            args = {"status": "pending", "limit": 20, "window_minutes": 60}
            group_match = re.search(r"(?:group_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
            if group_match:
                args["group_id"] = int(group_match.group(1))
            return args

        if tool_name == "query_recent_ad_messages":
            hours_match = re.search(r"(\d{1,3})\s*(?:小时|h)", text, re.I)
            args = {
                "hours": max(1, min(int(hours_match.group(1)) if hours_match else 24, 24)),
                "limit": 20,
                "sample_size": 5,
            }
            group_match = re.search(r"(?:group_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
            if group_match:
                args["group_id"] = int(group_match.group(1))
            return args

        if tool_name == "telegram_sdk_query":
            group_match = re.search(r"(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
            if group_match:
                return {
                    "operation": "chat_capabilities",
                    "chat_id": int(group_match.group(1)),
                    "include_admins": False,
                }
            return {
                "operation": "bound_chat_capabilities",
                "limit": 20,
                "include_admins": False,
            }

        if tool_name == "delete_ad_message":
            args = {"delete_latest_ad": True}
            group_match = re.search(r"(?:group_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
            tg_msg_match = re.search(r"(?:tg_msg_id|消息|message)\s*[:：=]?\s*(\d{1,12})", text, re.I)
            msg_id_match = re.search(r"(?:msg_id|数据库消息)\s*[:：=]?\s*(\d{1,12})", text, re.I)
            if group_match:
                args["group_id"] = int(group_match.group(1))
            if tg_msg_match:
                args["tg_msg_id"] = int(tg_msg_match.group(1))
            if msg_id_match:
                args["msg_id"] = int(msg_id_match.group(1))
            return args

        if tool_name == "delete_ad_cluster":
            cluster_match = re.search(r"(?:cluster_id|cluster|聚类)\s*[:：=]?\s*([a-zA-Z0-9:_-]+)", text, re.I)
            args = {"dry_run": True}
            if cluster_match:
                args["cluster_id"] = cluster_match.group(1)
            return args

        if tool_name == "generate_content":
            return {"topic": text, "style": "professional", "length": "medium", "purpose": "announcement"}

        if tool_name == "system_query":
            lower = text.lower()
            group_name = self._extract_group_name(text)
            if any(k in text for k in (
                "广告消息", "群广告", "疑似广告", "广告群消息", "有没有广告", "是否有广告",
                "灰黑产", "推广", "垃圾信息", "打广告", "发广告", "谁在广告",
                "谁在打广告", "哪些人打广告", "有哪些人打广告",
            )):
                args = {"query_type": "group_ad_messages", "limit": 20}
                if group_name:
                    args["group_name"] = group_name
                return args
            if any(k in text for k in ("最近有谁", "谁给我发", "谁给我发送", "最近发消息", "谁发了消息")):
                return {"query_type": "recent_senders", "limit": 10}
            if any(k in text for k in (
                "有多少群", "有几个群", "几个群", "有多少频道", "有几个频道", "几个频道",
                "绑定群", "绑定频道", "绑定群聊", "群列表", "频道列表", "机器人绑定",
                "查看群聊", "查询群聊", "我的群聊",
                "我有多少群", "我有几个群", "我有多少频道", "我有几个频道",
            )):
                return {"query_type": "bound_chats", "limit": 20}
            if any(k in text for k in ("频道消息", "频道发言", "最近频道", "频道动态")):
                args = {"query_type": "channel_messages", "limit": 10}
                if group_name:
                    args["group_name"] = group_name
                return args
            if any(k in text for k in ("群消息", "群聊消息", "最近群消息", "群聊动态", "群里消息")):
                args = {"query_type": "group_messages", "limit": 10}
                if group_name:
                    args["group_name"] = group_name
                return args
            if "最近消息" in text:
                args = {"query_type": "group_messages", "limit": 10} if any(k in text for k in ("群", "群聊")) else {"query_type": "recent_messages", "limit": 10}
                if group_name and args["query_type"] == "group_messages":
                    args["group_name"] = group_name
                return args
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
            if any(k in text for k in ("新增", "添加", "增加")) and ("skill" in lower or "技能" in text):
                return {"action": "add"}
            if "启用" in text:
                return {"action": "enable"}
            if any(k in text for k in ("停用", "禁用")):
                return {"action": "disable"}
            if any(k in text for k in ("更新", "修改")):
                return {"action": "update"}
            return {"action": "list"}

        return {}

    def _extract_group_name(self, text: str) -> str:
        """从常见中文表达里尽量提取群名，用于只读查询预执行。"""
        raw = text or ""
        bold = re.search(r"\*\*([^*]{2,80})\*\*", raw)
        if bold:
            return bold.group(1).strip()
        quoted = re.search(r"[「『“\"]([^」』”\"]{2,80})[」』”\"]", raw)
        if quoted:
            return quoted.group(1).strip()
        possessive = re.search(r"(?:我的)?(?:那个|这个)?([^，。,.!?！？\n]{2,60}?)(?:的)?群(?:聊|组)?", raw)
        if possessive:
            candidate = possessive.group(1).strip()
            candidate = re.sub(r"^(?:检查|检测|查询|查看|检索|分析|一下|下|我的|那个|这个)+", "", candidate).strip()
            if candidate:
                return candidate
        named = re.search(r"(?:检查|检测|查询|查看|检索|分析)\s*(?:一下|下)?\s*([^，。,.!?！？\n]{2,80}?群)", raw)
        if named:
            candidate = named.group(1).strip()
            candidate = re.sub(r"^(?:我的|那个|这个)+", "", candidate).strip()
            return candidate
        return ""

    def _build_preflight_tool_calls(self, user_text: str, tools: list) -> List[dict]:
        """对明确的只读查询先执行工具，再让 LLM 总结真实结果。"""
        text = (user_text or "").strip()
        if len(text) < 4:
            return []

        matched_tool_name = self._match_tool_from_text(text, tools)
        if matched_tool_name not in {
            "finance_query",
            "detect_advertising",
            "system_query",
            "telegram_sdk_query",
            "query_ad_clusters",
            "query_recent_ad_messages",
        }:
            return []

        args = self._build_fallback_args(matched_tool_name, text)
        calls = [{
            "function": {
                "name": matched_tool_name,
                "arguments": json.dumps(args, ensure_ascii=False),
            }
        }]
        if matched_tool_name in {
            "finance_query",
            "detect_advertising",
            "telegram_sdk_query",
            "query_ad_clusters",
            "query_recent_ad_messages",
        }:
            return calls

        query_type = args.get("query_type")
        needs_sdk_visibility = query_type in {"group_messages", "channel_messages", "group_ad_messages", "bound_chats"}
        sdk_available = any(t.get("function", {}).get("name") == "telegram_sdk_query" for t in tools)
        if needs_sdk_visibility and sdk_available:
            sdk_args = {
                "operation": "bot_visibility_status" if query_type != "bound_chats" else "bound_chat_capabilities",
                "limit": 50 if query_type == "bound_chats" else 10,
                "include_admins": False,
            }
            if args.get("group_id"):
                sdk_args["chat_id"] = args["group_id"]
            calls.append({
                "function": {
                    "name": "telegram_sdk_query",
                    "arguments": json.dumps(sdk_args, ensure_ascii=False),
                }
            })
        return calls


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
            stripped_text = text.strip() if isinstance(text, str) else text
            if isinstance(stripped_text, str) and stripped_text.startswith("__TOOL_CALL__"):
                try:
                    tool_data = json.loads(stripped_text[len("__TOOL_CALL__"):])
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
                if "用户请求:" in latest_user_text:
                    latest_user_text = latest_user_text.rsplit("用户请求:", 1)[-1].strip()

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
                preview_limit = 1200 if name == "system_query" else 500
                data = result.get("data", "")
                evidence_meta = {
                    "source": result.get("source"),
                    "data_complete": result.get("data_complete"),
                    "analysis_allowed": result.get("analysis_allowed"),
                    "evidence_count": result.get("evidence_count"),
                    "summary": result.get("summary"),
                    "limitation": result.get("limitation"),
                }
                try:
                    data_preview = json.dumps(data, ensure_ascii=False, default=str)[:preview_limit]
                except Exception:
                    data_preview = str(data)[:preview_limit]
                conf_note = " ⚠️ 需确认" if r.get("needs_confirmation") else ""
                meta_preview = json.dumps(
                    {k: v for k, v in evidence_meta.items() if v is not None},
                    ensure_ascii=False,
                    default=str,
                )
                parts.append(f"- [{name}] ✅ {msg}{conf_note}\n  证据状态: {meta_preview}\n  数据: {data_preview}")
        
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
