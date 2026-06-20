# -*- coding: utf-8 -*-
"""
已验证用户 Repository - 已验证用户与提醒功能相关数据库操作
"""
import aiosqlite
import logging
from typing import Optional, List, Dict, Any
from config import DB_NAME

logger = logging.getLogger(__name__)


async def get_verified_users(group_id: int, period: str = 'all') -> List[Dict[str, Any]]:
    """查询群组中已验证的用户列表"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if period == 'today':
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at
                FROM group_members
                WHERE group_id = ?
                  AND is_verified = 1
                  AND DATE(verified_at) = DATE('now')
                ORDER BY verified_at DESC
            '''
        else:
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at
                FROM group_members
                WHERE group_id = ?
                  AND is_verified = 1
                ORDER BY verified_at DESC
            '''
        
        cursor = await db.execute(query, (group_id,))
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


async def get_verify_growth_summary(group_id: Optional[int] = None) -> Dict[str, int]:
    """获取验证增长统计摘要"""
    async with aiosqlite.connect(DB_NAME) as db:
        if group_id:
            cursor = await db.execute('''
                SELECT 
                    COUNT(CASE WHEN DATE(verified_at) = DATE('now') THEN 1 END) as today,
                    COUNT(*) as total
                FROM group_members
                WHERE group_id = ? AND is_verified = 1
            ''', (group_id,))
        else:
            cursor = await db.execute('''
                SELECT 
                    COUNT(CASE WHEN DATE(verified_at) = DATE('now') THEN 1 END) as today,
                    COUNT(*) as total
                FROM group_members
                WHERE is_verified = 1
            ''')
        
        row = await cursor.fetchone()
        return {'today': row[0] if row else 0, 'total': row[1] if row else 0}


async def get_all_verified_users(period: str = 'all') -> List[Dict[str, Any]]:
    """查询所有群组中已验证的用户列表（全局统计）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if period == 'today':
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at,
                    group_id,
                    group_title
                FROM group_members
                WHERE is_verified = 1
                  AND DATE(verified_at) = DATE('now')
                ORDER BY verified_at DESC
            '''
        else:
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at,
                    group_id,
                    group_title
                FROM group_members
                WHERE is_verified = 1
                ORDER BY verified_at DESC
            '''
        
        cursor = await db.execute(query)
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


async def get_unverified_users(group_id: Optional[int] = None, period: str = 'all') -> List[Dict[str, Any]]:
    """查询未关注频道的用户列表"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if period == 'today':
            if group_id:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE group_id = ? AND is_verified = 0
                      AND DATE(joined_at) = DATE('now')
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query, (group_id,))
            else:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE is_verified = 0
                      AND DATE(joined_at) = DATE('now')
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query)
        else:
            if group_id:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE group_id = ? AND is_verified = 0
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query, (group_id,))
            else:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE is_verified = 0
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query)
        
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


async def save_channel_reminder(group_id: int, user_id: int, message_id: int, expires_at: str) -> int:
    """保存频道关注提醒记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute('''
            INSERT INTO channel_follow_reminders 
            (group_id, user_id, message_id, sent_at, expires_at, is_deleted)
            VALUES (?, ?, ?, CURRENT_TIMESTAMP, ?, 0)
        ''', (group_id, user_id, message_id, expires_at))
        await db.commit()
        return cursor.lastrowid


async def get_expired_reminders() -> List[Dict[str, Any]]:
    """获取已过期的提醒记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute('''
            SELECT * FROM channel_follow_reminders
            WHERE is_deleted = 0 AND expires_at < CURRENT_TIMESTAMP
        ''')
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


async def mark_reminder_deleted(reminder_id: int) -> None:
    """标记提醒为已删除"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute('''
            UPDATE channel_follow_reminders
            SET is_deleted = 1
            WHERE id = ?
        ''', (reminder_id,))
        await db.commit()


async def get_reminder_by_user_group(user_id: int, group_id: int) -> Optional[Dict[str, Any]]:
    """获取用户在特定群组的提醒记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute('''
            SELECT * FROM channel_follow_reminders
            WHERE user_id = ? AND group_id = ? AND is_deleted = 0
            ORDER BY sent_at DESC LIMIT 1
        ''', (user_id, group_id))
        row = await cursor.fetchone()
        return dict(row) if row else None


async def get_group_member(group_id: int, user_id: int) -> Optional[Dict[str, Any]]:
    """获取群组成员信息"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute('''
            SELECT user_id, username, first_name, last_name, avatar_file_id
            FROM group_members
            WHERE group_id = ? AND user_id = ?
            LIMIT 1
        ''', (group_id, user_id))
        row = await cursor.fetchone()
        return dict(row) if row else None
