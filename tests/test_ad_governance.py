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


class FakeBot:
    def __init__(self, *, can_delete=True, can_restrict=True, admin_user_ids=None):
        self.deleted = []
        self.delete_batches = []
        self.muted = []
        self.warnings = []
        self.can_delete = can_delete
        self.can_restrict = can_restrict
        self.admin_user_ids = set(admin_user_ids or [])

    async def get_me(self):
        return SimpleNamespace(id=999)

    async def get_chat(self, chat_id):
        return SimpleNamespace(id=chat_id, title="治理测试群", type="supergroup")

    async def get_chat_member(self, chat_id, user_id):
        if int(user_id) == 999:
            return SimpleNamespace(
                status="administrator", can_delete_messages=self.can_delete,
                can_restrict_members=self.can_restrict,
            )
        if int(user_id) in self.admin_user_ids:
            return SimpleNamespace(status="administrator")
        return SimpleNamespace(status="member")

    async def delete_messages(self, chat_id, message_ids):
        self.delete_batches.append(list(message_ids))
        self.deleted.extend((int(chat_id), int(mid)) for mid in message_ids)

    async def restrict_chat_member(self, chat_id, user_id, permissions, until_date):
        self.muted.append((int(chat_id), int(user_id)))

    async def send_message(self, chat_id, text, **kwargs):
        self.warnings.append((int(chat_id), text))


async def _make_db(path, rows):
    async with aiosqlite.connect(path) as db:
        await db.execute(
            """
            CREATE TABLE tracked_groups (
                group_id INTEGER PRIMARY KEY, group_title TEXT, tenant_id TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER, tg_msg_id INTEGER, user_id INTEGER, username TEXT,
                sender_type TEXT DEFAULT 'user', content TEXT, file_id TEXT,
                media_unique_id TEXT, msg_type TEXT DEFAULT 'text',
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP, message_ts INTEGER,
                is_advertising INTEGER DEFAULT 0, ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low', ad_cluster_id TEXT,
                ad_checked_at DATETIME, ad_action_status TEXT DEFAULT 'none',
                is_deleted INTEGER DEFAULT 0, deleted_at DATETIME, tenant_id TEXT DEFAULT 'default'
            )
            """
        )
        await db.execute(
            """
            CREATE TABLE ad_text_fingerprints (
                tenant_id TEXT NOT NULL DEFAULT 'default', text_hash TEXT NOT NULL,
                normalized_text TEXT NOT NULL, representative_text TEXT, source_msg_id INTEGER,
                confirmed_by TEXT DEFAULT 'agent', confirmed_count INTEGER DEFAULT 1,
                hit_count INTEGER DEFAULT 0, first_confirmed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                last_confirmed_at DATETIME DEFAULT CURRENT_TIMESTAMP, last_hit_at DATETIME,
                PRIMARY KEY (tenant_id,text_hash)
            )
            """
        )
        await db.execute(
            "INSERT INTO tracked_groups(group_id,group_title,tenant_id) VALUES(-1001,'治理测试群','default')"
        )
        await db.executemany(
            """
            INSERT INTO group_messages(
                group_id,tg_msg_id,user_id,username,content,message_ts,
                is_advertising,ad_score,tenant_id
            ) VALUES(-1001,?,?,?,?,?,?,?,'default')
            """,
            rows,
        )
        await db.commit()


async def _run_scan(
    tmp_path, monkeypatch, rows, *, known_text=None, classifier=None,
    dry_run=True, bot=None,
):
    import config
    from services.ad_governance import AdGovernanceService
    from services.ad_text import ad_text_hash, normalize_ad_text

    db_path = tmp_path / f"gov-{time.time_ns()}.sqlite3"
    await _make_db(db_path, rows)
    monkeypatch.setattr(config, "DB_NAME", str(db_path), raising=False)
    service = AdGovernanceService()
    await service.ensure_schema()
    if known_text:
        async with aiosqlite.connect(db_path) as db:
            await db.execute(
                """
                INSERT INTO ad_text_fingerprints(
                    tenant_id,text_hash,normalized_text,representative_text,confirmed_by
                ) VALUES('default',?,?,?,'test')
                """,
                (ad_text_hash(known_text), normalize_ad_text(known_text), known_text),
            )
            await db.commit()
    bot = bot or FakeBot()
    job = await service.create_job(
        tenant_id=None, group_id=-1001, requested_by="test", bot=bot, dry_run=dry_run
    )
    await service.start_job(
        job_id=job["job_id"], tenant_id=None, bot=bot, classifier=classifier
    )
    await service._tasks[job["job_id"]]
    return service, await service.get_job(job_id=job["job_id"], tenant_id=None), bot


@pytest.mark.asyncio
async def test_exact_five_warns_but_does_not_delete(tmp_path, monkeypatch):
    now = int(time.time())
    text = "长期稳定广告服务 联系 @seller01"
    rows = [(i, 700 + i, f"u{i}", text, now - i, 0, 0) for i in range(1, 6)]
    _, job, _ = await _run_scan(tmp_path, monkeypatch, rows, known_text=text)

    assert job["summary"]["planned_delete_count"] == 0
    assert job["summary"]["warning_sender_count"] == 5
    assert job["summary"]["muted_sender_count"] == 0


@pytest.mark.asyncio
async def test_exact_six_from_three_senders_plans_delete_and_mute(tmp_path, monkeypatch):
    now = int(time.time())
    text = "长期稳定广告服务 联系 @seller01"
    rows = [(i, 700 + i % 3, f"u{i}", text, now - i, 0, 0) for i in range(1, 7)]
    _, job, _ = await _run_scan(tmp_path, monkeypatch, rows, known_text=text)

    assert job["summary"]["planned_delete_count"] == 6
    assert job["summary"]["muted_sender_count"] == 3


