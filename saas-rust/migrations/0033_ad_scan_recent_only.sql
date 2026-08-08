-- 0033: ad scan incremental (recent_only) support
-- Agent-triggered scans default to a rolling 7-day window instead of full history,
-- keeping first-run dry-runs and activations fast on large groups.

ALTER TABLE groups.ad_scan_jobs
    ADD COLUMN IF NOT EXISTS recent_only boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN groups.ad_scan_jobs.recent_only IS
    'When true the worker only scans messages from the last 7 days (rolling window).';
