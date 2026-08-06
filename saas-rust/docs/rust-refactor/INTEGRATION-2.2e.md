# Ad Governance Integration Points (Task 2.2e)

## Overview
Task 2.2e ports the ad-governance subsystem (Python commit 332ba4c) to Rust. This document describes integration points where Rust code must hook into existing systems.

> **Current Beta evidence (2026-07-27).** The hook described below is now
> implemented by the `groups.group_messages` insert/update path and migration
> `0025_realtime_ad_scan_outbox.sql`: the trigger creates a tenant-scoped
> `groups.ad_scan_jobs` row and `job_queue` payload when an enabled policy and
> message evidence are present. `AdGovernanceWorker` consumes that payload with
> tenant/group rechecks. Migrations `0020`–`0026` and the realtime path pass the
> disposable-database runtime gate; see
> `target/beta-reports/runtime-gate-report.json`.
>
> This file retains the original porting notes below as historical context.
> Unknown LLM decisions remain pending and fail closed, and Telegram deletion
> or warning actions still require the explicit environment gates and a real Bot
> acceptance run. The local gate is not production or MiniApp acceptance.

> **Status override for the historical deferred checklist.** Tenant-scoped
> clustering, action idempotency, deletion/warning execution gates, and the
> realtime trigger are implemented in this Beta candidate. The old unchecked
> items below are retained as design notes; they are not current evidence that
> the code path is still a placeholder. Remaining open items are unknown-LLM
> decisions, real Telegram side-effect acceptance, and the intentionally
> deferred Agent UI extension.

## 1. Telegram Webhook Integration Hook
**Location**: `crates/http/src/routes/telegram_webhook.rs` line ~945 (group_messages INSERT)

**What to do**:
When processing incoming Telegram group messages, after storing the message in `chat.messages`, inject two additional fields and schedule realtime scan:

```rust
// After INSERT INTO chat.messages:
UPDATE chat.messages
SET media_unique_id = $1,  // Extract from message.photo[].file_unique_id or document.file_unique_id
    ad_text_hash = $2      // Hash via crates/services/src/ad_text.rs::ad_text_hash()
WHERE id = $3
```

Then schedule a realtime scan if policy is enabled:

```rust
if ad_governance_service.policy_enabled(&tenant_id, group_id).await? {
    // Enqueue realtime scan job with source='realtime' and recent_only=true
    job_queue::enqueue(
        pool,
        EnqueueOpts::new(
            JobType::Custom("ad_scan_job".into()),
            json!({
                "job_id": format!("rt_{}_{}", group_id, timestamp_bucket),
                "tenant_id": tenant_id,
                "group_id": group_id,
                "source": "realtime",
                "dry_run": false,
                "activate_policy": false,
            }),
        ).with_priority(5),
    ).await?;
}
```

**Rationale**: Real-time ad detection feedback loop. Policies enabled on groups should trigger background scans as new messages arrive.

---

## 2. Agent Chat Response Extension
**DEFERRED** — Currently listed as FORBIDDEN in task description.

**Location**: `crates/http/src/routes/ai.rs`

**What to do** (deferred to next phase):
Extend `/api/ai/chat` response JSON to include:
```json
{
  "ui_actions": [
    { "id": "view_ad_scan", "label": "查看广告扫描", "payload": { "job_id": "..." } },
    { "id": "start_scan", "label": "开始扫描", "payload": { "group_id": 123 } }
  ],
  "job": { "job_id": "scan_...", "status": "prepared", ... }
}
```

This allows the Agent workflow in n8n to trigger ad scans and display job status in the UI.

**Why deferred**: Requires coordination with n8n workflow changes; marked as FORBIDDEN to avoid cross-cutting concerns.

---

## 3. LiteLLM Integration (Clustering Classifier)
**Location**: `crates/integrations/src/litellm.rs` (existing) + `crates/services/src/ad_governance.rs`

**What to do**:
The worker calls an LLM classifier to judge whether text clusters are ads. Use existing LiteLLM client to:

```rust
pub async fn classify_families_with_llm(
    litellm_client: &LiteLLMClient,
    items: Vec<FamilyCluster>,
) -> Result<HashMap<String, ClassificationResult>> {
    let prompt = format!(
        "你是Telegram群广告分类器。只判断下列聚类代表文本是否为广告、推广。\
         必须只返回JSON数组。格式: {{\"family_hash\": \"...\", \"is_ad\": bool, \"confidence\": 0.0-1.0}}\n\
         {}", 
        serde_json::to_string(&items)?
    );

    let response = litellm_client.chat_completion(
        model = "gemini-2.5-flash-lite",
        messages = vec![
            {"role": "system", "content": "仅输出合法JSON数组"},
            {"role": "user", "content": prompt[..16000]},  // Truncate at 16k
        ],
        temperature = 0.0,
        max_tokens = 8192,
        timeout_seconds = 15.0,
    ).await?;

    // Parse JSON, validate family_hash whitelist, filter by confidence >= 0.65
    // Only apply is_ad when confidence meets threshold
}
```

