# -*- coding: utf-8 -*-
"""
AI API - AI 聊天、OpenAI 兼容接口
"""
from aiohttp import web
import logging
import time
from backend.repositories.ai_usage_repo import save_ai_usage_log, get_global_usage_summary
from backend.app.services.quota_service import get_quota_service

logger = logging.getLogger(__name__)


def _session_ctx(request: web.Request) -> dict:
    session = request.get('session') or {}
    return {
        'role': str(session.get('role') or 'platform_admin'),
        'tenant_id': str(session.get('tenant_id') or '').strip() or 'default',
        'merchant_id': session.get('merchant_id'),
    }


async def api_ai_chat(request: web.Request) -> web.Response:
    """AI 聊天接口 - 兼容前端 ChatPanel.vue 的调用格式"""
    try:
        user_id = int(request.match_info['user_id'])
        data = await request.json()
        model = data.get('model', 'gemini-2.5-flash-lite')
        input_text = data.get('content') or data.get('input', '')
        feature_key = str(data.get('feature_key') or 'general')
        ctx = _session_ctx(request)
        tenant_id = ctx['tenant_id']
        merchant_id = ctx['merchant_id']

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

        reply = None
        try:
            from backend.app.services.ai_service import get_ai_service
            svc = get_ai_service()
            reply = await svc.chat(user_id=user_id, message=input_text, model=model)
        except Exception as ai_err:
            logger.error(f"AI 服务调用失败: {ai_err}")
            reply = f"[AI] 收到您的消息: {input_text}"

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

        return web.json_response({'status': 'ok', 'reply': reply, 'data': {'reply': reply}})
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
