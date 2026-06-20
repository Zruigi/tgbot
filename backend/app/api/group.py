# -*- coding: utf-8 -*-
"""
群组管理 API - 群组列表、成员管理、验证配置
"""
from aiohttp import web
import logging
from backend.repositories.group_repo import (
    get_tracked_groups_with_stats,
    is_group_listen_enabled,
    set_group_listen_status,
    verify_group_ownership,
)
from backend.repositories.verify_repo import (
    get_group_verify_config,
    save_group_verify_config,
    get_verify_stats,
    get_verify_growth,
)
from backend.repositories.member_repo import get_group_members_live
from backend.repositories.audit_repo import log_tenant_isolation_event

logger = logging.getLogger(__name__)


async def api_list_groups(request: web.Request) -> web.Response:
    """获取群组列表"""
    try:
        page = int(request.query.get('page', 1))
        size = int(request.query.get('size', 20))
        
        tenant_id = request.get('tenant_id', '')
        role = request.get('role', 'merchant_owner')
        effective_tenant = tenant_id if role == 'merchant_owner' else None
        
        groups_data = await get_tracked_groups_with_stats(tenant_id=effective_tenant)
        
        # 分页
        total = len(groups_data)
        start = (page - 1) * size
        end = start + size
        paginated = groups_data[start:end]
        
        return web.json_response({
            'status': 'ok',
            'data': paginated,
            'total': total
        })
    except Exception as e:
        logger.error(f"获取群组列表失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_toggle_group_listen(request: web.Request) -> web.Response:
    """切换群组监听状态"""
    try:
        data = await request.json()
        group_id = int(data['group_id'])
        enabled = bool(data.get('enabled', False))
        
        tenant_id = request.get('tenant_id', '')
        role = request.get('role', 'merchant_owner')
        
        if role == 'merchant_owner':
            group = await verify_group_ownership(group_id, tenant_id)
            if not group:
                return web.json_response(
                    {'status': 'error', 'message': '无权操作该群组'},
                    status=403
                )
        
        await set_group_listen_status(group_id, enabled)
        
        return web.json_response({
            'status': 'ok',
            'new_status': 'enabled' if enabled else 'disabled'
        })
    except (KeyError, ValueError) as e:
        return web.json_response(
            {'status': 'error', 'message': '参数无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"切换监听状态失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_group_members(request: web.Request) -> web.Response:
    """获取群成员列表"""
    try:
        group_id = int(request.query.get('group_id', 0))
        if not group_id:
            return web.json_response(
                {'status': 'error', 'message': '群组 ID 无效'},
                status=400
            )
        
        # TODO: 需要传入 bot 实例
        members = []  # await get_group_members_live(group_id, bot=None)
        
        return web.json_response({
            'status': 'ok',
            'data': members
        })
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '参数无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"获取群成员失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_group_messages(request: web.Request) -> web.Response:
    """获取群消息历史"""
    try:
        from backend.repositories.group_repo import get_group_messages
        group_id = int(request.query.get('group_id', 0))
        limit = int(request.query.get('limit', 50))
        offset = int(request.query.get('offset', 0))
        
        if not group_id:
            return web.json_response(
                {'status': 'error', 'message': '群组 ID 无效'},
                status=400
            )
        
        tenant_id = request.get('tenant_id', '')
        role = request.get('role', 'merchant_owner')
        
        if role == 'merchant_owner':
            group = await verify_group_ownership(group_id, tenant_id)
            if not group:
                await log_tenant_isolation_event(
                    tenant_id=tenant_id,
                    action='cross_tenant_access_denied',
                    resource_type='group',
                    resource_id=str(group_id),
                    operator_role=role,
                    request_path=request.path,
                )
                return web.json_response(
                    {'status': 'error', 'message': '无权访问该群组'},
                    status=403
                )
        
        messages = await get_group_messages(group_id, limit=limit, offset=offset)
        
        return web.json_response({
            'status': 'ok',
            'data': messages,
            'total': len(messages)
        })
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '参数无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"获取群消息失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_group_verify_config(request: web.Request) -> web.Response:
    """获取群组验证配置"""
    try:
        group_id = int(request.query.get('group_id', 0))
        if not group_id:
            return web.json_response(
                {'status': 'error', 'message': '群组 ID 无效'},
                status=400
            )
        
        tenant_id = request.get('tenant_id', '')
        role = request.get('role', 'merchant_owner')
        
        if role == 'merchant_owner':
            group = await verify_group_ownership(group_id, tenant_id)
            if not group:
                return web.json_response(
                    {'status': 'error', 'message': '无权访问该群组'},
                    status=403
                )
        
        config = await get_group_verify_config(group_id)
        
        return web.json_response({
            'status': 'ok',
            'data': config
        })
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '参数无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"获取验证配置失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_save_group_verify_config(request: web.Request) -> web.Response:
    """保存群组验证配置"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id', 0))
        
        if not group_id:
            return web.json_response(
                {'status': 'error', 'message': '群组 ID 无效'},
                status=400
            )
        
        tenant_id = request.get('tenant_id', '')
        role = request.get('role', 'merchant_owner')
        
        if role == 'merchant_owner':
            group = await verify_group_ownership(group_id, tenant_id)
            if not group:
                return web.json_response(
                    {'status': 'error', 'message': '无权操作该群组'},
                    status=403
                )
        
        config = {
            'verify_enabled': data.get('verify_enabled', False),
            'verify_type': data.get('verify_type', 'captcha'),
            'verify_timeout': data.get('verify_timeout', 120),
            'verify_channel_id': data.get('verify_channel_id'),
            'verify_questions': data.get('verify_questions', []),
            'verify_msg_template': data.get('verify_msg_template'),
        }
        
        await save_group_verify_config(group_id, config)
        
        return web.json_response({'status': 'ok'})
    except (KeyError, ValueError) as e:
        return web.json_response(
            {'status': 'error', 'message': '参数无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"保存验证配置失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_verify_stats(request: web.Request) -> web.Response:
    """获取验证统计"""
    try:
        group_id = int(request.query.get('group_id', 0))
        stats = await get_verify_stats(group_id)
        return web.json_response({
            'status': 'ok',
            'data': stats
        })
    except Exception as e:
        logger.error(f"获取统计失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_verify_growth(request: web.Request) -> web.Response:
    """获取验证增长数据"""
    try:
        group_id = int(request.query.get('group_id', 0)) or None
        period = request.query.get('period', 'day')
        
        growth = await get_verify_growth(group_id, period)
        
        return web.json_response({
            'status': 'ok',
            'data': growth
        })
    except Exception as e:
        logger.error(f"获取增长数据失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )
