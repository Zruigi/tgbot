# -*- coding: utf-8 -*-
"""
Web3 企业级财务 ERP - API 路由处理器
"""
import logging
from aiohttp import web
import json
import config as _config
from finance_core import (
    init_finance_db, init_default_chart_of_accounts,
    create_finance_ledger, reconcile_ledger, reverse_ledger_entry,
    get_unreconciled_ledgers, get_reconciliation_summary, get_audit_logs
)
from finance_reports import (
    collect_finance_summary, collect_ledger_details,
    generate_pdf_report, generate_excel_report
)
from database import (
    get_finance_summary_by_currency,
    get_finance_unreconciled_with_fx,
    reconcile_finance_ledger,
    get_finance_category_tree,
)

logger = logging.getLogger(__name__)


def _get_request_admin_id(request: web.Request) -> int:
    """
    从请求上下文提取管理员 ID。
    当前系统为单管理员架构，统一使用 config.ADMIN_ID。
    修复 P0: 财务数据无租户隔离 —— 禁止从请求体获取 admin_userid（可被伪造）。
    """
    return _config.ADMIN_ID


async def _check_monitor_ownership(monitor_id: int, admin_id: int) -> bool:
    """
    校验 monitor_id 是否属于当前管理员（租户隔离）。
    修复 P0: 监听管理无所有权验证。
    """
    if not monitor_id:
        return True
    try:
        import aiosqlite
        async with aiosqlite.connect(_config.DB_NAME, timeout=10) as db:
            cursor = await db.execute(
                "SELECT user_id FROM address_monitors WHERE monitor_id = ? AND is_active = 1",
                (monitor_id,)
            )
            row = await cursor.fetchone()
            if not row:
                return False
            # 单管理员系统：user_id 须与 ADMIN_ID 一致
            return int(row[0]) == admin_id
    except Exception as e:
        logger.error(f"租户隔离校验失败: {e}")
        return False

async def init_finance_routes(app: web.Application):
    """注册财务 API 路由"""
    # v1 规范路由 - 财务相关
    app.router.add_get('/api/v1/monitors/finance/categories', get_chart_of_accounts_handler)
    app.router.add_get('/api/v1/monitors/finance/account_categories', get_chart_of_accounts_handler)
    app.router.add_post('/api/v1/monitors/finance/ledger', create_ledger_handler)
    app.router.add_post('/api/v1/monitors/finance/reconcile', reconcile_handler)
    app.router.add_post('/api/v1/monitors/finance/reverse', reverse_handler)
    app.router.add_get('/api/v1/monitors/finance/unreconciled', get_unreconciled_handler)
    app.router.add_get('/api/v1/monitors/finance/summary', get_summary_handler)
    app.router.add_get('/api/v1/monitors/finance/reports/generate', generate_report_handler)
    app.router.add_get('/api/v1/monitors/finance/audit_logs', get_audit_logs_handler)

    # 无 v1 前缀兼容路由（修复前端 404）
    app.router.add_get('/api/monitors/finance/categories', get_chart_of_accounts_handler)
    app.router.add_get('/api/monitors/finance/unreconciled', get_unreconciled_handler)
    app.router.add_get('/api/monitors/finance/summary', get_summary_handler)
    app.router.add_post('/api/monitors/finance/reconcile', reconcile_handler)
    
    # v1 规范路由 - 链上监听相关
    app.router.add_get('/api/v1/monitors/finance/monitors', get_monitors_handler)
    app.router.add_post('/api/v1/monitors/finance/monitors', add_monitor_handler)
    app.router.add_delete('/api/v1/monitors/finance/monitors/{monitor_id}', delete_monitor_handler)
    app.router.add_patch('/api/v1/monitors/finance/monitors/{monitor_id}', update_monitor_handler)
    app.router.add_get('/api/v1/monitors/finance/transactions', get_transactions_handler)
    
    # 兼容旧路由
    app.router.add_get('/api/finance/categories', get_chart_of_accounts_handler)
    app.router.add_get('/api/finance/chart_of_accounts', get_chart_of_accounts_handler)
    app.router.add_post('/api/finance/ledger', create_ledger_handler)
    app.router.add_post('/api/finance/reconcile', reconcile_handler)
    app.router.add_post('/api/finance/reverse', reverse_handler)
    app.router.add_get('/api/finance/unreconciled', get_unreconciled_handler)
    app.router.add_get('/api/finance/summary', get_summary_handler)
    app.router.add_get('/api/finance/reports/generate', generate_report_handler)
    app.router.add_get('/api/finance/audit_logs', get_audit_logs_handler)
    
    logger.info("✅ 财务 API 路由已注册（v1 + 兼容旧路由）")

