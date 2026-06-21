//! Broadcast service – message sending logic and worker integration.
use anyhow::Result;
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::{info, error};
use tokio::time::{sleep, Duration};

/// Send a broadcast message to a list of user IDs.
///
/// This is a placeholder implementation that would integrate with the Telegram bot.
/// In production, this would use the bot client to send messages.
pub async fn send_broadcast_to_users(
    pool: &PgPool,
    user_ids: &[i64],
    content: &str,
    do_translate: bool,
) -> Result<Value> {
    let total = user_ids.len();
    let mut success = 0;
    let mut failed = 0;
    let mut errors: Vec<Value> = Vec::new();

    info!("Starting broadcast to {} users", total);

    // Placeholder: in production, this would iterate through user_ids
    // and call the Telegram bot API to send messages.
    // For now, we simulate success for all users.
    for user_id in user_ids {
        // Simulate sending (replace with actual bot API call)
        // await bot.send_message(*user_id, content);
        success += 1;

        // Rate limiting would go here
        sleep(Duration::from_millis(50)).await;
    }

    info!("Broadcast completed: {} success, {} failed", success, failed);

    Ok(json!({
        "total": total,
        "success": success,
        "failed": failed,
        "errors": errors,
    }))
}

/// Send a broadcast message to a list of chat IDs (groups/channels).
pub async fn send_broadcast_to_chats(
    pool: &PgPool,
    chat_ids: &[i64],
    content: &str,
    do_translate: bool,
) -> Result<Value> {
    let total = chat_ids.len();
    let mut success = 0;
    let mut failed = 0;
    let mut success_ids: Vec<i64> = Vec::new();
    let mut failed_items: Vec<Value> = Vec::new();

    info!("Starting broadcast to {} chats", total);

    // Placeholder: in production, this would iterate through chat_ids
    // and call the Telegram bot API to send messages to groups/channels.
    for chat_id in chat_ids {
        // Simulate sending (replace with actual bot API call)
        // await bot.send_message(*chat_id, content);
        success += 1;
        success_ids.push(*chat_id);

        // Rate limiting would go here
        sleep(Duration::from_millis(100)).await;
    }

    info!("Chat broadcast completed: {} success, {} failed", success, failed);

    Ok(json!({
        "total": total,
        "success": success,
        "failed": failed,
        "success_ids": success_ids,
        "failed_items": failed_items,
    }))
}

/// Execute a push task immediately.
pub async fn execute_push_task(
    pool: &PgPool,
    task_id: i64,
) -> Result<Value> {
    // Fetch the task
    let task = sqlx::query("SELECT * FROM push_tasks WHERE id = $1")
        .bind(task_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("task not found"))?;

    // Extract task details
    let content: String = task.try_get("content")?;
    let do_translate: bool = task.try_get("do_translate")?;
    let target_chat_ids: Value = task.try_get("target_chat_ids")?;

    // Parse target chat IDs
    let chat_ids: Vec<i64> = target_chat_ids
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_i64())
                .collect()
        })
        .unwrap_or_default();

    // Execute broadcast
    let result = send_broadcast_to_chats(pool, &chat_ids, &content, do_translate).await?;

    Ok(result)
}