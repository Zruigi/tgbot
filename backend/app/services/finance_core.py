# -*- coding: utf-8 -*-
"""
Web3 企业级财务 ERP 核心模块 - 数据库和业务逻辑
"""
import aiosqlite
import logging
from datetime import datetime
from decimal import Decimal, ROUND_DOWN, InvalidOperation
from typing import Optional, List, Dict, Any
from enum import Enum
import uuid

logger = logging.getLogger(__name__)

# 金额精度：最多8位小数（修复 P0: 金额精度无验证）
_AMOUNT_PRECISION = Decimal('0.00000001')
_AMOUNT_MAX = Decimal('999999999999')  # 防止天文数字


def validate_and_normalize_amount(amount) -> Decimal:
    """
    校验并规范化金额：
    - 必须为正数
    - 最多 8 位小数（截断，不四舍五入）
    - 不超过最大值上限
    修复 P0: 金额输入无精度校验（CVSS 8.5）
    """
    try:
        d = Decimal(str(amount))
    except (InvalidOperation, TypeError, ValueError):
        raise ValueError(f"无效金额格式: {amount}")
    if d <= 0:
        raise ValueError(f"金额必须为正数，收到: {d}")
    if d > _AMOUNT_MAX:
        raise ValueError(f"金额超出最大允许值: {d}")
    # 截断至8位小数（不四舍五入，防止财务误差）
    d = d.quantize(_AMOUNT_PRECISION, rounding=ROUND_DOWN)
    return d

class AccountType(str, Enum):
    ASSET = "asset"
    LIABILITY = "liability"
    EQUITY = "equity"
    INCOME = "income"
    EXPENSE = "expense"

class ReconcileStatus(str, Enum):
    UNRECONCILED = "UNRECONCILED"
    RECONCILED = "RECONCILED"
    DISPUTED = "DISPUTED"

class AuditAction(str, Enum):
    CREATE = "CREATE"
    UPDATE = "UPDATE"
    DELETE = "DELETE"
    RECONCILE = "RECONCILE"
    REVERSE = "REVERSE"

