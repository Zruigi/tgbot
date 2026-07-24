//! Group policy worker – processes group_policy_sync jobs from the queue.
//!
//! For each group policy sync job:
//! 1. Reads the group's current config from `tracked_groups`.
//! 2. Evaluates and applies policy rules (verify settings, keyword filters, etc.).
//! 3. Updates the group config in the database.
//! 4. Optionally enqueues sub-jobs (e.g. AI content analysis of recent messages).

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::{info, instrument, warn};

use crate::job_queue::{self, ClaimedJob, JobType};
use crate::scheduler::Worker;

use tgbot_repos::group_repo;

// ---------------------------------------------------------------------------
// Group policy worker
// ---------------------------------------------------------------------------

pub struct GroupPolicyWorker {
    // Future: could hold a reference to group_service or policy engine.
}

impl GroupPolicyWorker {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Worker for GroupPolicyWorker {
    fn job_type(&self) -> &str {
        "group_policy_sync"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;

        let group_id: i64 = payload
            .get("group_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("missing group_id in payload"))?;
        let tenant_id: String = payload
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        info!(group_id, tenant_id, "processing group policy sync job");

        // Fetch the current group config
        let group = group_repo::get_group(pool, group_id, &tenant_id)
            .await?
            .ok_or_else(|| anyhow!("group {} not found for tenant {}", group_id, tenant_id))?;

        // Evaluate policy rules
        let verify_enabled: bool = group
            .get("verify_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let is_monitored: bool = group
            .get("is_monitored")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let custom_keywords: Option<String> = group
            .get("custom_keywords")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Policy: if monitoring is enabled, ensure verify is also enabled
        if is_monitored && !verify_enabled {
            info!(group_id, "policy: enabling verify for monitored group");
            group_repo::upsert_group_config(
                pool,
                group_id,
                group.get("group_title").and_then(|v| v.as_str()).unwrap_or("Unknown"),
                &tenant_id,
                Some(true),           // is_monitored
                custom_keywords.as_deref(),
                Some(true),           // verify_enabled
                None,                 // verify_type (keep current)
                None,                 // verify_timeout (keep current)
            )
            .await?;
        }

        // Policy: check for stale keyword configs and clean up
        if let Some(ref keywords) = custom_keywords {
            if keywords.trim().is_empty() {
                info!(group_id, "policy: clearing empty custom_keywords");
                group_repo::upsert_group_config(
                    pool,
                    group_id,
                    group.get("group_title").and_then(|v| v.as_str()).unwrap_or("Unknown"),
                    &tenant_id,
                    None,
                    None,             // clear keywords
                    None,
                    None,
                    None,
                )
                .await?;
            }
        }

        // Optionally: enqueue AI analysis job if there are custom keywords
        if let Some(keywords) = &custom_keywords {
            if !keywords.trim().is_empty() && is_monitored {
                // Check if there's already a pending AI analysis for this group
                let existing: Option<i64> = sqlx::query_scalar(
                    r#"
                    SELECT id FROM job_queue
                    WHERE job_type = 'ai_process'
                      AND payload->>'group_id' = $1
                      AND status IN ('pending', 'running')
                    LIMIT 1
                    "#,
                )
                .bind(group_id.to_string())
                .fetch_optional(pool)
                .await?;

                if existing.is_none() {
                    let opts = job_queue::EnqueueOpts::new(
                        JobType::AiProcess,
                        json!({
                            "group_id": group_id,
                            "tenant_id": tenant_id,
                            "analysis_type": "keyword_monitoring",
                            "keywords": keywords,
                        }),
                    )
                    .with_priority(1)
                    .with_max_attempts(2);

                    job_queue::enqueue(pool, opts).await?;
                    info!(group_id, "enqueued AI analysis job for keyword monitoring");
                }
            }
        }

        info!(group_id, "group policy sync completed");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_policy_worker_job_type() {
        let worker = GroupPolicyWorker::new();
        assert_eq!(worker.job_type(), "group_policy_sync");
    }
}
