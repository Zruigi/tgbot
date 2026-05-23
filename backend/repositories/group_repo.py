# -*- coding: utf-8 -*-
"""
群组 Repository - 群组监听、成员管理相关数据库操作
"""
import aiosqlite
import logging
import json
from datetime import datetime, timedelta
from typing import Optional, List, Dict, Any
from config import DB_NAME, ADMIN_ID

logger = logging.getLogger(__name__)


async def upsert_potential_user(
    user_id: int,
    username: Optional[str],
    first_name: Optional[str] = None,
    last_name: Optional[str] = None,
    message_text: Optional[str] = None,
    keywords: Optional[str] = None,
    profile_tags: Optional[str] = None,
    group_id: Optional[int] = None,
    group_title: Optional[str] = None
) -> None:
    """保存或更新潜在用户信息（含标签累积逻辑）"""
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


async def upsert_tracked_group(group_id: int, title: str, tenant_id: Optional[str] = None) -> None:
    """登记或更新追踪的群组信息（支持租户隔离）"""
    effective_tenant = tenant_id or 'default'
    async with aiosqlite.connect(DB_NAME) as db:
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


async def get_potential_tags(tenant_id: Optional[str] = None) -> List[str]:
    """从 potential_users 表中去重聚合所有标签，支持租户隔离"""
    async with aiosqlite.connect(DB_NAME) as db:
        if tenant_id is not None:
            async with db.execute("SELECT profile_tags FROM potential_users WHERE profile_tags IS NOT NULL AND tenant_id = ?", (tenant_id,)) as cursor:
                rows = await cursor.fetchall()
        else:
            async with db.execute("SELECT profile_tags FROM potential_users WHERE profile_tags IS NOT NULL") as cursor:
                rows = await cursor.fetchall()

        all_tags = set()
        for row in rows:
            tags = [t.strip() for t in row[0].split(',') if t.strip()]
            all_tags.update(tags)
        return sorted(list(all_tags))


async def get_tracked_groups_paginated(
    page: int = 1, size: int = 20, tenant_id: Optional[str] = None
) -> Dict[str, Any]:
    """分页获取群组列表，支持租户隔离"""
    offset = (page - 1) * size
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        if tenant_id is not None:
            async with db.execute("SELECT COUNT(*) FROM tracked_groups WHERE tenant_id = ?", (tenant_id,)) as cursor:
                total = (await cursor.fetchone())[0]
            async with db.execute(
                "SELECT * FROM tracked_groups WHERE tenant_id = ? ORDER BY updated_at DESC LIMIT ? OFFSET ?",
                (tenant_id, size, offset)
            ) as cursor:
                rows = await cursor.fetchall()
        else:
            async with db.execute("SELECT COUNT(*) FROM tracked_groups") as cursor:
                total = (await cursor.fetchone())[0]
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


async def set_group_listen_status(group_id: int, is_monitored: bool) -> None:
    """开启或关闭群组监听"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE tracked_groups SET is_monitored = ? WHERE group_id = ?",
            (1 if is_monitored else 0, group_id)
        )
        await db.commit()


async def get_tracked_groups_with_stats(tenant_id: Optional[str] = None) -> List[Dict[str, Any]]:
    """获取所有追踪群组及其详细监测状态，支持租户隔离"""
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
                # 计算在线状态 - 更健壮的日期解析
                updated_at_str = g.get('updated_at', '')
                if updated_at_str:
                    # 尝试多种日期格式
                    try:
                        updated_at_dt = datetime.fromisoformat(updated_at_str.replace(' ', 'T'))
                    except (ValueError, AttributeError):
                        # 如果解析失败，使用当前时间作为后备
                        updated_at_dt = now
                    g['is_online'] = updated_at_dt > online_threshold
                else:
                    g['is_online'] = False
                # 兼容旧字段
                g['name'] = g['group_title']
                g['listen_status'] = bool(g['is_monitored'])
                groups.append(g)
            except Exception as e:
                logger.warning(f"处理群组数据时出错: {e}, row: {dict(row)}")
                continue
        return groups


async def update_group_custom_keywords(group_id: int, keywords: str) -> None:
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


async def set_group_delete_on_revoke(group_id: int, enabled: bool) -> None:
    """设置群组撤回同步删除开关"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE tracked_groups SET delete_on_revoke = ? WHERE group_id = ?",
            (1 if enabled else 0, group_id)
        )
        await db.commit()


async def get_groups_with_revoke_sync() -> List[Dict[str, Any]]:
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


async def mark_group_message_revoke_checked(group_id: int, tg_msg_id: int) -> None:
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
    is_advertising: Optional[bool] = None,
    ad_score: Optional[int] = None,
    ad_confidence: Optional[str] = None,
    ad_findings: Optional[List[str]] = None,
) -> None:
    """保存全量群组消息记录"""
    ts = int(message_ts) if message_ts else None
    findings_text = None
    if isinstance(ad_findings, list):
        try:
            findings_text = json.dumps(ad_findings, ensure_ascii=False)
        except Exception:
            findings_text = None
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            """
            INSERT OR IGNORE INTO group_messages (
                group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, file_id,
                quoted_tg_msg_id, quoted_text, message_ts, avatar_file_id,
                is_advertising, ad_score, ad_confidence, ad_findings
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, file_id,
                quoted_tg_msg_id, quoted_text, ts, avatar_file_id,
                1 if is_advertising else 0,
                int(ad_score or 0),
                (ad_confidence or "low"),
                findings_text,
            )
        )
        await db.commit()


async def get_group_messages(group_id: int, limit: int = 50, offset: int = 0) -> List[Dict[str, Any]]:
    """获取指定群组的消息历史"""
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
    """删除群消息记录"""
    async with aiosqlite.connect(DB_NAME) as db:
        cur = await db.execute(
            "DELETE FROM group_messages WHERE group_id = ? AND tg_msg_id = ?",
            (group_id, tg_msg_id),
        )
        await db.commit()
        return cur.rowcount > 0


async def save_group_member(
    user_id: int,
    username: Optional[str],
    first_name: Optional[str] = None,
    last_name: Optional[str] = None,
    group_id: Optional[int] = None,
    group_title: Optional[str] = None
) -> None:
    """保存群组成员信息"""
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


async def update_group_member_status(user_id: int, group_id: int, status: str) -> None:
    """更新群成员状态"""
    async with aiosqlite.connect(DB_NAME) as db:
        await db.execute(
            "UPDATE group_members SET status = ? WHERE user_id = ? AND group_id = ?",
            (status, user_id, group_id)
        )
        await db.commit()


async def get_recent_active_conversations(limit: int = 10, days: int = 7) -> List[Dict[str, Any]]:
    """获取最近活跃的私聊对话"""
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


async def get_tracked_group_by_id(group_id: int) -> Optional[Dict[str, Any]]:
    """根据 group_id 获取群组信息（含 tenant_id）"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute("SELECT * FROM tracked_groups WHERE group_id = ?", (group_id,)) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None


async def verify_group_ownership(group_id: int, tenant_id: str) -> Optional[Dict[str, Any]]:
    """验证群组归属并返回群组信息，不属于当前租户则返回 None"""
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT * FROM tracked_groups WHERE group_id = ? AND tenant_id = ?",
            (group_id, tenant_id)
        ) as cursor:
            row = await cursor.fetchone()
            return dict(row) if row else None
