-- Ad deletion snapshot store table
-- Stores transient snapshots of ad messages locked for deletion confirmation
-- Per-group pre-check of bot can_delete_messages stored in excluded_groups count
-- TTL enforced at query time (15-minute validity window)

BEGIN;

CREATE TABLE IF NOT EXISTS groups.ad_delete_snapshots (
    snapshot_id text NOT NULL PRIMARY KEY,
    tenant_id text NOT NULL DEFAULT 'default',
    group_id bigint,
    refs jsonb NOT NULL,
    excluded_groups_count integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    consumed_at timestamptz,
    created_by text
);

CREATE INDEX IF NOT EXISTS idx_ad_delete_snapshots_tenant_created
    ON groups.ad_delete_snapshots (tenant_id, created_at);

COMMIT;
