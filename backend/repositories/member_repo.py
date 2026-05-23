# -*- coding: utf-8 -*-
"""
群成员 Repository - 群组成员管理相关数据库操作
"""
import aiosqlite
import logging
from typing import Optional, List, Dict, Any
from config import DB_NAME

logger = logging.getLogger(__name__)


async def get_group_members(group_id: int) -> List[Dict[str, Any]]:
    """获取群组成员列表（严格按 group_id）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM group_members WHERE group_id = ? AND status = 'active' ORDER BY joined_at DESC, id DESC",
            (group_id,)
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_group_members_live(group_id: int, bot=None) -> List[Dict[str, Any]]:
    """实时获取群成员（优先 Telegram API，失败回退缓存）"""
    members: List[Dict[str, Any]] = []

    async def _append_from_cached() -> None:
        cached = await get_group_members(group_id)
        seen_local = {m.get("user_id") for m in members}
        for m in cached:
            uid = m.get("user_id")
            if uid in seen_local:
                continue
            members.append({
                "user_id": uid,
                "username": m.get("username"),
                "first_name": m.get("first_name"),
                "last_name": m.get("last_name"),
                "status": m.get("status") or "member",
                "is_bot": False,
            })
            seen_local.add(uid)

    async def _append_from_group_messages() -> None:
        """兜底：从已记录群消息提取最近活跃发言者"""
        seen_local = {m.get("user_id") for m in members}
        async with aiosqlite.connect(DB_NAME) as db:
            db.row_factory = aiosqlite.Row
            async with db.execute(
                """
                SELECT user_id,
                       MAX(COALESCE(NULLIF(username, ''), '')) AS username
                FROM group_messages
                WHERE group_id = ?
                  AND user_id > 0
                  AND COALESCE(sender_type, 'user') IN ('user', 'member')
                GROUP BY user_id
                ORDER BY MAX(COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER))) DESC
                LIMIT 300
                """,
                (group_id,)
            ) as cursor:
                rows = await cursor.fetchall()

        for r in rows:
            uid = r["user_id"]
            if uid in seen_local:
                continue
            username = r["username"]
            members.append({
                "user_id": uid,
                "username": username,
                "first_name": None,
                "last_name": None,
                "status": "member",
                "is_bot": False,
            })
            await save_group_member(
                user_id=uid,
                username=username or f"User{uid}",
                first_name=None,
                last_name=None,
                group_id=group_id,
                group_title=None,
            )
            seen_local.add(uid)

    try:
        admins = await bot.get_chat_administrators(group_id) if bot else []
        seen = set()
        for admin in admins:
            u = admin.user
            seen.add(u.id)
            members.append({
                "user_id": u.id,
                "username": u.username,
                "first_name": u.first_name,
                "last_name": u.last_name,
                "status": getattr(admin, 'status', 'member'),
                "is_bot": u.is_bot,
            })
        
        # 添加普通成员
        all_members = await bot.get_chat_members(group_id) if bot else []
        for m in all_members:
            u = m.user
            if u.id not in seen:
                seen.add(u.id)
                members.append({
                    "user_id": u.id,
                    "username": u.username,
                    "first_name": u.first_name,
                    "last_name": u.last_name,
                    "status": getattr(m, 'status', 'member'),
                    "is_bot": u.is_bot,
                })
    except Exception as e:
        logger.warning(f"获取群成员失败，回退到本地缓存：{e}")
        await _append_from_cached()
        if not members:
            await _append_from_group_messages()

    return members


async def save_group_member(
    user_id: int,
    username: Optional[str],
    first_name: Optional[str] = None,
    last_name: Optional[str] = None,
    group_id: Optional[int] = None,
    group_title: Optional[str] = None
) -> None:
    """保存群组成员信息"""
    if group_id is None:
        return
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """INSERT INTO group_members 
               (user_id, username, first_name, last_name, group_id, group_title, status, joined_at)
               VALUES (?, ?, ?, ?, ?, ?, 'active', CURRENT_TIMESTAMP)
               ON CONFLICT(group_id, user_id) DO UPDATE SET
               username=excluded.username,
               first_name=excluded.first_name,
               last_name=excluded.last_name,
               group_title=excluded.group_title,
               status='active',
               joined_at=CURRENT_TIMESTAMP""",
            (user_id, username, first_name, last_name, group_id, group_title)
        )
        await db.commit()


async def update_group_member_status(user_id: int, group_id: int, status: str) -> None:
    """更新群成员状态"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE group_members SET status = ? WHERE user_id = ? AND group_id = ?",
            (status, user_id, group_id)
        )
        await db.commit()


async def promote_potential_user(user_id: int) -> bool:
    """将潜在用户转正"""
    async with aiosqlite.connect(DB_NAME) as db:
        # 1. 获取潜在用户信息
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM potential_users WHERE user_id = ?", (user_id,)) as cursor:
            p_user = await cursor.fetchone()
        
        if not p_user:
            return False
            
        # 2. 插入或更新到 users 表
        await db.execute(
            "INSERT OR IGNORE INTO users (user_id, created_at) VALUES (?, CURRENT_TIMESTAMP)",
            (user_id,)
        )
        
        # 3. 在 admin_inbox 创建记录
        await db.execute(
            """INSERT OR IGNORE INTO admin_inbox 
               (from_userid, from_username, summary, unread_count, last_active) 
               VALUES (?, ?, ?, 0, CURRENT_TIMESTAMP)""",
            (user_id, p_user['username'] or p_user['first_name'] or str(user_id), "[系统：潜在用户转正]")
        )
        
        # 4. 从 potential_users 移除
        await db.execute("DELETE FROM potential_users WHERE user_id = ?", (user_id,))
        
        await db.commit()
        return True
