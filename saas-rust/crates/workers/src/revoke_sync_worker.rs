//! Revoke sync worker – 防撤回对账（对齐 Python main.py revoke_sync_loop）。
//!
//! 每 30s 处理一轮（并发 ≤ 3 群），对 `groups.group_messages` 中最近 48h 内
//! 未校对（revoke_checked = false）的消息：
//!   1. 调用 Telegram `forwardMessage` 把消息转发到暂存频道试探是否存在。
//!      - 转发成功 → 消息仍在 → 删除暂存副本 + 标记 revoke_checked = true
//!        （避免下一轮重复试探造成死循环）。
//!      - 转发报 "message to forward not found" 等 → 消息已被撤回 →
//!        删除 DB 记录（对齐 Python delete_group_message）。
//!      - 触发限速（flood/too many）→ 暂停 5s 再继续。
//!
//! 暂存频道 id 优先从 DB 读取（`tenant_saas.tenant_configs['revoke_sync_chat_id']`，
//! 即 Rust 后台 `POST /api/group_listen/create_sync_chat` 的写入位置，对齐 Python
//! `admin_settings['revoke_sync_chat_id']` 的门控语义）；DB 缺失时回退到环境变量
//! `REVOKE_PROBE_CHAT_ID`（兼容旧部署）。两者都未配置时整轮跳过（防止 forward
//! 回原群造成死循环，对齐 Python revoke_sync_chat_id 未配置跳过）。
//!
//! 该 worker 不依赖 job_queue：由 `main.rs` 以 `tokio::spawn` 启动常驻循环
//! （与 scheduler 的 channel_reminder_cleanup 周期任务同构）。

use anyhow::{Context, Result};
use serde_json::json;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::push_task_worker::TelegramClientResolver;

/// 一轮对账最多同时处理的群数（对齐 Python Semaphore(3)）。
const MAX_CONCURRENT_GROUPS: usize = 3;
/// 消息试探的最大批量（对齐 Python LIMIT 200）。
const MAX_MESSAGES_PER_GROUP: i64 = 200;

/// 常驻对账循环：每 `interval` 执行一轮（对齐 Python `while True: sleep(30); process`）。
/// 由 `main.rs` 以 `tokio::spawn` 启动；进程退出时随 runtime 一并回收，无需优雅停机。
/// `ws_tx` 为 WebSocket 广播发送端：删除被撤回消息时推送 `group_message_deleted` 事件
/// （对齐 Python `notify_web_clients("group_message_deleted", ...)`），可为 None（无前端）。
pub async fn run_revoke_sync_loop(
    pool: PgPool,
    telegram_resolver: Arc<dyn TelegramClientResolver>,
    interval: Duration,
    ws_tx: Option<tokio::sync::broadcast::Sender<String>>,
) {
    info!("revoke sync loop started, interval={:?}", interval);
    loop {
        tokio::time::sleep(interval).await;
        if let Err(error) = run_revoke_sync_round(&pool, telegram_resolver.clone(), ws_tx.clone())
            .await
        {
            debug!("revoke sync round error: {}", error);
        }
    }
}

/// 执行一轮防撤回对账（可独立调用，便于测试）。
pub async fn run_revoke_sync_round(
    pool: &PgPool,
    telegram_resolver: Arc<dyn TelegramClientResolver>,
    ws_tx: Option<tokio::sync::broadcast::Sender<String>>,
) -> Result<usize> {
    // 全局门控兜底：环境变量 REVOKE_PROBE_CHAT_ID（DB 未配置时的回退项，兼容旧部署）
    let env_probe: Option<i64> = std::env::var("REVOKE_PROBE_CHAT_ID")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value != 0);

    // 开启信息校对 + 监听的群（对齐 Python get_groups_with_revoke_sync）。
    // 每个群使用自己的 revoke_sync_chat_id 作为试探目标（Python 行为）；
    // 租户级门控取 tenant_configs['revoke_sync_chat_id']（Rust 后台写入位置），
    // 缺失时回退到环境变量 REVOKE_PROBE_CHAT_ID。
    let groups = sqlx::query(
        "SELECT g.group_id, g.revoke_sync_chat_id, g.tenant_id, g.revoke_sync_mirror, \
                tc.config_value AS tenant_probe \
         FROM groups.tracked_groups g \
         LEFT JOIN tenant_saas.tenant_configs tc \
           ON tc.tenant_id = g.tenant_id AND tc.config_key = 'revoke_sync_chat_id' \
         WHERE g.delete_on_revoke = true AND g.is_monitored = true \
           AND g.revoke_sync_chat_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .context("failed to load groups with revoke sync")?;
    if groups.is_empty() {
        return Ok(0);
    }

    // 并发处理，最多同时 3 个群
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_GROUPS));
    let mut handles = Vec::new();
    for group in groups {
        let group_id: i64 = group.try_get("group_id")?;
        let target_chat_id: i64 = group.try_get("revoke_sync_chat_id")?;
        let tenant_id: String = group.try_get("tenant_id")?;
        let mirror: bool = group.try_get("revoke_sync_mirror")?;
        let tenant_probe: Option<String> = group.try_get("tenant_probe").ok();
        // 租户门控：tenant_configs 优先，其次环境变量（对齐 Python admin_settings 门控）
        let tenant_probe_id: Option<i64> = tenant_probe
            .as_deref()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .filter(|value| *value != 0);
        let probe = match tenant_probe_id.or(env_probe) {
            Some(id) if id != 0 => id,
            _ => {
                debug!("revoke sync: tenant {} 未配置校对群，跳过", tenant_id);
                continue;
            }
        };
        let permit = semaphore.clone();
        let pool = pool.clone();
        let resolver = telegram_resolver.clone();
        let ws_tx = ws_tx.clone();
        // 优先使用群自身配置的暂存频道，缺失时回退到租户/全局门控（对齐 Python 行为）
        let probe = if target_chat_id != 0 {
            target_chat_id
        } else {
            probe
        };
        handles.push(tokio::spawn(async move {
            let _guard = permit.acquire_owned().await;
            check_group_revoke(
                &pool,
                resolver.as_ref(),
                group_id,
                probe,
                &tenant_id,
                mirror,
                ws_tx,
            )
            .await
        }));
    }

    let mut total_deleted = 0_usize;
    for handle in handles {
        match handle.await {
            Ok(Ok(deleted)) => total_deleted += deleted,
            Ok(Err(error)) => debug!("revoke sync group error: {}", error),
            Err(error) => debug!("revoke sync group task join error: {}", error),
        }
    }
    if total_deleted > 0 {
        info!(total_deleted, "revoke sync round completed");
    }
    Ok(total_deleted)
}

