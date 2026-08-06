BEGIN;

ALTER TABLE job_queue
    ADD COLUMN IF NOT EXISTS claim_id uuid;

COMMIT;
