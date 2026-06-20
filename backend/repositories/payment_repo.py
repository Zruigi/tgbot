# -*- coding: utf-8 -*-
"""
支付 Repository - 套餐支付订单与平台收款配置
"""
import aiosqlite
from typing import Optional, Dict, Any, List
from config import DB_NAME


async def create_payment_order(
    merchant_id: int,
    plan_code: str,
    amount_usdt: float,
    *,
    base_amount_usdt: float = 0,
    amount_offset_usdt: float = 0,
    chain_type: str = 'TRON',
    payment_address: Optional[str] = None,
    payment_qr_image_path: Optional[str] = None,
    expire_at: Optional[str] = None,
) -> int:
    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute(
            """
            INSERT INTO payment_orders
            (
                merchant_id, plan_code, base_amount_usdt, amount_offset_usdt, amount_usdt, chain_type, payment_address,
                payment_qr_image_path, status, expire_at, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            """,
            (
                merchant_id,
                plan_code,
                float(base_amount_usdt or 0),
                float(amount_offset_usdt or 0),
                float(amount_usdt),
                chain_type,
                payment_address,
                payment_qr_image_path,
                expire_at,
            ),
        )
        await db.commit()
        return int(cursor.lastrowid or 0)


async def get_payment_order(order_id: int) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM payment_orders WHERE id = ?", (order_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def list_payment_orders(status: Optional[str] = None, merchant_id: Optional[int] = None, limit: int = 100) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM payment_orders WHERE 1=1"
        params: List[Any] = []
        if status:
            sql += " AND status = ?"
            params.append(status)
        if merchant_id is not None:
            sql += " AND merchant_id = ?"
            params.append(merchant_id)
        sql += " ORDER BY created_at DESC, id DESC LIMIT ?"
        params.append(int(limit))
        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def list_open_orders_for_matching(
    chain_type: str,
    payment_address: str,
    *,
    limit: int = 200,
) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT * FROM payment_orders
            WHERE chain_type = ?
              AND payment_address = ?
              AND status IN ('pending', 'paid_pending_review', 'user_submitted')
              AND (expire_at IS NULL OR expire_at >= CURRENT_TIMESTAMP)
            ORDER BY created_at DESC, id DESC
            LIMIT ?
            """,
            (chain_type, payment_address, int(limit)),
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_latest_open_payment_order(merchant_id: int) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT * FROM payment_orders
            WHERE merchant_id = ?
              AND status IN ('pending', 'paid_pending_review', 'user_submitted')
              AND (expire_at IS NULL OR expire_at >= CURRENT_TIMESTAMP)
            ORDER BY created_at DESC, id DESC
            LIMIT 1
            """,
            (merchant_id,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def mark_payment_order_paid(order_id: int) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE payment_orders
            SET status = 'paid_pending_review', updated_at = CURRENT_TIMESTAMP
            WHERE id = ? AND status = 'pending'
            """,
            (order_id,),
        )
        await db.commit()
        return cur.rowcount > 0


async def submit_payment_tx_hash(order_id: int, tx_hash: str) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE payment_orders
            SET tx_hash = ?, status = 'user_submitted', updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            """,
            (tx_hash, order_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def update_payment_tx_hash_only(order_id: int, tx_hash: str) -> bool:
    """仅更新 tx_hash 字段，不改变订单状态（用于链上自动匹配）"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE payment_orders
            SET tx_hash = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            """,
            (tx_hash, order_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def update_payment_status(order_id: int, status: str, *, confirmed_at: Optional[str] = None) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE payment_orders
            SET status = ?, confirmed_at = COALESCE(?, confirmed_at), updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            """,
            (status, confirmed_at, order_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def upsert_platform_payment_config(
    chain_type: str,
    wallet_address: str,
    *,
    wallet_qr_image_path: Optional[str] = None,
    instruction_text: str = '',
    is_active: bool = True,
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO platform_payment_configs
            (chain_type, wallet_address, wallet_qr_image_path, instruction_text, is_active, updated_at)
            VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(chain_type) DO UPDATE SET
                wallet_address = excluded.wallet_address,
                wallet_qr_image_path = excluded.wallet_qr_image_path,
                instruction_text = excluded.instruction_text,
                is_active = excluded.is_active,
                updated_at = CURRENT_TIMESTAMP
            """,
            (chain_type, wallet_address, wallet_qr_image_path, instruction_text, 1 if is_active else 0),
        )
        await db.commit()
        return True


async def get_active_platform_payment_config(chain_type: str = 'TRON') -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM platform_payment_configs WHERE chain_type = ? AND is_active = 1",
            (chain_type,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def expire_stale_payment_orders() -> int:
    """将超时未支付的订单标记为 expired（expire_at < CURRENT_TIMESTAMP 且 status='pending'）"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE payment_orders
            SET status = 'expired', updated_at = CURRENT_TIMESTAMP
            WHERE status = 'pending'
              AND expire_at IS NOT NULL
              AND expire_at < CURRENT_TIMESTAMP
            """,
        )
        await db.commit()
        return cur.rowcount
