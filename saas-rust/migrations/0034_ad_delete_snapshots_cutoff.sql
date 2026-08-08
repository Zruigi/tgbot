-- 0034: ad_delete_snapshots stores query window lower bound
-- Delete re-verification requires message_ts BETWEEN cutoff_ts AND created_at
-- (mirrors Python tools.py semantics; fixes reversed time condition in Rust).

BEGIN;

ALTER TABLE groups.ad_delete_snapshots
    ADD COLUMN IF NOT EXISTS cutoff_ts bigint;

COMMENT ON COLUMN groups.ad_delete_snapshots.cutoff_ts IS
    'Query window lower bound (unix seconds); NULL for legacy rows (lower bound not enforced).';

COMMIT;
