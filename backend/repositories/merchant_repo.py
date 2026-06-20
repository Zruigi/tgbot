# -*- coding: utf-8 -*-
"""
商户 Repository - 平台域商户主档与租户版本信息
"""
import aiosqlite
from typing import Optional, Dict, Any, List, Tuple
from config import DB_NAME


def _enforce_tenant_filter(sql: str, params: List[Any], tenant_id: Optional[str], require_tenant: bool = True) -> Tuple[str, List[Any]]:
    """
    [已废弃] 请使用 backend.app.core.scope.enforce_tenant_filter（基于 SessionScope 的统一版本）
    保留此函数以兼容现有调用方，内部委托到统一版本
    """
    from backend.app.core.scope import SessionScope, enforce_tenant_filter as _unified_etf
    scope_type = 'merchant' if tenant_id else ''
    scope = SessionScope(
        authenticated=True,
        actor_role='merchant_owner' if tenant_id else '',
        effective_role='merchant_owner' if tenant_id else '',
        scope_type=scope_type,
        tenant_id=tenant_id or '',
        merchant_id=None,
    )
    return _unified_etf(sql, params, scope, require_tenant)


def _enforce_merchant_filter(sql: str, params: List[Any], merchant_id: Optional[int], require_merchant: bool = True) -> Tuple[str, List[Any]]:
    """
    强制在 SQL 查询中添加 merchant_id 过滤条件。
    - 如果 merchant_id 为空且 require_merchant=True，抛出异常
    - 如果 merchant_id 为空且 require_merchant=False，返回原查询
    """
    if merchant_id is None:
        if require_merchant:
            raise ValueError("merchant_id is required but not provided - potential data leak prevention")
        return sql, params

    if ' WHERE ' in sql.upper():
        return sql + " AND merchant_id = ?", params + [merchant_id]
    else:
        return sql + " WHERE merchant_id = ?", params + [merchant_id]


async def create_merchant(
    tenant_id: str,
    owner_tg_user_id: int,
    merchant_name: str,
    *,
    bot_id: Optional[int] = None,
    bot_username: Optional[str] = None,
    status: str = 'active_free',
    plan_type: str = 'free',
    subscription_status: str = 'free',
) -> int:
    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute(
            """
            INSERT INTO merchants
            (
                tenant_id, owner_tg_user_id, merchant_name, bot_id, bot_username,
                status, plan_type, subscription_status, runtime_version, config_version, feature_version,
                created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, 1, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            """,
            (tenant_id, owner_tg_user_id, merchant_name, bot_id, bot_username, status, plan_type, subscription_status),
        )
        await db.commit()
        return int(cursor.lastrowid or 0)


async def get_merchant_by_id(merchant_id: int) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM merchants WHERE id = ?", (merchant_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def get_merchant_by_tenant_id(tenant_id: str) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM merchants WHERE tenant_id = ?", (tenant_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def get_merchant_by_owner(owner_tg_user_id: int) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM merchants WHERE owner_tg_user_id = ? ORDER BY id DESC LIMIT 1",
            (owner_tg_user_id,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def list_merchants(limit: int = 100, offset: int = 0, status: Optional[str] = None) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM merchants"
        params: List[Any] = []
        if status:
            sql += " WHERE status = ?"
            params.append(status)
        sql += " ORDER BY updated_at DESC, id DESC LIMIT ? OFFSET ?"
        params.extend([int(limit), int(offset)])
        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def update_merchant_bot(merchant_id: int, bot_id: Optional[int], bot_username: Optional[str]) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE merchants
            SET bot_id = ?, bot_username = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            """,
            (bot_id, bot_username, merchant_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def update_merchant(merchant_id: int, data: Dict[str, Any]) -> bool:
    if not data:
        return False

    set_clauses: List[str] = []
    params: List[Any] = []
    for key, value in data.items():
        set_clauses.append(f"{key} = ?")
        params.append(value)

    set_clauses.append("updated_at = CURRENT_TIMESTAMP")
    params.append(merchant_id)

    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            f"UPDATE merchants SET {', '.join(set_clauses)} WHERE id = ?",
            params,
        )
        await db.commit()
        return cur.rowcount > 0


async def update_merchant_plan(
    merchant_id: int,
    plan_type: str,
    subscription_status: str,
    subscription_expires_at: Optional[str] = None,
    status: Optional[str] = None,
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE merchants
            SET plan_type = ?,
                subscription_status = ?,
                subscription_expires_at = ?,
                status = COALESCE(?, status),
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            """,
            (plan_type, subscription_status, subscription_expires_at, status, merchant_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def bump_runtime_version(merchant_id: int) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE merchants SET runtime_version = runtime_version + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            (merchant_id,),
        )
        await db.commit()
        return cur.rowcount > 0


async def bump_config_version(merchant_id: int) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE merchants SET config_version = config_version + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            (merchant_id,),
        )
        await db.commit()
        return cur.rowcount > 0


async def bump_feature_version(merchant_id: int) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE merchants SET feature_version = feature_version + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            (merchant_id,),
        )
        await db.commit()
        return cur.rowcount > 0


# === 带租户隔离的查询函数 ===

async def list_merchants_by_tenant(
    tenant_id: str,
    limit: int = 100,
    offset: int = 0,
    status: Optional[str] = None
) -> List[Dict[str, Any]]:
    """
    按租户 ID 查询商户列表（强制 tenant_id 过滤）
    """
    if not tenant_id:
        raise ValueError("tenant_id is required for tenant-scoped query")

    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM merchants WHERE tenant_id = ?"
        params: List[Any] = [tenant_id]

        if status:
            sql += " AND status = ?"
            params.append(status)

        sql += " ORDER BY updated_at DESC, id DESC LIMIT ? OFFSET ?"
        params.extend([int(limit), int(offset)])

        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_merchant_by_id_for_tenant(
    merchant_id: int,
    tenant_id: str,
) -> Optional[Dict[str, Any]]:
    """
    按商户 ID 和租户 ID 查询（双重验证，防止越权访问）
    """
    if not tenant_id:
        raise ValueError("tenant_id is required for tenant-scoped query")

    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM merchants WHERE id = ? AND tenant_id = ?",
            (merchant_id, tenant_id),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def update_merchant_by_tenant(
    merchant_id: int,
    tenant_id: str,
    **kwargs
) -> bool:
    """
    按商户 ID 和租户 ID 更新（防止越权修改）
    """
    if not tenant_id:
        raise ValueError("tenant_id is required for tenant-scoped update")

    if not kwargs:
        return False

    set_clauses = [f"{k} = ?" for k in kwargs.keys()]
    set_clauses.append("updated_at = CURRENT_TIMESTAMP")

    sql = f"UPDATE merchants SET {', '.join(set_clauses)} WHERE id = ? AND tenant_id = ?"
    params = list(kwargs.values()) + [merchant_id, tenant_id]

    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(sql, params)
        await db.commit()
        return cur.rowcount > 0
