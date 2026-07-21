# -*- coding: utf-8 -*-
"""Telegram group/channel advertisement clustering pipeline.

The Python bot remains the only Telegram update consumer. This module gives
n8n and the Agent a shared read/action surface over messages already captured
by the bot, with optional PostgreSQL + pgvector sidecar storage.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import logging
import re
import time
from dataclasses import dataclass
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

import aiosqlite
import httpx

import config
import database
from services.ai_agent.ad_detection import detect_advertising_signals

logger = logging.getLogger(__name__)


_URL_RE = re.compile(r"https?://\S+|t\.me/\S+|@\w{3,}", re.I)
_SPACE_RE = re.compile(r"\s+")


def normalize_text(text: str) -> str:
    """Normalize text for exact duplicate grouping without hiding intent."""
    value = str(text or "").strip().lower()
    value = _URL_RE.sub("[link]", value)
    value = _SPACE_RE.sub(" ", value)
    return value.strip()


def text_md5(text: str) -> str:
    return hashlib.md5(normalize_text(text).encode("utf-8")).hexdigest()


def _now_ts() -> int:
    return int(time.time())


def _sqlite_row_to_message(row: Dict[str, Any]) -> Dict[str, Any]:
    content = str(row.get("content") or "")
    return {
        "sqlite_msg_id": row.get("msg_id"),
        "tenant_id": row.get("tenant_id") or "default",
        "chat_id": int(row.get("group_id")),
        "group_id": int(row.get("group_id")),
        "chat_title": row.get("group_title") or "",
        "tg_msg_id": row.get("tg_msg_id"),
        "sender_id": row.get("user_id"),
        "sender_name": row.get("username") or "",
        "sender_type": row.get("sender_type") or "user",
        "text": content,
        "normalized_text": normalize_text(content),
        "text_md5": text_md5(content),
        "msg_ts": int(row.get("message_ts") or 0) or None,
        "is_deleted": False,
        "is_checked": bool(row.get("ad_checked_at")),
        "ad_cluster_id": row.get("ad_cluster_id"),
    }


@dataclass
class PipelineStatus:
    enabled: bool
    pg_configured: bool
    pg_available: bool
    mode: str
    message: str


class AdPipeline:
    def __init__(self) -> None:
        self._schema_ready = False
        self._pg_available: Optional[bool] = None

    def status(self) -> PipelineStatus:
        pg_configured = bool(getattr(config, "AD_PIPELINE_PG_DSN", ""))
        return PipelineStatus(
            enabled=bool(getattr(config, "AD_PIPELINE_ENABLED", False)),
            pg_configured=pg_configured,
            pg_available=bool(self._pg_available) if self._pg_available is not None else False,
            mode="postgres_pgvector" if pg_configured else "sqlite_fallback",
            message=(
                "PostgreSQL/pgvector sidecar configured"
                if pg_configured
                else "AD_PIPELINE_PG_DSN 未配置，使用 SQLite 已落库消息做聚合查询"
            ),
        )

    def _connect_pg(self):
        dsn = getattr(config, "AD_PIPELINE_PG_DSN", "")
        if not dsn:
            raise RuntimeError("AD_PIPELINE_PG_DSN is not configured")
        try:
            import psycopg
        except Exception as exc:
            raise RuntimeError("psycopg is not installed; install requirements.txt first") from exc
        return psycopg.connect(dsn)

    async def ensure_pg_schema(self) -> bool:
        if not getattr(config, "AD_PIPELINE_PG_DSN", ""):
            return False
        if self._schema_ready:
            return True

        def _sync() -> bool:
            dim = int(getattr(config, "AD_EMBEDDING_DIM", 768))
            with self._connect_pg() as conn:
                with conn.cursor() as cur:
                    cur.execute("CREATE EXTENSION IF NOT EXISTS vector")
                    cur.execute(
                        f"""
                        CREATE TABLE IF NOT EXISTS tg_ad_clusters (
                            cluster_id BIGSERIAL PRIMARY KEY,
                            tenant_id TEXT NOT NULL DEFAULT 'default',
                            representative_text TEXT,
                            representative_hash TEXT,
                            embedding vector({dim}),
                            occurrence_count INTEGER NOT NULL DEFAULT 0,
                            sender_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
                            message_refs JSONB NOT NULL DEFAULT '[]'::jsonb,
                            first_seen TIMESTAMPTZ DEFAULT NOW(),
                            last_seen TIMESTAMPTZ DEFAULT NOW(),
                            llm_status TEXT NOT NULL DEFAULT 'pending',
                            llm_reason TEXT DEFAULT NULL,
                            action_status TEXT NOT NULL DEFAULT 'none',
                            created_at TIMESTAMPTZ DEFAULT NOW(),
                            updated_at TIMESTAMPTZ DEFAULT NOW()
                        )
                        """
                    )
                    cur.execute(
                        f"""
                        CREATE TABLE IF NOT EXISTS tg_ad_messages (
                            id BIGSERIAL PRIMARY KEY,
                            tenant_id TEXT NOT NULL DEFAULT 'default',
                            chat_id BIGINT NOT NULL,
                            tg_msg_id BIGINT NOT NULL,
                            sender_id BIGINT,
                            sender_name TEXT,
                            sender_type TEXT DEFAULT 'user',
                            text TEXT,
                            normalized_text TEXT,
                            text_md5 TEXT,
                            embedding vector({dim}),
                            msg_ts BIGINT,
                            sqlite_msg_id BIGINT,
                            cluster_id BIGINT REFERENCES tg_ad_clusters(cluster_id),
                            is_checked BOOLEAN NOT NULL DEFAULT FALSE,
                            is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
                            created_at TIMESTAMPTZ DEFAULT NOW(),
                            updated_at TIMESTAMPTZ DEFAULT NOW(),
                            UNIQUE(tenant_id, chat_id, tg_msg_id)
                        )
                        """
                    )
                    cur.execute(
                        """
                        CREATE TABLE IF NOT EXISTS tg_ad_actions (
                            action_id BIGSERIAL PRIMARY KEY,
                            tenant_id TEXT NOT NULL DEFAULT 'default',
                            cluster_id BIGINT,
                            action_type TEXT NOT NULL,
                            dry_run BOOLEAN NOT NULL DEFAULT TRUE,
                            status TEXT NOT NULL DEFAULT 'pending',
                            message_refs JSONB NOT NULL DEFAULT '[]'::jsonb,
                            errors JSONB NOT NULL DEFAULT '[]'::jsonb,
                            created_at TIMESTAMPTZ DEFAULT NOW(),
                            executed_at TIMESTAMPTZ DEFAULT NULL
                        )
                        """
                    )
                    cur.execute("CREATE INDEX IF NOT EXISTS idx_tg_ad_msg_hash ON tg_ad_messages(text_md5)")
                    cur.execute("CREATE INDEX IF NOT EXISTS idx_tg_ad_msg_tenant_ts ON tg_ad_messages(tenant_id, msg_ts DESC)")
                    cur.execute("CREATE INDEX IF NOT EXISTS idx_tg_ad_cluster_status ON tg_ad_clusters(tenant_id, llm_status, updated_at DESC)")
                    cur.execute(
                        "CREATE INDEX IF NOT EXISTS idx_tg_ad_msg_embedding ON tg_ad_messages USING hnsw (embedding vector_cosine_ops)"
                    )
                    conn.commit()
            return True

        try:
            ok = await asyncio.to_thread(_sync)
            self._schema_ready = ok
            self._pg_available = ok
            return ok
        except Exception as exc:
            self._pg_available = False
            logger.warning("广告管线 pgvector schema 初始化失败，将使用 SQLite fallback: %s", exc)
            return False

    async def ollama_embedding(self, text: str) -> Optional[List[float]]:
        if not text.strip():
            return None
        base_url = getattr(config, "OLLAMA_BASE_URL", "http://localhost:11434").rstrip("/")
        model = getattr(config, "OLLAMA_EMBED_MODEL", "nomic-embed-text")
        try:
            async with httpx.AsyncClient(timeout=20.0) as client:
                resp = await client.post(
                    f"{base_url}/api/embeddings",
                    json={"model": model, "prompt": text[:4000]},
                )
                resp.raise_for_status()
                data = resp.json()
                emb = data.get("embedding")
                return emb if isinstance(emb, list) else None
        except Exception as exc:
            logger.debug("Ollama embedding failed: %s", exc)
            return None

    def _vector_literal(self, embedding: Optional[Sequence[float]]) -> Optional[str]:
        if not embedding:
            return None
        return "[" + ",".join(f"{float(x):.8f}" for x in embedding) + "]"

    async def upsert_event(self, event: Dict[str, Any], *, compute_embedding: bool = False) -> Dict[str, Any]:
        message = {
            "sqlite_msg_id": event.get("sqlite_msg_id") or event.get("msg_id"),
            "tenant_id": event.get("tenant_id") or "default",
            "chat_id": int(event.get("chat_id") or event.get("group_id")),
            "tg_msg_id": int(event.get("tg_msg_id")),
            "sender_id": event.get("sender_id") or event.get("sender_user_id"),
            "sender_name": event.get("sender_name") or event.get("sender_username") or "",
            "sender_type": event.get("sender_type") or ("channel" if event.get("event") == "channel_message" else "user"),
            "text": str(event.get("text") or event.get("content") or ""),
            "msg_ts": event.get("msg_ts") or event.get("message_ts"),
        }
        message["normalized_text"] = normalize_text(message["text"])
        message["text_md5"] = text_md5(message["text"])

        pg_ok = await self.ensure_pg_schema()
        if not pg_ok:
            return {
                "success": True,
                "mode": "sqlite_fallback",
                "data": {"message": message},
                "message": "事件已接收；PostgreSQL 未启用，原始消息以主 SQLite 为准",
            }

        embedding = await self.ollama_embedding(message["text"]) if compute_embedding else None
        vector = self._vector_literal(embedding)

        def _sync() -> Dict[str, Any]:
            with self._connect_pg() as conn:
                with conn.cursor() as cur:
                    cur.execute(
                        """
                        INSERT INTO tg_ad_messages (
                            tenant_id, chat_id, tg_msg_id, sender_id, sender_name, sender_type,
                            text, normalized_text, text_md5, embedding, msg_ts, sqlite_msg_id, updated_at
                        )
                        VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s::vector,%s,%s,NOW())
                        ON CONFLICT (tenant_id, chat_id, tg_msg_id) DO UPDATE SET
                            sender_id = EXCLUDED.sender_id,
                            sender_name = EXCLUDED.sender_name,
                            sender_type = EXCLUDED.sender_type,
                            text = EXCLUDED.text,
                            normalized_text = EXCLUDED.normalized_text,
                            text_md5 = EXCLUDED.text_md5,
                            embedding = COALESCE(EXCLUDED.embedding, tg_ad_messages.embedding),
                            msg_ts = EXCLUDED.msg_ts,
                            sqlite_msg_id = EXCLUDED.sqlite_msg_id,
                            updated_at = NOW()
                        RETURNING id
                        """,
                        (
                            message["tenant_id"], message["chat_id"], message["tg_msg_id"],
                            message.get("sender_id"), message.get("sender_name"), message.get("sender_type"),
                            message["text"], message["normalized_text"], message["text_md5"],
                            vector, message.get("msg_ts"), message.get("sqlite_msg_id"),
                        ),
                    )
                    row = cur.fetchone()
                    conn.commit()
                    return {"pg_id": row[0] if row else None}

        data = await asyncio.to_thread(_sync)
        return {"success": True, "mode": "postgres_pgvector", "data": data, "message": "广告管线消息事件已同步"}

    async def _fetch_sqlite_messages(
        self,
        *,
        tenant_id: Optional[str],
        group_id: Optional[int],
        window_minutes: int,
        limit: int,
        unchecked_only: bool = False,
    ) -> List[Dict[str, Any]]:
        cutoff = _now_ts() - max(1, int(window_minutes or 60)) * 60
        where = [
            "COALESCE(gm.content, '') != ''",
            "COALESCE(gm.sender_type, 'user') IN ('user', 'channel', 'bot')",
            "COALESCE(gm.message_ts, CAST(strftime('%s', gm.timestamp) AS INTEGER)) >= ?",
        ]
        params: List[Any] = [cutoff]
        if group_id is not None:
            where.append("gm.group_id = ?")
            params.append(int(group_id))
        if tenant_id:
            where.append("gm.tenant_id = ?")
            params.append(tenant_id)
        if unchecked_only:
            where.append("(gm.ad_checked_at IS NULL OR gm.ad_checked_at = '')")
        params.append(max(1, min(int(limit or 500), 2000)))

        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            cols_cur = await db.execute("PRAGMA table_info(group_messages)")
            gm_cols = {row[1] for row in await cols_cur.fetchall()}
            if "ad_checked_at" not in gm_cols:
                unchecked_only = False
                where = [part for part in where if "ad_checked_at" not in part]
            if "is_deleted" in gm_cols:
                where.append("COALESCE(gm.is_deleted, 0) = 0")
            join_title = "LEFT JOIN tracked_groups tg ON tg.group_id = gm.group_id"
            cur = await db.execute(
                f"""
                SELECT gm.msg_id, gm.group_id, gm.tg_msg_id, gm.user_id, gm.username,
                       gm.sender_type, gm.content, gm.message_ts, gm.timestamp,
                       gm.tenant_id,
                       {'gm.ad_cluster_id' if 'ad_cluster_id' in gm_cols else 'NULL AS ad_cluster_id'},
                       {'gm.ad_checked_at' if 'ad_checked_at' in gm_cols else 'NULL AS ad_checked_at'},
                       tg.group_title AS group_title
                FROM group_messages gm
                {join_title}
                WHERE {' AND '.join(where)}
                ORDER BY COALESCE(gm.message_ts, CAST(strftime('%s', gm.timestamp) AS INTEGER)) DESC, gm.msg_id DESC
                LIMIT ?
                """,
                tuple(params),
            )
            return [_sqlite_row_to_message(dict(row)) for row in await cur.fetchall()]

    def _cluster_sqlite_messages(self, rows: List[Dict[str, Any]], limit: int) -> List[Dict[str, Any]]:
        by_hash: Dict[str, Dict[str, Any]] = {}
        for row in rows:
            h = row["text_md5"]
            cluster = by_hash.setdefault(
                h,
                {
                    "cluster_id": f"sqlite:{h}",
                    "representative_hash": h,
                    "representative_text": row["text"],
                    "occurrences": 0,
                    "sender_ids": [],
                    "message_ids": [],
                    "message_refs": [],
                    "chat_ids": [],
                    "llm_status": "pending",
                    "action_status": "none",
                    "source": "sqlite.group_messages",
                },
            )
            cluster["occurrences"] += 1
            if row.get("sender_id") not in cluster["sender_ids"]:
                cluster["sender_ids"].append(row.get("sender_id"))
            if row.get("tg_msg_id") not in cluster["message_ids"]:
                cluster["message_ids"].append(row.get("tg_msg_id"))
            if row.get("chat_id") not in cluster["chat_ids"]:
                cluster["chat_ids"].append(row.get("chat_id"))
            cluster["message_refs"].append({
                "msg_id": row.get("sqlite_msg_id"),
                "chat_id": row.get("chat_id"),
                "group_id": row.get("group_id"),
                "tg_msg_id": row.get("tg_msg_id"),
                "sender_id": row.get("sender_id"),
                "sender_name": row.get("sender_name"),
                "text": row.get("text"),
                "msg_ts": row.get("msg_ts"),
            })

        min_dup = max(1, int(getattr(config, "AD_MIN_DUPLICATE_COUNT", 2)))
        clusters: List[Dict[str, Any]] = []
        for cluster in by_hash.values():
            flags = detect_advertising_signals(cluster["representative_text"], "group_message")
            cluster["live_detection"] = flags
            score = int(flags.get("ad_score", 0) or 0)
            if cluster["occurrences"] >= min_dup or bool(flags.get("is_advertising")) or score >= 40:
                cluster["risk_score"] = max(score, 55 if cluster["occurrences"] >= min_dup else 0)
                clusters.append(cluster)

        clusters.sort(key=lambda c: (int(c.get("risk_score") or 0), int(c.get("occurrences") or 0)), reverse=True)
        return clusters[: max(1, min(int(limit or 20), 100))]

    async def query_clusters(
        self,
        *,
        tenant_id: Optional[str] = None,
        group_id: Optional[int] = None,
        status: str = "pending",
        limit: int = 20,
        window_minutes: int = 60,
    ) -> Dict[str, Any]:
        pg_ok = await self.ensure_pg_schema()
        if pg_ok:
            return await self._query_pg_clusters(tenant_id=tenant_id, status=status, limit=limit)

        rows = await self._fetch_sqlite_messages(
            tenant_id=tenant_id,
            group_id=group_id,
            window_minutes=window_minutes,
            limit=max(200, int(limit or 20) * 20),
        )
        clusters = self._cluster_sqlite_messages(rows, limit)
        return {
            "success": True,
            "source": "sqlite.group_messages",
            "mode": "sqlite_fallback",
            "data_complete": bool(rows),
            "analysis_allowed": bool(rows),
            "evidence_count": len(rows),
            "data": {"clusters": clusters, "raw_message_count": len(rows)},
            "count": len(clusters),
            "message": f"广告聚合查询完成：{len(clusters)} 个候选 cluster，来源 SQLite 已落库消息",
            "limitation": None if rows else "当前没有 bot 已收到并落库的群/频道消息，不能判断没有广告。",
        }

    async def _query_pg_clusters(self, *, tenant_id: Optional[str], status: str, limit: int) -> Dict[str, Any]:
        def _sync() -> Dict[str, Any]:
            where = []
            params: List[Any] = []
            if tenant_id:
                where.append("tenant_id = %s")
                params.append(tenant_id)
            if status and status != "all":
                where.append("llm_status = %s")
                params.append(status)
            params.append(max(1, min(int(limit or 20), 100)))
            where_sql = "WHERE " + " AND ".join(where) if where else ""
            with self._connect_pg() as conn:
                with conn.cursor() as cur:
                    cur.execute(
                        f"""
                        SELECT cluster_id, tenant_id, representative_text, representative_hash,
                               occurrence_count, sender_ids, message_refs, first_seen, last_seen,
                               llm_status, llm_reason, action_status
                        FROM tg_ad_clusters
                        {where_sql}
                        ORDER BY last_seen DESC, occurrence_count DESC
                        LIMIT %s
                        """,
                        tuple(params),
                    )
                    rows = cur.fetchall()
                    cols = [d.name for d in cur.description]
                    clusters = [dict(zip(cols, row)) for row in rows]
                    if clusters or (status and status not in {"pending", "all"}):
                        return {"clusters": clusters}

                    dyn_where = ["COALESCE(text, '') != ''", "is_deleted = FALSE"]
                    dyn_params: List[Any] = []
                    if tenant_id:
                        dyn_where.append("tenant_id = %s")
                        dyn_params.append(tenant_id)
                    dyn_params.append(max(1, min(int(limit or 20), 100)))
                    cur.execute(
                        f"""
                        SELECT
                            ('pg-md5:' || text_md5) AS cluster_id,
                            MIN(tenant_id) AS tenant_id,
                            MIN(text) AS representative_text,
                            text_md5 AS representative_hash,
                            COUNT(*)::int AS occurrence_count,
                            jsonb_agg(DISTINCT sender_id) FILTER (WHERE sender_id IS NOT NULL) AS sender_ids,
                            jsonb_agg(jsonb_build_object(
                                'pg_id', id,
                                'chat_id', chat_id,
                                'group_id', chat_id,
                                'tg_msg_id', tg_msg_id,
                                'sender_id', sender_id,
                                'sender_name', sender_name,
                                'text', text,
                                'msg_ts', msg_ts
                            ) ORDER BY msg_ts DESC NULLS LAST) AS message_refs,
                            to_timestamp(MIN(msg_ts)) AS first_seen,
                            to_timestamp(MAX(msg_ts)) AS last_seen,
                            'pending' AS llm_status,
                            NULL AS llm_reason,
                            'none' AS action_status
                        FROM tg_ad_messages
                        WHERE {' AND '.join(dyn_where)}
                        GROUP BY text_md5
                        HAVING COUNT(*) >= %s
                        ORDER BY COUNT(*) DESC, MAX(msg_ts) DESC NULLS LAST
                        LIMIT %s
                        """,
                        tuple(dyn_params[:-1] + [max(1, int(getattr(config, "AD_MIN_DUPLICATE_COUNT", 2))), dyn_params[-1]]),
                    )
                    dyn_rows = cur.fetchall()
                    dyn_cols = [d.name for d in cur.description]
                    return {"clusters": [dict(zip(dyn_cols, row)) for row in dyn_rows]}

        data = await asyncio.to_thread(_sync)
        clusters = data.get("clusters") or []
        return {
            "success": True,
            "source": "postgres.tg_ad_clusters",
            "mode": "postgres_pgvector",
            "data_complete": bool(clusters),
            "analysis_allowed": bool(clusters),
            "evidence_count": len(clusters),
            "data": data,
            "count": len(clusters),
            "message": f"广告聚合查询完成：{len(clusters)} 个 cluster",
        }

    async def backfill_from_sqlite(
        self,
        *,
        tenant_id: Optional[str] = None,
        days: int = 3,
        limit: int = 1000,
    ) -> Dict[str, Any]:
        rows = await self._fetch_sqlite_messages(
            tenant_id=tenant_id,
            group_id=None,
            window_minutes=max(1, int(days or 3)) * 24 * 60,
            limit=limit,
        )
        synced = 0
        for row in rows:
            event = {
                "sqlite_msg_id": row.get("sqlite_msg_id"),
                "tenant_id": row.get("tenant_id"),
                "chat_id": row.get("chat_id"),
                "tg_msg_id": row.get("tg_msg_id"),
                "sender_id": row.get("sender_id"),
                "sender_name": row.get("sender_name"),
                "sender_type": row.get("sender_type"),
                "content": row.get("text"),
                "message_ts": row.get("msg_ts"),
            }
            if not event.get("tg_msg_id"):
                continue
            result = await self.upsert_event(event, compute_embedding=False)
            if result.get("success"):
                synced += 1
        return {
            "success": True,
            "data": {"scanned": len(rows), "synced": synced},
            "message": f"backfill 完成：扫描 {len(rows)} 条，同步 {synced} 条",
        }

    async def mark_cluster_llm_result(
        self,
        *,
        cluster_id: str,
        is_ad: bool,
        reason: str = "",
        message_refs: Optional[List[Dict[str, Any]]] = None,
    ) -> Dict[str, Any]:
        if str(cluster_id).startswith("sqlite:"):
            status = "ad" if is_ad else "not_ad"
            refs = message_refs or []
            async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                cur = await db.execute("PRAGMA table_info(group_messages)")
                cols = {row[1] for row in await cur.fetchall()}
                if {"ad_cluster_id", "ad_checked_at", "ad_findings", "is_advertising"}.issubset(cols):
                    for ref in refs:
                        try:
                            chat_id = int(ref.get("chat_id") or ref.get("group_id"))
                            tg_msg_id = int(ref.get("tg_msg_id"))
                        except Exception:
                            continue
                        await db.execute(
                            """
                            UPDATE group_messages
                            SET ad_cluster_id = ?, ad_checked_at = CURRENT_TIMESTAMP,
                                is_advertising = CASE WHEN ? THEN 1 ELSE is_advertising END,
                                ad_findings = ?
                            WHERE group_id = ? AND tg_msg_id = ?
                            """,
                            (
                                cluster_id,
                                1 if is_ad else 0,
                                json.dumps([reason or status], ensure_ascii=False),
                                chat_id,
                                tg_msg_id,
                            ),
                        )
                await db.commit()
            return {"success": True, "message": f"SQLite fallback cluster {cluster_id} 标记为 {status}"}

        pg_ok = await self.ensure_pg_schema()
        if not pg_ok:
            return {"success": False, "message": "PostgreSQL 未启用，无法持久化该 cluster 判定"}

        def _sync() -> None:
            with self._connect_pg() as conn:
                with conn.cursor() as cur:
                    cur.execute(
                        """
                        UPDATE tg_ad_clusters
                        SET llm_status = %s, llm_reason = %s, updated_at = NOW()
                        WHERE cluster_id = %s
                        """,
                        ("ad" if is_ad else "not_ad", reason, int(cluster_id)),
                    )
                    conn.commit()

        await asyncio.to_thread(_sync)
        return {"success": True, "message": f"cluster {cluster_id} 已标记"}


_PIPELINE = AdPipeline()


def get_ad_pipeline() -> AdPipeline:
    return _PIPELINE


async def delete_cluster_messages(
    *,
    cluster: Dict[str, Any],
    bot: Any,
    dry_run: bool = True,
    ban_senders: bool = False,
) -> Dict[str, Any]:
    refs = cluster.get("message_refs") or []
    if isinstance(refs, str):
        try:
            refs = json.loads(refs)
        except Exception:
            refs = []
    if not isinstance(refs, list):
        refs = []

    delete_enabled = bool(getattr(config, "AD_DELETE_ENABLED", False))
    kick_enabled = bool(getattr(config, "AD_KICK_ENABLED", False))
    effective_dry_run = bool(dry_run or not delete_enabled)
    errors: List[Dict[str, Any]] = []
    deleted: List[Dict[str, Any]] = []
    skipped_admins: List[Dict[str, Any]] = []
    banned: List[int] = []

    if not bot:
        return {"success": False, "message": "bot 未初始化，无法执行删除", "action_taken": False}

    for ref in refs:
        try:
            chat_id = int(ref.get("chat_id") or ref.get("group_id"))
            tg_msg_id = int(ref.get("tg_msg_id"))
            sender_id = int(ref.get("sender_id") or 0)
        except Exception:
            errors.append({"ref": ref, "error": "缺少 chat_id/tg_msg_id"})
            continue

        if sender_id:
            try:
                member = await bot.get_chat_member(chat_id, sender_id)
                if getattr(member, "status", "") in ("administrator", "creator"):
                    skipped_admins.append(ref)
                    continue
            except Exception as exc:
                logger.debug("广告删除前检查成员身份失败: %s", exc)

        if not effective_dry_run:
            try:
                await bot.delete_message(chat_id=chat_id, message_id=tg_msg_id)
                await database.delete_group_message(chat_id, tg_msg_id)
            except Exception as exc:
                errors.append({"ref": ref, "error": str(exc)[:300]})
                continue
        deleted.append(ref)

        if ban_senders and sender_id and kick_enabled and not effective_dry_run:
            try:
                await bot.ban_chat_member(chat_id=chat_id, user_id=sender_id)
                if sender_id not in banned:
                    banned.append(sender_id)
            except Exception as exc:
                errors.append({"ref": ref, "ban_error": str(exc)[:300]})

    return {
        "success": not bool(errors),
        "action_taken": bool(deleted) and not effective_dry_run,
        "dry_run": effective_dry_run,
        "data": {
            "deleted_count": len(deleted),
            "skipped_admin_count": len(skipped_admins),
            "banned_sender_ids": banned,
            "errors": errors,
            "delete_enabled": delete_enabled,
            "kick_enabled": kick_enabled,
        },
        "message": (
            f"{'模拟' if effective_dry_run else '实际'}处理 cluster："
            f"目标 {len(refs)} 条，处理 {len(deleted)} 条，跳过管理员 {len(skipped_admins)} 条，错误 {len(errors)} 条"
        ),
    }