**Safety gates**:
- LLM receives only cluster representative texts, never raw message lists
- Output must NOT contain deletion instructions (filter for "delete", "remove" keywords)
- Apply family_hash whitelist (only hashes present in request are accepted)
- confidence >= 0.65 threshold enforced on server-side after parsing

---

## 4. Message Deletion Execution (Gated by Env Vars)
**Location**: `crates/workers/src/ad_governance_worker.rs`

**Safety gates**:
```rust
// GATE 1: AD_AUTO_ENFORCE_ENABLED must be true (defaults OFF)
let auto_enforce = std::env::var("AD_AUTO_ENFORCE_ENABLED")
    .unwrap_or_default()
    .to_lowercase() == "true";
if !auto_enforce {
    info!("deletion blocked: AD_AUTO_ENFORCE_ENABLED not set");
    return Err(anyhow!("enforcement disabled"));
}

// GATE 2: AD_DELETE_ENABLED must be true (defaults OFF)
let delete_enabled = std::env::var("AD_DELETE_ENABLED")
    .unwrap_or_default()
    .to_lowercase() == "true";
if !delete_enabled {
    info!("deletion blocked: AD_DELETE_ENABLED not set");
    return Err(anyhow!("deletion disabled"));
}

// GATE 3: Dry-run mode
if job.dry_run {
    info!("dry-run mode: no actual deletion");
    return Ok(());
}

// GATE 4: Per-group Telegram API permission check
let bot_member = bot.get_chat_member(group_id, bot_id).await?;
let can_delete = bot_member.can_delete_messages();
if !can_delete {
    return Err(anyhow!("bot lacks delete permission"));
}

// GATE 5: Re-verify cluster is confirmed (not pending)
if cluster.classification != "ad" {
    return Err(anyhow!("cluster not confirmed as ad"));
}
```

---

## 5. Exact Clustering Algorithm
**Location**: `crates/services/src/ad_governance.rs::_exact_clusters()`

**Uses existing**: `crates/services/src/ad_text.rs::ad_text_hash()` — must NOT modify.

**Algorithm** (copied from Python `services/ad_governance.py::_exact_clusters`):
1. For each message: normalize content → compute exact_hash = MD5(normalized)
2. Group by exact_hash
3. For each group:
   - Extract representative text (original from first message)
   - Compute fingerprint_hash via `ad_text_hash()` (for whitelist matching)
   - Compute template_hash = MD5(template_content) — URLs/contacts replaced with `[link]`
   - Count occurrences, senders, recent messages

```rust
fn exact_clusters(messages: Vec<Message>) -> Vec<ExactCluster> {
    let mut by_hash: HashMap<String, ExactCluster> = HashMap::new();
    
    for msg in messages {
        let normalized = normalize_ad_text(&msg.content);
        let exact_hash = ad_text_hash(&normalized); // Use existing function
        
        let entry = by_hash.entry(exact_hash).or_insert_with(|| {
            ExactCluster {
                exact_hash: exact_hash.clone(),
                fingerprint_hash: ad_text_hash(&exact_basis), // For whitelist
                template_hash: md5(template_content(&normalized)),
                representative_text: msg.content.clone(),
                messages: vec![],
            }
        });
        entry.messages.push(msg);
    }
    
    by_hash.into_values().collect()
}
```

---

## 6. Warning Deduplication Window
**Location**: `crates/services/src/ad_governance.rs::claim_warning_senders()`

**PK**: `(tenant_id, group_id, family_hash, sender_id, window_key)`

**Window**: Daily (`window_key = timestamp / 86400`)

**Ensures**: Each sender gets warned at most once per family per day per group.

---

## 7. Env Var Configuration
**Required (if not present, features disabled)**:
- `AD_AUTO_ENFORCE_ENABLED=false` (default) — enable auto enforcement
- `AD_DELETE_ENABLED=false` (default) — enable deletion actions
- Existing: `LITELLM_API_KEY`, `LITELLM_BASE_URL` — for LLM calls

**Recommended**:
- `AD_SCAN_BATCH_SIZE=100` — messages per batch for clustering
- `AD_CLASSIFICATION_BATCH_SIZE=20` — families per batch for LLM
- `AD_CONFIDENCE_THRESHOLD=0.65` — LLM confidence cutoff

---

## 8. Tenant Isolation Points
Every query in `ad_governance.rs` and `ad_governance_worker.rs` must include `tenant_id` in WHERE clause:

- ✅ list_groups: `WHERE tenant_id = $1`
- ✅ create_job: `INSERT ... VALUES (..., $tenant_id, ...)`
- ✅ get_job: `WHERE job_id = $1 AND tenant_id = $2`
- ✅ get_clusters: `WHERE job_id = $1 AND tenant_id = $2`
- ✅ record_action: `INSERT ... VALUES (..., $tenant_id, ...)`
- ✅ policy_enabled: `WHERE tenant_id = $1 AND group_id = $2`

