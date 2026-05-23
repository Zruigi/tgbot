# -*- coding: utf-8 -*-
"""
异步数据库操作模块
"""
import aiosqlite
import aiohttp
import logging
import re
import json
from datetime import datetime, timedelta
import calendar
from typing import Optional, List, Dict, Any, Union
from config import DB_NAME, ADMIN_ID

logger = logging.getLogger(__name__)


def _normalize_tenant_id(value: Optional[str]) -> str:
    """统一清洗 tenant_id 输入，避免空白/None 混用。"""
    return str(value or "").strip()


async def _resolve_scope_bot_id(
    db: aiosqlite.Connection,
    *,
    tenant_id: Optional[str] = None,
    merchant_id: Optional[int] = None,
) -> Optional[int]:
    """根据 tenant/merchant scope 解析对应商户 bot_id。"""
    normalized_tenant = _normalize_tenant_id(tenant_id)
    if merchant_id is not None:
        async with db.execute(
            "SELECT bot_id FROM merchants WHERE id = ? LIMIT 1",
            (int(merchant_id),),
        ) as cursor:
            row = await cursor.fetchone()
            if row and row[0]:
                return int(row[0])
    if normalized_tenant:
        async with db.execute(
            "SELECT bot_id FROM merchants WHERE tenant_id = ? LIMIT 1",
            (normalized_tenant,),
        ) as cursor:
            row = await cursor.fetchone()
            if row and row[0]:
                return int(row[0])
    return None


async def _resolve_tenant_id_from_group(
    db: aiosqlite.Connection,
    group_id: Optional[int],
) -> Optional[str]:
    """由 group_id 反查租户。"""
    if group_id is None:
        return None
    async with db.execute(
        "SELECT tenant_id FROM tracked_groups WHERE group_id = ? LIMIT 1",
        (int(group_id),),
    ) as cursor:
        row = await cursor.fetchone()
        if not row:
            return None
        tenant_id = _normalize_tenant_id(row[0])
        return tenant_id or None


async def _resolve_tenant_id_for_user(
    db: aiosqlite.Connection,
    user_id: Optional[int],
) -> Optional[str]:
    """通过用户在各业务表中的痕迹反查 tenant_id。"""
    if user_id is None:
        return None
    uid = int(user_id)

    for sql in (
        "SELECT tenant_id FROM admin_inbox WHERE from_userid = ? AND tenant_id IS NOT NULL AND tenant_id != '' LIMIT 1",
        "SELECT tenant_id FROM potential_users WHERE user_id = ? AND tenant_id IS NOT NULL AND tenant_id != '' LIMIT 1",
        "SELECT tenant_id FROM group_members WHERE user_id = ? AND tenant_id IS NOT NULL AND tenant_id != '' LIMIT 1",
        "SELECT tenant_id FROM messages WHERE (from_userid = ? OR to_userid = ?) AND tenant_id IS NOT NULL AND tenant_id != '' ORDER BY timestamp DESC LIMIT 1",
    ):
        params = (uid, uid) if sql.count("?") == 2 else (uid,)
        async with db.execute(sql, params) as cursor:
            row = await cursor.fetchone()
            if row:
                tenant_id = _normalize_tenant_id(row[0])
                if tenant_id:
                    return tenant_id

    async with db.execute(
        """
        SELECT tg.tenant_id
        FROM group_members gm
        JOIN tracked_groups tg ON tg.group_id = gm.group_id
        WHERE gm.user_id = ?
          AND tg.tenant_id IS NOT NULL
          AND tg.tenant_id != ''
        ORDER BY tg.updated_at DESC
        LIMIT 1
        """,
        (uid,),
    ) as cursor:
        row = await cursor.fetchone()
        if row:
            tenant_id = _normalize_tenant_id(row[0])
            if tenant_id:
                return tenant_id

    return None


async def _resolve_message_tenant_id(
    db: aiosqlite.Connection,
    *,
    from_userid: int,
    to_userid: int,
    tenant_id: Optional[str] = None,
) -> str:
    """为消息写入推导 tenant_id，优先显式值，兜底按 bot/user 痕迹推导。"""
    normalized_tenant = _normalize_tenant_id(tenant_id)
    if normalized_tenant:
        return normalized_tenant

    async with db.execute(
        "SELECT tenant_id FROM merchants WHERE bot_id IN (?, ?) AND tenant_id IS NOT NULL AND tenant_id != '' LIMIT 1",
        (int(from_userid), int(to_userid)),
    ) as cursor:
        row = await cursor.fetchone()
        if row:
            merchant_tenant = _normalize_tenant_id(row[0])
            if merchant_tenant:
                return merchant_tenant

    for candidate_user_id in (from_userid, to_userid):
        if int(candidate_user_id) == int(ADMIN_ID):
            continue
        tenant_from_user = await _resolve_tenant_id_for_user(db, int(candidate_user_id))
        if tenant_from_user:
            return tenant_from_user

    return "default"

