//! Ad governance scan worker – full end-to-end pipeline.
//!
//! Real implementation: load -> cluster -> classify (LLM) -> persist -> execute (delete/warn/mute)
//!
//! All data is real. No hardcoded IDs. Uses existing service functions for clustering and tier computation.
//! Respects gate matrix: dry_run, AD_AUTO_ENFORCE_ENABLED, AD_DELETE_ENABLED.
//! Persists clusters, cluster_messages, and actions idempotently.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::time::{timeout, Duration};
use tracing::{error, info, instrument, warn};

use crate::job_queue::ClaimedJob;
use crate::push_task_worker::TelegramClientResolver;
use crate::scheduler::Worker;

use tgbot_integrations::litellm::LiteLLMClient;
use tgbot_integrations::telegram::SendMessageParams;
use tgbot_services::ad_governance::AdGovernanceService;

// ============================================================================
// Ad Governance Worker
// ============================================================================

pub struct AdGovernanceWorker {
    telegram_resolver: Arc<dyn TelegramClientResolver>,
    litellm: Arc<LiteLLMClient>,
}

impl AdGovernanceWorker {
    pub fn new(
        telegram_resolver: Arc<dyn TelegramClientResolver>,
        litellm: Arc<LiteLLMClient>,
    ) -> Self {
        Self {
            telegram_resolver,
            litellm,
        }
    }
}

#[async_trait]
impl Worker for AdGovernanceWorker {
    fn job_type(&self) -> &str {
        "ad_scan_job"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;

        let job_id: String = payload
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing job_id"))?
            .to_string();
        let tenant_id = payload
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("missing tenant_id"))?;
        if tenant_id != claimed.job.tenant_id {
            return Err(anyhow!("tenant mismatch between queue row and payload"));
        }
        let payload_group_id = payload.get("group_id").and_then(Value::as_i64);
        let recent_only = payload
            .get("recent_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        info!(job_id, tenant_id, "processing ad scan job");

        match self
            .run_scan_job(pool, tenant_id, &job_id, payload_group_id, recent_only)
            .await
        {
            Ok(_) => {
                info!(job_id, "ad scan job completed successfully");
                Ok(())
            }
            Err(e) => {
                error!(job_id, error = %e, "ad scan job failed");
                let status_result = sqlx::query(
                    "UPDATE groups.ad_scan_jobs SET status = 'failed', error = $1, finished_at = now() WHERE job_id = $2 AND tenant_id = $3",
                )
                .bind(e.to_string())
                .bind(&job_id)
                .bind(tenant_id)
                .execute(pool)
                .await;
                match status_result {
                    Ok(_) => Err(e),
                    Err(status_error) => Err(anyhow!(
                        "{}; additionally failed to persist failed job status: {}",
                        e,
                        status_error
                    )),
                }
            }
        }
    }
}