async def get_chart_of_accounts_handler(request: web.Request) -> web.Response:
    """获取会计科目树"""
    try:
        categories = await get_finance_category_tree()
        return web.json_response({
            "status": "ok",
            "data": categories
        })
    except Exception as e:
        logger.error(f"获取科目树失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def create_ledger_handler(request: web.Request) -> web.Response:
    """创建总账记录"""
    try:
        data = await request.json()
        
        ledger_id = await create_finance_ledger(
            monitor_id=data.get("monitor_id"),
            user_id=data.get("user_id"),
            account_id=data.get("account_id"),
            direction=data.get("direction"),
            amount=float(data.get("amount")),
            token_symbol=data.get("token_symbol"),
            base_fiat_value=float(data.get("base_fiat_value")),
            fiat_currency=data.get("fiat_currency", "USD"),
            exchange_rate=float(data.get("exchange_rate", 1.0)),
            counterparty=data.get("counterparty"),
            description=data.get("description"),
            tx_hash=data.get("tx_hash"),
            tx_timestamp=data.get("tx_timestamp")
        )
        
        return web.json_response({
            "status": "ok",
            "ledger_id": ledger_id
        })
    except Exception as e:
        logger.error(f"创建总账失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=400)

async def reconcile_handler(request: web.Request) -> web.Response:
    """核销总账记录（安全加固：admin_userid 从服务端获取，禁止客户端伪造）"""
    try:
        data = await request.json()
        ledger_id = data.get("ledger_id")
        account_code = data.get("account_code")
        amount = data.get("amount")

        if not ledger_id or not account_code:
            return web.json_response({
                "status": "error",
                "message": "缺少必要参数: ledger_id, account_code"
            }, status=400)

        admin_userid = _get_request_admin_id(request)
        ok = await reconcile_finance_ledger(
            ledger_id=str(ledger_id),
            account_code=str(account_code),
            admin_userid=admin_userid,
            amount=float(amount) if amount is not None else None,
            ip_address=request.remote,
        )
        if not ok:
            return web.json_response({"status": "error", "message": "账目记录不存在或无权操作"}, status=404)

        return web.json_response({
            "status": "ok",
            "success": True
        })
    except Exception as e:
        logger.error(f"核销失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=400)

async def reverse_handler(request: web.Request) -> web.Response:
    """红字冲销（安全加固：admin_userid 从服务端获取，禁止客户端伪造）"""
    try:
        data = await request.json()
        ledger_id = data.get("ledger_id")

        if not ledger_id:
            return web.json_response({"status": "error", "message": "缺少必要参数: ledger_id"}, status=400)

        # 修复 P0: 租户隔离 —— admin_userid 从服务端获取，禁止客户端传入
        admin_userid = _get_request_admin_id(request)

        # 校验 ledger 归属（租户隔离）
        from config import DB_NAME
        import aiosqlite
        async with aiosqlite.connect(DB_NAME) as db:
            cursor = await db.execute(
                "SELECT user_id FROM finance_ledgers WHERE ledger_id = ?",
                (ledger_id,)
            )
            ledger_row = await cursor.fetchone()
            if not ledger_row:
                return web.json_response({"status": "error", "message": "账目记录不存在"}, status=404)
            if int(ledger_row[0]) != admin_userid:
                logger.warning(f"租户隔离拦截: admin {admin_userid} 尝试冲销不属于自己的账目 {ledger_id}")
                return web.json_response({"status": "error", "message": "无权操作此账目"}, status=403)
        
        reverse_id = await reverse_ledger_entry(
            original_ledger_id=ledger_id,
            admin_userid=admin_userid,
            reason=data.get("reason"),
            ip_address=request.remote  # 修复 Medium: 审计日志 IP 未记录
        )
        
        return web.json_response({
            "status": "ok" if reverse_id else "error",
            "reverse_ledger_id": reverse_id
        })
    except Exception as e:
        logger.error(f"红字冲销失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=400)

async def get_unreconciled_handler(request: web.Request) -> web.Response:
    """获取未核销流水列表（含法币换算）"""
    try:
        admin_userid = _get_request_admin_id(request)
        monitor_id = request.query.get('monitor_id')
        rows = await get_finance_unreconciled_with_fx(
            user_id=admin_userid,
            monitor_id=int(monitor_id) if monitor_id else None,
        )
        return web.json_response({
            "status": "ok",
            "data": rows,
            "count": len(rows)
        })
    except Exception as e:
        logger.error(f"获取未核销流水失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def get_summary_handler(request: web.Request) -> web.Response:
    """获取多币种汇总数据"""
    try:
        admin_userid = _get_request_admin_id(request)
        summary = await get_finance_summary_by_currency(admin_userid)
        return web.json_response({
            "status": "ok",
            "data": summary
        })
    except Exception as e:
        logger.error(f"获取摘要失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def generate_report_handler(request: web.Request) -> web.Response:
    """生成财务报表"""
    try:
        month = request.query.get("month")
        report_format = request.query.get("format", "pdf")  # pdf 或 excel
        
        # 收集数据
        summary = await collect_finance_summary(month)
        details = await collect_ledger_details(month, limit=500)
        
        # 生成报表
        if report_format == "excel":
            report_data = generate_excel_report(summary, details)
            content_type = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            filename = f"财务报表_{month}.xlsx"
        else:
            report_data = generate_pdf_report(summary, details)
            content_type = "application/pdf"
            filename = f"财务报表_{month}.pdf"
        
        if not report_data:
            return web.json_response({
                "status": "error",
                "message": "报表生成失败，请检查依赖库是否已安装"
            }, status=500)
        
        return web.Response(
            body=report_data,
            content_type=content_type,
            headers={
                'Content-Disposition': f'attachment; filename="{filename}"'
            }
        )
    except Exception as e:
        logger.error(f"生成报表失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def get_audit_logs_handler(request: web.Request) -> web.Response:
    """获取审计日志"""
    try:
        days = int(request.query.get("days", 30))
        action = request.query.get("action")
        target_table = request.query.get("target_table")
        
        logs = await get_audit_logs(days, action, target_table)
        
        # 转换字段格式以匹配前端期望
        formatted_logs = []
        for log in logs:
            formatted_logs.append({
                "audit_id": log.get("audit_id"),
                "timestamp": log.get("timestamp"),
                "admin_userid": log.get("admin_userid"),
                "action": log.get("action"),
                "target_table": log.get("target_table")
            })
        
        return web.json_response({
            "status": "ok",
            "data": formatted_logs,
            "count": len(formatted_logs)
        })
    except Exception as e:
        logger.error(f"获取审计日志失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

# ===== 链上监听 API =====

async def get_monitors_handler(request: web.Request) -> web.Response:
    """获取链上监听列表"""
    try:
        from config import DB_NAME
        import aiosqlite
        
        async with aiosqlite.connect(DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            cursor = await db.execute('''
                SELECT monitor_id, user_id, address, network, remark, 
                       inflow_20, outflow_20, profit_20
                FROM address_monitors
                WHERE is_active = 1
                ORDER BY monitor_id DESC
            ''')
            rows = await cursor.fetchall()
            monitors = [dict(row) for row in rows]
        
        return web.json_response({
            "status": "ok",
            "data": monitors,
            "count": len(monitors)
        })
    except Exception as e:
        logger.error(f"获取监听列表失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def add_monitor_handler(request: web.Request) -> web.Response:
    """新增链上监听（安全加固：地址格式验证 + 所有权绑定）"""
    try:
        data = await request.json()
        address = (data.get('address') or '').strip()
        network = (data.get('network') or '').strip().lower()

        if not address or not network:
            return web.json_response({"status": "error", "message": "缺少必要参数: address, network"}, status=400)

        # 修复 P0: 地址输入无格式验证
        import re as _re
        _TRON_RE = _re.compile(r'^T[1-9A-HJ-NP-Za-km-z]{33}$')
        _ETH_RE = _re.compile(r'^0x[0-9a-fA-F]{40}$')
        net = network.lower()
        valid = False
        if net in ('trc20', 'tron'):
            valid = bool(_TRON_RE.match(address))
        elif net in ('bsc', 'eth', 'erc20'):
            valid = bool(_ETH_RE.match(address))
        if not valid:
            logger.warning(f"非法地址格式拦截 (finance_api): address={address!r} network={network}")
            return web.json_response({"status": "error", "message": "钱包地址格式不合法"}, status=400)

        # 修复 P0: 租户隔离 —— user_id 从服务端获取，禁止客户端传入
        admin_id = _get_request_admin_id(request)
        from config import DB_NAME
        import aiosqlite

        async with aiosqlite.connect(DB_NAME, timeout=10) as db:
            cursor = await db.execute('''
                INSERT INTO address_monitors (user_id, address, network, is_active)
                VALUES (?, ?, ?, 1)
            ''', (admin_id, address, network))
            await db.commit()
            monitor_id = cursor.lastrowid

        return web.json_response({
            "status": "ok",
            "monitor_id": monitor_id
        })
    except Exception as e:
        logger.error(f"新增监听失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=400)

async def delete_monitor_handler(request: web.Request) -> web.Response:
    """删除链上监听（安全加固：校验所有权，防止越权删除）"""
    try:
        monitor_id = int(request.match_info['monitor_id'])
        admin_id = _get_request_admin_id(request)

        # 修复 P0: 监听管理无所有权验证
        if not await _check_monitor_ownership(monitor_id, admin_id):
            logger.warning(f"租户隔离拦截: admin {admin_id} 尝试删除不属于自己的 monitor {monitor_id}")
            return web.json_response({"status": "error", "message": "无权操作此监听"}, status=403)

        from config import DB_NAME
        import aiosqlite
        
        async with aiosqlite.connect(DB_NAME, timeout=10) as db:
            await db.execute('''
                UPDATE address_monitors SET is_active = 0 WHERE monitor_id = ?
            ''', (monitor_id,))
            await db.commit()
        
        return web.json_response({"status": "ok", "success": True})
    except Exception as e:
        logger.error(f"删除监听失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=400)

async def update_monitor_handler(request: web.Request) -> web.Response:
    """更新监听备注（安全加固：校验所有权，防止越权修改）"""
    try:
        monitor_id = int(request.match_info['monitor_id'])
        admin_id = _get_request_admin_id(request)

        # 修复 P0: 监听管理无所有权验证
        if not await _check_monitor_ownership(monitor_id, admin_id):
            logger.warning(f"租户隔离拦截: admin {admin_id} 尝试修改不属于自己的 monitor {monitor_id}")
            return web.json_response({"status": "error", "message": "无权操作此监听"}, status=403)

        data = await request.json()
        from config import DB_NAME
        import aiosqlite
        
        async with aiosqlite.connect(DB_NAME, timeout=10) as db:
            await db.execute('''
                UPDATE address_monitors SET remark = ? WHERE monitor_id = ?
            ''', (data.get('remark'), monitor_id))
            await db.commit()
        
        return web.json_response({"status": "ok", "success": True})
    except Exception as e:
        logger.error(f"更新监听失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=400)

async def get_transactions_handler(request: web.Request) -> web.Response:
    """获取交易列表（待对账流水）"""
    try:
        monitor_id = request.query.get('monitor_id')
        from config import DB_NAME
        import aiosqlite
        
        query = '''
            SELECT ledger_id, monitor_id, tx_hash, direction, amount, token_symbol,
                   base_fiat_value, created_at, reconciled_status
            FROM finance_ledgers
            WHERE reconciled_status = 'UNRECONCILED'
        '''
        params = []
        
        if monitor_id:
            query += " AND monitor_id = ?"
            params.append(int(monitor_id))
        
        query += " ORDER BY created_at DESC LIMIT 500"
        
        async with aiosqlite.connect(DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            cursor = await db.execute(query, params)
            rows = await cursor.fetchall()
            transactions = [dict(row) for row in rows]
        
        return web.json_response({
            "status": "ok",
            "data": transactions,
            "count": len(transactions)
        })
    except Exception as e:
        logger.error(f"获取交易列表失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)
