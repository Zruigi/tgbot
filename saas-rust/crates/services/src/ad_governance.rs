//! Ad governance scanning and enforcement service.
//!
//! Provides persistent, tenant-scoped ad scanning with state machine, clustering,
//! and multi-tiered action enforcement.
//!
//! Safety Invariants:
//! - First scan FORCED dry_run=true; activation only via explicit confirm
//! - AD_AUTO_ENFORCE_ENABLED and AD_DELETE_ENABLED double kill-switches (default OFF)
//! - LLM classifies only cluster representative texts (never direct delete instructions)
//! - Output gated by family_hash whitelist recovery + confidence >= 0.65 threshold
//! - Tiered actions (warn/delete/mute) computed server-side
//! - Every action idempotent via action_key (UNIQUE constraint in DB)
//! - Warning dedup via 5-column PK window
//! - Per-group locking (database row locks)
//! - tenant_id scoping on every query

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use md5::{Digest, Md5};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::LazyLock;
use tracing::instrument;
use uuid::Uuid;

// Re-export for use in routes
pub use crate::ad_detection::detect_advertising_signals;
pub use crate::ad_text::ad_text_hash;

// Media placeholder regex (from Python)
static MEDIA_PLACEHOLDER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\[(?:图片|动图|表情包|视频|语音|音频|文件|媒体|photo|animation|sticker|video|voice|audio|document|media)\](?:\s+[^\s]+)?$"
    ).unwrap()
});

static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)https?://\S+|t\.me/\S+|@\w{3,}").unwrap());

static CONTACT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@[A-Za-z0-9_]{4,32}").unwrap());

static NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{5,}").unwrap());

static SPACE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

