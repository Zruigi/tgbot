# -*- coding: utf-8 -*-
"""
钱包监控 Repository - 链上监听与交易相关数据库操作
"""
import aiosqlite
import logging
from datetime import datetime, timedelta
import calendar
from typing import Optional, List, Dict, Any, Union
from config import DB_NAME, ADMIN_ID

logger = logging.getLogger(__name__)


async def get_active_monitors() -> List[Dict[str, Any]]:
    """获取所有活跃的钱包监听任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM address_monitors WHERE is_active = 1") as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def update_monitor_timestamp(monitor_id: int, timestamp: int) -> None:
    """更新钱包监听的最后交易时间戳"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE address_monitors SET last_tx_timestamp = ? WHERE monitor_id = ?", (timestamp, monitor_id))
        await db.commit()


async def update_monitor_risk(monitor_id: int, risk_level: str = 'normal', risk_note: Optional[str] = None) -> None:
    """更新钱包风险等级"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE address_monitors SET risk_level = ?, risk_note = ? WHERE monitor_id = ?",
            (risk_level, risk_note, monitor_id),
        )
        await db.commit()


async def update_monitor_remark(monitor_id: int, remark: str = '') -> bool:
    """更新钱包备注"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE address_monitors SET remark = ? WHERE monitor_id = ?",
            (remark or '', monitor_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def get_recent_monitor_transactions(monitor_id: int, direction: Optional[str] = None, limit: int = 20) -> List[Dict[str, Any]]:
    """获取最近的交易记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM monitor_transactions WHERE monitor_id = ?"
        params: List[Union[int, str]] = [monitor_id]
        if direction:
            sql += " AND direction = ?"
            params.append(direction)
        sql += " ORDER BY tx_timestamp DESC LIMIT ?"
        params.append(limit)
        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def save_monitor_transaction(
    tx_id: str, monitor_id: int, user_id: int, address: str, network: str,
    direction: str, amount: float, token_symbol: str, tx_timestamp: int,
    counterparty: Optional[str] = None, risk_level: str = 'normal',
    risk_note: Optional[str] = None
) -> bool:
    """保存交易记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """INSERT OR IGNORE INTO monitor_transactions
               (tx_id, monitor_id, user_id, address, network, direction, amount, token_symbol, tx_timestamp, counterparty, risk_level, risk_note)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (tx_id, monitor_id, user_id, address, network, direction, amount, token_symbol, tx_timestamp, counterparty, risk_level, risk_note),
        )
        await db.commit()
        return cur.rowcount > 0


def _period_boundaries_ms(period: str, previous: bool = False) -> tuple[int, int]:
    """计算时间周期边界（毫秒）"""
    now = datetime.now()

    if period == 'day':
        start = datetime(now.year, now.month, now.day)
        end = start + timedelta(days=1)
        if previous:
            start = start - timedelta(days=1)
            end = end - timedelta(days=1)
    elif period == 'week':
        start = datetime(now.year, now.month, now.day) - timedelta(days=now.weekday())
        end = start + timedelta(days=7)
        if previous:
            start = start - timedelta(days=7)
            end = end - timedelta(days=7)
    else:  # month
        start = datetime(now.year, now.month, 1)
        if now.month == 12:
            end = datetime(now.year + 1, 1, 1)
        else:
            end = datetime(now.year, now.month + 1, 1)
        if previous:
            prev_end = start
            prev_year = start.year if start.month > 1 else start.year - 1
            prev_month = start.month - 1 if start.month > 1 else 12
            prev_start = datetime(prev_year, prev_month, 1)
            start = prev_start
            end = prev_end

    return int(start.timestamp() * 1000), int(end.timestamp() * 1000)


def _build_monitor_scope(
    addresses: Optional[List[str]] = None,
    monitor_ids: Optional[List[int]] = None,
    user_id: Optional[int] = None
) -> tuple[str, List[Union[int, str]]]:
    """构建监听范围 SQL"""
    where = []
    params: List[Union[int, str]] = []

    if addresses:
        placeholders = ','.join(['?'] * len(addresses))
        where.append(f"address IN ({placeholders})")
        params.extend(addresses)

    if monitor_ids:
        placeholders = ','.join(['?'] * len(monitor_ids))
        where.append(f"monitor_id IN ({placeholders})")
        params.extend([int(x) for x in monitor_ids])

    if user_id is not None:
        where.append("user_id = ?")
        params.append(int(user_id))

    return (" AND " + " AND ".join(where)) if where else "", params


async def get_monitor_pnl_summary(
    period: str = 'day',
    user_id: Optional[int] = None,
    addresses: Optional[List[str]] = None,
    monitor_ids: Optional[List[int]] = None,
    previous: bool = False
) -> Dict[str, float]:
    """获取盈亏汇总"""
    start_ms, end_ms = _period_boundaries_ms(period, previous=previous)
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)

    sql = f"""
        SELECT
            SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END) AS inflow,
            SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END) AS outflow
        FROM monitor_transactions
        WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql}
    """

    params: List[Union[int, str]] = [start_ms, end_ms]
    params.extend(scope_params)

    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(sql, tuple(params)) as cursor:
            row = await cursor.fetchone()
            inflow = float(row[0] or 0)
            outflow = float(row[1] or 0)
            return {
                'inflow': inflow,
                'outflow': outflow,
                'profit': inflow - outflow,
                'start_ms': start_ms,
                'end_ms': end_ms
            }


async def get_monitor_period_series(
    period: str = 'day',
    user_id: Optional[int] = None,
    addresses: Optional[List[str]] = None,
    monitor_ids: Optional[List[int]] = None
) -> List[Dict[str, Any]]:
    """获取周期序列数据"""
    now = datetime.now()
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)

    if period == 'day':
        point_count = 24
        start = datetime(now.year, now.month, now.day)
        step = timedelta(hours=1)
        fmt = '%H:00'
    elif period == 'week':
        point_count = 7
        start = datetime(now.year, now.month, now.day) - timedelta(days=now.weekday())
        step = timedelta(days=1)
        fmt = '%m-%d'
    else:  # month
        point_count = calendar.monthrange(now.year, now.month)[1]
        start = datetime(now.year, now.month, 1)
        step = timedelta(days=1)
        fmt = '%m-%d'

    result: List[Dict[str, Any]] = []
    async with aiosqlite.connect(DB_NAME) as db:
        for idx in range(point_count):
            b_start = start + step * idx
            b_end = b_start + step
            b_start_ms = int(b_start.timestamp() * 1000)
            b_end_ms = int(b_end.timestamp() * 1000)

            sql = f"""
                SELECT
                    SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END) AS inflow,
                    SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END) AS outflow
                FROM monitor_transactions
                WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql}
            """
            params: List[Union[int, str]] = [b_start_ms, b_end_ms]
            params.extend(scope_params)
            async with db.execute(sql, tuple(params)) as cursor:
                row = await cursor.fetchone()
                inflow = float(row[0] or 0)
                outflow = float(row[1] or 0)
            result.append({
                'label': b_start.strftime(fmt),
                'inflow': inflow,
                'outflow': outflow,
                'profit': inflow - outflow,
                'start_ms': b_start_ms,
                'end_ms': b_end_ms,
            })
    return result


async def update_monitor_transaction_meta(
    tx_id: str,
    income_reason: Optional[str] = None,
    expense_reason: Optional[str] = None,
    payer: Optional[str] = None,
    payee: Optional[str] = None
) -> bool:
    """更新交易业务字段"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE monitor_transactions
            SET income_reason = ?, expense_reason = ?, payer = ?, payee = ?
            WHERE tx_id = ?
            """,
            (income_reason, expense_reason, payer, payee, tx_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def batch_update_monitor_transaction_meta(items: List[Dict[str, Any]]) -> int:
    """批量更新交易业务字段"""
    updated = 0
    async with aiosqlite.connect(DB_NAME) as db:
        for it in items:
            tx_id = (it.get('tx_id') or '').strip()
            if not tx_id:
                continue
            cur = await db.execute(
                """
                UPDATE monitor_transactions
                SET income_reason = ?, expense_reason = ?, payer = ?, payee = ?
                WHERE tx_id = ?
                """,
                (
                    (it.get('income_reason') or '').strip() or None,
                    (it.get('expense_reason') or '').strip() or None,
                    (it.get('payer') or '').strip() or None,
                    (it.get('payee') or '').strip() or None,
                    tx_id,
                ),
            )
            if cur.rowcount > 0:
                updated += 1
        await db.commit()
    return updated


async def get_monitor_transactions_for_export(
    start_ms: int,
    end_ms: int,
    user_id: Optional[int] = None,
    addresses: Optional[List[str]] = None,
    monitor_ids: Optional[List[int]] = None,
    annotated_only: bool = False
) -> List[Dict[str, Any]]:
    """获取导出用交易列表"""
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)
    annotated_sql = ""
    if annotated_only:
        annotated_sql = " AND (COALESCE(income_reason, '') <> '' OR COALESCE(expense_reason, '') <> '' OR COALESCE(payer, '') <> '' OR COALESCE(payee, '') <> '')"
    
    sql = f"""
        SELECT *
        FROM monitor_transactions
        WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql} {annotated_sql}
        ORDER BY tx_timestamp ASC, tx_id ASC
    """
    params: List[Union[int, str]] = [start_ms, end_ms]
    params.extend(scope_params)
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def get_monitor_transactions_list(
    period: str = 'month',
    user_id: Optional[int] = None,
    addresses: Optional[List[str]] = None,
    monitor_ids: Optional[List[int]] = None,
    limit: int = 200
) -> List[Dict[str, Any]]:
    """获取交易列表"""
    start_ms, end_ms = _period_boundaries_ms(period)
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)
    
    sql = f"""
        SELECT *
        FROM monitor_transactions
        WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql}
        ORDER BY tx_timestamp DESC, tx_id DESC
        LIMIT ?
    """
    params: List[Union[int, str]] = [start_ms, end_ms]
    params.extend(scope_params)
    params.append(int(limit))
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def get_monitors_by_user(user_id: int, active_only: bool = True) -> List[Dict[str, Any]]:
    """获取某用户绑定的钱包监听记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        q = "SELECT * FROM address_monitors WHERE user_id = ?"
        if active_only:
            q += " AND is_active = 1"
        q += " ORDER BY monitor_id DESC"
        async with db.execute(q, (user_id,)) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_all_monitors_for_admin(active_only: bool = True, tenant_id: Optional[str] = None) -> List[Dict[str, Any]]:
    """管理端：列出所有监听记录，支持租户隔离"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        conditions = []
        params: List[Union[str, int]] = []
        if active_only:
            conditions.append("is_active = 1")
        if tenant_id is not None:
            conditions.append("tenant_id = ?")
            params.append(tenant_id)
        where_sql = f" WHERE {' AND '.join(conditions)}" if conditions else ""
        q = f"SELECT * FROM address_monitors{where_sql} ORDER BY monitor_id DESC"
        async with db.execute(q, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def deactivate_wallet_monitor(monitor_id: int, operator_user_id: int) -> bool:
    """取消监听：仅地址所属用户或管理员可操作"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT user_id FROM address_monitors WHERE monitor_id = ? AND is_active = 1",
            (monitor_id,),
        ) as cursor:
            row = await cursor.fetchone()
        if not row:
            return False
        owner_id = row[0]
        is_admin = str(operator_user_id) == str(ADMIN_ID)
        if owner_id != operator_user_id and not is_admin:
            return False
        await db.execute(
            "UPDATE address_monitors SET is_active = 0 WHERE monitor_id = ?",
            (monitor_id,),
        )
        await db.commit()
        return True


