BEGIN;

ALTER TABLE groups.channel_reminders
    ADD COLUMN IF NOT EXISTS tenant_id text;

UPDATE groups.channel_reminders AS reminder
SET tenant_id = tracked.tenant_id
FROM groups.tracked_groups AS tracked
WHERE reminder.group_id = tracked.group_id
  AND reminder.tenant_id IS NULL;

UPDATE groups.channel_reminders
SET tenant_id = 'default'
WHERE tenant_id IS NULL;

ALTER TABLE groups.channel_reminders
    ALTER COLUMN tenant_id SET DEFAULT 'default',
    ALTER COLUMN tenant_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_channel_reminders_tenant
    ON groups.channel_reminders (tenant_id, group_id, sent_at DESC);

COMMIT;
