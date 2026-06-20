# -*- coding: utf-8 -*-
"""
潜在用户 API - 潜在用户管理、画像标签
"""
from aiohttp import web
import logging
import database
from backend.repositories.group_repo import (
    get_potential_tags,
    upsert_potential_user,
)
from backend.repositories.member_repo import promote_potential_user

logger = logging.getLogger(__name__)


async def api_get_potential_users(request: web.Request) -> web.Response:
    """获取潜在用户列表 (支持分页与画像/关键词过滤)"""
    try:
        limit = min(int(request.query.get('limit', 50)), 500)
        offset = int(request.query.get('offset', 0))
        keyword = request.query.get('keyword')
        tag = request.query.get('tag')
        group_id = request.query.get('group_id')
        if group_id:
            group_id = int(group_id)

        result = await database.get_potential_users(
            limit=limit,
            offset=offset,
            keyword=keyword,
            tag=tag,
            group_id=group_id
        )
        return web.json_response({"status": "ok", "data": result['users'], "total": result['total']})
    except Exception as e:
        logger.error(f"获取潜在用户失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_sync_potential_users(request: web.Request) -> web.Response:
    """手动触发潜在用户回填同步"""
    try:
        limit = int(request.query.get('limit', 2000))
        # 调用 web_admin.py 中的同步函数
        from web_admin import _sync_potential_users_from_group_messages
        stats = await _sync_potential_users_from_group_messages(limit=limit)
        return web.json_response({
            'status': 'ok',
            'summary': f"同步完成：扫描 {stats['scanned_messages']} 条消息，命中 {stats['matched_messages']} 条，更新 {stats['upserted_users']} 位潜在用户。",
            'data': stats
        })
    except Exception as e:
        logger.error(f"同步潜在用户失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_potential_tags(request: web.Request) -> web.Response:
    """获取画像标签列表"""
    try:
        tags = await get_potential_tags()
        
        return web.json_response({
            'status': 'ok',
            'data': tags,
            'total': len(tags)
        })
    except Exception as e:
        logger.error(f"获取标签失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_promote_potential_user(request: web.Request) -> web.Response:
    """一键转正潜在用户"""
    try:
        user_id = int(request.match_info['user_id'])
        
        success = await promote_potential_user(user_id)
        
        if success:
            return web.json_response({'status': 'ok'})
        else:
            return web.json_response(
                {'status': 'error', 'message': '转正失败'},
                status=500
            )
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '用户 ID 无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"转正失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )
