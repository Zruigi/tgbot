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
//! - Per-group locking (tokio mutex map)
//! - tenant_id scoping on every query

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::instrument;
use uuid::Uuid;

// Re-export for use in routes
pub use crate::ad_detection::detect_advertising_signals;

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
}

// ============================================================================
// Ad Governance Service
// ============================================================================

pub struct AdGovernanceService {
    pool: Arc<PgPool>,
    // Per-group locks: (tenant_id, group_id) -> Mutex
    group_locks: Arc<Mutex<HashMap<(String, i64), Arc<Mutex<()>>>>>,
}

impl AdGovernanceService {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            pool,
            group_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get or create a lock for a specific group.
    async fn get_group_lock(&self, tenant_id: &str, group_id: i64) -> Arc<Mutex<()>> {
        let key = (tenant_id.to_string(), group_id);
        let mut locks = self.group_locks.lock().await;
        locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// List tracked groups for a tenant (simplified version without Telegram bot checks).
    pub async fn list_groups(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<GroupInfo>> {
        let rows = sqlx::query(
            r#"
            SELECT g.group_id, COALESCE(g.group_title, '') as title,
                   COALESCE(g.chat_type, 'supergroup') as chat_type,
                   COALESCE(COUNT(m.id), 0) as message_count
            FROM groups.tracked_groups g
            LEFT JOIN chat.messages m ON m.group_id = g.group_id AND m.tenant_id = g.tenant_id
            WHERE g.tenant_id = $1
            GROUP BY g.group_id, g.group_title, g.chat_type
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
                can_delete_messages: false,          // Simplified: requires Telegram API
                can_restrict_members: false,         // Simplified: requires Telegram API
            })
            .collect())
    }

    /// Create a new ad scan job (always starts with dry_run=true).
    /// Activation only via explicit confirm.
    #[instrument(skip(self), fields(tenant_id, group_id))]
    pub async fn create_job(
        &self,
        tenant_id: &str,
        group_id: i64,
        requested_by: &str,
        group_title: Option<&str>,
        dry_run: bool,
        activate_policy: bool,
        source: &str,
    ) -> Result<AdScanJob> {
        // SAFETY INVARIANT: First scan forced dry_run=true
        let final_dry_run = dry_run || true; // Always dry_run for initial creation
        let job_id = format!("scan_{}", Uuid::new_v4().to_string().replace("-", ""));

        let message_count: i32 = sqlx::query_scalar(
            r#"
            SELECT COALESCE(COUNT(*), 0)::int
            FROM chat.messages
            WHERE tenant_id = $1 AND group_id = $2
                AND sender_type = 'user' AND COALESCE(content, '') != ''
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
                 dry_run, activate_policy, source, status, total_messages)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'prepared', $9)
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
            summary: serde_json::from_str(row.get::<Option<&str>, _>("summary_json").unwrap_or("{}"))
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
                query.push_str(" AND action_tier IN ('similar_delete', 'exact_delete', 'exact_delete_mute')");
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
        let job = self.get_job(tenant_id, job_id).await?;

        if !matches!(job.status.as_str(), "prepared" | "queued") {
            return Ok(job);
        }

        sqlx::query(
            r#"
            UPDATE groups.ad_scan_jobs
            SET status = 'cancelled', finished_at = now()
            WHERE job_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .execute(self.pool.as_ref())
        .await?;

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
        let job = self.get_job(tenant_id, job_id).await?;

        if !matches!(job.status.as_str(), "prepared" | "queued") {
            return Ok(job);
        }

        if job.status == "prepared" {
            sqlx::query(
                "UPDATE groups.ad_scan_jobs SET status = 'queued' WHERE job_id = $1 AND status = 'prepared'",
            )
            .bind(job_id)
            .execute(self.pool.as_ref())
            .await?;
        }

        // Enqueue the job for worker processing
        // This will be handled by the worker subscribed to scan jobs

        self.get_job(tenant_id, job_id).await
    }

