# -*- coding: utf-8 -*-
"""
AI 使用量 Repository - AI 对话日志、用量统计（tenant-aware）
"""
import aiosqlite
from typing import Dict, Any, List, Optional
from config import DB_NAME


async def save_ai_usage_log(
    user_id: Optional[int],
    model: str,
    tokens_used: int,
    cost: float,
    endpoint: str = 'chat',
    *,
    channel: str = 'web_ai',
    sandbox_id: str = 'default',
    tenant_id: str = 'default',
    merchant_id: Optional[int] = None,
    feature_key: str = 'general',
    request_trace_id: Optional[str] = None,
    prompt_template_version: Optional[str] = None,
    result_status: str = 'success',
    prompt_tokens: Optional[int] = None,
    completion_tokens: Optional[int] = None,
) -> int:
    """保存 AI 使用日志。"""
    prompt_tokens = int(prompt_tokens if prompt_tokens is not None else max(int(tokens_used or 0), 0))
    completion_tokens = int(completion_tokens if completion_tokens is not None else 0)
    total_tokens = int(tokens_used or 0)

    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute(
            """
            INSERT INTO ai_usage_logs
            (
                channel, user_id, sandbox_id, tenant_id, merchant_id,
                feature_key, request_trace_id, prompt_template_version,
                result_status, endpoint, model,
                prompt_tokens, completion_tokens, total_tokens, estimated_cost_usd, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            """,
            (
                channel,
                user_id,
                sandbox_id,
                tenant_id,
                merchant_id,
                feature_key,
                request_trace_id,
                prompt_template_version,
                result_status,
                endpoint,
                model,
                prompt_tokens,
                completion_tokens,
                total_tokens,
                float(cost or 0.0),
            )
        )
        await db.commit()
        return int(cursor.lastrowid or 0)


async def get_user_usage(user_id: int, days: int = 30, tenant_id: Optional[str] = None) -> Dict[str, Any]:
    """获取用户 AI 使用情况。"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row

        where_clause = "WHERE user_id = ? AND created_at >= datetime('now', ?)"
        params: List[Any] = [user_id, f"-{int(days)} day"]
        if tenant_id:
            where_clause += " AND tenant_id = ?"
            params.append(tenant_id)

        cursor = await db.execute(
            f"""
            SELECT
                COUNT(*) as total_requests,
                COALESCE(SUM(total_tokens), 0) as total_tokens,
                COALESCE(SUM(estimated_cost_usd), 0) as total_cost,
                COALESCE(AVG(estimated_cost_usd), 0) as avg_cost
            FROM ai_usage_logs
            {where_clause}
            """,
            params,
        )
        summary = await cursor.fetchone()

        cursor = await db.execute(
            f"""
            SELECT
                model,
                COUNT(*) as requests,
                COALESCE(SUM(total_tokens), 0) as tokens,
                COALESCE(SUM(estimated_cost_usd), 0) as cost
            FROM ai_usage_logs
            {where_clause}
            GROUP BY model
            ORDER BY cost DESC
            """,
            params,
        )
        by_model = [dict(row) for row in await cursor.fetchall()]

        cursor = await db.execute(
            f"""
            SELECT
                DATE(created_at) as date,
                COUNT(*) as requests,
                COALESCE(SUM(estimated_cost_usd), 0) as cost
            FROM ai_usage_logs
            {where_clause}
            GROUP BY DATE(created_at)
            ORDER BY date DESC
            LIMIT 30
            """,
            params,
        )
        daily_trend = [dict(row) for row in await cursor.fetchall()]

        return {
            'summary': dict(summary) if summary else {},
            'by_model': by_model,
            'daily_trend': daily_trend,
        }


async def get_global_usage_summary(days: int = 7, tenant_id: Optional[str] = None) -> Dict[str, Any]:
    """获取全局 AI 使用摘要。"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row

        where_clause = "WHERE created_at >= datetime('now', ?)"
        params: List[Any] = [f"-{int(days)} day"]
        if tenant_id:
            where_clause += " AND tenant_id = ?"
            params.append(tenant_id)

        cursor = await db.execute(
            f"""
            SELECT
                COUNT(DISTINCT user_id) as unique_users,
                COUNT(*) as total_requests,
                COALESCE(SUM(total_tokens), 0) as total_tokens,
                COALESCE(SUM(estimated_cost_usd), 0) as total_cost
            FROM ai_usage_logs
            {where_clause}
            """,
            params,
        )
        summary = await cursor.fetchone()

        cursor = await db.execute(
            f"""
            SELECT
                user_id,
                COUNT(*) as requests,
                COALESCE(SUM(estimated_cost_usd), 0) as total_cost
            FROM ai_usage_logs
            {where_clause}
            GROUP BY user_id
            ORDER BY total_cost DESC
            LIMIT 10
            """,
            params,
        )
        top_users = [dict(row) for row in await cursor.fetchall()]

        return {
            'summary': dict(summary) if summary else {},
            'top_users': top_users,
        }