async def init_db():
    """初始化数据库表结构并执行必要的迁移"""
    async with aiosqlite.connect(DB_NAME) as db:
        # 1. users 表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS users (
                user_id INTEGER PRIMARY KEY,
                email TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # 2. alerts 表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS alerts (
                alert_id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER,
                symbol TEXT NOT NULL,
                market_type TEXT NOT NULL,
                alert_type TEXT NOT NULL,
                target_value REAL,
                upper_line REAL,
                lower_line REAL,
                channel_state TEXT,
                notify_method TEXT NOT NULL,
                is_active BOOLEAN DEFAULT 1,
                FOREIGN KEY (user_id) REFERENCES users (user_id)
            )
        ''')

        # 3. address_monitors 表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS address_monitors (
                monitor_id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER,
                address TEXT NOT NULL,
                network TEXT NOT NULL,
                remark TEXT DEFAULT '',
                risk_level TEXT DEFAULT 'normal',
                risk_note TEXT DEFAULT NULL,
                last_tx_timestamp INTEGER DEFAULT 0,
                is_active BOOLEAN DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (user_id) REFERENCES users (user_id)
            )
        ''')

        # 4. messages 表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                trace_id TEXT UNIQUE,
                from_userid INTEGER NOT NULL,
                to_userid INTEGER NOT NULL,
                content TEXT,
                type TEXT DEFAULT 'text',
                file_id TEXT DEFAULT NULL,
                media_url TEXT DEFAULT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                is_read BOOLEAN DEFAULT 0,
                quoted_text TEXT DEFAULT NULL,
                quoted_msg_id INTEGER DEFAULT NULL,
                quoted_file_id TEXT DEFAULT NULL,
                quoted_type TEXT DEFAULT 'text',
                tg_msg_id INTEGER DEFAULT NULL,
                status TEXT DEFAULT 'sent',
                auto_translation TEXT DEFAULT NULL,
                is_deleted BOOLEAN DEFAULT 0
            )
        ''')

        # 4.1 emoji_messages 表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS emoji_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                msg_id INTEGER NOT NULL,
                emoji TEXT,
                FOREIGN KEY(msg_id) REFERENCES messages(msg_id) ON DELETE CASCADE
            )
        ''')

        # 5. admin_inbox 表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS admin_inbox (
                from_userid INTEGER PRIMARY KEY,
                from_username TEXT,
                summary TEXT,
                unread_count INTEGER DEFAULT 0,
                last_active DATETIME DEFAULT CURRENT_TIMESTAMP,
                avatar_file_id TEXT DEFAULT NULL
            )
        ''')

        # 检查并添加缺失的列 (2026 规格迁移)
        cursor = await db.execute("PRAGMA table_info(messages)")
        msg_columns = [row[1] for row in await cursor.fetchall()]
        if 'admin_userid' not in msg_columns:
            await db.execute("ALTER TABLE messages ADD COLUMN admin_userid TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 messages 添加 admin_userid 字段")
        if 'translated_text' not in msg_columns:
            await db.execute("ALTER TABLE messages ADD COLUMN translated_text TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 messages 添加 translated_text 字段")
        if 'original_text' not in msg_columns:
            await db.execute("ALTER TABLE messages ADD COLUMN original_text TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 messages 添加 original_text 字段")

        # 检查并添加缺失的列 (2026 规格迁移)
        cursor = await db.execute("PRAGMA table_info(admin_inbox)")
        columns = [row[1] for row in await cursor.fetchall()]
        if 'avatar_file_id' not in columns:
            await db.execute("ALTER TABLE admin_inbox ADD COLUMN avatar_file_id TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 admin_inbox 添加 avatar_file_id 字段")

        cursor = await db.execute("PRAGMA table_info(address_monitors)")
        monitor_columns = [row[1] for row in await cursor.fetchall()]
        if 'remark' not in monitor_columns:
            await db.execute("ALTER TABLE address_monitors ADD COLUMN remark TEXT DEFAULT ''")
            logger.info("migrated address_monitors.remark")
        if 'risk_level' not in monitor_columns:
            await db.execute("ALTER TABLE address_monitors ADD COLUMN risk_level TEXT DEFAULT 'normal'")
            logger.info("migrated address_monitors.risk_level")
        if 'risk_note' not in monitor_columns:
            await db.execute("ALTER TABLE address_monitors ADD COLUMN risk_note TEXT DEFAULT NULL")
            logger.info("migrated address_monitors.risk_note")
        if 'created_at' not in monitor_columns:
            await db.execute("ALTER TABLE address_monitors ADD COLUMN created_at DATETIME DEFAULT NULL")
            await db.execute("UPDATE address_monitors SET created_at = CURRENT_TIMESTAMP WHERE created_at IS NULL")
            logger.info("migrated address_monitors.created_at")

        cursor = await db.execute("PRAGMA table_info(monitor_transactions)")
        tx_columns = [row[1] for row in await cursor.fetchall()]
        if tx_columns:
            if 'income_reason' not in tx_columns:
                await db.execute("ALTER TABLE monitor_transactions ADD COLUMN income_reason TEXT DEFAULT NULL")
                logger.info("migrated monitor_transactions.income_reason")
            if 'expense_reason' not in tx_columns:
                await db.execute("ALTER TABLE monitor_transactions ADD COLUMN expense_reason TEXT DEFAULT NULL")
                logger.info("migrated monitor_transactions.expense_reason")
            if 'payer' not in tx_columns:
                await db.execute("ALTER TABLE monitor_transactions ADD COLUMN payer TEXT DEFAULT NULL")
                logger.info("migrated monitor_transactions.payer")
            if 'payee' not in tx_columns:
                await db.execute("ALTER TABLE monitor_transactions ADD COLUMN payee TEXT DEFAULT NULL")
                logger.info("migrated monitor_transactions.payee")

        # 6. admin_message_mapping 表
        await db.execute('''
            CREATE TABLE IF NOT EXISTS admin_message_mapping (
                admin_msg_id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                original_tg_msg_id INTEGER DEFAULT NULL
            )
        ''')

        # 7. admin_settings 表 (用于存储管理员配置)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS admin_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )
        ''')

        # 12. admin_users 表 (2026 规格：多客服系统预留)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS admin_users (
                admin_id INTEGER PRIMARY KEY,
                username TEXT,
                worker_no TEXT,
                role TEXT DEFAULT 'staff',
                is_online BOOLEAN DEFAULT 1
            )
        ''')

        # --- 自动迁移逻辑：检查并添加缺失的列 ---
        tables_to_check = {
            "messages": [
                ("media_url", "TEXT DEFAULT NULL"),
                ("quoted_text", "TEXT DEFAULT NULL"),
                ("quoted_msg_id", "INTEGER DEFAULT NULL"),
                ("quoted_file_id", "TEXT DEFAULT NULL"),
                ("quoted_type", "TEXT DEFAULT 'text'"),
                ("tg_msg_id", "INTEGER DEFAULT NULL"),
                ("status", "TEXT DEFAULT 'sent'"),
                ("auto_translation", "TEXT DEFAULT NULL"),
                ("is_deleted", "BOOLEAN DEFAULT 0"),
                ("translated_text", "TEXT DEFAULT NULL"),
                ("original_text", "TEXT DEFAULT NULL"),
                ("admin_userid", "TEXT DEFAULT NULL") # 2026 规格：新增字段
            ],
            "admin_message_mapping": [
                ("original_tg_msg_id", "INTEGER DEFAULT NULL")
            ]
        }

        for table_name, columns in tables_to_check.items():
            # 获取当前表的列信息
            cursor = await db.execute(f"PRAGMA table_info({table_name})")
            existing_columns = [row[1] for row in await cursor.fetchall()]
            
            for col_name, col_def in columns:
                if col_name not in existing_columns:
                    logger.info(f"迁移：向表 {table_name} 添加列 {col_name}")
                    try:
                        await db.execute(f"ALTER TABLE {table_name} ADD COLUMN {col_name} {col_def}")
                    except Exception as e:
                        logger.error(f"添加列 {col_name} 失败: {e}")

        # 初始化默认设置
        default_settings = {
            "translation_engine": "llm",
            "llm_model": "gpt-3.5-turbo",
            "llm_base_url": "https://api.openai.com/v1",
            "system_prompt": (
                "（以下为可选补充，会附加在系统固定规则之后；留空亦可）\n"
                "例如：优先保留用户语气；遇到合约/风控词务必译准确。"
            ),
            "global_auto_translate": "false",
            "llm_api_mode": "rented"
        }
        for key, val in default_settings.items():
            await db.execute("INSERT OR IGNORE INTO admin_settings (key, value) VALUES (?, ?)", (key, val))

        # 11. pending_admin_messages 表 (2026 规格：多轮对话防丢失消息队列)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS pending_admin_messages (
                msg_uuid TEXT PRIMARY KEY,
                admin_id INTEGER NOT NULL,
                content TEXT,
                msg_type TEXT DEFAULT 'text',
                file_id TEXT DEFAULT NULL,
                status TEXT DEFAULT 'pending',
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        await db.execute('''
            CREATE TABLE IF NOT EXISTS monitor_transactions (
                tx_id TEXT PRIMARY KEY,
                monitor_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                address TEXT NOT NULL,
                network TEXT NOT NULL,
                direction TEXT NOT NULL,
                amount REAL NOT NULL,
                token_symbol TEXT NOT NULL,
                tx_timestamp INTEGER NOT NULL,
                counterparty TEXT DEFAULT NULL,
                risk_level TEXT DEFAULT 'normal',
                risk_note TEXT DEFAULT NULL,
                income_reason TEXT DEFAULT NULL,
                expense_reason TEXT DEFAULT NULL,
                payer TEXT DEFAULT NULL,
                payee TEXT DEFAULT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')
        await db.execute('''
            CREATE TABLE IF NOT EXISTS monitor_counterparty_labels (
                label_id INTEGER PRIMARY KEY AUTOINCREMENT,
                admin_userid INTEGER NOT NULL,
                counterparty TEXT NOT NULL,
                network TEXT DEFAULT '',
                label TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(admin_userid, counterparty, network)
            )
        ''')
        await db.execute(
            "CREATE INDEX IF NOT EXISTS idx_monitor_counterparty_label ON monitor_counterparty_labels(admin_userid, counterparty, network)"
        )

        # 13. tracked_groups 表 (群组监听主表)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS tracked_groups (
                group_id INTEGER PRIMARY KEY,
                group_title TEXT,
                is_monitored BOOLEAN DEFAULT 0,
                custom_keywords TEXT DEFAULT NULL,
                first_seen DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                delete_on_revoke BOOLEAN DEFAULT 0,
                revoke_sync_chat_id INTEGER DEFAULT NULL,
                verify_enabled BOOLEAN DEFAULT 0,
                verify_type TEXT DEFAULT 'captcha',
                verify_timeout INTEGER DEFAULT 120,
                verify_channel_id TEXT DEFAULT NULL,
                verify_questions TEXT DEFAULT NULL
            )
        ''')

        # 13b. join_verify_sessions 表 (入群验证会话)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS join_verify_sessions (
                session_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                username TEXT,
                first_name TEXT,
                verify_type TEXT NOT NULL,
                answer TEXT,
                bot_msg_id INTEGER,
                status TEXT DEFAULT 'pending',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                expires_at DATETIME
            )
        ''')

        # 14. potential_users 表 (群组监听 - 潜在用户)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS potential_users (
                user_id INTEGER PRIMARY KEY,
                username TEXT,
                first_name TEXT,
                last_name TEXT,
                message_text TEXT,
                keywords TEXT,
                profile_tags TEXT,
                group_id INTEGER,
                group_title TEXT,
                captured_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # 15. broadcast_targets 表 (统一群组/频道广播目标)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS broadcast_targets (
                chat_id INTEGER PRIMARY KEY,
                chat_title TEXT,
                chat_type TEXT NOT NULL,
                chat_username TEXT DEFAULT NULL,
                can_broadcast BOOLEAN DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # 16. group_members 表 (群组成员追踪)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS group_members (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                username TEXT,
                first_name TEXT,
                last_name TEXT,
                group_id INTEGER NOT NULL,
                group_title TEXT,
                status TEXT DEFAULT 'active',
                joined_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                is_verified BOOLEAN DEFAULT 0,
                verified_at DATETIME DEFAULT NULL,
                avatar_file_id TEXT DEFAULT NULL
            )
        ''')
        await db.execute(
            """
            DELETE FROM group_members
            WHERE id NOT IN (
                SELECT MAX(id)
                FROM group_members
                GROUP BY group_id, user_id
            )
            """
        )
        await db.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_group_members_group_user ON group_members(group_id, user_id)")

        # 17. group_messages 表 (全量群组消息记录 - 用于历史回顾)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS group_messages (
                msg_id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                username TEXT,
                tg_msg_id INTEGER DEFAULT NULL,
                sender_type TEXT DEFAULT 'user',
                content TEXT,
                msg_type TEXT DEFAULT 'text',
                file_id TEXT DEFAULT NULL,
                quoted_tg_msg_id INTEGER DEFAULT NULL,
                quoted_text TEXT DEFAULT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                message_ts INTEGER DEFAULT (strftime('%s','now')),
                revoke_checked BOOLEAN DEFAULT 0,
                is_advertising BOOLEAN DEFAULT 0,
                ad_score INTEGER DEFAULT 0,
                ad_confidence TEXT DEFAULT 'low',
                ad_findings TEXT DEFAULT NULL
            )
        ''')
        
        # 检查并添加缺失的列 (group_messages 迁移)
        cursor = await db.execute("PRAGMA table_info(group_messages)")
        gm_columns = [row[1] for row in await cursor.fetchall()]
        if 'message_ts' not in gm_columns:
            await db.execute("ALTER TABLE group_messages ADD COLUMN message_ts INTEGER DEFAULT NULL")
            await db.execute("UPDATE group_messages SET message_ts = CAST(strftime('%s', timestamp) AS INTEGER)")
            logger.info("✅ 数据库迁移：已为 group_messages 添加 message_ts 字段并初始化数据")
        if 'revoke_checked' not in gm_columns:
            await db.execute("ALTER TABLE group_messages ADD COLUMN revoke_checked BOOLEAN DEFAULT 0")
            logger.info("✅ 数据库迁移：已为 group_messages 添加 revoke_checked 字段")
        if 'is_advertising' not in gm_columns:
            await db.execute("ALTER TABLE group_messages ADD COLUMN is_advertising BOOLEAN DEFAULT 0")
            logger.info("✅ 数据库迁移：已为 group_messages 添加 is_advertising 字段")
        if 'ad_score' not in gm_columns:
            await db.execute("ALTER TABLE group_messages ADD COLUMN ad_score INTEGER DEFAULT 0")
            logger.info("✅ 数据库迁移：已为 group_messages 添加 ad_score 字段")
        if 'ad_confidence' not in gm_columns:
            await db.execute("ALTER TABLE group_messages ADD COLUMN ad_confidence TEXT DEFAULT 'low'")
            logger.info("✅ 数据库迁移：已为 group_messages 添加 ad_confidence 字段")
        if 'ad_findings' not in gm_columns:
            await db.execute("ALTER TABLE group_messages ADD COLUMN ad_findings TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 group_messages 添加 ad_findings 字段")

        # 检查并添加缺失的列 (group_members 迁移 - 验证字段)
        cursor = await db.execute("PRAGMA table_info(group_members)")
        gm_cols = [row[1] for row in await cursor.fetchall()]
        if 'is_verified' not in gm_cols:
            await db.execute("ALTER TABLE group_members ADD COLUMN is_verified BOOLEAN DEFAULT 0")
            logger.info("✅ 数据库迁移：已为 group_members 添加 is_verified 字段")
        if 'verified_at' not in gm_cols:
            await db.execute("ALTER TABLE group_members ADD COLUMN verified_at DATETIME DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 group_members 添加 verified_at 字段")
        if 'avatar_file_id' not in gm_cols:
            await db.execute("ALTER TABLE group_members ADD COLUMN avatar_file_id TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 group_members 添加 avatar_file_id 字段")

        # 为 group_id 和 timestamp 创建索引以优化查询
        await db.execute("CREATE INDEX IF NOT EXISTS idx_group_msg_query ON group_messages(group_id, timestamp DESC)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_group_msg_ts ON group_messages(group_id, message_ts DESC)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_group_msg_ad ON group_messages(group_id, is_advertising, ad_score DESC)")
        await db.execute(
            """
            DELETE FROM group_messages
            WHERE tg_msg_id IS NOT NULL
              AND msg_id NOT IN (
                  SELECT MIN(msg_id)
                  FROM group_messages
                  WHERE tg_msg_id IS NOT NULL
                  GROUP BY group_id, tg_msg_id
              )
            """
        )
        await db.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_group_msg_unique_tg ON group_messages(group_id, tg_msg_id) WHERE tg_msg_id IS NOT NULL")

        # 18. group_address_hits 表（群内地址自动识别去重）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS group_address_hits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL,
                user_id INTEGER,
                address TEXT NOT NULL,
                network TEXT NOT NULL,
                source_tg_msg_id INTEGER DEFAULT NULL,
                query_result TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(group_id, address)
            )
        ''')

        # 19. push_tasks 表（定时与周期推送任务）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS push_tasks (
                task_id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_name TEXT NOT NULL,
                content TEXT,
                image_file_id TEXT DEFAULT NULL,
                do_translate BOOLEAN DEFAULT 0,
                target_chat_ids TEXT NOT NULL,
                start_at TEXT NOT NULL,
                interval_hours INTEGER NOT NULL,
                status TEXT DEFAULT 'running',
                next_run_at TEXT,
                last_run_at TEXT,
                last_result TEXT,
                last_summary TEXT,
                retry_count INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # 2026 规格：数据库迁移 (处理旧版 tracked_groups 表)
        try:
            # 1. 检查 tracked_groups 列
            # 0. 检查 group_messages 列
            async with db.execute("PRAGMA table_info(group_messages)") as cursor:
                gm_columns = [row[1] for row in await cursor.fetchall()]
                if 'tg_msg_id' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 tg_msg_id 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN tg_msg_id INTEGER DEFAULT NULL")
                if 'sender_type' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 sender_type 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN sender_type TEXT DEFAULT 'user'")
                if 'quoted_tg_msg_id' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 quoted_tg_msg_id 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN quoted_tg_msg_id INTEGER DEFAULT NULL")
                if 'quoted_text' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 quoted_text 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN quoted_text TEXT DEFAULT NULL")
                if 'file_id' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 file_id 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN file_id TEXT DEFAULT NULL")
                if 'message_ts' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 message_ts 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN message_ts INTEGER DEFAULT (strftime('%s','now'))")
                if 'revoke_checked' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 revoke_checked 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN revoke_checked BOOLEAN DEFAULT 0")
                if 'is_advertising' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 is_advertising 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN is_advertising BOOLEAN DEFAULT 0")
                if 'ad_score' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 ad_score 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN ad_score INTEGER DEFAULT 0")
                if 'ad_confidence' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 ad_confidence 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN ad_confidence TEXT DEFAULT 'low'")
                if 'ad_findings' not in gm_columns:
                    logger.info("迁移 group_messages: 新增 ad_findings 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN ad_findings TEXT DEFAULT NULL")
                await db.execute("UPDATE group_messages SET message_ts = CAST(strftime('%s', timestamp) AS INTEGER) WHERE message_ts IS NULL OR message_ts = 0")

            # 1. 检查 tracked_groups 列
            async with db.execute("PRAGMA table_info(tracked_groups)") as cursor:
                columns = [row[1] for row in await cursor.fetchall()]
                
                if 'title' in columns and 'group_title' not in columns:
                    logger.info("迁移 tracked_groups: 重命名 title -> group_title")
                    await db.execute("ALTER TABLE tracked_groups RENAME COLUMN title TO group_title")
                
                if 'listen_enabled' in columns and 'is_monitored' not in columns:
                    logger.info("迁移 tracked_groups: 重命名 listen_enabled -> is_monitored")
                    await db.execute("ALTER TABLE tracked_groups RENAME COLUMN listen_enabled TO is_monitored")
                
                if 'last_seen' in columns and 'updated_at' not in columns:
                    logger.info("迁移 tracked_groups: 重命名 last_seen -> updated_at")
                    await db.execute("ALTER TABLE tracked_groups RENAME COLUMN last_seen TO updated_at")
                
                if 'first_seen' not in columns:
                    logger.info("迁移 tracked_groups: 新增 first_seen 列")
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN first_seen DATETIME DEFAULT '2026-01-01 00:00:00'")
                
                if 'custom_keywords' not in columns:
                    logger.info("迁移 tracked_groups: 新增 custom_keywords 列")
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN custom_keywords TEXT DEFAULT NULL")

                if 'delete_on_revoke' not in columns:
                    logger.info("迁移 tracked_groups: 新增 delete_on_revoke 列")
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN delete_on_revoke BOOLEAN DEFAULT 0")
                if 'revoke_sync_chat_id' not in columns:
                    logger.info("迁移 tracked_groups: 新增 revoke_sync_chat_id 列")
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN revoke_sync_chat_id INTEGER DEFAULT NULL")

                if 'verify_enabled' not in columns:
                    logger.info("迁移 tracked_groups: 新增 verify_enabled 列")
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN verify_enabled BOOLEAN DEFAULT 0")
                if 'verify_type' not in columns:
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN verify_type TEXT DEFAULT 'captcha'")
                if 'verify_timeout' not in columns:
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN verify_timeout INTEGER DEFAULT 120")
                if 'verify_channel_id' not in columns:
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN verify_channel_id TEXT DEFAULT NULL")
                if 'verify_questions' not in columns:
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN verify_questions TEXT DEFAULT NULL")
                if 'verify_msg_template' not in columns:
                    logger.info("迁移 tracked_groups: 新增 verify_msg_template 列")
                    await db.execute("ALTER TABLE tracked_groups ADD COLUMN verify_msg_template TEXT DEFAULT NULL")

            # 1b-extra. 检查 group_members 的验证字段
            async with db.execute("PRAGMA table_info(group_members)") as cursor:
                gmbr_cols = [row[1] for row in await cursor.fetchall()]
                if 'is_verified' not in gmbr_cols:
                    logger.info("迁移 group_members: 新增 is_verified 列")
                    await db.execute("ALTER TABLE group_members ADD COLUMN is_verified BOOLEAN DEFAULT 0")
                if 'verified_at' not in gmbr_cols:
                    logger.info("迁移 group_members: 新增 verified_at 列")
                    await db.execute("ALTER TABLE group_members ADD COLUMN verified_at DATETIME DEFAULT NULL")
                if 'pending_verify_msg_id' not in gmbr_cols:
                    logger.info("迁移 group_members: 新增 pending_verify_msg_id 列")
                    await db.execute("ALTER TABLE group_members ADD COLUMN pending_verify_msg_id INTEGER DEFAULT NULL")

            # 1b. 检查 group_messages 的 avatar_file_id 列
            async with db.execute("PRAGMA table_info(group_messages)") as cursor:
                gm_cols = [row[1] for row in await cursor.fetchall()]
                if 'avatar_file_id' not in gm_cols:
                    logger.info("迁移 group_messages: 新增 avatar_file_id 列")
                    await db.execute("ALTER TABLE group_messages ADD COLUMN avatar_file_id TEXT DEFAULT NULL")

            # 2. 检查 potential_users 列
            async with db.execute("PRAGMA table_info(potential_users)") as cursor:
                p_columns = [row[1] for row in await cursor.fetchall()]
                if 'profile_tags' not in p_columns:
                    logger.info("迁移 potential_users: 新增 profile_tags 列")
                    await db.execute("ALTER TABLE potential_users ADD COLUMN profile_tags TEXT DEFAULT NULL")
        except Exception as e:
            logger.error("数据库迁移异常: %s", e)

        # 20. chain_sync_state 表 (用于区块扫描架构)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS chain_sync_state (
                network TEXT PRIMARY KEY,
                last_scanned_block INTEGER DEFAULT 0,
                last_scanned_ts INTEGER DEFAULT 0
            )
        ''')

        # 21. tenant_configs 表 (v5.7 新增：加密存储 API 密钥与敏感配置)
        await db.execute('''
            CREATE TABLE IF NOT EXISTS tenant_configs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tenant_id TEXT DEFAULT 'default',
                config_key TEXT NOT NULL,
                config_value TEXT, -- 存储 field_crypto.encrypt() 后的加密串
                remark TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tenant_id, config_key)
            )
        ''')

        # 22. ai_usage_logs 表（AI 用量统计）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS ai_usage_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel TEXT DEFAULT 'web_ai',
                user_id INTEGER,
                sandbox_id TEXT,
                tenant_id TEXT DEFAULT 'default',
                merchant_id INTEGER DEFAULT NULL,
                feature_key TEXT DEFAULT 'general',
                request_trace_id TEXT DEFAULT NULL,
                prompt_template_version TEXT DEFAULT NULL,
                result_status TEXT DEFAULT 'success',
                endpoint TEXT DEFAULT NULL,
                model TEXT,
                prompt_tokens INTEGER DEFAULT 0,
                completion_tokens INTEGER DEFAULT 0,
                total_tokens INTEGER DEFAULT 0,
                estimated_cost_usd REAL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        cursor = await db.execute("PRAGMA table_info(ai_usage_logs)")
        ai_usage_columns = [row[1] for row in await cursor.fetchall()]
        if 'tenant_id' not in ai_usage_columns:
            await db.execute("ALTER TABLE ai_usage_logs ADD COLUMN tenant_id TEXT DEFAULT 'default'")
            logger.info("✅ 数据库迁移：已为 ai_usage_logs 添加 tenant_id 字段")
        if 'merchant_id' not in ai_usage_columns:
            await db.execute("ALTER TABLE ai_usage_logs ADD COLUMN merchant_id INTEGER DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 ai_usage_logs 添加 merchant_id 字段")
        if 'feature_key' not in ai_usage_columns:
            await db.execute("ALTER TABLE ai_usage_logs ADD COLUMN feature_key TEXT DEFAULT 'general'")
            logger.info("✅ 数据库迁移：已为 ai_usage_logs 添加 feature_key 字段")
        if 'request_trace_id' not in ai_usage_columns:
            await db.execute("ALTER TABLE ai_usage_logs ADD COLUMN request_trace_id TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 ai_usage_logs 添加 request_trace_id 字段")
        if 'prompt_template_version' not in ai_usage_columns:
            await db.execute("ALTER TABLE ai_usage_logs ADD COLUMN prompt_template_version TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 ai_usage_logs 添加 prompt_template_version 字段")
        if 'result_status' not in ai_usage_columns:
            await db.execute("ALTER TABLE ai_usage_logs ADD COLUMN result_status TEXT DEFAULT 'success'")
            logger.info("✅ 数据库迁移：已为 ai_usage_logs 添加 result_status 字段")
        if 'endpoint' not in ai_usage_columns:
            await db.execute("ALTER TABLE ai_usage_logs ADD COLUMN endpoint TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 ai_usage_logs 添加 endpoint 字段")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_ai_usage_logs_tenant_created ON ai_usage_logs(tenant_id, created_at)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_ai_usage_logs_merchant_created ON ai_usage_logs(merchant_id, created_at)")

        # ── 多租户数据隔离迁移：为核心业务表添加 tenant_id ──────────
        _tenant_migration_tables = [
            'broadcast_targets',
            'push_tasks',
            'tracked_groups',
            'potential_users',
            'address_monitors',
            'monitor_transactions',
            'group_messages',
            'group_members',
            'group_address_hits',
        ]
        for _tbl in _tenant_migration_tables:
            cursor = await db.execute(f"PRAGMA table_info({_tbl})")
            _cols = [row[1] for row in await cursor.fetchall()]
            if 'tenant_id' not in _cols:
                await db.execute(f"ALTER TABLE {_tbl} ADD COLUMN tenant_id TEXT DEFAULT 'default'")
                logger.info(f"✅ 数据库迁移：已为 {_tbl} 添加 tenant_id 字段")
            # 创建索引（幂等）
            await db.execute(f"CREATE INDEX IF NOT EXISTS idx_{_tbl}_tenant ON {_tbl}(tenant_id)")
        # push_tasks 额外添加 created_by 字段（如果不存在），用于任务归属
        cursor = await db.execute("PRAGMA table_info(push_tasks)")
        _pt_cols = [row[1] for row in await cursor.fetchall()]
        if 'created_by' not in _pt_cols:
            await db.execute("ALTER TABLE push_tasks ADD COLUMN created_by INTEGER DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 push_tasks 添加 created_by 字段")

        # 23. oneapi_keys 表（OneAPI sk 映射）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS oneapi_keys (
                sk_token TEXT PRIMARY KEY,
                provider TEXT DEFAULT 'vertex_gemini',
                model TEXT,
                sandbox_id TEXT DEFAULT 'default',
                is_active BOOLEAN DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # 24. merchants 表（平台域：商户主档）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS merchants (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tenant_id TEXT NOT NULL UNIQUE,
                owner_tg_user_id INTEGER NOT NULL,
                merchant_name TEXT NOT NULL,
                bot_id INTEGER DEFAULT NULL,
                bot_username TEXT DEFAULT NULL,
                status TEXT DEFAULT 'active_free',
                plan_type TEXT DEFAULT 'free',
                subscription_status TEXT DEFAULT 'free',
                subscription_expires_at DATETIME DEFAULT NULL,
                runtime_version INTEGER DEFAULT 1,
                config_version INTEGER DEFAULT 1,
                feature_version INTEGER DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')
        cursor = await db.execute("PRAGMA table_info(merchants)")
        merchant_columns = [row[1] for row in await cursor.fetchall()]
        if 'onboarding_completed' not in merchant_columns:
            await db.execute("ALTER TABLE merchants ADD COLUMN onboarding_completed INTEGER DEFAULT 0")
            logger.info("✅ 数据库迁移：已为 merchants 添加 onboarding_completed 字段")
        if 'onboarding_approved_at' not in merchant_columns:
            await db.execute("ALTER TABLE merchants ADD COLUMN onboarding_approved_at DATETIME DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 merchants 添加 onboarding_approved_at 字段")
        if 'onboarding_notified_at' not in merchant_columns:
            await db.execute("ALTER TABLE merchants ADD COLUMN onboarding_notified_at DATETIME DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 merchants 添加 onboarding_notified_at 字段")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_merchants_owner ON merchants(owner_tg_user_id)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_merchants_status ON merchants(status, plan_type)")

        # 25. plans 表（平台域：套餐定义）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS plans (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_code TEXT NOT NULL UNIQUE,
                plan_name TEXT NOT NULL,
                price_usdt REAL DEFAULT 0,
                duration_days INTEGER DEFAULT 30,
                features_json TEXT DEFAULT '{}',
                status TEXT DEFAULT 'active',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')
        await db.execute(
            """
            INSERT OR IGNORE INTO plans (plan_code, plan_name, price_usdt, duration_days, features_json, status)
            VALUES
                ('free', 'Free', 0, 30, '{"base_bot": true, "basic_ai": true, "basic_translate": true, "dual_bot": true, "broadcast": true, "group_chat": true, "subscription_check": true, "shared_ai_translate": false}', 'active'),
                ('pro', 'Pro', 99, 30, '{"base_bot": true, "basic_ai": true, "basic_translate": true, "dual_bot": true, "broadcast": true, "group_chat": true, "subscription_check": true, "shared_ai_translate": true}', 'active')
            """
        )

        # 26. merchant_subscriptions 表（平台域：商户订阅）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS merchant_subscriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                merchant_id INTEGER NOT NULL,
                plan_code TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                start_at DATETIME DEFAULT NULL,
                end_at DATETIME DEFAULT NULL,
                payment_order_id INTEGER DEFAULT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (merchant_id) REFERENCES merchants(id)
            )
        ''')
        await db.execute("CREATE INDEX IF NOT EXISTS idx_merchant_subscriptions_merchant ON merchant_subscriptions(merchant_id, status, end_at)")

        # 27. payment_orders 表（平台域：支付订单）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS payment_orders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                merchant_id INTEGER NOT NULL,
                plan_code TEXT NOT NULL,
                base_amount_usdt REAL NOT NULL DEFAULT 0,
                amount_offset_usdt REAL NOT NULL DEFAULT 0,
                amount_usdt REAL NOT NULL,
                chain_type TEXT DEFAULT 'TRON',
                payment_address TEXT,
                payment_qr_image_path TEXT DEFAULT NULL,
                tx_hash TEXT DEFAULT NULL,
                status TEXT DEFAULT 'pending',
                expire_at DATETIME DEFAULT NULL,
                confirmed_at DATETIME DEFAULT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (merchant_id) REFERENCES merchants(id)
            )
        ''')
        cursor = await db.execute("PRAGMA table_info(payment_orders)")
        payment_order_columns = [row[1] for row in await cursor.fetchall()]
        if 'base_amount_usdt' not in payment_order_columns:
            await db.execute("ALTER TABLE payment_orders ADD COLUMN base_amount_usdt REAL NOT NULL DEFAULT 0")
            await db.execute("UPDATE payment_orders SET base_amount_usdt = amount_usdt WHERE base_amount_usdt = 0")
            logger.info("✅ 数据库迁移：已为 payment_orders 添加 base_amount_usdt 字段")
        if 'amount_offset_usdt' not in payment_order_columns:
            await db.execute("ALTER TABLE payment_orders ADD COLUMN amount_offset_usdt REAL NOT NULL DEFAULT 0")
            logger.info("✅ 数据库迁移：已为 payment_orders 添加 amount_offset_usdt 字段")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_payment_orders_merchant ON payment_orders(merchant_id, status, created_at)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_payment_orders_tx_hash ON payment_orders(tx_hash)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_payment_orders_match_lookup ON payment_orders(chain_type, payment_address, status, amount_usdt, expire_at)")


        # 28. platform_payment_configs 表（平台域：收款配置）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS platform_payment_configs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chain_type TEXT NOT NULL UNIQUE,
                wallet_address TEXT NOT NULL,
                wallet_qr_image_path TEXT DEFAULT NULL,
                instruction_text TEXT DEFAULT '',
                is_active BOOLEAN DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')

        # 29. merchant_features 表（租户功能快照）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS merchant_features (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                merchant_id INTEGER NOT NULL,
                tenant_id TEXT NOT NULL,
                feature_key TEXT NOT NULL,
                enabled BOOLEAN DEFAULT 0,
                source TEXT DEFAULT 'plan',
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tenant_id, feature_key),
                FOREIGN KEY (merchant_id) REFERENCES merchants(id)
            )
        ''')
        await db.execute("CREATE INDEX IF NOT EXISTS idx_merchant_features_merchant ON merchant_features(merchant_id, tenant_id)")

        # 30. merchant_ai_quota 表（租户 AI 配额）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS merchant_ai_quota (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                merchant_id INTEGER NOT NULL,
                tenant_id TEXT NOT NULL UNIQUE,
                daily_quota INTEGER DEFAULT 0,
                monthly_quota INTEGER DEFAULT 0,
                used_daily INTEGER DEFAULT 0,
                used_monthly INTEGER DEFAULT 0,
                last_reset_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (merchant_id) REFERENCES merchants(id)
            )
        ''')

        # 31. tenant_audit_logs 表（租户审计日志）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS tenant_audit_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tenant_id TEXT NOT NULL,
                merchant_id INTEGER DEFAULT NULL,
                operator_role TEXT DEFAULT 'system',
                operator_id TEXT DEFAULT NULL,
                action TEXT NOT NULL,
                resource_type TEXT DEFAULT NULL,
                resource_id TEXT DEFAULT NULL,
                payload_json TEXT DEFAULT NULL,
                trace_id TEXT DEFAULT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (merchant_id) REFERENCES merchants(id)
            )
        ''')
        await db.execute("CREATE INDEX IF NOT EXISTS idx_tenant_audit_logs_tenant_created ON tenant_audit_logs(tenant_id, created_at)")

        # 33. web_login_otps 表（Web OTP 持久化存储，仅保存哈希）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS web_login_otps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                otp_hash TEXT NOT NULL UNIQUE,
                role TEXT DEFAULT 'platform_admin',
                tenant_id TEXT DEFAULT '',
                merchant_id INTEGER DEFAULT NULL,
                issued_for TEXT DEFAULT NULL,
                expires_at DATETIME NOT NULL,
                used_at DATETIME DEFAULT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')
        await db.execute("CREATE INDEX IF NOT EXISTS idx_web_login_otps_expires ON web_login_otps(expires_at, used_at)")

        # 检查 web_login_otps 表是否有 session_token 字段
        cursor = await db.execute("PRAGMA table_info(web_login_otps)")
        web_otp_columns = [row[1] for row in await cursor.fetchall()]
        if 'session_token' not in web_otp_columns:
            await db.execute("ALTER TABLE web_login_otps ADD COLUMN session_token TEXT DEFAULT NULL")
            logger.info("✅ 数据库迁移：已为 web_login_otps 添加 session_token 字段")

        # 34. web_entry_grants 表（Mini App 入口 grant）
        await db.execute('''
            CREATE TABLE IF NOT EXISTS web_entry_grants (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                grant_hash TEXT NOT NULL UNIQUE,
                issued_for TEXT NOT NULL,
                role_hint TEXT DEFAULT 'platform_admin',
                tenant_id TEXT DEFAULT '',
                merchant_id INTEGER DEFAULT NULL,
                entry_intent TEXT DEFAULT 'menu',
                ua_fingerprint TEXT DEFAULT '',
                proof_salt TEXT NOT NULL,
                expires_at DATETIME NOT NULL,
                last_rotated_at DATETIME DEFAULT NULL,
                revoked_at DATETIME DEFAULT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')
        await db.execute("CREATE INDEX IF NOT EXISTS idx_web_entry_grants_lookup ON web_entry_grants(grant_hash, revoked_at, expires_at)")

        # ── 多租户隔离迁移：为间接隔离表添加 tenant_id ──────────
        _indirect_tenant_tables = {
            'admin_inbox': 'idx_admin_inbox_tenant',
            'messages': 'idx_messages_tenant',
            'merchant_subscriptions': 'idx_merchant_subscriptions_tenant',
            'payment_orders': 'idx_payment_orders_tenant',
        }
        for _tbl, _idx_name in _indirect_tenant_tables.items():
            cursor = await db.execute(f"PRAGMA table_info({_tbl})")
            _cols = [row[1] for row in await cursor.fetchall()]
            if 'tenant_id' not in _cols:
                await db.execute(f"ALTER TABLE {_tbl} ADD COLUMN tenant_id TEXT DEFAULT 'default'")
                logger.info(f"✅ 数据库迁移：已为 {_tbl} 添加 tenant_id 字段")
            await db.execute(f"CREATE INDEX IF NOT EXISTS {_idx_name} ON {_tbl}(tenant_id)")

        # 验证已有的 tenant_id 索引（确保13张核心表的索引存在）
        _verify_tenant_index_tables = [
            'tracked_groups', 'group_messages', 'group_members', 'potential_users',
            'broadcast_targets', 'push_tasks', 'address_monitors', 'monitor_transactions',
            'group_address_hits', 'ai_usage_logs', 'merchant_features',
        ]
        for _tbl in _verify_tenant_index_tables:
            await db.execute(f"CREATE INDEX IF NOT EXISTS idx_{_tbl}_tenant ON {_tbl}(tenant_id)")

        await db.commit()



async def save_web_login_otp(
    otp_hash: str,
    *,
    role: str = 'platform_admin',
    tenant_id: str = '',
    merchant_id: Optional[int] = None,
    issued_for: Optional[str] = None,
    expires_at: str,
    session_token: Optional[str] = None,
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT OR REPLACE INTO web_login_otps
            (otp_hash, role, tenant_id, merchant_id, issued_for, expires_at, used_at, session_token, created_at)
            VALUES (?, ?, ?, ?, ?, ?, NULL, ?, CURRENT_TIMESTAMP)
            """,
            (otp_hash, role, tenant_id, merchant_id, issued_for, expires_at, session_token),
        )
        await db.commit()
        return True


async def get_web_login_otp(otp_hash: str) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM web_login_otps WHERE otp_hash = ? AND used_at IS NULL",
            (otp_hash,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def consume_web_login_otp(otp_hash: str) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE web_login_otps SET used_at = CURRENT_TIMESTAMP WHERE otp_hash = ? AND used_at IS NULL",
            (otp_hash,),
        )
        await db.commit()
        return cur.rowcount > 0


async def cleanup_expired_web_login_otps() -> int:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "DELETE FROM web_login_otps WHERE expires_at < CURRENT_TIMESTAMP OR used_at IS NOT NULL"
        )
        await db.commit()
        return int(cur.rowcount or 0)


async def save_web_entry_grant(
    grant_hash: str,
    *,
    issued_for: str,
    role_hint: str = 'platform_admin',
    tenant_id: str = '',
    merchant_id: Optional[int] = None,
    entry_intent: str = 'menu',
    ua_fingerprint: str = '',
    proof_salt: str,
    expires_at: str,
) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT OR REPLACE INTO web_entry_grants
            (
                grant_hash, issued_for, role_hint, tenant_id, merchant_id,
                entry_intent, ua_fingerprint, proof_salt, expires_at,
                revoked_at, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, CURRENT_TIMESTAMP)
            """,
            (
                grant_hash,
                str(issued_for or ''),
                str(role_hint or 'platform_admin'),
                str(tenant_id or ''),
                merchant_id,
                str(entry_intent or 'menu'),
                str(ua_fingerprint or ''),
                str(proof_salt or ''),
                expires_at,
            ),
        )
        await db.commit()
        return True


async def get_web_entry_grant(grant_hash: str) -> Optional[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT * FROM web_entry_grants
            WHERE grant_hash = ?
              AND revoked_at IS NULL
            LIMIT 1
            """,
            (grant_hash,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def bind_web_entry_grant_proof(grant_hash: str, *, ua_fingerprint: str) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE web_entry_grants
            SET ua_fingerprint = CASE
                WHEN COALESCE(ua_fingerprint, '') = '' THEN ?
                ELSE ua_fingerprint
            END
            WHERE grant_hash = ?
              AND revoked_at IS NULL
            """,
            (str(ua_fingerprint or ''), grant_hash),
        )
        await db.commit()
        return cur.rowcount > 0


async def touch_web_entry_grant_rotation(grant_hash: str) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE web_entry_grants
            SET last_rotated_at = CURRENT_TIMESTAMP
            WHERE grant_hash = ?
              AND revoked_at IS NULL
            """,
            (grant_hash,),
        )
        await db.commit()
        return cur.rowcount > 0


async def revoke_web_entry_grant(grant_hash: str) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE web_entry_grants
            SET revoked_at = CURRENT_TIMESTAMP
            WHERE grant_hash = ?
              AND revoked_at IS NULL
            """,
            (grant_hash,),
        )
        await db.commit()
        return cur.rowcount > 0


