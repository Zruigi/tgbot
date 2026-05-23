# -*- coding: utf-8 -*-
"""
推送任务 Repository - 定时推送任务相关数据库操作
"""
import aiosqlite
import json
import logging
from typing import List, Dict, Any, Optional
from config import DB_NAME

logger = logging.getLogger(__name__)


async def create_push_task(
    task_name: str,
    content: str,
    image_file_id: Optional[str],
    do_translate: bool,
    target_chat_ids: List[int],
    start_at: str,
    interval_hours: int,
    status: str,
    next_run_at: str,
    tenant_id: str = 'default',
    created_by: Optional[int] = None,
) -> int:
    """创建定时推送任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            INSERT INTO push_tasks (task_name, content, image_file_id, do_translate, target_chat_ids, start_at, interval_hours, status, next_run_at, tenant_id, created_by, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            """,
            (task_name, content, image_file_id, 1 if do_translate else 0, json.dumps(target_chat_ids), start_at, interval_hours, status, next_run_at, tenant_id, created_by),
        )
        await db.commit()
        return cur.lastrowid


async def list_push_tasks(tenant_id: Optional[str] = None) -> List[Dict[str, Any]]:
    """列出推送任务，支持租户隔离"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        if tenant_id is not None:
            async with db.execute("SELECT * FROM push_tasks WHERE tenant_id = ? ORDER BY created_at DESC", (tenant_id,)) as cursor:
                rows = await cursor.fetchall()
        else:
            async with db.execute("SELECT * FROM push_tasks ORDER BY created_at DESC") as cursor:
                rows = await cursor.fetchall()
        result = []
        for row in rows:
            item = dict(row)
            try:
                item['target_chat_ids'] = json.loads(item.get('target_chat_ids') or '[]')
            except Exception:
                item['target_chat_ids'] = []
            result.append(item)
        return result


async def get_due_push_tasks(now_iso: str) -> List[Dict[str, Any]]:
    """获取到期待执行任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM push_tasks WHERE status = 'running' AND next_run_at IS NOT NULL AND next_run_at <= ? ORDER BY next_run_at ASC",
            (now_iso,),
        ) as cursor:
            rows = await cursor.fetchall()
            result = []
            for row in rows:
                item = dict(row)
                try:
                    item['target_chat_ids'] = json.loads(item.get('target_chat_ids') or '[]')
                except Exception:
                    item['target_chat_ids'] = []
                result.append(item)
            return result


async def update_push_task_status(task_id: int, status: str) -> bool:
    """更新任务状态"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE push_tasks SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE task_id = ?",
            (status, task_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def update_push_task_next_run(task_id: int, next_run_at: str) -> bool:
    """更新任务下次执行时间"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE push_tasks SET next_run_at = ?, updated_at = CURRENT_TIMESTAMP WHERE task_id = ?",
            (next_run_at, task_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def update_push_task_run_result(task_id: int, run_result: str) -> bool:
    """更新任务执行结果"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE push_tasks SET run_result = ?, updated_at = CURRENT_TIMESTAMP WHERE task_id = ?",
            (run_result, task_id),
        )
        await db.commit()
        return cur.rowcount > 0