@pytest.mark.asyncio
async def test_similar_five_variants_plan_one_family_delete(tmp_path, monkeypatch):
    now = int(time.time())
    base = "专业广告推广服务 联系 @seller01 长期稳定合作"
    rows = [
        (i, 700 + i, f"u{i}", f"专业广告推广服务 联系 @seller0{i} 长期稳定合作", now - i, 0, 0)
        for i in range(1, 6)
    ]
    _, job, _ = await _run_scan(tmp_path, monkeypatch, rows, known_text=base)

    assert job["summary"]["candidate_clusters"] == 1
    assert job["summary"]["planned_delete_count"] == 5


@pytest.mark.asyncio
async def test_old_exact_ads_only_seed_fingerprint(tmp_path, monkeypatch):
    old = int(time.time()) - 3 * 86400
    text = "长期稳定广告服务 联系 @seller01"
    rows = [(i, 700 + i % 3, f"u{i}", text, old - i, 0, 0) for i in range(1, 7)]
    _, job, _ = await _run_scan(tmp_path, monkeypatch, rows, known_text=text)

    assert job["summary"]["planned_delete_count"] == 0
    assert job["summary"]["fingerprint_only_count"] == 6


@pytest.mark.asyncio
async def test_thousand_identical_unknown_messages_use_one_llm_item(tmp_path, monkeypatch):
    now = int(time.time())
    rows = [(i, 700 + i % 5, f"u{i}", "普通未知重复模板", now - i, 0, 0) for i in range(1, 1001)]
    calls = []

    async def classifier(items):
        calls.append(items)
        return {
            items[0]["family_hash"]: {"is_ad": True, "confidence": 0.9, "reason": "批量推广"}
        }

    _, job, _ = await _run_scan(tmp_path, monkeypatch, rows, classifier=classifier)

    assert len(calls) == 1
    assert len(calls[0]) == 1
    assert job["summary"]["candidate_clusters"] == 1
    assert job["summary"]["confirmed_clusters"] == 1


def test_normalization_and_similarity_are_stable():
    from services.ad_governance import normalize_content, template_content, text_similarity

    assert normalize_content("ＡＢＣ\u200b  测试") == "abc 测试"
    assert template_content("联系 @seller01 https://t.me/a 123456") == "联系 [contact] [link] [number]"
    assert text_similarity("专业推广 联系 @seller01", "专业推广 联系 @seller02") == 1.0


def test_different_media_placeholders_do_not_share_a_cluster():
    from services.ad_governance import AdGovernanceService

    rows = [
        {"msg_id": 1, "content": "[图片]", "media_unique_id": "photo-a", "ad_score": 0},
        {"msg_id": 2, "content": "[图片]", "media_unique_id": "photo-b", "ad_score": 0},
        {"msg_id": 3, "content": "[图片]", "media_unique_id": "photo-a", "ad_score": 0},
    ]
    service = AdGovernanceService()
    exact = service._exact_clusters(rows)
    families = service._similar_families(exact)

    assert sorted(len(item["rows"]) for item in exact) == [1, 2]
    assert len(families) == 2


@pytest.mark.asyncio
async def test_real_execution_batches_deletes_by_one_hundred(tmp_path, monkeypatch):
    import config

    monkeypatch.setattr(config, "AD_AUTO_ENFORCE_ENABLED", True, raising=False)
    monkeypatch.setattr(config, "AD_DELETE_ENABLED", True, raising=False)
    now = int(time.time())
    text = "批量广告推广 联系 @seller01"
    rows = [(i, 700 + i % 5, f"u{i}", text, now - i, 0, 0) for i in range(1, 206)]
    bot = FakeBot()
    _, job, bot = await _run_scan(
        tmp_path, monkeypatch, rows, known_text=text, dry_run=False, bot=bot,
    )

    assert [len(batch) for batch in bot.delete_batches] == [100, 100, 5]
    assert job["summary"]["deleted_count"] == 205


@pytest.mark.asyncio
async def test_real_execution_without_delete_permission_is_safe(tmp_path, monkeypatch):
    import config

    monkeypatch.setattr(config, "AD_AUTO_ENFORCE_ENABLED", True, raising=False)
    monkeypatch.setattr(config, "AD_DELETE_ENABLED", True, raising=False)
    now = int(time.time())
    text = "无权限广告推广 联系 @seller01"
    rows = [(i, 700 + i % 3, f"u{i}", text, now - i, 0, 0) for i in range(1, 7)]
    bot = FakeBot(can_delete=False, can_restrict=False)
    _, job, bot = await _run_scan(
        tmp_path, monkeypatch, rows, known_text=text, dry_run=False, bot=bot,
    )

    assert bot.delete_batches == []
    assert bot.muted == []
    assert job["summary"]["failure_count"] == 6


@pytest.mark.asyncio
async def test_administrator_is_never_muted(tmp_path, monkeypatch):
    import config

    monkeypatch.setattr(config, "AD_AUTO_ENFORCE_ENABLED", True, raising=False)
    monkeypatch.setattr(config, "AD_DELETE_ENABLED", True, raising=False)
    now = int(time.time())
    text = "管理员排除广告 联系 @seller01"
    rows = [(i, 700 + i % 3, f"u{i}", text, now - i, 0, 0) for i in range(1, 7)]
    bot = FakeBot(admin_user_ids={700})
    _, _, bot = await _run_scan(
        tmp_path, monkeypatch, rows, known_text=text, dry_run=False, bot=bot,
    )

    assert all(user_id != 700 for _, user_id in bot.muted)
