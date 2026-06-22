//! Task scheduler – orchestrates periodic job polling and cron-based scheduling.
//!
//! The scheduler runs a background loop that:
//! 1. Polls the `job_queue` table for pending jobs at a configurable interval.
//! 2. Dispatches claimed jobs to the appropriate worker based on `job_type`.
//! 3. Runs periodic maintenance tasks (re-queue stale jobs, purge old jobs).
//! 4. Supports cron-based scheduling for recurring tasks (push tasks, monitor checks, etc.).
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────┐     claim      ┌─────────────────┐
//! │  Scheduler     │ ──────────────►│  Job Queue (PG) │
//! │  (tokio task)  │ ◄──────────────│  FOR UPDATE      │
//! └───────┬───────┘   job records   │  SKIP LOCKED     │
//!         │                          └─────────────────┘
//!         │ dispatch
//!         ▼
//! ┌──────────────────────────────────────────┐
//! │           Worker Dispatch Router          │
//! │  push_task | group_policy | ai | ocr | … │
//! └──────────────────────────────────────────┘
//! ```

use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, instrument, warn};

use crate::job_queue::{self, ClaimedJob, JobType};
use crate::WorkerHandle;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// How often to poll the job_queue for pending jobs.
    pub poll_interval: Duration,
    /// Maximum number of jobs to claim per poll cycle.
    pub claim_batch_size: i32,
    /// Number of worker tasks to spawn for processing jobs concurrently.
    pub worker_concurrency: usize,
    /// Timeout (in minutes) after which a running job is considered stale.
    pub stale_job_timeout_minutes: i32,
    /// How often to run maintenance (re-queue stale, purge old).
    pub maintenance_interval: Duration,
    /// Retention period (days) for completed/dead jobs before purging.
    pub job_retention_days: i32,
    /// How often to check cron-based recurring tasks.
    pub cron_check_interval: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(5),
            claim_batch_size: 20,
            worker_concurrency: 4,
            stale_job_timeout_minutes: 30,
            maintenance_interval: Duration::from_secs(300),
            job_retention_days: 7,
            cron_check_interval: Duration::from_secs(60),
        }
    }
}

// ---------------------------------------------------------------------------
// Worker trait
// ---------------------------------------------------------------------------

/// Trait that each worker type must implement.
#[async_trait::async_trait]
pub trait Worker: Send + Sync {
    /// Return the job type this worker handles.
    fn job_type(&self) -> &str;

    /// Process a single job.  On success, the job will be marked completed.
    /// On error, the job will be marked failed (and possibly retried).
    async fn process(&self, pool: &PgPool, job: &ClaimedJob) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

/// Routes claimed jobs to the correct worker implementation.
pub struct Dispatcher {
    workers: Vec<Arc<dyn Worker>>,
}

impl Dispatcher {
    pub fn new(workers: Vec<Arc<dyn Worker>>) -> Self {
        Self { workers }
    }

    /// Find a worker for the given job type.
    pub fn find_worker(&self, job_type: &str) -> Option<&Arc<dyn Worker>> {
        self.workers.iter().find(|w| w.job_type() == job_type)
    }

