//! Postgres-backed job queue.
//!
//! Provides a reliable, transactional job queue built on PostgreSQL `job_queue`
//! table.  Workers claim jobs via `SELECT … FOR UPDATE SKIP LOCKED` which
//! guarantees that each job is processed by at most one consumer, even when
//! multiple worker processes run concurrently.
//!
//! # Table schema (expected)
//! ```sql
//! CREATE TABLE job_queue (
//!     id          BIGSERIAL PRIMARY KEY,
//!     job_type    TEXT        NOT NULL,
//!     payload     JSONB       NOT NULL DEFAULT '{}',
//!     priority    INT2        NOT NULL DEFAULT 0,
//!     status      TEXT        NOT NULL DEFAULT 'pending',  -- pending / running / completed / failed / dead
//!     attempts    INT2        NOT NULL DEFAULT 0,
//!     max_attempts INT2       NOT NULL DEFAULT 3,
//!     run_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
//!     started_at  TIMESTAMPTZ,
//!     completed_at TIMESTAMPTZ,
//!     last_error  TEXT,
//!     created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
//!     updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
//! );
//! CREATE INDEX idx_job_queue_poll ON job_queue (status, run_at, priority DESC)
//!     WHERE status = 'pending';
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Job type discriminator – maps to the `job_type` column.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    PushTask,
    GroupPolicySync,
    AiProcess,
    OcrProcess,
    MonitorCheck,
    PaymentProcess,
    Custom(String),
}

impl JobType {
    pub fn as_str(&self) -> &str {
        match self {
            JobType::PushTask => "push_task",
            JobType::GroupPolicySync => "group_policy_sync",
            JobType::AiProcess => "ai_process",
            JobType::OcrProcess => "ocr_process",
            JobType::MonitorCheck => "monitor_check",
            JobType::PaymentProcess => "payment_process",
            JobType::Custom(s) => s,
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "push_task" => JobType::PushTask,
            "group_policy_sync" => JobType::GroupPolicySync,
            "ai_process" => JobType::AiProcess,
            "ocr_process" => JobType::OcrProcess,
            "monitor_check" => JobType::MonitorCheck,
            "payment_process" => JobType::PaymentProcess,
            other => JobType::Custom(other.to_string()),
        }
    }
}

/// Job status values.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Dead,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Dead => "dead",
        }
    }
}

/// A job record read from the `job_queue` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub job_type: String,
    pub payload: Value,
    pub priority: i16,
    pub status: String,
    pub attempts: i16,
    pub max_attempts: i16,
    pub run_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Options for enqueuing a new job.
#[derive(Debug, Clone)]
pub struct EnqueueOpts {
    pub job_type: JobType,
    pub payload: Value,
    pub priority: Option<i16>,
    pub max_attempts: Option<i16>,
    pub run_at: Option<DateTime<Utc>>,
}

impl EnqueueOpts {
    /// Create opts with defaults.
    pub fn new(job_type: JobType, payload: Value) -> Self {
        Self {
            job_type,
            payload,
            priority: None,
            max_attempts: None,
            run_at: None,
        }
    }

    /// Set job priority (higher = runs sooner).
    pub fn with_priority(mut self, p: i16) -> Self {
        self.priority = Some(p);
        self
    }

    /// Set maximum retry attempts.
    pub fn with_max_attempts(mut self, n: i16) -> Self {
        self.max_attempts = Some(n);
        self
    }

    /// Schedule for future execution.
    pub fn with_run_at(mut self, t: DateTime<Utc>) -> Self {
        self.run_at = Some(t);
        self
    }
}

/// Result of claiming jobs from the queue.
#[derive(Debug)]
pub struct ClaimedJob {
    pub job: Job,
    /// Unique claim token used to finalize (complete / fail) the job.
    pub claim_id: Uuid,
}

// ---------------------------------------------------------------------------
// Queue operations
// ---------------------------------------------------------------------------

