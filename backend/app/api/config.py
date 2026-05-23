# -*- coding: utf-8 -*-
"""
配置 API - 系统配置获取与保存
"""
from aiohttp import web
import logging
from typing import Dict, Any
from backend.repositories.admin_repo import get_admin_setting, save_admin_setting
import config as bot_config

logger = logging.getLogger(__name__)
LOCKED_MODEL_DEFAULT = 'gemini-3.1-pro-preview'


def _is_truthy(value: Any) -> bool:
    return str(value).strip().lower() in ('1', 'true', 'yes', 'on')


async def api_get_config(request: web.Request) -> web.Response:
    """获取系统配置"""
    try:
        # 从数据库和管理员设置中获取配置
        llm_base_url = await get_admin_setting('llm_base_url', bot_config.LLM_BASE_URL or '')
        llm_api_key = await get_admin_setting('llm_api_key', bot_config.LLM_API_KEY or '')
        llm_model = await get_admin_setting('llm_model', getattr(bot_config, 'LLM_MODEL_NAME', '') or '')
        llm_model_lock_enabled = await get_admin_setting('llm_model_lock_enabled', 'false')
        llm_model_lock_target = await get_admin_setting('llm_model_lock_target', LOCKED_MODEL_DEFAULT)
        global_auto_translate = await get_admin_setting('global_auto_translate', 'false') == 'true'
        translation_engine = await get_admin_setting('translation_engine', 'llm')
        
        return web.json_response({
            'status': 'ok',
            'data': {
                'llm_base_url': llm_base_url,
                'llm_api_key': llm_api_key,
                'llm_model': llm_model,
                'llm_model_lock_enabled': llm_model_lock_enabled,
                'llm_model_lock_target': llm_model_lock_target,
                'global_auto_translate': global_auto_translate,
                'translation_engine': translation_engine,
                'bot_token_masked': bot_config.BOT_TOKEN[:10] + '...' if bot_config.BOT_TOKEN else '',
                'admin_id': bot_config.ADMIN_ID,
            }
        })
    except Exception as e:
        logger.error(f"获取配置失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_save_config(request: web.Request) -> web.Response:
    """保存系统配置"""
    try:
        data = await request.json()
        
        # 保存各项配置到数据库
        if 'llm_base_url' in data:
            await save_admin_setting('llm_base_url', data['llm_base_url'])
        if 'llm_api_key' in data:
            await save_admin_setting('llm_api_key', data['llm_api_key'])
        if 'llm_model' in data and str(data.get('llm_model', '')).strip():
            await save_admin_setting('llm_model', str(data['llm_model']).strip())
        if 'llm_model_lock_target' in data and str(data.get('llm_model_lock_target', '')).strip():
            await save_admin_setting('llm_model_lock_target', str(data['llm_model_lock_target']).strip())
        if 'llm_model_lock_enabled' in data:
            lock_enabled = _is_truthy(data.get('llm_model_lock_enabled'))
            await save_admin_setting('llm_model_lock_enabled', 'true' if lock_enabled else 'false')
            if lock_enabled:
                lock_target = (
                    str(data.get('llm_model_lock_target', '')).strip()
                    or await get_admin_setting('llm_model_lock_target', LOCKED_MODEL_DEFAULT)
                    or LOCKED_MODEL_DEFAULT
                )
                await save_admin_setting('llm_model', lock_target)
        if 'global_auto_translate' in data:
            await save_admin_setting('global_auto_translate', 'true' if data['global_auto_translate'] else 'false')
        if 'translation_engine' in data:
            await save_admin_setting('translation_engine', data['translation_engine'])
        
        return web.json_response({'status': 'ok'})
    except Exception as e:
        logger.error(f"保存配置失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_vertex_oneclick_config(request: web.Request) -> web.Response:
    """Vertex AI 一键配置 - 自动生成 SK 格式和完整 URL"""
    try:
        data = await request.json()
        vertex_api_key = data.get('vertex_api_key', '').strip()
        oneapi_base_url = data.get('oneapi_base_url', '').strip()
        relay_api_key = data.get('relay_api_key', '').strip()
        model = data.get('model', 'gemini-2.5-flash-lite').strip()
        
        if not vertex_api_key:
            return web.json_response(
                {'status': 'error', 'message': 'Vertex API Key 不能为空'},
                status=400
            )
        
        # 如果没有提供 OneAPI Base URL，使用默认格式
        if not oneapi_base_url:
            # 从 Vertex Key 解析项目信息（如果可能）
            oneapi_base_url = "https://generativelanguage.googleapis.com/v1beta"
        
        # 如果没有提供中转 SK，生成一个标准的 SK 格式密钥
        import secrets
        import hashlib
        if not relay_api_key:
            # 生成随机字符串作为 SK
            random_part = secrets.token_hex(16)
            # 结合 Vertex Key 的哈希确保唯一性
            key_hash = hashlib.sha256(vertex_api_key.encode()).hexdigest()[:16]
            relay_api_key = f"sk-{key_hash}{random_part}"
        
        # 构建完整的 API 调用 URL
        # Vertex AI / Gemini 标准格式
        model_infer_url = f"{oneapi_base_url.rstrip('/')}/models/{model}:generateContent"
        model_list_url = f"{oneapi_base_url.rstrip('/')}/models"
        
        # OpenAI 兼容格式（通过 oneapi 中转）
        openai_chat_url = f"{oneapi_base_url.rstrip('/')}/chat/completions"
        
        # 保存所有配置到数据库
        await save_admin_setting('vertex_api_key', vertex_api_key)
        await save_admin_setting('oneapi_base_url', oneapi_base_url)
        await save_admin_setting('relay_api_key', relay_api_key)
        await save_admin_setting('llm_model', model)
        await save_admin_setting('llm_base_url', openai_chat_url)
        await save_admin_setting('llm_api_key', relay_api_key)
        
        # 返回完整的配置信息
        return web.json_response({
            'status': 'ok',
            'data': {
                'oneapi_base_url': oneapi_base_url,
                'model_infer_url': model_infer_url,
                'model_list_url': model_list_url,
                'openai_chat_url': openai_chat_url,
                'relay_api_key': relay_api_key,
                'relay_api_key_masked': f"{relay_api_key[:8]}...{relay_api_key[-4:]}" if len(relay_api_key) > 12 else relay_api_key,
                'model': model,
                'vertex_api_key_masked': f"{vertex_api_key[:4]}...{vertex_api_key[-4:]}" if len(vertex_api_key) > 8 else vertex_api_key,
            },
            'message': 'Vertex AI 配置已生成并保存'
        })
    except Exception as e:
        logger.error(f"Vertex 配置失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_gemini_models(request: web.Request) -> web.Response:
    """获取最新的 Gemini 模型列表"""
    try:
        # 最新的 Gemini 模型列表（2026 年）
        gemini_models = [
            # Gemini 3.1 系列
            'gemini-3.1-pro-preview',

            # Gemini 2.5 系列（最新）
            'gemini-2.5-pro',
            'gemini-2.5-flash',
            'gemini-2.5-flash-lite',
            
            # Gemini 2.0 系列
            'gemini-2.0-flash',
            'gemini-2.0-flash-lite',
            'gemini-2.0-flash-exp',
            
            # Gemini 1.5 系列
            'gemini-1.5-pro',
            'gemini-1.5-pro-latest',
            'gemini-1.5-flash',
            'gemini-1.5-flash-latest',
            'gemini-1.5-flash-8b',
            
            # 其他专用模型
            'gemini-pro',
            'gemini-pro-vision',
        ]
        
        return web.json_response({
            'status': 'ok',
            'data': gemini_models
        })
    except Exception as e:
        logger.error(f"获取模型列表失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )
