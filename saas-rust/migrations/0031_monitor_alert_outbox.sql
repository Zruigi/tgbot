BEGIN;

CREATE UNIQUE INDEX IF NOT EXISTS idx_job_queue_monitor_alert_once
    ON job_queue (
        tenant_id,
        (payload ->> 'monitor_id'),
        (payload ->> 'tx_id')
    )
    WHERE job_type = 'monitor_alert';

COMMIT;