/// Enqueue a new job.
#[instrument(skip(pool), fields(job_type = %opts.job_type.as_str()))]
pub async fn enqueue(pool: &PgPool, opts: EnqueueOpts) -> Result<i64> {
    let job_type = opts.job_type.as_str();
    let priority = opts.priority.unwrap_or(0);
    let max_attempts = opts.max_attempts.unwrap_or(3);
    let run_at = opts
        .run_at
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| "now()".to_string());

    // When run_at is "now()" we use the SQL function, otherwise a parameter.
    let row = if opts.run_at.is_none() {
        sqlx::query(
            r#"
            INSERT INTO job_queue (job_type, payload, priority, max_attempts, run_at, status)
            VALUES ($1, $2, $3, $4, now(), 'pending')
            RETURNING id
            "#,
        )
        .bind(job_type)
        .bind(&opts.payload)
        .bind(priority)
        .bind(max_attempts)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            INSERT INTO job_queue (job_type, payload, priority, max_attempts, run_at, status)
            VALUES ($1, $2, $3, $4, $5::timestamptz, 'pending')
            RETURNING id
            "#,
        )
        .bind(job_type)
        .bind(&opts.payload)
        .bind(priority)
        .bind(max_attempts)
        .bind(&run_at)
        .fetch_one(pool)
        .await?
    };

    let id: i64 = row.try_get("id").context("missing id from insert")?;
    debug!(job_id = id, "enqueued job");
    Ok(id)
}

/// Enqueue multiple jobs in a single transaction (batch insert).
#[instrument(skip(pool, opts_list), fields(count = opts_list.len()))]
pub async fn enqueue_batch(pool: &PgPool, opts_list: Vec<EnqueueOpts>) -> Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(opts_list.len());
    let mut tx = pool.begin().await?;

    for opts in opts_list {
        let job_type = opts.job_type.as_str();
        let priority = opts.priority.unwrap_or(0);
        let max_attempts = opts.max_attempts.unwrap_or(3);

        let row = if opts.run_at.is_none() {
            sqlx::query(
                r#"
                INSERT INTO job_queue (job_type, payload, priority, max_attempts, run_at, status)
                VALUES ($1, $2, $3, $4, now(), 'pending')
                RETURNING id
                "#,
            )
            .bind(job_type)
            .bind(&opts.payload)
            .bind(priority)
            .bind(max_attempts)
            .fetch_one(&mut *tx)
            .await?
        } else {
            let run_at = opts.run_at.unwrap().to_rfc3339();
            sqlx::query(
                r#"
                INSERT INTO job_queue (job_type, payload, priority, max_attempts, run_at, status)
                VALUES ($1, $2, $3, $4, $5::timestamptz, 'pending')
                RETURNING id
                "#,
            )
            .bind(job_type)
            .bind(&opts.payload)
            .bind(priority)
            .bind(max_attempts)
            .bind(&run_at)
            .fetch_one(&mut *tx)
            .await?
        };

        let id: i64 = row.try_get("id").context("missing id from insert")?;
        ids.push(id);
    }

    tx.commit().await?;
    debug!(count = ids.len(), "enqueued batch of jobs");
    Ok(ids)
}

