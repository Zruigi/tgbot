# -*- coding: utf-8 -*-
"""
异步数据库操作模块
"""
import aiosqlite
import logging
import re
import json
from datetime import datetime, timedelta
import calendar
from typing import Optional, List, Dict, Any, Union
from config import DB_NAME, ADMIN_ID
from services.field_crypto import encrypt, decrypt, decrypt_list, decrypt_dict

logger = logging.getLogger(__name__)

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
        if 'owner_type' not in monitor_columns:
            await db.execute("ALTER TABLE address_monitors ADD COLUMN owner_type TEXT DEFAULT 'user'")
            # 将管理员自己的地址标记为 admin_own
            from config import ADMIN_ID
            await db.execute("UPDATE address_monitors SET owner_type = 'admin_own' WHERE user_id = ?", (int(ADMIN_ID),))
            logger.info("migrated address_monitors.owner_type")
        await db.commit()

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
            "global_auto_translate": "false"
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
                joined_at DATETIME DEFAULT CURRENT_TIMESTAMP
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
                message_ts INTEGER DEFAULT (strftime('%s','now'))
            )
        ''')
        
        # 检查并添加缺失的列 (group_messages 迁移)
        cursor = await db.execute("PRAGMA table_info(group_messages)")
        gm_columns = [row[1] for row in await cursor.fetchall()]
        if 'message_ts' not in gm_columns:
            await db.execute("ALTER TABLE group_messages ADD COLUMN message_ts INTEGER DEFAULT NULL")
            await db.execute("UPDATE group_messages SET message_ts = CAST(strftime('%s', timestamp) AS INTEGER)")
            logger.info("✅ 数据库迁移：已为 group_messages 添加 message_ts 字段并初始化数据")

        # 为 group_id 和 timestamp 创建索引以优化查询
        await db.execute("CREATE INDEX IF NOT EXISTS idx_group_msg_query ON group_messages(group_id, timestamp DESC)")
        await db.execute("CREATE INDEX IF NOT EXISTS idx_group_msg_ts ON group_messages(group_id, message_ts DESC)")
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

        await db.commit()
        logger.info("数据库初始化及迁移完成")

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
    trace_id: Optional[str] = None
):
    """统一保存消息，并更新管理员收件箱汇总 (2026 规格：支持多媒体、引用、翻译、实名及头像更新)"""
    import uuid
    if not trace_id:
        trace_id = str(uuid.uuid4())
    # 加密敏感字段
    content = encrypt(content)
    original_text = encrypt(original_text)
    translated_text = encrypt(translated_text)
    quoted_text = encrypt(quoted_text)
    async with aiosqlite.connect(DB_NAME) as db:
        # 1. 存入 messages 主表
        cur = await db.execute(
            """INSERT INTO messages 
               (trace_id, from_userid, to_userid, content, type, file_id, media_url, is_read, tg_msg_id, auto_translation, translated_text, original_text, admin_userid, quoted_text, quoted_msg_id, quoted_file_id, quoted_type, status) 
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (trace_id, from_userid, to_userid, content, msg_type, file_id, media_url, 1 if is_read else 0, tg_msg_id, translated_text, translated_text, original_text, admin_userid, quoted_text, quoted_msg_id, quoted_file_id, quoted_type, status)
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
            async with db.execute("SELECT content FROM messages WHERE from_userid = ? AND is_read = 0 AND is_deleted = 0 ORDER BY timestamp ASC", (from_userid,)) as cursor:
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
                update_parts = ["summary = ?", "unread_count = ?", "last_active = CURRENT_TIMESTAMP"]
                params = [full_summary, len(unread_msgs)]
                
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
                    "INSERT INTO admin_inbox (from_userid, from_username, summary, unread_count, avatar_file_id) VALUES (?, ?, ?, ?, ?)",
                    (from_userid, display_name, full_summary, len(unread_msgs), avatar_file_id)
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

async def mark_messages_as_read(from_userid: int):
    """将特定用户发来的消息标记为已读"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute("UPDATE messages SET is_read = 1 WHERE from_userid = ?", (from_userid,))
        await db.execute("UPDATE admin_inbox SET unread_count = 0, summary = '' WHERE from_userid = ?", (from_userid,))
        await db.commit()

async def get_inbox_list():
    """获取来访用户列表 (含自动重建逻辑)"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
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

async def get_chat_history(user_id: int, limit: int = 50, offset: int = 0):
    """获取与特定用户的聊天历史"""
    import config
    admin_id = config.ADMIN_ID
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
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
        async with db.execute(query, (user_id, admin_id, admin_id, user_id, limit, offset)) as cursor:
            rows = await cursor.fetchall()
            messages = [dict(row) for row in rows]
            messages.reverse() # 前端需要按时间顺序列出（旧到新），所以反转返回
            # 解密敏感字段
            _enc_fields = ['content', 'original_text', 'translated_text', 'quoted_text']
            messages = decrypt_list(messages, _enc_fields)
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
            result = [dict(row) for row in rows]
            return decrypt_list(result, ['address', 'remark'])

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
            (tx_id, monitor_id, user_id, encrypt(address), network, direction, amount, token_symbol, tx_timestamp, encrypt(counterparty), risk_level, risk_note),
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
            return decrypt_list([dict(r) for r in rows], ['address', 'counterparty'])


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
            return decrypt_list([dict(r) for r in rows], ['address', 'counterparty'])

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
            return decrypt_list([dict(row) for row in rows], ['address', 'remark'])


async def get_all_monitors_for_admin(active_only: bool = True):
    """管理端：列出管理员自己的监听地址（owner_type=admin_own）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        q = "SELECT * FROM address_monitors WHERE (owner_type = 'admin_own' OR user_id = ?)"
        params = [int(ADMIN_ID)]
        if active_only:
            q += " AND is_active = 1"
        q += " ORDER BY monitor_id DESC"
        async with db.execute(q, params) as cursor:
            rows = await cursor.fetchall()
            return decrypt_list([dict(row) for row in rows], ['address', 'remark'])


async def get_user_query_addresses(active_only: bool = True):
    """管理端：列出其他用户（非管理员）的查询地址"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        q = "SELECT am.*, u.email FROM address_monitors am LEFT JOIN users u ON am.user_id = u.user_id WHERE am.user_id != ? AND (am.owner_type IS NULL OR am.owner_type = 'user')"
        params = [int(ADMIN_ID)]
        if active_only:
            q += " AND am.is_active = 1"
        q += " ORDER BY am.monitor_id DESC"
        async with db.execute(q, params) as cursor:
            rows = await cursor.fetchall()
            return decrypt_list([dict(row) for row in rows], ['address', 'remark'])


async def claim_monitor_as_admin(monitor_id: int) -> bool:
    """将用户查询地址转为管理员自有监听地址"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE address_monitors SET owner_type = 'admin_own', user_id = ? WHERE monitor_id = ?",
            (int(ADMIN_ID), monitor_id)
        )
        await db.commit()
        return True


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
    enc_address = encrypt(address)
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute(
            "SELECT monitor_id FROM address_monitors WHERE user_id = ? AND address = ? AND network = ?",
            (user_id, enc_address, network),
        ) as cursor:
            row = await cursor.fetchone()
        if not row:
            # 兼容旧明文地址
            async with db.execute(
                "SELECT monitor_id FROM address_monitors WHERE user_id = ? AND address = ? AND network = ?",
                (user_id, address, network),
            ) as cursor:
                row = await cursor.fetchone()
        if row:
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
                (user_id, enc_address, network, remark or '', initial_timestamp),
            )
        await db.commit()
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

async def soft_delete_chat(user_id: int):
    """软删除特定用户的聊天记录"""
    async with aiosqlite.connect(DB_NAME) as db:
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
                             group_id: Optional[int] = None, group_title: Optional[str] = None):
    """保存或更新潜在用户信息 (含标签累积逻辑)"""
    async with aiosqlite.connect(DB_NAME) as db:
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
               (user_id, username, first_name, last_name, message_text, keywords, profile_tags, group_id, group_title, captured_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
               ON CONFLICT(user_id) DO UPDATE SET
               username=excluded.username,
               first_name=excluded.first_name,
               last_name=excluded.last_name,
               message_text=excluded.message_text,
               keywords=?,
               profile_tags=?,
               group_id=excluded.group_id,
               group_title=excluded.group_title,
               captured_at=CURRENT_TIMESTAMP""",
            (user_id, username, first_name, last_name, message_text, new_kws, new_tags, group_id, group_title, new_kws, new_tags)
        )
        await db.commit()


async def upsert_tracked_group(group_id: int, title: str):
    """登记或更新追踪的群组信息 (支持首次发现与更新时间)"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """INSERT INTO tracked_groups (group_id, group_title, first_seen, updated_at)
               VALUES (?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
               ON CONFLICT(group_id) DO UPDATE SET
               group_title=excluded.group_title,
               updated_at=CURRENT_TIMESTAMP""",
            (group_id, title)
        )
        await db.commit()


async def get_potential_tags():
    """从 potential_users 表中去重聚合所有标签"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT profile_tags FROM potential_users WHERE profile_tags IS NOT NULL") as cursor:
            rows = await cursor.fetchall()
            all_tags = set()
            for row in rows:
                tags = [t.strip() for t in row[0].split(',') if t.strip()]
                all_tags.update(tags)
            return sorted(list(all_tags))


async def get_tracked_groups_paginated(page: int = 1, size: int = 20):
    """分页获取群组列表"""
    offset = (page - 1) * size
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        # 获取总数
        async with db.execute("SELECT COUNT(*) FROM tracked_groups") as cursor:
            total = (await cursor.fetchone())[0]
            
        # 获取分页数据
        async with db.execute(
            "SELECT * FROM tracked_groups ORDER BY updated_at DESC LIMIT ? OFFSET ?",
            (size, offset)
        ) as cursor:
            rows = await cursor.fetchall()
            return {"groups": [dict(row) for row in rows], "total": total}


async def is_group_listen_enabled(group_id: int) -> bool:
    """检查群组监听是否开启"""
    async with aiosqlite.connect(DB_NAME) as db:
        async with db.execute("SELECT is_monitored FROM tracked_groups WHERE group_id = ?", (group_id,)) as cursor:
            row = await cursor.fetchone()
            return bool(row[0]) if row else False


async def set_group_listen_status(group_id: int, is_monitored: bool):
    """开启或关闭群组监听"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE tracked_groups SET is_monitored = ? WHERE group_id = ?",
            (1 if is_monitored else 0, group_id)
        )
        await db.commit()


async def get_tracked_groups_with_stats():
    """获取所有追踪群组及其详细监测状态"""
    from datetime import datetime, timedelta
    now = datetime.now()
    online_threshold = now - timedelta(minutes=10)
    
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM tracked_groups ORDER BY updated_at DESC") as cursor:
            rows = await cursor.fetchall()
            groups = []
            for row in rows:
                g = dict(row)
                # 计算在线状态
                updated_at_dt = datetime.fromisoformat(g['updated_at'].replace(' ', 'T'))
                g['is_online'] = updated_at_dt > online_threshold
                # 兼容旧字段
                g['name'] = g['group_title']
                g['listen_status'] = bool(g['is_monitored'])
                groups.append(g)
            return groups


async def update_group_custom_keywords(group_id: int, keywords: str):
    """更新群组自定义关键词"""
    async with aiosqlite.connect(DB_NAME) as db:
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


async def set_group_delete_on_revoke(group_id: int, enabled: bool):
    """设置群组撤回同步删除开关"""
    async with aiosqlite.connect(DB_NAME) as db:
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
            "SELECT group_id FROM tracked_groups WHERE delete_on_revoke = 1 AND is_monitored = 1"
        ) as cursor:
            groups = [row['group_id'] for row in await cursor.fetchall()]
        result = []
        for gid in groups:
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
                result.append({'group_id': gid, 'messages': msgs})
        return result


async def mark_group_message_revoke_checked(group_id: int, tg_msg_id: int):
    """标记消息已完成撤回校对，防止重复处理"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE group_messages SET revoke_checked = 1 WHERE group_id = ? AND tg_msg_id = ?",
            (group_id, tg_msg_id),
        )
        await db.commit()


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
):
    """保存全量群组消息记录（按 group_id + tg_msg_id 去重）"""
    ts = int(message_ts) if message_ts else None
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "INSERT OR IGNORE INTO group_messages (group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, file_id, quoted_tg_msg_id, quoted_text, message_ts, avatar_file_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, file_id, quoted_tg_msg_id, quoted_text, ts, avatar_file_id)
        )
        await db.commit()


async def get_group_messages(group_id: int, limit: int = 50, offset: int = 0):
    """获取指定群组的消息历史（按真实发言时间稳定排序），自动补全头像"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            """
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
            """,
            (group_id, limit, offset)
        ) as cursor:
            rows = await cursor.fetchall()
            return [dict(row) for row in rows]


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
                           group_id: Optional[int] = None, group_title: Optional[str] = None):
    """保存群组成员信息（按 group_id + user_id 幂等更新）"""
    if group_id is None:
        return
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """INSERT INTO group_members 
               (user_id, username, first_name, last_name, group_id, group_title, status, joined_at)
               VALUES (?, ?, ?, ?, ?, ?, 'active', CURRENT_TIMESTAMP)
               ON CONFLICT(group_id, user_id) DO UPDATE SET
               username=excluded.username,
               first_name=excluded.first_name,
               last_name=excluded.last_name,
               group_title=excluded.group_title,
               status='active',
               joined_at=CURRENT_TIMESTAMP""",
            (user_id, username, first_name, last_name, group_id, group_title)
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


async def get_potential_users(limit: int = 50, offset: int = 0, keyword: Optional[str] = None, tag: Optional[str] = None, group_id: Optional[int] = None, start_time: Optional[str] = None, end_time: Optional[str] = None):
    """获取潜在用户列表 (支持分页与画像/关键词过滤)"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        where_clauses = []
        params = []
        
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



async def upsert_broadcast_target(
    chat_id: int,
    chat_title: str,
    chat_type: str,
    chat_username: Optional[str] = None,
    can_broadcast: bool = True,
):
    """登记/更新广播目标（群组、超级群、频道）"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT INTO broadcast_targets (chat_id, chat_title, chat_type, chat_username, can_broadcast, updated_at)
            VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(chat_id) DO UPDATE SET
                chat_title=excluded.chat_title,
                chat_type=excluded.chat_type,
                chat_username=excluded.chat_username,
                can_broadcast=excluded.can_broadcast,
                updated_at=CURRENT_TIMESTAMP
            """,
            (chat_id, chat_title, chat_type, chat_username, 1 if can_broadcast else 0),
        )
        await db.commit()


async def get_broadcast_targets(chat_types: Optional[List[str]] = None, enabled_only: bool = True):
    """获取广播目标列表（支持按类型过滤）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        where_parts = []
        params: List[Any] = []

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


async def create_push_task(task_name: str, content: str, image_file_id: Optional[str], do_translate: bool, target_chat_ids: List[int], start_at: str, interval_hours: int, status: str, next_run_at: str):
    """创建定时推送任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            """
            INSERT INTO push_tasks (task_name, content, image_file_id, do_translate, target_chat_ids, start_at, interval_hours, status, next_run_at, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            """,
            (task_name, content, image_file_id, 1 if do_translate else 0, json.dumps(target_chat_ids), start_at, interval_hours, status, next_run_at),
        )
        await db.commit()
        return cur.lastrowid


async def list_push_tasks():
    """列出推送任务"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
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

async def get_group_verify_config(group_id: int) -> dict:
    """获取群组入群验证配置"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT verify_enabled, verify_type, verify_timeout, verify_channel_id, verify_questions FROM tracked_groups WHERE group_id = ?",
            (group_id,)
        ) as cursor:
            row = await cursor.fetchone()
            if not row:
                return {"verify_enabled": False, "verify_type": "captcha", "verify_timeout": 120, "verify_channel_id": None, "verify_questions": None}
            return {
                "verify_enabled": bool(row["verify_enabled"]),
                "verify_type": row["verify_type"] or "captcha",
                "verify_timeout": row["verify_timeout"] or 120,
                "verify_channel_id": row["verify_channel_id"],
                "verify_questions": json.loads(row["verify_questions"]) if row["verify_questions"] else [],
            }


async def save_group_verify_config(group_id: int, config: dict) -> None:
    """保存群组入群验证配置"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """UPDATE tracked_groups SET
                verify_enabled = ?, verify_type = ?, verify_timeout = ?,
                verify_channel_id = ?, verify_questions = ?
               WHERE group_id = ?""",
            (
                1 if config.get("verify_enabled") else 0,
                config.get("verify_type", "captcha"),
                config.get("verify_timeout", 120),
                config.get("verify_channel_id"),
                json.dumps(config.get("verify_questions", []), ensure_ascii=False) if config.get("verify_questions") else None,
                group_id,
            )
        )
        await db.commit()


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


