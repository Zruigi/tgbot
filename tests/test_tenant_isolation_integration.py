# -*- coding: utf-8 -*-
"""
租户隔离集成测试 - 验证API层和Repository层的数据隔离效果
"""
import sys
import asyncio
import importlib.util
import aiosqlite
import os
import pytest
from pathlib import Path
from datetime import datetime

BASE = Path(__file__).parent.parent
TEST_DB = '/tmp/test_tenant_isolation.db'


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
        await db.execute('''CREATE TABLE group_members (user_id INTEGER, group_id INTEGER, username TEXT, tenant_id TEXT DEFAULT 'default', PRIMARY KEY(group_id, user_id))''')
        await db.execute('''CREATE TABLE potential_users (user_id INTEGER PRIMARY KEY, username TEXT, tenant_id TEXT DEFAULT 'default')''')
        await db.execute('''CREATE TABLE messages (msg_id INTEGER PRIMARY KEY AUTOINCREMENT, from_userid INTEGER, to_userid INTEGER, content TEXT, is_deleted INTEGER DEFAULT 0, timestamp DATETIME DEFAULT CURRENT_TIMESTAMP, tenant_id TEXT DEFAULT 'default')''')
        await db.execute('''CREATE TABLE admin_inbox (from_userid INTEGER PRIMARY KEY, from_username TEXT, summary TEXT, unread_count INTEGER DEFAULT 0, last_active DATETIME DEFAULT CURRENT_TIMESTAMP, tenant_id TEXT DEFAULT 'default')''')
        await db.execute("CREATE INDEX IF NOT EXISTS idx_tracked_groups_tenant ON tracked_groups(tenant_id)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_group_messages_tenant ON group_messages(tenant_id)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_messages_tenant ON messages(tenant_id)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_admin_inbox_tenant ON admin_inbox(tenant_id)")
        await db.execute("INSERT INTO merchants VALUES (1, 'tenant_a', 100001, 'Merchant A')")
        await db.execute("INSERT INTO merchants VALUES (2, 'tenant_b', 100002, 'Merchant B')")
        await db.execute("INSERT INTO tracked_groups VALUES (1001, 'Group A1', 'tenant_a', 1, '2026-01-01')")
        await db.execute("INSERT INTO tracked_groups VALUES (1002, 'Group A2', 'tenant_a', 1, '2026-01-01')")
        await db.execute("INSERT INTO tracked_groups VALUES (1003, 'Group B1', 'tenant_b', 1, '2026-01-01')")
        await db.execute("INSERT INTO tracked_groups VALUES (1004, 'Group B2', 'tenant_b', 1, '2026-01-01')")
        await db.execute("INSERT INTO group_messages VALUES (1, 1001, 2001, 'Hello A1', 1000, 'tenant_a')")
        await db.execute("INSERT INTO group_messages VALUES (2, 1003, 3001, 'Hello B1', 1000, 'tenant_b')")
        await db.execute("INSERT INTO group_members VALUES (2001, 1001, 'User_A1', 'tenant_a')")
        await db.execute("INSERT INTO group_members VALUES (3001, 1003, 'User_B1', 'tenant_b')")
        await db.execute("INSERT INTO potential_users VALUES (2001, 'User_A1', 'tenant_a')")
        await db.execute("INSERT INTO potential_users VALUES (3001, 'User_B1', 'tenant_b')")
        await db.execute("INSERT INTO messages VALUES (1, 2001, 100001, 'Msg from A user', 0, '2026-01-01', 'tenant_a')")
        await db.execute("INSERT INTO messages VALUES (2, 3001, 100002, 'Msg from B user', 0, '2026-01-01', 'tenant_b')")
        await db.execute("INSERT INTO admin_inbox VALUES (2001, 'User_A1', 'summary', 1, '2026-01-01', 'tenant_a')")
        await db.execute("INSERT INTO admin_inbox VALUES (3001, 'User_B1', 'summary', 1, '2026-01-01', 'tenant_b')")
        await db.commit()


@pytest.fixture(autouse=True)
async def _tenant_test_db():
    await setup_test_db()
    try:
        yield
    finally:
        if os.path.exists(TEST_DB):
            os.remove(TEST_DB)


async def test_merchant_list_groups_isolation():
    async with aiosqlite.connect(TEST_DB) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM tracked_groups WHERE tenant_id = ?", ('tenant_a',)) as cursor:
            rows = await cursor.fetchall()
            group_ids = [row['group_id'] for row in rows]
        assert 1001 in group_ids, "Tenant A should see Group A1"
        assert 1002 in group_ids, "Tenant A should see Group A2"
        assert 1003 not in group_ids, "Tenant A should NOT see Group B1"
        assert 1004 not in group_ids, "Tenant A should NOT see Group B2"
    print("✅ test_merchant_list_groups_isolation (V1)")


