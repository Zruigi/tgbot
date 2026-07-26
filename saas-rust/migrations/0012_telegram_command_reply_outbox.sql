BEGIN;

CREATE UNIQUE INDEX IF NOT EXISTS idx_job_queue_telegram_command_reply_once
    ON job_queue (
        tenant_id,
        (payload ->> 'inbound_message_id')
    )
    WHERE job_type = 'telegram_command_reply';

CREATE OR REPLACE FUNCTION chat.enqueue_telegram_command_reply_job()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF current_setting('tgbot.telegram_commands_enabled', true) = 'true'
       AND NEW.is_read = false
       AND NEW.is_deleted = false
       AND NEW.from_userid > 0
       AND NEW.to_userid > 0
       AND NEW.from_userid <> NEW.to_userid
       AND NEW.msg_type = 'text'
       AND NEW.tg_msg_id IS NOT NULL
       AND NULLIF(btrim(NEW.content), '') LIKE '/%' THEN
        INSERT INTO job_queue (
            tenant_id,
            job_type,
            payload,
            priority,
            max_attempts,
            run_at,
            status
        ) VALUES (
            NEW.tenant_id,
            'telegram_command_reply',
            jsonb_build_object(
                'tenant_id', NEW.tenant_id,
                'inbound_message_id', NEW.id
            ),
            4,
            5,
            now(),
            'pending'
        )
        ON CONFLICT DO NOTHING;
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS telegram_command_reply_outbox ON chat.messages;
CREATE TRIGGER telegram_command_reply_outbox
AFTER INSERT ON chat.messages
FOR EACH ROW
EXECUTE FUNCTION chat.enqueue_telegram_command_reply_job();

COMMIT;