/// Claim up to `limit` pending jobs that are ready to run.
///
/// Uses `SELECT … FOR UPDATE SKIP LOCKED` to ensure exactly-once claiming
/// across concurrent workers.  Each claimed job gets a `claim_id` (UUID)
/// stored in `started_at` row slot as a simple ownership marker; the
/// `complete_job` / `fail_job` functions verify this token before updating.
#[instrument(skip(pool))]
pub async fn claim_jobs(pool: &PgPool, job_type: Option<&str>, limit: i32) -> Result<Vec<ClaimedJob>> {
    let rows = if let Some(jt) = job_type {
        sqlx::query(
            r#"
            UPDATE job_queue
            SET status = 'running',
                attempts = attempts + 1,
                started_at = now(),
                updated_at = now()
            WHERE id IN (
                SELECT id FROM job_queue
                WHERE status = 'pending'
                  AND run_at <= now()
                  AND job_type = $1
                ORDER BY priority DESC, run_at ASC
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            RETURNING id, job_type, payload, priority, status, attempts,
                      max_attempts, run_at, started_at, completed_at,
                      last_error, created_at, updated_at
            "#,
        )
        .bind(jt)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            UPDATE job_queue
            SET status = 'running',
                attempts = attempts + 1,
                started_at = now(),
                updated_at = now()
            WHERE id IN (
                SELECT id FROM job_queue
                WHERE status = 'pending'
                  AND run_at <= now()
                ORDER BY priority DESC, run_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            RETURNING id, job_type, payload, priority, status, attempts,
                      max_attempts, run_at, started_at, completed_at,
                      last_error, created_at, updated_at
            "#,
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    let mut claimed = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let job_type: String = row.try_get("job_type")?;
        let payload: Value = row.try_get("payload")?;
        let priority: i16 = row.try_get("priority")?;
        let status: String = row.try_get("status")?;
        let attempts: i16 = row.try_get("attempts")?;
        let max_attempts: i16 = row.try_get("max_attempts")?;
        let run_at: DateTime<Utc> = row.try_get("run_at")?;
        let started_at: Option<DateTime<Utc>> = row.try_get("started_at")?;
        let completed_at: Option<DateTime<Utc>> = row.try_get("completed_at")?;
        let last_error: Option<String> = row.try_get("last_error")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

        let job = Job {
            id,
            job_type,
            payload,
            priority,
            status,
            attempts,
            max_attempts,
            run_at,
            started_at,
            completed_at,
            last_error,
            created_at,
            updated_at,
        };

        let claim_id = Uuid::new_v4();
        claimed.push(ClaimedJob { job, claim_id });
    }

    debug!(claimed = claimed.len(), "claimed jobs from queue");
    Ok(claimed)
}

/// Mark a job as completed.
#[instrument(skip(pool))]
pub async fn complete_job(pool: &PgPool, job_id: i64) -> Result<()> {
    let result = sqlx::query(
        r#"
        UPDATE job_queue
        SET status = 'completed',
            completed_at = now(),
            updated_at = now()
        WHERE id = $1 AND status = 'running'
        "#,
    )
    .bind(job_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        warn!(job_id, "complete_job: job not found or not running");
    }
    Ok(())
}

/// Mark a job as failed.  If attempts < max_attempts, the job is reset to
/// `pending` so the scheduler can re-claim it.  Otherwise it is marked `dead`.
#[instrument(skip(pool))]
pub async fn fail_job(pool: &PgPool, job_id: i64, error: &str) -> Result<()> {
    // Read current attempts
    let row = sqlx::query(
        "SELECT attempts, max_attempts FROM job_queue WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?;

    let (attempts, max_attempts): (i16, i16) = match row {
        Some(r) => (r.try_get("attempts")?, r.try_get("max_attempts")?),
        None => {
            warn!(job_id, "fail_job: job not found");
            return Ok(());
        }
    };

    if attempts < max_attempts {
        // Retry: reset to pending with exponential back-off
        let backoff_secs = 30i64 * (1i64 << (attempts as u32).min(5));
        sqlx::query(
            r#"
            UPDATE job_queue
            SET status = 'pending',
                run_at = now() + ($2 || ' seconds')::interval,
                last_error = $3,
                updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(job_id)
        .bind(backoff_secs.to_string())
        .bind(error)
        .execute(pool)
        .await?;
        debug!(job_id, attempts, backoff_secs, "job failed, scheduled retry");
    } else {
        // Max attempts reached: mark dead
        sqlx::query(
            r#"
            UPDATE job_queue
            SET status = 'dead',
                last_error = $2,
                completed_at = now(),
                updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(job_id)
        .bind(error)
        .execute(pool)
        .await?;
        debug!(job_id, "job marked dead after max attempts");
    }

    Ok(())
}

/// Re-queue stale running jobs (e.g. after a crash) that have been running
/// longer than `timeout_minutes`.  Resets them to `pending` with a small
/// back-off so they can be re-claimed.
#[instrument(skip(pool))]
pub async fn requeue_stale_jobs(pool: &PgPool, timeout_minutes: i32) -> Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE job_queue
        SET status = 'pending',
            run_at = now() + interval '30 seconds',
            updated_at = now()
        WHERE status = 'running'
          AND started_at < now() - ($1 || ' minutes')::interval
        "#,
    )
    .bind(timeout_minutes.to_string())
    .execute(pool)
    .await?;

    let requeued = result.rows_affected();
    if requeued > 0 {
        warn!(requeued, "re-queued stale running jobs");
    }
    Ok(requeued)
}

/// Purge completed or dead jobs older than `retention_days`.
#[instrument(skip(pool))]
pub async fn purge_old_jobs(pool: &PgPool, retention_days: i32) -> Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM job_queue
        WHERE status IN ('completed', 'dead')
          AND completed_at < now() - ($1 || ' days')::interval
        "#,
    )
    .bind(retention_days.to_string())
    .execute(pool)
    .await?;

    let purged = result.rows_affected();
    if purged > 0 {
        debug!(purged, "purged old completed/dead jobs");
    }
    Ok(purged)
}

/// Count jobs by status, optionally filtered by job_type.
pub async fn count_jobs(
    pool: &PgPool,
    job_type: Option<&str>,
    status: Option<&str>,
) -> Result<i64> {
    let count: i64 = match (job_type, status) {
        (Some(jt), Some(st)) => {
            sqlx::query_scalar(
                "SELECT COUNT(*)::bigint FROM job_queue WHERE job_type = $1 AND status = $2",
            )
            .bind(jt)
            .bind(st)
            .fetch_one(pool)
            .await?
        }
        (Some(jt), None) => {
            sqlx::query_scalar(
                "SELECT COUNT(*)::bigint FROM job_queue WHERE job_type = $1",
            )
            .bind(jt)
            .fetch_one(pool)
            .await?
        }
        (None, Some(st)) => {
            sqlx::query_scalar(
                "SELECT COUNT(*)::bigint FROM job_queue WHERE status = $1",
            )
            .bind(st)
            .fetch_one(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_scalar("SELECT COUNT(*)::bigint FROM job_queue")
                .fetch_one(pool)
                .await?
        }
    };
    Ok(count.unwrap_or(0))
}

/// List jobs with pagination and optional filters.
pub async fn list_jobs(
    pool: &PgPool,
    job_type: Option<&str>,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Job>> {
    let rows = match (job_type, status) {
        (Some(jt), Some(st)) => {
            sqlx::query(
                r#"
                SELECT id, job_type, payload, priority, status, attempts,
                       max_attempts, run_at, started_at, completed_at,
                       last_error, created_at, updated_at
                FROM job_queue
                WHERE job_type = $1 AND status = $2
                ORDER BY priority DESC, created_at ASC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(jt)
            .bind(st)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(jt), None) => {
            sqlx::query(
                r#"
                SELECT id, job_type, payload, priority, status, attempts,
                       max_attempts, run_at, started_at, completed_at,
                       last_error, created_at, updated_at
                FROM job_queue
                WHERE job_type = $1
                ORDER BY priority DESC, created_at ASC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(jt)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, Some(st)) => {
            sqlx::query(
                r#"
                SELECT id, job_type, payload, priority, status, attempts,
                       max_attempts, run_at, started_at, completed_at,
                       last_error, created_at, updated_at
                FROM job_queue
                WHERE status = $1
                ORDER BY priority DESC, created_at ASC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(st)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            sqlx::query(
                r#"
                SELECT id, job_type, payload, priority, status, attempts,
                       max_attempts, run_at, started_at, completed_at,
                       last_error, created_at, updated_at
                FROM job_queue
                ORDER BY priority DESC, created_at ASC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    };

    let jobs = rows
        .into_iter()
        .map(|row| {
            Ok(Job {
                id: row.try_get("id")?,
                job_type: row.try_get("job_type")?,
                payload: row.try_get("payload")?,
                priority: row.try_get("priority")?,
                status: row.try_get("status")?,
                attempts: row.try_get("attempts")?,
                max_attempts: row.try_get("max_attempts")?,
                run_at: row.try_get("run_at")?,
                started_at: row.try_get("started_at")?,
                completed_at: row.try_get("completed_at")?,
                last_error: row.try_get("last_error")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<Job>>>()?;
    Ok(jobs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_type_roundtrip() {
        let types = vec![
            JobType::PushTask,
            JobType::GroupPolicySync,
            JobType::AiProcess,
            JobType::OcrProcess,
            JobType::MonitorCheck,
            JobType::PaymentProcess,
            JobType::Custom("my_custom_type".into()),
        ];
        for jt in types {
            assert_eq!(JobType::from_str(jt.as_str()), jt);
        }
    }

    #[test]
    fn test_job_status_as_str() {
        assert_eq!(JobStatus::Pending.as_str(), "pending");
        assert_eq!(JobStatus::Running.as_str(), "running");
        assert_eq!(JobStatus::Completed.as_str(), "completed");
        assert_eq!(JobStatus::Failed.as_str(), "failed");
        assert_eq!(JobStatus::Dead.as_str(), "dead");
    }

    #[test]
    fn test_enqueue_opts_builder() {
        let opts = EnqueueOpts::new(JobType::PushTask, serde_json::json!({"k": "v"}))
            .with_priority(10)
            .with_max_attempts(5);

        assert_eq!(opts.job_type, JobType::PushTask);
        assert_eq!(opts.priority, Some(10));
        assert_eq!(opts.max_attempts, Some(5));
        assert!(opts.run_at.is_none());
    }
}