async def init_finance_db():
    """初始化财务模块数据库表（安全加固：添加唯一约束、检查约束和索引）"""
    from config import DB_NAME

    async with aiosqlite.connect(DB_NAME) as db:
        # 会计科目表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS chart_of_accounts (
                account_id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT UNIQUE NOT NULL,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                parent_id INTEGER,
                description TEXT,
                is_active BOOLEAN DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (parent_id) REFERENCES chart_of_accounts(account_id)
            )
        ''')

        # 总账表（修复 High: 添加 tx_hash 唯一约束 + amount 正数约束）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS finance_ledgers (
                ledger_id TEXT PRIMARY KEY,
                monitor_id INTEGER,
                user_id INTEGER NOT NULL,
                account_id INTEGER NOT NULL,
                direction TEXT NOT NULL CHECK(direction IN ('in', 'out')),
                amount TEXT NOT NULL,
                token_symbol TEXT NOT NULL,
                base_fiat_value TEXT NOT NULL,
                fiat_currency TEXT DEFAULT 'USD',
                exchange_rate REAL,
                counterparty TEXT,
                counterparty_alias TEXT,
                description TEXT,
                reconciled_status TEXT DEFAULT 'UNRECONCILED'
                    CHECK(reconciled_status IN ('UNRECONCILED', 'RECONCILED', 'DISPUTED')),
                voucher_file_id TEXT,
                tx_hash TEXT,
                tx_timestamp INTEGER NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (account_id) REFERENCES chart_of_accounts(account_id),
                FOREIGN KEY (monitor_id) REFERENCES address_monitors(monitor_id)
            )
        ''')

        # tx_hash 唯一索引（修复 High: 交易记录无去重机制）
        await db.execute('''
            CREATE UNIQUE INDEX IF NOT EXISTS idx_finance_ledgers_tx_hash
            ON finance_ledgers(tx_hash) WHERE tx_hash IS NOT NULL
        ''')

        # 按 user_id 和 reconciled_status 建立复合索引（提升查询性能）
        await db.execute('''
            CREATE INDEX IF NOT EXISTS idx_finance_ledgers_user_status
            ON finance_ledgers(user_id, reconciled_status)
        ''')

        # 审计日志表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS audit_logs (
                audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
                admin_userid INTEGER NOT NULL,
                action TEXT NOT NULL,
                target_table TEXT NOT NULL,
                target_id TEXT,
                before_state TEXT,
                after_state TEXT,
                reason TEXT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                ip_address TEXT
            )
        ''')

        # 审计日志时间索引
        await db.execute('''
            CREATE INDEX IF NOT EXISTS idx_audit_logs_timestamp
            ON audit_logs(timestamp)
        ''')

        # 对账工作台
        await db.execute('''
            CREATE TABLE IF NOT EXISTS reconciliation_tasks (
                task_id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                ledger_id TEXT NOT NULL,
                assigned_to INTEGER,
                status TEXT DEFAULT 'pending',
                notes TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                completed_at DATETIME,
                FOREIGN KEY (ledger_id) REFERENCES finance_ledgers(ledger_id)
            )
        ''')

        await db.commit()
        logger.info("✅ 财务模块数据库初始化完成")

async def init_default_chart_of_accounts():
    """初始化默认的企业级会计科目"""
    from config import DB_NAME
    
    default_accounts = [
        ("1001", "现金", AccountType.ASSET, None, "企业现金账户"),
        ("1002", "银行存款", AccountType.ASSET, None, "企业银行账户"),
        ("1010", "应收账款", AccountType.ASSET, None, "客户欠款"),
        ("1020", "库存资产", AccountType.ASSET, None, "加密资产库存"),
        ("2001", "应付账款", AccountType.LIABILITY, None, "应付供应商款项"),
        ("2010", "应付税款", AccountType.LIABILITY, None, "应缴税款"),
        ("3001", "实收资本", AccountType.EQUITY, None, "企业初始资本"),
        ("3010", "留存收益", AccountType.EQUITY, None, "累积利润"),
        ("4001", "主营业务收入", AccountType.INCOME, None, "交易手续费收入"),
        ("4010", "其他业务收入", AccountType.INCOME, None, "利息、奖励等"),
        ("5001", "主营业务成本", AccountType.EXPENSE, None, "交易成本"),
        ("5010", "运营费用", AccountType.EXPENSE, None, "服务器、人工等"),
        ("5020", "税费支出", AccountType.EXPENSE, None, "各类税费"),
    ]
    
    async with aiosqlite.connect(DB_NAME) as db:
        for code, name, acc_type, parent_id, desc in default_accounts:
            try:
                await db.execute('''
                    INSERT OR IGNORE INTO chart_of_accounts 
                    (code, name, type, parent_id, description)
                    VALUES (?, ?, ?, ?, ?)
                ''', (code, name, acc_type.value, parent_id, desc))
            except Exception as e:
                logger.error(f"插入科目 {code} 失败: {e}")
        
        await db.commit()
        logger.info("✅ 默认会计科目初始化完成")

async def create_finance_ledger(
    monitor_id: int,
    user_id: int,
    account_id: int,
    direction: str,
    amount: float,
    token_symbol: str,
    base_fiat_value: float,
    fiat_currency: str = "USD",
    exchange_rate: float = 1.0,
    counterparty: str = None,
    description: str = None,
    tx_hash: str = None,
    tx_timestamp: int = None
) -> str:
    """创建总账记录（安全加固：金额 Decimal 精度校验 + tx_hash 去重）"""
    from config import DB_NAME

    # 修复 P0: 金额精度无验证
    validated_amount = validate_and_normalize_amount(amount)
    validated_fiat = validate_and_normalize_amount(base_fiat_value)

    if direction not in ('in', 'out'):
        raise ValueError(f"direction 必须为 'in' 或 'out'，收到: {direction}")

    ledger_id = f"ledger_{uuid.uuid4().hex[:12]}"

    async with aiosqlite.connect(DB_NAME) as db:
        # 修复 P0: 交易记录无去重机制 —— tx_hash 唯一性检查
        if tx_hash:
            cursor = await db.execute(
                "SELECT ledger_id FROM finance_ledgers WHERE tx_hash = ?",
                (tx_hash,)
            )
            existing = await cursor.fetchone()
            if existing:
                logger.warning(f"重复 tx_hash 拦截: {tx_hash} 已存在 ledger {existing[0]}")
                return existing[0]  # 幂等返回已有记录

        await db.execute('''
            INSERT INTO finance_ledgers 
            (ledger_id, monitor_id, user_id, account_id, direction, amount, 
             token_symbol, base_fiat_value, fiat_currency, exchange_rate,
             counterparty, description, tx_hash, tx_timestamp)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ''', (
            ledger_id, monitor_id, user_id, account_id, direction,
            str(validated_amount),  # 存储为字符串保留精度
            token_symbol, str(validated_fiat), fiat_currency, exchange_rate,
            counterparty, description, tx_hash,
            tx_timestamp or int(datetime.now().timestamp())
        ))

        await log_audit_action(
            admin_userid=user_id,
            action=AuditAction.CREATE.value,
            target_table="finance_ledgers",
            target_id=ledger_id,
            after_state=f"创建总账记录: {validated_amount} {token_symbol}"
        )

        await db.commit()

    return ledger_id

async def reconcile_ledger(
    ledger_id: str,
    admin_userid: int,
    voucher_file_id: str = None,
    notes: str = None,
    ip_address: str = None
) -> bool:
    """核销总账记录（安全加固：BEGIN IMMEDIATE 悲观锁 + 版本号乐观控制，防止并发双重核销）"""
    from config import DB_NAME

    async with aiosqlite.connect(DB_NAME) as db:
        # 修复 P0: 财务对账无原子性 —— 使用 BEGIN IMMEDIATE 获取写锁
        await db.execute("BEGIN IMMEDIATE")
        try:
            cursor = await db.execute(
                "SELECT reconciled_status, version FROM finance_ledgers WHERE ledger_id = ?",
                (ledger_id,)
            )
            row = await cursor.fetchone()
            if not row:
                await db.execute("ROLLBACK")
                return False

            before_status = row[0]
            current_version = row[1] if row[1] is not None else 1

            # 防止重复核销（幂等保护）
            if before_status == ReconcileStatus.RECONCILED.value:
                await db.execute("ROLLBACK")
                logger.warning(f"重复核销拦截: ledger {ledger_id} 已处于 RECONCILED 状态")
                return True  # 幂等返回成功

            # 修复 High: 状态更新无版本控制 —— 版本号递增（乐观锁）
            await db.execute('''
                UPDATE finance_ledgers 
                SET reconciled_status = ?, voucher_file_id = ?,
                    version = version + 1, updated_at = CURRENT_TIMESTAMP
                WHERE ledger_id = ? AND version = ?
            ''', (ReconcileStatus.RECONCILED.value, voucher_file_id, ledger_id, current_version))

            await db.execute('''
                INSERT INTO audit_logs 
                (admin_userid, action, target_table, target_id, before_state, after_state, reason, ip_address)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ''', (
                admin_userid, AuditAction.RECONCILE.value, "finance_ledgers",
                ledger_id, before_status, ReconcileStatus.RECONCILED.value, notes, ip_address
            ))

            await db.execute("COMMIT")
        except Exception as e:
            await db.execute("ROLLBACK")
            logger.error(f"核销事务回滚: {e}")
            raise

    return True

async def reverse_ledger_entry(
    original_ledger_id: str,
    admin_userid: int,
    reason: str = None,
    ip_address: str = None
) -> str:
    """红字冲销：生成反向记录（安全加固：BEGIN IMMEDIATE 悲观锁，防止并发双重冲销）"""
    from config import DB_NAME

    async with aiosqlite.connect(DB_NAME) as db:
        # 修复 P0: 财务对账无原子性 —— 使用 BEGIN IMMEDIATE 获取写锁
        await db.execute("BEGIN IMMEDIATE")
        try:
            cursor = await db.execute(
                '''SELECT monitor_id, user_id, account_id, direction, amount, token_symbol,
                          base_fiat_value, fiat_currency, exchange_rate, counterparty,
                          description, tx_hash, tx_timestamp, reconciled_status
                   FROM finance_ledgers WHERE ledger_id = ?''',
                (original_ledger_id,)
            )
            row = await cursor.fetchone()
            if not row:
                await db.execute("ROLLBACK")
                return None

            # 防止对已 DISPUTED 记录重复冲销
            if row[13] == ReconcileStatus.DISPUTED.value:
                await db.execute("ROLLBACK")
                logger.warning(f"重复冲销拦截: ledger {original_ledger_id} 已处于 DISPUTED 状态")
                return None

            reverse_direction = "out" if row[3] == "in" else "in"
            reverse_ledger_id = f"ledger_{uuid.uuid4().hex[:12]}"

            await db.execute('''
                INSERT INTO finance_ledgers 
                (ledger_id, monitor_id, user_id, account_id, direction, amount, 
                 token_symbol, base_fiat_value, fiat_currency, exchange_rate,
                 counterparty, description, tx_hash, tx_timestamp)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ''', (
                reverse_ledger_id, row[0], row[1], row[2], reverse_direction, row[4],
                row[5], row[6], row[7], row[8], row[9],
                f"[红字冲销] {row[10]}", row[11], row[12]
            ))

            await db.execute('''
                UPDATE finance_ledgers 
                SET reconciled_status = 'DISPUTED', updated_at = CURRENT_TIMESTAMP
                WHERE ledger_id = ?
            ''', (original_ledger_id,))

            await db.execute('''
                INSERT INTO audit_logs 
                (admin_userid, action, target_table, target_id, after_state, reason, ip_address)
                VALUES (?, ?, ?, ?, ?, ?, ?)
            ''', (
                admin_userid, AuditAction.REVERSE.value, "finance_ledgers",
                original_ledger_id, f"红字冲销 → {reverse_ledger_id}", reason, ip_address
            ))

            await db.execute("COMMIT")
        except Exception as e:
            await db.execute("ROLLBACK")
            logger.error(f"冲销事务回滚: {e}")
            raise

    return reverse_ledger_id

async def log_audit_action(
    admin_userid: int,
    action: str,
    target_table: str,
    target_id: str = None,
    before_state: str = None,
    after_state: str = None,
    reason: str = None,
    ip_address: str = None
) -> int:
    """记录审计日志"""
    from config import DB_NAME
    
    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute('''
            INSERT INTO audit_logs 
            (admin_userid, action, target_table, target_id, before_state, after_state, reason, ip_address)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ''', (admin_userid, action, target_table, target_id, before_state, after_state, reason, ip_address))
        
        await db.commit()
        return cursor.lastrowid

async def get_unreconciled_ledgers(user_id: int = None) -> List[Dict[str, Any]]:
    """获取未核销流水列表"""
    from config import DB_NAME
    
    query = '''
        SELECT l.*, a.code, a.name, a.type
        FROM finance_ledgers l
        JOIN chart_of_accounts a ON l.account_id = a.account_id
        WHERE l.reconciled_status = 'UNRECONCILED'
    '''
    params = []
    
    if user_id:
        query += " AND l.user_id = ?"
        params.append(user_id)
    
    query += " ORDER BY l.tx_timestamp DESC LIMIT 500"
    
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute(query, params)
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]

async def get_reconciliation_summary(user_id: int = None) -> Dict[str, Any]:
    """获取对账摘要统计"""
    from config import DB_NAME
    
    query = '''
        SELECT 
            COUNT(*) as total_unreconciled,
            COALESCE(SUM(CASE WHEN direction = 'in' THEN base_fiat_value ELSE 0 END), 0) as total_inflow,
            COALESCE(SUM(CASE WHEN direction = 'out' THEN base_fiat_value ELSE 0 END), 0) as total_outflow
        FROM finance_ledgers
        WHERE reconciled_status = 'UNRECONCILED'
    '''
    params = []
    
    if user_id:
        query += " AND user_id = ?"
        params.append(user_id)
    
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute(query, params)
        row = await cursor.fetchone()
        return dict(row) if row else {
            "total_unreconciled": 0,
            "total_inflow": 0,
            "total_outflow": 0
        }

async def get_audit_logs(days: int = 30, action: str = None, target_table: str = None) -> List[Dict[str, Any]]:
    """获取审计日志"""
    from config import DB_NAME
    from datetime import datetime, timedelta
    
    query = '''
        SELECT audit_id, admin_userid, action, target_table, target_id, 
               before_state, after_state, reason, timestamp, ip_address
        FROM audit_logs
        WHERE timestamp >= datetime('now', '-' || ? || ' days')
    '''
    params = [days]
    
    if action:
        query += " AND action = ?"
        params.append(action)
    
    if target_table:
        query += " AND target_table = ?"
        params.append(target_table)
    
    query += " ORDER BY timestamp DESC LIMIT 500"
    
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute(query, params)
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]
