# -*- coding: utf-8 -*-
"""
功能 Repository - 商户功能快照与功能开关
"""
import aiosqlite
from typing import Optional, Dict, Any, List
from config import DB_NAME


async def upsert_merchant_feature(
    merchant_id: int,
    tenant_id: str,
    feature_key: str,
    *,
    enabled: bool,
    source: str = 'plan',
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO merchant_features
            (merchant_id, tenant_id, feature_key, enabled, source, updated_at)
            VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(tenant_id, feature_key) DO UPDATE SET
                merchant_id = excluded.merchant_id,
                enabled = excluded.enabled,
                source = excluded.source,
                updated_at = CURRENT_TIMESTAMP
            """,
            (merchant_id, tenant_id, feature_key, 1 if enabled else 0, source),
        )
        await db.commit()
        return True


async def get_merchant_feature(tenant_id: str, feature_key: str) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM merchant_features WHERE tenant_id = ? AND feature_key = ?",
            (tenant_id, feature_key),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def list_merchant_features(tenant_id: str) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM merchant_features WHERE tenant_id = ? ORDER BY feature_key ASC",
            (tenant_id,),
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def delete_merchant_features(tenant_id: str) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute("DELETE FROM merchant_features WHERE tenant_id = ?", (tenant_id,))
        await db.commit()
        return cur.rowcount > 0


async def list_enabled_features(tenant_id: str) -> List[str]:
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT feature_key FROM merchant_features WHERE tenant_id = ? AND enabled = 1 ORDER BY feature_key ASC",
            (tenant_id,),
        ) as cursor:
            rows = await cursor.fetchall()
            return [str(row[0]) for row in rows]