async def deactivate_monitor_admin(monitor_id: int) -> bool:
    """Web 管理端：强制关闭某条监听"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE address_monitors SET is_active = 0 WHERE monitor_id = ? AND is_active = 1",
            (monitor_id,),
        )
        await db.commit()
        return cur.rowcount > 0


async def save_wallet_monitor(
    user_id: int,
    address: str,
    network: str,
    initial_timestamp: Optional[int] = None,
    remark: str = ''
) -> int:
    """保存钱包监听地址"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT monitor_id FROM address_monitors WHERE user_id = ? AND address = ? AND network = ?",
            (user_id, address, network),
        ) as cursor:
            existing = await cursor.fetchone()
        
        if existing:
            monitor_id = existing[0]
            await db.execute(
                "UPDATE address_monitors SET is_active = 1, remark = ?, last_tx_timestamp = ? WHERE monitor_id = ?",
                (remark, initial_timestamp, monitor_id),
            )
        else:
            cur = await db.execute(
                """INSERT INTO address_monitors (user_id, address, network, remark, last_tx_timestamp, is_active)
                   VALUES (?, ?, ?, ?, ?, 1)""",
                (user_id, address, network, remark, initial_timestamp),
            )
            monitor_id = cur.lastrowid
        await db.commit()
        return monitor_id


async def delete_monitor_admin(monitor_id: int) -> bool:
    """管理端：删除监听记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "DELETE FROM address_monitors WHERE monitor_id = ?",
            (monitor_id,),
        )
        await db.commit()
        return cur.rowcount > 0
