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
//! 暂存频道 id 从环境变量 `REVOKE_PROBE_CHAT_ID` 读取；未配置时整轮跳过
//! （防止 forward 回原群造成死循环，对齐 Python revoke_sync_chat_id 未配置跳过）。
//!
//! 该 worker 不依赖 job_queue：由 `main.rs` 以 `tokio::spawn` 启动常驻循环
//! （与 scheduler 的 channel_reminder_cleanup 周期任务同构）。

use anyhow::{Context, Result};
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
pub async fn run_revoke_sync_loop(
    pool: PgPool,
    telegram_resolver: Arc<dyn TelegramClientResolver>,
    interval: Duration,
) {
    info!("revoke sync loop started, interval={:?}", interval);
    loop {
        tokio::time::sleep(interval).await;
        if let Err(error) = run_revoke_sync_round(&pool, telegram_resolver.clone()).await {
            debug!("revoke sync round error: {}", error);
        }
    }
}

/// 执行一轮防撤回对账（可独立调用，便于测试）。
pub async fn run_revoke_sync_round(
    pool: &PgPool,
    telegram_resolver: Arc<dyn TelegramClientResolver>,
) -> Result<usize> {
    // 暂存频道 id（环境变量；未配置跳过，避免 forward 回原群死循环）
    let probe_chat_id: Option<i64> = std::env::var("REVOKE_PROBE_CHAT_ID")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value != 0);
    let Some(probe_chat_id) = probe_chat_id else {
        debug!("REVOKE_PROBE_CHAT_ID 未配置，跳过本轮防撤回对账");
        return Ok(0);
    };

    // 开启信息校对 + 监听的群（对齐 Python get_groups_with_revoke_sync）。
    // 每个群使用自己的 revoke_sync_chat_id 作为试探目标（Python 行为）；
    // 环境变量 REVOKE_PROBE_CHAT_ID 作为全局门控（未配置则整轮跳过）。
    let groups = sqlx::query(
        "SELECT group_id, revoke_sync_chat_id, tenant_id \
         FROM groups.tracked_groups \
         WHERE delete_on_revoke = true AND is_monitored = true \
           AND revoke_sync_chat_id IS NOT NULL",
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
        let permit = semaphore.clone();
        let pool = pool.clone();
        let resolver = telegram_resolver.clone();
        // 优先使用群自身配置的暂存频道，缺失时回退到全局 env（对齐 Python 行为）
        let probe = if target_chat_id != 0 {
            target_chat_id
        } else {
            probe_chat_id
        };
        handles.push(tokio::spawn(async move {
            let _guard = permit.acquire_owned().await;
            check_group_revoke(&pool, resolver.as_ref(), group_id, probe, &tenant_id).await
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
async fn check_group_revoke(
    pool: &PgPool,
    telegram_resolver: &dyn TelegramClientResolver,
    group_id: i64,
    probe_chat_id: i64,
    tenant_id: &str,
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
                // 消息还在：删除暂存副本 + 标记已校对（防止重复试探死循环）
                let _ = telegram.delete_message(probe_chat_id, forwarded.message_id).await;
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
