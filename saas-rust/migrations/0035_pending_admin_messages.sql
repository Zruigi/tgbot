-- 0035_pending_admin_messages.sql - 待确认管理员消息（批量补发）补充列
--
-- platform.pending_admin_messages 表结构在 0003_saas.sql 中已创建（msg_uuid 主键）。
-- 本迁移补齐批量补发流程需要的 target_user_id / expires_at 列，均为幂等操作：
--   - target_user_id：管理员选择的目标用户（select_other_user / confirm_send_to 使用）
--   - expires_at：消息默认 30 分钟失效（对齐 Python is_pending_message_expired）
-- 若生产库尚未建表（迁移顺序异常），这里也兜底建表。

BEGIN;

CREATE TABLE IF NOT EXISTS platform.pending_admin_messages (
    msg_uuid text PRIMARY KEY,
    admin_id bigint NOT NULL,
    content text,
    msg_type text NOT NULL DEFAULT 'text',
    file_id text,
    target_user_id bigint,
    status text NOT NULL DEFAULT 'pending',
    timestamp timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE platform.pending_admin_messages ADD COLUMN IF NOT EXISTS target_user_id bigint;
ALTER TABLE platform.pending_admin_messages ADD COLUMN IF NOT EXISTS expires_at timestamptz;

CREATE INDEX IF NOT EXISTS idx_pending_admin_messages_admin
    ON platform.pending_admin_messages(admin_id, status);
CREATE INDEX IF NOT EXISTS idx_pending_admin_messages_status
    ON platform.pending_admin_messages(status, timestamp);
CREATE INDEX IF NOT EXISTS idx_pending_admin_messages_expires
    ON platform.pending_admin_messages(expires_at) WHERE status = 'pending';

COMMIT;
