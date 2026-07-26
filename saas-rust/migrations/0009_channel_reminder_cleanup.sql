BEGIN;

ALTER TABLE groups.channel_reminders
    ADD COLUMN IF NOT EXISTS delete_claimed_at timestamptz,
    ADD COLUMN IF NOT EXISTS delete_retry_at timestamptz,
    ADD COLUMN IF NOT EXISTS delete_attempts integer NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS last_delete_error text;

CREATE INDEX IF NOT EXISTS idx_channel_reminders_cleanup
    ON groups.channel_reminders (delete_retry_at, expires_at, id)
    WHERE is_deleted = false;

COMMIT;
