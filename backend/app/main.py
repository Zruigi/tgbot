# -*- coding: utf-8 -*-
"""
Web 服务器主入口 - 统一路由注册
"""
from aiohttp import web
import logging
from typing import Optional
from telegram import Bot

from backend.app.api import auth, chat, config, group, broadcast, ai, monitor, potential, finance
from backend.app.core.security import SecurityMiddleware, TokenManager, RateLimiter
from backend.app.core.scope import tenant_context_middleware

logger = logging.getLogger(__name__)

# Bot 实例引用
_bot_instance: Optional[Bot] = None


def create_app() -> web.Application:
    """创建 Web 应用"""
    # 初始化安全组件
    token_manager = TokenManager()
    rate_limiter = RateLimiter(window_seconds=60, max_attempts=5)
    security_middleware = SecurityMiddleware(token_manager, rate_limiter)
    
    # 创建应用
    app = web.Application(middlewares=[security_middleware.auth_middleware, tenant_context_middleware])
    
    # 存储全局对象
    app['token_manager'] = token_manager
    app['rate_limiter'] = rate_limiter
    
    # 注册认证路由
    app.router.add_post('/api/login', auth.api_login)
    app.router.add_post('/api/login/otp', auth.api_login_otp)
    app.router.add_get('/api/csrf-token', auth.api_get_csrf_token)
    app.router.add_post('/api/csp-report', auth.api_csp_report)
    
    # 注册私聊路由
    app.router.add_get('/api/users', chat.api_get_users)
    app.router.add_post('/api/users/rebuild', chat.api_rebuild_inbox)  # 调试用：手动重建 Inbox
    app.router.add_get('/api/messages/{user_id}', chat.api_get_messages)
    app.router.add_post('/api/messages/{user_id}', chat.api_send_message)
    app.router.add_post('/api/messages/{user_id}/delete', chat.api_delete_single_message)
    app.router.add_post('/api/mark_read/{user_id}', chat.api_mark_read)
    app.router.add_post('/api/delete_chat/{user_id}', chat.api_delete_chat)
    app.router.add_get('/api/search', chat.api_search)
    
    # 注册配置路由
    app.router.add_get('/api/config', config.api_get_config)
    app.router.add_post('/api/config', config.api_save_config)
    app.router.add_post('/api/config/vertex/oneclick', config.api_vertex_oneclick_config)
    app.router.add_get('/api/config/gemini-models', config.api_get_gemini_models)
    
    # 注册群组路由
    app.router.add_get('/api/group_listen/list', group.api_list_groups)
    app.router.add_post('/api/group_listen/toggle', group.api_toggle_group_listen)
    app.router.add_get('/api/group_members', group.api_get_group_members)
    app.router.add_get('/api/group_messages', group.api_get_group_messages)
    app.router.add_get('/api/group_verify_config', group.api_get_group_verify_config)
    app.router.add_post('/api/group_verify_config', group.api_save_group_verify_config)
    app.router.add_get('/api/verify/stats', group.api_get_verify_stats)
    app.router.add_get('/api/verify/growth', group.api_get_verify_growth)
    
    # 注册广播路由
    app.router.add_get('/api/broadcast/targets', broadcast.api_get_broadcast_targets)
    app.router.add_post('/api/broadcast/targets/sync', broadcast.api_sync_broadcast_targets)
    app.router.add_post('/api/broadcast/send', broadcast.api_broadcast_unified)
    app.router.add_post('/api/broadcast/unified', broadcast.api_broadcast_unified)
    app.router.add_get('/api/push_tasks', broadcast.api_list_push_tasks)
    app.router.add_post('/api/push_tasks', broadcast.api_create_push_task)
    app.router.add_put('/api/push_tasks/status', broadcast.api_update_push_task_status)
    
    # 注册 AI 路由
    app.router.add_post('/api/messages/{user_id}/ai', ai.api_ai_chat)  # 前端 ChatPanel.vue 使用的路径
    app.router.add_post('/api/ai/chat/{user_id}', ai.api_ai_chat)      # 兼容旧路径
    app.router.add_get('/api/openai/models', ai.api_openai_models)
    app.router.add_post('/api/openai/chat/completions', ai.api_openai_chat_completions)
    app.router.add_post('/api/openai/exec', ai.api_openai_exec)
    app.router.add_get('/api/ai/usage', ai.api_get_ai_usage)
    
    # 注册监听路由
    app.router.add_get('/api/monitors', monitor.api_list_monitors_wrapper)
    app.router.add_post('/api/monitors', monitor.api_create_monitor_wrapper)
    app.router.add_delete('/api/monitors/{monitor_id}', monitor.api_delete_monitor_wrapper)
    app.router.add_put('/api/monitors/{monitor_id}/remark', monitor.api_update_monitor_remark_wrapper)
    app.router.add_get('/api/monitors/pnl', monitor.api_monitor_pnl_wrapper)
    app.router.add_put('/api/monitors/transactions/meta', monitor.api_update_tx_meta_wrapper)
    app.router.add_put('/api/monitors/transactions/meta/batch', monitor.api_batch_update_tx_meta_wrapper)
    app.router.add_get('/api/monitors/export', monitor.api_export_wrapper)
    app.router.add_get('/api/monitors/stats', monitor.api_monitor_stats_wrapper)
    
    # 注册潜在用户路由
    app.router.add_get('/api/potential_users', potential.api_get_potential_users)
    app.router.add_post('/api/potential_users/sync', potential.api_sync_potential_users)
    app.router.add_get('/api/potential_users/tags', potential.api_get_potential_tags)
    app.router.add_post('/api/potential_users/{user_id}/promote', potential.api_promote_potential_user)
    
    # 注册财务路由
    app.router.add_get('/api/finance/summary', finance.api_get_finance_summary)
    app.router.add_post('/api/finance/reconcile', finance.api_reconcile_finance)
    app.router.add_get('/api/finance/categories', finance.api_get_finance_categories)
    app.router.add_post('/api/finance/calculate_pnl', finance.api_calculate_pnl)
    app.router.add_post('/api/finance/report', finance.api_generate_finance_report)
    
    # SPA 前端静态文件服务
    app.router.add_static('/static', path='frontend/dist', name='static')
    app.router.add_get('/', lambda r: web.HTTPFound('/static/index.html'))
    
    return app


async def start_web_server(bot: Optional[Bot] = None) -> None:
    """启动 Web 服务器"""
    global _bot_instance
    _bot_instance = bot
    
    # 创建应用
    app = create_app()
    
    # 启动服务器
    runner = web.AppRunner(app)
    await runner.setup()
    
    host = '0.0.0.0'
    port = 8080
    
    site = web.TCPSite(runner, host, port)
    await site.start()
    
    logger.info(f"✅ Web 服务器已启动：http://{host}:{port}")


def get_bot_instance() -> Optional[Bot]:
    """获取 Bot 实例"""
    return _bot_instance
