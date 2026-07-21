# -*- coding: utf-8 -*-
import os
import sys
import time
from types import SimpleNamespace

import aiosqlite
import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)


async def _create_ad_db(path, *, now: int) -> None:
    async with aiosqlite.connect(path) as db:
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL,
                tg_msg_id INTEGER,
                user_id INTEGER NOT NULL,
                username TEXT,
                sender_type TEXT DEFAULT 'user',
                content TEXT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                message_ts INTEGER,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT,
                ad_action_status TEXT DEFAULT 'none',
                ad_checked_at DATETIME,
                is_deleted INTEGER DEFAULT 0,
                deleted_at DATETIME,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE tenant_audit_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tenant_id TEXT NOT NULL,
                merchant_id INTEGER,
                operator_role TEXT,
                operator_id TEXT,
                action TEXT NOT NULL,
                resource_type TEXT,
                resource_id TEXT,
                payload_json TEXT,
                trace_id TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            """
        )
        rows = [
            (-1001, 101, 701, "ad_one", "user", "高收益项目加微信", now - 300, 1, 91, 0, "tenant-a"),
            (-1001, 102, 702, "ad_two", "user", "注册送福利", now - 3600, 1, 85, 0, "tenant-a"),
            (-1001, 103, 703, "old_ad", "user", "25小时前的广告", now - 25 * 3600, 1, 99, 0, "tenant-a"),
            (-1001, 104, 704, "channel", "channel", "频道转发广告", now - 1200, 1, 99, 0, "tenant-a"),
            (-1001, 105, 705, "low_score", "user", "低分疑似内容", now - 900, 1, 20, 0, "tenant-a"),
            (-1001, 106, 706, "normal", "user", "普通聊天", now - 600, 0, 0, 0, "tenant-a"),
            (-1001, 107, 707, "deleted", "user", "已删除广告", now - 700, 1, 90, 1, "tenant-a"),
            (-1002, 108, 708, "other_tenant", "user", "其他租户广告", now - 400, 1, 95, 0, "tenant-b"),
        ]
        await db.executemany(
            """
            INSERT INTO group_messages (
                group_id, tg_msg_id, user_id, username, sender_type, content,
                message_ts, is_advertising, ad_score, is_deleted, tenant_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            rows,
        )
        await db.commit()


class FakeBot:
    def __init__(self):
        self.deleted = []

    async def get_me(self):
        return SimpleNamespace(id=999001)

    async def get_chat_member(self, chat_id, user_id):
        if int(user_id) == 999001:
            return SimpleNamespace(status="administrator", can_delete_messages=True)
        return SimpleNamespace(status="member")

    async def delete_message(self, chat_id, message_id):
        self.deleted.append((int(chat_id), int(message_id)))


@pytest.mark.asyncio
async def test_query_recent_ads_returns_times_and_excludes_unsafe_rows(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.schema import AgentContext
    from services.ai_agent.tools import QueryRecentAdMessagesTool

    now = int(time.time())
    db_path = tmp_path / "recent_ads.sqlite3"
    await _create_ad_db(db_path, now=now)
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    ctx = AgentContext(
        user_id=-99999,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1},
    )
    result = await QueryRecentAdMessagesTool().execute(
        {"hours": 24, "limit": 50, "sample_size": 5},
        context=ctx,
    )

    assert result["success"] is True
    assert result["data"]["total_eligible_count"] == 2
    assert result["data"]["snapshot_count"] == 2
    assert result["data"]["latest_recorded_at"]
    assert result["data"]["earliest_recorded_at"]
    assert len(result["data"]["random_sample_times"]) == 2
    assert {ref["tg_msg_id"] for ref in result["data"]["candidate_refs"]} == {101, 102}


@pytest.mark.asyncio
async def test_agent_prepares_then_confirms_recent_ad_deletion(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    now = int(time.time())
    db_path = tmp_path / "agent_delete_recent_ads.sqlite3"
    await _create_ad_db(db_path, now=now)
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(config, "AD_DELETE_ENABLED", True, raising=False)

    llm_calls = []

    async def fake_llm(messages, model=""):
        llm_calls.append(messages)
        return "LLM 不应该接管删除路由", {"total_tokens": 1}

    bot = FakeBot()
    ctx = AgentContext(
        user_id=-99999,
        bot_instance=bot,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1, "chat_id": -1001},
    )
    agent = AIAgent(llm_call_fn=fake_llm)

    pending = await agent.process_message("执行删除 最近24小时的广告", ctx)

    assert pending.requires_confirmation is True
    assert pending.is_action_taken is False
    assert pending.tool_calls_made == ["query_recent_ad_messages", "delete_recent_ad_messages"]
    assert pending.tool_results["query_recent_ad_messages"]["data"]["snapshot_count"] == 2
    assert pending.tool_results["delete_recent_ad_messages"]["needs_confirmation"] is True
    assert bot.deleted == []
    assert llm_calls == []

    executed = await agent.confirm_and_execute(pending, ctx)

    assert executed.requires_confirmation is False
    assert executed.is_action_taken is True
    assert set(bot.deleted) == {(-1001, 101), (-1001, 102)}
    result = executed.tool_results["delete_recent_ad_messages"]
    assert result["data"]["deleted_count"] == 2

    async with aiosqlite.connect(db_path) as db:
        cur = await db.execute(
            "SELECT tg_msg_id, is_deleted, ad_action_status FROM group_messages WHERE tg_msg_id IN (101, 102) ORDER BY tg_msg_id"
        )
        rows = await cur.fetchall()
        assert rows == [(101, 1, "deleted"), (102, 1, "deleted")]
        audit_cur = await db.execute(
            "SELECT COUNT(*) FROM tenant_audit_logs WHERE action = 'agent_delete_recent_ads'"
        )
        assert (await audit_cur.fetchone())[0] == 1


@pytest.mark.asyncio
async def test_recent_ad_delete_is_blocked_when_switch_is_off(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    now = int(time.time())
    db_path = tmp_path / "agent_delete_disabled.sqlite3"
    await _create_ad_db(db_path, now=now)
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(config, "AD_DELETE_ENABLED", False, raising=False)

    bot = FakeBot()
    ctx = AgentContext(
        user_id=-99999,
        bot_instance=bot,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1, "chat_id": -1001},
    )
    agent = AIAgent(llm_call_fn=None)
    pending = await agent.process_message("执行删除 最近24小时的广告", ctx)
    executed = await agent.confirm_and_execute(pending, ctx)

    assert executed.is_action_taken is False
    assert executed.tool_results["delete_recent_ad_messages"]["blocked"] is True
    assert bot.deleted == []