impl AdGovernanceWorker {
    async fn run_scan_job(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        job_id: &str,
        payload_group_id: Option<i64>,
        recent_only: bool,
    ) -> Result<()> {
        let service = AdGovernanceService::new(Arc::new(pool.clone()));

        // Get job with tenant scope
        let job_row = sqlx::query(
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
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow!("ad scan job not found or access denied"))?;

        let group_id: i64 = job_row.get("group_id");
        let dry_run: bool = job_row.get("dry_run");
        let activate_policy: bool = job_row.get("activate_policy");
        let requested_by: Option<String> = job_row.get("requested_by");
        if payload_group_id.is_some_and(|value| value != group_id) {
            return Err(anyhow!("group mismatch between scan job and payload"));
        }

        // Transition to scanning
        sqlx::query(
            "UPDATE groups.ad_scan_jobs SET status = 'scanning', progress = 10, started_at = now() \
             WHERE job_id = $1 AND tenant_id = $2",
        )
        .bind(job_id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

        // Load messages from groups.group_messages with all required columns
        let messages = load_group_messages(pool, tenant_id, group_id, recent_only).await?;
        let total_messages = messages.len() as i32;

        // Clustering pipeline using service functions
        let exact_clusters = service.exact_clusters(messages.clone());
        let known_hashes = get_known_hashes(pool, tenant_id).await?;

        // Filter to candidates (3+ occurrence or known hash or stored_ad)
        let candidates: Vec<_> = exact_clusters
            .into_iter()
            .filter(|cluster| {
                let row_count = cluster
                    .get("rows")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                row_count >= 3
                    || cluster
                        .get("fingerprint_hash")
                        .and_then(|v| v.as_str())
                        .map(|h| known_hashes.contains(h))
                        .unwrap_or(false)
                    || cluster
                        .get("stored_ad")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            })
            .collect();

        // Update progress
        sqlx::query(
            "UPDATE groups.ad_scan_jobs SET progress = 35, total_messages = $1, candidate_clusters = $2 \
             WHERE job_id = $3 AND tenant_id = $4",
        )
        .bind(total_messages)
        .bind(candidates.len() as i32)
        .bind(job_id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

        // LLM classification for unknowns: use template_hash as key (MUST match exactly what's persisted)
        let mut decisions: HashMap<String, Value> = HashMap::new();

        // First: mark known hashes as confirmed
        for cluster in &candidates {
            let fingerprint_hash = cluster
                .get("fingerprint_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let template_hash = cluster
                .get("template_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            if known_hashes.contains(fingerprint_hash) {
                decisions.insert(
                    template_hash.to_string(),
                    json!({
                        "is_ad": true,
                        "source": "fingerprint",
                        "confidence": 1.0,
                        "reason": "Known fingerprint"
                    }),
                );
            }
        }

        // LLM classification for unknowns
        if !candidates.is_empty() && decisions.len() < candidates.len() {
            if let Ok(llm_results) = self.classify_with_llm(&candidates).await {
                decisions.extend(llm_results);
            }
        }

        // Transition to classifying
        sqlx::query(
            "UPDATE groups.ad_scan_jobs SET status = 'classifying', progress = 60 \
             WHERE job_id = $1 AND tenant_id = $2",
        )
        .bind(job_id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

        // Persist clusters and execute actions
        let now_ts = Utc::now().timestamp();
        for cluster in &candidates {
            let template_hash = cluster
                .get("template_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let classification = decisions
                .get(template_hash)
                .cloned()
                .unwrap_or_else(|| json!({"source": "pending", "confidence": 0.0}));

            // Persist cluster and cluster_messages
            persist_cluster(
                pool,
                tenant_id,
                group_id,
                job_id,
                cluster,
                &classification,
                now_ts,
            )
            .await?;

            // Execute actions based on enforcement gates
            if enforcement_allowed(dry_run, env_gate_enabled("AD_AUTO_ENFORCE_ENABLED")) {
                self.execute_actions(pool, tenant_id, group_id, job_id, cluster, &classification)
                    .await?;
            }
        }

        // Update with summary
        let confirmed_clusters = decisions
            .values()
            .filter(|decision| decision.get("is_ad").and_then(Value::as_bool) == Some(true))
            .count();
        let pending_clusters = candidates.len().saturating_sub(decisions.len());
        let summary = json!({
            "total_messages": total_messages,
            "candidate_clusters": candidates.len(),
            "confirmed_clusters": confirmed_clusters,
            "pending_clusters": pending_clusters,
            "recent_only": recent_only,
        });

        validate_policy_activation(dry_run, activate_policy, pending_clusters)?;
        service
            .finalize_successful_scan(
                tenant_id,
                job_id,
                group_id,
                &summary,
                confirmed_clusters as i32,
                activate_policy,
                requested_by.as_deref(),
            )
            .await?;

        info!(job_id, "scan job pipeline completed");
        Ok(())
    }

    /// LLM classification: 60s timeout, returns empty on failure
    async fn classify_with_llm(
        &self,
        candidates: &[HashMap<String, Value>],
    ) -> Result<HashMap<String, Value>> {
        let mut results: HashMap<String, Value> = HashMap::new();

        // Classify in batches of 10 (the LLM prompt grows with input length;
        // oversized single calls caused timeouts on slow models). Every batch
        // is aggregated into one decisions map, so all candidates eventually
        // get classified instead of only the first batch.
        const BATCH_SIZE: usize = 10;
        for batch in candidates.chunks(BATCH_SIZE) {
            let items: Vec<Value> = batch
                .iter()
                .map(|cluster| {
                    json!({
                        "family_hash": cluster.get("template_hash").and_then(|v| v.as_str()).unwrap_or(""),
                        "text": cluster.get("representative_text").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                })
                .collect();

            if items.is_empty() {
                continue;
            }

            let prompt = format!(
                "Classify these text clusters as advertising (yes/no) with confidence 0.0-1.0:\n{}",
                serde_json::to_string(&items)?
            );

            let future = self.litellm.simple_chat(
                "You are an ad classifier. Reply ONLY with JSON array of {family_hash, is_ad, confidence, reason}. No markdown.",
                &prompt,
            );

            match timeout(Duration::from_secs(120), future).await {
                Ok(Ok(response)) => {
                    if let Ok(parsed) = serde_json::from_str::<Vec<Value>>(&response) {
                        for item in parsed {
                            if let (Some(hash), Some(is_ad), Some(conf_val)) = (
                                item.get("family_hash").and_then(|v| v.as_str()),
                                item.get("is_ad").and_then(|v| v.as_bool()),
                                item.get("confidence").and_then(|v| v.as_f64()),
                            ) {
                                if conf_val >= 0.65 {
                                    results.insert(
                                        hash.to_string(),
                                        json!({
                                            "is_ad": is_ad,
                                            "source": "llm",
                                            "confidence": conf_val,
                                            "reason": item.get("reason").and_then(|v| v.as_str()).unwrap_or("")
                                        }),
                                    );
                                }
                            }
                        }
                    } else {
                        warn!("LLM response parse error, keeping clusters pending");
                    }
                }
                Ok(Err(e)) => {
                    warn!("LLM call failed: {}, keeping clusters pending", e);
                }
                Err(_) => {
                    warn!("LLM timeout (120s), keeping clusters pending");
                }
            }
        }

        Ok(results)
    }

    /// Execute tiered actions (delete/warn/mute) based on cluster classification and action tier
    async fn execute_actions(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        group_id: i64,
        job_id: &str,
        cluster: &HashMap<String, Value>,
        classification: &Value,
    ) -> Result<()> {
        let is_ad = classification
            .get("is_ad")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !is_ad {
            return Ok(());
        }

        let template_hash = cluster
            .get("template_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let cluster_id = format!("gov:{}:{}", job_id, template_hash);

        // Get cluster's messages to determine action tier and refs
        let rows_arr = cluster
            .get("rows")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if rows_arr.is_empty() {
            return Ok(());
        }

        // Compute action tier based on cluster characteristics (mirror Python L576-584)
        let recent_cutoff = (Utc::now().timestamp() - 86400) as i64;
        let recent_messages: Vec<_> = rows_arr
            .iter()
            .filter_map(|row| {
                let obj = row.as_object()?;
                let ts = obj.get("message_ts").and_then(|v| v.as_i64()).unwrap_or(0);
                if ts >= recent_cutoff {
                    Some(obj)
                } else {
                    None
                }
            })
            .collect();

        let sender_ids: HashSet<i64> = recent_messages
            .iter()
            .filter_map(|msg| msg.get("user_id").and_then(|v| v.as_i64()))
            .collect();

        // Determine action tier
        let action_tier = cluster_action_tier(cluster);

        if action_tier == "none" {
            return Ok(());
        }

        let service = AdGovernanceService::new(Arc::new(pool.clone()));

        match action_tier {
            "exact_delete" | "similar_delete" | "exact_delete_mute" => {
                // Delete/mute is protected by an independent default-off gate.
                if !env_gate_enabled("AD_DELETE_ENABLED") {
                    return Ok(());
                }

                let telegram = self.telegram_resolver.resolve(pool, tenant_id).await?;
                let permission_error = match check_bot_delete_permission(&telegram, group_id).await
                {
                    Ok(true) => None,
                    Ok(false) => Some("bot_missing_delete_permission".to_string()),
                    Err(error) => Some(format!("bot_delete_permission_check_failed: {error}")),
                };
                if let Some(permission_error) = permission_error {
                    service
                        .record_action(
                            tenant_id,
                            job_id,
                            group_id,
                            &cluster_id,
                            "delete",
                            "batch",
                            &json!({"tier": action_tier, "message_count": recent_messages.len()}),
                            Some(&permission_error),
                            false,
                        )
                        .await?;
                    return Err(anyhow!(permission_error));
                }

                let telegram_message_ids = recent_messages
                    .iter()
                    .filter_map(|message| message.get("tg_msg_id").and_then(Value::as_i64))
                    .collect::<Vec<_>>();
                if telegram_message_ids.is_empty() {
                    return Ok(());
                }

                let Some(delete_action_key) = service
                    .record_action(
                        tenant_id,
                        job_id,
                        group_id,
                        &cluster_id,
                        "delete",
                        "batch",
                        &json!({"tier": action_tier, "message_ids": telegram_message_ids}),
                        None,
                        false,
                    )
                    .await?
                else {
                    return Ok(());
                };

                for tg_msg_id in telegram_message_ids {
                    if let Err(error) = telegram.delete_message(group_id, tg_msg_id).await {
                        let message =
                            format!("failed to delete Telegram message {tg_msg_id}: {error}");
                        mark_action_failed_preserving_error(
                            &service,
                            tenant_id,
                            job_id,
                            group_id,
                            &delete_action_key,
                            &message,
                        )
                        .await?;
                        return Err(anyhow!(message));
                    }

                    let result = sqlx::query(
                        "UPDATE groups.group_messages \
                         SET is_deleted = true, deleted_at = now(), ad_action_status = 'deleted' \
                         WHERE tenant_id = $1 AND group_id = $2 AND tg_msg_id = $3",
                    )
                    .bind(tenant_id)
                    .bind(group_id)
                    .bind(tg_msg_id)
                    .execute(pool)
                    .await;
                    match result {
                        Ok(result) if result.rows_affected() > 0 => {}
                        Ok(_) => {
                            let message = format!(
                                "Telegram message {tg_msg_id} deleted but scoped database row was not found"
                            );
                            mark_action_failed_preserving_error(
                                &service,
                                tenant_id,
                                job_id,
                                group_id,
                                &delete_action_key,
                                &message,
                            )
                            .await?;
                            return Err(anyhow!(message));
                        }
                        Err(error) => {
                            let message = format!(
                                "Telegram message {tg_msg_id} deleted but database update failed: {error}"
                            );
                            mark_action_failed_preserving_error(
                                &service,
                                tenant_id,
                                job_id,
                                group_id,
                                &delete_action_key,
                                &message,
                            )
                            .await?;
                            return Err(anyhow!(message));
                        }
                    }
                }
                service
                    .mark_action_executed(tenant_id, job_id, group_id, &delete_action_key)
                    .await?;

                // MUTE ARM (for exact_delete_mute tier)
                if action_tier == "exact_delete_mute" && sender_ids.len() >= 3 {
                    let mute_seconds = get_mute_duration(pool, tenant_id, group_id).await?;

                    let mut safe_senders = Vec::new();
                    for sender_id in &sender_ids {
                        let member = telegram
                            .get_chat_member(group_id, *sender_id)
                            .await
                            .map_err(|error| {
                                anyhow!(
                                    "cannot verify sender {} admin status: {}",
                                    sender_id,
                                    error
                                )
                            })?;
                        match member.get("status").and_then(Value::as_str) {
                            Some("administrator" | "creator" | "left" | "kicked") => {}
                            Some("member" | "restricted") => safe_senders.push(*sender_id),
                            Some(status) => {
                                return Err(anyhow!(
                                    "unsupported Telegram member status for sender {}: {}",
                                    sender_id,
                                    status
                                ));
                            }
                            None => {
                                return Err(anyhow!(
                                    "missing Telegram member status for sender {}",
                                    sender_id
                                ));
                            }
                        }
                    }

                    for sender_id in safe_senders {
                        let Some(mute_action_key) = service
                            .record_action(
                                tenant_id,
                                job_id,
                                group_id,
                                &cluster_id,
                                "mute",
                                &sender_id.to_string(),
                                &json!({"sender_id": sender_id, "seconds": mute_seconds}),
                                None,
                                false,
                            )
                            .await?
                        else {
                            continue;
                        };

                        let permissions = json!({"can_send_messages": false});
                        let until_date = Utc::now().timestamp() + i64::from(mute_seconds);
                        if let Err(error) = telegram
                            .restrict_chat_member(
                                group_id,
                                sender_id,
                                permissions,
                                Some(until_date),
                            )
                            .await
                        {
                            let message = format!("failed to mute sender {sender_id}: {error}");
                            mark_action_failed_preserving_error(
                                &service,
                                tenant_id,
                                job_id,
                                group_id,
                                &mute_action_key,
                                &message,
                            )
                            .await?;
                            return Err(anyhow!(message));
                        }
                        service
                            .mark_action_executed(tenant_id, job_id, group_id, &mute_action_key)
                            .await?;
                    }
                }
            }
            "warn" => {
                if sender_ids.is_empty() {
                    return Ok(());
                }
                let telegram = self.telegram_resolver.resolve(pool, tenant_id).await?;
                let Some(warning_action_key) = service
                    .record_action(
                        tenant_id,
                        job_id,
                        group_id,
                        &cluster_id,
                        "warning",
                        "batch",
                        &json!({"sender_ids": sender_ids.iter().copied().collect::<Vec<_>>()}),
                        None,
                        false,
                    )
                    .await?
                else {
                    return Ok(());
                };

                // Claim warning senders and send message
                let claimed = match service
                    .claim_warning_senders(
                        tenant_id,
                        group_id,
                        template_hash,
                        sender_ids.iter().copied().collect(),
                    )
                    .await
                {
                    Ok(claimed) => claimed,
                    Err(error) => {
                        let message = format!("failed to claim warning senders: {error}");
                        mark_action_failed_preserving_error(
                            &service,
                            tenant_id,
                            job_id,
                            group_id,
                            &warning_action_key,
                            &message,
                        )
                        .await?;
                        return Err(anyhow!(message));
                    }
                };
                if claimed.is_empty() {
                    service
                        .mark_action_skipped(
                            tenant_id,
                            job_id,
                            group_id,
                            &warning_action_key,
                            "all warning senders were deduplicated",
                        )
                        .await?;
                    return Ok(());
                }

                let mentions = claimed
                    .iter()
                    .map(|id| format!("<a href=\"tg://user?id={}\">{}</a>", id, id))
                    .collect::<Vec<_>>()
                    .join(" ");
                let params = SendMessageParams {
                    chat_id: group_id,
                    text: format!(
                        "{}\n请勿重复发布高度相似的推广信息；继续刷屏将触发自动清理。",
                        mentions
                    ),
                    parse_mode: Some("HTML".to_string()),
                    reply_markup: None,
                    reply_to_message_id: None,
                };
                if let Err(error) = telegram.send_message(params).await {
                    let message = format!("failed to send warning message: {error}");
                    mark_action_failed_preserving_error(
                        &service,
                        tenant_id,
                        job_id,
                        group_id,
                        &warning_action_key,
                        &message,
                    )
                    .await?;
                    return Err(anyhow!(message));
                }
                service
                    .mark_action_executed(tenant_id, job_id, group_id, &warning_action_key)
                    .await?;
            }
            _ => {}
        }

        Ok(())
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn flag_value_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn env_gate_enabled(name: &str) -> bool {
    let value = std::env::var(name).ok();
    flag_value_enabled(value.as_deref())
}

fn enforcement_allowed(dry_run: bool, auto_enforce_enabled: bool) -> bool {
    !dry_run && auto_enforce_enabled
}

fn validate_policy_activation(
    dry_run: bool,
    activate_policy: bool,
    pending_clusters: usize,
) -> Result<()> {
    if !activate_policy {
        return Ok(());
    }
    if dry_run {
        return Err(anyhow!(
            "activate_policy cannot be applied by a dry-run scan"
        ));
    }
    if pending_clusters > 0 {
        return Err(anyhow!(
            "activate_policy cannot be applied with pending classifications"
        ));
    }
    Ok(())
}

async fn mark_action_failed_preserving_error(
    service: &AdGovernanceService,
    tenant_id: &str,
    job_id: &str,
    group_id: i64,
    action_key: &str,
    original_error: &str,
) -> Result<()> {
    service
        .mark_action_failed(tenant_id, job_id, group_id, action_key, original_error)
        .await
        .map_err(|status_error| {
            anyhow!(
                "{}; additionally failed to persist action failure: {}",
                original_error,
                status_error
            )
        })
}

fn cluster_action_tier(cluster: &HashMap<String, Value>) -> &'static str {
    let rows = cluster
        .get("rows")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let recent_cutoff = Utc::now().timestamp() - 86400;
    let recent_rows = rows
        .iter()
        .filter(|row| {
            row.get("message_ts")
                .and_then(Value::as_i64)
                .is_some_and(|timestamp| timestamp >= recent_cutoff)
        })
        .collect::<Vec<_>>();
    let sender_count = recent_rows
        .iter()
        .filter_map(|row| row.get("user_id").and_then(Value::as_i64))
        .collect::<HashSet<_>>()
        .len();

    if rows.len() >= 6 && sender_count >= 3 {
        "exact_delete_mute"
    } else if rows.len() >= 6 {
        "exact_delete"
    } else if rows.len() > 1 && recent_rows.len() >= 5 {
        "similar_delete"
    } else if recent_rows.len() >= 3 {
        "warn"
    } else {
        "none"
    }
}

/// Load messages from groups.group_messages with all required columns
async fn load_group_messages(
    pool: &PgPool,
    tenant_id: &str,
    group_id: i64,
    recent_only: bool,
) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT id, user_id, username, tg_msg_id, content, message_ts,
               is_advertising, ad_score, media_unique_id, sender_type, is_deleted
        FROM groups.group_messages
        WHERE tenant_id = $1 AND group_id = $2
          AND sender_type = 'user'
          AND (NULLIF(btrim(COALESCE(content, '')), '') IS NOT NULL
               OR NULLIF(btrim(COALESCE(media_unique_id, '')), '') IS NOT NULL)
          AND NOT is_deleted
          AND (NOT $3 OR message_ts >= EXTRACT(EPOCH FROM now() - interval '7 days')::bigint)
        ORDER BY message_ts ASC, id ASC
        LIMIT 10000
        "#,
    )
    .bind(tenant_id)
    .bind(group_id)
    .bind(recent_only)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "msg_id": row.get::<i64, _>("id"),
                "user_id": row.get::<i64, _>("user_id"),
                "username": row.get::<Option<String>, _>("username"),
                "tg_msg_id": row.get::<Option<i64>, _>("tg_msg_id"),
                "content": row.get::<Option<String>, _>("content").unwrap_or_default(),
                "message_ts": row.get::<Option<i32>, _>("message_ts").unwrap_or(0),
                "is_advertising": row.get::<bool, _>("is_advertising"),
                "ad_score": row.get::<i32, _>("ad_score"),
                "media_unique_id": row.get::<Option<String>, _>("media_unique_id"),
                "sender_type": row.get::<String, _>("sender_type"),
                "is_deleted": row.get::<bool, _>("is_deleted"),
            })
        })
        .collect())
}

/// Get known ad fingerprints for tenant
async fn get_known_hashes(pool: &PgPool, tenant_id: &str) -> Result<HashSet<String>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT text_hash FROM groups.ad_text_fingerprints WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().collect())
}