async def cleanup_expired_web_entry_grants() -> int:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            DELETE FROM web_entry_grants
            WHERE revoked_at IS NOT NULL
               OR expires_at < CURRENT_TIMESTAMP
            """
        )
        await db.commit()
        return int(cur.rowcount or 0)
async def get_admin_setting(key: str, default: Optional[str] = None):
    """获取管理员设置"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT value FROM admin_settings WHERE key = ?", (key,)) as cursor:
            row = await cursor.fetchone()
            return row[0] if row else default


async def save_admin_setting(key: str, value: str):
    """更新管理员设置"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("INSERT OR REPLACE INTO admin_settings (key, value) VALUES (?, ?)", (key, str(value)))
        await db.commit()


async def list_ai_custom_skills(enabled_only: bool = False) -> List[Dict[str, Any]]:
    """列出 AI 助手自定义技能。"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM ai_custom_skills"
        if enabled_only:
            sql += " WHERE is_enabled = 1"
        sql += " ORDER BY updated_at DESC, id DESC"
        async with db.execute(sql) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def upsert_ai_custom_skill(
    name: str,
    description: str = "",
    prompt_template: str = "",
    source: str = "custom",
    is_enabled: bool = True,
) -> bool:
    """新增或更新 AI 自定义技能。"""
    if not (name or "").strip():
        return False
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO ai_custom_skills (name, description, prompt_template, source, is_enabled, updated_at)
            VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(name) DO UPDATE SET
                description=excluded.description,
                prompt_template=excluded.prompt_template,
                source=excluded.source,
                is_enabled=excluded.is_enabled,
                updated_at=CURRENT_TIMESTAMP
            """,
            (name.strip(), (description or "").strip(), prompt_template or "", (source or "custom").strip(), 1 if is_enabled else 0),
        )
        await db.commit()
        return True


async def set_ai_custom_skill_enabled(name: str, is_enabled: bool) -> bool:
    """启用/停用技能。"""
    if not (name or "").strip():
        return False
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE ai_custom_skills SET is_enabled = ?, updated_at = CURRENT_TIMESTAMP WHERE name = ?",
            (1 if is_enabled else 0, name.strip()),
        )
        await db.commit()
        return cur.rowcount > 0


async def save_ai_usage_log(
    channel: str,
    user_id: Optional[int],
    sandbox_id: str,
    model: str,
    prompt_tokens: int = 0,
    completion_tokens: int = 0,
    total_tokens: int = 0,
    estimated_cost_usd: float = 0.0,
):
    """记录 AI 调用用量"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO ai_usage_logs
            (channel, user_id, sandbox_id, model, prompt_tokens, completion_tokens, total_tokens, estimated_cost_usd)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                channel,
                user_id,
                sandbox_id,
                model,
                int(prompt_tokens or 0),
                int(completion_tokens or 0),
                int(total_tokens or 0),
                float(estimated_cost_usd or 0.0),
            ),
        )
        await db.commit()


async def get_ai_usage_summary(days: int = 7) -> Dict[str, Any]:
    """汇总 AI 用量（按最近 N 天）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT
                COUNT(*) AS calls,
                COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
                COALESCE(SUM(completion_tokens), 0) AS completion_tokens,
                COALESCE(SUM(total_tokens), 0) AS total_tokens,
                COALESCE(SUM(estimated_cost_usd), 0) AS estimated_cost_usd
            FROM ai_usage_logs
            WHERE created_at >= datetime('now', ?)
            """,
            (f"-{int(days)} day",),
        ) as cursor:
            summary_row = await cursor.fetchone()

        async with db.execute(
            """
            SELECT model, COUNT(*) AS calls, COALESCE(SUM(total_tokens), 0) AS total_tokens
            FROM ai_usage_logs
            WHERE created_at >= datetime('now', ?)
            GROUP BY model
            ORDER BY total_tokens DESC
            LIMIT 10
            """,
            (f"-{int(days)} day",),
        ) as cursor:
            model_rows = await cursor.fetchall()

        async with db.execute(
            """
            SELECT sandbox_id, COUNT(*) AS calls, COALESCE(SUM(total_tokens), 0) AS total_tokens
            FROM ai_usage_logs
            WHERE created_at >= datetime('now', ?)
            GROUP BY sandbox_id
            ORDER BY total_tokens DESC
            LIMIT 20
            """,
            (f"-{int(days)} day",),
        ) as cursor:
            sandbox_rows = await cursor.fetchall()

    return {
        "calls": int((summary_row or {}).get("calls", 0) if summary_row else 0),
        "prompt_tokens": int((summary_row or {}).get("prompt_tokens", 0) if summary_row else 0),
        "completion_tokens": int((summary_row or {}).get("completion_tokens", 0) if summary_row else 0),
        "total_tokens": int((summary_row or {}).get("total_tokens", 0) if summary_row else 0),
        "estimated_cost_usd": float((summary_row or {}).get("estimated_cost_usd", 0.0) if summary_row else 0.0),
        "top_models": [dict(r) for r in (model_rows or [])],
        "top_sandboxes": [dict(r) for r in (sandbox_rows or [])],
    }


async def save_oneapi_key(sk_token: str, provider: str, model: str, sandbox_id: str = 'default'):
    """保存 OneAPI sk 映射"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO oneapi_keys (sk_token, provider, model, sandbox_id, is_active, updated_at)
            VALUES (?, ?, ?, ?, 1, CURRENT_TIMESTAMP)
            ON CONFLICT(sk_token) DO UPDATE SET
                provider=excluded.provider,
                model=excluded.model,
                sandbox_id=excluded.sandbox_id,
                is_active=1,
                updated_at=CURRENT_TIMESTAMP
            """,
            (sk_token, provider, model, sandbox_id),
        )
        await db.commit()


async def get_oneapi_key(sk_token: str) -> Optional[Dict[str, Any]]:
    """读取 OneAPI sk 映射"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM oneapi_keys WHERE sk_token = ? AND is_active = 1",
            (sk_token,),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def list_oneapi_keys(limit: int = 50) -> List[Dict[str, Any]]:
    """列出 OneAPI sk 映射（后台展示）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT sk_token, provider, model, sandbox_id, is_active, created_at, updated_at
            FROM oneapi_keys
            ORDER BY updated_at DESC
            LIMIT ?
            """,
            (int(limit),),
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def deactivate_oneapi_key(sk_token: str) -> bool:
    """停用 OneAPI sk"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE oneapi_keys SET is_active = 0, updated_at = CURRENT_TIMESTAMP WHERE sk_token = ?",
            (sk_token,),
        )
        await db.commit()
        return cur.rowcount > 0

