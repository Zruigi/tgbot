-- 2026 群组监听与画像标签系统 数据库迁移脚本
-- 适用范围：SQLite

-- 1. 更新 tracked_groups 表
-- 注意：SQLite 不支持一次性重命名多列，且 ALTER TABLE ADD COLUMN 有限制。
-- 这里的逻辑与 database.py 中的自动迁移保持一致。

-- 确保表存在
CREATE TABLE IF NOT EXISTS tracked_groups (
    group_id INTEGER PRIMARY KEY,
    group_title TEXT,
    is_monitored BOOLEAN DEFAULT 0,
    custom_keywords TEXT DEFAULT NULL,
    first_seen DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 2. 更新 potential_users 表以支持画像标签累积
-- 确保表存在
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
);

-- 索引优化
CREATE INDEX IF NOT EXISTS idx_potential_tags ON potential_users(profile_tags);
CREATE INDEX IF NOT EXISTS idx_potential_group ON potential_users(group_id);
CREATE INDEX IF NOT EXISTS idx_tracked_updated ON tracked_groups(updated_at);
