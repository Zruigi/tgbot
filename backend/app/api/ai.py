# -*- coding: utf-8 -*-
"""
AI API - AI 聊天、OpenAI 兼容接口

架构说明：
  本模块采用「结构化意图路由（Structured Intent Router）」架构，替代传统的
  AgentExecutor 方案。核心思路是将 LLM 的角色从"自由发挥的 Agent"降级为
  "严格的选择题机器"——强迫大模型输出固定结构的 JSON，由 Python 后端
  接管工具调用执行逻辑。

  数据流：
  [Telegram 群聊消息] → [n8n / 前端] → [Python 后端 /api/ai/chat]
      → [LLM JSON Mode] → 结构化意图解析
      → 情况 A (need_tool=True)：后端硬编码执行本地广告查询 → LLM 总结 → 返回文本
      → 情况 B (need_tool=False)：直接返回 LLM 生成的 reply_content
      → [纯文本回复发送给 TG 用户]
"""
from aiohttp import web
import logging
import os
import time
from typing import Any, Dict, Optional
from pydantic import BaseModel, Field
import config
from backend.repositories.ai_usage_repo import save_ai_usage_log, get_global_usage_summary
from backend.app.services.quota_service import get_quota_service

logger = logging.getLogger(__name__)

AI_ASSISTANT_USER_ID = config.AI_ASSISTANT_USER_ID
_ai_agent_sessions: Dict[str, Dict[str, Any]] = {}


# =============================================================================
# 结构化意图路由 - 数据模型
# =============================================================================

class IntentAnalysis(BaseModel):
    """LLM 结构化意图识别输出模型

    强制大模型以 JSON 格式输出，Python 后端根据字段做确定性路由。
    """
    need_tool: bool = Field(
        description="如果用户要求查询、检测、聚类群聊内的广告或牛皮癣信息，必须为 True；"
                    "如果是普通聊天或问候，为 False。"
    )
    tool_name: Optional[str] = Field(
        None,
        description="固定值：'query_ad_clusters'。当 need_tool=True 时必填。",
    )
    group_id: Optional[str] = Field(
        None,
        description="从上下文或消息中提取的群组/聊天 ID（仅辅助作用，实际以系统传入为准）",
    )
    reply_content: str = Field(
        description="当 need_tool=False 时的直接回复内容；当 need_tool=True 时，"
                    "给用户的等待提示语（例如：'正在为您查询群聊广告，请稍候...'）",
    )


# =============================================================================
# 结构化意图路由 - 本地工具执行
# =============================================================================

# 意图识别系统提示词（强制 LLM 输出固定结构 JSON）
INTENT_SYSTEM_PROMPT = """你是一个严格的 Telegram 群聊管理助手。
分析用户的输入，决定是否需要调用广告检测或聚类工具。
你必须严格按照 JSON 格式输出，包含以下字段：
- need_tool: 如果用户要求查询、检测、聚类群聊内的广告或牛皮癣信息，必须为 true；如果是普通聊天或问候，为 false。
- tool_name: 如果 need_tool 为 true，固定为 "query_ad_clusters"；否则为 null。
- group_id: 如果 need_tool 为 true 且消息中提到了群组 ID，提取出来；否则为 null。
- reply_content: 当 need_tool 为 false 时的直接回复内容；当 need_tool 为 true 时，给用户的等待提示语。

示例 1 (普通聊天):
{"need_tool": false, "tool_name": null, "group_id": null, "reply_content": "你好！有什么我可以帮你的吗？"}

示例 2 (广告查询):
{"need_tool": true, "tool_name": "query_ad_clusters", "group_id": "-1001234567890", "reply_content": "正在为您查询群聊广告，请稍候..."}

示例 3 (问候):
{"need_tool": false, "tool_name": null, "group_id": null, "reply_content": "下午好！我是群聊管理助手，有什么需要帮助的吗？"}"""


