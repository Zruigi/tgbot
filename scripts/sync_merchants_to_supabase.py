#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Sync current SQLite merchant records to Supabase/PostgREST.

Required env:
  SUPABASE_URL
  SUPABASE_SERVICE_ROLE_KEY

The script only writes the compact merchant storage fields needed by the SaaS
merchant list: user id, username, registration time, paid option, and expiry.
"""
from __future__ import annotations

import json
import os
import sqlite3
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


PROJECT_DIR = Path(__file__).resolve().parents[1]


def load_dotenv(path: Path) -> None:
    if not path.exists():
        return
    for raw in path.read_text(encoding="utf-8", errors="ignore").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip("'\"")
        os.environ.setdefault(key, value)


def db_path() -> str:
    load_dotenv(PROJECT_DIR / ".env")
    candidates = [
        os.getenv("DB_NAME"),
        os.getenv("DATABASE_PATH"),
        str(PROJECT_DIR / "bot.db"),
        str(PROJECT_DIR / "database.db"),
        str(PROJECT_DIR / "tgbot.db"),
    ]
    for candidate in candidates:
        if candidate and Path(candidate).exists():
            return candidate
    raise SystemExit("未找到 SQLite 数据库。请设置 DB_NAME 或确认数据库文件存在。")


def fetch_merchants(sqlite_path: str) -> list[dict[str, Any]]:
    conn = sqlite3.connect(sqlite_path)
    conn.row_factory = sqlite3.Row
    try:
        rows = conn.execute(
            """
            SELECT
                m.id AS legacy_merchant_id,
                m.tenant_id,
                m.owner_tg_user_id,
                COALESCE(
                    NULLIF(pu.username, ''),
                    NULLIF(ai.from_username, ''),
                    NULLIF(gm.username, ''),
                    NULLIF(m.merchant_name, ''),
                    'User ' || m.owner_tg_user_id
                ) AS owner_username,
                m.merchant_name,
                m.bot_id,
                m.bot_username,
                m.status,
                COALESCE(ms.plan_code, m.plan_type, 'free') AS plan_type,
                m.subscription_status,
                COALESCE(ms.end_at, m.subscription_expires_at) AS subscription_expires_at,
                COALESCE(m.onboarding_completed, 0) AS onboarding_completed,
                m.runtime_version,
                m.config_version,
                m.feature_version,
                m.created_at,
                m.updated_at
            FROM merchants m
            LEFT JOIN potential_users pu ON pu.user_id = m.owner_tg_user_id
            LEFT JOIN admin_inbox ai ON ai.from_userid = m.owner_tg_user_id
            LEFT JOIN (
                SELECT user_id, MAX(COALESCE(NULLIF(username, ''), NULLIF(first_name, ''))) AS username
                FROM group_members
                GROUP BY user_id
            ) gm ON gm.user_id = m.owner_tg_user_id
            LEFT JOIN (
                SELECT s.*
                FROM merchant_subscriptions s
                JOIN (
                    SELECT merchant_id, MAX(COALESCE(end_at, created_at)) AS max_at
                    FROM merchant_subscriptions
                    GROUP BY merchant_id
                ) latest ON latest.merchant_id = s.merchant_id
                    AND latest.max_at = COALESCE(s.end_at, s.created_at)
            ) ms ON ms.merchant_id = m.id
            ORDER BY m.id ASC
            """
        ).fetchall()
    finally:
        conn.close()

    payload: list[dict[str, Any]] = []
    for row in rows:
        item = dict(row)
        item["onboarding_completed"] = bool(item.get("onboarding_completed"))
        item["runtime_version"] = int(item.get("runtime_version") or 1)
        item["config_version"] = int(item.get("config_version") or 1)
        item["feature_version"] = int(item.get("feature_version") or 1)
        payload.append({k: v for k, v in item.items() if v is not None})
    return payload


def upsert_supabase(rows: list[dict[str, Any]]) -> Any:
    supabase_url = (os.getenv("SUPABASE_URL") or "").rstrip("/")
    service_key = os.getenv("SUPABASE_SERVICE_ROLE_KEY") or ""
    if not supabase_url or not service_key:
        raise SystemExit("缺少 SUPABASE_URL 或 SUPABASE_SERVICE_ROLE_KEY，无法同步。")
    if not rows:
        return []

    endpoint = f"{supabase_url}/rest/v1/merchants?{urllib.parse.urlencode({'on_conflict': 'legacy_merchant_id'})}"
    body = json.dumps(rows, ensure_ascii=False).encode("utf-8")
    request = urllib.request.Request(
        endpoint,
        data=body,
        method="POST",
        headers={
            "apikey": service_key,
            "Authorization": f"Bearer {service_key}",
            "Content-Type": "application/json",
            "Prefer": "resolution=merge-duplicates,return=representation",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read().decode("utf-8")
            return json.loads(raw) if raw else []
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="ignore")
        raise SystemExit(f"Supabase upsert failed: HTTP {exc.code} {detail}") from exc


def main() -> int:
    sqlite_path = db_path()
    rows = fetch_merchants(sqlite_path)
    result = upsert_supabase(rows)
    print(json.dumps({"status": "ok", "sqlite": sqlite_path, "synced": len(rows), "returned": len(result)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