async def get_user(user_id: int):
    """获取用户信息"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM users WHERE user_id = ?", (user_id,)) as cursor:
            return await cursor.fetchone()

async def get_message_by_tg_id(tg_msg_id: int):
    """通过 tg_msg_id 获取消息的完整上下文"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM messages WHERE tg_msg_id = ?", (tg_msg_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None

async def get_user_id_by_admin_msg(admin_msg_id: int):
    """通过管理员收到的消息 ID 查找原始发送者的 User ID"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT user_id FROM admin_message_mapping WHERE admin_msg_id = ?", (admin_msg_id,)) as cursor:
            row = await cursor.fetchone()
            return row[0] if row else None

async def get_original_msg_id_by_admin_msg(admin_msg_id: int):
    """通过管理员收到的消息 ID 查找该消息在 messages 表中的 tg_msg_id (用户端的消息 ID)"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT original_tg_msg_id FROM admin_message_mapping WHERE admin_msg_id = ?", (admin_msg_id,)) as cursor:
            row = await cursor.fetchone()
            return row[0] if row and row[0] else None

async def get_admin_msg_id_by_user_msg(original_tg_msg_id: int):
    """通过用户的原始消息 ID，查找管理员接收到的对应消息 ID"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT admin_msg_id FROM admin_message_mapping WHERE original_tg_msg_id = ?", (original_tg_msg_id,)) as cursor:
            row = await cursor.fetchone()
            return row[0] if row else None

async def save_admin_mapping(admin_msg_id: int, user_id: int, original_tg_msg_id: Optional[int] = None):
    """保存管理员收到的消息 ID 与 User ID / 原始 TG 消息 ID 的映射关系"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("INSERT OR REPLACE INTO admin_message_mapping (admin_msg_id, user_id, original_tg_msg_id) VALUES (?, ?, ?)", (admin_msg_id, user_id, original_tg_msg_id))
        await db.commit()

async def create_or_update_user(user_id: int, email: Optional[str] = None):
    """创建或更新用户"""
    async with aiosqlite.connect(DB_NAME) as db:
        user = await get_user(user_id)
        if user:
            if email is not None:
                await db.execute("UPDATE users SET email = ? WHERE user_id = ?", (email, user_id))
        else:
            await db.execute("INSERT INTO users (user_id, email) VALUES (?, ?)", (user_id, email))
        await db.commit()

async def save_message(
    from_userid: int, 
    to_userid: int, 
    content: Optional[str], 
    msg_type: str = 'text', 
    file_id: Optional[str] = None, 
    is_read: bool = False, 
    username: Optional[str] = None, 
    tg_msg_id: Optional[int] = None, 
    translated_text: Optional[str] = None, 
    quoted_text: Optional[str] = None, 
    quoted_msg_id: Optional[int] = None, 
    quoted_file_id: Optional[str] = None, 
    quoted_type: str = 'text', 
    status: str = 'sent', 
    avatar_file_id: Optional[str] = None,
    media_url: Optional[str] = None,
    original_text: Optional[str] = None,
    admin_userid: Optional[str] = None,
    emoji: Optional[str] = None,
    trace_id: Optional[str] = None,
    tenant_id: Optional[str] = None,
):
    """统一保存消息，并更新管理员收件箱汇总 (2026 规格：支持多媒体、引用、翻译、实名及头像更新)"""
    import uuid
    if not trace_id:
        trace_id = str(uuid.uuid4())
    async with aiosqlite.connect(DB_NAME) as db:
        effective_tenant_id = await _resolve_message_tenant_id(
            db,
            from_userid=from_userid,
            to_userid=to_userid,
            tenant_id=tenant_id,
        )
        # 1. 存入 messages 主表
        cur = await db.execute(
            """INSERT INTO messages 
               (trace_id, from_userid, to_userid, content, type, file_id, media_url, is_read, tg_msg_id, auto_translation, translated_text, original_text, admin_userid, quoted_text, quoted_msg_id, quoted_file_id, quoted_type, status, tenant_id) 
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (trace_id, from_userid, to_userid, content, msg_type, file_id, media_url, 1 if is_read else 0, tg_msg_id, translated_text, translated_text, original_text, admin_userid, quoted_text, quoted_msg_id, quoted_file_id, quoted_type, status, effective_tenant_id)
        )
        msg_db_id = cur.lastrowid
        
        # 1.1 如果有表情包，存入 emoji_messages
        if emoji:
            await db.execute("INSERT INTO emoji_messages (msg_id, emoji) VALUES (?, ?)", (msg_db_id, emoji))
        
        # 2. 如果是用户发给管理员的消息，更新 admin_inbox 收件箱
        import config
        if to_userid == config.ADMIN_ID:
            # 获取该用户的所有未读消息
            db.row_factory = aiosqlite.Row
            async with db.execute(
                "SELECT content FROM messages WHERE from_userid = ? AND is_read = 0 AND is_deleted = 0 AND tenant_id = ? ORDER BY timestamp ASC",
                (from_userid, effective_tenant_id),
            ) as cursor:
                unread_rows = await cursor.fetchall()
                unread_msgs = [row['content'] for row in unread_rows]
            
            # 构造摘要 (取最新 3 条)
            from datetime import datetime
            summary_lines = []
            for msg_content in unread_msgs[-3:]:
                time_str = datetime.now().strftime("%H:%M")
                display_content = (msg_content[:20] + "...") if msg_content and len(msg_content) > 20 else (msg_content or f"[{msg_type}]")
                summary_lines.append(f"{time_str}: {display_content}")
                
            full_summary = "\n".join(summary_lines)
            if len(full_summary) > 500:
                full_summary = full_summary[:497] + "..."
                
            # 2026 修复：强制更新 from_username 和 avatar_file_id 以解决“匿名客户”和“头像显示”问题
            # 检查是否已存在 inbox 记录
            async with db.execute("SELECT from_userid FROM admin_inbox WHERE from_userid = ?", (from_userid,)) as cursor:
                exists = await cursor.fetchone()
                
            if exists:
                # 无论是否有 username 传入，都尝试更新
                update_parts = ["summary = ?", "unread_count = ?", "last_active = CURRENT_TIMESTAMP", "tenant_id = ?"]
                params = [full_summary, len(unread_msgs), effective_tenant_id]
                
                if username:
                    update_parts.append("from_username = ?")
                    params.append(username)
                
                if avatar_file_id:
                    update_parts.append("avatar_file_id = ?")
                    params.append(avatar_file_id)
                    
                update_sql = f"UPDATE admin_inbox SET {', '.join(update_parts)} WHERE from_userid = ?"
                params.append(from_userid)
                await db.execute(update_sql, tuple(params))
            else:
                # 新用户入库，必须确保有名字
                display_name = username or f"User {from_userid}"
                await db.execute(
                    "INSERT INTO admin_inbox (from_userid, from_username, summary, unread_count, avatar_file_id, tenant_id) VALUES (?, ?, ?, ?, ?, ?)",
                    (from_userid, display_name, full_summary, len(unread_msgs), avatar_file_id, effective_tenant_id)
                )
        await db.commit()
        return msg_db_id

async def update_user_avatar(user_id: int, avatar_file_id: str):
    """更新用户头像缓存"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE admin_inbox SET avatar_file_id = ? WHERE from_userid = ?",
            (avatar_file_id, user_id)
        )
        await db.commit()

async def update_message_status(msg_db_id: Optional[int], status: str, tg_msg_id: Optional[int] = None):
    """更新消息的发送状态和 Telegram 消息 ID"""
    if msg_db_id is None:
        return
    async with aiosqlite.connect(DB_NAME) as db:
        if tg_msg_id:
            await db.execute("UPDATE messages SET status = ?, tg_msg_id = ? WHERE msg_id = ?", (status, tg_msg_id, msg_db_id))
        else:
            await db.execute("UPDATE messages SET status = ? WHERE msg_id = ?", (status, msg_db_id))
        await db.commit()

async def mark_messages_as_read(from_userid: int, tenant_id: Optional[str] = None):
    """将特定用户发来的消息标记为已读"""
    async with aiosqlite.connect(DB_NAME) as db:
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            await db.execute(
                "UPDATE messages SET is_read = 1 WHERE from_userid = ? AND tenant_id = ?",
                (from_userid, normalized_tenant),
            )
            await db.execute(
                "UPDATE admin_inbox SET unread_count = 0, summary = '' WHERE from_userid = ? AND tenant_id = ?",
                (from_userid, normalized_tenant),
            )
        else:
            await db.execute("UPDATE messages SET is_read = 1 WHERE from_userid = ?", (from_userid,))
            await db.execute("UPDATE admin_inbox SET unread_count = 0, summary = '' WHERE from_userid = ?", (from_userid,))
        await db.commit()

async def get_inbox_list(tenant_id: Optional[str] = None, merchant_id: Optional[int] = None):
    """获取来访用户列表；tenant scope 下只返回当前商户会话。"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            counterpart_id = await _resolve_scope_bot_id(db, tenant_id=normalized_tenant, merchant_id=merchant_id)
            if counterpart_id is None:
                counterpart_id = ADMIN_ID

            async with db.execute(
                """
                SELECT
                    CASE WHEN from_userid = ? THEN to_userid ELSE from_userid END AS uid,
                    MAX(timestamp) AS last_active
                FROM messages
                WHERE tenant_id = ?
                  AND ((from_userid = ? AND to_userid != ?) OR (to_userid = ? AND from_userid != ?))
                  AND is_deleted = 0
                GROUP BY uid
                ORDER BY MAX(timestamp) DESC
                """,
                (counterpart_id, normalized_tenant, counterpart_id, counterpart_id, counterpart_id, counterpart_id),
            ) as cursor:
                rows = await cursor.fetchall()

            users: list[dict] = []
            for row in rows:
                uid = int(row["uid"])
                last_active = row["last_active"]
                async with db.execute(
                    """
                    SELECT content
                    FROM messages
                    WHERE tenant_id = ?
                      AND is_deleted = 0
                      AND ((from_userid = ? AND to_userid = ?) OR (from_userid = ? AND to_userid = ?))
                    ORDER BY timestamp DESC
                    LIMIT 1
                    """,
                    (normalized_tenant, uid, counterpart_id, counterpart_id, uid),
                ) as msg_cursor:
                    latest_row = await msg_cursor.fetchone()

                async with db.execute(
                    """
                    SELECT COUNT(*)
                    FROM messages
                    WHERE tenant_id = ?
                      AND from_userid = ?
                      AND to_userid = ?
                      AND is_read = 0
                      AND is_deleted = 0
                    """,
                    (normalized_tenant, uid, counterpart_id),
                ) as unread_cursor:
                    unread_row = await unread_cursor.fetchone()

                display_name = None
                avatar_file_id = None
                async with db.execute(
                    "SELECT from_username, avatar_file_id FROM admin_inbox WHERE from_userid = ? AND tenant_id = ? LIMIT 1",
                    (uid, normalized_tenant),
                ) as inbox_cursor:
                    inbox_row = await inbox_cursor.fetchone()
                    if inbox_row:
                        display_name = inbox_row["from_username"]
                        avatar_file_id = inbox_row["avatar_file_id"]

                if not display_name:
                    async with db.execute(
                        "SELECT username, first_name FROM potential_users WHERE user_id = ? AND tenant_id = ? LIMIT 1",
                        (uid, normalized_tenant),
                    ) as potential_cursor:
                        potential_row = await potential_cursor.fetchone()
                        if potential_row:
                            display_name = potential_row["username"] or potential_row["first_name"]

                if not display_name:
                    async with db.execute(
                        """
                        SELECT username, first_name, avatar_file_id
                        FROM group_members
                        WHERE user_id = ?
                          AND tenant_id = ?
                        ORDER BY joined_at DESC
                        LIMIT 1
                        """,
                        (uid, normalized_tenant),
                    ) as member_cursor:
                        member_row = await member_cursor.fetchone()
                        if member_row:
                            display_name = member_row["username"] or member_row["first_name"]
                            avatar_file_id = avatar_file_id or member_row["avatar_file_id"]

                users.append({
                    "from_userid": uid,
                    "from_username": display_name or f"User {uid}",
                    "summary": (latest_row["content"] if latest_row and latest_row["content"] else "[媒体消息]"),
                    "unread_count": int(unread_row[0] if unread_row else 0),
                    "last_active": last_active,
                    "tenant_id": normalized_tenant,
                    "avatar_file_id": avatar_file_id,
                })

            return users

        async with db.execute("SELECT * FROM admin_inbox ORDER BY last_active DESC") as cursor:
            rows = await cursor.fetchall()
            if rows:
                return [dict(row) for row in rows]
            
            # 如果收件箱为空，尝试从 messages 表重建 (防止更新后数据展示不全)
            logger.info("Admin Inbox 为空，尝试从 messages 表自动重建...")
            await rebuild_inbox_from_messages()
            
            # 重新查询
            async with db.execute("SELECT * FROM admin_inbox ORDER BY last_active DESC") as cursor:
                rows = await cursor.fetchall()
                return [dict(row) for row in rows]

async def rebuild_inbox_from_messages():
    """从 messages 历史记录中重建 admin_inbox 汇总表"""
    async with aiosqlite.connect(DB_NAME) as db:
        # 1. 查找所有给管理员发过消息的用户，以及管理员回复过的用户
        # 注意：messages 表中没有 username 字段，因此从 potential_users 或 group_members 尝试获取名称
        query = """
            SELECT 
                CASE WHEN from_userid = ? THEN to_userid ELSE from_userid END as uid,
                MAX(timestamp) as last_ts
            FROM messages
            WHERE from_userid = ? OR to_userid = ?
            GROUP BY uid
        """
        async with db.execute(query, (ADMIN_ID, ADMIN_ID, ADMIN_ID)) as cursor:
            users = await cursor.fetchall()
            
        for user in users:
            uid, last_ts = user
            
            # 尝试从 potential_users 获取用户名
            uname = None
            async with db.execute("SELECT username, first_name FROM potential_users WHERE user_id = ?", (uid,)) as c:
                p_row = await c.fetchone()
                if p_row:
                    uname = p_row[0] or p_row[1]
            
            if not uname:
                # 尝试从 group_members 获取
                async with db.execute("SELECT username, first_name FROM group_members WHERE user_id = ?", (uid,)) as c:
                    m_row = await c.fetchone()
                    if m_row:
                        uname = m_row[0] or m_row[1]
            
            # 获取未读数
            async with db.execute(
                "SELECT COUNT(*) FROM messages WHERE from_userid = ? AND to_userid = ? AND is_read = 0",
                (uid, ADMIN_ID)
            ) as c:
                unread = (await c.fetchone())[0]
                
            # 获取最新一条消息内容
            async with db.execute(
                "SELECT content FROM messages WHERE (from_userid = ? AND to_userid = ?) OR (from_userid = ? AND to_userid = ?) ORDER BY timestamp DESC LIMIT 1",
                (uid, ADMIN_ID, ADMIN_ID, uid)
            ) as c:
                msg = await c.fetchone()
                summary = msg[0][:30] + "..." if msg and msg[0] else "[媒体消息]"

            await db.execute(
                """INSERT OR REPLACE INTO admin_inbox 
                   (from_userid, from_username, summary, unread_count, last_active)
                   VALUES (?, ?, ?, ?, ?)""",
                (uid, uname or f"User {uid}", summary, unread, last_ts)
            )
        await db.commit()
        logger.info(f"✅ 已成功从历史记录重建 {len(users)} 个会话入口")

async def get_latest_active_user():
    """获取最近活跃的用户 ID"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT from_userid, from_username FROM admin_inbox ORDER BY last_active DESC LIMIT 1") as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None

async def get_recent_active_count(minutes: int = 5):
    """
    2026 规格：获取指定分钟内有活跃咨询的用户数量 (并发检测核心算法)
    """
    from datetime import datetime, timedelta
    threshold_time = (datetime.now() - timedelta(minutes=minutes)).isoformat()
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(DISTINCT from_userid) FROM admin_inbox WHERE last_active > ?",
            (threshold_time,)
        ) as cursor:
            row = await cursor.fetchone()
            return row[0] if row else 0

async def get_chat_history(
    user_id: int,
    limit: int = 50,
    offset: int = 0,
    tenant_id: Optional[str] = None,
    merchant_id: Optional[int] = None,
):
    """获取与特定用户的聊天历史"""
    import config
    admin_id = config.ADMIN_ID
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        normalized_tenant = _normalize_tenant_id(tenant_id)
        counterpart_id = admin_id
        if normalized_tenant:
            counterpart_id = await _resolve_scope_bot_id(db, tenant_id=normalized_tenant, merchant_id=merchant_id) or admin_id

        query = """
            SELECT m.*, e.emoji 
            FROM messages m
            LEFT JOIN emoji_messages e ON m.msg_id = e.msg_id
            WHERE ((m.from_userid = ? AND m.to_userid = ?) 
               OR (m.from_userid = ? AND m.to_userid = ?))
               AND m.is_deleted = 0
            ORDER BY m.timestamp DESC
            LIMIT ? OFFSET ?
        """
        params: list[Any] = [user_id, counterpart_id, counterpart_id, user_id]
        if normalized_tenant:
            query = query.replace("AND m.is_deleted = 0", "AND m.is_deleted = 0 AND m.tenant_id = ?")
            params.append(normalized_tenant)
        params.extend([limit, offset])
        async with db.execute(query, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            messages = [dict(row) for row in rows]
            messages.reverse() # 前端需要按时间顺序列出（旧到新），所以反转返回
            return messages

async def get_active_alerts():
    """获取所有活跃的价格监控任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM alerts WHERE is_active = 1") as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]

async def update_alert_state(alert_id: int, state: str, is_active: bool = True):
    """更新监控任务的状态"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE alerts SET channel_state = ?, is_active = ? WHERE alert_id = ?", (state, 1 if is_active else 0, alert_id))
        await db.commit()

