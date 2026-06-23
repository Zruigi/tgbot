# -*- coding: utf-8 -*-
import os
import sys
import time

import aiosqlite
import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)


def test_ad_pipeline_normalize_and_hash():
    from services.ad_pipeline import normalize_text, text_md5

    a = " 加微信 @spam 领取高收益  https://example.com/a "
    b = "加微信 @other 领取高收益 https://example.com/b"
    assert normalize_text(a) == "加微信 [link] 领取高收益 [link]"
    assert text_md5(a) == text_md5(b)


@pytest.mark.asyncio
async def test_ad_pipeline_sqlite_fallback_clusters_duplicate_messages(tmp_path, monkeypatch):
    import config
    import database
    from services.ad_pipeline import get_ad_pipeline

    db_path = tmp_path / "ad_pipeline.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(config, "AD_PIPELINE_PG_DSN", "", raising=False)
    monkeypatch.setattr(config, "AD_MIN_DUPLICATE_COUNT", 2, raising=False)

    now = int(time.time())
    async with aiosqlite.connect(db_path) as db:
        await db.execute("CREATE TABLE tracked_groups (group_id INTEGER PRIMARY KEY, group_title TEXT, tenant_id TEXT DEFAULT 'default')")
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER,
                user_id INTEGER,
                username TEXT,
                tg_msg_id INTEGER,
                sender_type TEXT DEFAULT 'user',
                content TEXT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                message_ts INTEGER,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT,
                ad_cluster_id TEXT,
                ad_checked_at DATETIME,
                ad_action_status TEXT DEFAULT 'none',
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute("INSERT INTO tracked_groups (group_id, group_title, tenant_id) VALUES (?, ?, ?)", (-1001, "测试群", "tenant_a"))
        for i in range(3):
            await db.execute(
                """
                INSERT INTO group_messages (group_id, user_id, username, tg_msg_id, content, message_ts, tenant_id)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                """,
                (-1001, 700 + i, f"spam{i}", 100 + i, "注册送高收益，加微信 @spam", now - i, "tenant_a"),
            )
        await db.commit()

    result = await get_ad_pipeline().query_clusters(tenant_id="tenant_a", group_id=-1001, limit=10, window_minutes=10)

    assert result["success"] is True
    assert result["mode"] == "sqlite_fallback"
    clusters = result["data"]["clusters"]
    assert len(clusters) == 1
    assert clusters[0]["occurrences"] == 3
    assert len(clusters[0]["message_refs"]) == 3


@pytest.mark.asyncio
async def test_agent_prefetches_ad_clusters_before_llm_reply(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    db_path = tmp_path / "agent_ad_clusters.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(config, "AD_PIPELINE_PG_DSN", "", raising=False)

    now = int(time.time())
    async with aiosqlite.connect(db_path) as db:
        await db.execute("CREATE TABLE tracked_groups (group_id INTEGER PRIMARY KEY, group_title TEXT, tenant_id TEXT DEFAULT 'default')")
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER,
                user_id INTEGER,
                username TEXT,
                tg_msg_id INTEGER,
                sender_type TEXT DEFAULT 'user',
                content TEXT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                message_ts INTEGER,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT,
                ad_cluster_id TEXT,
                ad_checked_at DATETIME,
                ad_action_status TEXT DEFAULT 'none',
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            "INSERT INTO group_messages (group_id, user_id, username, tg_msg_id, content, message_ts, tenant_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (-1002, 888, "spammer", 55, "加微信领取高收益项目", now, "default"),
        )
        await db.commit()

    seen_messages = []

    async def fake_llm(messages, model=""):
        seen_messages.append(messages)
        return "我已基于工具结果回答。", {"total_tokens": 8}

    agent = AIAgent(llm_call_fn=fake_llm)
    response = await agent.process_message("最近群里有没有广告？", AgentContext(user_id=-99999))

    assert response.tool_calls_made[0] == "query_ad_clusters"
    assert "query_ad_clusters" in response.tool_results
    assert seen_messages
    assert "[工具执行结果]" in seen_messages[0][-1]["content"]
