# -*- coding: utf-8 -*-
"""
企业财务 API - 财务报表、对账、统计
"""
from aiohttp import web
import logging
from backend.repositories.finance_repo import (
    get_finance_summary_by_currency,
    reconcile_finance_ledger,
    get_finance_category_tree
)
from backend.app.services.finance_calc import FinanceCalcService

logger = logging.getLogger(__name__)


async def api_get_finance_summary(request: web.Request) -> web.Response:
    """获取财务摘要"""
    try:
        data = await request.json() if request.can_json() else {}
        currency = data.get('currency', 'USDT')
        days = int(data.get('days', 30))
        
        summary = await get_finance_summary_by_currency(currency=currency, days=days)
        
        return web.json_response({
            'status': 'ok',
            'data': summary
        })
    except Exception as e:
        logger.error(f"获取财务摘要失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_reconcile_finance(request: web.Request) -> web.Response:
    """执行财务对账"""
    try:
        data = await request.json()
        ledger_type = data.get('ledger_type', 'wallet')
        start_date = data.get('start_date')
        end_date = data.get('end_date')
        
        result = await reconcile_finance_ledger(
            ledger_type=ledger_type,
            start_date=start_date,
            end_date=end_date
        )
        
        return web.json_response({
            'status': 'ok',
            'data': result
        })
    except Exception as e:
        logger.error(f"财务对账失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_get_finance_categories(request: web.Request) -> web.Response:
    """获取财务分类树"""
    try:
        categories = await get_finance_category_tree()
        
        return web.json_response({
            'status': 'ok',
            'data': categories
        })
    except Exception as e:
        logger.error(f"获取分类树失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_calculate_pnl(request: web.Request) -> web.Response:
    """计算盈亏（使用 FinanceCalcService）"""
    try:
        from decimal import Decimal
        data = await request.json()
        
        calc_service = FinanceCalcService()
        result = calc_service.calculate_pnl(
            buy_price=Decimal(str(data.get('buy_price', 0))),
            sell_price=Decimal(str(data.get('sell_price', 0))),
            quantity=Decimal(str(data.get('quantity', 0))),
            fee_rate=Decimal(str(data.get('fee_rate', '0.001')))
        )
        
        # 转换为 JSON 可序列化格式
        serializable_result = {
            k: float(v) for k, v in result.items()
        }
        
        return web.json_response({
            'status': 'ok',
            'data': serializable_result
        })
    except Exception as e:
        logger.error(f"盈亏计算失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_generate_finance_report(request: web.Request) -> web.Response:
    """生成财务报表"""
    try:
        from decimal import Decimal
        data = await request.json()
        
        transactions = data.get('transactions', [])
        
        calc_service = FinanceCalcService()
        summary = calc_service.generate_summary(transactions)
        
        # 转换 Decimal 为 float
        serializable_summary = {}
        for k, v in summary.items():
            if isinstance(v, Decimal):
                serializable_summary[k] = float(v)
            elif isinstance(v, int):
                serializable_summary[k] = v
            else:
                serializable_summary[k] = v
        
        return web.json_response({
            'status': 'ok',
            'data': serializable_summary
        })
    except Exception as e:
        logger.error(f"生成财务报表失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)