async def get_active_monitors():
    """获取所有活跃的钱包监听任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM address_monitors WHERE is_active = 1") as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]

async def update_monitor_timestamp(monitor_id: int, timestamp: int):
    """更新钱包监听的最后交易时间戳"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE address_monitors SET last_tx_timestamp = ? WHERE monitor_id = ?", (timestamp, monitor_id))
        await db.commit()


async def update_monitor_risk(monitor_id: int, risk_level: str = 'normal', risk_note: Optional[str] = None):
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE address_monitors SET risk_level = ?, risk_note = ? WHERE monitor_id = ?",
            (risk_level, risk_note, monitor_id),
        )
        await db.commit()


async def update_monitor_remark(monitor_id: int, remark: str = '') -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE address_monitors SET remark = ? WHERE monitor_id = ?",
            (remark or '', monitor_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def get_recent_monitor_transactions(monitor_id: int, direction: Optional[str] = None, limit: int = 20) -> List[Dict[str, Any]]:
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        sql = "SELECT * FROM monitor_transactions WHERE monitor_id = ?"
        params: List[Union[int, str]] = [monitor_id]
        if direction:
            sql += " AND direction = ?"
            params.append(direction)
        sql += " ORDER BY tx_timestamp DESC LIMIT ?"
        params.append(limit)
        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def save_monitor_transaction(tx_id: str, monitor_id: int, user_id: int, address: str, network: str,
                                   direction: str, amount: float, token_symbol: str, tx_timestamp: int,
                                   counterparty: Optional[str] = None, risk_level: str = 'normal',
                                   risk_note: Optional[str] = None) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """INSERT OR IGNORE INTO monitor_transactions
               (tx_id, monitor_id, user_id, address, network, direction, amount, token_symbol, tx_timestamp, counterparty, risk_level, risk_note)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (tx_id, monitor_id, user_id, address, network, direction, amount, token_symbol, tx_timestamp, counterparty, risk_level, risk_note),
        )
        await db.commit()
        return cur.rowcount > 0


def _period_boundaries_ms(period: str, previous: bool = False) -> tuple[int, int]:
    now = datetime.now()

    if period == 'day':
        start = datetime(now.year, now.month, now.day)
        end = start + timedelta(days=1)
        if previous:
            start = start - timedelta(days=1)
            end = end - timedelta(days=1)
    elif period == 'week':
        start = datetime(now.year, now.month, now.day) - timedelta(days=now.weekday())
        end = start + timedelta(days=7)
        if previous:
            start = start - timedelta(days=7)
            end = end - timedelta(days=7)
    else:
        start = datetime(now.year, now.month, 1)
        if now.month == 12:
            end = datetime(now.year + 1, 1, 1)
        else:
            end = datetime(now.year, now.month + 1, 1)
        if previous:
            prev_end = start
            prev_year = start.year if start.month > 1 else start.year - 1
            prev_month = start.month - 1 if start.month > 1 else 12
            prev_start = datetime(prev_year, prev_month, 1)
            start = prev_start
            end = prev_end

    return int(start.timestamp() * 1000), int(end.timestamp() * 1000)


def _build_monitor_scope(addresses: Optional[List[str]] = None, monitor_ids: Optional[List[int]] = None, user_id: Optional[int] = None) -> tuple[str, List[Union[int, str]]]:
    where = []
    params: List[Union[int, str]] = []

    if addresses:
        placeholders = ','.join(['?'] * len(addresses))
        where.append(f"address IN ({placeholders})")
        params.extend(addresses)

    if monitor_ids:
        placeholders = ','.join(['?'] * len(monitor_ids))
        where.append(f"monitor_id IN ({placeholders})")
        params.extend([int(x) for x in monitor_ids])

    if user_id is not None:
        where.append("user_id = ?")
        params.append(int(user_id))

    return (" AND " + " AND ".join(where)) if where else "", params


async def get_monitor_pnl_summary(period: str = 'day', user_id: Optional[int] = None,
                                  addresses: Optional[List[str]] = None,
                                  monitor_ids: Optional[List[int]] = None,
                                  previous: bool = False) -> Dict[str, float]:
    start_ms, end_ms = _period_boundaries_ms(period, previous=previous)
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)

    sql = f"""
        SELECT
            SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END) AS inflow,
            SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END) AS outflow
        FROM monitor_transactions
        WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql}
    """

    params: List[Union[int, str]] = [start_ms, end_ms]
    params.extend(scope_params)

    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(sql, tuple(params)) as cursor:
            row = await cursor.fetchone()
            inflow = float(row[0] or 0)
            outflow = float(row[1] or 0)
            return {'inflow': inflow, 'outflow': outflow, 'profit': inflow - outflow, 'start_ms': start_ms, 'end_ms': end_ms}


async def get_monitor_period_series(period: str = 'day', user_id: Optional[int] = None,
                                    addresses: Optional[List[str]] = None,
                                    monitor_ids: Optional[List[int]] = None) -> List[Dict[str, Any]]:
    now = datetime.now()
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)

    if period == 'day':
        point_count = 24
        start = datetime(now.year, now.month, now.day)
        step = timedelta(hours=1)
        fmt = '%H:00'
    elif period == 'week':
        point_count = 7
        start = datetime(now.year, now.month, now.day) - timedelta(days=now.weekday())
        step = timedelta(days=1)
        fmt = '%m-%d'
    else:
        point_count = calendar.monthrange(now.year, now.month)[1]
        start = datetime(now.year, now.month, 1)
        step = timedelta(days=1)
        fmt = '%m-%d'

    result: List[Dict[str, Any]] = []
    async with aiosqlite.connect(DB_NAME) as db:
        for idx in range(point_count):
            b_start = start + step * idx
            b_end = b_start + step
            b_start_ms = int(b_start.timestamp() * 1000)
            b_end_ms = int(b_end.timestamp() * 1000)

            sql = f"""
                SELECT
                    SUM(CASE WHEN direction = 'in' THEN amount ELSE 0 END) AS inflow,
                    SUM(CASE WHEN direction = 'out' THEN amount ELSE 0 END) AS outflow
                FROM monitor_transactions
                WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql}
            """
            params: List[Union[int, str]] = [b_start_ms, b_end_ms]
            params.extend(scope_params)
            async with db.execute(sql, tuple(params)) as cursor:
                row = await cursor.fetchone()
                inflow = float(row[0] or 0)
                outflow = float(row[1] or 0)
            result.append({
                'label': b_start.strftime(fmt),
                'inflow': inflow,
                'outflow': outflow,
                'profit': inflow - outflow,
                'start_ms': b_start_ms,
                'end_ms': b_end_ms,
            })
    return result


async def update_monitor_transaction_meta(tx_id: str, income_reason: Optional[str] = None,
                                          expense_reason: Optional[str] = None,
                                          payer: Optional[str] = None,
                                          payee: Optional[str] = None) -> bool:
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE monitor_transactions
            SET income_reason = ?, expense_reason = ?, payer = ?, payee = ?
            WHERE tx_id = ?
            """,
            (income_reason, expense_reason, payer, payee, tx_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def batch_update_monitor_transaction_meta(items: List[Dict[str, Any]]) -> int:
    updated = 0
    async with aiosqlite.connect(DB_NAME) as db:
        for it in items:
            tx_id = (it.get('tx_id') or '').strip()
            if not tx_id:
                continue
            cur = await db.execute(
                """
                UPDATE monitor_transactions
                SET income_reason = ?, expense_reason = ?, payer = ?, payee = ?
                WHERE tx_id = ?
                """,
                (
                    (it.get('income_reason') or '').strip() or None,
                    (it.get('expense_reason') or '').strip() or None,
                    (it.get('payer') or '').strip() or None,
                    (it.get('payee') or '').strip() or None,
                    tx_id,
                ),
            )
            if cur.rowcount > 0:
                updated += 1
        await db.commit()
    return updated


async def get_monitor_transactions_for_export(start_ms: int, end_ms: int, user_id: Optional[int] = None,
                                              addresses: Optional[List[str]] = None,
                                              monitor_ids: Optional[List[int]] = None,
                                              annotated_only: bool = False) -> List[Dict[str, Any]]:
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)
    annotated_sql = ""
    if annotated_only:
        annotated_sql = " AND (COALESCE(income_reason, '') <> '' OR COALESCE(expense_reason, '') <> '' OR COALESCE(payer, '') <> '' OR COALESCE(payee, '') <> '')"
    sql = f"""
        SELECT *
        FROM monitor_transactions
        WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql} {annotated_sql}
        ORDER BY tx_timestamp ASC, tx_id ASC
    """
    params: List[Union[int, str]] = [start_ms, end_ms]
    params.extend(scope_params)
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def get_monitor_transactions_list(period: str = 'month', user_id: Optional[int] = None,
                                        addresses: Optional[List[str]] = None,
                                        monitor_ids: Optional[List[int]] = None,
                                        limit: int = 200) -> List[Dict[str, Any]]:
    start_ms, end_ms = _period_boundaries_ms(period)
    scope_sql, scope_params = _build_monitor_scope(addresses=addresses, monitor_ids=monitor_ids, user_id=user_id)
    sql = f"""
        SELECT *
        FROM monitor_transactions
        WHERE tx_timestamp >= ? AND tx_timestamp < ? {scope_sql}
        ORDER BY tx_timestamp DESC, tx_id DESC
        LIMIT ?
    """
    params: List[Union[int, str]] = [start_ms, end_ms]
    params.extend(scope_params)
    params.append(int(limit))
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]

async def get_monitors_by_user(user_id: int, active_only: bool = True):
    """获取某用户绑定的钱包监听记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        q = "SELECT * FROM address_monitors WHERE user_id = ?"
        if active_only:
            q += " AND is_active = 1"
        q += " ORDER BY monitor_id DESC"
        async with db.execute(q, (user_id,)) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_all_monitors_for_admin(active_only: bool = True):
    """管理端：列出所有监听记录（含 user_id）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        q = "SELECT * FROM address_monitors"
        if active_only:
            q += " WHERE is_active = 1"
        q += " ORDER BY monitor_id DESC"
        async with db.execute(q) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def deactivate_wallet_monitor(monitor_id: int, operator_user_id: int) -> bool:
    """
    取消监听：仅地址所属用户或管理员可操作。
    返回是否成功更新一行。
    """
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT user_id FROM address_monitors WHERE monitor_id = ? AND is_active = 1",
            (monitor_id,),
        ) as cursor:
            row = await cursor.fetchone()
        if not row:
            return False
        owner_id = row[0]
        is_admin = str(operator_user_id) == str(ADMIN_ID)
        if owner_id != operator_user_id and not is_admin:
            return False
        await db.execute(
            "UPDATE address_monitors SET is_active = 0 WHERE monitor_id = ?",
            (monitor_id,),
        )
        await db.commit()
        return True


async def deactivate_monitor_admin(monitor_id: int) -> bool:
    """Web 管理端：强制关闭某条监听（不校验操作者）"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE address_monitors SET is_active = 0 WHERE monitor_id = ? AND is_active = 1",
            (monitor_id,),
        )
        await db.commit()
        return cur.rowcount > 0


async def save_wallet_monitor(user_id: int, address: str, network: str, initial_timestamp: Optional[int] = None, remark: str = ''):
    """保存钱包监听地址（同一用户+地址+网络只保留一条，并支持初始化时间戳避免历史推送）"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT monitor_id FROM address_monitors WHERE user_id = ? AND address = ? AND network = ?",
            (user_id, address, network),
        ) as cursor:
            row = await cursor.fetchone()
        if row:
            # 如果已存在，更新为活跃状态，如果提供了时间戳则更新时间戳
            if initial_timestamp is not None:
                await db.execute(
                    "UPDATE address_monitors SET is_active = 1, last_tx_timestamp = ?, remark = ? WHERE monitor_id = ?",
                    (initial_timestamp, remark or '', row[0]),
                )
            else:
                await db.execute(
                    "UPDATE address_monitors SET is_active = 1, remark = ? WHERE monitor_id = ?",
                    (remark or '', row[0]),
                )
        else:
            await db.execute(
                "INSERT INTO address_monitors (user_id, address, network, remark, is_active, last_tx_timestamp) VALUES (?, ?, ?, ?, 1, ?)",
                (user_id, address, network, remark or '', initial_timestamp),
            )
        await db.commit()

async def save_pending_admin_msg(msg_uuid: str, admin_id: int, content: Optional[str], msg_type: str = 'text', file_id: Optional[str] = None):
    """保存管理员待确认的消息到队列 (防丢失)"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "INSERT INTO pending_admin_messages (msg_uuid, admin_id, content, msg_type, file_id) VALUES (?, ?, ?, ?, ?)",
            (msg_uuid, admin_id, content, msg_type, file_id)
        )
        await db.commit()

async def get_pending_admin_msg(msg_uuid: str):
    """根据 UUID 获取待确认消息"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM pending_admin_messages WHERE msg_uuid = ?", (msg_uuid,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None

async def update_admin_info(admin_id: int, username: str):
    """更新管理员的 TG 姓名/用户名（无记录时先插入，避免启动预热时 UPDATE 影响 0 行）"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "INSERT OR IGNORE INTO admin_users (admin_id, username, worker_no, role) VALUES (?, ?, 'CS001', 'root')",
            (admin_id, username),
        )
        await db.execute(
            "UPDATE admin_users SET username = ? WHERE admin_id = ?",
            (username, admin_id),
        )
        await db.commit()

async def get_admin_info(admin_id: int):
    """获取管理员详细信息 (工号/角色/状态)"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM admin_users WHERE admin_id = ?", (admin_id,)) as cursor:
            row = await cursor.fetchone()
            if row:
                return dict(row)
            # 如果不存在，尝试初始化默认值 (为了向前兼容)
            import config
            if str(admin_id) == str(config.ADMIN_ID):
                await db.execute(
                    "INSERT OR IGNORE INTO admin_users (admin_id, username, worker_no, role) VALUES (?, ?, ?, ?)",
                    (admin_id, "System Admin", "CS001", "root")
                )
                await db.commit()
                return {"admin_id": admin_id, "username": "System Admin", "worker_no": "CS001", "role": "root", "is_online": 1}
            return None

async def update_pending_admin_msg_status(msg_uuid: str, status: str):
    """更新待确认消息的状态 (sent/expired)"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE pending_admin_messages SET status = ? WHERE msg_uuid = ?", (status, msg_uuid))
        await db.commit()

async def soft_delete_chat(user_id: int, tenant_id: Optional[str] = None, merchant_id: Optional[int] = None):
    """软删除特定用户的聊天记录；tenant scope 下仅删除当前租户会话。"""
    async with aiosqlite.connect(DB_NAME) as db:
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            counterpart_id = await _resolve_scope_bot_id(db, tenant_id=normalized_tenant, merchant_id=merchant_id) or ADMIN_ID
            await db.execute(
                """
                UPDATE messages
                SET is_deleted = 1
                WHERE tenant_id = ?
                  AND ((from_userid = ? AND to_userid = ?) OR (from_userid = ? AND to_userid = ?))
                """,
                (normalized_tenant, user_id, counterpart_id, counterpart_id, user_id),
            )
        else:
            # 将该用户的消息标记为删除
            await db.execute("UPDATE messages SET is_deleted = 1 WHERE from_userid = ? OR to_userid = ?", (user_id, user_id))
            # 同时从 inbox 移除
            await db.execute("DELETE FROM admin_inbox WHERE from_userid = ?", (user_id,))
        await db.commit()


async def get_active_users_for_selection(days: int = 7, limit: int = 50) -> list:
    """获取最近活跃用户列表（用于管理员选择发送目标）"""
    from datetime import datetime, timedelta
    cutoff_date = (datetime.now() - timedelta(days=days)).isoformat()
    
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        query = """
            SELECT 
                CASE WHEN m.from_userid = ? THEN m.to_userid ELSE m.from_userid END as from_userid,
                COALESCE(a.from_username, 'User ' || CASE WHEN m.from_userid = ? THEN m.to_userid ELSE m.from_userid END) as from_username,
                MAX(m.timestamp) as last_msg_time,
                SUM(CASE WHEN m.is_read = 0 AND m.to_userid = ? THEN 1 ELSE 0 END) as unread_count
            FROM messages m
            LEFT JOIN admin_inbox a ON (CASE WHEN m.from_userid = ? THEN m.to_userid ELSE m.from_userid END) = a.from_userid
            WHERE m.timestamp > ? AND ((m.from_userid = ? AND m.to_userid != ?) OR (m.from_userid != ? AND m.to_userid = ?))
            GROUP BY CASE WHEN m.from_userid = ? THEN m.to_userid ELSE m.from_userid END
            ORDER BY MAX(m.timestamp) DESC
            LIMIT ?
        """
        async with db.execute(query, (ADMIN_ID, ADMIN_ID, ADMIN_ID, ADMIN_ID, cutoff_date, ADMIN_ID, ADMIN_ID, ADMIN_ID, ADMIN_ID, ADMIN_ID, limit)) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def delete_pending_admin_msg(msg_uuid: str):
    """删除待发送的管理员消息"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("DELETE FROM pending_admin_messages WHERE msg_uuid = ?", (msg_uuid,))
        await db.commit()



