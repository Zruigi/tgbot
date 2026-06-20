# -*- coding: utf-8 -*-
import os
import sys
from pathlib import Path

import aiosqlite
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

import database


@pytest.fixture()
async def tenant_db(tmp_path):
    db_path = tmp_path / "tenant_scope.sqlite3"
    original_db_name = database.DB_NAME
    database.DB_NAME = str(db_path)

    async with aiosqlite.connect(database.DB_NAME) as db:
        await db.execute("CREATE TABLE merchants (id INTEGER PRIMARY KEY, tenant_id TEXT UNIQUE, bot_id INTEGER)")
        await db.execute(
            """
            CREATE TABLE messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                trace_id TEXT,
                from_userid INTEGER,
                to_userid INTEGER,
                content TEXT,
                type TEXT,
                file_id TEXT,
                media_url TEXT,
                is_read INTEGER DEFAULT 0,
                tg_msg_id INTEGER,
                auto_translation TEXT,
                translated_text TEXT,
                original_text TEXT,
                admin_userid TEXT,
                quoted_text TEXT,
                quoted_msg_id INTEGER,
                quoted_file_id TEXT,
                quoted_type TEXT,
                status TEXT,
                is_deleted INTEGER DEFAULT 0,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute("CREATE TABLE emoji_messages (msg_id INTEGER PRIMARY KEY, emoji TEXT)")
        await db.execute(
            """
            CREATE TABLE admin_inbox (
                from_userid INTEGER PRIMARY KEY,
                from_username TEXT,
                summary TEXT,
                unread_count INTEGER DEFAULT 0,
                last_active DATETIME DEFAULT CURRENT_TIMESTAMP,
                avatar_file_id TEXT,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE potential_users (
                user_id INTEGER PRIMARY KEY,
                username TEXT,
                first_name TEXT,
                last_name TEXT,
                message_text TEXT,
                keywords TEXT,
                profile_tags TEXT,
                group_id INTEGER,
                group_title TEXT,
                captured_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE tracked_groups (
                group_id INTEGER PRIMARY KEY,
                group_title TEXT,
                tenant_id TEXT DEFAULT 'default',
                is_monitored INTEGER DEFAULT 1,
                custom_keywords TEXT,
                delete_on_revoke INTEGER DEFAULT 0,
                revoke_sync_chat_id INTEGER,
                verify_enabled INTEGER DEFAULT 0,
                verify_type TEXT,
                verify_timeout INTEGER,
                verify_channel_id INTEGER,
                verify_questions TEXT,
                verify_msg_template TEXT,
                first_seen DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER,
                user_id INTEGER,
                username TEXT,
                tg_msg_id INTEGER,
                sender_type TEXT,
                content TEXT,
                msg_type TEXT,
                file_id TEXT,
                quoted_tg_msg_id INTEGER,
                quoted_text TEXT,
                message_ts INTEGER,
                avatar_file_id TEXT,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT,
                ad_findings TEXT,
                tenant_id TEXT DEFAULT 'default',
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE group_members (
                user_id INTEGER,
                username TEXT,
                first_name TEXT,
                last_name TEXT,
                group_id INTEGER,
                group_title TEXT,
                tenant_id TEXT DEFAULT 'default',
                status TEXT,
                joined_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                avatar_file_id TEXT,
                PRIMARY KEY(group_id, user_id)
            )
            """
        )
        await db.execute("INSERT INTO merchants (id, tenant_id, bot_id) VALUES (1, 'tenant_a', 100001)")
        await db.execute("INSERT INTO merchants (id, tenant_id, bot_id) VALUES (2, 'tenant_b', 100002)")
        await db.commit()

    try:
        yield str(db_path)
    finally:
        database.DB_NAME = original_db_name
        if os.path.exists(db_path):
            os.remove(db_path)


@pytest.mark.asyncio
async def test_tenant_scoped_chat_queries_and_updates(tenant_db):
    async with aiosqlite.connect(database.DB_NAME) as db:
        await db.execute(
            "INSERT INTO messages (trace_id, from_userid, to_userid, content, type, quoted_type, status, tenant_id) VALUES ('a1', 2001, 100001, 'hello-a', 'text', 'text', 'sent', 'tenant_a')"
        )
        await db.execute(
            "INSERT INTO messages (trace_id, from_userid, to_userid, content, type, quoted_type, status, tenant_id) VALUES ('a2', 100001, 2001, 'reply-a', 'text', 'text', 'sent', 'tenant_a')"
        )
        await db.execute(
            "INSERT INTO messages (trace_id, from_userid, to_userid, content, type, quoted_type, status, tenant_id) VALUES ('b1', 3001, 100002, 'hello-b', 'text', 'text', 'sent', 'tenant_b')"
        )
        await db.execute(
            "INSERT INTO potential_users (user_id, username, tenant_id) VALUES (2001, 'UserA', 'tenant_a')"
        )
        await db.execute(
            "INSERT INTO potential_users (user_id, username, tenant_id) VALUES (3001, 'UserB', 'tenant_b')"
        )
        await db.commit()

    inbox_a = await database.get_inbox_list(tenant_id="tenant_a", merchant_id=1)
    assert [row["from_userid"] for row in inbox_a] == [2001]

    chat_a = await database.get_chat_history(2001, tenant_id="tenant_a", merchant_id=1)
    assert [row["content"] for row in chat_a] == ["reply-a", "hello-a"]

    chat_cross = await database.get_chat_history(3001, tenant_id="tenant_a", merchant_id=1)
    assert chat_cross == []

    await database.mark_messages_as_read(2001, tenant_id="tenant_a")
    async with aiosqlite.connect(database.DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(*) FROM messages WHERE from_userid = 2001 AND tenant_id = 'tenant_a' AND is_read = 1"
        ) as cursor:
            assert (await cursor.fetchone())[0] == 1
        async with db.execute(
            "SELECT COUNT(*) FROM messages WHERE from_userid = 3001 AND tenant_id = 'tenant_b' AND is_read = 1"
        ) as cursor:
            assert (await cursor.fetchone())[0] == 0


@pytest.mark.asyncio
async def test_tenant_scoped_group_and_potential_queries(tenant_db):
    async with aiosqlite.connect(database.DB_NAME) as db:
        await db.execute(
            "INSERT INTO tracked_groups (group_id, group_title, tenant_id, updated_at) VALUES (1001, 'Group A', 'tenant_a', '2026-01-01 00:00:00')"
        )
        await db.execute(
            "INSERT INTO tracked_groups (group_id, group_title, tenant_id, updated_at) VALUES (1002, 'Group B', 'tenant_b', '2026-01-01 00:00:00')"
        )
        await db.execute(
            "INSERT INTO potential_users (user_id, username, profile_tags, tenant_id) VALUES (2001, 'UserA', 'vip,alpha', 'tenant_a')"
        )
        await db.execute(
            "INSERT INTO potential_users (user_id, username, profile_tags, tenant_id) VALUES (3001, 'UserB', 'beta', 'tenant_b')"
        )
        await db.execute(
            "INSERT INTO group_members (user_id, username, group_id, tenant_id, status) VALUES (2001, 'UserA', 1001, 'tenant_a', 'active')"
        )
        await db.execute(
            "INSERT INTO group_members (user_id, username, group_id, tenant_id, status) VALUES (3001, 'UserB', 1002, 'tenant_b', 'active')"
        )
        await db.execute(
            "INSERT INTO group_messages (group_id, user_id, username, content, tenant_id, sender_type) VALUES (1001, 2001, 'UserA', 'group-a', 'tenant_a', 'user')"
        )
        await db.execute(
            "INSERT INTO group_messages (group_id, user_id, username, content, tenant_id, sender_type) VALUES (1002, 3001, 'UserB', 'group-b', 'tenant_b', 'user')"
        )
        await db.commit()

    potential_a = await database.get_potential_users(tenant_id="tenant_a")
    assert [row["user_id"] for row in potential_a["users"]] == [2001]

    tags_a = await database.get_potential_tags(tenant_id="tenant_a")
    assert tags_a == ["alpha", "vip"]

    groups_a = await database.get_tracked_groups_paginated(tenant_id="tenant_a")
    assert [row["group_id"] for row in groups_a["groups"]] == [1001]

    messages_a = await database.get_group_messages(1001, tenant_id="tenant_a")
    assert [row["content"] for row in messages_a] == ["group-a"]


@pytest.mark.asyncio
async def test_tenant_id_is_persisted_for_new_records(tenant_db):
    async with aiosqlite.connect(database.DB_NAME) as db:
        await db.execute(
            "INSERT INTO tracked_groups (group_id, group_title, tenant_id, updated_at) VALUES (1001, 'Group A', 'tenant_a', '2026-01-01 00:00:00')"
        )
        await db.commit()

    await database.save_message(
        from_userid=2001,
        to_userid=100001,
        content="fresh-message",
        msg_type="text",
        quoted_type="text",
        status="sent",
    )
    await database.upsert_potential_user(
        user_id=2001,
        username="UserA",
        group_id=1001,
        group_title="Group A",
    )
    await database.save_group_member(
        user_id=2001,
        username="UserA",
        group_id=1001,
        group_title="Group A",
    )
    await database.save_group_message(
        group_id=1001,
        user_id=2001,
        username="UserA",
        content="fresh-group-message",
    )

    async with aiosqlite.connect(database.DB_NAME) as db:
        async with db.execute("SELECT tenant_id FROM messages WHERE content = 'fresh-message'") as cursor:
            assert (await cursor.fetchone())[0] == "tenant_a"
        async with db.execute("SELECT tenant_id FROM potential_users WHERE user_id = 2001") as cursor:
            assert (await cursor.fetchone())[0] == "tenant_a"
        async with db.execute("SELECT tenant_id FROM group_members WHERE user_id = 2001 AND group_id = 1001") as cursor:
            assert (await cursor.fetchone())[0] == "tenant_a"
        async with db.execute("SELECT tenant_id FROM group_messages WHERE content = 'fresh-group-message'") as cursor:
            assert (await cursor.fetchone())[0] == "tenant_a"
