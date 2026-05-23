# -*- coding: utf-8 -*-
"""
用户 Repository - 用户相关数据库操作
"""
import aiosqlite
import logging
from datetime import datetime, timedelta
from typing import Optional, Dict, Any, List
from config import DB_NAME, ADMIN_ID

logger = logging.getLogger(__name__)


async def get_user(user_id: int) -> Optional[Dict[str, Any]]:
    """获取用户信息"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM users WHERE user_id = ?", (user_id,)) as cursor:
            return await cursor.fetchone()


async def create_or_update_user(user_id: int, email: Optional[str] = None) -> None:
    """创建或更新用户"""
    async with aiosqlite.connect(DB_NAME) as db:
        user = await get_user(user_id)
        if user:
            if email is not None:
                await db.execute("UPDATE users SET email = ? WHERE user_id = ?", (email, user_id))
        else:
            await db.execute("INSERT INTO users (user_id, email) VALUES (?, ?)", (user_id, email))
        await db.commit()


async def update_user_avatar(user_id: int, avatar_file_id: str) -> None:
    """更新用户头像缓存"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE admin_inbox SET avatar_file_id = ? WHERE from_userid = ?",
            (avatar_file_id, user_id)
        )
        await db.commit()


async def get_inbox_list() -> List[Dict[str, Any]]:
    """获取来访用户列表 (含自动重建逻辑)"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM admin_inbox ORDER BY last_active DESC") as cursor:
            rows = await cursor.fetchall()
            if rows:
                logger.info(f"Admin Inbox: 找到 {len(rows)} 个用户")
                return [dict(row) for row in rows]
            
            # 如果收件箱为空，尝试从 messages 表重建
            logger.warning("Admin Inbox 为空，尝试从 messages 表自动重建...")
            try:
                await rebuild_inbox_from_messages()
                
                # 重新查询
                async with db.execute("SELECT * FROM admin_inbox ORDER BY last_active DESC") as cursor:
                    rows = await cursor.fetchall()
                    logger.info(f"Admin Inbox 重建完成：找到 {len(rows)} 个用户")
                    return [dict(row) for row in rows]
            except Exception as e:
                logger.error(f"重建 Admin Inbox 失败：{e}")
                return []


async def rebuild_inbox_from_messages() -> None:
    """从 messages 历史记录中重建 admin_inbox 汇总表"""
    async with aiosqlite.connect(DB_NAME) as db:
        # 1. 查找所有给管理员发过消息的用户，以及管理员回复过的用户
        query = """
            SELECT 
                CASE WHEN from_userid = ? THEN to_userid ELSE from_userid END as uid,
                MAX(timestamp) as last_ts
            FROM messages
            WHERE from_userid = ? OR to_userid = ?
            GROUP BY uid
        """
        async with db.execute(query, (ADMIN_ID, ADMIN_ID, ADMIN_ID)) as cursor:
            users = await cursor.fetchall()
        
        logger.info(f"找到 {len(users)} 个与管理员有过消息往来的用户")
        
        if not users:
            logger.warning("messages 表中没有找到相关记录")
            return
        
        count = 0
        for user in users:
            uid, last_ts = user
            
            # 尝试从 potential_users 获取用户名
            uname = None
            async with db.execute("SELECT username, first_name FROM potential_users WHERE user_id = ?", (uid,)) as c:
                p_row = await c.fetchone()
                if p_row:
                    uname = p_row[0] or p_row[1]
            
            if not uname:
                # 尝试从 group_members 获取
                async with db.execute("SELECT username, first_name FROM group_members WHERE user_id = ?", (uid,)) as c:
                    m_row = await c.fetchone()
                    if m_row:
                        uname = m_row[0] or m_row[1]
            
            # 获取未读数
            async with db.execute(
                "SELECT COUNT(*) FROM messages WHERE from_userid = ? AND to_userid = ? AND is_read = 0",
                (uid, ADMIN_ID)
            ) as c:
                unread = (await c.fetchone())[0]
                
            # 获取最新一条消息内容
            async with db.execute(
                "SELECT content FROM messages WHERE (from_userid = ? AND to_userid = ?) OR (from_userid = ? AND to_userid = ?) ORDER BY timestamp DESC LIMIT 1",
                (uid, ADMIN_ID, ADMIN_ID, uid)
            ) as c:
                msg = await c.fetchone()
                summary = msg[0][:30] + "..." if msg and msg[0] else "[媒体消息]"

            await db.execute(
                """INSERT OR REPLACE INTO admin_inbox 
                   (from_userid, from_username, summary, unread_count, last_active)
                   VALUES (?, ?, ?, ?, ?)""",
                (uid, uname or f"User {uid}", summary, unread, last_ts)
            )
            count += 1
        
        await db.commit()
        logger.info(f"成功重建 {count}/{len(users)} 个用户的 Inbox 记录")
        logger.info(f"✅ 已成功从历史记录重建 {len(users)} 个会话入口")


async def get_latest_active_user() -> Optional[Dict[str, Any]]:
    """获取最近活跃的用户 ID"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT from_userid, from_username FROM admin_inbox ORDER BY last_active DESC LIMIT 1") as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def get_recent_active_count(minutes: int = 5) -> int:
    """获取指定分钟内有活跃咨询的用户数量"""
    threshold_time = (datetime.now() - timedelta(minutes=minutes)).isoformat()
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(DISTINCT from_userid) FROM admin_inbox WHERE last_active > ?",
            (threshold_time,)
        ) as cursor:
            row = await cursor.fetchone()
            return row[0] if row else 0


