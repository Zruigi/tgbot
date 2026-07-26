BEGIN;

CREATE INDEX IF NOT EXISTS idx_job_queue_group_message_policy_active
    ON job_queue (
        tenant_id,
        job_type,
        (payload ->> 'group_id'),
        (payload ->> 'telegram_message_id')
    )
    WHERE job_type = 'group_message_policy'
      AND status IN ('pending', 'running');

CREATE OR REPLACE FUNCTION groups.enqueue_group_message_policy_job()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.sender_type = 'user'
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

DROP TRIGGER IF EXISTS group_message_policy_outbox ON groups.group_messages;
CREATE TRIGGER group_message_policy_outbox
AFTER INSERT OR UPDATE OF content, username, user_id, tg_msg_id, tenant_id
ON groups.group_messages
FOR EACH ROW
EXECUTE FUNCTION groups.enqueue_group_message_policy_job();

COMMIT;
