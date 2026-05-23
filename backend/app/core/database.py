# -*- coding: utf-8 -*-
"""
数据库连接池管理
"""
import aiosqlite
import logging
from typing import Optional
from contextlib import asynccontextmanager

logger = logging.getLogger(__name__)


class DatabasePool:
    """数据库连接池"""
    
    _instance: Optional['DatabasePool'] = None
    _db_path: str = 'tgbot.db'
    
    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance
    
    def set_db_path(self, db_path: str):
        """设置数据库路径"""
        self._db_path = db_path
    
    @asynccontextmanager
    async def get_connection(self):
        """获取数据库连接"""
        conn = await aiosqlite.connect(self._db_path)
        try:
            await conn.execute('PRAGMA journal_mode=WAL')
            yield conn
        finally:
            await conn.close()
    
    async def execute(self, sql: str, params: tuple = ()):
        """执行 SQL（快捷方法）"""
        async with self.get_connection() as conn:
            cursor = await conn.execute(sql, params)
            await conn.commit()
            return cursor
    
    async def fetchone(self, sql: str, params: tuple = ()):
        """查询单条记录"""
        async with self.get_connection() as conn:
            conn.row_factory = aiosqlite.Row
            cursor = await conn.execute(sql, params)
            row = await cursor.fetchone()
            return row
    
    async def fetchall(self, sql: str, params: tuple = ()):
        """查询多条记录"""
        async with self.get_connection() as conn:
            conn.row_factory = aiosqlite.Row
            cursor = await conn.execute(sql, params)
            rows = await cursor.fetchall()
            return rows


# 全局实例
db_pool = DatabasePool()
