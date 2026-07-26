BEGIN;

DO $$
DECLARE
    primary_key_name text;
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'chat.admin_inbox'::regclass
          AND contype = 'p'
          AND pg_get_constraintdef(oid) = 'PRIMARY KEY (user_id, tenant_id)'
    ) THEN
        SELECT conname INTO primary_key_name
        FROM pg_constraint
        WHERE conrelid = 'chat.admin_inbox'::regclass
          AND contype = 'p';

        IF primary_key_name IS NOT NULL THEN
            EXECUTE format('ALTER TABLE chat.admin_inbox DROP CONSTRAINT %I', primary_key_name);
        END IF;
        ALTER TABLE chat.admin_inbox
            ADD CONSTRAINT admin_inbox_pkey PRIMARY KEY (user_id, tenant_id);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_chat_messages_tenant_participants
    ON chat.messages (tenant_id, from_userid, to_userid, timestamp DESC)
    WHERE is_deleted = false;

CREATE INDEX IF NOT EXISTS idx_chat_messages_tenant_tg_msg
    ON chat.messages (tenant_id, tg_msg_id)
    WHERE tg_msg_id IS NOT NULL;

COMMIT;
