# -*- coding: utf-8 -*-
"""
配额 Repository - 商户 AI 配额与用量重置
"""
import aiosqlite
from typing import Optional, Dict, Any, List
from config import DB_NAME


async def upsert_merchant_ai_quota(
    merchant_id: int,
    tenant_id: str,
    *,
    daily_quota: int,
    monthly_quota: int,
    used_daily: int = 0,
    used_monthly: int = 0,
    last_reset_at: Optional[str] = None,
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO merchant_ai_quota
            (
                merchant_id, tenant_id, daily_quota, monthly_quota,
                used_daily, used_monthly, last_reset_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, COALESCE(?, CURRENT_TIMESTAMP), CURRENT_TIMESTAMP)
            ON CONFLICT(tenant_id) DO UPDATE SET
                merchant_id = excluded.merchant_id,
                daily_quota = excluded.daily_quota,
                monthly_quota = excluded.monthly_quota,
                used_daily = excluded.used_daily,
                used_monthly = excluded.used_monthly,
                last_reset_at = excluded.last_reset_at,
                updated_at = CURRENT_TIMESTAMP
            """,
            (
                merchant_id,
                tenant_id,
                int(daily_quota),
                int(monthly_quota),
                int(used_daily),
                int(used_monthly),
                last_reset_at,
            ),
        )
        await db.commit()
        return True


async def get_merchant_ai_quota(tenant_id: str) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM merchant_ai_quota WHERE tenant_id = ?",
            (tenant_id,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def increment_ai_quota_usage(tenant_id: str, daily_inc: int = 0, monthly_inc: int = 0) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE merchant_ai_quota
            SET used_daily = used_daily + ?,
                used_monthly = used_monthly + ?,
                updated_at = CURRENT_TIMESTAMP
            WHERE tenant_id = ?
            """,
            (int(daily_inc), int(monthly_inc), tenant_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def reset_daily_ai_quota(tenant_id: str, *, last_reset_at: Optional[str] = None) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE merchant_ai_quota
            SET used_daily = 0,
                last_reset_at = COALESCE(?, CURRENT_TIMESTAMP),
                updated_at = CURRENT_TIMESTAMP
            WHERE tenant_id = ?
            """,
            (last_reset_at, tenant_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def reset_monthly_ai_quota(tenant_id: str, *, last_reset_at: Optional[str] = None) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE merchant_ai_quota
            SET used_daily = 0,
                used_monthly = 0,
                last_reset_at = COALESCE(?, CURRENT_TIMESTAMP),
                updated_at = CURRENT_TIMESTAMP
            WHERE tenant_id = ?
            """,
            (last_reset_at, tenant_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def list_merchants_needing_quota_reset(limit: int = 200) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT * FROM merchant_ai_quota
            ORDER BY last_reset_at ASC, id ASC
            LIMIT ?
            """,
            (int(limit),),
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]
