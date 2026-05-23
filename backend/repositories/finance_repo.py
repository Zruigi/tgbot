# -*- coding: utf-8 -*-
"""
财务 Repository - 财务对账与会计科目相关数据库操作
"""
import aiosqlite
import aiohttp
import logging
from datetime import datetime
from typing import Optional, List, Dict, Any
from config import DB_NAME

logger = logging.getLogger(__name__)


async def _ensure_finance_exchange_rate_table(db: aiosqlite.Connection) -> None:
    """确保汇率表存在"""
    await db.execute('''
        CREATE TABLE IF NOT EXISTS finance_exchange_rates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token_symbol TEXT NOT NULL,
            fiat_currency TEXT NOT NULL,
            rate REAL NOT NULL,
            rate_timestamp INTEGER NOT NULL,
            source TEXT DEFAULT 'binance',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )
    ''')
    await db.execute('''
        CREATE INDEX IF NOT EXISTS idx_finance_exchange_rates_key
        ON finance_exchange_rates(token_symbol, fiat_currency, rate_timestamp)
    ''')


async def _fetch_binance_spot_rate(token_symbol: str, fiat_currency: str) -> Optional[float]:
    """从币安获取现货价格"""
    token = (token_symbol or '').upper()
    fiat = (fiat_currency or 'USD').upper()

    if token in ('USD', 'USDT', 'USDC') and fiat in ('USD', 'USDT'):
        return 1.0

    quote = 'USDT' if fiat in ('USD', 'USDT') else fiat
    symbol = f"{token}{quote}"
    url = f"https://api.binance.com/api/v3/ticker/price?symbol={symbol}"

    try:
        timeout = aiohttp.ClientTimeout(total=5)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.get(url) as resp:
                if resp.status != 200:
                    return None
                payload = await resp.json()
                price = payload.get('price')
                return float(price) if price is not None else None
    except Exception:
        return None


async def _get_or_fill_exchange_rate(
    db: aiosqlite.Connection,
    token_symbol: str,
    fiat_currency: str,
    tx_timestamp: Optional[int],
    ledger_exchange_rate: Optional[float] = None,
) -> float:
    """获取或填充汇率"""
    token = (token_symbol or '').upper()
    fiat = (fiat_currency or 'USD').upper()
    ts = int(tx_timestamp or datetime.now().timestamp())

    if ledger_exchange_rate and float(ledger_exchange_rate) > 0:
        return float(ledger_exchange_rate)

    cursor = await db.execute(
        '''
        SELECT rate
        FROM finance_exchange_rates
        WHERE token_symbol = ? AND fiat_currency = ? AND rate_timestamp <= ?
        ORDER BY rate_timestamp DESC
        LIMIT 1
        ''',
        (token, fiat, ts),
    )
    row = await cursor.fetchone()
    if row and row[0] is not None:
        return float(row[0])

    live_rate = await _fetch_binance_spot_rate(token, fiat)
    if live_rate is None:
        live_rate = 1.0 if token == fiat else 0.0

    await db.execute(
        '''
        INSERT INTO finance_exchange_rates(token_symbol, fiat_currency, rate, rate_timestamp, source)
        VALUES (?, ?, ?, ?, 'binance')
        ''',
        (token, fiat, live_rate, ts),
    )
    return float(live_rate)


