# -*- coding: utf-8 -*-
"""
已验证用户列表 API 端点
"""
import aiosqlite
import logging
from datetime import datetime
from config import DB_NAME

logger = logging.getLogger(__name__)


async def get_verified_users(group_id: int, period: str = 'all') -> list:
    """
    查询群组中已验证的用户列表
    
    Args:
        group_id: 群组 ID
        period: 'today' 或 'all'
    
    Returns:
        已验证用户列表，包含 user_id, username, first_name, avatar_file_id, verified_at
    """
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if period == 'today':
            # 查询今天验证的用户
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
        else:  # 'all'
            # 查询所有已验证的用户
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


async def get_verify_growth_summary(group_id: int = None) -> dict:
    """
    获取验证增长统计摘要
    
    Args:
        group_id: 可选，特定群组 ID；如果为 None，返回全局统计
    
    Returns:
        {'today': int, 'total': int}
    """
    async with aiosqlite.connect(DB_NAME) as db:
        if group_id:
            # 特定群组统计
            cursor = await db.execute('''
                SELECT 
                    COUNT(CASE WHEN DATE(verified_at) = DATE('now') THEN 1 END) as today,
                    COUNT(*) as total
                FROM group_members
                WHERE group_id = ? AND is_verified = 1
            ''', (group_id,))
        else:
            # 全局统计
            cursor = await db.execute('''
                SELECT 
                    COUNT(CASE WHEN DATE(verified_at) = DATE('now') THEN 1 END) as today,
                    COUNT(*) as total
                FROM group_members
                WHERE is_verified = 1
            ''')
        
        row = await cursor.fetchone()
        return {
            'today': row[0] if row else 0,
            'total': row[1] if row else 0
        }


async def mark_user_verified(group_id: int, user_id: int) -> bool:
    """
    标记用户为已验证
    
    Args:
        group_id: 群组 ID
        user_id: 用户 ID
    
    Returns:
        是否成功
    """
    async with aiosqlite.connect(DB_NAME) as db:
        try:
            await db.execute('''
                UPDATE group_members
                SET is_verified = 1, verified_at = CURRENT_TIMESTAMP
                WHERE group_id = ? AND user_id = ?
            ''', (group_id, user_id))
            await db.commit()
            return True
        except Exception as e:
            logger.error(f"标记用户验证失败: {e}")
            return False
