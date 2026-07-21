# -*- coding: utf-8 -*-
import os
import sys

import aiosqlite
import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)


@pytest.mark.asyncio
async def test_agent_uses_llm_and_keeps_user_message_separate():
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    seen_messages = []

    async def fake_llm(messages, model=""):
        seen_messages.extend(messages)
        return "这是来自真实 LLM 调用链路的回答，不是回声。", {"total_tokens": 12}

    agent = AIAgent(llm_call_fn=fake_llm)
    user_text = "请解释一下为什么需要风控"
    response = await agent.process_message(user_text, AgentContext(user_id=-99999))

    assert response.text == "这是来自真实 LLM 调用链路的回答，不是回声。"
    assert response.text != f"[AI] 收到您的消息：{user_text}"
    assert seen_messages[-1]["role"] == "user"
    assert f"用户请求: {user_text}" in seen_messages[-1]["content"]
    assert seen_messages[0]["role"] == "system"
    assert user_text not in seen_messages[0]["content"]


@pytest.mark.asyncio
async def test_agent_detects_advertising_with_tool():
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    calls = 0

    async def fake_llm(messages, model=""):
        nonlocal calls
        calls += 1
        if calls == 1:
            return "我来检测。", {"total_tokens": 8}
        return "这条消息广告特征明显，建议删除。", {"total_tokens": 8}

    agent = AIAgent(llm_call_fn=fake_llm)
    response = await agent.process_message(
        "检测广告：注册送福利，加微信领取高收益项目 http://spam.example",
        AgentContext(user_id=-99999),
    )

    assert "detect_advertising" in response.tool_calls_made
    assert "评分 30/100" in response.text
    assert "微信" in response.text


@pytest.mark.asyncio
async def test_agent_prefetches_single_keyword_telegram_permission_request():
    from services.ai_agent.core import AIAgent
    from services.ai_agent.tools import get_tools_for_llm

    agent = AIAgent(llm_call_fn=None)
    matched = agent._match_tool_from_text("检查机器人删除权限", get_tools_for_llm())

    assert matched == "telegram_sdk_query"


@pytest.mark.asyncio
async def test_agent_formats_finance_summary_without_second_llm(monkeypatch):
    import database
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    async def fake_summary(user_id):
        return {"by_currency": [], "total_unreconciled": 0}

    async def fail_llm(messages, model=""):
        raise AssertionError("财务摘要不应再交给 LLM 编造总结")

    monkeypatch.setattr(database, "get_finance_summary_by_currency", fake_summary)
    response = await AIAgent(llm_call_fn=fail_llm).process_message(
        "查财务摘要", AgentContext(user_id=-99999)
    )

    assert response.tool_calls_made == ["finance_query"]
    assert response.text == "财务摘要：0 个币种，待核销 0 条。"


def test_agent_formats_telegram_permissions_without_llm_hallucination():
    from services.ai_agent.core import AIAgent

    result = {
        "success": True,
        "data": {
            "items": [
                {
                    "chat_id": -1001, "title": "可删群", "chat_type": "supergroup",
                    "success": True, "bot_status": "administrator",
                    "bot_permissions": {"can_delete_messages": True},
                },
                {
                    "chat_id": -1002, "title": "受限群", "chat_type": "supergroup",
                    "success": True, "bot_status": "restricted", "bot_permissions": {},
                },
                {"chat_id": -1003, "success": False, "errors": ["Chat not found"]},
            ]
        },
    }

    text = AIAgent._format_telegram_sdk_result(result)

    assert "1 个可删除消息，1 个不可删除" in text
    assert "可删群 (-1001) [administrator]" in text
    assert "受限群 (-1002) [restricted]" in text
    assert "1 条数据库绑定记录未通过 Telegram 验证" in text


def test_bound_chat_preflight_requests_complete_sdk_verification():
    import json
    from services.ai_agent.core import AIAgent
    from services.ai_agent.tools import get_tools_for_llm

    calls = AIAgent(llm_call_fn=None)._build_preflight_tool_calls(
        "我有几个群几个频道", get_tools_for_llm()
    )
    sdk_call = next(call for call in calls if call["function"]["name"] == "telegram_sdk_query")
    args = json.loads(sdk_call["function"]["arguments"])

    assert args["operation"] == "bound_chat_capabilities"
    assert args["limit"] == 50


