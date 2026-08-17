-- 一次性数据修复（2026-08-11）：回填 chat.admin_inbox / chat.users
-- 背景：Python 时代的历史客服消息 to_userid=商户 owner(6889612019)，
-- Rust 管道只维护 to=bot 的消息 → 这些历史会话在 admin_inbox 无行 → 会话列表缺失。
-- 执行前已备份：/home/ubuntu/backup_admin_inbox.csv /home/ubuntu/backup_chat_users.csv

-- 1) 回填 chat.users（从 group_members + potential_users）
INSERT INTO chat.users (user_id, username, first_name, last_name)
SELECT DISTINCT ON (t.user_id) t.user_id, t.username, t.first_name, t.last_name FROM (
  SELECT gm.user_id, gm.username, gm.first_name, gm.last_name FROM groups.group_members gm WHERE gm.user_id IS NOT NULL
  UNION ALL
  SELECT pu.user_id, pu.username, pu.first_name, NULL::text FROM tenant_saas.potential_users pu WHERE pu.user_id IS NOT NULL
) t
ON CONFLICT (user_id) DO UPDATE SET
  username = COALESCE(NULLIF(EXCLUDED.username,''), chat.users.username),
  first_name = COALESCE(NULLIF(EXCLUDED.first_name,''), chat.users.first_name),
  last_name = COALESCE(NULLIF(EXCLUDED.last_name,''), chat.users.last_name),
  updated_at = now();

-- 2) 回填 admin_inbox（客服方 = bot 6464245290 或 owner 6889612019）
WITH staff AS (SELECT 6464245290 AS sid UNION SELECT 6889612019),
agg AS (
  SELECT
    CASE WHEN m.from_userid IN (SELECT sid FROM staff) THEN m.to_userid ELSE m.from_userid END AS uid,
    m.tenant_id, MAX(m.timestamp) AS last_active
  FROM chat.messages m
  WHERE (m.from_userid IN (SELECT sid FROM staff) OR m.to_userid IN (SELECT sid FROM staff))
    AND m.from_userid NOT IN (6464245290, -99999, 900000001, 0)
    AND m.to_userid NOT IN (-99999, 900000001, 0)
  GROUP BY uid, m.tenant_id
),
latest AS (
  SELECT DISTINCT ON (uid, tenant_id)
    CASE WHEN m.from_userid IN (SELECT sid FROM staff) THEN m.to_userid ELSE m.from_userid END AS uid,
    m.tenant_id, m.content
  FROM chat.messages m
  WHERE (m.from_userid IN (SELECT sid FROM staff) OR m.to_userid IN (SELECT sid FROM staff))
    AND m.from_userid NOT IN (6464245290, -99999, 900000001, 0)
    AND m.to_userid NOT IN (-99999, 900000001, 0)
  ORDER BY uid, m.tenant_id, m.timestamp DESC
)
INSERT INTO chat.admin_inbox (user_id, username, summary, unread_count, last_active, tenant_id, created_at, updated_at)
SELECT
  a.uid,
  COALESCE(
    NULLIF((SELECT gm.username FROM groups.group_members gm WHERE gm.user_id=a.uid AND gm.tenant_id=a.tenant_id ORDER BY gm.joined_at DESC LIMIT 1), ''),
    NULLIF((SELECT cu.username FROM chat.users cu WHERE cu.user_id=a.uid LIMIT 1), ''),
    NULLIF((SELECT pu.username FROM tenant_saas.potential_users pu WHERE pu.user_id=a.uid AND pu.tenant_id=a.tenant_id LIMIT 1), ''),
    ''
  ),
  COALESCE(l.content, ''), 0, a.last_active, a.tenant_id, now(), now()
FROM agg a
LEFT JOIN latest l ON l.uid = a.uid AND l.tenant_id = a.tenant_id
ON CONFLICT (user_id, tenant_id) DO UPDATE SET
  summary = EXCLUDED.summary,
  last_active = EXCLUDED.last_active,
  username = CASE WHEN chat.admin_inbox.username IS NULL OR chat.admin_inbox.username = '' THEN EXCLUDED.username ELSE chat.admin_inbox.username END,
  updated_at = now();