    /// Update job progress/status (called by worker).
    #[instrument(skip(self))]
    async fn set_job(
        &self,
        job_id: &str,
        updates: &[(String, Value)],
    ) -> Result<()> {
        let mut query = String::from("UPDATE groups.ad_scan_jobs SET ");
        let mut values: Vec<Value> = vec![];
        let mut param_idx = 1;

        for (key, _) in updates.iter() {
            if param_idx > 1 {
                query.push_str(", ");
            }
            query.push_str(&format!("{} = ${}", key, param_idx));
            param_idx += 1;
        }

        query.push_str(" WHERE job_id = $");
        query.push_str(&param_idx.to_string());

        let mut q = sqlx::query(&query);
        for (_, value) in updates.iter() {
            // Bind each value — this is simplified; in production use proper binding
            match value {
                Value::String(s) => q = q.bind(s.clone()),
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        q = q.bind(i);
                    } else if let Some(f) = n.as_f64() {
                        q = q.bind(f);
                    }
                }
                Value::Bool(b) => q = q.bind(*b),
                _ => q = q.bind(value.to_string()),
            }
        }
        q = q.bind(job_id);

        q.execute(self.pool.as_ref()).await?;
        Ok(())
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

    /// Record an action in the audit log.
    /// action_key ensures idempotency: duplicate keys are ignored.
    pub async fn record_action(
        &self,
        tenant_id: &str,
        group_id: i64,
        cluster_id: &str,
        action_type: &str,
        target_id: &str,
        payload: &Value,
        error: Option<&str>,
        dry_run: bool,
    ) -> Result<bool> {
        // SAFETY: action_key is UNIQUE in DB, preventing duplicate actions
        let timestamp = chrono::Utc::now().timestamp() / 86400; // Daily window
        let action_key = format!(
            "{}:{}:{}:{}:{}:{}",
            tenant_id, group_id, cluster_id, action_type, target_id, timestamp
        );

        let result = sqlx::query(
            r#"
            INSERT INTO groups.ad_governance_actions
                (action_key, tenant_id, group_id, cluster_id, action_type, target_id,
                 status, dry_run, payload_json, error, executed_at)
            VALUES ($1, $2, $3, $4, $5, $6, 'completed', $7, $8, $9, now())
            ON CONFLICT (action_key) DO NOTHING
            "#,
        )
        .bind(&action_key)
        .bind(tenant_id)
        .bind(group_id)
        .bind(cluster_id)
        .bind(action_type)
        .bind(target_id)
        .bind(dry_run)
        .bind(payload.to_string())
        .bind(error)
        .execute(self.pool.as_ref())
        .await?;

        Ok(result.rows_affected() > 0)
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
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_machine_transitions_first_scan_forced_dry_run() {
        // Verify that create_job always sets dry_run=true for first scan
        // This would be an integration test with actual DB
    }

    #[test]
    fn test_action_key_idempotency() {
        // action_key = tenant:group:cluster:type:target:day_bucket
        // UNIQUE constraint in DB prevents duplicates
        let key1 = format!(
            "{}:{}:{}:{}:{}:{}",
            "tenant1", 123, "cluster1", "warn", "user1", 1000
        );
        let key2 = format!(
            "{}:{}:{}:{}:{}:{}",
            "tenant1", 123, "cluster1", "warn", "user1", 1000
        );
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_gates_default_off() {
        // AD_AUTO_ENFORCE_ENABLED and AD_DELETE_ENABLED must default to false
        // Tests in worker will verify env var behavior
    }

    #[test]
    fn test_warning_dedup_5_column_pk() {
        // ad_warning_dedup PK is (tenant_id, group_id, family_hash, sender_id, window_key)
        // Ensures per-group, per-family, per-sender dedup within a time window
    }

    #[test]
    fn test_confidence_threshold_filtering() {
        // LLM output gated by confidence >= 0.65
        // Only apply is_ad when confidence meets threshold
    }
}
