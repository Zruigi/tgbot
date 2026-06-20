# -*- coding: utf-8 -*-
"""
订阅 Repository - 套餐与商户订阅生命周期
"""
import aiosqlite
from typing import Optional, Dict, Any, List
from config import DB_NAME


async def list_plans(active_only: bool = True) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM plans"
        params: List[Any] = []
        if active_only:
            sql += " WHERE status = 'active'"
        sql += " ORDER BY price_usdt ASC, duration_days ASC, id ASC"
        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_plan(plan_code: str) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM plans WHERE plan_code = ?", (plan_code,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def upsert_plan(
    plan_code: str,
    plan_name: str,
    *,
    price_usdt: float,
    duration_days: int,
    features_json: str,
    status: str = 'active',
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO plans (plan_code, plan_name, price_usdt, duration_days, features_json, status, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT(plan_code) DO UPDATE SET
                plan_name = excluded.plan_name,
                price_usdt = excluded.price_usdt,
                duration_days = excluded.duration_days,
                features_json = excluded.features_json,
                status = excluded.status,
                updated_at = CURRENT_TIMESTAMP
            """,
            (plan_code, plan_name, float(price_usdt), int(duration_days), features_json, status),
        )
        await db.commit()
        return True


async def create_subscription(
    merchant_id: int,
    plan_code: str,
    *,
    status: str = 'pending',
    start_at: Optional[str] = None,
    end_at: Optional[str] = None,
    payment_order_id: Optional[int] = None,
) -> int:
    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute(
            """
            INSERT INTO merchant_subscriptions
            (merchant_id, plan_code, status, start_at, end_at, payment_order_id, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            """,
            (merchant_id, plan_code, status, start_at, end_at, payment_order_id),
        )
        await db.commit()
        return int(cursor.lastrowid or 0)


async def get_active_subscription(merchant_id: int) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT * FROM merchant_subscriptions
            WHERE merchant_id = ? AND status IN ('active', 'active_subscription')
            ORDER BY COALESCE(end_at, created_at) DESC, id DESC
            LIMIT 1
            """,
            (merchant_id,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def list_expired_subscriptions() -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT * FROM merchant_subscriptions
            WHERE status IN ('active', 'active_subscription')
              AND end_at IS NOT NULL
              AND end_at < CURRENT_TIMESTAMP
            ORDER BY end_at ASC, id ASC
            """
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def update_subscription_status(
    subscription_id: int,
    status: str,
    *,
    start_at: Optional[str] = None,
    end_at: Optional[str] = None,
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE merchant_subscriptions
            SET status = ?,
                start_at = COALESCE(?, start_at),
                end_at = COALESCE(?, end_at),
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            """,
            (status, start_at, end_at, subscription_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def list_subscriptions(
    status: Optional[str] = None,
    merchant_id: Optional[int] = None,
    limit: int = 100,
) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM merchant_subscriptions WHERE 1=1"
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
