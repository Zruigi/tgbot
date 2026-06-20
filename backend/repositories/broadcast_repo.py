# -*- coding: utf-8 -*-
"""
广播 Repository - 广播目标与地址去重相关数据库操作
"""
import aiosqlite
import logging
from typing import Optional, List, Dict, Any
from config import DB_NAME

logger = logging.getLogger(__name__)


async def upsert_broadcast_target(
    chat_id: int,
    chat_title: str,
    chat_type: str,
    chat_username: Optional[str] = None,
    can_broadcast: bool = True,
    tenant_id: str = 'default',
) -> None:
    """登记/更新广播目标"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, tenant_id, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(chat_id) DO UPDATE SET
                chat_title=excluded.chat_title,
                chat_type=excluded.chat_type,
                chat_username=excluded.chat_username,
                can_broadcast=excluded.can_broadcast,
                tenant_id=excluded.tenant_id,
                updated_at=CURRENT_TIMESTAMP
            """,
            (chat_id, chat_title, chat_type, chat_username, 1 if can_broadcast else 0, tenant_id),
        )
        await db.commit()


async def get_broadcast_targets(
    chat_types: Optional[List[str]] = None,
    enabled_only: bool = True,
    tenant_id: Optional[str] = None,
) -> List[Dict[str, Any]]:
    """获取广播目标列表，支持租户隔离"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        where_parts = []
        params: List[Any] = []

        if tenant_id is not None:
            where_parts.append("tenant_id = ?")
            params.append(tenant_id)

        if enabled_only:
            where_parts.append("can_broadcast = 1")

        if chat_types:
            placeholders = ",".join(["?"] * len(chat_types))
            where_parts.append(f"chat_type IN ({placeholders})")
            params.extend(chat_types)

        where_sql = f"WHERE {' AND '.join(where_parts)}" if where_parts else ""
        sql = f"SELECT * FROM broadcast_targets {where_sql} ORDER BY updated_at DESC"

        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def set_broadcast_target_permission(chat_id: int, can_broadcast: bool) -> bool:
    """更新目标是否允许参与广播"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE broadcast_targets SET can_broadcast = ?, updated_at = CURRENT_TIMESTAMP WHERE chat_id = ?",
            (1 if can_broadcast else 0, chat_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def get_group_address_hit(group_id: int, address: str) -> Optional[Dict[str, Any]]:
    """获取群内地址去重记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM group_address_hits WHERE group_id = ? AND address = ?",
            (group_id, address),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def upsert_group_address_hit(group_id: int, address: str, network: str, user_id: int, tenant_id: str = 'default') -> None:
    """保存群内地址去重记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """INSERT OR REPLACE INTO group_address_hits 
               (group_id, address, network, user_id, tenant_id, detected_at)
               VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)""",
            (group_id, address, network, user_id, tenant_id),
        )
        await db.commit()
