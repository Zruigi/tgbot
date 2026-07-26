BEGIN;

DO $$
DECLARE
    primary_key_name text;
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'broadcast.broadcast_targets'::regclass
          AND contype = 'p'
          AND pg_get_constraintdef(oid) = 'PRIMARY KEY (chat_id, tenant_id)'
    ) THEN
        SELECT conname INTO primary_key_name
        FROM pg_constraint
        WHERE conrelid = 'broadcast.broadcast_targets'::regclass
          AND contype = 'p';

        IF primary_key_name IS NOT NULL THEN
            EXECUTE format(
                'ALTER TABLE broadcast.broadcast_targets DROP CONSTRAINT %I',
                primary_key_name
            );
        END IF;
        ALTER TABLE broadcast.broadcast_targets
            ADD CONSTRAINT broadcast_targets_pkey PRIMARY KEY (chat_id, tenant_id);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_broadcast_targets_tenant_enabled
    ON broadcast.broadcast_targets (tenant_id, can_broadcast, updated_at DESC);

COMMIT;