async def upsert_potential_user(user_id: int, username: Optional[str], first_name: Optional[str] = None, last_name: Optional[str] = None, 
                             message_text: Optional[str] = None, keywords: Optional[str] = None, profile_tags: Optional[str] = None,
                             group_id: Optional[int] = None, group_title: Optional[str] = None,
                             tenant_id: Optional[str] = None):
    """保存或更新潜在用户信息 (含标签累积逻辑)"""
    async with aiosqlite.connect(DB_NAME) as db:
        effective_tenant_id = _normalize_tenant_id(tenant_id) or await _resolve_tenant_id_from_group(db, group_id) or 'default'
        # 获取现有记录以进行累积
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT keywords, profile_tags FROM potential_users WHERE user_id = ?", (user_id,)) as cursor:
            row = await cursor.fetchone()
            
        new_kws = keywords or ""
        new_tags = profile_tags or ""
        
        if row:
            # 累积关键词
            existing_kws = set([k.strip() for k in (row['keywords'] or "").split(',') if k.strip()])
            current_kws = set([k.strip() for k in (keywords or "").split(',') if k.strip()])
            new_kws = ",".join(sorted(list(existing_kws | current_kws)))
            
            # 累积标签
            existing_tags = set([t.strip() for t in (row['profile_tags'] or "").split(',') if t.strip()])
            current_tags = set([t.strip() for t in (profile_tags or "").split(',') if t.strip()])
            new_tags = ",".join(sorted(list(existing_tags | current_tags)))

        await db.execute(
            """INSERT INTO potential_users 
               (user_id, username, first_name, last_name, message_text, keywords, profile_tags, group_id, group_title, tenant_id, captured_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
               ON CONFLICT(user_id) DO UPDATE SET
               username=excluded.username,
               first_name=excluded.first_name,
               last_name=excluded.last_name,
               message_text=excluded.message_text,
               keywords=?,
               profile_tags=?,
               group_id=excluded.group_id,
               group_title=excluded.group_title,
               tenant_id=excluded.tenant_id,
               captured_at=CURRENT_TIMESTAMP""",
            (user_id, username, first_name, last_name, message_text, new_kws, new_tags, group_id, group_title, effective_tenant_id, new_kws, new_tags)
        )
        await db.commit()


async def get_tracked_group_tenant_id(group_id: int) -> Optional[str]:
    """获取群组当前归属租户"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT tenant_id FROM tracked_groups WHERE group_id = ?",
            (group_id,),
        ) as cursor:
            row = await cursor.fetchone()
            if not row:
                return None
            tenant_id = row[0]
            return str(tenant_id).strip() if tenant_id is not None else None


async def upsert_tracked_group(group_id: int, title: str, tenant_id: Optional[str] = None):
    """登记或更新追踪的群组信息 (支持首次发现与更新时间 + 租户隔离)"""
    effective_tenant = (tenant_id or '').strip()
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT tenant_id FROM tracked_groups WHERE group_id = ?",
            (group_id,),
        ) as cursor:
            existing = await cursor.fetchone()
        if existing and str(existing['tenant_id'] or '').strip():
            effective_tenant = str(existing['tenant_id']).strip()
        if not effective_tenant:
            effective_tenant = 'default'
        await db.execute(
            """INSERT INTO tracked_groups (group_id, group_title, tenant_id, first_seen, updated_at)
               VALUES (?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
               ON CONFLICT(group_id) DO UPDATE SET
               group_title=excluded.group_title,
               tenant_id=excluded.tenant_id,
               updated_at=CURRENT_TIMESTAMP""",
            (group_id, title, effective_tenant)
        )
        await db.commit()


async def get_potential_tags(tenant_id: Optional[str] = None):
    """从 potential_users 表中去重聚合所有标签。"""
    async with aiosqlite.connect(DB_NAME) as db:
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            cursor = await db.execute(
                "SELECT profile_tags FROM potential_users WHERE profile_tags IS NOT NULL AND tenant_id = ?",
                (normalized_tenant,),
            )
        else:
            cursor = await db.execute("SELECT profile_tags FROM potential_users WHERE profile_tags IS NOT NULL")
        async with cursor:
            rows = await cursor.fetchall()
            all_tags = set()
            for row in rows:
                tags = [t.strip() for t in row[0].split(',') if t.strip()]
                all_tags.update(tags)
            return sorted(list(all_tags))


async def get_tracked_groups_paginated(page: int = 1, size: int = 20, tenant_id: Optional[str] = None):
    """分页获取群组列表；tenant scope 下自动过滤 tenant_id。"""
    offset = (page - 1) * size
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            async with db.execute("SELECT COUNT(*) FROM tracked_groups WHERE tenant_id = ?", (normalized_tenant,)) as cursor:
                total = (await cursor.fetchone())[0]
            cursor = await db.execute(
                "SELECT * FROM tracked_groups WHERE tenant_id = ? ORDER BY updated_at DESC LIMIT ? OFFSET ?",
                (normalized_tenant, size, offset)
            )
        else:
            async with db.execute("SELECT COUNT(*) FROM tracked_groups") as cursor:
                total = (await cursor.fetchone())[0]
            cursor = await db.execute(
                "SELECT * FROM tracked_groups ORDER BY updated_at DESC LIMIT ? OFFSET ?",
                (size, offset)
            )
        async with cursor:
            rows = await cursor.fetchall()
            return {"groups": [dict(row) for row in rows], "total": total}


async def is_group_listen_enabled(group_id: int) -> bool:
    """检查群组监听是否开启"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT is_monitored FROM tracked_groups WHERE group_id = ?", (group_id,)) as cursor:
            row = await cursor.fetchone()
            return bool(row[0]) if row else False


async def set_group_listen_status(group_id: int, is_monitored: bool, tenant_id: Optional[str] = None):
    """开启或关闭群组监听；tenant scope 下强制 group_id+tenant_id 联合命中。"""
    async with aiosqlite.connect(DB_NAME) as db:
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            await db.execute(
                "UPDATE tracked_groups SET is_monitored = ? WHERE group_id = ? AND tenant_id = ?",
                (1 if is_monitored else 0, group_id, normalized_tenant)
            )
        else:
            await db.execute(
                "UPDATE tracked_groups SET is_monitored = ? WHERE group_id = ?",
                (1 if is_monitored else 0, group_id)
            )
        await db.commit()


async def get_tracked_groups_with_stats(tenant_id: Optional[str] = None):
    """获取所有追踪群组及其详细监测状态，支持租户隔离"""
    from datetime import datetime, timedelta
    now = datetime.now()
    online_threshold = now - timedelta(minutes=10)
    
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        if tenant_id is not None:
            async with db.execute("SELECT * FROM tracked_groups WHERE tenant_id = ? ORDER BY updated_at DESC", (tenant_id,)) as cursor:
                rows = await cursor.fetchall()
        else:
            async with db.execute("SELECT * FROM tracked_groups ORDER BY updated_at DESC") as cursor:
                rows = await cursor.fetchall()
        groups = []
        for row in rows:
            try:
                g = dict(row)
                # 计算在线状态
                updated_at_str = g.get('updated_at', '')
                if updated_at_str:
                    try:
                        updated_at_dt = datetime.fromisoformat(updated_at_str.replace(' ', 'T'))
                    except (ValueError, AttributeError):
                        updated_at_dt = now
                    g['is_online'] = updated_at_dt > online_threshold
                else:
                    g['is_online'] = False
                # 兼容旧字段
                g['name'] = g['group_title']
                g['listen_status'] = bool(g['is_monitored'])
                groups.append(g)
            except Exception as e:
                continue
        return groups


async def update_group_custom_keywords(group_id: int, keywords: str, tenant_id: Optional[str] = None):
    """更新群组自定义关键词；tenant scope 下强制 group_id+tenant_id 联合命中。"""
    async with aiosqlite.connect(DB_NAME) as db:
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            await db.execute(
                "UPDATE tracked_groups SET custom_keywords = ? WHERE group_id = ? AND tenant_id = ?",
                (keywords, group_id, normalized_tenant)
            )
        else:
            await db.execute(
                "UPDATE tracked_groups SET custom_keywords = ? WHERE group_id = ?",
                (keywords, group_id)
            )
        await db.commit()


async def get_group_delete_on_revoke(group_id: int) -> bool:
    """获取群组撤回同步删除开关"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT delete_on_revoke FROM tracked_groups WHERE group_id = ?", (group_id,)
        ) as cursor:
            row = await cursor.fetchone()
            return bool(row[0]) if row else False


async def get_group_revoke_sync_chat_id(group_id: int):
    """获取群组绑定的校对信息群 ID"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT revoke_sync_chat_id FROM tracked_groups WHERE group_id = ?", (group_id,)
        ) as cursor:
            row = await cursor.fetchone()
            return int(row[0]) if row and row[0] is not None else None


async def set_group_revoke_sync_chat_id(group_id: int, chat_id: Optional[int]):
    """为指定群组设置校对信息群 ID"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE tracked_groups SET revoke_sync_chat_id = ? WHERE group_id = ?",
            (int(chat_id) if chat_id is not None else None, group_id)
        )
        await db.commit()


async def get_revoke_sync_assigned_groups(chat_id: int) -> list:
    """获取已分配到某个校对信息群的群组列表"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
            SELECT group_id, group_title, is_monitored, delete_on_revoke, revoke_sync_chat_id, updated_at
            FROM tracked_groups
            WHERE revoke_sync_chat_id = ?
            ORDER BY updated_at DESC, group_id DESC
            """,
            (chat_id,),
        ) as cursor:
            return [dict(row) for row in await cursor.fetchall()]


async def save_revoke_sync_group_assignments(chat_id: int, group_ids: list[int]):
    """批量保存某个校对信息群的群组授权/分配关系"""
    normalized_ids = sorted({int(gid) for gid in (group_ids or []) if str(gid).strip()})
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT group_id FROM tracked_groups WHERE revoke_sync_chat_id = ?",
            (chat_id,),
        ) as cursor:
            old_group_ids = {int(row["group_id"]) for row in await cursor.fetchall()}

        removed_group_ids = sorted(old_group_ids - set(normalized_ids))
        await db.execute(
            "UPDATE tracked_groups SET revoke_sync_chat_id = NULL WHERE revoke_sync_chat_id = ?",
            (chat_id,),
        )
        if removed_group_ids:
            placeholders = ",".join("?" for _ in removed_group_ids)
            await db.execute(
                f"UPDATE tracked_groups SET delete_on_revoke = 0 WHERE group_id IN ({placeholders})",
                tuple(removed_group_ids),
            )
        if normalized_ids:
            placeholders = ",".join("?" for _ in normalized_ids)
            await db.execute(
                f"UPDATE tracked_groups SET revoke_sync_chat_id = ? WHERE group_id IN ({placeholders})",
                (chat_id, *normalized_ids),
            )
        await db.commit()


async def reassign_revoke_sync_groups(old_chat_id: Optional[int], new_chat_id: Optional[int]):
    """将旧校对信息群下的群组授权迁移到新群"""
    if old_chat_id is None:
        return
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE tracked_groups SET revoke_sync_chat_id = ? WHERE revoke_sync_chat_id = ?",
            (int(new_chat_id) if new_chat_id is not None else None, int(old_chat_id)),
        )
        await db.commit()


async def set_group_delete_on_revoke(group_id: int, enabled: bool, tenant_id: Optional[str] = None):
    """设置群组撤回同步删除开关；tenant scope 下强制 group_id+tenant_id 联合命中。"""
    async with aiosqlite.connect(DB_NAME) as db:
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            await db.execute(
                "UPDATE tracked_groups SET delete_on_revoke = ? WHERE group_id = ? AND tenant_id = ?",
                (1 if enabled else 0, group_id, normalized_tenant)
            )
        else:
            await db.execute(
                "UPDATE tracked_groups SET delete_on_revoke = ? WHERE group_id = ?",
                (1 if enabled else 0, group_id)
            )
        await db.commit()


async def get_groups_with_revoke_sync() -> list:
    """获取所有开启了信息校对的群组及其最近 48h 内未校对的消息"""
    import time
    cutoff_ts = int(time.time()) - 48 * 3600  # 48 小时前的 Unix 时间戳
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT group_id, revoke_sync_chat_id FROM tracked_groups WHERE delete_on_revoke = 1 AND is_monitored = 1 AND revoke_sync_chat_id IS NOT NULL"
        ) as cursor:
            groups = [dict(row) for row in await cursor.fetchall()]
        result = []
        for item in groups:
            gid = item['group_id']
            # 只取 48h 内、tg_msg_id 不为空、且未校对过的消息，最多 200 条
            async with db.execute(
                "SELECT msg_id, tg_msg_id FROM group_messages "
                "WHERE group_id = ? AND tg_msg_id IS NOT NULL "
                "AND COALESCE(revoke_checked, 0) = 0 "
                "AND COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) >= ? "
                "ORDER BY COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) DESC "
                "LIMIT 200",
                (gid, cutoff_ts)
            ) as cursor:
                msgs = [dict(row) for row in await cursor.fetchall()]
            if msgs:
                result.append({
                    'group_id': gid,
                    'revoke_sync_chat_id': item.get('revoke_sync_chat_id'),
                    'messages': msgs,
                })
        return result


async def mark_group_message_revoke_checked(group_id: int, tg_msg_id: int):
    """标记消息已完成撤回校对，防止重复处理"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE group_messages SET revoke_checked = 1 WHERE group_id = ? AND tg_msg_id = ?",
            (group_id, tg_msg_id),
        )
        await db.commit()


async def search_messages(keyword: str, group_id: int = None, user_id: int = None, limit: int = 30):
    """全局搜索私聊和群聊消息，返回匹配的消息片段列表"""
    results = []
    kw = f"%{keyword}%"
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        # 搜私聊消息
        if user_id is None and group_id is None or user_id is not None:
            uid_clause = "AND from_userid = ?" if user_id else ""
            uid_params = (user_id,) if user_id else ()
            async with db.execute(
                f"""SELECT msg_id, tg_msg_id, from_userid AS user_id, NULL AS group_id,
                           content, timestamp, 'chat' AS source_type
                    FROM messages
                    WHERE content LIKE ?
                    {uid_clause}
                    ORDER BY msg_id DESC LIMIT ?""",
                (kw, *uid_params, limit)
            ) as cur:
                rows = await cur.fetchall()
                results.extend([dict(r) for r in rows])
        # 搜群聊消息
        if user_id is None:
            gid_clause = "AND group_id = ?" if group_id else ""
            gid_params = (group_id,) if group_id else ()
            async with db.execute(
                f"""SELECT msg_id, tg_msg_id, user_id, group_id,
                           content, COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) AS timestamp,
                           'group' AS source_type, username
                    FROM group_messages
                    WHERE content LIKE ?
                    {gid_clause}
                    ORDER BY msg_id DESC LIMIT ?""",
                (kw, *gid_params, limit)
            ) as cur:
                rows = await cur.fetchall()
                results.extend([dict(r) for r in rows])
    # 按时间倒序合并
    results.sort(key=lambda x: x.get('timestamp') or 0, reverse=True)
    return results[:limit]


async def save_group_message(
    group_id: int,
    user_id: int,
    username: str,
    content: str,
    msg_type: str = 'text',
    file_id: Optional[str] = None,
    tg_msg_id: Optional[int] = None,
    sender_type: str = 'user',
    quoted_tg_msg_id: Optional[int] = None,
    quoted_text: Optional[str] = None,
    message_ts: Optional[int] = None,
    avatar_file_id: Optional[str] = None,
    is_advertising: Optional[bool] = None,
    ad_score: Optional[int] = None,
    ad_confidence: Optional[str] = None,
    ad_findings: Optional[Union[str, List[str]]] = None,
    tenant_id: Optional[str] = None,
):
    """保存全量群组消息记录（按 group_id + tg_msg_id 去重）"""
    ts = int(message_ts) if message_ts else None
    findings_text: Optional[str] = None
    if isinstance(ad_findings, str):
        findings_text = ad_findings
    elif isinstance(ad_findings, list):
        try:
            findings_text = json.dumps(ad_findings, ensure_ascii=False)
        except Exception:
            findings_text = None

    async with aiosqlite.connect(DB_NAME) as db:
        effective_tenant_id = _normalize_tenant_id(tenant_id) or await _resolve_tenant_id_from_group(db, group_id) or 'default'
        await db.execute(
            """
            INSERT OR IGNORE INTO group_messages (
                group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, file_id,
                quoted_tg_msg_id, quoted_text, message_ts, avatar_file_id,
                is_advertising, ad_score, ad_confidence, ad_findings, tenant_id
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, file_id,
                quoted_tg_msg_id, quoted_text, ts, avatar_file_id,
                1 if is_advertising else 0,
                int(ad_score or 0),
                (ad_confidence or "low"),
                findings_text,
                effective_tenant_id,
            )
        )
        await db.commit()


async def get_group_messages(group_id: int, limit: int = 50, offset: int = 0, tenant_id: Optional[str] = None):
    """获取指定群组的消息历史；tenant scope 下强制过滤 tenant_id。"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            query = """
                SELECT t.*,
                       COALESCE(t.avatar_file_id, gm.avatar_file_id) AS avatar_file_id
                FROM (
                    SELECT *
                    FROM group_messages
                    WHERE group_id = ?
                      AND tenant_id = ?
                    ORDER BY COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) DESC, tg_msg_id DESC, msg_id DESC
                    LIMIT ? OFFSET ?
                ) t
                LEFT JOIN (
                    SELECT user_id, avatar_file_id
                    FROM group_members
                    WHERE avatar_file_id IS NOT NULL
                      AND tenant_id = ?
                    GROUP BY user_id
                ) gm ON gm.user_id = t.user_id
                ORDER BY COALESCE(t.message_ts, CAST(strftime('%s', t.timestamp) AS INTEGER)) ASC, t.tg_msg_id ASC, t.msg_id ASC
            """
            params = (group_id, normalized_tenant, limit, offset, normalized_tenant)
        else:
            query = """
                SELECT t.*,
                       COALESCE(t.avatar_file_id, gm.avatar_file_id) AS avatar_file_id
                FROM (
                    SELECT *
                    FROM group_messages
                    WHERE group_id = ?
                    ORDER BY COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) DESC, tg_msg_id DESC, msg_id DESC
                    LIMIT ? OFFSET ?
                ) t
                LEFT JOIN (
                    SELECT user_id, avatar_file_id
                    FROM group_members
                    WHERE avatar_file_id IS NOT NULL
                    GROUP BY user_id
                ) gm ON gm.user_id = t.user_id
                ORDER BY COALESCE(t.message_ts, CAST(strftime('%s', t.timestamp) AS INTEGER)) ASC, t.tg_msg_id ASC, t.msg_id ASC
            """
            params = (group_id, limit, offset)
        async with db.execute(query, params) as cursor:
            rows = await cursor.fetchall()
            results = [dict(row) for row in rows]
            for item in results:
                raw_findings = item.get("ad_findings")
                if isinstance(raw_findings, str) and raw_findings.strip():
                    try:
                        item["ad_findings"] = json.loads(raw_findings)
                    except Exception:
                        pass
            return results


async def delete_group_message(group_id: int, tg_msg_id: int) -> bool:
    """删除群消息记录（按群ID + TG消息ID）"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "DELETE FROM group_messages WHERE group_id = ? AND tg_msg_id = ?",
            (group_id, tg_msg_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def save_group_member(user_id: int, username: Optional[str], first_name: Optional[str] = None, last_name: Optional[str] = None,
                           group_id: Optional[int] = None, group_title: Optional[str] = None,
                           tenant_id: Optional[str] = None):
    """保存群组成员信息（按 group_id + user_id 幂等更新）"""
    if group_id is None:
        return
    async with aiosqlite.connect(DB_NAME) as db:
        effective_tenant_id = _normalize_tenant_id(tenant_id) or await _resolve_tenant_id_from_group(db, group_id) or 'default'
        await db.execute(
            """INSERT INTO group_members 
               (user_id, username, first_name, last_name, group_id, group_title, tenant_id, status, joined_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, 'active', CURRENT_TIMESTAMP)
               ON CONFLICT(group_id, user_id) DO UPDATE SET
               username=excluded.username,
               first_name=excluded.first_name,
               last_name=excluded.last_name,
               group_title=excluded.group_title,
               tenant_id=excluded.tenant_id,
               status='active',
               joined_at=CURRENT_TIMESTAMP""",
            (user_id, username, first_name, last_name, group_id, group_title, effective_tenant_id)
        )
        await db.commit()


async def update_group_member_status(user_id: int, group_id: int, status: str):
    """更新群组成员状态"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE group_members SET status = ? WHERE user_id = ? AND group_id = ?",
            (status, user_id, group_id)
        )
        await db.commit()


async def delete_monitor_admin(monitor_id: int) -> bool:
    """Web 管理端：物理删除某条监听记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "DELETE FROM address_monitors WHERE monitor_id = ?",
            (monitor_id,),
        )
        await db.commit()
        return cur.rowcount > 0


