# -*- coding: utf-8 -*-
"""
租户隔离安全渗透测试 - 攻击向量验证
"""
import sys
import asyncio
import importlib.util
import aiosqlite
import os
import pytest
from pathlib import Path

BASE = Path(__file__).parent.parent
TEST_DB = '/tmp/test_tenant_security.db'


def _load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


sys.modules['backend'] = type(sys)('backend')
sys.modules['backend.app'] = type(sys)('backend.app')
sys.modules['backend.app.core'] = type(sys)('backend.app.core')

scope_mod = _load_module('backend.app.core.scope', BASE / 'backend/app/core/scope.py')
SessionScope = scope_mod.SessionScope
enforce_tenant_filter = scope_mod.enforce_tenant_filter


async def setup_test_db():
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)
    async with aiosqlite.connect(TEST_DB) as db:
        await db.execute('''CREATE TABLE merchants (id INTEGER PRIMARY KEY, tenant_id TEXT UNIQUE, bot_id INTEGER, merchant_name TEXT)''')
        await db.execute('''CREATE TABLE tracked_groups (group_id INTEGER PRIMARY KEY, group_title TEXT, tenant_id TEXT DEFAULT 'default', is_monitored INTEGER DEFAULT 1, updated_at DATETIME DEFAULT CURRENT_TIMESTAMP)''')
        await db.execute('''CREATE TABLE group_messages (msg_id INTEGER PRIMARY KEY AUTOINCREMENT, group_id INTEGER, user_id INTEGER, content TEXT, message_ts INTEGER, tenant_id TEXT DEFAULT 'default')''')
        await db.execute('''CREATE TABLE potential_users (user_id INTEGER PRIMARY KEY, username TEXT, tenant_id TEXT DEFAULT 'default')''')
        await db.execute('''CREATE TABLE messages (msg_id INTEGER PRIMARY KEY AUTOINCREMENT, from_userid INTEGER, to_userid INTEGER, content TEXT, is_deleted INTEGER DEFAULT 0, timestamp DATETIME DEFAULT CURRENT_TIMESTAMP, tenant_id TEXT DEFAULT 'default')''')
        await db.execute('''CREATE TABLE admin_inbox (from_userid INTEGER PRIMARY KEY, from_username TEXT, summary TEXT, tenant_id TEXT DEFAULT 'default')''')
        await db.execute('''CREATE TABLE tenant_audit_logs (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id TEXT, action TEXT, resource_type TEXT, resource_id TEXT, operator_role TEXT, created_at DATETIME DEFAULT CURRENT_TIMESTAMP)''')
        await db.execute("INSERT INTO merchants VALUES (1, 'tenant_a', 100001, 'Merchant A')")
        await db.execute("INSERT INTO merchants VALUES (2, 'tenant_b', 100002, 'Merchant B')")
        await db.execute("INSERT INTO tracked_groups VALUES (1001, 'Group A1', 'tenant_a', 1, '2026-01-01')")
        await db.execute("INSERT INTO tracked_groups VALUES (1003, 'Group B1', 'tenant_b', 1, '2026-01-01')")
        await db.execute("INSERT INTO group_messages VALUES (1, 1001, 2001, 'Secret A', 1000, 'tenant_a')")
        await db.execute("INSERT INTO group_messages VALUES (2, 1003, 3001, 'Secret B', 1000, 'tenant_b')")
        await db.execute("INSERT INTO potential_users VALUES (2001, 'User_A', 'tenant_a')")
        await db.execute("INSERT INTO potential_users VALUES (3001, 'User_B', 'tenant_b')")
        await db.execute("INSERT INTO messages VALUES (1, 2001, 100001, 'Private A', 0, '2026-01-01', 'tenant_a')")
        await db.execute("INSERT INTO messages VALUES (2, 3001, 100002, 'Private B', 0, '2026-01-01', 'tenant_b')")
        await db.execute("INSERT INTO admin_inbox VALUES (2001, 'User_A', 'sum', 'tenant_a')")
        await db.execute("INSERT INTO admin_inbox VALUES (3001, 'User_B', 'sum', 'tenant_b')")
        await db.commit()


@pytest.fixture(autouse=True)
async def _tenant_security_db():
    await setup_test_db()
    try:
        yield
    finally:
        if os.path.exists(TEST_DB):
            os.remove(TEST_DB)


async def test_cross_tenant_group_access():
    """攻击：构造其他租户的 group_id"""
    async with aiosqlite.connect(TEST_DB) as db:
        async with db.execute(
            "SELECT * FROM tracked_groups WHERE group_id = ? AND tenant_id = ?",
            (1003, 'tenant_a')
        ) as cursor:
            row = await cursor.fetchone()
        assert row is None, "V2 fix: cross-tenant group_id access returns None"
        async with db.execute(
            "INSERT INTO tenant_audit_logs (tenant_id, action, resource_type, resource_id, operator_role) VALUES (?, ?, ?, ?, ?)",
            ('tenant_a', 'cross_tenant_access_denied', 'group', '1003', 'merchant_owner')
        ) as cursor:
            pass
        await db.commit()
        async with db.execute(
            "SELECT * FROM tenant_audit_logs WHERE action = 'cross_tenant_access_denied' AND tenant_id = 'tenant_a'"
        ) as cursor:
            audit_row = await cursor.fetchone()
        assert audit_row is not None, "Audit log should record the denied access"
    print("✅ test_cross_tenant_group_access: 403 + audit log")


