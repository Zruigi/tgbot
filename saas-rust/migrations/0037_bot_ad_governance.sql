-- 0037_bot_ad_governance.sql
-- P0(2026-08-13): 广告治理覆盖 bot 小号消息。
-- 广告刷屏几乎全是 bot（@gtq13ky5bot/@AZUcvv_BOT 等），此前 webhook 丢弃 bot 消息导致系统失明。
-- 现在 bot 消息入库(sender_type='bot')，两个 outbox 触发器从 'user' 放宽到 ('user','bot')。

BEGIN;

-- 1) 实时广告策略（规则检测）outbox：bot 消息也要入队检测
CREATE OR REPLACE FUNCTION groups.enqueue_group_message_policy_job()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.sender_type IN ('user', 'bot')
       AND NEW.tg_msg_id IS NOT NULL
       AND NULLIF(btrim(NEW.content), '') IS NOT NULL THEN
        INSERT INTO job_queue (
            tenant_id,
            job_type,
            payload,
            priority,
            max_attempts,
            run_at,
            status
        )
        SELECT
            NEW.tenant_id,
            'group_message_policy',
            jsonb_build_object(
                'tenant_id', NEW.tenant_id,
                'group_id', NEW.group_id,
                'telegram_message_id', NEW.tg_msg_id
            ),
            2,
            5,
            now(),
            'pending'
        WHERE NOT EXISTS (
            SELECT 1
            FROM job_queue queued
            WHERE queued.tenant_id = NEW.tenant_id
              AND queued.job_type = 'group_message_policy'
              AND queued.payload ->> 'group_id' = NEW.group_id::text
              AND queued.payload ->> 'telegram_message_id' = NEW.tg_msg_id::text
              AND queued.status IN ('pending', 'running')
        );
    END IF;
    RETURN NEW;
END;
$$;

-- 2) 实时广告扫描（聚类+LLM）outbox：bot 消息也要触发
CREATE OR REPLACE FUNCTION groups.enqueue_realtime_ad_scan_job()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    realtime_job_id text;
BEGIN
    IF NEW.sender_type NOT IN ('user', 'bot')
       OR NEW.tg_msg_id IS NULL
       OR (NULLIF(btrim(COALESCE(NEW.content, '')), '') IS NULL
           AND NULLIF(btrim(COALESCE(NEW.media_unique_id, '')), '') IS NULL)
       OR NOT EXISTS (
           SELECT 1
           FROM groups.ad_group_policies policy
           WHERE policy.tenant_id = NEW.tenant_id
             AND policy.group_id = NEW.group_id
             AND policy.is_enabled = true
       ) THEN
        RETURN NEW;
    END IF;

    realtime_job_id := format(
        'rt_%s_%s_%s_%s',
        substr(md5(NEW.tenant_id), 1, 8),
        NEW.group_id,
        NEW.tg_msg_id,
        substr(md5(COALESCE(NEW.ad_text_hash, '') || ':' || COALESCE(NEW.media_unique_id, '')), 1, 12)
    );

    WITH inserted_job AS (
        INSERT INTO groups.ad_scan_jobs (
            job_id, tenant_id, group_id, group_title, requested_by, status,
            dry_run, activate_policy, source, total_messages
        )
        SELECT
            realtime_job_id,
            NEW.tenant_id,
            NEW.group_id,
            tracked.group_title,
            'telegram_webhook',
            'queued',
            false,
            false,
            'realtime',
            (
                SELECT COUNT(*)::integer
                FROM groups.group_messages message
                WHERE message.tenant_id = NEW.tenant_id
                  AND message.group_id = NEW.group_id
                  AND message.sender_type IN ('user', 'bot')
                  AND NOT message.is_deleted
            )
        FROM groups.tracked_groups tracked
        WHERE tracked.tenant_id = NEW.tenant_id
          AND tracked.group_id = NEW.group_id
        ON CONFLICT (job_id) DO NOTHING
        RETURNING job_id, tenant_id, group_id
    )
    INSERT INTO job_queue (
        tenant_id, job_type, payload, priority, max_attempts, run_at, status
    )
    SELECT
        tenant_id,
        'ad_scan_job',
        jsonb_build_object(
            'job_id', job_id,
            'tenant_id', tenant_id,
            'group_id', group_id,
            'source', 'realtime',
            'recent_only', true
        ),
        5,
        3,
        now(),
        'pending'
    FROM inserted_job;

    RETURN NEW;
END;
$$;

COMMIT;
