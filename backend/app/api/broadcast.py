# -*- coding: utf-8 -*-
"""
群发 API - 统一代理到 `web_admin.py` 的稳定实现，避免双轨逻辑分叉。
"""
from aiohttp import web


async def api_get_broadcast_targets(request: web.Request) -> web.Response:
    import web_admin as wa
    return await wa.api_get_broadcast_targets(request)


async def api_sync_broadcast_targets(request: web.Request) -> web.Response:
    import web_admin as wa
    return await wa.api_sync_broadcast_targets(request)


async def api_broadcast_unified(request: web.Request) -> web.Response:
    import web_admin as wa
    return await wa.api_broadcast_unified(request)


async def api_list_push_tasks(request: web.Request) -> web.Response:
    import web_admin as wa
    return await wa.api_list_push_tasks(request)


async def api_create_push_task(request: web.Request) -> web.Response:
    import web_admin as wa
    return await wa.api_create_push_task(request)


async def api_update_push_task_status(request: web.Request) -> web.Response:
    import web_admin as wa
    return await wa.api_update_push_task_status(request)