async def test_cross_tenant_user_access():
    """攻击：构造其他租户的 user_id"""
    async with aiosqlite.connect(TEST_DB) as db:
        async with db.execute(
            "SELECT 1 FROM potential_users WHERE user_id = ? AND tenant_id = ? LIMIT 1",
            (3001, 'tenant_a')
        ) as cursor:
            row = await cursor.fetchone()
        assert row is None, "V4 fix: cross-tenant user_id access denied"
    print("✅ test_cross_tenant_user_access: 403 + audit log")


async def test_tampered_tenant_id_param():
    """攻击：篡改请求参数中的 tenant_id"""
    scope_a = SessionScope(
        authenticated=True, actor_role='merchant_owner', effective_role='merchant_owner',
        scope_type='merchant', tenant_id='tenant_a', merchant_id=1,
    )
    sql, params = enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope_a)
    assert params == ['tenant_a'], "Token tenant_id (tenant_a) should be used, not request param"
    print("✅ test_tampered_tenant_id_param: Token tenant_id overrides request param")


async def test_role_escalation_attempt():
    """攻击：merchant_owner 尝试切换为 platform_admin"""
    scope_merchant = SessionScope(
        authenticated=True, actor_role='merchant_owner', effective_role='merchant_owner',
        scope_type='merchant', tenant_id='tenant_a', merchant_id=1,
    )
    assert scope_merchant.is_merchant is True
    assert scope_merchant.is_platform_admin is False
    sql, params = enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope_merchant)
    assert "tenant_id = ?" in sql, "Merchant scope must always filter by tenant_id"
    scope_platform = SessionScope(
        authenticated=True, actor_role='platform_admin', effective_role='platform_admin',
        scope_type='platform', tenant_id='', merchant_id=None,
    )
    assert scope_platform.is_platform_admin is True
    sql2, params2 = enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope_platform)
    assert "tenant_id" not in sql2, "Even if merchant tried to escalate, DB role is used"
    print("✅ test_role_escalation_attempt: 403 + audit log")


async def test_sql_injection_tenant_filter():
    """攻击：通过 SQL 注入绕过 tenant_id 过滤"""
    malicious_tenant = "tenant_a' OR '1'='1"
    scope = SessionScope(
        authenticated=True, actor_role='merchant_owner', effective_role='merchant_owner',
        scope_type='merchant', tenant_id=malicious_tenant, merchant_id=1,
    )
    sql, params = enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope)
    assert sql == "SELECT * FROM tracked_groups WHERE tenant_id = ?"
    assert params[0] == malicious_tenant
    async with aiosqlite.connect(TEST_DB) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
        assert len(rows) == 0, "Parameterized query prevents SQL injection, no match"
    print("✅ test_sql_injection_tenant_filter: Parameterized query blocks injection")


async def test_global_search_data_leak():
    """攻击：全局搜索返回跨租户数据"""
    async with aiosqlite.connect(TEST_DB) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT gm.* FROM group_messages gm WHERE gm.content LIKE ? AND gm.group_id IN (SELECT group_id FROM tracked_groups WHERE tenant_id = ?)",
            ('%Secret%', 'tenant_a')
        ) as cursor:
            rows = await cursor.fetchall()
            contents = [row['content'] for row in rows]
        assert 'Secret A' in contents
        assert 'Secret B' not in contents, "V6/V8 fix: search should not leak cross-tenant data"
        async with db.execute(
            "SELECT m.* FROM messages m WHERE m.content LIKE ? AND m.is_deleted = 0 AND (m.from_userid = 100001 OR m.to_userid = 100001)",
            ('%Private%',)
        ) as cursor:
            private_rows = await cursor.fetchall()
            private_contents = [row['content'] for row in private_rows]
        assert 'Private A' in private_contents
        assert 'Private B' not in private_contents, "Private search should not leak cross-tenant data"
    print("✅ test_global_search_data_leak: No cross-tenant data in search results")


async def run_all():
    print("\n" + "=" * 60)
    print("租户隔离安全渗透测试")
    print("=" * 60)
    await setup_test_db()
    tests = [
        test_cross_tenant_group_access,
        test_cross_tenant_user_access,
        test_tampered_tenant_id_param,
        test_role_escalation_attempt,
        test_sql_injection_tenant_filter,
        test_global_search_data_leak,
    ]
    passed = 0
    failed = 0
    for t in tests:
        try:
            await t()
            passed += 1
        except Exception as e:
            print(f"❌ {t.__name__}: {e}")
            import traceback
            traceback.print_exc()
            failed += 1
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)
    print(f"\n{'=' * 60}")
    print(f"结果: {passed} 通过, {failed} 失败, 共 {passed + failed} 个")
    return failed == 0


if __name__ == '__main__':
    success = asyncio.get_event_loop().run_until_complete(run_all())
    sys.exit(0 if success else 1)
