# -*- coding: utf-8 -*-
"""
审计 Repository - 租户操作审计日志
"""
import aiosqlite
import logging
from typing import Optional, Dict, Any, List
from config import DB_NAME

logger = logging.getLogger(__name__)


async def create_audit_log(
    tenant_id: str,
    action: str,
    *,
    merchant_id: Optional[int] = None,
    operator_role: str = 'system',
    operator_id: Optional[str] = None,
    resource_type: Optional[str] = None,
    resource_id: Optional[str] = None,
    payload_json: Optional[str] = None,
    trace_id: Optional[str] = None,
) -> int:
    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute(
            """
            INSERT INTO tenant_audit_logs
            (
                tenant_id, merchant_id, operator_role, operator_id,
                action, resource_type, resource_id, payload_json,
                trace_id, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            """,
            (
                tenant_id,
                merchant_id,
                operator_role,
                operator_id,
                action,
                resource_type,
                resource_id,
                payload_json,
                trace_id,
            ),
        )
        await db.commit()
        return int(cursor.lastrowid or 0)


async def list_audit_logs(
    tenant_id: str,
    *,
    action: Optional[str] = None,
    resource_type: Optional[str] = None,
    limit: int = 100,
) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM tenant_audit_logs WHERE tenant_id = ?"
        params: List[Any] = [tenant_id]
        if action:
            sql += " AND action = ?"
            params.append(action)
        if resource_type:
            sql += " AND resource_type = ?"
            params.append(resource_type)
        sql += " ORDER BY created_at DESC, id DESC LIMIT ?"
        params.append(int(limit))
        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_audit_log(log_id: int) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM tenant_audit_logs WHERE id = ?", (log_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def log_tenant_isolation_event(
    tenant_id: str,
    action: str,
    resource_type: str,
    resource_id: str,
    operator_role: str,
    request_path: str,
) -> None:
    """记录租户隔离事件到审计日志，写入失败不阻断业务"""
    try:
        await create_audit_log(
            tenant_id=tenant_id,
            action=action,
            operator_role=operator_role,
            resource_type=resource_type,
            resource_id=resource_id,
            payload_json=f'{{"request_path": "{request_path}"}}',
        )
    except Exception as e:
        logger.warning(f"[TenantIsolation] 审计日志写入失败: {e}, action={action}, tenant_id={tenant_id}")