def test_numbered_group_followup_resolves_to_verified_chat_id():
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    ctx = AgentContext(
        user_id=-99999,
        conversation_history=[{
            "role": "assistant",
            "content": (
                "Telegram 官方 API 当前验证到群组 2 个，频道 1 个。\n"
                "群组：\n1. 甲群 (-1001111111111)\n2. 乙群 (-1002222222222)\n"
                "频道：\n1. 甲频道 (-1003333333333)"
            ),
        }],
    )

    assert AIAgent._resolve_contextual_command("查看第二个群消息", ctx) == "查看群 -1002222222222 最近群消息"
    assert AIAgent._resolve_contextual_command("第一个", ctx) == "查看群 -1001111111111 最近群消息"


@pytest.mark.asyncio
async def test_llm_cannot_create_empty_recent_delete_confirmation():
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    calls = 0

    async def fake_llm(messages, model=""):
        nonlocal calls
        calls += 1
        if calls == 1:
            return '__TOOL_CALL__{"function":{"name":"delete_recent_ad_messages","arguments":{}}}', {"total_tokens": 1}
        return "删除请求因缺少后端快照已拒绝。", {"total_tokens": 1}

    response = await AIAgent(llm_call_fn=fake_llm).process_message(
        "删除", AgentContext(user_id=-99999)
    )

    assert response.requires_confirmation is False
    assert response.tool_results["delete_recent_ad_messages"]["blocked"] is True
    assert "缺少后端快照" in response.text


def test_agent_cancellation_phrases_are_deterministic():
    from web_admin import _is_agent_cancellation_text

    assert _is_agent_cancellation_text("取消操作") is True
    assert _is_agent_cancellation_text("不要了") is True
    assert _is_agent_cancellation_text("继续") is False