async def execute_local_ad_cluster_query(
    group_id: Optional[str] = None,
    tenant_id: Optional[str] = None,
) -> dict:
    """本地执行广告聚类查询，完全跳过 LLM 盲盒执行。

    此函数由结构化意图路由器在 need_tool=True 时硬编码调用，
    确保工具执行逻辑 100% 在 Python 后端掌控之下。
    """
    from services.ad_pipeline import get_ad_pipeline

    try:
        result = await get_ad_pipeline().query_clusters(
            tenant_id=tenant_id,
            group_id=int(group_id) if str(group_id or "").strip() else None,
            status="pending",
            limit=20,
            window_minutes=60,
        )
        return result
    except Exception as e:
        logger.error(f"本地广告聚类查询失败: {e}", exc_info=True)
        return {
            "success": False,
            "message": f"广告聚类查询失败: {str(e)[:200]}",
            "data": {},
        }


def _session_ctx(request: web.Request) -> dict:
    session = request.get('session') or {}
    return {
        'role': str(session.get('role') or 'platform_admin'),
        'tenant_id': str(session.get('tenant_id') or '').strip() or 'default',
        'merchant_id': session.get('merchant_id'),
    }


def _is_agent_confirmation_text(text: str) -> bool:
    normalized = str(text or '').strip().lower()
    return normalized in {'确认', '确认执行', '同意', 'yes', 'y', 'ok', 'confirm'}


def _agent_session_key(*, tenant_id: str, merchant_id: Any, user_id: int) -> str:
    return f"{tenant_id or 'default'}:{merchant_id or 0}:{user_id}"


def _agent_response_payload(agent_result) -> dict:
    evidence = {}
    data_complete_values = []
    analysis_allowed_values = []
    for tool_name, tool_result in (agent_result.tool_results or {}).items():
        if not isinstance(tool_result, dict):
            continue
        evidence[tool_name] = {
            'success': tool_result.get('success', True),
            'message': tool_result.get('message'),
            'source': tool_result.get('source'),
            'data_complete': tool_result.get('data_complete'),
            'analysis_allowed': tool_result.get('analysis_allowed'),
            'evidence_count': tool_result.get('evidence_count'),
            'limitation': tool_result.get('limitation'),
        }
        if tool_result.get('data_complete') is not None:
            data_complete_values.append(bool(tool_result.get('data_complete')))
        if tool_result.get('analysis_allowed') is not None:
            analysis_allowed_values.append(bool(tool_result.get('analysis_allowed')))

    return {
        'tool_calls_made': agent_result.tool_calls_made,
        'tool_results': agent_result.tool_results,
        'evidence': evidence,
        'data_complete': all(data_complete_values) if data_complete_values else None,
        'analysis_allowed': all(analysis_allowed_values) if analysis_allowed_values else None,
        'is_action_taken': agent_result.is_action_taken,
        'requires_confirmation': agent_result.requires_confirmation,
        'action_summary': agent_result.action_summary,
    }


