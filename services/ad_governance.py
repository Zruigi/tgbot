# -*- coding: utf-8 -*-
"""Persistent, tenant-scoped advertisement scanning and enforcement."""

from __future__ import annotations

import asyncio
import hashlib
import html
import json
import logging
import re
import time
import unicodedata
import uuid
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from typing import Any, Awaitable, Callable, Dict, Iterable, List, Optional, Sequence, Set, Tuple

import aiosqlite

import config
from services.ai_agent.ad_detection import detect_advertising_signals
from services.ad_text import ad_text_hash

logger = logging.getLogger(__name__)

Classifier = Callable[[List[Dict[str, Any]]], Awaitable[Dict[str, Dict[str, Any]]]]

_URL_RE = re.compile(r"https?://\S+|(?:t\.me|telegram\.me)/\S+", re.I)
_CONTACT_RE = re.compile(r"@[A-Za-z0-9_]{4,32}")
_ZERO_WIDTH_RE = re.compile(r"[\u200b-\u200f\u2060\ufeff]")
_SPACE_RE = re.compile(r"\s+")
_MEDIA_PLACEHOLDER_RE = re.compile(
    r"^\[(?:图片|动图|表情包|视频|语音|音频|文件|媒体|photo|animation|sticker|video|voice|audio|document|media)\](?:\s+[^\s]+)?$",
    re.I,
)


def normalize_content(text: str) -> str:
    value = unicodedata.normalize("NFKC", str(text or "")).lower()
    value = _ZERO_WIDTH_RE.sub("", value)
    return _SPACE_RE.sub(" ", value).strip()


def template_content(text: str) -> str:
    value = normalize_content(text)
    value = _URL_RE.sub("[link]", value)
    value = _CONTACT_RE.sub("[contact]", value)
    value = re.sub(r"\d{5,}", "[number]", value)
    return _SPACE_RE.sub(" ", value).strip()


def stable_hash(value: str) -> str:
    return hashlib.md5(str(value or "").encode("utf-8")).hexdigest()


def _ngrams(value: str, size: int = 3) -> Set[str]:
    compact = re.sub(r"\s+", "", value)
    if len(compact) <= size:
        return {compact} if compact else set()
    return {compact[i:i + size] for i in range(len(compact) - size + 1)}


def text_similarity(left: str, right: str) -> float:
    if left == right:
        return 1.0
    a = _ngrams(template_content(left))
    b = _ngrams(template_content(right))
    if not a or not b:
        return 0.0
    return (2.0 * len(a & b)) / (len(a) + len(b))


def extract_entities(text: str) -> Dict[str, List[str]]:
    raw = str(text or "")
    return {
        "urls": sorted(set(_URL_RE.findall(raw))),
        "contacts": sorted(set(_CONTACT_RE.findall(raw))),
    }


def _tenant(tenant_id: Optional[str]) -> str:
    value = str(tenant_id or "").strip()
    return value if value and value != "platform" else "default"


