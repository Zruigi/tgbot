BEGIN;

CREATE OR REPLACE FUNCTION groups.enqueue_realtime_ad_scan_job()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    realtime_job_id text;
BEGIN
    IF NEW.sender_type <> 'user'
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
                  AND message.sender_type = 'user'
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

DROP TRIGGER IF EXISTS realtime_ad_scan_outbox ON groups.group_messages;
CREATE TRIGGER realtime_ad_scan_outbox
AFTER INSERT OR UPDATE OF content, media_unique_id, ad_text_hash
ON groups.group_messages
FOR EACH ROW
EXECUTE FUNCTION groups.enqueue_realtime_ad_scan_job();

COMMIT;
