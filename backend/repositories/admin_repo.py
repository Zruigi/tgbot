# -*- coding: utf-8 -*-
"""
管理员 Repository - 管理员配置与 AI 用量相关数据库操作
"""
import aiosqlite
import logging
from typing import Optional, Dict, Any
from config import DB_NAME

logger = logging.getLogger(__name__)


async def get_admin_setting(key: str, default: Optional[str] = None) -> Optional[str]:
    """获取管理员设置"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT value FROM admin_settings WHERE key = ?", (key,)) as cursor:
            row = await cursor.fetchone()
            return row[0] if row else default


async def save_admin_setting(key: str, value: str) -> None:
    """更新管理员设置"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("INSERT OR REPLACE INTO admin_settings (key, value) VALUES (?, ?)", (key, str(value)))
        await db.commit()


async def update_admin_info(admin_id: int, username: str, worker_no: Optional[str] = None) -> None:
    """更新管理员信息"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "INSERT OR REPLACE INTO admin_users (admin_id, username, worker_no, role, is_online) VALUES (?, ?, ?, ?, ?)",
            (admin_id, username, worker_no or 'CS001', 'staff', 1)
        )
        await db.commit()


async def get_admin_info(admin_id: int) -> Optional[Dict[str, Any]]:
    """获取管理员信息"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM admin_users WHERE admin_id = ?", (admin_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else {'admin_id': admin_id, 'username': 'Administrator', 'worker_no': 'CS001'}


async def get_admin_by_tg_id(admin_id: int) -> Optional[Dict[str, Any]]:
    """按 Telegram 用户 ID 查询管理员。"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM admin_users WHERE admin_id = ? LIMIT 1", (admin_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def save_ai_usage_log(
    channel: str,
    user_id: Optional[int],
    sandbox_id: str,
    model: str,
    prompt_tokens: int = 0,
    completion_tokens: int = 0,
    total_tokens: int = 0,
    estimated_cost_usd: float = 0.0,
) -> None:
    """记录 AI 调用用量"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO ai_usage_logs
            (channel, user_id, sandbox_id, model, prompt_tokens, completion_tokens, total_tokens, estimated_cost_usd)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (channel, user_id, sandbox_id, model, int(prompt_tokens or 0), int(completion_tokens or 0), int(total_tokens or 0), float(estimated_cost_usd or 0.0)),
        )
        await db.commit()


async def get_ai_usage_summary(days: int = 7) -> Dict[str, Any]:
    """汇总 AI 用量（按最近 N 天）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT
                COUNT(*) AS calls,
                COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
                COALESCE(SUM(completion_tokens), 0) AS completion_tokens,
                COALESCE(SUM(total_tokens), 0) AS total_tokens,
                COALESCE(SUM(estimated_cost_usd), 0) AS estimated_cost_usd
            FROM ai_usage_logs
            WHERE created_at >= datetime('now', ?)
            """,
            (f"-{int(days)} day",),
        ) as cursor:
            summary_row = await cursor.fetchone()

        async with db.execute(
            """
            SELECT model, COUNT(*) AS calls, COALESCE(SUM(total_tokens), 0) AS total_tokens
            FROM ai_usage_logs
            WHERE created_at >= datetime('now', ?)
            GROUP BY model
            ORDER BY total_tokens DESC
            LIMIT 10
            """,
            (f"-{int(days)} day",),
        ) as cursor:
            model_rows = await cursor.fetchall()

        async with db.execute(
            """
            SELECT sandbox_id, COUNT(*) AS calls, COALESCE(SUM(total_tokens), 0) AS total_tokens
            FROM ai_usage_logs
            WHERE created_at >= datetime('now', ?)
            GROUP BY sandbox_id
            ORDER BY total_tokens DESC
            LIMIT 20
            """,
            (f"-{int(days)} day",),
        ) as cursor:
            sandbox_rows = await cursor.fetchall()

    return {
        "calls": int((summary_row or {}).get("calls", 0) if summary_row else 0),
        "prompt_tokens": int((summary_row or {}).get("prompt_tokens", 0) if summary_row else 0),
        "completion_tokens": int((summary_row or {}).get("completion_tokens", 0) if summary_row else 0),
        "total_tokens": int((summary_row or {}).get("total_tokens", 0) if summary_row else 0),
        "estimated_cost_usd": float((summary_row or {}).get("estimated_cost_usd", 0.0) if summary_row else 0.0),
        "top_models": [dict(r) for r in (model_rows or [])],
        "top_sandboxes": [dict(r) for r in (sandbox_rows or [])],
    }


async def save_oneapi_key(name: str, api_key: str, base_url: str, is_active: bool = True) -> None:
    """保存 OneAPI 密钥"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT OR REPLACE INTO oneapi_keys (name, api_key, base_url, is_active, created_at)
            VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)
            """,
            (name, api_key, base_url, 1 if is_active else 0),
        )
        await db.commit()


async def get_oneapi_key(name: str) -> Optional[Dict[str, Any]]:
    """获取 OneAPI 密钥"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM oneapi_keys WHERE name = ? AND is_active = 1", (name,)
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def list_oneapi_keys() -> list:
    """列出所有 OneAPI 密钥"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM oneapi_keys ORDER BY created_at DESC") as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def deactivate_oneapi_key(key_id: int) -> None:
    """停用 OneAPI 密钥"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE oneapi_keys SET is_active = 0 WHERE id = ?", (key_id,))
        await db.commit()
