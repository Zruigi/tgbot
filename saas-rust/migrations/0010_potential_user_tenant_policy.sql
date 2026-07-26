BEGIN;

DO $$
DECLARE
    primary_key_name text;
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'tenant_saas.potential_users'::regclass
          AND contype = 'p'
          AND pg_get_constraintdef(oid) = 'PRIMARY KEY (tenant_id, user_id)'
    ) THEN
        SELECT conname INTO primary_key_name
        FROM pg_constraint
        WHERE conrelid = 'tenant_saas.potential_users'::regclass
          AND contype = 'p';

        IF primary_key_name IS NOT NULL THEN
            EXECUTE format(
                'ALTER TABLE tenant_saas.potential_users DROP CONSTRAINT %I',
                primary_key_name
            );
        END IF;

        ALTER TABLE tenant_saas.potential_users
            ADD CONSTRAINT potential_users_pkey PRIMARY KEY (tenant_id, user_id);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_potential_users_tenant_captured
    ON tenant_saas.potential_users (tenant_id, captured_at DESC, user_id);

COMMIT;