async def _run_structured_ad_query(
    *,
    request: web.Request,
    user_id: int,
    input_text: str,
    model: str,
    scope: dict,
    chat_id: Optional[str] = None,
):
    """结构化意图路由 — 替代原有的 AgentExecutor 架构。

    核心思想：
      1. 强迫 LLM 以 JSON Mode 输出结构化意图（IntentAnalysis）
      2. Python 后端根据 intent.need_tool 做确定性路由
      3. 需要工具时，后端硬编码调用本地函数，再将真实结果丢给 LLM 总结
      4. 不需要工具时，直接返回 LLM 生成的 reply_content

    安全措施：
      - 输入截断：消息最长 500 字符（防垃圾流撑爆 2G 内存）
      - 硬超时：每次 LLM 调用最长 15 秒（防 API 悬挂拖死进程）
      - JSON 解析兜底：解析失败则降级为纯文本回复，拒绝执行工具
      - Prompt Injection 防御：以系统传入的 real_chat_id 为准，忽略 LLM 返回的 group_id
    """
    from backend.app.services.ai_service import get_ai_service
    from services.ai_agent.schema import AgentResponse

    # ── 安全措施 1：输入截断 ──
    input_text = (input_text or "").strip()[:500]
    if not input_text:
        return AgentResponse(text="请输入有效内容。", confidence=0.0)

    tenant_id = scope.get('tenant_id') or 'default'

    # ── 安全措施 2：以系统传入的真实 chat_id 为准（防 Prompt Injection）──
    real_group_id = str(chat_id or "").strip() or None

    svc = get_ai_service()

    # ── 第 1 步：强迫模型进行结构化意图识别 ──
    try:
        intent_text, _ = await svc.chat_messages(
            messages=[
                {"role": "system", "content": INTENT_SYSTEM_PROMPT},
                {"role": "user", "content": input_text},
            ],
            model=model,
            temperature=0.0,          # 降为 0，追求极端稳定性
            timeout_seconds=15.0,      # 硬超时，绝不拖死服务器
            response_format={"type": "json_object"},
        )
    except Exception as e:
        logger.error(f"LLM 意图识别失败: {e}")
        return AgentResponse(
            text="抱歉，AI 服务暂时不可用，请稍后再试。",
            confidence=0.0,
        )

    # ── 安全措施 3：JSON 解析兜底 ──
    try:
        intent = IntentAnalysis.model_validate_json(intent_text)
    except Exception as e:
        logger.warning(f"JSON 意图解析失败: {intent_text[:200]}, 错误: {e}")
        return AgentResponse(
            text="抱歉，刚刚脑子卡壳了，请再试一次。",
            confidence=0.3,
        )

    # ── 第 2 步：确定性路由逻辑 ──
    if intent.need_tool and intent.tool_name == "query_ad_clusters":
        # 安全措施 4：以真实 group_id 覆盖 LLM 提取的（防 Prompt Injection）
        target_group = real_group_id or intent.group_id or None

        # 本地执行广告聚类查询（完全跳过 LLM 盲盒）
        try:
            tool_result_data = await execute_local_ad_cluster_query(
                group_id=target_group,
                tenant_id=tenant_id,
            )
        except Exception as e:
            logger.error(f"广告聚类查询执行异常: {e}", exc_info=True)
            return AgentResponse(
                text="查询广告时出现错误，请稍后重试。",
                tool_calls_made=["query_ad_clusters"],
                tool_results={"query_ad_clusters": {"success": False, "message": str(e)}},
                is_action_taken=False,
                confidence=0.3,
            )

        # ── 第 3 步：将真实数据塞给大模型进行人性化总结 ──
        import json
        try:
            summary_text, _ = await svc.chat_messages(
                messages=[
                    {
                        "role": "system",
                        "content": (
                            "你是一个群聊管理员。请根据以下真实的广告查询数据，"
                            "为用户写一份简明扼要的汇报。"
                            "只有 analysis_allowed=true 且 evidence_count>0 时，才能判断是否发现广告。"
                            "如果没有采集证据，请明确说明当前无法判断，不能说群聊正常或没有广告；"
                            "如果发现广告，说明广告数量、内容和推荐处理方式。"
                        ),
                    },
                    {
                        "role": "user",
                        "content": (
                            f"广告查询结果数据：\n"
                            f"{json.dumps(tool_result_data, ensure_ascii=False, default=str)[:3000]}"
                        ),
                    },
                ],
                model=model,
                temperature=0.3,
                timeout_seconds=15.0,
            )
        except Exception as e:
            logger.error(f"广告结果总结失败: {e}")
            summary_text = "查询完成，但总结生成失败，请查看原始数据。"

        return AgentResponse(
            text=summary_text,
            tool_calls_made=["query_ad_clusters"],
            tool_results={"query_ad_clusters": tool_result_data},
            is_action_taken=False,
            action_summary="query_ad_clusters",
            confidence=0.9,
        )

    # ── 普通对话，直接返回 LLM 生成的内容 ──
    return AgentResponse(
        text=intent.reply_content,
        confidence=0.8,
    )