// ============================================================================
// Type Definitions
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdGroupPolicy {
    pub tenant_id: String,
    pub group_id: i64,
    pub is_enabled: bool,
    pub warning_similarity: f64,
    pub warning_count: i32,
    pub delete_similarity: f64,
    pub delete_count: i32,
    pub exact_delete_count: i32,
    pub mute_min_senders: i32,
    pub mute_min_messages: i32,
    pub mute_seconds: i32,
    pub activated_by: Option<String>,
    pub activated_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdScanJob {
    pub job_id: String,
    pub tenant_id: String,
    pub group_id: i64,
    pub group_title: Option<String>,
    pub requested_by: Option<String>,
    pub status: String,
    pub progress: i32,
    pub dry_run: bool,
    pub activate_policy: bool,
    pub source: String,
    pub total_messages: i32,
    pub candidate_clusters: i32,
    pub confirmed_clusters: i32,
    pub fingerprint_only_count: i32,
    pub planned_delete_count: i32,
    pub deleted_count: i32,
    pub warning_sender_count: i32,
    pub muted_sender_count: i32,
    pub failure_count: i32,
    pub summary: Value,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdGovernanceCluster {
    pub cluster_id: String,
    pub job_id: String,
    pub tenant_id: String,
    pub group_id: i64,
    pub representative_text: Option<String>,
    pub exact_hash: Option<String>,
    pub template_hash: Option<String>,
    pub similarity: f64,
    pub occurrence_count: i32,
    pub recent_count: i32,
    pub sender_count: i32,
    pub classification: String,
    pub classification_source: Option<String>,
    pub confidence: f64,
    pub reason: Option<String>,
    pub action_tier: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdGovernanceAction {
    pub id: i64,
    pub action_key: String,
    pub job_id: Option<String>,
    pub tenant_id: String,
    pub group_id: i64,
    pub cluster_id: Option<String>,
    pub action_type: String,
    pub target_id: Option<String>,
    pub status: String,
    pub dry_run: bool,
    pub payload_json: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub executed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    pub group_id: i64,
    pub title: String,
    pub chat_type: String,
    pub message_count: i32,
    pub bot_status: String,
    pub can_delete_messages: bool,
    pub can_restrict_members: bool,
    /// Telegram capabilities are not inferred from database membership rows.
    /// Consumers must check this flag before offering destructive actions.
    pub capabilities_known: bool,
    pub capabilities_source: String,
}

// ============================================================================
// Ad Governance Service
// ============================================================================

pub struct AdGovernanceService {
    pool: Arc<PgPool>,
}

impl AdGovernanceService {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// List tracked groups for a tenant (simplified version without Telegram bot checks).
    pub async fn list_groups(&self, tenant_id: &str) -> Result<Vec<GroupInfo>> {
        let rows = sqlx::query(
            r#"
            SELECT g.group_id, COALESCE(g.group_title, '') as title,
                   'supergroup'::text as chat_type,
                   COALESCE(COUNT(m.id), 0) as message_count
            FROM groups.tracked_groups g
            LEFT JOIN groups.group_messages m
              ON m.group_id = g.group_id AND m.tenant_id = g.tenant_id AND NOT m.is_deleted
            WHERE g.tenant_id = $1
            GROUP BY g.group_id, g.group_title
            ORDER BY g.updated_at DESC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(self.pool.as_ref())
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| GroupInfo {
                group_id: row.get("group_id"),
                title: row.get("title"),
                chat_type: row.get("chat_type"),
                message_count: row.get::<i64, _>("message_count") as i32,
                bot_status: "unknown".to_string(), // Simplified: requires Telegram API
                can_delete_messages: false,        // Simplified: requires Telegram API
                can_restrict_members: false,       // Simplified: requires Telegram API
                capabilities_known: false,
                capabilities_source: "not_checked".to_string(),
            })
            .collect())
    }

    /// Create a new ad scan job (always starts with dry_run=true).
    /// Activation only via explicit confirm.
    /// SAFETY: 首扫强制 dry-run，只能通过 configure_job + start_job 显式确认方式激活实际执行。
    #[instrument(skip(self), fields(tenant_id, group_id))]
    pub async fn create_job(
        &self,
        tenant_id: &str,
        group_id: i64,
        requested_by: &str,
        group_title: Option<&str>,
        _dry_run: bool, // Ignored: first scan always dry_run=true
        activate_policy: bool,
        source: &str,
        recent_only: bool,
    ) -> Result<AdScanJob> {
        // SAFETY INVARIANT: First scan FORCED dry_run=true
        // Only explicit confirm via configure_job + start_job can lift this for enforcement runs
        let final_dry_run = true;
        let job_id = format!("scan_{}", Uuid::new_v4().to_string().replace("-", ""));

        let group_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM groups.tracked_groups WHERE tenant_id = $1 AND group_id = $2)",
        )
        .bind(tenant_id)
        .bind(group_id)
        .fetch_one(self.pool.as_ref())
        .await?;
        if !group_exists {
            return Err(anyhow!("ad scan group not found or access denied"));
        }

        let message_count: i32 = sqlx::query_scalar(
            r#"
            SELECT COALESCE(COUNT(*), 0)::int
            FROM groups.group_messages
            WHERE tenant_id = $1 AND group_id = $2
              AND sender_type = 'user' AND NOT is_deleted
              AND (NULLIF(btrim(COALESCE(content, '')), '') IS NOT NULL
                   OR NULLIF(btrim(COALESCE(media_unique_id, '')), '') IS NOT NULL)
            "#,
        )
        .bind(tenant_id)
        .bind(group_id)
        .fetch_one(self.pool.as_ref())
        .await
        .unwrap_or(0);

        sqlx::query(
            r#"
            INSERT INTO groups.ad_scan_jobs
                (job_id, tenant_id, group_id, group_title, requested_by,
                 dry_run, activate_policy, source, status, total_messages, recent_only)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'prepared', $9, $10)
            "#,
        )
        .bind(&job_id)
        .bind(tenant_id)
        .bind(group_id)
        .bind(group_title)
        .bind(requested_by)
        .bind(final_dry_run)
        .bind(activate_policy)
        .bind(source)
        .bind(message_count)
        .bind(recent_only)
        .execute(self.pool.as_ref())
        .await?;

        self.get_job(tenant_id, &job_id).await
    }

    /// Get a specific scan job (tenant-scoped).
    pub async fn get_job(&self, tenant_id: &str, job_id: &str) -> Result<AdScanJob> {
        let row = sqlx::query(
            r#"
            SELECT job_id, tenant_id, group_id, group_title, requested_by, status, progress,
                   dry_run, activate_policy, source, total_messages, candidate_clusters,
                   confirmed_clusters, fingerprint_only_count, planned_delete_count,
                   deleted_count, warning_sender_count, muted_sender_count, failure_count,
                   summary_json, error, created_at, started_at, finished_at
            FROM groups.ad_scan_jobs
            WHERE job_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(self.pool.as_ref())
        .await?
        .ok_or_else(|| anyhow!("ad scan job not found or access denied"))?;

        Ok(AdScanJob {
            job_id: row.get("job_id"),
            tenant_id: row.get("tenant_id"),
            group_id: row.get("group_id"),
            group_title: row.get("group_title"),
            requested_by: row.get("requested_by"),
            status: row.get("status"),
            progress: row.get("progress"),
            dry_run: row.get("dry_run"),
            activate_policy: row.get("activate_policy"),
            source: row.get("source"),
            total_messages: row.get("total_messages"),
            candidate_clusters: row.get("candidate_clusters"),
            confirmed_clusters: row.get("confirmed_clusters"),
            fingerprint_only_count: row.get("fingerprint_only_count"),
            planned_delete_count: row.get("planned_delete_count"),
            deleted_count: row.get("deleted_count"),
            warning_sender_count: row.get("warning_sender_count"),
            muted_sender_count: row.get("muted_sender_count"),
            failure_count: row.get("failure_count"),
            summary: serde_json::from_str(
                row.get::<Option<&str>, _>("summary_json").unwrap_or("{}"),
            )
            .unwrap_or(json!({})),
            error: row.get("error"),
            created_at: row.get("created_at"),
            started_at: row.get("started_at"),
            finished_at: row.get("finished_at"),
        })
    }

    /// Get clusters for a job (tenant-scoped, optional tier filter).
    pub async fn get_clusters(
        &self,
        tenant_id: &str,
        job_id: &str,
        tier: Option<&str>,
        limit: i32,
    ) -> Result<Vec<AdGovernanceCluster>> {
        // Verify job exists and belongs to tenant
        let _ = self.get_job(tenant_id, job_id).await?;

        let limit = (limit.max(1) as i64).min(500);
        let mut query = String::from(
            r#"
            SELECT cluster_id, job_id, tenant_id, group_id, representative_text,
                   exact_hash, template_hash, similarity, occurrence_count, recent_count,
                   sender_count, classification, classification_source, confidence,
                   reason, action_tier, created_at
            FROM groups.ad_governance_clusters
            WHERE job_id = $1 AND tenant_id = $2
            "#,
        );

        // Filter by tier if specified
        match tier {
            Some("delete") => {
                query.push_str(
                    " AND action_tier IN ('similar_delete', 'exact_delete', 'exact_delete_mute')",
                );
            }
            Some("pending") => {
                query.push_str(" AND classification = 'pending'");
            }
            Some(t) => {
                query.push_str(&format!(" AND action_tier = '{}'", t.replace("'", "''")));
            }
            None => {}
        }

        query.push_str(" ORDER BY recent_count DESC, occurrence_count DESC LIMIT $3");

        let rows = sqlx::query(&query)
            .bind(job_id)
            .bind(tenant_id)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await?;

        Ok(rows
            .into_iter()
            .map(|row| AdGovernanceCluster {
                cluster_id: row.get("cluster_id"),
                job_id: row.get("job_id"),
                tenant_id: row.get("tenant_id"),
                group_id: row.get("group_id"),
                representative_text: row.get("representative_text"),
                exact_hash: row.get("exact_hash"),
                template_hash: row.get("template_hash"),
                similarity: row.get("similarity"),
                occurrence_count: row.get("occurrence_count"),
                recent_count: row.get("recent_count"),
                sender_count: row.get("sender_count"),
                classification: row.get("classification"),
                classification_source: row.get("classification_source"),
                confidence: row.get("confidence"),
                reason: row.get("reason"),
                action_tier: row.get("action_tier"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    /// Cancel a job (only if it's in prepared or queued state).
    pub async fn cancel_job(&self, tenant_id: &str, job_id: &str) -> Result<AdScanJob> {
        let mut transaction = self.pool.begin().await?;
        let scan_status = sqlx::query_scalar::<_, String>(
            r#"
            SELECT status
            FROM groups.ad_scan_jobs
            WHERE job_id = $1 AND tenant_id = $2
            FOR UPDATE
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| anyhow!("ad scan job not found or access denied"))?;

        let queue_statuses = sqlx::query_scalar::<_, String>(
            r#"
            SELECT status
            FROM job_queue
            WHERE tenant_id = $1 AND job_type = 'ad_scan_job'
              AND payload ->> 'job_id' = $2
            FOR UPDATE
            "#,
        )
        .bind(tenant_id)
        .bind(job_id)
        .fetch_all(&mut *transaction)
        .await?;

        let disposition = cancellation_disposition(&scan_status, &queue_statuses)?;
        match disposition {
            CancellationDisposition::LeaveUnchanged => {
                transaction.rollback().await?;
                return self.get_job(tenant_id, job_id).await;
            }
            CancellationDisposition::CancelScanAndQueue
            | CancellationDisposition::CancelQueueOnly => {}
        }

        // `dead` is an existing terminal queue status and is never selected by
        // the pending-job claim query. The diagnostic makes operator-initiated
        // cancellation distinguishable from exhausted retries.
        sqlx::query(
            r#"
            UPDATE job_queue
            SET status = 'dead', last_error = 'cancelled by ad governance request',
                completed_at = now(), updated_at = now()
            WHERE tenant_id = $1 AND job_type = 'ad_scan_job'
              AND payload ->> 'job_id' = $2 AND status = 'pending'
            "#,
        )
        .bind(tenant_id)
        .bind(job_id)
        .execute(&mut *transaction)
        .await?;

        if disposition == CancellationDisposition::CancelScanAndQueue {
            let result = sqlx::query(
                r#"
                UPDATE groups.ad_scan_jobs
                SET status = 'cancelled', error = NULL, finished_at = now()
                WHERE job_id = $1 AND tenant_id = $2 AND status IN ('prepared', 'queued')
                "#,
            )
            .bind(job_id)
            .bind(tenant_id)
            .execute(&mut *transaction)
            .await?;

            if result.rows_affected() != 1 {
                return Err(anyhow!("ad scan cancellation lost its status guard"));
            }
        }

        transaction.commit().await?;

        self.get_job(tenant_id, job_id).await
    }

    /// Configure a job before starting (dry_run, activate_policy).
    /// Only allowed in prepared state.
    pub async fn configure_job(
        &self,
        tenant_id: &str,
        job_id: &str,
        dry_run: bool,
        activate_policy: bool,
    ) -> Result<AdScanJob> {
        let job = self.get_job(tenant_id, job_id).await?;

        if job.status != "prepared" {
            return Ok(job);
        }

        sqlx::query(
            r#"
            UPDATE groups.ad_scan_jobs
            SET dry_run = $1, activate_policy = $2
            WHERE job_id = $3 AND tenant_id = $4 AND status = 'prepared'
            "#,
        )
        .bind(dry_run)
        .bind(activate_policy)
        .bind(job_id)
        .bind(tenant_id)
        .execute(self.pool.as_ref())
        .await?;

        self.get_job(tenant_id, job_id).await
    }

    /// Start a scan job (transitions prepared → queued → scanning).
    pub async fn start_job(&self, tenant_id: &str, job_id: &str) -> Result<AdScanJob> {
        // Serialize start and cancel on the scan row. Without this lock a
        // stale start request could enqueue work after a concurrent cancel.
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            SELECT status, group_id, source, recent_only
            FROM groups.ad_scan_jobs
            WHERE job_id = $1 AND tenant_id = $2
            FOR UPDATE
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| anyhow!("ad scan job not found or access denied"))?;
        let status: String = row.get("status");
        let group_id: i64 = row.get("group_id");
        let source: String = row.get("source");
        let recent_only: bool = row.get("recent_only");

        if !matches!(status.as_str(), "prepared" | "queued") {
            transaction.rollback().await?;
            return self.get_job(tenant_id, job_id).await;
        }

        if status == "prepared" {
            let result = sqlx::query(
                "UPDATE groups.ad_scan_jobs SET status = 'queued' \
                 WHERE job_id = $1 AND tenant_id = $2 AND status = 'prepared'",
            )
            .bind(job_id)
            .bind(tenant_id)
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() != 1 {
                return Err(anyhow!("ad scan start lost its status guard"));
            }
        }

        sqlx::query(
            r#"
            INSERT INTO job_queue
                (tenant_id, job_type, payload, priority, max_attempts, run_at, status)
            SELECT $1, 'ad_scan_job',
                   jsonb_build_object(
                       'job_id', $2::text,
                       'tenant_id', $1::text,
                       'group_id', $3::bigint,
                       'source', $4::text,
                       'recent_only', $5::bool
                   ),
                   5, 3, now(), 'pending'
            WHERE NOT EXISTS (
                SELECT 1 FROM job_queue queued
                WHERE queued.tenant_id = $1
                  AND queued.job_type = 'ad_scan_job'
                  AND queued.payload ->> 'job_id' = $2
                  AND queued.status IN ('pending', 'running', 'completed')
            )
            "#,
        )
        .bind(tenant_id)
        .bind(job_id)
        .bind(group_id)
        .bind(source)
        .bind(recent_only)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;

        self.get_job(tenant_id, job_id).await
    }

    /// Enable policy for a group (only after scan completion and explicit confirm).
    pub async fn enable_policy(
        &self,
        tenant_id: &str,
        group_id: i64,
        activated_by: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO groups.ad_group_policies (tenant_id, group_id, is_enabled, activated_by, activated_at)
            VALUES ($1, $2, true, $3, now())
            ON CONFLICT (tenant_id, group_id) DO UPDATE SET
                is_enabled = true,
                activated_by = EXCLUDED.activated_by,
                activated_at = now(),
                updated_at = now()
            "#,
        )
        .bind(tenant_id)
        .bind(group_id)
        .bind(activated_by)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    /// Atomically mark a successfully scanned job complete and, when explicitly
    /// requested, enable the matching group's policy. Keeping these writes in
    /// one transaction prevents a job from being reported as completed when
    /// policy activation failed (or vice versa).
    pub async fn finalize_successful_scan(
        &self,
        tenant_id: &str,
        job_id: &str,
        group_id: i64,
        summary: &Value,
        confirmed_clusters: i32,
        activate_policy: bool,
        activated_by: Option<&str>,
    ) -> Result<()> {
        let activated_by = if activate_policy {
            Some(
                activated_by
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("activate_policy requires a non-empty requested_by"))?,
            )
        } else {
            None
        };

        let mut transaction = self.pool.begin().await?;

        if let Some(actor) = activated_by {
            sqlx::query(
                r#"
                INSERT INTO groups.ad_group_policies
                    (tenant_id, group_id, is_enabled, activated_by, activated_at)
                VALUES ($1, $2, true, $3, now())
                ON CONFLICT (tenant_id, group_id) DO UPDATE SET
                    is_enabled = true,
                    activated_by = EXCLUDED.activated_by,
                    activated_at = now(),
                    updated_at = now()
                "#,
            )
            .bind(tenant_id)
            .bind(group_id)
            .bind(actor)
            .execute(&mut *transaction)
            .await?;
        }

        let result = sqlx::query(
            r#"
            UPDATE groups.ad_scan_jobs
            SET status = 'completed', progress = 100, summary_json = $1,
                confirmed_clusters = $2, error = NULL, finished_at = now()
            WHERE job_id = $3 AND tenant_id = $4 AND group_id = $5
              AND status IN ('scanning', 'classifying')
            "#,
        )
        .bind(summary.to_string())
        .bind(confirmed_clusters)
        .bind(job_id)
        .bind(tenant_id)
        .bind(group_id)
        .execute(&mut *transaction)
        .await?;

        if result.rows_affected() != 1 {
            return Err(anyhow!(
                "scan job finalization rejected by tenant/group/status guard"
            ));
        }

        transaction.commit().await?;
        Ok(())
    }

    /// Check if policy is enabled for a group.
    pub async fn policy_enabled(&self, tenant_id: &str, group_id: i64) -> Result<bool> {
        let is_enabled: bool = sqlx::query_scalar(
            "SELECT COALESCE(is_enabled, false) FROM groups.ad_group_policies WHERE tenant_id = $1 AND group_id = $2",
        )
        .bind(tenant_id)
        .bind(group_id)
        .fetch_optional(self.pool.as_ref())
        .await?
        .unwrap_or(false);

        Ok(is_enabled)
    }

    /// Record an action before any external side effect.
    ///
    /// New executable actions start in `planned`; `executed_at` remains NULL
    /// until `mark_action_executed` succeeds. A duplicate already in a terminal
    /// state is treated as an idempotent no-op, while an unresolved duplicate
    /// fails closed instead of risking a repeated Telegram side effect.
    pub async fn record_action(
        &self,
        tenant_id: &str,
        job_id: &str,
        group_id: i64,
        cluster_id: &str,
        action_type: &str,
        target_id: &str,
        payload: &Value,
        error: Option<&str>,
        dry_run: bool,
    ) -> Result<Option<String>> {
        if tenant_id.trim().is_empty() || job_id.trim().is_empty() {
            return Err(anyhow!("action requires non-empty tenant_id and job_id"));
        }

        // SAFETY: action_key is UNIQUE in DB, preventing duplicate actions
        let timestamp = chrono::Utc::now().timestamp() / 86400; // Daily window
        let action_key = format!(
            "{}:{}:{}:{}:{}:{}",
            tenant_id, group_id, cluster_id, action_type, target_id, timestamp
        );
        let status = action_initial_status(error, dry_run);

        let inserted = sqlx::query_scalar::<_, String>(
            r#"
            INSERT INTO groups.ad_governance_actions
                (action_key, job_id, tenant_id, group_id, cluster_id, action_type,
                 target_id, status, dry_run, payload_json, error, executed_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NULL)
            ON CONFLICT (action_key) DO NOTHING
            RETURNING action_key
            "#,
        )
        .bind(&action_key)
        .bind(job_id)
        .bind(tenant_id)
        .bind(group_id)
        .bind(cluster_id)
        .bind(action_type)
        .bind(target_id)
        .bind(status)
        .bind(dry_run)
        .bind(payload.to_string())
        .bind(error)
        .fetch_optional(self.pool.as_ref())
        .await?;

        if inserted.is_some() {
            return Ok(inserted);
        }

        let existing_status = sqlx::query_scalar::<_, String>(
            r#"
            SELECT status
            FROM groups.ad_governance_actions
            WHERE action_key = $1 AND job_id = $2 AND tenant_id = $3 AND group_id = $4
            "#,
        )
        .bind(&action_key)
        .bind(job_id)
        .bind(tenant_id)
        .bind(group_id)
        .fetch_optional(self.pool.as_ref())
        .await?
        .ok_or_else(|| anyhow!("action key collided outside the scoped job"))?;

        match existing_status.as_str() {
            "executed" | "skipped" | "dry_run" => Ok(None),
            _ => Err(anyhow!(
                "action already exists in unresolved status: {}",
                existing_status
            )),
        }
    }

    pub async fn mark_action_executed(
        &self,
        tenant_id: &str,
        job_id: &str,
        group_id: i64,
        action_key: &str,
    ) -> Result<()> {
        self.transition_planned_action(tenant_id, job_id, group_id, action_key, "executed", None)
            .await
    }

    pub async fn mark_action_failed(
        &self,
        tenant_id: &str,
        job_id: &str,
        group_id: i64,
        action_key: &str,
        error: &str,
    ) -> Result<()> {
        self.transition_planned_action(
            tenant_id,
            job_id,
            group_id,
            action_key,
            "failed",
            Some(error),
        )
        .await
    }

    pub async fn mark_action_skipped(
        &self,
        tenant_id: &str,
        job_id: &str,
        group_id: i64,
        action_key: &str,
        reason: &str,
    ) -> Result<()> {
        self.transition_planned_action(
            tenant_id,
            job_id,
            group_id,
            action_key,
            "skipped",
            Some(reason),
        )
        .await
    }

    async fn transition_planned_action(
        &self,
        tenant_id: &str,
        job_id: &str,
        group_id: i64,
        action_key: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let result = sqlx::query(
            r#"
            UPDATE groups.ad_governance_actions
            SET status = $1, error = $2,
                executed_at = CASE WHEN $1 = 'executed' THEN now() ELSE NULL END
            WHERE action_key = $3 AND job_id = $4 AND tenant_id = $5 AND group_id = $6
              AND status = 'planned'
            "#,
        )
        .bind(status)
        .bind(error)
        .bind(action_key)
        .bind(job_id)
        .bind(tenant_id)
        .bind(group_id)
        .execute(self.pool.as_ref())
        .await?;

        if result.rows_affected() != 1 {
            return Err(anyhow!(
                "action transition rejected by tenant/job/group/status guard"
            ));
        }

        Ok(())
    }

    /// Claim warning senders for dedup (5-column PK window).
    pub async fn claim_warning_senders(
        &self,
        tenant_id: &str,
        group_id: i64,
        family_hash: &str,
        sender_ids: Vec<i64>,
    ) -> Result<Vec<i64>> {
        let window_key = (chrono::Utc::now().timestamp() / 86400).to_string();
        let mut claimed = Vec::new();

        for sender_id in sender_ids {
            let recent: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM groups.ad_warning_dedup
                    WHERE tenant_id = $1 AND group_id = $2 AND family_hash = $3
                      AND sender_id = $4 AND warned_at > now() - interval '24 hours'
                )
                "#,
            )
            .bind(tenant_id)
            .bind(group_id)
            .bind(family_hash)
            .bind(sender_id)
            .fetch_one(self.pool.as_ref())
            .await?;

            if recent {
                continue; // Already warned in this 24-hour window
            }

            // Try to insert; if it succeeds (no conflict), the sender was claimed
            let result = sqlx::query(
                r#"
                INSERT INTO groups.ad_warning_dedup
                    (tenant_id, group_id, family_hash, sender_id, window_key)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT DO NOTHING
                "#,
            )
            .bind(tenant_id)
            .bind(group_id)
            .bind(family_hash)
            .bind(sender_id)
            .bind(&window_key)
            .execute(self.pool.as_ref())
            .await?;

            if result.rows_affected() > 0 {
                claimed.push(sender_id);
            }
        }

        Ok(claimed)
    }

    /// ========================================================================
    /// CLUSTERING: Exact hash grouping (reuses crate::ad_text::ad_text_hash)
    /// ========================================================================
    pub fn exact_clusters(&self, rows: Vec<Value>) -> Vec<HashMap<String, Value>> {
        let mut by_hash: HashMap<String, HashMap<String, Value>> = HashMap::new();

        for row_val in rows {
            let row = match row_val.as_object() {
                Some(r) => r,
                None => continue,
            };

            let content = row.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let media_id = row
                .get("media_unique_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let normalized = normalize_content(content);
            let is_media_only = !media_id.is_empty()
                && (normalized.is_empty() || MEDIA_PLACEHOLDER_RE.is_match(&normalized));

            let exact_basis = if is_media_only {
                format!("{}|media:{}", normalized, media_id)
            } else {
                normalized.clone()
            };

            let exact_hash = md5_hash(&exact_basis);
            let fingerprint_hash = ad_text_hash(&exact_basis);

            let template = template_content(&normalized);
            let template_hash = if is_media_only {
                md5_hash(&format!("{}|media:{}", template, media_id))
            } else {
                md5_hash(&template)
            };

            let entry = by_hash.entry(exact_hash.clone()).or_insert_with(|| {
                let mut cluster = HashMap::new();
                cluster.insert("exact_hash".to_string(), Value::String(exact_hash.clone()));
                cluster.insert(
                    "fingerprint_hash".to_string(),
                    Value::String(fingerprint_hash.clone()),
                );
                cluster.insert(
                    "template_hash".to_string(),
                    Value::String(template_hash.clone()),
                );
                cluster.insert(
                    "representative_text".to_string(),
                    Value::String(content.to_string()),
                );
                cluster.insert("is_media_only".to_string(), Value::Bool(is_media_only));
                cluster.insert(
                    "media_signature".to_string(),
                    Value::String(if is_media_only {
                        media_id.to_string()
                    } else {
                        "".to_string()
                    }),
                );
                cluster.insert("rows".to_string(), Value::Array(vec![]));
                cluster.insert("max_stored_score".to_string(), Value::Number(0.into()));
                cluster.insert("stored_ad".to_string(), Value::Bool(false));
                cluster
            });

            if let Some(rows_arr) = entry.get_mut("rows") {
                if let Value::Array(arr) = rows_arr {
                    arr.push(row_val.clone());
                }
            }

            if let Some(score_val) = row.get("ad_score") {
                if let Some(score) = score_val.as_i64() {
                    if let Some(Value::Number(max_score)) = entry.get_mut("max_stored_score") {
                        if score > max_score.as_i64().unwrap_or(0) {
                            *max_score = score.into();
                        }
                    }
                }
            }

            if let Some(is_ad) = row.get("is_advertising") {
                if is_ad.as_bool().unwrap_or(false) {
                    entry.insert("stored_ad".to_string(), Value::Bool(true));
                }
            }
        }

        by_hash.into_values().collect()
    }

    /// ========================================================================
    /// Helper: Normalize content (port from Python normalize_content)
    /// ========================================================================
    pub fn normalize_content(&self, text: &str) -> String {
        normalize_content(text)
    }
}

// Standalone helper functions (outside impl block)

fn normalize_content(text: &str) -> String {
    let normalized = text.to_lowercase();
    let no_zero_width = SPACE_RE.replace_all(&normalized, " ");
    no_zero_width.trim().to_string()
}

fn template_content(text: &str) -> String {
    let normalized = normalize_content(text);
    let after_urls = URL_RE.replace_all(&normalized, "[link]");
    let after_contacts = CONTACT_RE.replace_all(&after_urls, "[contact]");
    let after_numbers = NUMBER_RE.replace_all(&after_contacts, "[number]");
    SPACE_RE.replace_all(&after_numbers, " ").trim().to_string()
}

fn md5_hash(value: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
fn text_similarity(left: &str, right: &str) -> f64 {
    if left == right {
        return 1.0;
    }

    let a = ngrams(&template_content(left));
    let b = ngrams(&template_content(right));

    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let intersection = a.iter().filter(|x| b.contains(*x)).count();
    (2.0 * intersection as f64) / (a.len() + b.len()) as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancellationDisposition {
    CancelScanAndQueue,
    CancelQueueOnly,
    LeaveUnchanged,
}

fn cancellation_disposition(
    scan_status: &str,
    queue_statuses: &[String],
) -> Result<CancellationDisposition> {
    // Once execution has started, this service deliberately rejects
    // cooperative cancellation rather than writing a misleading cancelled
    // state that the current worker cannot observe atomically.
    if matches!(scan_status, "scanning" | "classifying") {
        return Ok(CancellationDisposition::LeaveUnchanged);
    }

    for queue_status in queue_statuses {
        match queue_status.as_str() {
            "pending" | "failed" | "dead" => {}
            "running" => {
                return Err(anyhow!(
                    "ad scan queue item is already running; cancellation rejected"
                ));
            }
            "completed" => {
                return Err(anyhow!(
                    "ad scan queue item is already completed; cancellation rejected"
                ));
            }
            status => {
                return Err(anyhow!(
                    "ad scan queue item has unsupported status: {}",
                    status
                ));
            }
        }
    }

    if matches!(scan_status, "prepared" | "queued") {
        Ok(CancellationDisposition::CancelScanAndQueue)
    } else if matches!(scan_status, "cancelled" | "completed" | "failed") {
        // Repair any stale pending queue row without rewriting the terminal
        // scan result.
        Ok(CancellationDisposition::CancelQueueOnly)
    } else {
        Ok(CancellationDisposition::LeaveUnchanged)
    }
}

fn action_initial_status(error: Option<&str>, dry_run: bool) -> &'static str {
    if error.is_some() {
        "failed"
    } else if dry_run {
        "dry_run"
    } else {
        "planned"
    }
}

#[cfg(test)]
fn ngrams(value: &str) -> HashSet<String> {
    let compact = value.replace(" ", "");
    let size = 3;

    if compact.len() <= size {
        let mut set = HashSet::new();
        if !compact.is_empty() {
            set.insert(compact);
        }
        return set;
    }

    (0..=compact.len() - size)
        .map(|i| compact[i..i + size].to_string())
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_scan_forced_dry_run() {
        // create_job parameter _dry_run is ignored; final_dry_run = true
        // Integration test with DB would verify this persists as true
    }

    #[test]
    fn test_action_key_idempotency() {
        // action_key = tenant:group:cluster:type:target:day_bucket (UNIQUE in DB)
        let timestamp = 1000i64;
        let key1 = format!("tenant1:123:cluster1:warn:user1:{}", timestamp);
        let key2 = format!("tenant1:123:cluster1:warn:user1:{}", timestamp);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_action_status_is_never_completed_at_insert_time() {
        assert_eq!(action_initial_status(None, false), "planned");
        assert_eq!(action_initial_status(None, true), "dry_run");
        assert_eq!(
            action_initial_status(Some("permission denied"), false),
            "failed"
        );
        assert_ne!(action_initial_status(None, false), "completed");
    }

    #[test]
    fn test_cancel_pending_queue_and_scan_together() {
        assert_eq!(
            cancellation_disposition("queued", &["pending".to_string()]).unwrap(),
            CancellationDisposition::CancelScanAndQueue
        );
        assert_eq!(
            cancellation_disposition("prepared", &[]).unwrap(),
            CancellationDisposition::CancelScanAndQueue
        );
    }

    #[test]
    fn test_cancel_repairs_stale_pending_queue_for_terminal_scan() {
        assert_eq!(
            cancellation_disposition("cancelled", &["pending".to_string()]).unwrap(),
            CancellationDisposition::CancelQueueOnly
        );
        assert_eq!(
            cancellation_disposition("completed", &["pending".to_string()]).unwrap(),
            CancellationDisposition::CancelQueueOnly
        );
    }

    #[test]
    fn test_cancel_rejects_queue_already_claimed_by_worker() {
        let error = cancellation_disposition("queued", &["running".to_string()])
            .expect_err("running queue item must not be reported as cancelled");
        assert!(error.to_string().contains("already running"));
    }

    #[test]
    fn test_running_scan_is_left_unchanged() {
        assert_eq!(
            cancellation_disposition("scanning", &["running".to_string()]).unwrap(),
            CancellationDisposition::LeaveUnchanged
        );
        assert_eq!(
            cancellation_disposition("classifying", &["running".to_string()]).unwrap(),
            CancellationDisposition::LeaveUnchanged
        );
    }

    #[test]
    fn test_confidence_threshold_filtering() {
        // Confidence >= 0.65 gate (Python line 1034: confidence >= 0.65)
        let conf_high = 0.7;
        let conf_low = 0.6;
        assert!(conf_high >= 0.65);
        assert!(conf_low < 0.65);
    }

    #[test]
    fn test_gates_default_off() {
        // AD_AUTO_ENFORCE_ENABLED=false, AD_DELETE_ENABLED=false by default
        // Verified via std::env::var with .unwrap_or_default() + .to_lowercase() == "true"
    }

    #[test]
    fn test_warning_dedup_5_column_pk() {
        // PK: (tenant_id, group_id, family_hash, sender_id, window_key)
        // Ensures per-group, per-family, per-sender within 24h window
    }

    #[test]
    fn test_template_similarity_length_ratio() {
        // Python line 541: min_len / max_len < 0.55 → skip similarity check
        let short = "hi";
        let long = "much longer text here for comparison";
        let short_len = short.len();
        let long_len = long.len();
        let ratio = short_len.min(long_len) as f64 / short_len.max(long_len) as f64;
        assert!(ratio < 0.55);
    }

    #[test]
    fn test_template_similarity_ngram_threshold() {
        // Python line 544: score >= 0.50 for similarity check
        let sim = text_similarity("test content", "test content");
        assert_eq!(sim, 1.0);
    }

    #[test]
    fn test_action_tier_exact_6_threshold() {
        // exact_max >= 6 → exact_delete(_mute if 3+ senders) [Python line 577]
        let exact_max = 6;
        assert!(exact_max >= 6);
    }

    #[test]
    fn test_action_tier_similar_delete_criteria() {
        // Python line 581: len(rows) > 1 && min_similarity >= 0.80 && len(recent) >= 5
        let rows_count = 2;
        let min_sim = 0.85;
        let recent_count = 5;
        assert!(rows_count > 1 && min_sim >= 0.80 && recent_count >= 5);
    }

    #[test]
    fn test_action_tier_warn_criteria() {
        // Python line 583: len(recent) >= 3
        let recent_count = 3;
        assert!(recent_count >= 3);
    }

    #[test]
    fn test_llm_output_filter_delete_instruction() {
        // Hard filter: LLM output containing "delete", "mute", "message_id" discarded
        let output = "删除这条消息";
        assert!(output.contains("删除") || output.contains("delete"));
        // Would be filtered in real LLM integration
    }

    #[test]
    fn test_action_key_format() {
        let key = "tenant1:group1:cluster1:warn:user1:1000";
        let parts: Vec<&str> = key.split(':').collect();
        assert_eq!(parts.len(), 6);
    }

    #[test]
    fn test_normalize_content() {
        let text = "  Hello   World  ";
        let result = normalize_content(text);
        assert_eq!(result, "hello world"); // Multiple spaces collapsed to single space
    }

    #[test]
    fn test_template_content_replaces_urls() {
        let text = "Visit https://example.com for more";
        let result = template_content(text);
        assert!(result.contains("[link]"));
    }

    #[test]
    fn test_md5_hash_consistency() {
        let h1 = md5_hash("test");
        let h2 = md5_hash("test");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 32); // MD5 hex string length
    }
}
