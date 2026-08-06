BEGIN;

CREATE TABLE IF NOT EXISTS platform.worker_runtime_heartbeats (
    worker_id text PRIMARY KEY,
    started_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    scheduler_started_at timestamptz,
    last_poll_at timestamptz,
    last_successful_poll_at timestamptz,
    last_poll_error_at timestamptz,
    total_polls bigint NOT NULL DEFAULT 0,
    total_claimed_jobs bigint NOT NULL DEFAULT 0,
    total_completed_jobs bigint NOT NULL DEFAULT 0,
    total_failed_jobs bigint NOT NULL DEFAULT 0,
    in_flight_jobs bigint NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_worker_runtime_heartbeats_last_seen
    ON platform.worker_runtime_heartbeats(last_seen_at DESC);

CREATE TABLE IF NOT EXISTS platform.worker_tenant_heartbeats (
    worker_id text NOT NULL REFERENCES platform.worker_runtime_heartbeats(worker_id) ON DELETE CASCADE,
    tenant_id text NOT NULL,
    last_seen_at timestamptz NOT NULL,
    claimed_jobs bigint NOT NULL DEFAULT 0,
    completed_jobs bigint NOT NULL DEFAULT 0,
    failed_jobs bigint NOT NULL DEFAULT 0,
    in_flight_jobs bigint NOT NULL DEFAULT 0,
    last_claimed_at timestamptz,
    last_completed_at timestamptz,
    last_failed_at timestamptz,
    PRIMARY KEY (worker_id, tenant_id)
);

CREATE INDEX IF NOT EXISTS idx_worker_tenant_heartbeats_tenant_seen
    ON platform.worker_tenant_heartbeats(tenant_id, last_seen_at DESC);

COMMIT;