async def _run_ai_agent_chat(
    *,
    request: web.Request,
    user_id: int,
    input_text: str,
    model: str,
    scope: dict,
    chat_id: Optional[str] = None,
):
    """备用 backend API 与主 web_admin 入口共用同一套 Agent/工具确认链路。"""
    from backend.app.services.ai_service import get_ai_service
    from services.ai_agent.core import get_agent
    from services.ai_agent.schema import AgentContext

    async def llm_call(messages: list, model: str = ""):
        return await get_ai_service().chat_messages(
            messages=messages,
            model=model or "gemini-2.5-flash-lite",
            timeout_seconds=110.0,
        )

    tenant_id = scope.get('tenant_id') or 'default'
    session_key = _agent_session_key(
        tenant_id=tenant_id,
        merchant_id=scope.get('merchant_id'),
        user_id=user_id,
    )
    session = _ai_agent_sessions.get(session_key)
    if not session:
        session = {
            'ctx': AgentContext(
                user_id=user_id,
                channel='web_ai_chat',
                sandbox_id=f"sbx_{tenant_id}_{user_id}",
                metadata={
                    'tenant_id': tenant_id,
                    'merchant_id': scope.get('merchant_id'),
                    'role': scope.get('role'),
                    'chat_id': chat_id,
                    'group_id': chat_id,
                },
                bot_instance=request.app.get('bot_instance'),
            ),
            'pending': None,
            'updated_at': time.time(),
        }
        _ai_agent_sessions[session_key] = session
    else:
        ctx = session['ctx']
        ctx.metadata['tenant_id'] = tenant_id
        ctx.metadata['merchant_id'] = scope.get('merchant_id')
        ctx.metadata['role'] = scope.get('role')
        ctx.metadata['chat_id'] = chat_id
        ctx.metadata['group_id'] = chat_id
        ctx.bot_instance = request.app.get('bot_instance')
        session['updated_at'] = time.time()

    if len(_ai_agent_sessions) > 300:
        cutoff = time.time() - 6 * 3600
        for key in list(_ai_agent_sessions.keys()):
            if float(_ai_agent_sessions[key].get('updated_at') or 0) < cutoff:
                _ai_agent_sessions.pop(key, None)

    agent = get_agent(llm_call_fn=llm_call)
    pending = session.get('pending')
    if pending is not None and _is_agent_confirmation_text(input_text):
        result = await agent.confirm_and_execute(pending, session['ctx'])
        session['pending'] = None
        return result

    result = await agent.process_message(input_text, ctx=session['ctx'], model=model)
    session['pending'] = result if result.requires_confirmation else None
    return result