    /// Dispatch a claimed job to the appropriate worker.
    pub async fn dispatch(&self, pool: &PgPool, claimed: &ClaimedJob) {
        let job_id = claimed.job.id;
        let job_type = &claimed.job.job_type;

        match self.find_worker(job_type) {
            Some(worker) => {
                info!(job_id, job_type, "dispatching job to worker");
                match worker.process(pool, claimed).await {
                    Ok(()) => {
                        if let Err(e) = job_queue::complete_job(pool, job_id).await {
                            error!(job_id, error = %e, "failed to mark job completed");
                        }
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        error!(job_id, job_type, error = %err_str, "job processing failed");
                        if let Err(e2) = job_queue::fail_job(pool, job_id, &err_str).await {
                            error!(job_id, error = %e2, "failed to mark job as failed");
                        }
                    }
                }
            }
            None => {
                warn!(job_id, job_type, "no worker registered for job type, marking dead");
                if let Err(e) =
                    job_queue::fail_job(pool, job_id, "no worker registered for job type").await
                {
                    error!(job_id, error = %e, "failed to mark unknown-type job as failed");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// The main scheduler that coordinates job polling, dispatch, and maintenance.
pub struct Scheduler {
    pool: PgPool,
    config: SchedulerConfig,
    dispatcher: Arc<Dispatcher>,
}

impl Scheduler {
    pub fn new(pool: PgPool, config: SchedulerConfig, dispatcher: Dispatcher) -> Self {
        Self {
            pool,
            config,
            dispatcher: Arc::new(dispatcher),
        }
    }

    /// Start the scheduler as a set of background tokio tasks.
    /// Returns a `WorkerHandle` that can be used to shut down the scheduler.
    pub fn start(self) -> WorkerHandle {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Channel for sending claimed jobs from the poller to worker tasks.
        let (job_tx, job_rx) = mpsc::channel::<ClaimedJob>(256);

        let pool = self.pool.clone();
        let config = self.config.clone();
        let dispatcher = self.dispatcher.clone();

        // --- Poller task ---
        let poller_shutdown = shutdown_rx.clone();
        let poller_handle = tokio::spawn(async move {
            Self::run_poller(pool, config, job_tx, poller_shutdown).await;
        });

        // --- Worker pool tasks ---
        let pool_clone = self.pool.clone();
        let dispatcher_clone = self.dispatcher.clone();
        let worker_shutdown = shutdown_rx.clone();
        let worker_handle = tokio::spawn(async move {
            Self::run_worker_pool(pool_clone, dispatcher_clone, job_rx, worker_shutdown).await;
        });

        // --- Maintenance task ---
        let maint_pool = self.pool.clone();
        let maint_config = self.config.clone();
        let maint_shutdown = shutdown_rx;
        let maint_handle = tokio::spawn(async move {
            Self::run_maintenance(maint_pool, maint_config, maint_shutdown).await;
        });

        WorkerHandle {
            shutdown_tx,
            tasks: vec![poller_handle, worker_handle, maint_handle],
        }
    }

    /// Poller loop: periodically claim pending jobs and send them to workers.
    async fn run_poller(
        pool: PgPool,
        config: SchedulerConfig,
        job_tx: mpsc::Sender<ClaimedJob>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("scheduler poller started, interval={:?}", config.poll_interval);
        let mut interval = tokio::time::interval(config.poll_interval);

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("scheduler poller shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    match job_queue::claim_jobs(&pool, None, config.claim_batch_size).await {
                        Ok(jobs) => {
                            for job in jobs {
                                if job_tx.send(job).await.is_err() {
                                    info!("worker pool dropped, stopping poller");
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "failed to claim jobs from queue");
                        }
                    }
                }
            }
        }
    }

    /// Worker pool: receives claimed jobs and dispatches them concurrently.
    async fn run_worker_pool(
        pool: PgPool,
        dispatcher: Arc<Dispatcher>,
        mut job_rx: mpsc::Receiver<ClaimedJob>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("scheduler worker pool started");
        let semaphore = Arc::new(tokio::sync::Semaphore::new(4)); // default concurrency

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("scheduler worker pool shutting down");
                        break;
                    }
                }
                job = job_rx.recv() => {
                    match job {
                        Some(claimed) => {
                            let pool = pool.clone();
                            let dispatcher = dispatcher.clone();
                            let permit = semaphore.clone().acquire_owned().await;
                            tokio::spawn(async move {
                                let _permit = permit;
                                dispatcher.dispatch(&pool, &claimed).await;
                            });
                        }
                        None => {
                            info!("job channel closed, stopping worker pool");
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Maintenance loop: re-queue stale jobs and purge old completed/dead jobs.
    async fn run_maintenance(
        pool: PgPool,
        config: SchedulerConfig,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("scheduler maintenance started, interval={:?}", config.maintenance_interval);
        let mut interval = tokio::time::interval(config.maintenance_interval);

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("scheduler maintenance shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    // Re-queue stale jobs
                    if let Err(e) = job_queue::requeue_stale_jobs(&pool, config.stale_job_timeout_minutes).await {
                        error!(error = %e, "maintenance: failed to re-queue stale jobs");
                    }
                    // Purge old jobs
                    if let Err(e) = job_queue::purge_old_jobs(&pool, config.job_retention_days).await {
                        error!(error = %e, "maintenance: failed to purge old jobs");
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cron-based push task scheduling
// ---------------------------------------------------------------------------

/// Check the `push_tasks` table for tasks whose `next_run_at` has passed
/// and enqueue a corresponding `push_task` job for each.
#[instrument(skip(pool))]
pub async fn schedule_push_tasks(pool: &PgPool) -> Result<usize> {
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, task_name FROM push_tasks
        WHERE status = 'running'
          AND next_run_at <= now()
        ORDER BY next_run_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut enqueued = 0;
    for row in &rows {
        let task_id: i64 = row.try_get("id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let task_name: String = row.try_get("task_name")?;

        let opts = job_queue::EnqueueOpts::new(
            JobType::PushTask,
            json!({
                "push_task_id": task_id,
                "tenant_id": tenant_id,
                "task_name": task_name,
            }),
        )
        .with_priority(5);

        match job_queue::enqueue(pool, opts).await {
            Ok(job_id) => {
                info!(push_task_id = task_id, job_id, "enqueued push_task job");
                enqueued += 1;
            }
            Err(e) => {
                error!(push_task_id = task_id, error = %e, "failed to enqueue push_task job");
            }
        }
    }

    Ok(enqueued)
}

/// Check for groups that need policy sync and enqueue jobs.
#[instrument(skip(pool))]
pub async fn schedule_group_policy_syncs(pool: &PgPool) -> Result<usize> {
    // Find groups with verify_enabled = true that haven't been synced recently.
    // This could be based on a last_synced_at column; for now we enqueue
    // a batch job periodically.
    let rows = sqlx::query(
        r#"
        SELECT group_id, tenant_id FROM groups.tracked_groups
        WHERE verify_enabled = true
          AND is_monitored = true
        ORDER BY updated_at ASC
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut enqueued = 0;
    for row in &rows {
        let group_id: i64 = row.try_get("group_id")?;
        let tenant_id: String = row.try_get("tenant_id")?;

        // Skip if there is already a pending/running job for this group
        let existing: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)::bigint FROM job_queue
            WHERE job_type = 'group_policy_sync'
              AND payload->>'group_id' = $1
              AND status IN ('pending', 'running')
            "#,
        )
        .bind(group_id.to_string())
        .fetch_one(pool)
        .await?
        .unwrap_or(0);

        if existing > 0 {
            continue;
        }

        let opts = job_queue::EnqueueOpts::new(
            JobType::GroupPolicySync,
            json!({
                "group_id": group_id,
                "tenant_id": tenant_id,
            }),
        )
        .with_priority(2)
        .with_max_attempts(2);

        match job_queue::enqueue(pool, opts).await {
            Ok(job_id) => {
                enqueued += 1;
            }
            Err(e) => {
                error!(group_id, error = %e, "failed to enqueue group_policy_sync job");
            }
        }
    }

    Ok(enqueued)
}

/// Check for wallet/monitor entries that need to be checked and enqueue jobs.
#[instrument(skip(pool))]
pub async fn schedule_monitor_checks(pool: &PgPool) -> Result<usize> {
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, address FROM wallet_monitors
        WHERE enabled = true
          AND (last_checked_at IS NULL OR last_checked_at < now() - interval '5 minutes')
        ORDER BY last_checked_at ASC NULLS FIRST
        LIMIT 50
        "#,
    )
    .fetch_optional(pool)
    .await;

    // The wallet_monitors table may not exist yet; silently skip if so.
    match rows {
        Ok(Some(_)) | Ok(None) => {
            // If the table exists, we'd iterate over rows here.
            // For now, return 0 as the monitor infrastructure is still stub.
            Ok(0)
        }
        Err(_) => {
            // Table doesn't exist yet, skip.
            Ok(0)
        }
    }
}

/// Run all cron-based scheduling checks.
pub async fn run_cron_checks(pool: &PgPool) -> (usize, usize, usize) {
    let push = schedule_push_tasks(pool).await.unwrap_or(0);
    let group = schedule_group_policy_syncs(pool).await.unwrap_or(0);
    let monitor = schedule_monitor_checks(pool).await.unwrap_or(0);
    (push, group, monitor)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_config_default() {
        let config = SchedulerConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(5));
        assert_eq!(config.claim_batch_size, 20);
        assert_eq!(config.worker_concurrency, 4);
        assert_eq!(config.stale_job_timeout_minutes, 30);
    }
}
