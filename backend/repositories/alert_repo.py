# -*- coding: utf-8 -*-
"""
警报 Repository - 价格提醒相关数据库操作
"""
import aiosqlite
import logging
from typing import Optional, List, Dict, Any
from config import DB_NAME

logger = logging.getLogger(__name__)


async def get_active_alerts() -> List[Dict[str, Any]]:
    """获取所有活跃的价格监控任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM alerts WHERE is_active = 1") as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def update_alert_state(alert_id: int, state: str, is_active: bool = True) -> None:
    """更新监控任务的状态"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE alerts SET channel_state = ?, is_active = ? WHERE alert_id = ?",
            (state, 1 if is_active else 0, alert_id)
        )
        await db.commit()


async def get_alert_by_id(alert_id: int) -> Optional[Dict[str, Any]]:
    """根据 ID 获取警报"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM alerts WHERE alert_id = ?", (alert_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def get_alerts_by_user(user_id: int, active_only: bool = True) -> List[Dict[str, Any]]:
    """获取用户的警报列表"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        query = "SELECT * FROM alerts WHERE user_id = ?"
        if active_only:
            query += " AND is_active = 1"
        query += " ORDER BY alert_id DESC"
        
        async with db.execute(query, (user_id,)) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def create_alert(
    user_id: int,
    symbol: str,
    market_type: str,
    alert_type: str,
    target_value: float,
    notify_method: str = 'telegram',
    channel_state: Optional[str] = None,
    upper_line: Optional[float] = None,
    lower_line: Optional[float] = None,
) -> int:
    """创建新的价格提醒"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """INSERT INTO alerts 
               (user_id, symbol, market_type, alert_type, target_value, notify_method, channel_state, upper_line, lower_line, is_active)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)""",
            (user_id, symbol, market_type, alert_type, target_value, notify_method, channel_state, upper_line, lower_line),
        )
        await db.commit()
        return cur.lastrowid


async def delete_alert(alert_id: int, user_id: Optional[int] = None) -> bool:
    """删除警报"""
    async with aiosqlite.connect(DB_NAME) as db:
        if user_id:
            cur = await db.execute(
                "DELETE FROM alerts WHERE alert_id = ? AND user_id = ?",
                (alert_id, user_id),
            )
        else:
            cur = await db.execute(
                "DELETE FROM alerts WHERE alert_id = ?",
                (alert_id,),
            )
        await db.commit()
        return cur.rowcount > 0


async def deactivate_alert(alert_id: int, user_id: Optional[int] = None) -> bool:
    """停用警报"""
    async with aiosqlite.connect(DB_NAME) as db:
        if user_id:
            cur = await db.execute(
                "UPDATE alerts SET is_active = 0 WHERE alert_id = ? AND user_id = ?",
                (alert_id, user_id),
            )
        else:
            cur = await db.execute(
                "UPDATE alerts SET is_active = 0 WHERE alert_id = ?",
                (alert_id,),
            )
        await db.commit()
        return cur.rowcount > 0