---

## 9. Current Beta Evidence and Deferred Items

The following matrix is the current release-candidate status. A local runtime
check uses disposable PostgreSQL and a mock Telegram API; it is evidence for
code-path behavior only, not real Bot acceptance.

### Verified by the local Beta gate

- [x] Schema, tenant columns, indexes, RLS policies, helper grants, and the
      `realtime_ad_scan_outbox` trigger (`0020`–`0026`), applied twice from an
      empty database.
- [x] Webhook message evidence writes and realtime `ad_scan_job` enqueue.
- [x] Worker claim/completion, tenant re-checks, row locking, and action
      idempotency with the disposable database.
- [x] Unknown ad-scan IDs and missing capability/LLM evidence fail closed.
- [x] Legacy advertisement compatibility event persistence, backfill, listing,
      and tenant-scoped lookup.
- [x] The full local runtime request matrix and method/path trace (102/102;
      see `target/beta-reports/runtime-gate-report.json`).

### Still required before gray release

- [ ] Real test Bot webhook delivery, group permissions, warning/deletion
      side-effects, Telegram retries, and rollback of a failed action.
- [ ] Production-approved LLM catalog/classifier behavior and an explicit
      acceptance record for unknown/low-confidence decisions.
- [ ] Real tenant acceptance of the MiniApp/Agent workflow; the UI extension
      remains intentionally deferred.
- [ ] Deployment, small-tenant gray rollout, at least three days of telemetry,
      and a rehearsed rollback.

The older checklist below is retained as porting/design history and should not
be used as the current release status.

## 10. Historical Known Gaps and Deferred Items

### Implemented (MVP scope):
- [x] Schema (6 tables, indexes, tenant_id columns)
- [x] Job state machine (prepared → queued → scanning → completed)
- [x] Safety gates (dry_run forced, env vars, confidence threshold)
- [x] Action idempotency (action_key UNIQUE)
- [x] Warning dedup (5-column PK)
- [x] Per-group locking pattern (tokio Mutex map)
- [x] HTTP endpoints (all 5 routes)
- [x] Basic worker skeleton

### Deferred (full implementation needed):
- Template similarity clustering (50%+ length ratio check, text_similarity() ngram Jaccard)
- LLM classification integration (full prompt + response parsing + confidence filtering)
- Message deletion execution (batch delete, DB audit, Telegram API call)
- Action execution: warn tier (@mention + message send)
- Action execution: exact_delete_mute tier (mute + delete)
- Real-time scan scheduling on message ingestion
- Agent UI integration (chat response extension)

### Not Implementing (out of scope for 2.2e):
- OCR-based ad detection (separate task)
- Ad fingerprint pre-population (pre-existing whitelist)
- Frontend ad-scan UI components (separate from this port)
- N8n workflow modifications (separate from API port)

---

## Historical Testing Checklist

### Unit Tests:
- [ ] State machine: first scan forced dry_run=true
- [ ] Action key generation: `tenant:group:cluster:type:target:day`
- [ ] Confidence threshold filtering
- [ ] Warning dedup 5-column PK composition
- [ ] Tenant isolation (cross-tenant job_id access denied)

### Integration Tests:
- [ ] Create job → get job (status prepared)
- [ ] Configure + start job → status transitions to queued/scanning
- [ ] List groups returns tenant-scoped groups only
- [ ] Get clusters with tier filter works
- [ ] Action recording is idempotent
- [ ] Claimed warning senders respects 24-hour window

### End-to-End:
- [ ] Full scan job flow (create → confirm → execute → completed)
- [ ] Deletion blocked when AD_DELETE_ENABLED=false
- [ ] Dry-run mode skips actual Telegram API calls
- [ ] Clusters with confidence < 0.65 not applied as ad

---

## Handoff Notes

**Created files**:
- `crates/services/src/ad_governance.rs` — core service (700+ lines)
- `crates/http/src/routes/ad_scan.rs` — 5 HTTP endpoints
- `crates/workers/src/ad_governance_worker.rs` — job worker skeleton

**Registered in**:
- `crates/services/src/lib.rs` — pub mod ad_governance
- `crates/http/src/routes/mod.rs` — pub mod ad_scan + route registrations
- `crates/workers/src/lib.rs` — pub mod ad_governance_worker

**Current evidence**:
- `scripts/beta-runtime-check.sh` runs migrations through `0031` twice and
  records the local matrix in
  `target/beta-reports/runtime-gate-report.json`.
- `target/beta-reports/frontend-runtime-contract-report.json` records 102/102
  method/path requests; its limitation is that request tracing does not itself
  prove response schemas or real Telegram behavior.

**Next phase** should implement:
1. Full clustering algorithm (exact + template similarity + families)
2. LLM integration for unknown cluster classification
3. Action execution (delete, warn, mute)
4. Real-time scan triggering on message ingestion
