// Telegram getUpdates long-polling receive mode.
//
// When enabled (`TGBOT_POLLING_ENABLED`, default true), every registered bot is
// polled via the Bot API's getUpdates long-poll endpoint instead of receiving
// webhook pushes. This removes the dependency on a public HTTPS ingress
// (Cloudflare Quick Tunnel) while reusing the exact same dispatch pipeline as
// the webhook path: raw JSON -> `TelegramUpdate` -> `UpdateDispatcher`.
//
// Concurrency model: one tokio task per bot (Telegram only allows a single
// concurrent getUpdates connection per bot) plus a supervisor that diffs the
// registry against the running tasks on an interval, so bots added/removed at
// runtime are picked up.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::routes::telegram_webhook::{BotEntry, BotRegistry, TelegramUpdate, UpdateDispatcher};
use tgbot_integrations::telegram::{GetUpdatesParams, TelegramClient, TelegramConfig};

/// Tuning knobs for the polling receive mode.
#[derive(Clone, Debug)]
pub struct PollingOptions {
    /// Telegram Bot API base URL.
    pub api_base_url: String,
    /// Long-poll timeout seconds passed to getUpdates (default 50).
    pub long_poll_secs: i32,
    /// Max updates per getUpdates call (default 100, Telegram's cap).
    pub limit: i32,
    /// Reqwest client timeout — MUST be greater than `long_poll_secs`
    /// (default 75) so the long poll is not cut short by the HTTP client.
    pub client_timeout_secs: u64,
    /// How often the supervisor diffs registry vs running tasks (default 30s).
    pub reconcile_interval: Duration,
    /// Optional DB pool for offset persistence across restarts.
    pub db: Option<sqlx::PgPool>,
}

/// Supervisor: keeps exactly one poll task per registered bot.
///
/// Every `reconcile_interval` it diffs the registry against the live tasks,
/// spawning new ones and aborting removed ones. On shutdown it aborts all.
pub async fn start_polling_supervisor(
    registry: BotRegistry,
    dispatcher: Arc<UpdateDispatcher>,
    options: PollingOptions,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tasks: HashMap<i64, tokio::task::JoinHandle<()>> = HashMap::new();
        if *shutdown.borrow() {
            return;
        }
        // Initial reconcile right away so bots start polling immediately
        // instead of waiting a full reconcile interval.
        reconcile_once(&registry, &dispatcher, &options, &mut tasks, &shutdown).await;
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!("polling supervisor shutting down; aborting all poll tasks");
                    for (_, handle) in tasks.drain() {
                        handle.abort();
                    }
                    return;
                }
                _ = tokio::time::sleep(options.reconcile_interval) => {
                    reconcile_once(&registry, &dispatcher, &options, &mut tasks, &shutdown).await;
                }
            }
        }
    })
}

/// One diff pass: spawn tasks for new bots, abort tasks for removed bots, and
/// respawn tasks that crashed (finished) so a bot is never left unpolled.
async fn reconcile_once(
    registry: &BotRegistry,
    dispatcher: &Arc<UpdateDispatcher>,
    options: &PollingOptions,
    tasks: &mut HashMap<i64, tokio::task::JoinHandle<()>>,
    shutdown: &watch::Receiver<bool>,
) {
    let entries = registry.list_entries().await;
    let live: HashSet<i64> = entries.iter().map(|e| e.bot_id).collect();
    for entry in entries {
        let needs_spawn = match tasks.get(&entry.bot_id) {
            Some(handle) => handle.is_finished(),
            None => true,
        };
        if !needs_spawn {
            continue;
        }
        let e = entry.clone();
        let d = dispatcher.clone();
        let opt = options.clone();
        let sd = shutdown.clone();
        tasks.insert(
            entry.bot_id,
            tokio::spawn(poll_bot_loop(e, d, opt, sd)),
        );
        info!(bot_id = entry.bot_id, "Polling task spawned");
    }
    let dead: Vec<i64> = tasks.keys().filter(|id| !live.contains(id)).cloned().collect();
    for id in dead {
        if let Some(handle) = tasks.remove(&id) {
            handle.abort();
        }
        info!(bot_id = id, "Polling task aborted");
    }
}

