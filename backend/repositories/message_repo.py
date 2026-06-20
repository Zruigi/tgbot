# -*- coding: utf-8 -*-
"""
消息 Repository - 消息相关数据库操作
"""
import aiosqlite
import logging
import uuid
from datetime import datetime
from typing import Optional, Dict, Any, List
from config import DB_NAME, ADMIN_ID

logger = logging.getLogger(__name__)


async def get_message_by_tg_id(tg_msg_id: int) -> Optional[Dict[str, Any]]:
    """通过 tg_msg_id 获取消息的完整上下文"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM messages WHERE tg_msg_id = ?", (tg_msg_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def save_message(
    from_userid: int, 
    to_userid: int, 
    content: Optional[str], 
    msg_type: str = 'text', 
    file_id: Optional[str] = None, 
    is_read: bool = False, 
    username: Optional[str] = None, 
    tg_msg_id: Optional[int] = None, 
    translated_text: Optional[str] = None, 
    quoted_text: Optional[str] = None, 
    quoted_msg_id: Optional[int] = None, 
    quoted_file_id: Optional[str] = None, 
    quoted_type: str = 'text', 
    status: str = 'sent', 
    avatar_file_id: Optional[str] = None,
    media_url: Optional[str] = None,
    original_text: Optional[str] = None,
    admin_userid: Optional[str] = None,
    emoji: Optional[str] = None,
    trace_id: Optional[str] = None
) -> int:
    """统一保存消息，并更新管理员收件箱汇总"""
    if not trace_id:
        trace_id = str(uuid.uuid4())
    
    async with aiosqlite.connect(DB_NAME) as db:
        # 1. 存入 messages 主表
        cur = await db.execute(
            """INSERT INTO messages 
               (trace_id, from_userid, to_userid, content, type, file_id, media_url, is_read, tg_msg_id, auto_translation, translated_text, original_text, admin_userid, quoted_text, quoted_msg_id, quoted_file_id, quoted_type, status) 
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (trace_id, from_userid, to_userid, content, msg_type, file_id, media_url, 1 if is_read else 0, tg_msg_id, translated_text, translated_text, original_text, admin_userid, quoted_text, quoted_msg_id, quoted_file_id, quoted_type, status)
        )
        msg_db_id = cur.lastrowid
        
        # 1.1 如果有表情包，存入 emoji_messages
        if emoji:
            await db.execute("INSERT INTO emoji_messages (msg_id, emoji) VALUES (?, ?)", (msg_db_id, emoji))
        
        # 2. 如果是用户发给管理员的消息，更新 admin_inbox 收件箱
        if to_userid == ADMIN_ID:
            # 获取该用户的所有未读消息
            db.row_factory = aiosqlite.Row
            async with db.execute("SELECT content FROM messages WHERE from_userid = ? AND is_read = 0 AND is_deleted = 0 ORDER BY timestamp ASC", (from_userid,)) as cursor:
                unread_rows = await cursor.fetchall()
                unread_msgs = [row['content'] for row in unread_rows]
            
            # 构造摘要 (取最新 3 条)
            summary_lines = []
            for msg_content in unread_msgs[-3:]:
                time_str = datetime.now().strftime("%H:%M")
                display_content = (msg_content[:20] + "...") if msg_content and len(msg_content) > 20 else (msg_content or f"[{msg_type}]")
                summary_lines.append(f"{time_str}: {display_content}")
                
            full_summary = "\n".join(summary_lines)
            if len(full_summary) > 500:
                full_summary = full_summary[:497] + "..."
                
            # 检查是否已存在 inbox 记录
            async with db.execute("SELECT from_userid FROM admin_inbox WHERE from_userid = ?", (from_userid,)) as cursor:
                exists = await cursor.fetchone()
                
            if exists:
                # 无论是否有 username 传入，都尝试更新
                update_parts = ["summary = ?", "unread_count = ?", "last_active = CURRENT_TIMESTAMP"]
                params = [full_summary, len(unread_msgs)]
                
                if username:
                    update_parts.append("from_username = ?")
                    params.append(username)
                
                if avatar_file_id:
                    update_parts.append("avatar_file_id = ?")
                    params.append(avatar_file_id)
                    
                update_sql = f"UPDATE admin_inbox SET {', '.join(update_parts)} WHERE from_userid = ?"
                params.append(from_userid)
                await db.execute(update_sql, tuple(params))
            else:
                # 新用户入库，必须确保有名字
                display_name = username or f"User {from_userid}"
                await db.execute(
                    "INSERT INTO admin_inbox (from_userid, from_username, summary, unread_count, avatar_file_id) VALUES (?, ?, ?, ?, ?)",
                    (from_userid, display_name, full_summary, len(unread_msgs), avatar_file_id)
                )
        await db.commit()
        return msg_db_id