async def get_potential_users(limit: int = 50, offset: int = 0, keyword: Optional[str] = None, tag: Optional[str] = None, group_id: Optional[int] = None, start_time: Optional[str] = None, end_time: Optional[str] = None, tenant_id: Optional[str] = None):
    """获取潜在用户列表 (支持分页与画像/关键词过滤 + tenant scope)"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        where_clauses = []
        params = []
        normalized_tenant = _normalize_tenant_id(tenant_id)

        if normalized_tenant:
            where_clauses.append("tenant_id = ?")
            params.append(normalized_tenant)
        
        if keyword:
            where_clauses.append("(keywords LIKE ? OR username LIKE ? OR first_name LIKE ? OR last_name LIKE ?)")
            kw_param = f"%{keyword}%"
            params.extend([kw_param, kw_param, kw_param, kw_param])
        
        if tag:
            where_clauses.append("profile_tags LIKE ?")
            params.append(f"%{tag}%")
        
        if group_id:
            where_clauses.append("group_id = ?")
            params.append(group_id)
            
        if start_time:
            where_clauses.append("captured_at >= ?")
            params.append(start_time)
            
        if end_time:
            where_clauses.append("captured_at <= ?")
            params.append(end_time)
            
        where_sql = " WHERE " + " AND ".join(where_clauses) if where_clauses else ""
        
        # 获取总数
        count_sql = f"SELECT COUNT(*) FROM potential_users{where_sql}"
        async with db.execute(count_sql, params) as cursor:
            row = await cursor.fetchone()
            total = row[0] if row else 0
            
        # 获取数据
        query_sql = f"SELECT * FROM potential_users{where_sql} ORDER BY captured_at DESC LIMIT ? OFFSET ?"
        params.extend([limit, offset])
        
        async with db.execute(query_sql, params) as cursor:
            rows = await cursor.fetchall()
            return {"total": total, "users": [dict(row) for row in rows]}


async def promote_potential_user_to_member(user_id: int) -> bool:
    """将潜在用户转正：加入正式用户表并记录系统日志"""
    async with aiosqlite.connect(DB_NAME) as db:
        # 1. 获取潜在用户信息
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM potential_users WHERE user_id = ?", (user_id,)) as cursor:
            p_user = await cursor.fetchone()
        
        if not p_user:
            return False
            
        # 2. 插入或更新到 users 表
        await db.execute(
            "INSERT OR IGNORE INTO users (user_id, created_at) VALUES (?, CURRENT_TIMESTAMP)",
            (user_id,)
        )
        
        # 3. 在 admin_inbox 创建记录，使其出现在会话列表
        await db.execute(
            """INSERT OR IGNORE INTO admin_inbox 
               (from_userid, from_username, summary, unread_count, last_active) 
               VALUES (?, ?, ?, 0, CURRENT_TIMESTAMP)""",
            (user_id, p_user['username'] or p_user['first_name'] or str(user_id), "[系统: 潜在用户转正]")
        )
        
        # 4. 从 potential_users 移除 (或者标记为已转正，这里选择移除以保持列表整洁)
        await db.execute("DELETE FROM potential_users WHERE user_id = ?", (user_id,))
        
        await db.commit()
        return True


async def get_group_members(group_id: int):
    """获取群组成员列表（严格按 group_id）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM group_members WHERE group_id = ? AND status = 'active' ORDER BY joined_at DESC, id DESC",
            (group_id,)
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


async def get_group_members_live(group_id: int, bot=None):
    """实时获取群成员（优先 Telegram API，失败回退缓存）"""
    # bot 由调用方传入，避免循环导入

    members: List[Dict[str, Any]] = []

    async def _append_from_cached() -> None:
        cached = await get_group_members(group_id)
        seen_local = {m.get("user_id") for m in members}
        for m in cached:
            uid = m.get("user_id")
            if uid in seen_local:
                continue
            members.append({
                "user_id": uid,
                "username": m.get("username"),
                "first_name": m.get("first_name"),
                "last_name": m.get("last_name"),
                "status": m.get("status") or "member",
                "is_bot": False,
            })
            seen_local.add(uid)

    async def _append_from_group_messages() -> None:
        """兜底：从已记录群消息提取最近活跃发言者"""
        seen_local = {m.get("user_id") for m in members}
        async with aiosqlite.connect(DB_NAME) as db:
            db.row_factory = aiosqlite.Row
            async with db.execute(
                """
                SELECT user_id,
                       MAX(COALESCE(NULLIF(username, ''), '')) AS username
                FROM group_messages
                WHERE group_id = ?
                  AND user_id > 0
                  AND COALESCE(sender_type, 'user') IN ('user', 'member')
                GROUP BY user_id
                ORDER BY MAX(COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER))) DESC
                LIMIT 300
                """,
                (group_id,)
            ) as cursor:
                rows = await cursor.fetchall()

        for r in rows:
            uid = r["user_id"]
            if uid in seen_local:
                continue
            username = r["username"]
            members.append({
                "user_id": uid,
                "username": username,
                "first_name": None,
                "last_name": None,
                "status": "member",
                "is_bot": False,
            })
            await save_group_member(
                user_id=uid,
                username=username or f"User{uid}",
                first_name=None,
                last_name=None,
                group_id=group_id,
                group_title=None,
            )
            seen_local.add(uid)

    try:
        admins = await bot.get_chat_administrators(group_id) if bot else []
        seen = set()
        for admin in admins:
            u = admin.user
            seen.add(u.id)
            members.append({
                "user_id": u.id,
                "username": u.username,
                "first_name": u.first_name,
                "last_name": u.last_name,
                "status": "admin",
                "is_bot": bool(u.is_bot),
            })

        await _append_from_cached()
        await _append_from_group_messages()
        return members
    except Exception:
        await _append_from_cached()
        await _append_from_group_messages()
        return members



async def get_broadcast_target_tenant_id(chat_id: int) -> Optional[str]:
    """获取广播目标当前归属租户"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT tenant_id FROM broadcast_targets WHERE chat_id = ?",
            (chat_id,),
        ) as cursor:
            row = await cursor.fetchone()
            if not row:
                return None
            tenant_id = row[0]
            return str(tenant_id).strip() if tenant_id is not None else None


async def upsert_broadcast_target(
    chat_id: int,
    chat_title: str,
    chat_type: str,
    chat_username: Optional[str] = None,
    can_broadcast: bool = True,
    tenant_id: Optional[str] = None,
):
    """登记/更新广播目标（群组、超级群、频道），支持租户隔离"""
    effective_tenant = (tenant_id or '').strip()
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT tenant_id FROM broadcast_targets WHERE chat_id = ?",
            (chat_id,),
        ) as cursor:
            existing = await cursor.fetchone()
        if existing and str(existing['tenant_id'] or '').strip():
            effective_tenant = str(existing['tenant_id']).strip()
        if not effective_tenant:
            effective_tenant = 'default'
        await db.execute(
            """
            INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, tenant_id, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(chat_id) DO UPDATE SET
                chat_title=excluded.chat_title,
                chat_type=excluded.chat_type,
                chat_username=excluded.chat_username,
                can_broadcast=excluded.can_broadcast,
                tenant_id=excluded.tenant_id,
                updated_at=CURRENT_TIMESTAMP
            """,
            (chat_id, chat_title, chat_type, chat_username, 1 if can_broadcast else 0, effective_tenant),
        )
        await db.commit()


async def get_broadcast_targets(chat_types: Optional[List[str]] = None, enabled_only: bool = True, tenant_id: Optional[str] = None):
    """获取广播目标列表（支持按类型过滤 + 租户隔离）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        where_parts = []
        params: List[Any] = []

        if tenant_id is not None:
            where_parts.append("tenant_id = ?")
            params.append(tenant_id)

        if enabled_only:
            where_parts.append("can_broadcast = 1")

        if chat_types:
            placeholders = ",".join(["?"] * len(chat_types))
            where_parts.append(f"chat_type IN ({placeholders})")
            params.extend(chat_types)

        where_sql = f"WHERE {' AND '.join(where_parts)}" if where_parts else ""
        sql = f"SELECT * FROM broadcast_targets {where_sql} ORDER BY updated_at DESC"

        async with db.execute(sql, tuple(params)) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]


async def set_broadcast_target_permission(chat_id: int, can_broadcast: bool) -> bool:
    """更新目标是否允许参与广播"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE broadcast_targets SET can_broadcast = ?, updated_at = CURRENT_TIMESTAMP WHERE chat_id = ?",
            (1 if can_broadcast else 0, chat_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def get_group_address_hit(group_id: int, address: str):
    """获取群内地址去重记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM group_address_hits WHERE group_id = ? AND address = ?",
            (group_id, address),
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def upsert_group_address_hit(group_id: int, user_id: int, address: str, network: str, source_tg_msg_id: Optional[int], query_result: Dict[str, Any]):
    """写入群内地址识别结果"""
    query_result_text = json.dumps(query_result, ensure_ascii=False)
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO group_address_hits (group_id, user_id, address, network, source_tg_msg_id, query_result, created_at)
            VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(group_id, address) DO UPDATE SET
                user_id=excluded.user_id,
                network=excluded.network,
                source_tg_msg_id=excluded.source_tg_msg_id,
                query_result=excluded.query_result,
                created_at=CURRENT_TIMESTAMP
            """,
            (group_id, user_id, address, network, source_tg_msg_id, query_result_text),
        )
        await db.commit()


async def create_push_task(task_name: str, content: str, image_file_id: Optional[str], do_translate: bool, target_chat_ids: List[int], start_at: str, interval_hours: int, status: str, next_run_at: str, tenant_id: Optional[str] = None, created_by: Optional[int] = None):
    """创建定时推送任务，支持租户隔离"""
    effective_tenant = tenant_id or 'default'
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            INSERT INTO push_tasks (task_name, content, image_file_id, do_translate, target_chat_ids, start_at, interval_hours, status, next_run_at, tenant_id, created_by, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            """,
            (task_name, content, image_file_id, 1 if do_translate else 0, json.dumps(target_chat_ids), start_at, interval_hours, status, next_run_at, effective_tenant, created_by),
        )
        await db.commit()
        return cur.lastrowid


async def list_push_tasks(tenant_id: Optional[str] = None):
    """列出推送任务，支持租户隔离"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        if tenant_id is not None:
            async with db.execute("SELECT * FROM push_tasks WHERE tenant_id = ? ORDER BY created_at DESC", (tenant_id,)) as cursor:
                rows = await cursor.fetchall()
        else:
            async with db.execute("SELECT * FROM push_tasks ORDER BY created_at DESC") as cursor:
                rows = await cursor.fetchall()
            result = []
            for row in rows:
                item = dict(row)
                try:
                    item['target_chat_ids'] = json.loads(item.get('target_chat_ids') or '[]')
                except Exception:
                    item['target_chat_ids'] = []
                result.append(item)
            return result


async def get_due_push_tasks(now_iso: str):
    """获取到期待执行任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM push_tasks WHERE status = 'running' AND next_run_at IS NOT NULL AND next_run_at <= ? ORDER BY next_run_at ASC",
            (now_iso,),
        ) as cursor:
            rows = await cursor.fetchall()
            result = []
            for row in rows:
                item = dict(row)
                try:
                    item['target_chat_ids'] = json.loads(item.get('target_chat_ids') or '[]')
                except Exception:
                    item['target_chat_ids'] = []
                result.append(item)
            return result


async def update_push_task_status(task_id: int, status: str):
    """更新任务状态"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE push_tasks SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE task_id = ?",
            (status, task_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def update_push_task_next_run(task_id: int, next_run_at: str):
    """更新任务下次执行时间"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "UPDATE push_tasks SET next_run_at = ?, updated_at = CURRENT_TIMESTAMP WHERE task_id = ?",
            (next_run_at, task_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def update_push_task_run_result(task_id: int, last_run_at: str, next_run_at: str, summary: Dict[str, Any], retry_count: int = 0):
    """更新任务执行结果"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            UPDATE push_tasks
            SET last_run_at = ?,
                next_run_at = ?,
                last_result = ?,
                last_summary = ?,
                retry_count = ?,
                updated_at = CURRENT_TIMESTAMP
            WHERE task_id = ?
            """,
            (
                last_run_at,
                next_run_at,
                'ok' if (summary.get('failed', 0) == 0) else 'partial',
                json.dumps(summary, ensure_ascii=False),
                retry_count,
                task_id,
            ),
        )
        await db.commit()
        return cur.rowcount > 0


# ─────────────────────────────────────────────────────────────
# 入群验证 (Join Verify)
# ─────────────────────────────────────────────────────────────

async def get_group_verify_config(group_id: int, tenant_id: Optional[str] = None) -> dict:
    """获取群组入群验证配置；tenant scope 下强制 group_id+tenant_id 联合命中。"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        normalized_tenant = _normalize_tenant_id(tenant_id)
        if normalized_tenant:
            cursor = await db.execute(
                "SELECT verify_enabled, verify_type, verify_timeout, verify_channel_id, verify_questions, verify_msg_template FROM tracked_groups WHERE group_id = ? AND tenant_id = ?",
                (group_id, normalized_tenant)
            )
        else:
            cursor = await db.execute(
                "SELECT verify_enabled, verify_type, verify_timeout, verify_channel_id, verify_questions, verify_msg_template FROM tracked_groups WHERE group_id = ?",
                (group_id,)
            )
        async with cursor:
            row = await cursor.fetchone()
            if not row:
                return {"verify_enabled": False, "verify_type": "captcha", "verify_timeout": 120, "verify_channel_id": None, "verify_questions": None, "verify_msg_template": None}
            return {
                "verify_enabled": bool(row["verify_enabled"]),
                "verify_type": row["verify_type"] or "captcha",
                "verify_timeout": row["verify_timeout"] or 120,
                "verify_channel_id": row["verify_channel_id"],
                "verify_questions": json.loads(row["verify_questions"]) if row["verify_questions"] else [],
                "verify_msg_template": row["verify_msg_template"],
            }

async def save_group_verify_config(group_id: int, config: dict, tenant_id: Optional[str] = None) -> None:
    """保存群组入群验证配置；tenant scope 下强制 group_id+tenant_id 联合命中。"""
    async with aiosqlite.connect(DB_NAME) as db:
        normalized_tenant = _normalize_tenant_id(tenant_id)
        params = [
            1 if config.get("verify_enabled") else 0,
            config.get("verify_type", "captcha"),
            config.get("verify_timeout", 120),
            config.get("verify_channel_id"),
            json.dumps(config.get("verify_questions", []), ensure_ascii=False) if config.get("verify_questions") else None,
            config.get("verify_msg_template"),
            group_id,
        ]
        sql = """UPDATE tracked_groups SET
            verify_enabled = ?, verify_type = ?, verify_timeout = ?,
            verify_channel_id = ?, verify_questions = ?, verify_msg_template = ?
           WHERE group_id = ?"""
        if normalized_tenant:
            sql += " AND tenant_id = ?"
            params.append(normalized_tenant)
        await db.execute(sql, tuple(params))
        await db.commit()


async def get_member_verified(group_id: int, user_id: int) -> bool:
    """检查成员是否已通过频道验证"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT is_verified FROM group_members WHERE group_id=? AND user_id=?",
            (group_id, user_id)
        ) as cursor:
            row = await cursor.fetchone()
            return bool(row[0]) if row else False


async def set_member_verified(group_id: int, user_id: int) -> None:
    """标记成员为已验证"""
    from datetime import datetime
    now = datetime.utcnow().strftime("%Y-%m-%d %H:%M:%S")
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """INSERT INTO group_members (group_id, user_id, is_verified, verified_at)
               VALUES (?, ?, 1, ?)
               ON CONFLICT(group_id, user_id) DO UPDATE SET is_verified=1, verified_at=?""",
            (group_id, user_id, now, now)
        )
        await db.commit()