class AdGovernanceService:
    def __init__(self) -> None:
        self._schema_ready = False
        self._tasks: Dict[str, asyncio.Task] = {}
        self._group_locks: Dict[Tuple[str, int], asyncio.Lock] = {}

    async def ensure_schema(self) -> None:
        if self._schema_ready:
            return
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            await db.executescript(
                """
                CREATE TABLE IF NOT EXISTS ad_group_policies (
                    tenant_id TEXT NOT NULL DEFAULT 'default',
                    group_id INTEGER NOT NULL,
                    is_enabled BOOLEAN NOT NULL DEFAULT 0,
                    warning_similarity REAL NOT NULL DEFAULT 0.50,
                    warning_count INTEGER NOT NULL DEFAULT 3,
                    delete_similarity REAL NOT NULL DEFAULT 0.80,
                    delete_count INTEGER NOT NULL DEFAULT 5,
                    exact_delete_count INTEGER NOT NULL DEFAULT 6,
                    mute_min_senders INTEGER NOT NULL DEFAULT 3,
                    mute_min_messages INTEGER NOT NULL DEFAULT 6,
                    mute_seconds INTEGER NOT NULL DEFAULT 2592000,
                    activated_by TEXT DEFAULT NULL,
                    activated_at DATETIME DEFAULT NULL,
                    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (tenant_id, group_id)
                );

                CREATE TABLE IF NOT EXISTS ad_scan_jobs (
                    job_id TEXT PRIMARY KEY,
                    tenant_id TEXT NOT NULL DEFAULT 'default',
                    group_id INTEGER NOT NULL,
                    group_title TEXT DEFAULT NULL,
                    requested_by TEXT DEFAULT NULL,
                    status TEXT NOT NULL DEFAULT 'prepared',
                    progress INTEGER NOT NULL DEFAULT 0,
                    dry_run BOOLEAN NOT NULL DEFAULT 1,
                    activate_policy BOOLEAN NOT NULL DEFAULT 0,
                    source TEXT NOT NULL DEFAULT 'agent',
                    total_messages INTEGER NOT NULL DEFAULT 0,
                    candidate_clusters INTEGER NOT NULL DEFAULT 0,
                    confirmed_clusters INTEGER NOT NULL DEFAULT 0,
                    fingerprint_only_count INTEGER NOT NULL DEFAULT 0,
                    planned_delete_count INTEGER NOT NULL DEFAULT 0,
                    deleted_count INTEGER NOT NULL DEFAULT 0,
                    warning_sender_count INTEGER NOT NULL DEFAULT 0,
                    muted_sender_count INTEGER NOT NULL DEFAULT 0,
                    failure_count INTEGER NOT NULL DEFAULT 0,
                    summary_json TEXT DEFAULT NULL,
                    error TEXT DEFAULT NULL,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    started_at DATETIME DEFAULT NULL,
                    finished_at DATETIME DEFAULT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_ad_scan_jobs_scope
                    ON ad_scan_jobs(tenant_id, group_id, created_at DESC);

                CREATE TABLE IF NOT EXISTS ad_governance_clusters (
                    cluster_id TEXT PRIMARY KEY,
                    job_id TEXT NOT NULL,
                    tenant_id TEXT NOT NULL DEFAULT 'default',
                    group_id INTEGER NOT NULL,
                    representative_text TEXT,
                    exact_hash TEXT,
                    template_hash TEXT,
                    similarity REAL NOT NULL DEFAULT 1.0,
                    occurrence_count INTEGER NOT NULL DEFAULT 0,
                    recent_count INTEGER NOT NULL DEFAULT 0,
                    sender_count INTEGER NOT NULL DEFAULT 0,
                    classification TEXT NOT NULL DEFAULT 'pending',
                    classification_source TEXT DEFAULT NULL,
                    confidence REAL NOT NULL DEFAULT 0,
                    reason TEXT DEFAULT NULL,
                    action_tier TEXT NOT NULL DEFAULT 'none',
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
                );
                CREATE INDEX IF NOT EXISTS idx_ad_gov_clusters_job
                    ON ad_governance_clusters(job_id, classification, action_tier);

                CREATE TABLE IF NOT EXISTS ad_governance_cluster_messages (
                    cluster_id TEXT NOT NULL,
                    msg_id INTEGER NOT NULL,
                    group_id INTEGER NOT NULL,
                    tg_msg_id INTEGER,
                    sender_id INTEGER,
                    message_ts INTEGER,
                    exact_hash TEXT,
                    similarity REAL NOT NULL DEFAULT 1.0,
                    is_actionable BOOLEAN NOT NULL DEFAULT 0,
                    PRIMARY KEY (cluster_id, msg_id)
                );
                CREATE INDEX IF NOT EXISTS idx_ad_gov_cluster_messages_msg
                    ON ad_governance_cluster_messages(group_id, tg_msg_id);

                CREATE TABLE IF NOT EXISTS ad_governance_actions (
                    action_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    action_key TEXT NOT NULL UNIQUE,
                    job_id TEXT,
                    tenant_id TEXT NOT NULL DEFAULT 'default',
                    group_id INTEGER NOT NULL,
                    cluster_id TEXT,
                    action_type TEXT NOT NULL,
                    target_id TEXT,
                    status TEXT NOT NULL DEFAULT 'planned',
                    dry_run BOOLEAN NOT NULL DEFAULT 1,
                    payload_json TEXT DEFAULT NULL,
                    error TEXT DEFAULT NULL,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    executed_at DATETIME DEFAULT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_ad_gov_actions_job
                    ON ad_governance_actions(job_id, action_type, status);

                CREATE TABLE IF NOT EXISTS ad_warning_dedup (
                    tenant_id TEXT NOT NULL DEFAULT 'default',
                    group_id INTEGER NOT NULL,
                    family_hash TEXT NOT NULL,
                    sender_id INTEGER NOT NULL,
                    window_key TEXT NOT NULL,
                    warned_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (tenant_id, group_id, family_hash, sender_id, window_key)
                );
                """
            )
            await db.commit()
        self._schema_ready = True

    async def list_groups(self, *, tenant_id: Optional[str], bot: Any) -> List[Dict[str, Any]]:
        await self.ensure_schema()
        effective_tenant = _tenant(tenant_id)
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            db.row_factory = aiosqlite.Row
            cols = {row[1] for row in await (await db.execute("PRAGMA table_info(tracked_groups)")).fetchall()}
            tenant_sql = "WHERE tenant_id = ?" if tenant_id and "tenant_id" in cols else ""
            params: Tuple[Any, ...] = (effective_tenant,) if tenant_sql else ()
            rows = [dict(row) for row in await (await db.execute(
                f"SELECT group_id, group_title FROM tracked_groups {tenant_sql} ORDER BY updated_at DESC",
                params,
            )).fetchall()]
            counts = {
                int(row["group_id"]): int(row["n"])
                for row in await (await db.execute(
                    "SELECT group_id, COUNT(*) n FROM group_messages GROUP BY group_id"
                )).fetchall()
            }

        if not bot:
            return []
        try:
            me = await bot.get_me()
            bot_id = int(me.id)
        except Exception:
            return []

        result: List[Dict[str, Any]] = []
        for row in rows:
            group_id = int(row["group_id"])
            try:
                chat = await bot.get_chat(group_id)
                member = await bot.get_chat_member(group_id, bot_id)
                status = str(getattr(member, "status", "") or "")
                result.append({
                    "group_id": group_id,
                    "title": getattr(chat, "title", None) or row.get("group_title") or str(group_id),
                    "chat_type": str(getattr(chat, "type", "") or ""),
                    "message_count": counts.get(group_id, 0),
                    "bot_status": status,
                    "can_delete_messages": status == "creator" or (
                        status == "administrator" and bool(getattr(member, "can_delete_messages", False))
                    ),
                    "can_restrict_members": status == "creator" or (
                        status == "administrator" and bool(getattr(member, "can_restrict_members", False))
                    ),
                })
            except Exception:
                continue
        return result

    async def create_job(
        self,
        *,
        tenant_id: Optional[str],
        group_id: int,
        requested_by: str,
        bot: Any,
        dry_run: bool = True,
        activate_policy: bool = False,
        source: str = "agent",
    ) -> Dict[str, Any]:
        await self.ensure_schema()
        effective_tenant = _tenant(tenant_id)
        groups = await self.list_groups(tenant_id=tenant_id, bot=bot)
        selected = next((item for item in groups if int(item["group_id"]) == int(group_id)), None)
        if not selected:
            raise ValueError("该群未通过 Telegram 官方 API 验证或不属于当前租户")
        job_id = "scan_" + uuid.uuid4().hex
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            await db.execute(
                """
                INSERT INTO ad_scan_jobs (
                    job_id, tenant_id, group_id, group_title, requested_by,
                    dry_run, activate_policy, source, status, total_messages
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'prepared', ?)
                """,
                (
                    job_id, effective_tenant, int(group_id), selected.get("title"), requested_by,
                    1 if dry_run else 0, 1 if activate_policy else 0, source,
                    int(selected.get("message_count") or 0),
                ),
            )
            await db.commit()
        return await self.get_job(job_id=job_id, tenant_id=effective_tenant)

    async def get_job(self, *, job_id: str, tenant_id: Optional[str]) -> Dict[str, Any]:
        await self.ensure_schema()
        effective_tenant = _tenant(tenant_id)
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            db.row_factory = aiosqlite.Row
            row = await (await db.execute(
                "SELECT * FROM ad_scan_jobs WHERE job_id = ? AND tenant_id = ?",
                (job_id, effective_tenant),
            )).fetchone()
        if not row:
            raise ValueError("扫描任务不存在或无权访问")
        data = dict(row)
        try:
            data["summary"] = json.loads(data.get("summary_json") or "{}")
        except Exception:
            data["summary"] = {}
        data.pop("summary_json", None)
        return data

    async def get_clusters(
        self, *, job_id: str, tenant_id: Optional[str], tier: Optional[str] = None, limit: int = 100
    ) -> List[Dict[str, Any]]:
        await self.get_job(job_id=job_id, tenant_id=tenant_id)
        where = "WHERE job_id = ?"
        params: List[Any] = [job_id]
        if tier == "delete":
            where += " AND action_tier IN ('similar_delete','exact_delete','exact_delete_mute')"
        elif tier == "pending":
            where += " AND classification = 'pending'"
        elif tier:
            where += " AND action_tier = ?"
            params.append(str(tier))
        params.append(max(1, min(int(limit or 100), 500)))
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            db.row_factory = aiosqlite.Row
            rows = await (await db.execute(
                f"SELECT * FROM ad_governance_clusters {where} ORDER BY recent_count DESC, occurrence_count DESC LIMIT ?",
                tuple(params),
            )).fetchall()
        return [dict(row) for row in rows]

    async def cancel_job(self, *, job_id: str, tenant_id: Optional[str]) -> Dict[str, Any]:
        job = await self.get_job(job_id=job_id, tenant_id=tenant_id)
        if job["status"] not in {"prepared", "queued"}:
            return job
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            await db.execute(
                "UPDATE ad_scan_jobs SET status='cancelled', finished_at=CURRENT_TIMESTAMP WHERE job_id=?",
                (job_id,),
            )
            await db.commit()
        return await self.get_job(job_id=job_id, tenant_id=tenant_id)

    async def start_job(
        self, *, job_id: str, tenant_id: Optional[str], bot: Any, classifier: Optional[Classifier] = None
    ) -> Dict[str, Any]:
        job = await self.get_job(job_id=job_id, tenant_id=tenant_id)
        if job["status"] not in {"prepared", "queued"}:
            return job
        if job["status"] == "prepared":
            async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
                await db.execute("UPDATE ad_scan_jobs SET status='queued' WHERE job_id=? AND status='prepared'", (job_id,))
                await db.commit()
        if job_id not in self._tasks or self._tasks[job_id].done():
            self._tasks[job_id] = asyncio.create_task(
                self._run_job(job_id=job_id, tenant_id=_tenant(tenant_id), bot=bot, classifier=classifier)
            )
        return await self.get_job(job_id=job_id, tenant_id=tenant_id)

    async def configure_job(
        self, *, job_id: str, tenant_id: Optional[str], dry_run: bool, activate_policy: bool
    ) -> Dict[str, Any]:
        job = await self.get_job(job_id=job_id, tenant_id=tenant_id)
        if job["status"] != "prepared":
            return job
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            await db.execute(
                "UPDATE ad_scan_jobs SET dry_run=?, activate_policy=? WHERE job_id=? AND status='prepared'",
                (1 if dry_run else 0, 1 if activate_policy else 0, job_id),
            )
            await db.commit()
        return await self.get_job(job_id=job_id, tenant_id=tenant_id)

    async def resume_pending_jobs(self, *, bot: Any, classifier: Optional[Classifier] = None) -> int:
        """Recover only previously confirmed jobs after a process restart."""
        await self.ensure_schema()
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            await db.execute(
                "UPDATE ad_scan_jobs SET status='queued' WHERE status IN ('scanning','classifying')"
            )
            rows = await (await db.execute(
                "SELECT job_id, tenant_id FROM ad_scan_jobs WHERE status='queued' ORDER BY created_at"
            )).fetchall()
            await db.commit()
        for job_id, tenant_id in rows:
            if job_id not in self._tasks or self._tasks[job_id].done():
                self._tasks[job_id] = asyncio.create_task(
                    self._run_job(
                        job_id=str(job_id), tenant_id=str(tenant_id), bot=bot,
                        classifier=classifier,
                    )
                )
        return len(rows)

    async def enable_policy(
        self, *, tenant_id: Optional[str], group_id: int, activated_by: str
    ) -> Dict[str, Any]:
        await self.ensure_schema()
        effective_tenant = _tenant(tenant_id)
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            await db.execute(
                """
                INSERT INTO ad_group_policies (tenant_id,group_id,is_enabled,activated_by,activated_at)
                VALUES (?,?,1,?,CURRENT_TIMESTAMP)
                ON CONFLICT(tenant_id,group_id) DO UPDATE SET
                    is_enabled=1, activated_by=excluded.activated_by,
                    activated_at=CURRENT_TIMESTAMP, updated_at=CURRENT_TIMESTAMP
                """,
                (effective_tenant, int(group_id), activated_by),
            )
            await db.commit()
        return {"tenant_id": effective_tenant, "group_id": int(group_id), "is_enabled": True}

    async def _set_job(self, job_id: str, **values: Any) -> None:
        if not values:
            return
        columns = ", ".join(f"{key} = ?" for key in values)
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            await db.execute(f"UPDATE ad_scan_jobs SET {columns} WHERE job_id = ?", (*values.values(), job_id))
            await db.commit()

    async def _load_messages(self, group_id: int, tenant_id: str, *, recent_only: bool = False) -> List[Dict[str, Any]]:
        async with aiosqlite.connect(config.DB_NAME, timeout=30) as db:
            db.row_factory = aiosqlite.Row
            cols = {row[1] for row in await (await db.execute("PRAGMA table_info(group_messages)")).fetchall()}
            media_expr = "media_unique_id" if "media_unique_id" in cols else "file_id AS media_unique_id"
            tenant_sql = "AND tenant_id = ?" if "tenant_id" in cols else ""
            recent_sql = "AND COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) >= ?" if recent_only else ""
            params_list: List[Any] = [int(group_id)]
            if tenant_sql:
                params_list.append(tenant_id)
            if recent_only:
                params_list.append(int(time.time()) - 86400)
            rows = await (await db.execute(
                f"""
                SELECT msg_id, group_id, tg_msg_id, user_id, username, sender_type,
                       content, file_id, {media_expr}, msg_type,
                       COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) message_ts,
                       COALESCE(is_advertising,0) is_advertising,
                       COALESCE(ad_score,0) ad_score,
                       COALESCE(is_deleted,0) is_deleted
                FROM group_messages
                WHERE group_id = ? {tenant_sql}
                  {recent_sql}
                  AND COALESCE(sender_type,'user') = 'user'
                  AND COALESCE(content,'') != ''
                ORDER BY message_ts ASC, msg_id ASC
                """,
                tuple(params_list),
            )).fetchall()
        return [dict(row) for row in rows]

    async def _known_hashes(self, tenant_id: str) -> Set[str]:
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            rows = await (await db.execute(
                "SELECT text_hash FROM ad_text_fingerprints WHERE tenant_id = ?",
                (tenant_id,),
            )).fetchall()
        return {str(row[0]) for row in rows}

    def _exact_clusters(self, rows: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        by_hash: Dict[str, Dict[str, Any]] = {}
        for row in rows:
            normalized = normalize_content(row.get("content") or "")
            media_signature = str(row.get("media_unique_id") or "").strip()
            is_media_only = bool(media_signature and _MEDIA_PLACEHOLDER_RE.match(normalized))
            exact_basis = f"{normalized}|media:{media_signature}" if is_media_only else normalized
            exact_hash = stable_hash(exact_basis)
            item = by_hash.setdefault(exact_hash, {
                "exact_hash": exact_hash,
                "fingerprint_hash": ad_text_hash(exact_basis),
                "template_hash": stable_hash(
                    f"{template_content(normalized)}|media:{media_signature}" if is_media_only
                    else template_content(normalized)
                ),
                "representative_text": row.get("content") or "",
                "is_media_only": is_media_only,
                "media_signature": media_signature if is_media_only else None,
                "rows": [],
                "max_stored_score": 0,
                "stored_ad": False,
            })
            enriched = dict(row)
            enriched["exact_hash"] = exact_hash
            enriched["entities"] = extract_entities(row.get("content") or "")
            item["rows"].append(enriched)
            item["max_stored_score"] = max(item["max_stored_score"], int(row.get("ad_score") or 0))
            item["stored_ad"] = item["stored_ad"] or bool(row.get("is_advertising"))
        return list(by_hash.values())

    def _candidate_exact_clusters(self, clusters: List[Dict[str, Any]], known: Set[str]) -> List[Dict[str, Any]]:
        candidates = []
        for cluster in clusters:
            text = cluster["representative_text"]
            flags = detect_advertising_signals(text, "group_message")
            entities = extract_entities(text)
            count = len(cluster["rows"])
            candidate = (
                cluster["fingerprint_hash"] in known
                or cluster["stored_ad"]
                or bool(flags.get("is_advertising"))
                or count >= 3
                or bool(entities["urls"] or entities["contacts"])
            )
            if not candidate:
                continue
            cluster["live_detection"] = flags
            cluster["known"] = cluster["fingerprint_hash"] in known
            candidates.append(cluster)
        return candidates

    def _similar_families(self, clusters: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        ordered = sorted(clusters, key=lambda item: len(item["rows"]), reverse=True)
        families: List[Dict[str, Any]] = []
        for cluster in ordered:
            text = cluster["representative_text"]
            best: Optional[Dict[str, Any]] = None
            best_score = 0.0
            for family in families:
                representative = family["members"][0][0]
                if cluster.get("is_media_only") or representative.get("is_media_only"):
                    if not (
                        cluster.get("is_media_only")
                        and representative.get("is_media_only")
                        and cluster.get("media_signature") == representative.get("media_signature")
                    ):
                        continue
                left_len = max(1, len(template_content(text)))
                right_len = max(1, len(template_content(family["representative_text"])))
                if min(left_len, right_len) / max(left_len, right_len) < 0.55:
                    continue
                score = text_similarity(text, family["representative_text"])
                if score >= 0.50 and score > best_score:
                    best = family
                    best_score = score
            if best is None:
                families.append({
                    "family_hash": cluster["template_hash"],
                    "representative_text": text,
                    "members": [(cluster, 1.0)],
                })
            else:
                best["members"].append((cluster, best_score))
        return families

    async def _persist_cluster(
        self, *, job: Dict[str, Any], family: Dict[str, Any], classification: Dict[str, Any], now_ts: int
    ) -> Dict[str, Any]:
        cluster_id = f"gov:{job['job_id']}:{family['family_hash']}"
        rows: List[Tuple[Dict[str, Any], float]] = family["members"]
        all_messages = [message for exact, _ in rows for message in exact["rows"]]
        recent = [message for message in all_messages if int(message.get("message_ts") or 0) >= now_ts - 86400 and not message.get("is_deleted")]
        senders = {int(message.get("user_id") or 0) for message in recent if message.get("user_id")}
        min_similarity = min((score for _, score in rows), default=1.0)
        exact_recent_groups = [
            [m for m in exact["rows"] if int(m.get("message_ts") or 0) >= now_ts - 86400 and not m.get("is_deleted")]
            for exact, _ in rows
        ]
        exact_action_messages = max(exact_recent_groups, key=len, default=[])
        exact_max = len(exact_action_messages)
        action_messages = recent
        action_senders = senders
        confirmed = bool(classification.get("is_ad"))
        action_tier = "none"
        if confirmed and recent:
            if exact_max >= 6:
                action_senders = {int(message.get("user_id") or 0) for message in exact_action_messages if message.get("user_id")}
                action_messages = exact_action_messages
                action_tier = "exact_delete_mute" if len(action_senders) >= 3 else "exact_delete"
            elif len(rows) > 1 and min_similarity >= 0.80 and len(recent) >= 5:
                action_tier = "similar_delete"
            elif len(recent) >= 3:
                action_tier = "warn"

        async with aiosqlite.connect(config.DB_NAME, timeout=30) as db:
            await db.execute(
                """
                INSERT OR REPLACE INTO ad_governance_clusters (
                    cluster_id, job_id, tenant_id, group_id, representative_text,
                    exact_hash, template_hash, similarity, occurrence_count,
                    recent_count, sender_count, classification, classification_source,
                    confidence, reason, action_tier
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    cluster_id, job["job_id"], job["tenant_id"], int(job["group_id"]),
                    family["representative_text"], rows[0][0]["exact_hash"], family["family_hash"],
                    min_similarity, len(all_messages), len(recent), len(senders),
                    "ad" if confirmed else "not_ad" if classification.get("decided") else "pending",
                    classification.get("source"), float(classification.get("confidence") or 0),
                    classification.get("reason"), action_tier,
                ),
            )
            for exact, similarity in rows:
                for message in exact["rows"]:
                    actionable = int(message.get("message_ts") or 0) >= now_ts - 86400 and not message.get("is_deleted")
                    await db.execute(
                        """
                        INSERT OR REPLACE INTO ad_governance_cluster_messages (
                            cluster_id, msg_id, group_id, tg_msg_id, sender_id,
                            message_ts, exact_hash, similarity, is_actionable
                        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                        """,
                        (
                            cluster_id, int(message["msg_id"]), int(message["group_id"]),
                            message.get("tg_msg_id"), message.get("user_id"), message.get("message_ts"),
                            exact["exact_hash"], similarity, 1 if actionable else 0,
                        ),
                    )
                    if confirmed:
                        await db.execute(
                            """
                            UPDATE group_messages
                            SET is_advertising=1,
                                ad_score=CASE WHEN COALESCE(ad_score,0)<90 THEN 90 ELSE ad_score END,
                                ad_confidence='high', ad_cluster_id=?, ad_checked_at=CURRENT_TIMESTAMP
                            WHERE msg_id=?
                            """,
                            (cluster_id, int(message["msg_id"])),
                        )
                if confirmed:
                    await db.execute(
                        """
                        INSERT INTO ad_text_fingerprints (
                            tenant_id, text_hash, normalized_text, representative_text,
                            source_msg_id, confirmed_by, confirmed_count
                        ) VALUES (?, ?, ?, ?, ?, ?, 1)
                        ON CONFLICT(tenant_id,text_hash) DO UPDATE SET
                            representative_text=excluded.representative_text,
                            confirmed_by=excluded.confirmed_by,
                            last_confirmed_at=CURRENT_TIMESTAMP
                        """,
                        (
                            job["tenant_id"], exact["fingerprint_hash"],
                            normalize_content(exact["representative_text"]), exact["representative_text"],
                            int(exact["rows"][0]["msg_id"]), classification.get("source") or "scan",
                        ),
                    )
            await db.commit()
        return {
            "cluster_id": cluster_id,
            "confirmed": confirmed,
            "action_tier": action_tier,
            "recent": recent,
            "action_messages": action_messages,
            "senders": action_senders,
            "family_hash": family["family_hash"],
        }

    async def _record_action(
        self, *, job: Dict[str, Any], cluster_id: str, action_type: str, target_id: str,
        status: str, payload: Dict[str, Any], error: Optional[str] = None
    ) -> bool:
        action_key = f"{job['tenant_id']}:{job['group_id']}:{cluster_id}:{action_type}:{target_id}:{int(time.time()) // 86400}"
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            cur = await db.execute(
                """
                INSERT OR IGNORE INTO ad_governance_actions (
                    action_key, job_id, tenant_id, group_id, cluster_id,
                    action_type, target_id, status, dry_run, payload_json, error, executed_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
                """,
                (
                    action_key, job["job_id"], job["tenant_id"], int(job["group_id"]), cluster_id,
                    action_type, target_id, status, int(job["dry_run"]),
                    json.dumps(payload, ensure_ascii=False, default=str), error,
                ),
            )
            await db.commit()
            return cur.rowcount == 1

    async def _claim_warning_senders(
        self, *, tenant_id: str, group_id: int, family_hash: str, sender_ids: Sequence[int]
    ) -> List[int]:
        window_key = str(int(time.time()) // 86400)
        claimed: List[int] = []
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            for sender_id in sender_ids:
                recent = await (await db.execute(
                    """
                    SELECT 1 FROM ad_warning_dedup
                    WHERE tenant_id=? AND group_id=? AND family_hash=? AND sender_id=?
                      AND warned_at > datetime('now','-24 hours')
                    LIMIT 1
                    """,
                    (tenant_id, int(group_id), family_hash, int(sender_id)),
                )).fetchone()
                if recent:
                    continue
                cur = await db.execute(
                    """
                    INSERT OR IGNORE INTO ad_warning_dedup (
                        tenant_id, group_id, family_hash, sender_id, window_key
                    ) VALUES (?, ?, ?, ?, ?)
                    """,
                    (tenant_id, int(group_id), family_hash, int(sender_id), window_key),
                )
                if cur.rowcount == 1:
                    claimed.append(int(sender_id))
            await db.commit()
        return claimed

    async def _execute_cluster(self, *, job: Dict[str, Any], plan: Dict[str, Any], bot: Any) -> Dict[str, int]:
        counts = {"planned_delete": 0, "deleted": 0, "warnings": 0, "muted": 0, "failures": 0}
        tier = plan["action_tier"]
        if tier == "none":
            return counts
        group_id = int(job["group_id"])
        messages = [m for m in plan.get("action_messages", plan["recent"]) if m.get("tg_msg_id")]
        sender_ids = sorted(int(uid) for uid in plan["senders"] if uid and int(uid) != int(config.ADMIN_ID))
        dry_run = bool(job["dry_run"])
        if tier in {"similar_delete", "exact_delete", "exact_delete_mute"}:
            counts["planned_delete"] = len(messages)
        if dry_run:
            if tier == "warn":
                counts["warnings"] = len(sender_ids)
            if tier == "exact_delete_mute" and len(sender_ids) >= 3:
                counts["muted"] = len(sender_ids)
            await self._record_action(
                job=job, cluster_id=plan["cluster_id"], action_type="dry_run", target_id="batch",
                status="planned", payload={"tier": tier, "message_count": len(messages), "sender_ids": sender_ids},
            )
            return counts
        if not bool(getattr(config, "AD_AUTO_ENFORCE_ENABLED", False)):
            counts["failures"] += 1
            return counts

        try:
            me = await bot.get_me()
            bot_member = await bot.get_chat_member(group_id, int(me.id))
            status = str(getattr(bot_member, "status", "") or "")
            can_delete = status == "creator" or (status == "administrator" and bool(getattr(bot_member, "can_delete_messages", False)))
            can_restrict = status == "creator" or (status == "administrator" and bool(getattr(bot_member, "can_restrict_members", False)))
        except Exception:
            counts["failures"] += 1
            return counts

        safe_senders: List[int] = []
        for sender_id in sender_ids:
            try:
                member = await bot.get_chat_member(group_id, sender_id)
                if str(getattr(member, "status", "") or "") not in {"administrator", "creator"}:
                    safe_senders.append(sender_id)
            except Exception:
                counts["failures"] += 1

        delete_enabled = bool(getattr(config, "AD_DELETE_ENABLED", False))
        if tier in {"similar_delete", "exact_delete", "exact_delete_mute"} and can_delete and delete_enabled:
            ids = sorted({int(message["tg_msg_id"]) for message in messages})
            for offset in range(0, len(ids), 100):
                batch = ids[offset:offset + 100]
                try:
                    await bot.delete_messages(chat_id=group_id, message_ids=batch)
                    counts["deleted"] += len(batch)
                    async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
                        placeholders = ",".join("?" for _ in batch)
                        await db.execute(
                            f"UPDATE group_messages SET is_deleted=1, deleted_at=CURRENT_TIMESTAMP, ad_action_status='deleted' WHERE group_id=? AND tg_msg_id IN ({placeholders})",
                            (group_id, *batch),
                        )
                        await db.commit()
                    await self._record_action(
                        job=job, cluster_id=plan["cluster_id"], action_type="delete",
                        target_id=f"{batch[0]}:{batch[-1]}", status="completed",
                        payload={"message_ids": batch},
                    )
                except Exception as exc:
                    counts["failures"] += len(batch)
                    await self._record_action(
                        job=job, cluster_id=plan["cluster_id"], action_type="delete", target_id=str(batch[0]),
                        status="failed", payload={"message_ids": batch}, error=str(exc)[:300],
                    )
        elif tier in {"similar_delete", "exact_delete", "exact_delete_mute"}:
            counts["failures"] += len(messages)

        if tier == "warn" and safe_senders:
            safe_senders = await self._claim_warning_senders(
                tenant_id=job["tenant_id"], group_id=group_id,
                family_hash=plan["family_hash"], sender_ids=safe_senders,
            )
        if tier == "warn" and safe_senders:
            mentions = " ".join(f'<a href="tg://user?id={uid}">{uid}</a>' for uid in safe_senders)
            try:
                await bot.send_message(
                    chat_id=group_id,
                    text=f"{mentions}\n请勿重复发布高度相似的推广信息；继续刷屏将触发自动清理。",
                    parse_mode="HTML",
                )
                counts["warnings"] = len(safe_senders)
                await self._record_action(
                    job=job, cluster_id=plan["cluster_id"], action_type="warning",
                    target_id=",".join(str(uid) for uid in safe_senders), status="completed",
                    payload={"sender_ids": safe_senders},
                )
            except Exception:
                counts["failures"] += len(safe_senders)

        if tier == "exact_delete_mute" and len(safe_senders) >= 3 and can_restrict:
            from telegram import ChatPermissions
            until_date = datetime.now(timezone.utc) + timedelta(seconds=2592000)
            for sender_id in safe_senders:
                try:
                    await bot.restrict_chat_member(
                        chat_id=group_id,
                        user_id=sender_id,
                        permissions=ChatPermissions(can_send_messages=False),
                        until_date=until_date,
                    )
                    counts["muted"] += 1
                    await self._record_action(
                        job=job, cluster_id=plan["cluster_id"], action_type="mute",
                        target_id=str(sender_id), status="completed",
                        payload={"sender_id": sender_id, "seconds": 2592000},
                    )
                except Exception:
                    counts["failures"] += 1
        return counts

    async def _run_job(
        self, *, job_id: str, tenant_id: str, bot: Any, classifier: Optional[Classifier]
    ) -> None:
        lock_key: Tuple[str, int]
        try:
            job = await self.get_job(job_id=job_id, tenant_id=tenant_id)
            lock_key = (tenant_id, int(job["group_id"]))
            lock = self._group_locks.setdefault(lock_key, asyncio.Lock())
            async with lock:
                async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
                    claim = await db.execute(
                        """
                        UPDATE ad_scan_jobs
                        SET status='scanning', progress=5, started_at=?
                        WHERE job_id=? AND status='queued'
                        """,
                        (datetime.now().isoformat(), job_id),
                    )
                    await db.commit()
                if claim.rowcount != 1:
                    return
                job = await self.get_job(job_id=job_id, tenant_id=tenant_id)
                rows = await self._load_messages(
                    int(job["group_id"]), tenant_id, recent_only=str(job.get("source") or "") == "realtime"
                )
                known = await self._known_hashes(tenant_id)
                exact = self._exact_clusters(rows)
                candidates = self._candidate_exact_clusters(exact, known)
                families = self._similar_families(candidates)
                await self._set_job(job_id, total_messages=len(rows), candidate_clusters=len(families), progress=35)

                unknown: List[Dict[str, Any]] = []
                decisions: Dict[str, Dict[str, Any]] = {}
                for family in families:
                    members = family["members"]
                    known_family = any(item["known"] for item, _ in members)
                    deterministic = any(item["stored_ad"] or item["live_detection"].get("is_advertising") for item, _ in members)
                    if known_family or deterministic:
                        decisions[family["family_hash"]] = {
                            "is_ad": True, "decided": True,
                            "source": "fingerprint" if known_family else "deterministic",
                            "confidence": 1.0 if known_family else 0.9,
                            "reason": "命中已确认广告指纹" if known_family else "本地高置信广告规则",
                        }
                    elif sum(len(item["rows"]) for item, _ in members) >= 3:
                        unknown.append({
                            "family_hash": family["family_hash"],
                            "text": family["representative_text"][:1000],
                            "occurrences": sum(len(item["rows"]) for item, _ in members),
                        })

                if unknown and classifier:
                    try:
                        classified = await classifier(unknown)
                        for key, value in classified.items():
                            decisions[key] = {
                                "is_ad": bool(value.get("is_ad")),
                                "decided": True,
                                "source": "llm",
                                "confidence": float(value.get("confidence") or 0),
                                "reason": str(value.get("reason") or "")[:500],
                            }
                    except Exception as exc:
                        logger.warning("广告扫描 LLM 分类失败 job=%s: %s", job_id, exc)

                await self._set_job(job_id, status="classifying", progress=60)
                now_ts = int(time.time())
                totals = defaultdict(int)
                for family in families:
                    decision = decisions.get(family["family_hash"], {
                        "is_ad": False, "decided": False, "source": None, "confidence": 0, "reason": "待确认",
                    })
                    plan = await self._persist_cluster(job=job, family=family, classification=decision, now_ts=now_ts)
                    if plan["confirmed"]:
                        totals["confirmed"] += 1
                        if not plan["recent"]:
                            totals["fingerprint_only"] += sum(len(item["rows"]) for item, _ in family["members"])
                    result = await self._execute_cluster(job=job, plan=plan, bot=bot)
                    for key, value in result.items():
                        totals[key] += value

                if bool(job.get("activate_policy")) and not bool(job.get("dry_run")):
                    async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
                        await db.execute(
                            """
                            INSERT INTO ad_group_policies (tenant_id, group_id, is_enabled, activated_by, activated_at)
                            VALUES (?, ?, 1, ?, CURRENT_TIMESTAMP)
                            ON CONFLICT(tenant_id,group_id) DO UPDATE SET
                                is_enabled=1, activated_by=excluded.activated_by,
                                activated_at=CURRENT_TIMESTAMP, updated_at=CURRENT_TIMESTAMP
                            """,
                            (tenant_id, int(job["group_id"]), job.get("requested_by")),
                        )
                        await db.commit()

                summary = {
                    "total_messages": len(rows),
                    "exact_clusters": len(exact),
                    "candidate_clusters": len(families),
                    "confirmed_clusters": totals["confirmed"],
                    "unknown_clusters": len(families) - totals["confirmed"],
                    "fingerprint_only_count": totals["fingerprint_only"],
                    "planned_delete_count": totals["planned_delete"],
                    "deleted_count": totals["deleted"],
                    "warning_sender_count": totals["warnings"],
                    "muted_sender_count": totals["muted"],
                    "failure_count": totals["failures"],
                }
                await self._set_job(
                    job_id,
                    status="completed" if not totals["failures"] else "partial",
                    progress=100,
                    confirmed_clusters=totals["confirmed"],
                    fingerprint_only_count=totals["fingerprint_only"],
                    planned_delete_count=totals["planned_delete"],
                    deleted_count=totals["deleted"],
                    warning_sender_count=totals["warnings"],
                    muted_sender_count=totals["muted"],
                    failure_count=totals["failures"],
                    summary_json=json.dumps(summary, ensure_ascii=False),
                    finished_at=datetime.now().isoformat(),
                )
        except Exception as exc:
            logger.error("广告扫描任务失败 job=%s: %s", job_id, exc, exc_info=True)
            await self._set_job(
                job_id, status="failed", error=str(exc)[:1000], finished_at=datetime.now().isoformat()
            )

    async def policy_enabled(self, *, tenant_id: Optional[str], group_id: int) -> bool:
        await self.ensure_schema()
        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            row = await (await db.execute(
                "SELECT is_enabled FROM ad_group_policies WHERE tenant_id=? AND group_id=?",
                (_tenant(tenant_id), int(group_id)),
            )).fetchone()
        return bool(row and row[0])

    async def schedule_realtime_scan(self, *, tenant_id: Optional[str], group_id: int, bot: Any) -> Optional[str]:
        if not bool(getattr(config, "AD_AUTO_ENFORCE_ENABLED", False)):
            return None
        if not await self.policy_enabled(tenant_id=tenant_id, group_id=group_id):
            return None
        effective_tenant = _tenant(tenant_id)
        bucket = int(time.time()) // 30
        job_id = "rt_" + stable_hash(f"{effective_tenant}:{int(group_id)}:{bucket}")
        async with aiosqlite.connect(config.DB_NAME, timeout=20) as db:
            cur = await db.execute(
                """
                INSERT OR IGNORE INTO ad_scan_jobs (
                    job_id, tenant_id, group_id, requested_by, status,
                    dry_run, activate_policy, source
                ) VALUES (?, ?, ?, 'realtime', 'queued', 0, 0, 'realtime')
                """,
                (job_id, effective_tenant, int(group_id)),
            )
            await db.commit()
        if cur.rowcount == 1:
            self._tasks[job_id] = asyncio.create_task(
                self._run_job(
                    job_id=job_id, tenant_id=effective_tenant, bot=bot,
                    classifier=classify_families_with_llm,
                )
            )
        return job_id


async def classify_families_with_llm(items: List[Dict[str, Any]]) -> Dict[str, Dict[str, Any]]:
    """Strict cluster-level classifier shared by Agent scans and realtime policy."""
    from backend.app.services.ai_service import get_ai_service

    output: Dict[str, Dict[str, Any]] = {}
    for offset in range(0, len(items), 20):
        batch = items[offset:offset + 20]
        prompt = (
            "你是Telegram群广告分类器。只判断下列聚类代表文本是否为广告、推广、灰黑产引流或重复刷屏。"
            "必须只返回JSON数组，不要Markdown。每项格式："
            '{"family_hash":"原值","is_ad":true,"confidence":0.0,"reason":"简短理由"}。'
            "不得生成删除、禁言或message_id。\n输入："
            + json.dumps(batch, ensure_ascii=False)
        )
        text, _ = await get_ai_service().chat_messages(
            messages=[
                {"role": "system", "content": "仅输出合法JSON数组。"},
                {"role": "user", "content": prompt[:16000]},
            ],
            model="gemini-2.5-flash-lite",
        )
        raw = str(text or "").strip()
        raw = re.sub(r"^```(?:json)?\s*|\s*```$", "", raw, flags=re.I)
        start, end = raw.find("["), raw.rfind("]")
        if start < 0 or end < start:
            raise ValueError("LLM 未返回JSON数组")
        parsed = json.loads(raw[start:end + 1])
        if not isinstance(parsed, list):
            raise ValueError("LLM 分类结果不是数组")
        allowed = {str(item.get("family_hash")) for item in batch}
        for row in parsed:
            if not isinstance(row, dict):
                continue
            family_hash = str(row.get("family_hash") or "")
            if family_hash not in allowed:
                continue
            confidence = max(0.0, min(float(row.get("confidence") or 0), 1.0))
            output[family_hash] = {
                "is_ad": bool(row.get("is_ad")) and confidence >= 0.65,
                "confidence": confidence,
                "reason": str(row.get("reason") or "")[:500],
            }
    return output


_SERVICE = AdGovernanceService()


def get_ad_governance() -> AdGovernanceService:
    return _SERVICE