/// 校对单个群：返回已删除的 tg_msg_id 数量。
/// `mirror` 为 true 时保留转发副本在校对群（双模式·镜像，供人工核验）；
/// false 时保持探测式（转发成功后立即删除副本，对齐 Python）。
async fn check_group_revoke(
    pool: &PgPool,
    telegram_resolver: &dyn TelegramClientResolver,
    group_id: i64,
    probe_chat_id: i64,
    tenant_id: &str,
    mirror: bool,
    ws_tx: Option<tokio::sync::broadcast::Sender<String>>,
) -> Result<usize> {
    let telegram = telegram_resolver
        .resolve(pool, tenant_id)
        .await
        .context("failed to resolve telegram client for revoke sync")?;

    // 最近 48h 内、未校对的群消息（对齐 Python cutoff_ts / LIMIT 200）
    let rows = sqlx::query(
        "SELECT tg_msg_id FROM groups.group_messages \
         WHERE group_id = $1 AND tenant_id = $2 \
           AND tg_msg_id IS NOT NULL \
           AND revoke_checked = false \
           AND COALESCE(to_timestamp(message_ts), timestamp) >= now() - interval '48 hours' \
         ORDER BY COALESCE(to_timestamp(message_ts), timestamp) DESC \
         LIMIT $3",
    )
    .bind(group_id)
    .bind(tenant_id)
    .bind(MAX_MESSAGES_PER_GROUP)
    .fetch_all(pool)
    .await
    .context("failed to load group messages for revoke sync")?;

    let mut deleted = 0_usize;
    for (index, row) in rows.iter().enumerate() {
        let tg_msg_id: i64 = row.try_get("tg_msg_id")?;
        match telegram
            .forward_message(probe_chat_id, group_id, tg_msg_id)
            .await
        {
            Ok(forwarded) => {
                // 镜像模式：保留副本在校对群供人工核验；探测模式：删除副本（对齐 Python）
                if !mirror {
                    let _ = telegram.delete_message(probe_chat_id, forwarded.message_id).await;
                }
                // 两种模式都标记已校对（防止重复试探死循环）
                let _ = sqlx::query(
                    "UPDATE groups.group_messages SET revoke_checked = true \
                     WHERE group_id = $1 AND tg_msg_id = $2 AND tenant_id = $3",
                )
                .bind(group_id)
                .bind(tg_msg_id)
                .bind(tenant_id)
                .execute(pool)
                .await;
            }
            Err(error) => {
                let err = error.to_string().to_lowercase();
                if err.contains("message to forward not found")
                    || err.contains("message_id_invalid")
                    || err.contains("invalid message")
                    || err.contains("message not found")
                {
                    // 消息已被撤回：删除 DB 记录
                    let result = sqlx::query(
                        "DELETE FROM groups.group_messages \
                         WHERE group_id = $1 AND tg_msg_id = $2 AND tenant_id = $3",
                    )
                    .bind(group_id)
                    .bind(tg_msg_id)
                    .bind(tenant_id)
                    .execute(pool)
                    .await?;
                    if result.rows_affected() > 0 {
                        deleted += 1;
                        info!(group_id, tg_msg_id, "message revoked, synced deletion");
                        // 实时通知前端移除消息（对齐 Python notify_web_clients group_message_deleted）
                        if let Some(tx) = ws_tx.as_ref() {
                            let _ = tx.send(
                                json!({
                                    "type": "group_message_deleted",
                                    "data": { "group_id": group_id, "tg_msg_id": tg_msg_id }
                                })
                                .to_string(),
                            );
                        }
                    }
                } else if err.contains("flood") || err.contains("too many") {
                    // 限速：暂停 5s 再继续（对齐 Python）
                    warn!("Telegram rate limited during revoke sync, pausing 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
        // 每处理 5 条暂停一下，避免触发 Telegram 限速（对齐 Python）
        if (index + 1) % 5 == 0 {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    Ok(deleted)
}
