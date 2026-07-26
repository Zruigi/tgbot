BEGIN;

ALTER TABLE groups.group_messages
    ADD COLUMN IF NOT EXISTS ad_text_hash text,
    ADD COLUMN IF NOT EXISTS media_unique_id text,
    ADD COLUMN IF NOT EXISTS ad_cluster_id text,
    ADD COLUMN IF NOT EXISTS ad_checked_at timestamptz,
    ADD COLUMN IF NOT EXISTS ad_action_status text NOT NULL DEFAULT 'none',
    ADD COLUMN IF NOT EXISTS is_deleted boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

CREATE INDEX IF NOT EXISTS idx_group_messages_ad_cluster
    ON groups.group_messages (ad_cluster_id)
    WHERE ad_cluster_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_group_messages_ad_text_hash
    ON groups.group_messages (ad_text_hash)
    WHERE ad_text_hash IS NOT NULL;

COMMIT;