async def update_message_status(msg_db_id: Optional[int], status: str, tg_msg_id: Optional[int] = None) -> None:
    """更新消息的发送状态和 Telegram 消息 ID"""
    if msg_db_id is None:
        return
    async with aiosqlite.connect(DB_NAME) as db:
        if tg_msg_id:
            await db.execute("UPDATE messages SET status = ?, tg_msg_id = ? WHERE msg_id = ?", (status, tg_msg_id, msg_db_id))
        else:
            await db.execute("UPDATE messages SET status = ? WHERE msg_id = ?", (status, msg_db_id))
        await db.commit()


async def delete_single_message(user_id: int, msg_id: int) -> bool:
    """软删除单条消息"""
    async with aiosqlite.connect(DB_NAME) as db:
        try:
            await db.execute(
                "UPDATE messages SET is_deleted = 1 WHERE msg_id = ? AND (from_userid = ? OR to_userid = ?)",
                (msg_id, user_id, user_id)
            )
            await db.commit()
            return True
        except Exception as e:
            logger.error(f"删除消息失败 msg_id={msg_id}: {e}")
            return False


async def search_messages(
    keyword: str, 
    group_id: Optional[int] = None, 
    user_id: Optional[int] = None, 
    limit: int = 30,
    tenant_id: Optional[str] = None,
    role: str = 'merchant_owner',
) -> List[Dict[str, Any]]:
    """全局搜索私聊和群聊消息，支持租户隔离"""
    is_merchant = role == 'merchant_owner' and tenant_id
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if group_id:
            if is_merchant:
                query = """
                    SELECT gm.*, u.username, u.first_name 
                    FROM group_messages gm
                    LEFT JOIN group_members u ON gm.user_id = u.user_id
                    WHERE gm.group_id = ? AND gm.content LIKE ?
                      AND gm.group_id IN (SELECT group_id FROM tracked_groups WHERE tenant_id = ?)
                    ORDER BY gm.message_ts DESC
                    LIMIT ?
                """
                async with db.execute(query, (group_id, f'%{keyword}%', tenant_id, limit)) as cursor:
                    rows = await cursor.fetchall()
                    return [dict(row) for row in rows]
            else:
                query = """
                    SELECT gm.*, u.username, u.first_name 
                    FROM group_messages gm
                    LEFT JOIN group_members u ON gm.user_id = u.user_id
                    WHERE gm.group_id = ? AND gm.content LIKE ?
                    ORDER BY gm.message_ts DESC
                    LIMIT ?
                """
                async with db.execute(query, (group_id, f'%{keyword}%', limit)) as cursor:
                    rows = await cursor.fetchall()
                    return [dict(row) for row in rows]
        elif user_id:
            if is_merchant:
                async with db.execute(
                    "SELECT bot_id FROM merchants WHERE tenant_id = ?",
                    (tenant_id,)
                ) as cursor:
                    bot_row = await cursor.fetchone()
                    bot_id = bot_row[0] if bot_row else None
                if bot_id:
                    query = """
                        SELECT m.*, 'private' as source_type
                        FROM messages m
                        WHERE (m.from_userid = ? OR m.to_userid = ?) 
                          AND m.content LIKE ?
                          AND m.is_deleted = 0
                          AND ((m.from_userid = ? OR m.to_userid = ?) OR (m.from_userid = ? OR m.to_userid = ?))
                        ORDER BY m.timestamp DESC
                        LIMIT ?
                    """
                    async with db.execute(query, (user_id, user_id, f'%{keyword}%', bot_id, bot_id, user_id, user_id, limit)) as cursor:
                        rows = await cursor.fetchall()
                        return [dict(row) for row in rows]
                return []
            else:
                query = """
                    SELECT m.*, 
                           CASE WHEN m.from_userid = ? THEN 'private' ELSE 'private' END as source_type
                    FROM messages m
                    WHERE (m.from_userid = ? OR m.to_userid = ?) 
                       AND m.content LIKE ?
                       AND m.is_deleted = 0
                    ORDER BY m.timestamp DESC
                    LIMIT ?
                """
                async with db.execute(query, (user_id, user_id, user_id, f'%{keyword}%', limit)) as cursor:
                    rows = await cursor.fetchall()
                    return [dict(row) for row in rows]
        else:
            if is_merchant:
                async with db.execute(
                    "SELECT bot_id FROM merchants WHERE tenant_id = ?",
                    (tenant_id,)
                ) as cursor:
                    bot_row = await cursor.fetchone()
                    bot_id = bot_row[0] if bot_row else None

                if bot_id:
                    private_query = """
                        SELECT m.*, 'private' as source_type
                        FROM messages m
                        WHERE m.content LIKE ? AND m.is_deleted = 0
                          AND (m.from_userid = ? OR m.to_userid = ?)
                        ORDER BY m.timestamp DESC
                        LIMIT ?
                    """
                    async with db.execute(private_query, (f'%{keyword}%', bot_id, bot_id, limit)) as cursor:
                        private_rows = await cursor.fetchall()
                else:
                    private_rows = []

                group_query = """
                    SELECT gm.*, u.username, u.first_name, 'group' as source_type
                    FROM group_messages gm
                    LEFT JOIN group_members u ON gm.user_id = u.user_id
                    WHERE gm.content LIKE ?
                      AND gm.group_id IN (SELECT group_id FROM tracked_groups WHERE tenant_id = ?)
                    ORDER BY gm.message_ts DESC
                    LIMIT ?
                """
                async with db.execute(group_query, (f'%{keyword}%', tenant_id, limit)) as cursor:
                    group_rows = await cursor.fetchall()
            else:
                private_query = """
                    SELECT m.*, 'private' as source_type
                    FROM messages m
                    WHERE m.content LIKE ? AND m.is_deleted = 0
                    ORDER BY m.timestamp DESC
                    LIMIT ?
                """
                async with db.execute(private_query, (f'%{keyword}%', limit)) as cursor:
                    private_rows = await cursor.fetchall()

                group_query = """
                    SELECT gm.*, u.username, u.first_name, 'group' as source_type
                    FROM group_messages gm
                    LEFT JOIN group_members u ON gm.user_id = u.user_id
                    WHERE gm.content LIKE ?
                    ORDER BY gm.message_ts DESC
                    LIMIT ?
                """
                async with db.execute(group_query, (f'%{keyword}%', limit)) as cursor:
                    group_rows = await cursor.fetchall()

            results = []
            for row in private_rows:
                d = dict(row)
                d['user_id'] = d.get('from_userid')
                results.append(d)
            for row in group_rows:
                d = dict(row)
                d['group_id'] = d.get('group_id')
                results.append(d)
            
            results.sort(key=lambda x: x.get('timestamp') or x.get('message_ts') or 0, reverse=True)
            return results[:limit]


async def mark_message_as_read(msg_id: int) -> None:
    """标记单条消息为已读"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE messages SET is_read = 1 WHERE msg_id = ?", (msg_id,))
        await db.commit()


async def get_message_count(user_id: int) -> int:
    """获取与某用户的消息总数"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(*) FROM messages WHERE (from_userid = ? OR to_userid = ?) AND is_deleted = 0",
            (user_id, user_id)
        ) as cursor:
            row = await cursor.fetchone()
            return row[0] if row else 0
