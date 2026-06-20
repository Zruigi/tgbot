# -*- coding: utf-8 -*-
"""
入群验证 Repository - 群组验证配置与会话管理
"""
import aiosqlite
import json
import logging
from datetime import datetime
from typing import Optional, List, Dict, Any
from config import DB_NAME

logger = logging.getLogger(__name__)


async def get_group_verify_config(group_id: int) -> Dict[str, Any]:
    """获取群组入群验证配置"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT verify_enabled, verify_type, verify_timeout, verify_channel_id, verify_questions, verify_msg_template FROM tracked_groups WHERE group_id = ?",
            (group_id,)
        ) as cursor:
            row = await cursor.fetchone()
            if not row:
                return {
                    "verify_enabled": False,
                    "verify_type": "captcha",
                    "verify_timeout": 120,
                    "verify_channel_id": None,
                    "verify_questions": [],
                    "verify_msg_template": None
                }
            return {
                "verify_enabled": bool(row["verify_enabled"]),
                "verify_type": row["verify_type"] or "captcha",
                "verify_timeout": row["verify_timeout"] or 120,
                "verify_channel_id": row["verify_channel_id"],
                "verify_questions": json.loads(row["verify_questions"]) if row["verify_questions"] else [],
                "verify_msg_template": row["verify_msg_template"],
            }


async def save_group_verify_config(group_id: int, config: dict) -> None:
    """保存群组入群验证配置"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """UPDATE tracked_groups SET
                verify_enabled = ?, verify_type = ?, verify_timeout = ?,
                verify_channel_id = ?, verify_questions = ?, verify_msg_template = ?
               WHERE group_id = ?""",
            (
                1 if config.get("verify_enabled") else 0,
                config.get("verify_type", "captcha"),
                config.get("verify_timeout", 120),
                config.get("verify_channel_id"),
                json.dumps(config.get("verify_questions", []), ensure_ascii=False) if config.get("verify_questions") else None,
                config.get("verify_msg_template"),
                group_id,
            )
        )
        await db.commit()


async def get_member_verified(group_id: int, user_id: int) -> bool:
    """检查成员是否已通过频道验证"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT is_verified FROM group_members WHERE group_id=? AND user_id=?",
            (group_id, user_id)
        ) as cursor:
            row = await cursor.fetchone()
            return bool(row[0]) if row else False


async def set_member_verified(group_id: int, user_id: int) -> None:
    """标记成员为已验证"""
    now = datetime.utcnow().strftime("%Y-%m-%d %H:%M:%S")
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """INSERT INTO group_members (group_id, user_id, is_verified, verified_at)
               VALUES (?, ?, 1, ?)
               ON CONFLICT(group_id, user_id) DO UPDATE SET is_verified=1, verified_at=?""",
            (group_id, user_id, now, now)
        )
        await db.commit()


async def get_verify_stats(group_id: int) -> Dict[str, int]:
    """获取群内验证统计：已验证 vs 待验证人数"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE group_id=? AND is_verified=1", (group_id,)
        ) as cur:
            verified = (await cur.fetchone())[0]
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE group_id=? AND (is_verified=0 OR is_verified IS NULL)", (group_id,)
        ) as cur:
            unverified = (await cur.fetchone())[0]
    return {"verified": verified, "unverified": unverified, "total": verified + unverified}


async def get_verify_growth(group_id: Optional[int] = None, period: str = 'day') -> List[Dict[str, Any]]:
    """按天/小时统计通过频道验证的用户数"""
    if period == 'hour':
        fmt = "%Y-%m-%d %H:00"
        trunc = "strftime('%Y-%m-%d %H:00', verified_at)"
    else:
        fmt = "%Y-%m-%d"
        trunc = "strftime('%Y-%m-%d', verified_at)"
    
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        if group_id:
            sql = f"SELECT {trunc} as period, COUNT(*) as count FROM group_members WHERE group_id=? AND is_verified=1 AND verified_at IS NOT NULL GROUP BY period ORDER BY period DESC LIMIT 30"
            params = (group_id,)
        else:
            sql = f"SELECT {trunc} as period, COUNT(*) as count FROM group_members WHERE is_verified=1 AND verified_at IS NOT NULL GROUP BY period ORDER BY period DESC LIMIT 30"
            params = ()
        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
            return [{"period": r["period"], "count": r["count"]} for r in rows]


async def get_verify_growth_summary() -> Dict[str, int]:
    """获取今日 & 总计引导关注数"""
    today = datetime.utcnow().strftime("%Y-%m-%d")
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE is_verified=1 AND date(verified_at)=?", (today,)
        ) as cur:
            today_count = (await cur.fetchone())[0]
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE is_verified=1"
        ) as cur:
            total_count = (await cur.fetchone())[0]
    return {"today": today_count, "total": total_count}


async def create_verify_session(
    group_id: int,
    user_id: int,
    username: str,
    first_name: str,
    verify_type: str,
    answer: str,
    bot_msg_id: int,
    timeout_secs: int
) -> int:
    """创建验证会话，返回 session_id"""
    from datetime import timedelta
    expires_at = (datetime.utcnow() + timedelta(seconds=timeout_secs)).strftime("%Y-%m-%d %H:%M:%S")
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """INSERT INTO join_verify_sessions
               (group_id, user_id, username, first_name, verify_type, answer, bot_msg_id, status, expires_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?)""",
            (group_id, user_id, username, first_name, verify_type, answer, bot_msg_id, expires_at)
        )
        await db.commit()
        return cur.lastrowid


async def get_pending_verify_session(group_id: int, user_id: int) -> Optional[Dict[str, Any]]:
    """获取用户待验证会话"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM join_verify_sessions WHERE group_id=? AND user_id=? AND status='pending' ORDER BY created_at DESC LIMIT 1",
            (group_id, user_id)
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def update_verify_session_status(session_id: int, status: str) -> None:
    """更新验证会话状态：passed / failed / expired"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE join_verify_sessions SET status=? WHERE session_id=?",
            (status, session_id)
        )
        await db.commit()


async def get_expired_verify_sessions() -> List[Dict[str, Any]]:
    """获取所有已超时的待验证会话"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM join_verify_sessions WHERE status='pending' AND expires_at < datetime('now')"
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]