async def api_ai_chat(request: web.Request) -> web.Response:
    """AI 聊天接口 - 兼容前端 ChatPanel.vue 的调用格式"""
    try:
        data = await request.json()
        raw_user_id = request.match_info.get('user_id') or data.get('userId') or data.get('user_id') or AI_ASSISTANT_USER_ID
        user_id = int(raw_user_id)
        model = data.get('model', 'gemini-2.5-flash-lite')
        input_text = str(data.get('content') or data.get('input') or data.get('message') or '').strip()
        feature_key = str(data.get('feature_key') or 'general')
        ctx = _session_ctx(request)
        body_context = data.get('context') if isinstance(data.get('context'), dict) else {}
        body_tenant_id = str(data.get('tenant_id') or body_context.get('tenant_id') or '').strip()
        if body_tenant_id and ctx.get('role') == 'platform_admin':
            ctx['tenant_id'] = body_tenant_id
        tenant_id = ctx['tenant_id']
        merchant_id = ctx['merchant_id']

        # 提取真实 chat_id / group_id（用于防 Prompt Injection）
        real_chat_id = str(
            data.get('chatId')
            or data.get('chat_id')
            or data.get('group_id')
            or body_context.get('chat_id')
            or body_context.get('group_id')
            or ''
        ).strip() or None

        use_agent = (
            user_id == AI_ASSISTANT_USER_ID
            or bool(data.get('use_agent'))
            or bool(data.get('agent'))
        )

        if not input_text:
            return web.json_response({'status': 'error', 'message': '输入不能为空'}, status=400)

        quota_check = await get_quota_service().check_available(tenant_id, amount=1)
        if not quota_check.get('allowed'):
            return web.json_response(
                {
                    'status': 'error',
                    'message': 'AI 配额不足',
                    'reason': quota_check.get('reason'),
                },
                status=403,
            )

        agent_result = None
        if use_agent:
            try:
                agent_result = await _run_ai_agent_chat(
                    request=request,
                    user_id=user_id,
                    input_text=input_text,
                    model=model,
                    scope=ctx,
                    chat_id=real_chat_id,
                )
                reply = agent_result.text
            except Exception as agent_err:
                logger.error(f"AI Agent 调用失败: {agent_err}", exc_info=True)
                return web.json_response(
                    {'status': 'error', 'message': f'AI Agent 调用失败: {str(agent_err)[:200]}'},
                    status=502,
                )
        else:
            try:
                from backend.app.services.ai_service import get_ai_service
                svc = get_ai_service()
                reply = await svc.chat(user_id=user_id, message=input_text, model=model)
            except Exception as ai_err:
                logger.error(f"AI 服务调用失败: {ai_err}")
                return web.json_response(
                    {'status': 'error', 'message': f'LLM 服务调用失败: {str(ai_err)[:200]}'},
                    status=502,
                )

        total_tokens = (len(input_text) + (len(reply) or 0)) // 4
        estimated_cost_usd = 0.0001

        try:
            await save_ai_usage_log(
                channel='web_chat',
                user_id=user_id,
                sandbox_id=f'sbx_{user_id}',
                tenant_id=tenant_id,
                merchant_id=int(merchant_id) if merchant_id is not None else None,
                feature_key=feature_key,
                result_status='success',
                endpoint='chat',
                model=model,
                prompt_tokens=len(input_text) // 4,
                completion_tokens=len(reply) // 4 if reply else 0,
                tokens_used=total_tokens,
                cost=estimated_cost_usd,
            )
            await get_quota_service().record_usage(tenant_id, amount=1)
        except Exception:
            pass

        payload = {'status': 'ok', 'reply': reply, 'data': {'reply': reply}}
        if agent_result is not None:
            payload.update(_agent_response_payload(agent_result))
        return web.json_response(payload)
    except ValueError:
        return web.json_response({'status': 'error', 'message': '用户 ID 无效'}, status=400)
    except Exception as e:
        logger.error(f"AI 聊天失败：{e}")
        import traceback
        logger.error(traceback.format_exc())
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_openai_models(request: web.Request) -> web.Response:
    """获取 OpenAI 兼容模型列表"""
    try:
        models = [
            {'id': 'gpt-3.5-turbo', 'object': 'model'},
            {'id': 'gpt-4', 'object': 'model'},
            {'id': 'gemini-2.5-flash-lite', 'object': 'model'},
            {'id': 'gemini-2.5-pro', 'object': 'model'},
        ]
        return web.json_response({'status': 'ok', 'data': models})
    except Exception as e:
        logger.error(f"获取模型列表失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_openai_chat_completions(request: web.Request) -> web.Response:
    """OpenAI 兼容聊天接口"""
    try:
        data = await request.json()
        model = data.get('model', 'gpt-3.5-turbo')
        response_message = {'role': 'assistant', 'content': '这是一个模拟的 OpenAI 兼容响应。'}
        usage = {'prompt_tokens': 10, 'completion_tokens': 20, 'total_tokens': 30}
        return web.json_response({
            'id': f'chatcmpl-{int(time.time())}',
            'object': 'chat.completion',
            'created': int(time.time()),
            'model': model,
            'choices': [{'index': 0, 'message': response_message, 'finish_reason': 'stop'}],
            'usage': usage,
        })
    except Exception as e:
        logger.error(f"OpenAI 兼容接口失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_openai_exec(request: web.Request) -> web.Response:
    """OpenAI Exec 协议兼容接口"""
    try:
        data = await request.json()
        input_data = data.get('input', '')
        output = f"Exec 输出：{input_data}"
        return web.json_response({'status': 'ok', 'output': output, 'usage': {'total_tokens': len(input_data) // 4}})
    except Exception as e:
        logger.error(f"Exec 接口失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_get_ai_usage(request: web.Request) -> web.Response:
    """获取 AI 使用统计"""
    try:
        days = int(request.query.get('days', 7))
        tenant_id = _session_ctx(request)['tenant_id']
        summary = await get_global_usage_summary(days=days, tenant_id=tenant_id if tenant_id != 'default' else None)
        return web.json_response({'status': 'ok', 'data': summary})
    except Exception as e:
        logger.error(f"获取使用统计失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


# =============================================================================
# n8n Integration API - Advertisement Detection
# =============================================================================

async def api_detect_advertising(request: web.Request) -> web.Response:
    """广告检测接口 - 供 n8n 节点调用"""
    try:
        data = await request.json()
        text = data.get("text", "")
        context_type = data.get("context_type", "group_message")
        
        if not text:
            return web.json_response({"error": "text is required"}, status=400)
        
        # 调用现有的广告检测逻辑
        from services.ai_agent.ad_detection import detect_advertising_signals
        result = detect_advertising_signals(text, context_type)
        
        return web.json_response(result)
        
    except Exception as e:
        logger.error(f"Ad detection error: {e}")
        return web.json_response({"error": str(e)}, status=500)