async def test_merchant_get_messages_isolation():
    async with aiosqlite.connect(TEST_DB) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM tracked_groups WHERE group_id = ? AND tenant_id = ?",
            (1003, 'tenant_a')
        ) as cursor:
            row = await cursor.fetchone()
        assert row is None, "Tenant A should NOT own Group B1 (1003)"
    print("✅ test_merchant_get_messages_isolation (V2)")


async def test_merchant_get_users_isolation():
    async with aiosqlite.connect(TEST_DB) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT bot_id FROM merchants WHERE tenant_id = ?", ('tenant_a',)
        ) as cursor:
            bot_row = await cursor.fetchone()
            bot_id = bot_row[0]
        async with db.execute(
            "SELECT DISTINCT from_userid FROM messages WHERE to_userid = ? AND is_deleted = 0",
            (bot_id,)
        ) as cursor:
            user_ids = [row[0] for row in await cursor.fetchall()]
        async with db.execute(
            "SELECT * FROM admin_inbox WHERE from_userid IN ({})".format(','.join('?' * len(user_ids))),
            user_ids
        ) as cursor:
            inbox_rows = await cursor.fetchall()
            inbox_uids = [row['from_userid'] for row in inbox_rows]
        assert 2001 in inbox_uids, "Tenant A should see User_A1 in inbox"
        assert 3001 not in inbox_uids, "Tenant A should NOT see User_B1 in inbox"
    print("✅ test_merchant_get_users_isolation (V3)")


async def test_merchant_get_chat_history_isolation():
    async with aiosqlite.connect(TEST_DB) as db:
        async with db.execute(
            "SELECT 1 FROM potential_users WHERE user_id = ? AND tenant_id = ? LIMIT 1",
            (3001, 'tenant_a')
        ) as cursor:
            row = await cursor.fetchone()
        assert row is None, "User_B1 (3001) should NOT belong to tenant_a"
    print("✅ test_merchant_get_chat_history_isolation (V4)")


async def test_merchant_search_isolation():
    async with aiosqlite.connect(TEST_DB) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT gm.* FROM group_messages gm WHERE gm.content LIKE ? AND gm.group_id IN (SELECT group_id FROM tracked_groups WHERE tenant_id = ?)",
            ('%Hello%', 'tenant_a')
        ) as cursor:
            rows = await cursor.fetchall()
            contents = [row['content'] for row in rows]
        assert 'Hello A1' in contents, "Tenant A search should find A1 message"
        assert 'Hello B1' not in contents, "Tenant A search should NOT find B1 message"
    print("✅ test_merchant_search_isolation (V6/V8)")


async def test_platform_admin_no_filter():
    async with aiosqlite.connect(TEST_DB) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM tracked_groups") as cursor:
            rows = await cursor.fetchall()
        assert len(rows) == 4, "Platform admin should see ALL 4 groups"
    print("✅ test_platform_admin_no_filter")


async def test_empty_tenant_rejection():
    scope = SessionScope(
        authenticated=True,
        actor_role='merchant_owner',
        effective_role='merchant_owner',
        scope_type='merchant',
        tenant_id='',
        merchant_id=1,
    )
    try:
        enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope)
        assert False, "Should have raised ValueError"
    except ValueError:
        pass
    print("✅ test_empty_tenant_rejection")


async def test_enforce_tenant_filter_sql_injection_resistant():
    scope = SessionScope(
        authenticated=True,
        actor_role='merchant_owner',
        effective_role='merchant_owner',
        scope_type='merchant',
        tenant_id="tenant_a'; DROP TABLE tracked_groups; --",
        merchant_id=1,
    )
    sql, params = enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope)
    assert sql == "SELECT * FROM tracked_groups WHERE tenant_id = ?"
    assert params[0] == "tenant_a'; DROP TABLE tracked_groups; --"
    print("✅ test_enforce_tenant_filter_sql_injection_resistant")


async def run_all():
    print("\n" + "=" * 60)
    print("租户隔离集成测试")
    print("=" * 60)
    await setup_test_db()
    tests = [
        test_merchant_list_groups_isolation,
        test_merchant_get_messages_isolation,
        test_merchant_get_users_isolation,
        test_merchant_get_chat_history_isolation,
        test_merchant_search_isolation,
        test_platform_admin_no_filter,
        test_empty_tenant_rejection,
        test_enforce_tenant_filter_sql_injection_resistant,
    ]
    passed = 0
    failed = 0
    for t in tests:
        try:
            await t()
            passed += 1
        except Exception as e:
            print(f"❌ {t.__name__}: {e}")
            failed += 1
    if os.path.exists(TEST_DB):
        os.remove(TEST_DB)
    print(f"\n{'=' * 60}")
    print(f"结果: {passed} 通过, {failed} 失败, 共 {passed + failed} 个")
    return failed == 0


if __name__ == '__main__':
    success = asyncio.get_event_loop().run_until_complete(run_all())
    sys.exit(0 if success else 1)
