BEGIN;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM monitor_transactions
        GROUP BY tenant_id, tx_id
        HAVING COUNT(*) > 1
    ) THEN
        RAISE EXCEPTION '0030 blocked: duplicate monitor transaction tenant key';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM address_monitors
        GROUP BY tenant_id, monitor_id
        HAVING COUNT(*) > 1
    ) THEN
        RAISE EXCEPTION '0030 blocked: duplicate address monitor tenant key';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM monitor_transactions transaction_row
        LEFT JOIN address_monitors monitor
          ON monitor.tenant_id = transaction_row.tenant_id
         AND monitor.monitor_id = transaction_row.monitor_id
        WHERE monitor.monitor_id IS NULL
    ) THEN
        RAISE EXCEPTION '0030 blocked: cross-tenant monitor transaction reference';
    END IF;
END
$$;

ALTER TABLE monitor_transactions
    DROP CONSTRAINT IF EXISTS monitor_transactions_pkey;

ALTER TABLE monitor_transactions
    ADD CONSTRAINT monitor_transactions_pkey PRIMARY KEY (tenant_id, tx_id);

ALTER TABLE monitor_transactions
    DROP CONSTRAINT IF EXISTS monitor_transactions_monitor_id_fkey;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'address_monitors'::regclass
          AND conname = 'address_monitors_tenant_monitor_key'
    ) THEN
        ALTER TABLE address_monitors
            ADD CONSTRAINT address_monitors_tenant_monitor_key
            UNIQUE (tenant_id, monitor_id);
    END IF;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'monitor_transactions'::regclass
          AND conname = 'monitor_transactions_tenant_monitor_fkey'
    ) THEN
        ALTER TABLE monitor_transactions
            ADD CONSTRAINT monitor_transactions_tenant_monitor_fkey
            FOREIGN KEY (tenant_id, monitor_id)
            REFERENCES address_monitors (tenant_id, monitor_id)
            ON DELETE CASCADE;
    END IF;
END
$$;

COMMIT;