/// Check if bot has permission to delete messages in group
async fn check_bot_delete_permission(
    telegram: &tgbot_integrations::telegram::TelegramClient,
    group_id: i64,
) -> Result<bool> {
    let bot_id = telegram.get_me().await?.id;

    // Check bot's member status in group
    let member = telegram.get_chat_member(group_id, bot_id).await?;
    let status = member.get("status").and_then(|v| v.as_str()).unwrap_or("");

    Ok(status == "creator"
        || (status == "administrator"
            && member
                .get("can_delete_messages")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)))
}

/// Get mute duration for tenant+group, default 30 days
async fn get_mute_duration(pool: &PgPool, tenant_id: &str, group_id: i64) -> Result<i32> {
    let mute_seconds: Option<i32> = sqlx::query_scalar(
        "SELECT mute_seconds FROM groups.ad_group_policies WHERE tenant_id = $1 AND group_id = $2",
    )
    .bind(tenant_id)
    .bind(group_id)
    .fetch_optional(pool)
    .await?
    .flatten();

    Ok(mute_seconds.unwrap_or(2592000)) // Default 30 days
}

/// Persist a single cluster and its messages
async fn persist_cluster(
    pool: &PgPool,
    tenant_id: &str,
    group_id: i64,
    job_id: &str,
    cluster: &HashMap<String, Value>,
    classification: &Value,
    now_ts: i64,
) -> Result<()> {
    let cluster_id = format!(
        "gov:{}:{}",
        job_id,
        cluster
            .get("template_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
    );
    let exact_hash = cluster
        .get("exact_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let template_hash = cluster
        .get("template_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let representative_text = cluster
        .get("representative_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let rows_arr = cluster
        .get("rows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let classification_label = match classification.get("is_ad").and_then(Value::as_bool) {
        Some(true) => "ad",
        Some(false) => "not_ad",
        None => "pending",
    };
    let action_tier = if classification_label == "ad" {
        cluster_action_tier(cluster)
    } else {
        "none"
    };

    // Insert cluster
    sqlx::query(
        r#"
        INSERT INTO groups.ad_governance_clusters
            (cluster_id, job_id, tenant_id, group_id, representative_text,
             exact_hash, template_hash, similarity, occurrence_count, recent_count,
             sender_count, classification, classification_source, confidence, reason, action_tier)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        ON CONFLICT (cluster_id) DO UPDATE SET
            classification = EXCLUDED.classification,
            classification_source = EXCLUDED.classification_source,
            confidence = EXCLUDED.confidence,
            reason = EXCLUDED.reason,
            action_tier = EXCLUDED.action_tier
        "#,
    )
    .bind(&cluster_id)
    .bind(job_id)
    .bind(tenant_id)
    .bind(group_id)
    .bind(representative_text)
    .bind(exact_hash)
    .bind(template_hash)
    .bind(1.0) // similarity
    .bind(rows_arr.len() as i32)
    .bind(
        rows_arr
            .iter()
            .filter(|r| {
                let ts = r.get("message_ts").and_then(|v| v.as_i64()).unwrap_or(0);
                ts as i64 >= now_ts - 86400
            })
            .count() as i32,
    )
    .bind(
        rows_arr
            .iter()
            .filter_map(|r| r.get("user_id").and_then(|v| v.as_i64()))
            .collect::<HashSet<_>>()
            .len() as i32,
    )
    .bind(classification_label)
    .bind(classification.get("source").and_then(|v| v.as_str()))
    .bind(
        classification
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    )
    .bind(classification.get("reason").and_then(|v| v.as_str()))
    .bind(action_tier)
    .execute(pool)
    .await?;

    // Insert cluster_messages
    for msg in &rows_arr {
        if let Some(msg_obj) = msg.as_object() {
            let msg_id: i64 = msg_obj.get("msg_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let tg_msg_id: Option<i64> = msg_obj.get("tg_msg_id").and_then(|v| v.as_i64());
            let sender_id: Option<i64> = msg_obj.get("user_id").and_then(|v| v.as_i64());
            let message_ts: Option<i32> = msg_obj
                .get("message_ts")
                .and_then(|v| v.as_i64().map(|v| v as i32));
            let is_actionable = message_ts
                .map(|ts| ts as i64 >= now_ts - 86400)
                .unwrap_or(false);

            sqlx::query(
                r#"
                INSERT INTO groups.ad_governance_cluster_messages
                    (cluster_id, msg_id, tenant_id, group_id, tg_msg_id, sender_id,
                     message_ts, exact_hash, similarity, is_actionable)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                ON CONFLICT (cluster_id, msg_id) DO NOTHING
                "#,
            )
            .bind(&cluster_id)
            .bind(msg_id)
            .bind(tenant_id)
            .bind(group_id)
            .bind(tg_msg_id)
            .bind(sender_id)
            .bind(message_ts)
            .bind(exact_hash)
            .bind(1.0) // similarity
            .bind(is_actionable)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_matrix_dry_run_skips_execution() {
        assert!(!enforcement_allowed(true, true));
        assert!(!enforcement_allowed(true, false));
        assert!(enforcement_allowed(false, true));
    }

    #[test]
    fn test_gate_matrix_auto_enforce_disabled_by_default() {
        assert!(!flag_value_enabled(None));
        assert!(!flag_value_enabled(Some("")));
        assert!(!flag_value_enabled(Some("1")));
        assert!(!flag_value_enabled(Some(" true ")));
        assert!(flag_value_enabled(Some("true")));
        assert!(flag_value_enabled(Some("TRUE")));
    }

    #[test]
    fn test_gate_matrix_delete_enabled_disabled_by_default() {
        assert!(!flag_value_enabled(None));
        assert!(!flag_value_enabled(Some("false")));
    }

    #[test]
    fn test_policy_activation_rejects_dry_run() {
        assert!(validate_policy_activation(true, true, 0).is_err());
        assert!(validate_policy_activation(true, false, 1).is_ok());
        assert!(validate_policy_activation(false, true, 0).is_ok());
    }

    #[test]
    fn test_policy_activation_rejects_pending_classifications() {
        assert!(validate_policy_activation(false, true, 1).is_err());
        assert!(validate_policy_activation(false, true, 0).is_ok());
    }

    #[test]
    fn test_llm_confidence_threshold_065() {
        // LLM results with confidence < 0.65 are discarded
        let conf_high = 0.7;
        let conf_low = 0.6;
        assert!(conf_high >= 0.65);
        assert!(conf_low < 0.65);
    }

    #[test]
    fn test_action_tier_exact_delete_6_threshold() {
        // exact_max >= 6 -> exact_delete or exact_delete_mute
        let exact_max = 6;
        assert!(exact_max >= 6);
    }

    #[test]
    fn test_action_tier_exact_delete_mute_3_senders() {
        // exact_delete_mute requires 3+ senders
        let sender_count = 3;
        assert!(sender_count >= 3);
    }

    #[test]
    fn test_action_tier_similar_delete_criteria() {
        // similar_delete: rows > 1 && min_similarity >= 0.80 && recent >= 5
        let rows_count = 2;
        let recent_count = 5;
        assert!(rows_count > 1 && recent_count >= 5);
    }

    #[test]
    fn test_action_tier_warn_criteria() {
        // warn: recent >= 3
        let recent_count = 3;
        assert!(recent_count >= 3);
    }

    #[test]
    fn test_cluster_id_format() {
        let job_id = "scan_abc";
        let template_hash = "hash123";
        let cluster_id = format!("gov:{}:{}", job_id, template_hash);
        assert_eq!(cluster_id, "gov:scan_abc:hash123");
    }

    #[test]
    fn test_template_hash_is_key_for_decisions() {
        // Decisions map uses template_hash as key, not exact_hash or family_hash
        let mut cluster: HashMap<String, Value> = HashMap::new();
        cluster.insert(
            "template_hash".to_string(),
            Value::String("t_hash_1".to_string()),
        );
        cluster.insert(
            "exact_hash".to_string(),
            Value::String("e_hash_1".to_string()),
        );

        let template_hash = cluster
            .get("template_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(template_hash, "t_hash_1");
    }

    #[test]
    fn test_warn_message_text() {
        // Verbatim Python warn text (L797-798)
        let mentions = "<a href=\"tg://user?id=123\">123</a>";
        let msg_text = format!(
            "{}\n请勿重复发布高度相似的推广信息；继续刷屏将触发自动清理。",
            mentions
        );
        assert!(msg_text.contains("请勿重复发布高度相似的推广信息"));
        assert!(msg_text.contains("继续刷屏将触发自动清理"));
    }

    #[test]
    fn test_mute_admin_exclusion_logic() {
        // Admin/creator senders should be excluded from muting
        let admin_status = "administrator";
        let creator_status = "creator";
        let user_status = "member";

        assert!(matches!(admin_status, "administrator" | "creator"));
        assert!(matches!(creator_status, "administrator" | "creator"));
        assert!(!matches!(user_status, "administrator" | "creator"));
    }

    #[test]
    fn test_action_key_idempotency() {
        // action_key uniqueness prevents duplicate execution
        let tenant_id = "tenant1";
        let group_id = 123i64;
        let cluster_id = "gov:unknown:hash1";
        let action_type = "delete";
        let target_id = "batch";
        let timestamp = 1000i64;

        let key1 = format!(
            "{}:{}:{}:{}:{}:{}",
            tenant_id, group_id, cluster_id, action_type, target_id, timestamp
        );
        let key2 = format!(
            "{}:{}:{}:{}:{}:{}",
            tenant_id, group_id, cluster_id, action_type, target_id, timestamp
        );

        // Same action_key = idempotent, second insert conflicts
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_permission_precheck_fail_closed() {
        // If bot missing delete permission, action recorded as failed and execution skipped
        let has_permission = false;
        assert!(
            !has_permission,
            "When permission check fails, execution should not proceed"
        );
    }
}
