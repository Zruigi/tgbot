//! Disposable PostgreSQL behavior coverage for the durable job queue.
//!
//! The test is ignored by default because it mutates only a disposable local
//! PostgreSQL database. Run it with `BETA_JOB_QUEUE_TEST_DATABASE_URL` set to a
//! database that has the Beta migrations applied.

use chrono::Utc;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Row};
use std::error::Error;
use tgbot_workers::job_queue::{self, EnqueueOpts, JobType};
use uuid::Uuid;

async fn fixture_pool() -> Result<sqlx::PgPool, Box<dyn Error>> {
    let database_url = std::env::var("BETA_JOB_QUEUE_TEST_DATABASE_URL").map_err(|_| {
        "BETA_JOB_QUEUE_TEST_DATABASE_URL is required; use a disposable migrated Beta database"
    })?;
    Ok(PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?)
}

#[tokio::test]
#[ignore = "requires an explicitly supplied disposable PostgreSQL database"]
async fn claim_token_retry_backoff_and_stale_requeue_are_fail_closed() -> Result<(), Box<dyn Error>>
{
    let pool = fixture_pool().await?;
    let job_type = format!("queue_lifecycle_fixture_{}", Uuid::new_v4().simple());
    let retry_tenant = format!("queue-lifecycle-retry-{}", Uuid::new_v4());
    let stale_tenant = format!("queue-lifecycle-stale-{}", Uuid::new_v4());

    let retry_job_id = job_queue::enqueue(
        &pool,
        EnqueueOpts::new(
            JobType::Custom(job_type.clone()),
            json!({"tenant_id": retry_tenant, "fixture": "retry"}),
        )
        .with_max_attempts(2),
    )
    .await?;
    let claimed = job_queue::claim_jobs(&pool, std::slice::from_ref(&job_type), 1).await?;
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].job.id, retry_job_id);
    assert_eq!(claimed[0].job.status, "running");
    assert_eq!(claimed[0].job.attempts, 1);
    let first_claim_id = claimed[0].claim_id;

    let wrong_claim_id = Uuid::new_v4();
    job_queue::complete_job(&pool, retry_job_id, wrong_claim_id).await?;
    job_queue::fail_job(&pool, retry_job_id, wrong_claim_id, "wrong owner").await?;
    let row = sqlx::query(
        "SELECT status, attempts, claim_id, run_at, last_error FROM job_queue WHERE id = $1",
    )
    .bind(retry_job_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row.try_get::<String, _>("status")?, "running");
    assert_eq!(row.try_get::<i16, _>("attempts")?, 1);
    assert_eq!(row.try_get::<Uuid, _>("claim_id")?, first_claim_id);
    assert!(row.try_get::<Option<String>, _>("last_error")?.is_none());

    job_queue::fail_job(&pool, retry_job_id, first_claim_id, "fixture retry").await?;
    let row = sqlx::query(
        "SELECT status, attempts, claim_id, run_at, last_error FROM job_queue WHERE id = $1",
    )
    .bind(retry_job_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row.try_get::<String, _>("status")?, "pending");
    assert_eq!(row.try_get::<i16, _>("attempts")?, 1);
    assert!(row.try_get::<Option<Uuid>, _>("claim_id")?.is_none());
    assert!(row.try_get::<chrono::DateTime<Utc>, _>("run_at")? > Utc::now());
    assert_eq!(
        row.try_get::<Option<String>, _>("last_error")?.as_deref(),
        Some("fixture retry")
    );

    // Advance the disposable clock instead of sleeping through the backoff.
    sqlx::query("UPDATE job_queue SET run_at = now() WHERE id = $1")
        .bind(retry_job_id)
        .execute(&pool)
        .await?;
    let reclaimed = job_queue::claim_jobs(&pool, std::slice::from_ref(&job_type), 1).await?;
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].job.id, retry_job_id);
    assert_eq!(reclaimed[0].job.attempts, 2);
    assert_ne!(reclaimed[0].claim_id, first_claim_id);
    job_queue::fail_job(&pool, retry_job_id, reclaimed[0].claim_id, "fixture dead").await?;
    let row = sqlx::query("SELECT status, completed_at, claim_id FROM job_queue WHERE id = $1")
        .bind(retry_job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(row.try_get::<String, _>("status")?, "dead");
    assert!(row
        .try_get::<Option<chrono::DateTime<Utc>>, _>("completed_at")?
        .is_some());
    assert!(row.try_get::<Option<Uuid>, _>("claim_id")?.is_none());

    let stale_job_id = job_queue::enqueue(
        &pool,
        EnqueueOpts::new(
            JobType::Custom(job_type.clone()),
            json!({"tenant_id": stale_tenant, "fixture": "stale"}),
        ),
    )
    .await?;
    let stale_claimed = job_queue::claim_jobs(&pool, std::slice::from_ref(&job_type), 1).await?;
    assert_eq!(stale_claimed.len(), 1);
    assert_eq!(stale_claimed[0].job.id, stale_job_id);
    sqlx::query("UPDATE job_queue SET started_at = now() - interval '31 minutes' WHERE id = $1")
        .bind(stale_job_id)
        .execute(&pool)
        .await?;
    assert!(job_queue::requeue_stale_jobs(&pool, 30).await? >= 1);
    let row = sqlx::query("SELECT status, claim_id, run_at FROM job_queue WHERE id = $1")
        .bind(stale_job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(row.try_get::<String, _>("status")?, "pending");
    assert!(row.try_get::<Option<Uuid>, _>("claim_id")?.is_none());
    assert!(row.try_get::<chrono::DateTime<Utc>, _>("run_at")? > Utc::now());

    for tenant_id in [&retry_tenant, &stale_tenant] {
        sqlx::query("DELETE FROM job_queue WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await?;
    }
    pool.close().await;
    Ok(())
}