@pytest.mark.asyncio
async def test_agent_deletes_flagged_ad_after_confirmation(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    db_path = tmp_path / "agent_ad_delete.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    async with aiosqlite.connect(db_path) as db:
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
                msg_type TEXT DEFAULT 'text',
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                message_ts INTEGER,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT DEFAULT NULL,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            INSERT INTO group_messages (
                group_id, user_id, username, tg_msg_id, content,
                message_ts, is_advertising, ad_score, ad_confidence
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (-100123, 777, "spammer", 42, "注册送高收益广告", 2000, 1, 92, "high"),
        )
        await db.commit()

    class Member:
        status = "member"

    class FakeBot:
        def __init__(self):
            self.deleted = []

        async def get_chat_member(self, group_id, user_id):
            return Member()

        async def delete_message(self, chat_id, message_id):
            self.deleted.append((chat_id, message_id))

    async def fake_llm(messages, model=""):
        return "我会先准备删除最近一条广告。", {"total_tokens": 8}

    fake_bot = FakeBot()
    ctx = AgentContext(user_id=-99999, bot_instance=fake_bot)
    agent = AIAgent(llm_call_fn=fake_llm)

    pending = await agent.process_message("识别广告并删除一条广告", ctx)
    assert pending.requires_confirmation is True
    assert "delete_ad_message" in pending.tool_calls_made
    assert fake_bot.deleted == []

    executed = await agent.confirm_and_execute(pending, ctx)
    assert executed.requires_confirmation is False
    assert executed.is_action_taken is True
    assert fake_bot.deleted == [(-100123, 42)]

    async with aiosqlite.connect(db_path) as db:
        cur = await db.execute("SELECT COUNT(*) FROM group_messages WHERE group_id = ? AND tg_msg_id = ?", (-100123, 42))
        remaining = (await cur.fetchone())[0]
    assert remaining == 0


@pytest.mark.asyncio
async def test_system_query_reads_bound_chats_for_current_tenant(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.schema import AgentContext
    from services.ai_agent.tools import SystemQueryTool

    db_path = tmp_path / "agent_bound_chats.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    async with aiosqlite.connect(db_path) as db:
        await db.execute(
            """
            CREATE TABLE tracked_groups (
                group_id INTEGER PRIMARY KEY,
                group_title TEXT,
                is_monitored INTEGER DEFAULT 0,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE broadcast_targets (
                chat_id INTEGER PRIMARY KEY,
                chat_title TEXT,
                chat_type TEXT,
                chat_username TEXT,
                can_broadcast INTEGER DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            "INSERT INTO tracked_groups (group_id, group_title, is_monitored, tenant_id) VALUES (?, ?, ?, ?)",
            (-1001, "租户A群", 1, "tenant-a"),
        )
        await db.execute(
            "INSERT INTO tracked_groups (group_id, group_title, is_monitored, tenant_id) VALUES (?, ?, ?, ?)",
            (-2001, "租户B群", 1, "tenant-b"),
        )
        await db.execute(
            "INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, tenant_id) VALUES (?, ?, ?, ?, ?, ?)",
            (-1001, "租户A群", "supergroup", "group_a", 1, "tenant-a"),
        )
        await db.execute(
            "INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, tenant_id) VALUES (?, ?, ?, ?, ?, ?)",
            (-1002, "租户A频道", "channel", "channel_a", 0, "tenant-a"),
        )
        await db.execute(
            "INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, tenant_id) VALUES (?, ?, ?, ?, ?, ?)",
            (-1003, "仅广播目标群", "supergroup", "broadcast_only", 1, "tenant-a"),
        )
        await db.execute(
            "INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, tenant_id) VALUES (?, ?, ?, ?, ?, ?)",
            (-2002, "租户B频道", "channel", "channel_b", 1, "tenant-b"),
        )
        await db.commit()

    ctx = AgentContext(
        user_id=123,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1},
    )
    result = await SystemQueryTool().execute({"query_type": "bound_chats", "limit": 20}, context=ctx)

    assert result["success"] is True
    assert result["data"]["group_count"] == 1
    assert result["data"]["channel_count"] == 1
    assert result["data"]["broadcast_only_group_count"] == 1
    assert {g["chat_id"] for g in result["data"]["groups"]} == {-1001}
    assert {c["chat_id"] for c in result["data"]["channels"]} == {-1002}
    assert {g["chat_id"] for g in result["data"]["broadcast_only_groups"]} == {-1003}


@pytest.mark.asyncio
async def test_system_query_reads_group_messages_for_current_tenant(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.schema import AgentContext
    from services.ai_agent.tools import SystemQueryTool

    db_path = tmp_path / "agent_group_messages.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    async with aiosqlite.connect(db_path) as db:
        await db.execute(
            """
            CREATE TABLE tracked_groups (
                group_id INTEGER PRIMARY KEY,
                group_title TEXT,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER,
                tg_msg_id INTEGER,
                user_id INTEGER,
                username TEXT,
                content TEXT,
                msg_type TEXT DEFAULT 'text',
                sender_type TEXT DEFAULT 'user',
                message_ts INTEGER,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT DEFAULT NULL,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute("INSERT INTO tracked_groups (group_id, group_title, tenant_id) VALUES (?, ?, ?)", (-1001, "租户A群", "tenant-a"))
        await db.execute("INSERT INTO tracked_groups (group_id, group_title, tenant_id) VALUES (?, ?, ?)", (-2001, "租户B群", "tenant-b"))
        await db.execute(
            "INSERT INTO group_messages (group_id, tg_msg_id, user_id, username, content, message_ts, tenant_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (-1001, 11, 501, "alice", "tenant-a message", 100, "tenant-a"),
        )
        await db.execute(
            "INSERT INTO group_messages (group_id, tg_msg_id, user_id, username, content, message_ts, tenant_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (-2001, 22, 601, "bob", "tenant-b message", 200, "tenant-b"),
        )
        await db.commit()

    ctx = AgentContext(
        user_id=123,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1},
    )
    result = await SystemQueryTool().execute({"query_type": "group_messages", "limit": 20}, context=ctx)

    assert result["success"] is True
    assert result["count"] == 1
    assert result["data"][0]["content"] == "tenant-a message"
    assert result["data"][0]["group_title"] == "租户A群"


@pytest.mark.asyncio
async def test_system_query_empty_group_messages_disables_analysis(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.schema import AgentContext
    from services.ai_agent.tools import SystemQueryTool

    db_path = tmp_path / "agent_empty_group_messages.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    async with aiosqlite.connect(db_path) as db:
        await db.execute(
            """
            CREATE TABLE tracked_groups (
                group_id INTEGER PRIMARY KEY,
                group_title TEXT,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER,
                tg_msg_id INTEGER,
                user_id INTEGER,
                username TEXT,
                content TEXT,
                msg_type TEXT DEFAULT 'text',
                sender_type TEXT DEFAULT 'user',
                message_ts INTEGER,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT DEFAULT NULL,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute("INSERT INTO tracked_groups (group_id, group_title, tenant_id) VALUES (?, ?, ?)", (-1001, "空群", "tenant-a"))
        await db.commit()

    ctx = AgentContext(
        user_id=123,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1},
    )
    result = await SystemQueryTool().execute({"query_type": "group_messages", "group_name": "空群"}, context=ctx)

    assert result["success"] is True
    assert result["count"] == 0
    assert result["evidence_count"] == 0
    assert result["data_complete"] is False
    assert result["analysis_allowed"] is False
    assert "不能" in result["limitation"]


@pytest.mark.asyncio
async def test_telegram_sdk_query_reads_capabilities_and_caches_snapshot(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.schema import AgentContext
    from services.ai_agent.tools import TelegramSdkQueryTool

    db_path = tmp_path / "agent_tg_sdk.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    async with aiosqlite.connect(db_path) as db:
        await db.execute(
            """
            CREATE TABLE tracked_groups (
                group_id INTEGER PRIMARY KEY,
                group_title TEXT,
                is_monitored INTEGER DEFAULT 1,
                tenant_id TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE broadcast_targets (
                chat_id INTEGER PRIMARY KEY,
                chat_title TEXT,
                chat_type TEXT NOT NULL,
                chat_username TEXT DEFAULT NULL,
                can_broadcast INTEGER DEFAULT 1,
                tenant_id TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE telegram_chat_capability_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                tenant_id TEXT DEFAULT 'default',
                chat_type TEXT DEFAULT NULL,
                title TEXT DEFAULT NULL,
                username TEXT DEFAULT NULL,
                member_count INTEGER DEFAULT NULL,
                admin_count INTEGER DEFAULT NULL,
                bot_status TEXT DEFAULT NULL,
                bot_permissions_json TEXT DEFAULT NULL,
                history_fetch_supported BOOLEAN DEFAULT 0,
                source TEXT DEFAULT 'telegram_bot_api',
                last_error TEXT DEFAULT NULL,
                last_checked_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(chat_id, tenant_id)
            )
            """
        )
        await db.commit()

    class Obj:
        def __init__(self, **kwargs):
            self.__dict__.update(kwargs)

        def to_dict(self):
            return dict(self.__dict__)

    class FakeBot:
        async def get_chat(self, chat_id):
            return Obj(id=chat_id, type="supergroup", title="SDK群", username="sdk_group")

        async def get_me(self):
            return Obj(id=777, username="bot")

        async def get_chat_member(self, chat_id, user_id):
            return Obj(status="administrator", can_delete_messages=True, can_invite_users=True)

        async def get_chat_member_count(self, chat_id):
            return 42

        async def get_chat_administrators(self, chat_id):
            return [Obj(status="administrator", user=Obj(id=777, username="bot"))]

    ctx = AgentContext(
        user_id=123,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1},
        bot_instance=FakeBot(),
    )
    result = await TelegramSdkQueryTool().execute(
        {"operation": "chat_capabilities", "chat_id": -1001, "include_admins": True},
        context=ctx,
    )

    assert result["success"] is True
    item = result["data"]["items"][0]
    assert item["title"] == "SDK群"
    assert item["member_count"] == 42
    assert item["bot_status"] == "administrator"
    assert item["bot_permissions"]["can_delete_messages"] is True
    assert result["data"]["history_fetch_supported"] is False

    async with aiosqlite.connect(db_path) as db:
        row = await (await db.execute(
            "SELECT title, member_count, bot_status, history_fetch_supported FROM telegram_chat_capability_snapshots WHERE chat_id = ? AND tenant_id = ?",
            (-1001, "tenant-a"),
        )).fetchone()
    assert row == ("SDK群", 42, "administrator", 0)


@pytest.mark.asyncio
async def test_agent_reads_bound_chats_and_group_messages(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext

    db_path = tmp_path / "agent_bound_chats.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    async with aiosqlite.connect(db_path) as db:
        await db.execute(
            """
            CREATE TABLE tracked_groups (
                group_id INTEGER PRIMARY KEY,
                group_title TEXT,
                is_monitored INTEGER DEFAULT 1,
                tenant_id TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE broadcast_targets (
                chat_id INTEGER PRIMARY KEY,
                chat_title TEXT,
                chat_type TEXT NOT NULL,
                chat_username TEXT DEFAULT NULL,
                can_broadcast INTEGER DEFAULT 1,
                tenant_id TEXT,
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
                sender_type TEXT DEFAULT 'user',
                content TEXT,
                msg_type TEXT DEFAULT 'text',
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                message_ts INTEGER,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT DEFAULT NULL,
                tenant_id TEXT
            )
            """
        )
        await db.execute(
            "INSERT INTO tracked_groups (group_id, group_title, tenant_id) VALUES (?, ?, ?)",
            (-1001, "测试群A", "tenant_a"),
        )
        await db.execute(
            """
            INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, tenant_id)
            VALUES (?, ?, ?, ?, ?)
            """,
            (-2001, "测试频道A", "channel", "channel_a", "tenant_a"),
        )
        await db.execute(
            """
            INSERT INTO group_messages (group_id, user_id, username, tg_msg_id, sender_type, content, message_ts, tenant_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (-1001, 88, "alice", 10, "user", "群里的一条真实消息", 3000, "tenant_a"),
        )
        await db.execute(
            """
            INSERT INTO group_messages (group_id, user_id, username, tg_msg_id, sender_type, content, message_ts, tenant_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (-1001, 99, "news_channel", 11, "channel", "频道身份发出的消息", 3001, "tenant_a"),
        )
        await db.execute(
            "INSERT INTO tracked_groups (group_id, group_title, tenant_id) VALUES (?, ?, ?)",
            (-1002, "其他租户群", "tenant_b"),
        )
        await db.commit()

    seen_llm_inputs = []
    seen_tool_prompt = []

    async def fake_llm(messages, model=""):
        latest = messages[-1]["content"]
        seen_llm_inputs.append(latest)
        if "[工具执行结果]" in latest:
            seen_tool_prompt.append(latest)
            return "你有 1 个群：测试群A；1 个频道：测试频道A。最近消息包含：群里的一条真实消息。", {"total_tokens": 12}
        return "我先查询绑定的群和频道。", {"total_tokens": 8}

    ctx = AgentContext(
        user_id=-99999,
        metadata={"role": "merchant_owner", "tenant_id": "tenant_a"},
    )
    agent = AIAgent(llm_call_fn=fake_llm)
    response = await agent.process_message("查看群聊", ctx)

    assert "system_query" in response.tool_calls_made
    assert "测试群A" in response.text
    assert "测试频道A" in response.text
    assert "其他租户群" not in response.text
    assert seen_tool_prompt == []
    assert seen_llm_inputs == []
    assert "请稍等" not in response.text

    from services.ai_agent.tools import SystemQueryTool

    tool = SystemQueryTool()
    group_rows = await tool.execute({"query_type": "group_messages", "limit": 5}, context=ctx)
    channel_rows = await tool.execute({"query_type": "channel_messages", "limit": 5}, context=ctx)

    assert group_rows["count"] == 2
    assert any(row["content"] == "群里的一条真实消息" for row in group_rows["data"])
    assert channel_rows["count"] == 1
    assert channel_rows["data"][0]["content"] == "频道身份发出的消息"


@pytest.mark.asyncio
async def test_agent_records_tool_exception_in_tool_results(monkeypatch):
    from services.ai_agent import core as core_mod
    from services.ai_agent.core import AIAgent
    from services.ai_agent.schema import AgentContext
    from services.ai_agent.tools import BaseTool

    class BrokenTool(BaseTool):
        name = "broken_query"
        description = "Broken read-only test tool"

        async def execute(self, params, context=None):
            raise RuntimeError("database timeout")

    calls = 0

    async def fake_llm(messages, model=""):
        nonlocal calls
        calls += 1
        if calls == 1:
            return (
                '__TOOL_CALL__{"function":{"name":"broken_query","arguments":"{}"}}',
                {"total_tokens": 8},
            )
        return "查询工具执行失败，请稍后重试。", {"total_tokens": 8}

    monkeypatch.setattr(
        core_mod,
        "get_tools_for_llm",
        lambda: [BrokenTool().schema_for_llm],
    )
    monkeypatch.setattr(
        core_mod,
        "get_tool",
        lambda name: BrokenTool() if name == "broken_query" else None,
    )

    response = await AIAgent(llm_call_fn=fake_llm).process_message(
        "查一下最近群广告消息",
        AgentContext(user_id=-99999),
    )

    assert response.tool_calls_made == ["broken_query"]
    assert response.tool_results["broken_query"]["success"] is False
    assert "database timeout" in response.tool_results["broken_query"]["message"]
    assert "失败" in response.text


@pytest.mark.asyncio
async def test_system_query_group_ad_messages_clamps_large_limit(tmp_path, monkeypatch):
    import config
    import database
    from services.ai_agent.schema import AgentContext
    from services.ai_agent.tools import SystemQueryTool

    db_path = tmp_path / "agent_group_ad_limit.sqlite3"
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    monkeypatch.setattr(database, "DB_NAME", str(db_path), raising=False)

    async with aiosqlite.connect(db_path) as db:
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER,
                tg_msg_id INTEGER,
                user_id INTEGER,
                username TEXT,
                content TEXT,
                msg_type TEXT DEFAULT 'text',
                sender_type TEXT DEFAULT 'user',
                message_ts INTEGER,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                is_advertising INTEGER DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT DEFAULT NULL,
                tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        for idx in range(60):
            await db.execute(
                """
                INSERT INTO group_messages (
                    group_id, tg_msg_id, user_id, username, content,
                    message_ts, is_advertising, ad_score, ad_confidence, tenant_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (-1001, idx + 1, 800 + idx, f"ad_{idx}", "注册送福利广告", 5000 + idx, 1, 90, "high", "tenant-a"),
            )
        await db.commit()

    ctx = AgentContext(
        user_id=123,
        metadata={"role": "merchant_owner", "tenant_id": "tenant-a", "merchant_id": 1},
    )
    result = await SystemQueryTool().execute(
        {"query_type": "group_ad_messages", "limit": 500},
        context=ctx,
    )

    assert result["success"] is True
    assert result["count"] == 50
    assert len(result["data"]) == 50


@pytest.mark.asyncio
async def test_api_ai_chat_uses_agent_for_virtual_assistant(monkeypatch):
    import json
    from services.ai_agent.schema import AgentResponse
    from backend.app.api import ai as ai_api

    class FakeQuota:
        async def check_available(self, tenant_id, amount=1):
            return {"allowed": True}

        async def record_usage(self, tenant_id, amount=1):
            return None

    class FakeRequest:
        match_info = {}
        app = {}

        def get(self, key, default=None):
            if key == "session":
                return {
                    "role": "platform_admin",
                    "tenant_id": "tenant-a",
                    "merchant_id": 7,
                    "authenticated": True,
                }
            return default

        async def json(self):
            return {
                "userId": -99999,
                "content": "查一下群里有没有广告",
                "model": "gemini-2.5-flash-lite",
            }

    async def fake_run_agent_chat(**kwargs):
        assert kwargs["user_id"] == -99999
        assert kwargs["scope"]["tenant_id"] == "tenant-a"
        return AgentResponse(
            text="查到 1 条广告证据",
            tool_calls_made=["system_query"],
            tool_results={
                "system_query": {
                    "success": True,
                    "source": "sql.group_messages",
                    "data_complete": True,
                    "analysis_allowed": True,
                    "evidence_count": 1,
                }
            },
        )

    monkeypatch.setattr(ai_api, "get_quota_service", lambda: FakeQuota())
    monkeypatch.setattr(ai_api, "save_ai_usage_log", lambda **kwargs: None)
    monkeypatch.setattr(ai_api, "_run_ai_agent_chat", fake_run_agent_chat)

    response = await ai_api.api_ai_chat(FakeRequest())
    payload = json.loads(response.text)

    assert payload["status"] == "ok"
    assert payload["reply"] == "查到 1 条广告证据"
    assert payload["tool_calls_made"] == ["system_query"]
    assert payload["evidence"]["system_query"]["evidence_count"] == 1
