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
use crate::push_task_worker::TelegramClientResolver;
use crate::WorkerHandle;
use tgbot_domain::worker_telemetry::{WorkerTelemetry, WorkerTelemetrySnapshot};
use uuid::Uuid;

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
    /// How often to persist worker liveness and job counters.
    pub heartbeat_interval: Duration,
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
            heartbeat_interval: Duration::from_secs(10),
        }
    }
}

impl SchedulerConfig {
    pub fn effective_worker_concurrency(&self) -> usize {
        self.worker_concurrency.max(1)
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
    pub async fn dispatch(&self, pool: &PgPool, claimed: &ClaimedJob, telemetry: &WorkerTelemetry) {
        let job_id = claimed.job.id;
        let job_type = &claimed.job.job_type;
        let tenant_id = &claimed.job.tenant_id;

        match self.find_worker(job_type) {
            Some(worker) => {
                info!(job_id, job_type, "dispatching job to worker");
                match worker.process(pool, claimed).await {
                    Ok(()) => {
                        if let Err(e) =
                            job_queue::complete_job(pool, job_id, claimed.claim_id).await
                        {
                            error!(job_id, error = %e, "failed to mark job completed");
                        }
                        telemetry.mark_completed(tenant_id);
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        error!(job_id, job_type, error = %err_str, "job processing failed");
                        if let Err(e2) =
                            job_queue::fail_job(pool, job_id, claimed.claim_id, &err_str).await
                        {
                            error!(job_id, error = %e2, "failed to mark job as failed");
                        }
                        telemetry.mark_failed(tenant_id);
                    }
                }
            }
            None => {
                warn!(
                    job_id,
                    job_type, "no worker registered for job type, marking dead"
                );
                if let Err(e) = job_queue::fail_job(
                    pool,
                    job_id,
                    claimed.claim_id,
                    "no worker registered for job type",
                )
                .await
                {
                    error!(job_id, error = %e, "failed to mark unknown-type job as failed");
                }
                telemetry.mark_failed(tenant_id);
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
    telemetry: WorkerTelemetry,
    worker_id: String,
    job_type_filters: Vec<String>,
    push_task_cron: bool,
    address_monitor_cron: bool,
    price_monitor_cron: bool,
    channel_reminder_cleanup: Option<(Arc<dyn TelegramClientResolver>, Duration)>,
}

impl Scheduler {
    pub fn new(pool: PgPool, config: SchedulerConfig, dispatcher: Dispatcher) -> Self {
        Self {
            pool,
            config,
            dispatcher: Arc::new(dispatcher),
            telemetry: WorkerTelemetry::default(),
            worker_id: Uuid::new_v4().to_string(),
            job_type_filters: Vec::new(),
            push_task_cron: false,
            address_monitor_cron: false,
            price_monitor_cron: false,
            channel_reminder_cleanup: None,
        }
    }

    pub fn with_job_type_filter(mut self, job_type: impl Into<String>) -> Self {
        let job_type = job_type.into();
        if !self.job_type_filters.contains(&job_type) {
            self.job_type_filters.push(job_type);
        }
        self
    }

    pub fn with_telemetry(mut self, telemetry: WorkerTelemetry) -> Self {
        self.telemetry = telemetry;
        self
    }

    pub fn with_push_task_cron(mut self) -> Self {
        self.push_task_cron = true;
        self
    }

    pub fn with_address_monitor_cron(mut self) -> Self {
        self.address_monitor_cron = true;
        self
    }

    pub fn with_price_monitor_cron(mut self) -> Self {
        self.price_monitor_cron = true;
        self
    }

    pub fn with_channel_reminder_cleanup(
        mut self,
        telegram_resolver: Arc<dyn TelegramClientResolver>,
        interval: Duration,
    ) -> Self {
        self.channel_reminder_cleanup = Some((telegram_resolver, interval));
        self
    }

    /// Start the scheduler as a set of background tokio tasks.
    /// Returns a `WorkerHandle` that can be used to shut down the scheduler.
    pub fn start(self) -> WorkerHandle {
        self.telemetry.mark_scheduler_started();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Channel for sending claimed jobs from the poller to worker tasks.
        let (job_tx, job_rx) = mpsc::channel::<ClaimedJob>(256);

        let pool = self.pool.clone();
        let config = self.config.clone();
        let job_type_filters = self.job_type_filters.clone();
        let worker_concurrency = self.config.effective_worker_concurrency();
        let telemetry = self.telemetry.clone();
        let heartbeat_pool = self.pool.clone();
        let heartbeat_id = self.worker_id.clone();
        let heartbeat_interval = self.config.heartbeat_interval;
        let heartbeat_telemetry = self.telemetry.clone();
        let heartbeat_shutdown = shutdown_rx.clone();

        // --- Poller task ---
        let poller_shutdown = shutdown_rx.clone();
        let poller_handle = tokio::spawn(async move {
            Self::run_poller(
                pool,
                config,
                job_type_filters,
                job_tx,
                telemetry,
                poller_shutdown,
            )
            .await;
        });

        // --- Worker pool tasks ---
        let pool_clone = self.pool.clone();
        let dispatcher_clone = self.dispatcher.clone();
        let worker_shutdown = shutdown_rx.clone();
        let telemetry = self.telemetry.clone();
        let worker_handle = tokio::spawn(async move {
            Self::run_worker_pool(
                pool_clone,
                dispatcher_clone,
                worker_concurrency,
                job_rx,
                telemetry,
                worker_shutdown,
            )
            .await;
        });

        // --- Maintenance task ---
        let maint_pool = self.pool.clone();
        let maint_config = self.config.clone();
        let maint_shutdown = shutdown_rx.clone();
        let maint_handle = tokio::spawn(async move {
            Self::run_maintenance(maint_pool, maint_config, maint_shutdown).await;
        });

        let heartbeat_handle = tokio::spawn(async move {
            Self::run_heartbeat(
                heartbeat_pool,
                heartbeat_id,
                heartbeat_telemetry,
                heartbeat_interval,
                heartbeat_shutdown,
            )
            .await;
        });

        let mut tasks = vec![poller_handle, worker_handle, maint_handle, heartbeat_handle];
        if self.push_task_cron {
            let cron_pool = self.pool.clone();
            let cron_interval = self.config.cron_check_interval;
            let cron_shutdown = shutdown_rx.clone();
            tasks.push(tokio::spawn(async move {
                Self::run_push_task_cron(cron_pool, cron_interval, cron_shutdown).await;
            }));
        }
        if self.address_monitor_cron {
            let monitor_pool = self.pool.clone();
            let monitor_interval = self.config.cron_check_interval;
            let monitor_shutdown = shutdown_rx.clone();
            tasks.push(tokio::spawn(async move {
                Self::run_address_monitor_cron(monitor_pool, monitor_interval, monitor_shutdown)
                    .await;
            }));
        }
        if self.price_monitor_cron {
            let price_pool = self.pool.clone();
            let price_interval = self.config.cron_check_interval;
            let price_shutdown = shutdown_rx.clone();
            tasks.push(tokio::spawn(async move {
                Self::run_price_monitor_cron(price_pool, price_interval, price_shutdown).await;
            }));
        }
        if let Some((telegram_resolver, cleanup_interval)) = self.channel_reminder_cleanup {
            let cleanup_pool = self.pool.clone();
            let cleanup_shutdown = shutdown_rx;
            tasks.push(tokio::spawn(async move {
                Self::run_channel_reminder_cleanup(
                    cleanup_pool,
                    telegram_resolver,
                    cleanup_interval,
                    cleanup_shutdown,
                )
                .await;
            }));
        }

        WorkerHandle { shutdown_tx, tasks }
    }

    async fn run_heartbeat(
        pool: PgPool,
        worker_id: String,
        telemetry: WorkerTelemetry,
        heartbeat_interval: Duration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(worker_id = %worker_id, ?heartbeat_interval, "worker heartbeat started");
        let mut interval = tokio::time::interval(heartbeat_interval);

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!(worker_id = %worker_id, "worker heartbeat shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    let snapshot = telemetry.snapshot();
                    if let Err(error) = persist_worker_telemetry(&pool, &worker_id, &snapshot).await {
                        error!(worker_id = %worker_id, %error, "failed to persist worker heartbeat");
                    }
                }
            }
        }
    }

    /// Poller loop: periodically claim pending jobs and send them to workers.
    async fn run_poller(
        pool: PgPool,
        config: SchedulerConfig,
        job_type_filters: Vec<String>,
        job_tx: mpsc::Sender<ClaimedJob>,
        telemetry: WorkerTelemetry,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(
            "scheduler poller started, interval={:?}",
            config.poll_interval
        );
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
                    telemetry.mark_poll_started();
                    match job_queue::claim_jobs(
                        &pool,
                        &job_type_filters,
                        config.claim_batch_size,
                    ).await {
                        Ok(jobs) => {
                            telemetry.mark_poll_succeeded();
                            for job in jobs {
                                let tenant_id = job.job.tenant_id.clone();
                                telemetry.mark_claimed(&tenant_id);
                                if job_tx.send(job).await.is_err() {
                                    telemetry.mark_dropped(&tenant_id);
                                    info!("worker pool dropped, stopping poller");
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            telemetry.mark_poll_failed();
                            error!(error = %e, "failed to claim jobs from queue");
                        }
                    }
                }
            }
        }
    }

    async fn run_push_task_cron(
        pool: PgPool,
        check_interval: Duration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(?check_interval, "push task cron started");
        let mut interval = tokio::time::interval(check_interval);
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("push task cron shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    if let Err(error) = schedule_push_tasks(&pool).await {
                        error!(%error, "failed to schedule push tasks");
                    }
                }
            }
        }
    }

    async fn run_address_monitor_cron(
        pool: PgPool,
        check_interval: Duration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(?check_interval, "address monitor cron started");
        let mut interval = tokio::time::interval(check_interval);
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("address monitor cron shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    if let Err(error) = schedule_address_monitor_checks(&pool).await {
                        error!(%error, "failed to schedule address monitor checks");
                    }
                }
            }
        }
    }

    async fn run_price_monitor_cron(
        pool: PgPool,
        check_interval: Duration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(?check_interval, "price monitor cron started");
        let mut interval = tokio::time::interval(check_interval);
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("price monitor cron shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    if let Err(error) = schedule_price_alerts(&pool).await {
                        error!(%error, "failed to schedule price monitor checks");
                    }
                }
            }
        }
    }

    async fn run_channel_reminder_cleanup(
        pool: PgPool,
        telegram_resolver: Arc<dyn TelegramClientResolver>,
        cleanup_interval: Duration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(?cleanup_interval, "channel reminder cleanup started");
        let mut interval = tokio::time::interval(cleanup_interval);
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("channel reminder cleanup shutting down");
                        break;
                    }
                }
                _ = interval.tick() => {
                    if let Err(error) = crate::channel_reminder_worker::cleanup_expired_channel_reminders(
                        &pool,
                        telegram_resolver.clone(),
                        100,
                    ).await {
                        error!(%error, "failed to clean expired channel reminders");
                    }
                }
            }
        }
    }

    /// Worker pool: receives claimed jobs and dispatches them concurrently.
    async fn run_worker_pool(
        pool: PgPool,
        dispatcher: Arc<Dispatcher>,
        worker_concurrency: usize,
        mut job_rx: mpsc::Receiver<ClaimedJob>,
        telemetry: WorkerTelemetry,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!("scheduler worker pool started");
        let semaphore = Arc::new(tokio::sync::Semaphore::new(worker_concurrency));

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
                            let telemetry = telemetry.clone();
                            let permit = semaphore.clone().acquire_owned().await;
                            tokio::spawn(async move {
                                let _permit = permit;
                                dispatcher.dispatch(&pool, &claimed, &telemetry).await;
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
        info!(
            "scheduler maintenance started, interval={:?}",
            config.maintenance_interval
        );
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

async fn persist_worker_telemetry(
    pool: &PgPool,
    worker_id: &str,
    snapshot: &WorkerTelemetrySnapshot,
) -> Result<()> {
    let now = Utc::now();
    let started_at = snapshot.global.scheduler_started_at.unwrap_or(now);
    let mut transaction = pool.begin().await?;

    sqlx::query(
        "INSERT INTO platform.worker_runtime_heartbeats (
             worker_id, started_at, last_seen_at, scheduler_started_at,
             last_poll_at, last_successful_poll_at, last_poll_error_at,
             total_polls, total_claimed_jobs, total_completed_jobs,
             total_failed_jobs, in_flight_jobs
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (worker_id) DO UPDATE SET
             started_at = EXCLUDED.started_at,
             last_seen_at = EXCLUDED.last_seen_at,
             scheduler_started_at = EXCLUDED.scheduler_started_at,
             last_poll_at = EXCLUDED.last_poll_at,
             last_successful_poll_at = EXCLUDED.last_successful_poll_at,
             last_poll_error_at = EXCLUDED.last_poll_error_at,
             total_polls = EXCLUDED.total_polls,
             total_claimed_jobs = EXCLUDED.total_claimed_jobs,
             total_completed_jobs = EXCLUDED.total_completed_jobs,
             total_failed_jobs = EXCLUDED.total_failed_jobs,
             in_flight_jobs = EXCLUDED.in_flight_jobs",
    )
    .bind(worker_id)
    .bind(started_at)
    .bind(now)
    .bind(snapshot.global.scheduler_started_at)
    .bind(snapshot.global.last_poll_at)
    .bind(snapshot.global.last_successful_poll_at)
    .bind(snapshot.global.last_poll_error_at)
    .bind(counter(snapshot.global.total_polls))
    .bind(counter(snapshot.global.total_claimed_jobs))
    .bind(counter(snapshot.global.total_completed_jobs))
    .bind(counter(snapshot.global.total_failed_jobs))
    .bind(counter(snapshot.global.in_flight_jobs))
    .execute(&mut *transaction)
    .await?;

    for (tenant_id, tenant) in &snapshot.tenants {
        sqlx::query(
            "INSERT INTO platform.worker_tenant_heartbeats (
                 worker_id, tenant_id, last_seen_at, claimed_jobs,
                 completed_jobs, failed_jobs, in_flight_jobs,
                 last_claimed_at, last_completed_at, last_failed_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (worker_id, tenant_id) DO UPDATE SET
                 last_seen_at = EXCLUDED.last_seen_at,
                 claimed_jobs = EXCLUDED.claimed_jobs,
                 completed_jobs = EXCLUDED.completed_jobs,
                 failed_jobs = EXCLUDED.failed_jobs,
                 in_flight_jobs = EXCLUDED.in_flight_jobs,
                 last_claimed_at = EXCLUDED.last_claimed_at,
                 last_completed_at = EXCLUDED.last_completed_at,
                 last_failed_at = EXCLUDED.last_failed_at",
        )
        .bind(worker_id)
        .bind(tenant_id)
        .bind(now)
        .bind(counter(tenant.claimed_jobs))
        .bind(counter(tenant.completed_jobs))
        .bind(counter(tenant.failed_jobs))
        .bind(counter(tenant.in_flight_jobs))
        .bind(tenant.last_claimed_at)
        .bind(tenant.last_completed_at)
        .bind(tenant.last_failed_at)
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;
    Ok(())
}

fn counter(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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
        SELECT id, tenant_id, task_name FROM broadcast.push_tasks
        WHERE status = 'running'
          AND next_run_at <= now()
          AND NOT EXISTS (
              SELECT 1
              FROM job_queue queued
              WHERE queued.job_type = 'push_task'
                AND queued.payload ->> 'push_task_id' = broadcast.push_tasks.id::text
                AND queued.status IN ('pending', 'running')
          )
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
        SELECT group_id, tenant_id FROM tracked_groups
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
        .await?;

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
            Ok(_job_id) => {
                enqueued += 1;
            }
            Err(e) => {
                error!(group_id, error = %e, "failed to enqueue group_policy_sync job");
            }
        }
    }

    Ok(enqueued)
}

/// Check formal `address_monitors` entries and enqueue tenant-scoped jobs.
#[instrument(skip(pool))]
pub async fn schedule_address_monitor_checks(pool: &PgPool) -> Result<usize> {
    let rows = sqlx::query(
        r#"
        SELECT m.monitor_id, m.tenant_id, m.address, m.network
        FROM address_monitors m
        WHERE m.is_active = true
          AND m.updated_at < now() - interval '30 seconds'
          AND NOT EXISTS (
              SELECT 1
              FROM job_queue queued
              WHERE queued.job_type = 'monitor_check'
                AND queued.tenant_id = m.tenant_id
                AND queued.payload ->> 'tenant_id' = m.tenant_id
                AND queued.payload ->> 'monitor_id' = m.monitor_id::text
                AND queued.status IN ('pending', 'running')
          )
        ORDER BY m.updated_at ASC
        LIMIT 50
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut enqueued = 0;
    for row in rows {
        let monitor_id: i64 = row.try_get("monitor_id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let address: String = row.try_get("address")?;
        let network: String = row.try_get("network")?;
        let opts = job_queue::EnqueueOpts::new(
            JobType::MonitorCheck,
            json!({
                "tenant_id": tenant_id,
                "monitor_id": monitor_id,
                "address": address,
                "network": network,
            }),
        )
        .with_priority(1)
        .with_max_attempts(3);

        match job_queue::enqueue(pool, opts).await {
            Ok(job_id) => {
                info!(
                    monitor_id,
                    tenant_id, job_id, "enqueued address monitor check"
                );
                enqueued += 1;
            }
            Err(error) => {
                error!(monitor_id, tenant_id, %error, "failed to enqueue address monitor check");
            }
        }
    }

    Ok(enqueued)
}

/// Check formal `platform.price_alerts` entries and enqueue tenant-scoped jobs.
#[instrument(skip(pool))]
pub async fn schedule_price_alerts(pool: &PgPool) -> Result<usize> {
    let rows = sqlx::query(
        r#"
        SELECT alert_id, tenant_id, user_id, symbol,
               threshold_high::float8 AS threshold_high,
               threshold_low::float8 AS threshold_low,
               notify_chat_id
        FROM platform.price_alerts
        WHERE is_active = true
          AND (last_checked_at IS NULL OR last_checked_at < now() - interval '5 minutes')
          AND NOT EXISTS (
              SELECT 1
              FROM job_queue queued
              WHERE queued.job_type = 'price_monitor_check'
                AND queued.payload ->> 'alert_id' = platform.price_alerts.alert_id::text
                AND queued.status IN ('pending', 'running')
          )
        ORDER BY last_checked_at ASC NULLS FIRST
        LIMIT 50
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut enqueued = 0;
    for row in rows {
        let alert_id: i64 = row.try_get("alert_id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let symbol: String = row.try_get("symbol")?;
        let threshold_high: Option<f64> = row.try_get("threshold_high").ok().flatten();
        let threshold_low: Option<f64> = row.try_get("threshold_low").ok().flatten();
        let notify_chat_id: Option<i64> = row.try_get("notify_chat_id").ok().flatten();
        let opts = job_queue::EnqueueOpts::new(
            JobType::Custom("price_monitor_check".into()),
            json!({
                "tenant_id": tenant_id,
                "alert_id": alert_id,
                "symbol": symbol,
                "threshold_high": threshold_high,
                "threshold_low": threshold_low,
                "monitor_task_type": "price_alert",
                "notify_chat_id": notify_chat_id,
            }),
        )
        .with_priority(2)
        .with_max_attempts(3);

        match job_queue::enqueue(pool, opts).await {
            Ok(job_id) => {
                info!(
                    alert_id,
                    tenant_id, job_id, "enqueued price monitor check"
                );
                enqueued += 1;
            }
            Err(error) => {
                error!(alert_id, tenant_id, %error, "failed to enqueue price monitor check");
            }
        }
    }

    Ok(enqueued)
}

/// Check for wallet/monitor entries that need to be checked and enqueue jobs.
#[cfg(feature = "legacy-workers")]
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
    .fetch_all(pool)
    .await?;

    let mut enqueued = 0;
    for row in rows {
        let monitor_id: i64 = row.try_get("id")?;
        let tenant_id: String = row.try_get("tenant_id")?;
        let address: String = row.try_get("address")?;
        let opts = job_queue::EnqueueOpts::new(
            JobType::MonitorCheck,
            json!({
                "tenant_id": tenant_id,
                "monitor_id": monitor_id,
                "address": address,
                "monitor_task_type": "wallet_balance"
            }),
        )
        .with_priority(1)
        .with_max_attempts(3);
        job_queue::enqueue(pool, opts).await?;
        enqueued += 1;
    }
    Ok(enqueued)
}

/// Run all cron-based scheduling checks.
#[cfg(feature = "legacy-workers")]
pub async fn run_cron_checks(pool: &PgPool) -> Result<(usize, usize, usize)> {
    let push = schedule_push_tasks(pool).await?;
    let group = schedule_group_policy_syncs(pool).await?;
    let monitor = schedule_monitor_checks(pool).await?;
    Ok((push, group, monitor))
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
        assert_eq!(config.effective_worker_concurrency(), 4);
        assert_eq!(config.stale_job_timeout_minutes, 30);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(10));
    }

    #[test]
    fn zero_worker_concurrency_is_clamped_to_one() {
        let config = SchedulerConfig {
            worker_concurrency: 0,
            ..SchedulerConfig::default()
        };
        assert_eq!(config.effective_worker_concurrency(), 1);
    }
}