async def get_finance_summary_by_currency(user_id: int) -> Dict[str, Any]:
    """多币种汇总：按 network, token_symbol 聚合统计"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute(
            '''
            SELECT
                COALESCE(m.network, 'unknown') AS network,
                l.token_symbol AS token_symbol,
                COUNT(1) AS tx_count,
                SUM(CASE WHEN l.direction = 'in' THEN CAST(l.amount AS REAL) ELSE 0 END) AS total_in,
                SUM(CASE WHEN l.direction = 'out' THEN CAST(l.amount AS REAL) ELSE 0 END) AS total_out,
                SUM(CASE WHEN l.reconciled_status = 'UNRECONCILED' THEN 1 ELSE 0 END) AS unreconciled_count
            FROM finance_ledgers l
            LEFT JOIN address_monitors m ON m.monitor_id = l.monitor_id
            WHERE l.user_id = ?
            GROUP BY COALESCE(m.network, 'unknown'), l.token_symbol
            ORDER BY network, token_symbol
            ''',
            (user_id,),
        )
        rows = [dict(r) for r in await cursor.fetchall()]

        total_unreconciled = sum(int(r.get('unreconciled_count') or 0) for r in rows)
        return {
            'by_currency': rows,
            'total_unreconciled': total_unreconciled,
        }


async def get_finance_unreconciled_with_fx(
    user_id: int,
    monitor_id: Optional[int] = None
) -> List[Dict[str, Any]]:
    """未对账流水：按交易时间匹配历史汇率，缺失时调用币安补全"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        await _ensure_finance_exchange_rate_table(db)

        query = '''
            SELECT
                l.ledger_id,
                l.monitor_id,
                l.direction,
                l.amount,
                l.token_symbol,
                l.base_fiat_value,
                l.fiat_currency,
                l.exchange_rate,
                l.counterparty,
                l.tx_hash,
                l.tx_timestamp,
                l.created_at,
                l.reconciled_status,
                COALESCE(m.network, 'unknown') AS network,
                a.code,
                a.name
            FROM finance_ledgers l
            LEFT JOIN address_monitors m ON m.monitor_id = l.monitor_id
            LEFT JOIN chart_of_accounts a ON a.account_id = l.account_id
            WHERE l.user_id = ? AND l.reconciled_status = 'UNRECONCILED'
        '''
        params: List[Any] = [user_id]

        if monitor_id is not None:
            query += ' AND l.monitor_id = ?'
            params.append(monitor_id)

        query += ' ORDER BY l.tx_timestamp DESC LIMIT 500'

        cursor = await db.execute(query, params)
        rows = [dict(r) for r in await cursor.fetchall()]

        for row in rows:
            rate = await _get_or_fill_exchange_rate(
                db,
                token_symbol=row.get('token_symbol') or '',
                fiat_currency=row.get('fiat_currency') or 'USD',
                tx_timestamp=row.get('tx_timestamp'),
                ledger_exchange_rate=row.get('exchange_rate'),
            )
            amount = float(row.get('amount') or 0)
            fiat_value = float(row.get('base_fiat_value') or 0)
            if fiat_value <= 0 and rate > 0:
                fiat_value = round(amount * rate, 8)

            row['exchange_rate_used'] = rate
            row['fiat_value'] = fiat_value

        await db.commit()
        return rows


async def reconcile_finance_ledger(
    ledger_id: str,
    account_code: str,
    admin_userid: int,
    amount: Optional[float] = None,
    ip_address: Optional[str] = None,
) -> bool:
    """按 ledger_id 执行对账并记录审计日志"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        await db.execute('BEGIN IMMEDIATE')
        try:
            db.row_factory = aiosqlite.Row
            cursor = await db.execute(
                'SELECT ledger_id, user_id, reconciled_status FROM finance_ledgers WHERE ledger_id = ?',
                (ledger_id,),
            )
            ledger = await cursor.fetchone()
            if not ledger or int(ledger['user_id']) != int(admin_userid):
                await db.execute('ROLLBACK')
                return False

            if ledger['reconciled_status'] == 'RECONCILED':
                await db.execute('ROLLBACK')
                return True

            cursor = await db.execute(
                'SELECT account_id FROM chart_of_accounts WHERE code = ? AND is_active = 1',
                (account_code,),
            )
            account = await cursor.fetchone()
            if not account:
                await db.execute('ROLLBACK')
                raise ValueError(f'会计科目不存在：{account_code}')

            await db.execute(
                '''
                UPDATE finance_ledgers
                SET account_id = ?, reconciled_status = 'RECONCILED', updated_at = CURRENT_TIMESTAMP
                WHERE ledger_id = ?
                ''',
                (int(account['account_id']), ledger_id),
            )

            await db.execute(
                '''
                INSERT INTO audit_logs(admin_userid, action, target_table, target_id, reason, ip_address)
                VALUES (?, 'RECONCILE', 'finance_ledgers', ?, ?, ?)
                ''',
                (admin_userid, ledger_id, f'对账至科目 {account_code}' + (f' 金额 {amount}' if amount is not None else ''), ip_address),
            )

            await db.execute('COMMIT')
            return True
        except Exception:
            await db.execute('ROLLBACK')
            raise


async def get_finance_category_tree() -> List[Dict[str, Any]]:
    """获取会计科目树"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute(
            '''
            SELECT account_id, code, name, type, parent_id, description
            FROM chart_of_accounts
            WHERE is_active = 1
            ORDER BY code
            '''
        )
        rows = [dict(r) for r in await cursor.fetchall()]

    node_map: Dict[int, Dict[str, Any]] = {}
    roots: List[Dict[str, Any]] = []

    for row in rows:
        node = dict(row)
        node['children'] = []
        node_map[node['account_id']] = node

        parent_id = node.get('parent_id')
        if parent_id and parent_id in node_map:
            node_map[parent_id]['children'].append(node)
        else:
            roots.append(node)

    return roots


async def get_chart_of_accounts(active_only: bool = True) -> List[Dict[str, Any]]:
    """获取会计科目列表"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        query = '''
            SELECT account_id, code, name, type, parent_id, description, is_active
            FROM chart_of_accounts
        '''
        if active_only:
            query += ' WHERE is_active = 1'
        query += ' ORDER BY code'
        
        cursor = await db.execute(query)
        rows = [dict(r) for r in await cursor.fetchall()]
        return rows