/// Long-poll loop for a single bot.
///
/// - Builds its own `TelegramClient` with `client_timeout_secs` (the shared
///   worker client's 30s timeout would cut the 50s long poll short).
/// - Deletes the webhook on start (idempotent) so getUpdates does not 409.
/// - Initializes offset from the persisted last_update_id (+1); `None` means
///   first run, letting Telegram replay any pending (up to 24h) updates.
/// - Advances the offset only when a whole batch dispatched successfully;
///   dispatch failures redeliver the batch (at-least-once); bad payloads are
///   skipped; 5 consecutive failed rounds trip a circuit breaker that
///   force-advances past the batch.
async fn poll_bot_loop(
    entry: BotEntry,
    dispatcher: Arc<UpdateDispatcher>,
    options: PollingOptions,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match TelegramClient::new(TelegramConfig {
        bot_token: entry.bot_token.clone(),
        api_base_url: options.api_base_url.clone(),
        timeout_secs: options.client_timeout_secs,
    }) {
        Ok(client) => client,
        Err(e) => {
            error!(bot_id = entry.bot_id, error = %e, "failed to build polling client");
            return;
        }
    };

    // Receive-mode exclusivity: deleteWebhook is idempotent.
    if let Err(e) = client.delete_webhook().await {
        warn!(bot_id = entry.bot_id, error = %e, "deleteWebhook failed; polling may see 409");
    } else {
        info!(bot_id = entry.bot_id, "webhook deleted; polling enabled");
    }

    // Offset init: persisted last_update_id + 1; None on first run (backfill).
    let mut offset = load_last_update_id(&options.db, entry.bot_id)
        .await
        .map(|last| last + 1);
    if let Some(off) = offset {
        info!(bot_id = entry.bot_id, last_update_id = off - 1, "resuming polling from persisted offset");
    } else {
        info!(bot_id = entry.bot_id, "no persisted offset; polling will replay pending updates");
    }

    let mut consecutive_failures = 0u32;
    let mut backoff = 2u64;

    loop {
        let updates = tokio::select! {
            _ = shutdown.changed() => break,
            result = client.get_updates_raw(GetUpdatesParams {
                offset,
                limit: Some(options.limit),
                timeout: Some(options.long_poll_secs),
            }) => result,
        };

        match updates {
            Ok(items) if items.is_empty() => {
                // Long-poll timed out naturally; reset backoff.
                consecutive_failures = 0;
                backoff = 2;
            }
            Ok(items) => {
                let (new_offset, all_ok) = process_batch(
                    &items,
                    |update| dispatcher.dispatch(update, entry.clone()),
                    offset,
                )
                .await;
                if all_ok {
                    offset = new_offset;
                    save_last_update_id(&options.db, entry.bot_id, offset).await;
                    consecutive_failures = 0;
                    backoff = 2;
                } else {
                    consecutive_failures += 1;
                    if let Some(forced) = check_circuit_breaker(&items, consecutive_failures) {
                        warn!(bot_id = entry.bot_id, "5 consecutive failed rounds; force-advancing offset");
                        offset = Some(forced);
                        save_last_update_id(&options.db, entry.bot_id, offset).await;
                        consecutive_failures = 0;
                    }
                }
            }
            Err(e) => {
                let message = e.to_string();
                if message.contains("409") {
                    error!(bot_id = entry.bot_id, error = %e, "409 conflict: webhook re-registered; deleting webhook");
                    let _ = client.delete_webhook().await;
                }
                if message.contains("429") {
                    let wait = extract_retry_after(&message).unwrap_or(backoff);
                    warn!(bot_id = entry.bot_id, seconds = wait, "429 rate limited; backing off");
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                } else {
                    warn!(bot_id = entry.bot_id, error = %e, "getUpdates transient error; backing off");
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(60);
                }
            }
        }
    }
}

/// Process a batch of raw updates in order. Advances `new_offset` past every
/// update that dispatched successfully or failed to deserialize (both are
/// terminal). Stops at the first dispatch error so the batch is redelivered.
///
/// Returns `(new_offset, all_ok)`. The caller only commits `new_offset` when
/// `all_ok` is true; otherwise the previous offset is kept (at-least-once).
async fn process_batch<F, Fut>(
    items: &[Value],
    mut dispatch: F,
    old_offset: Option<i64>,
) -> (Option<i64>, bool)
where
    F: FnMut(TelegramUpdate) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut new_offset = old_offset;
    let mut all_ok = true;
    for raw in items {
        let update_id = raw.get("update_id").and_then(Value::as_i64).unwrap_or(0);
        match serde_json::from_value::<TelegramUpdate>(raw.clone()) {
            Ok(update) => match dispatch(update).await {
                Ok(()) => {
                    new_offset = Some(update_id + 1);
                }
                Err(e) => {
                    error!(update_id, error = %e, "dispatch failed; keeping offset for redelivery");
                    all_ok = false;
                    break;
                }
            },
            Err(e) => {
                warn!(update_id, error = %e, "bad update payload; skipped");
                new_offset = Some(update_id + 1);
            }
        }
    }
    (new_offset, all_ok)
}

/// Circuit breaker: once a batch fails to dispatch N consecutive rounds, force
/// the offset past it so a poison update cannot stall the queue forever.
/// Returns the forced offset when tripped (5 or more consecutive failures).
fn check_circuit_breaker(items: &[Value], consecutive_failures: u32) -> Option<i64> {
    if consecutive_failures >= 5 {
        return max_update_id_plus_one(items);
    }
    None
}

/// `max(update_id) + 1` over a batch — the offset that confirms the batch.
fn max_update_id_plus_one(items: &[Value]) -> Option<i64> {
    items
        .iter()
        .filter_map(|v| v.get("update_id").and_then(Value::as_i64))
        .max()
        .map(|max| max + 1)
}

