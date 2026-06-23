# -*- coding: utf-8 -*-
"""
AI API - AI 聊天、OpenAI 兼容接口
"""
from aiohttp import web
import logging
import os
import time
from typing import Any, Dict
from backend.repositories.ai_usage_repo import save_ai_usage_log, get_global_usage_summary
from backend.app.services.quota_service import get_quota_service

logger = logging.getLogger(__name__)

AI_ASSISTANT_USER_ID = int(os.getenv("AI_ASSISTANT_USER_ID", "-99999"))
_ai_agent_sessions: Dict[str, Dict[str, Any]] = {}


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


async def _run_ai_agent_chat(
    *,
    request: web.Request,
    user_id: int,
    input_text: str,
    model: str,
    scope: dict,
):
    from backend.app.services.ai_service import get_ai_service
    from services.ai_agent.core import get_agent
    from services.ai_agent.schema import AgentContext

    async def llm_call(messages: list, model: str = ""):
        return await get_ai_service().chat_messages(
            messages=messages,
            model=model or "gemini-2.5-flash-lite",
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
