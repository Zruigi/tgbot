# -*- coding: utf-8 -*-
"""
链上监听 API - 钱包监听管理、盈亏统计
"""
from aiohttp import web
import logging
from backend.repositories.monitor_repo import (
    get_monitor_pnl_summary,
    batch_update_monitor_transaction_meta,
    get_monitor_transactions_for_export
)

logger = logging.getLogger(__name__)


async def api_list_monitors_wrapper(request: web.Request) -> web.Response:
    """获取监听列表（包装器）"""
    # TODO: 实现实际的监听列表逻辑
    return web.json_response({'status': 'ok', 'data': []})


async def api_create_monitor_wrapper(request: web.Request) -> web.Response:
    """创建监听（包装器）"""
    # TODO: 实现实际的创建监听逻辑
    return web.json_response({'status': 'ok'})


async def api_delete_monitor_wrapper(request: web.Request) -> web.Response:
    """删除监听（包装器）"""
    # TODO: 实现实际的删除监听逻辑
    return web.json_response({'status': 'ok'})


async def api_update_monitor_remark_wrapper(request: web.Request) -> web.Response:
    """更新监听备注（包装器）"""
    # TODO: 实现实际的更新备注逻辑
    return web.json_response({'status': 'ok'})


async def api_monitor_pnl_wrapper(request: web.Request) -> web.Response:
    """获取盈亏统计"""
    try:
        data = await request.json() if request.can_json() else {}
        days = int(data.get('days', 7))
        
        summary = await get_monitor_pnl_summary(days=days)
        
        return web.json_response({
            'status': 'ok',
            'data': summary
        })
    except Exception as e:
        logger.error(f"获取盈亏统计失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_update_tx_meta_wrapper(request: web.Request) -> web.Response:
    """更新交易元数据"""
    # TODO: 实现实际逻辑
    return web.json_response({'status': 'ok'})


async def api_batch_update_tx_meta_wrapper(request: web.Request) -> web.Response:
    """批量更新交易元数据"""
    try:
        data = await request.json()
        # 调用 repository 层的批量更新函数
        await batch_update_monitor_transaction_meta(data.get('transactions', []))
        return web.json_response({'status': 'ok'})
    except Exception as e:
        logger.error(f"批量更新失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_export_wrapper(request: web.Request) -> web.Response:
    """导出交易记录"""
    try:
        # 调用 repository 层的导出函数
        transactions = await get_monitor_transactions_for_export()
        return web.json_response({
            'status': 'ok',
            'data': transactions
        })
    except Exception as e:
        logger.error(f"导出失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_monitor_stats_wrapper(request: web.Request) -> web.Response:
    """获取监听统计"""
    # TODO: 实现实际的统计逻辑
    return web.json_response({'status': 'ok', 'data': {}})