/// Load the last confirmed update_id for a bot. Best-effort: DB failures only
/// warn and fall back to `None` (start without offset, replay pending updates).
async fn load_last_update_id(db: &Option<sqlx::PgPool>, bot_id: i64) -> Option<i64> {
    let Some(pool) = db else {
        return None;
    };
    match sqlx::query_scalar::<_, i64>(
        "SELECT last_update_id FROM bot_polling_state WHERE bot_id = $1",
    )
    .bind(bot_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(last)) => Some(last),
        Ok(None) => None,
        Err(e) => {
            warn!(bot_id, error = %e, "failed to load last update id");
            None
        }
    }
}

/// Persist the last confirmed update_id for a bot (UPSERT). Best-effort:
/// failures only warn, the in-memory offset still advances.
async fn save_last_update_id(db: &Option<sqlx::PgPool>, bot_id: i64, offset: Option<i64>) {
    let (Some(pool), Some(last)) = (db, offset) else {
        return;
    };
    if let Err(e) = sqlx::query(
        "INSERT INTO bot_polling_state (bot_id, last_update_id) VALUES ($1, $2) \
         ON CONFLICT (bot_id) DO UPDATE SET last_update_id = EXCLUDED.last_update_id, updated_at = now()",
    )
    .bind(bot_id)
    .bind(last)
    .execute(pool)
    .await
    {
        warn!(bot_id, error = %e, "failed to save last update id");
    }
}

/// Best-effort extraction of Telegram's `retry_after` seconds from an error
/// string. Handles the JSON body form (`"retry_after":120` / `retry_after: 120`)
/// and the prose form (`retry after 120`). Returns `None` when not found.
fn extract_retry_after(error: &str) -> Option<u64> {
    let jsonish = regex::Regex::new(r#""?retry_after"?\s*[:=]\s*(\d+)"#).ok()?;
    if let Some(caps) = jsonish.captures(error) {
        if let Some(m) = caps.get(1) {
            if let Ok(secs) = m.as_str().parse::<u64>() {
                return Some(secs);
            }
        }
    }
    let prose = regex::Regex::new(r"(?i)retry\s+(?:after|in)\s+(\d+)").ok()?;
    if let Some(caps) = prose.captures(error) {
        if let Some(m) = caps.get(1) {
            if let Ok(secs) = m.as_str().parse::<u64>() {
                return Some(secs);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn upd(id: i64) -> Value {
        json!({
            "update_id": id,
            "message": {
                "message_id": 1,
                "date": 0,
                "chat": { "id": 1, "type": "private" },
                "text": "hello"
            }
        })
    }

    #[tokio::test]
    async fn offset_advances_when_whole_batch_ok() {
        let items = vec![upd(10), upd(11), upd(12)];
        let (new_offset, all_ok) = process_batch(&items, |_| async { Ok(()) }, None).await;
        assert!(all_ok);
        assert_eq!(new_offset, Some(13));
    }

    #[tokio::test]
    async fn offset_kept_on_partial_failure() {
        let items = vec![upd(10), upd(11), upd(12)];
        let (_, all_ok) = process_batch(
            &items,
            |update| async move {
                if update.update_id == 11 {
                    Err("handler exploded".into())
                } else {
                    Ok(())
                }
            },
            None,
        )
        .await;
        // all_ok == false => caller must NOT commit the new offset (redelivery).
        assert!(!all_ok);
    }

    #[tokio::test]
    async fn bad_payload_skipped_and_queue_continues() {
        let items = vec![
            json!({ "update_id": "not-an-int" }), // fails deserialize -> skipped
            upd(10),
        ];
        let (new_offset, all_ok) = process_batch(&items, |_| async { Ok(()) }, None).await;
        assert!(all_ok);
        assert_eq!(new_offset, Some(11));
    }

    #[test]
    fn circuit_breaker_trips_after_5_failures() {
        let items = vec![upd(100), upd(101)];
        assert_eq!(check_circuit_breaker(&items, 4), None);
        assert_eq!(check_circuit_breaker(&items, 5), Some(102));
        assert_eq!(check_circuit_breaker(&items, 7), Some(102));
    }

    #[test]
    fn max_update_id_plus_one_computes() {
        assert_eq!(max_update_id_plus_one(&[upd(1), upd(7), upd(3)]), Some(8));
        assert_eq!(max_update_id_plus_one(&[]), None);
    }

    #[test]
    fn extract_retry_after_parses_known_formats() {
        assert_eq!(
            extract_retry_after("Too Many Requests: retry after 120"),
            Some(120)
        );
        assert_eq!(
            extract_retry_after(r#"{"ok":false,"error_code":429,"parameters":{"retry_after":8}}"#),
            Some(8)
        );
        assert_eq!(extract_retry_after("HTTP 429: retry_after:120"), Some(120));
        assert_eq!(extract_retry_after("connection reset by peer"), None);
    }
}