async def get_chat_history(user_id: int, limit: int = 50, offset: int = 0) -> List[Dict[str, Any]]:
    """获取与特定用户的聊天历史"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        query = """
            SELECT m.*, e.emoji 
            FROM messages m
            LEFT JOIN emoji_messages e ON m.msg_id = e.msg_id
            WHERE ((m.from_userid = ? AND m.to_userid = ?) 
               OR (m.from_userid = ? AND m.to_userid = ?))
               AND m.is_deleted = 0
            ORDER BY m.timestamp DESC
            LIMIT ? OFFSET ?
        """
        async with db.execute(query, (user_id, ADMIN_ID, ADMIN_ID, user_id, limit, offset)) as cursor:
            rows = await cursor.fetchall()
            messages = [dict(row) for row in rows]
            messages.reverse()  # 前端需要按时间顺序列出（旧到新）
            return messages


async def mark_messages_as_read(from_userid: int) -> None:
    """将特定用户发来的消息标记为已读"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE messages SET is_read = 1 WHERE from_userid = ?", (from_userid,))
        await db.execute("UPDATE admin_inbox SET unread_count = 0, summary = '' WHERE from_userid = ?", (from_userid,))
        await db.commit()


async def soft_delete_chat(user_id: int) -> None:
    """软删除会话（隐藏而不物理删除）"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE admin_inbox SET is_hidden = 1 WHERE from_userid = ?", (user_id,))
        await db.commit()


async def get_active_users_for_selection() -> List[Dict[str, Any]]:
    """2026 规格：获取当前活跃用户列表（用于并发检测）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("""
            SELECT from_userid, from_username, last_active, unread_count 
            FROM admin_inbox 
            WHERE last_active > datetime('now', '-5 minutes')
            ORDER BY last_active DESC
        """) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_tenant_user_ids(tenant_id: str, merchant_id: int) -> set:
    """获取与指定租户关联的所有用户 ID 集合"""
    async with aiosqlite.connect(DB_NAME) as db:
        user_ids = set()
        async with db.execute(
            "SELECT user_id FROM potential_users WHERE tenant_id = ?",
            (tenant_id,)
        ) as cursor:
            for row in await cursor.fetchall():
                user_ids.add(row[0])
        async with db.execute(
            "SELECT user_id FROM group_members WHERE group_id IN (SELECT group_id FROM tracked_groups WHERE tenant_id = ?)",
            (tenant_id,)
        ) as cursor:
            for row in await cursor.fetchall():
                user_ids.add(row[0])
        async with db.execute(
            "SELECT bot_id FROM merchants WHERE id = ?",
            (merchant_id,)
        ) as cursor:
            bot_row = await cursor.fetchone()
            if bot_row and bot_row[0]:
                bot_id = bot_row[0]
                async with db.execute(
                    "SELECT from_userid FROM messages WHERE to_userid = ? AND is_deleted = 0",
                    (bot_id,)
                ) as cursor:
                    for row in await cursor.fetchall():
                        user_ids.add(row[0])
                async with db.execute(
                    "SELECT to_userid FROM messages WHERE from_userid = ? AND is_deleted = 0",
                    (bot_id,)
                ) as cursor:
                    for row in await cursor.fetchall():
                        user_ids.add(row[0])
        user_ids.discard(None)
        return user_ids


async def get_inbox_list_for_tenant(tenant_id: str, merchant_id: int) -> List[Dict[str, Any]]:
    """获取与指定租户相关的收件箱数据（间接隔离）"""
    tenant_user_ids = await get_tenant_user_ids(tenant_id, merchant_id)
    if not tenant_user_ids:
        return []
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        placeholders = ','.join('?' * len(tenant_user_ids))
        async with db.execute(
            f"SELECT * FROM admin_inbox WHERE from_userid IN ({placeholders}) ORDER BY last_active DESC",
            list(tenant_user_ids)
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def is_user_belong_to_tenant(user_id: int, tenant_id: str, merchant_id: int) -> bool:
    """验证用户是否属于指定租户（策略链）"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT 1 FROM potential_users WHERE user_id = ? AND tenant_id = ? LIMIT 1",
            (user_id, tenant_id)
        ) as cursor:
            if await cursor.fetchone():
                return True
        async with db.execute(
            "SELECT 1 FROM group_members WHERE user_id = ? AND group_id IN (SELECT group_id FROM tracked_groups WHERE tenant_id = ?) LIMIT 1",
            (user_id, tenant_id)
        ) as cursor:
            if await cursor.fetchone():
                return True
        async with db.execute(
            "SELECT bot_id FROM merchants WHERE id = ?",
            (merchant_id,)
        ) as cursor:
            bot_row = await cursor.fetchone()
            if bot_row and bot_row[0]:
                bot_id = bot_row[0]
                async with db.execute(
                    "SELECT 1 FROM messages WHERE (from_userid = ? AND to_userid = ?) OR (from_userid = ? AND to_userid = ?) AND is_deleted = 0 LIMIT 1",
                    (user_id, bot_id, bot_id, user_id)
                ) as cursor:
                    if await cursor.fetchone():
                        return True
    return False