async def get_verify_stats(group_id: int) -> dict:
    """获取群内验证统计：已验证 vs 待验证人数"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE group_id=? AND is_verified=1", (group_id,)
        ) as cur:
            verified = (await cur.fetchone())[0]
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE group_id=? AND (is_verified=0 OR is_verified IS NULL)", (group_id,)
        ) as cur:
            unverified = (await cur.fetchone())[0]
    return {"verified": verified, "unverified": unverified, "total": verified + unverified}


async def get_verify_growth(group_id: Optional[int] = None, period: str = 'day') -> list:
    """按天/小时统计通过频道验证的用户数"""
    if period == 'hour':
        fmt = "%Y-%m-%d %H:00"
        trunc = "strftime('%Y-%m-%d %H:00', verified_at)"
    else:
        fmt = "%Y-%m-%d"
        trunc = "strftime('%Y-%m-%d', verified_at)"
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        if group_id:
            sql = f"SELECT {trunc} as period, COUNT(*) as count FROM group_members WHERE group_id=? AND is_verified=1 AND verified_at IS NOT NULL GROUP BY period ORDER BY period DESC LIMIT 30"
            params = (group_id,)
        else:
            sql = f"SELECT {trunc} as period, COUNT(*) as count FROM group_members WHERE is_verified=1 AND verified_at IS NOT NULL GROUP BY period ORDER BY period DESC LIMIT 30"
            params = ()
        async with db.execute(sql, params) as cursor:
            rows = await cursor.fetchall()
            return [{"period": r["period"], "count": r["count"]} for r in rows]


async def get_verify_growth_summary() -> dict:
    """获取今日 & 总计引导关注数"""
    from datetime import datetime
    today = datetime.utcnow().strftime("%Y-%m-%d")
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE is_verified=1 AND date(verified_at)=?", (today,)
        ) as cur:
            today_count = (await cur.fetchone())[0]
        async with db.execute(
            "SELECT COUNT(*) FROM group_members WHERE is_verified=1"
        ) as cur:
            total_count = (await cur.fetchone())[0]
    return {"today": today_count, "total": total_count}


async def create_verify_session(group_id: int, user_id: int, username: str, first_name: str,
                                 verify_type: str, answer: str, bot_msg_id: int, timeout_secs: int) -> int:
    """创建验证会话，返回 session_id"""
    from datetime import datetime, timedelta
    expires_at = (datetime.utcnow() + timedelta(seconds=timeout_secs)).strftime("%Y-%m-%d %H:%M:%S")
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """INSERT INTO join_verify_sessions
               (group_id, user_id, username, first_name, verify_type, answer, bot_msg_id, status, expires_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?)""",
            (group_id, user_id, username, first_name, verify_type, answer, bot_msg_id, expires_at)
        )
        await db.commit()
        return cur.lastrowid


async def get_pending_verify_session(group_id: int, user_id: int) -> Optional[dict]:
    """获取用户待验证会话"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM join_verify_sessions WHERE group_id=? AND user_id=? AND status='pending' ORDER BY created_at DESC LIMIT 1",
            (group_id, user_id)
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def update_verify_session_status(session_id: int, status: str) -> None:
    """更新验证会话状态: passed / failed / expired"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE join_verify_sessions SET status=? WHERE session_id=?",
            (status, session_id)
        )
        await db.commit()


async def get_expired_verify_sessions() -> list:
    """获取所有已超时的待验证会话"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM join_verify_sessions WHERE status='pending' AND expires_at < datetime('now')"
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(r) for r in rows]




# ============ 已验证用户相关函数 (v5.1+) ============

async def get_verified_users(group_id: int, period: str = 'all') -> list:
    """
    查询群组中已验证的用户列表
    
    Args:
        group_id: 群组 ID
        period: 'today' 或 'all'
    
    Returns:
        已验证用户列表，包含 user_id, username, first_name, avatar_file_id, verified_at
    """
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if period == 'today':
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at
                FROM group_members
                WHERE group_id = ?
                  AND is_verified = 1
                  AND DATE(verified_at) = DATE('now')
                ORDER BY verified_at DESC
            '''
        else:
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at
                FROM group_members
                WHERE group_id = ?
                  AND is_verified = 1
                ORDER BY verified_at DESC
            '''
        
        cursor = await db.execute(query, (group_id,))
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


async def get_verify_growth_summary(group_id: int = None) -> dict:
    """获取验证增长统计摘要"""
    async with aiosqlite.connect(DB_NAME) as db:
        if group_id:
            cursor = await db.execute('''
                SELECT 
                    COUNT(CASE WHEN DATE(verified_at) = DATE('now') THEN 1 END) as today,
                    COUNT(*) as total
                FROM group_members
                WHERE group_id = ? AND is_verified = 1
            ''', (group_id,))
        else:
            cursor = await db.execute('''
                SELECT 
                    COUNT(CASE WHEN DATE(verified_at) = DATE('now') THEN 1 END) as today,
                    COUNT(*) as total
                FROM group_members
                WHERE is_verified = 1
            ''')
        
        row = await cursor.fetchone()
        return {'today': row[0] if row else 0, 'total': row[1] if row else 0}


async def get_all_verified_users(period: str = 'all') -> list:
    """
    查询所有群组中已验证的用户列表（全局统计）
    
    Args:
        period: 'today' 或 'all'
    
    Returns:
        已验证用户列表，包含 user_id, username, first_name, avatar_file_id, verified_at, group_id, group_title
    """
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if period == 'today':
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at,
                    group_id,
                    group_title
                FROM group_members
                WHERE is_verified = 1
                  AND DATE(verified_at) = DATE('now')
                ORDER BY verified_at DESC
            '''
        else:
            query = '''
                SELECT 
                    user_id,
                    username,
                    first_name,
                    avatar_file_id,
                    verified_at,
                    group_id,
                    group_title
                FROM group_members
                WHERE is_verified = 1
                ORDER BY verified_at DESC
            '''
        
        cursor = await db.execute(query)
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


# ============ 未关注频道用户提醒功能 (v5.3+) ============

async def get_unverified_users(group_id: int = None, period: str = 'all') -> list:
    """
    查询未关注频道的用户列表
    
    Args:
        group_id: 群组 ID（可选，不传则返回全局）
        period: 'today' 或 'all'
    
    Returns:
        未关注用户列表
    """
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        
        if period == 'today':
            if group_id:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE group_id = ? AND is_verified = 0
                      AND DATE(joined_at) = DATE('now')
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query, (group_id,))
            else:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE is_verified = 0
                      AND DATE(joined_at) = DATE('now')
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query)
        else:
            if group_id:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE group_id = ? AND is_verified = 0
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query, (group_id,))
            else:
                query = '''
                    SELECT 
                        user_id, username, first_name, avatar_file_id,
                        group_id, group_title, joined_at
                    FROM group_members
                    WHERE is_verified = 0
                    ORDER BY joined_at DESC
                '''
                cursor = await db.execute(query)
        
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


async def save_channel_reminder(group_id: int, user_id: int, message_id: int, expires_at: str) -> int:
    """
    保存频道关注提醒记录
    
    Args:
        group_id: 群组 ID
        user_id: 用户 ID
        message_id: TG 消息 ID
        expires_at: 过期时间
    
    Returns:
        记录 ID
    """
    async with aiosqlite.connect(DB_NAME) as db:
        cursor = await db.execute('''
            INSERT INTO channel_follow_reminders 
            (group_id, user_id, message_id, sent_at, expires_at, is_deleted)
            VALUES (?, ?, ?, CURRENT_TIMESTAMP, ?, 0)
        ''', (group_id, user_id, message_id, expires_at))
        await db.commit()
        return cursor.lastrowid


async def get_expired_reminders() -> list:
    """获取已过期的提醒记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute('''
            SELECT * FROM channel_follow_reminders
            WHERE is_deleted = 0 AND expires_at < CURRENT_TIMESTAMP
        ''')
        rows = await cursor.fetchall()
        return [dict(row) for row in rows]


async def mark_reminder_deleted(reminder_id: int) -> None:
    """标记提醒为已删除"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute('''
            UPDATE channel_follow_reminders
            SET is_deleted = 1
            WHERE id = ?
        ''', (reminder_id,))
        await db.commit()


async def get_reminder_by_user_group(user_id: int, group_id: int) -> dict:
    """获取用户在特定群组的提醒记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute('''
            SELECT * FROM channel_follow_reminders
            WHERE user_id = ? AND group_id = ? AND is_deleted = 0
            ORDER BY sent_at DESC LIMIT 1
        ''', (user_id, group_id))
        row = await cursor.fetchone()
        return dict(row) if row else None


async def get_group_member(group_id: int, user_id: int) -> dict:
    """
    获取群组成员信息
    
    Args:
        group_id: 群组 ID
        user_id: 用户 ID
    
    Returns:
        成员信息字典，包含 username, first_name 等
    """
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute('''
            SELECT user_id, username, first_name, last_name, avatar_file_id
            FROM group_members
            WHERE group_id = ? AND user_id = ?
            LIMIT 1
        ''', (group_id, user_id))
        row = await cursor.fetchone()
        return dict(row) if row else None


async def _ensure_finance_exchange_rate_table(db: aiosqlite.Connection) -> None:
    await db.execute('''
        CREATE TABLE IF NOT EXISTS finance_exchange_rates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token_symbol TEXT NOT NULL,
            fiat_currency TEXT NOT NULL,
            rate REAL NOT NULL,
            rate_timestamp INTEGER NOT NULL,
            source TEXT DEFAULT 'binance',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )
    ''')
    await db.execute('''
        CREATE INDEX IF NOT EXISTS idx_finance_exchange_rates_key
        ON finance_exchange_rates(token_symbol, fiat_currency, rate_timestamp)
    ''')


async def _fetch_binance_spot_rate(token_symbol: str, fiat_currency: str) -> Optional[float]:
    token = (token_symbol or '').upper()
    fiat = (fiat_currency or 'USD').upper()

    if token in ('USD', 'USDT', 'USDC') and fiat in ('USD', 'USDT'):
        return 1.0

    quote = 'USDT' if fiat in ('USD', 'USDT') else fiat
    symbol = f"{token}{quote}"
    url = f"https://api.binance.com/api/v3/ticker/price?symbol={symbol}"

    try:
        timeout = aiohttp.ClientTimeout(total=5)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.get(url) as resp:
                if resp.status != 200:
                    return None
                payload = await resp.json()
                price = payload.get('price')
                return float(price) if price is not None else None
    except Exception:
        return None


async def _get_or_fill_exchange_rate(
    db: aiosqlite.Connection,
    token_symbol: str,
    fiat_currency: str,
    tx_timestamp: Optional[int],
    ledger_exchange_rate: Optional[float] = None,
) -> float:
    token = (token_symbol or '').upper()
    fiat = (fiat_currency or 'USD').upper()
    ts = int(tx_timestamp or datetime.now().timestamp())

    if ledger_exchange_rate and float(ledger_exchange_rate) > 0:
        return float(ledger_exchange_rate)

    cursor = await db.execute(
        '''
        SELECT rate
        FROM finance_exchange_rates
        WHERE token_symbol = ? AND fiat_currency = ? AND rate_timestamp <= ?
        ORDER BY rate_timestamp DESC
        LIMIT 1
        ''',
        (token, fiat, ts),
    )
    row = await cursor.fetchone()
    if row and row[0] is not None:
        return float(row[0])

    live_rate = await _fetch_binance_spot_rate(token, fiat)
    if live_rate is None:
        live_rate = 1.0 if token == fiat else 0.0

    await db.execute(
        '''
        INSERT INTO finance_exchange_rates(token_symbol, fiat_currency, rate, rate_timestamp, source)
        VALUES (?, ?, ?, ?, 'binance')
        ''',
        (token, fiat, live_rate, ts),
    )
    return float(live_rate)


async def get_finance_summary_by_currency(user_id: int) -> Dict[str, Any]:
    """多币种汇总：按 network, token_symbol 聚合统计。"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute(
            '''
            SELECT
                COALESCE(m.network, 'unknown') AS network,
                l.token_symbol AS token_symbol,
                COUNT(1) AS tx_count,
                SUM(CASE WHEN l.direction = 'in' THEN CAST(l.amount AS REAL) ELSE 0 END) AS total_in,
                SUM(CASE WHEN l.direction = 'out' THEN CAST(l.amount AS REAL) ELSE 0 END) AS total_out,
                SUM(CASE WHEN l.reconciled_status = 'UNRECONCILED' THEN 1 ELSE 0 END) AS unreconciled_count
            FROM finance_ledgers l
            LEFT JOIN address_monitors m ON m.monitor_id = l.monitor_id
            WHERE l.user_id = ?
            GROUP BY COALESCE(m.network, 'unknown'), l.token_symbol
            ORDER BY network, token_symbol
            ''',
            (user_id,),
        )
        rows = [dict(r) for r in await cursor.fetchall()]

        total_unreconciled = sum(int(r.get('unreconciled_count') or 0) for r in rows)
        return {
            'by_currency': rows,
            'total_unreconciled': total_unreconciled,
        }


async def get_finance_unreconciled_with_fx(user_id: int, monitor_id: Optional[int] = None) -> List[Dict[str, Any]]:
    """未对账流水：按交易时间匹配历史汇率，缺失时调用币安补全。"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        await _ensure_finance_exchange_rate_table(db)

        query = '''
            SELECT
                l.ledger_id,
                l.monitor_id,
                l.direction,
                l.amount,
                l.token_symbol,
                l.base_fiat_value,
                l.fiat_currency,
                l.exchange_rate,
                l.counterparty,
                l.tx_hash,
                l.tx_timestamp,
                l.created_at,
                l.reconciled_status,
                COALESCE(m.network, 'unknown') AS network,
                a.code,
                a.name
            FROM finance_ledgers l
            LEFT JOIN address_monitors m ON m.monitor_id = l.monitor_id
            LEFT JOIN chart_of_accounts a ON a.account_id = l.account_id
            WHERE l.user_id = ? AND l.reconciled_status = 'UNRECONCILED'
        '''
        params: List[Any] = [user_id]

        if monitor_id is not None:
            query += ' AND l.monitor_id = ?'
            params.append(monitor_id)

        query += ' ORDER BY l.tx_timestamp DESC LIMIT 500'

        cursor = await db.execute(query, params)
        rows = [dict(r) for r in await cursor.fetchall()]

        for row in rows:
            rate = await _get_or_fill_exchange_rate(
                db,
                token_symbol=row.get('token_symbol') or '',
                fiat_currency=row.get('fiat_currency') or 'USD',
                tx_timestamp=row.get('tx_timestamp'),
                ledger_exchange_rate=row.get('exchange_rate'),
            )
            amount = float(row.get('amount') or 0)
            fiat_value = float(row.get('base_fiat_value') or 0)
            if fiat_value <= 0 and rate > 0:
                fiat_value = round(amount * rate, 8)

            row['exchange_rate_used'] = rate
            row['fiat_value'] = fiat_value

        await db.commit()
        return rows


async def reconcile_finance_ledger(
    ledger_id: str,
    account_code: str,
    admin_userid: int,
    amount: Optional[float] = None,
    ip_address: Optional[str] = None,
) -> bool:
    """按 ledger_id 执行对账并记录审计日志。"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        await db.execute('BEGIN IMMEDIATE')
        try:
            db.row_factory = aiosqlite.Row
            cursor = await db.execute(
                'SELECT ledger_id, user_id, reconciled_status FROM finance_ledgers WHERE ledger_id = ?',
                (ledger_id,),
            )
            ledger = await cursor.fetchone()
            if not ledger or int(ledger['user_id']) != int(admin_userid):
                await db.execute('ROLLBACK')
                return False

            if ledger['reconciled_status'] == 'RECONCILED':
                await db.execute('ROLLBACK')
                return True

            cursor = await db.execute(
                'SELECT account_id FROM chart_of_accounts WHERE code = ? AND is_active = 1',
                (account_code,),
            )
            account = await cursor.fetchone()
            if not account:
                await db.execute('ROLLBACK')
                raise ValueError(f'会计科目不存在: {account_code}')

            await db.execute(
                '''
                UPDATE finance_ledgers
                SET account_id = ?, reconciled_status = 'RECONCILED', updated_at = CURRENT_TIMESTAMP
                WHERE ledger_id = ?
                ''',
                (int(account['account_id']), ledger_id),
            )

            await db.execute(
                '''
                INSERT INTO audit_logs(admin_userid, action, target_table, target_id, reason, ip_address)
                VALUES (?, 'RECONCILE', 'finance_ledgers', ?, ?, ?)
                ''',
                (admin_userid, ledger_id, f'对账至科目 {account_code}' + (f' 金额 {amount}' if amount is not None else ''), ip_address),
            )

            await db.execute('COMMIT')
            return True
        except Exception:
            await db.execute('ROLLBACK')
            raise


async def get_finance_category_tree() -> List[Dict[str, Any]]:
    """获取会计科目树。"""
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        db.row_factory = aiosqlite.Row
        cursor = await db.execute(
            '''
            SELECT account_id, code, name, type, parent_id, description
            FROM chart_of_accounts
            WHERE is_active = 1
            ORDER BY code
            '''
        )
        rows = [dict(r) for r in await cursor.fetchall()]

    node_map: Dict[int, Dict[str, Any]] = {}
    roots: List[Dict[str, Any]] = []

    for r in rows:
        node_map[int(r['account_id'])] = {
            'id': r['account_id'],
            'code': r['code'],
            'name': r['name'],
            'type': r['type'],
            'parent_id': r['parent_id'],
            'description': r.get('description'),
            'children': [],
        }

    for r in rows:
        node = node_map[int(r['account_id'])]
        parent_id = r.get('parent_id')
        if parent_id and int(parent_id) in node_map:
            node_map[int(parent_id)]['children'].append(node)
        else:
            roots.append(node)

    return roots

# ============ 财务 ERP 模块初始化 ============

async def init_finance_module():
    """初始化财务 ERP 模块"""
    try:
        from backend.app.services.finance_core import init_finance_db, init_default_chart_of_accounts
        await init_finance_db()
        await init_default_chart_of_accounts()
        logger.info("✅ 财务 ERP 模块初始化完成")
    except Exception as e:
        logger.error(f"❌ 财务 ERP 模块初始化失败: {e}")
