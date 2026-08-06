use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tgbot_integrations::telegram::SendMessageParams;
use tgbot_repos::message_repo;
use tgbot_services::telegram_commands::{
    build_command_reply, parse_channel_start_payload, parse_telegram_command,
    parse_unmute_start_payload, TelegramCommand,
};
use tgbot_services::telegram_preferences::{
    localized_command_reply, localized_start_menu_reply, telegram_locale,
};
use tgbot_services::telegram_wallet::{
    normalize_wallet_address, register_wallet_monitor, wallet_list_reply_for_locale,
    wallet_monitor_prompt, wallet_registration_reply,
};
use tracing::{info, instrument};

use crate::job_queue::ClaimedJob;
use crate::push_task_worker::TelegramClientResolver;
use crate::scheduler::Worker;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramCommandJobKey {
    tenant_id: String,
    inbound_message_id: i64,
}

pub struct TelegramCommandWorker {
    telegram_resolver: Arc<dyn TelegramClientResolver>,
    wallet_monitors_enabled: bool,
}

impl TelegramCommandWorker {
    pub fn new(
        telegram_resolver: Arc<dyn TelegramClientResolver>,
        wallet_monitors_enabled: bool,
    ) -> Self {
        Self {
            telegram_resolver,
            wallet_monitors_enabled,
        }
    }
}

#[async_trait]
impl Worker for TelegramCommandWorker {
    fn job_type(&self) -> &str {
        "telegram_command_reply"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let key = parse_job_key(&claimed.job.payload)?;
        if claimed.job.tenant_id != key.tenant_id {
            bail!("telegram command job tenant mismatch");
        }

        let row = sqlx::query(
            r#"
            SELECT inbound.from_userid AS user_id,
                   inbound.to_userid AS bot_id,
                   inbound.content,
                   inbound.tg_msg_id,
                   merchant.bot_username,
                   sender.first_name
            FROM chat.messages inbound
            JOIN merchants merchant
              ON merchant.tenant_id = inbound.tenant_id
             AND merchant.bot_id = inbound.to_userid
             AND merchant.status IN ('active', 'active_free')
            LEFT JOIN chat.users sender
              ON sender.user_id = inbound.from_userid
            WHERE inbound.id = $2
              AND inbound.tenant_id = $1
              AND inbound.is_read = false
              AND inbound.is_deleted = false
              AND inbound.from_userid > 0
              AND inbound.to_userid > 0
              AND inbound.from_userid <> inbound.to_userid
              AND inbound.msg_type = 'text'
              AND inbound.tg_msg_id IS NOT NULL
              AND NULLIF(btrim(inbound.content), '') IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1
                  FROM chat.messages outbound
                  WHERE outbound.tenant_id = inbound.tenant_id
                    AND outbound.from_userid = inbound.to_userid
                    AND outbound.to_userid = inbound.from_userid
                    AND outbound.quoted_msg_id = inbound.tg_msg_id
                    AND outbound.is_read = true
                    AND outbound.is_deleted = false
              )
            "#,
        )
        .bind(&key.tenant_id)
        .bind(key.inbound_message_id)
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            info!(
                tenant_id = key.tenant_id,
                inbound_message_id = key.inbound_message_id,
                "telegram command reply job is stale or already delivered"
            );
            return Ok(());
        };

        let content: String = row.try_get("content")?;
        let bot_username: Option<String> = row.try_get("bot_username")?;
        let Some(command) = parse_telegram_command(&content, bot_username.as_deref().unwrap_or(""))
        else {
            return Ok(());
        };
        let first_name: Option<String> = row.try_get("first_name")?;
        let user_id: i64 = row.try_get("user_id")?;
        let bot_id: i64 = row.try_get("bot_id")?;
        let telegram_message_id: i64 = row.try_get("tg_msg_id")?;
        let locale = telegram_locale(pool, &key.tenant_id, user_id).await?;
        let telegram = self
            .telegram_resolver
            .resolve(pool, &key.tenant_id)
            .await
            .context("failed to resolve Telegram command client")?;
        let (reply_text, reply_markup) = match &command {
            TelegramCommand::Start(Some(payload)) => {
                start_payload_reply(pool, &key.tenant_id, &telegram, user_id, payload).await?
            }
            TelegramCommand::Start(None) => {
                localized_start_menu_reply(first_name.as_deref().unwrap_or("朋友"), &locale)
            }
            TelegramCommand::Wallets => {
                wallet_list_reply_for_locale(pool, &key.tenant_id, user_id, &locale).await?
            }
            TelegramCommand::WalletMonitor(address) => {
                if !self.wallet_monitors_enabled {
                    (
                        if locale == "en" {
                            "Address monitor commands are disabled in this Beta runtime.".into()
                        } else {
                            "当前 Beta 运行时未启用地址监听命令。".into()
                        },
                        None,
                    )
                } else if let Some(address) = address {
                    if normalize_wallet_address(address).is_err() {
                        (
                            if locale == "en" {
                                "Invalid address. Send a 34-character TRC20 address or a 0x-prefixed 40-byte BSC address.".into()
                            } else {
                                "地址格式无效。请发送 34 位 TRC20 地址，或 0x 开头的 40 位 BSC 地址。".into()
                            },
                            None,
                        )
                    } else {
                        let registration =
                            register_wallet_monitor(pool, &key.tenant_id, user_id, address)
                                .await
                                .context("failed to register Telegram wallet monitor")?;
                        (
                            wallet_registration_reply(&registration, &locale),
                            Some(json!({
                                "inline_keyboard": [[{
                                    "text": if locale == "en" { "View my monitors" } else { "查看我的监听" },
                                    "callback_data": "wallet_my_list"
                                }]]
                            })),
                        )
                    }
                } else {
                    (wallet_monitor_prompt(&locale), None)
                }
            }
            _ => {
                let command_name = match command {
                    TelegramCommand::Help => "help",
                    TelegramCommand::Wallets => "wallets",
                    TelegramCommand::Start(_) => "start",
                    TelegramCommand::WalletMonitor(_) => "wallet_monitor",
                };
                (
                    localized_command_reply(
                        command_name,
                        first_name.as_deref().unwrap_or("朋友"),
                        &locale,
                    ),
                    None,
                )
            }
        };
        let sent = telegram
            .send_message(SendMessageParams {
                chat_id: user_id,
                text: reply_text.clone(),
                parse_mode: None,
                reply_markup,
                reply_to_message_id: Some(telegram_message_id),
            })
            .await
            .context("failed to deliver Telegram command reply")?;

        message_repo::save_message(
            pool,
            bot_id,
            user_id,
            Some(&reply_text),
            "text",
            None,
            None,
            true,
            Some(sent.message_id),
            &key.tenant_id,
            Some(telegram_message_id),
        )
        .await
        .context("failed to persist Telegram command reply")?;

        info!(
            tenant_id = key.tenant_id,
            inbound_message_id = key.inbound_message_id,
            "telegram command reply delivered"
        );
        Ok(())
    }
}

async fn start_payload_reply(
    pool: &PgPool,
    tenant_id: &str,
    telegram: &tgbot_integrations::telegram::TelegramClient,
    user_id: i64,
    payload: &str,
) -> Result<(String, Option<Value>)> {
    if let Some(group_id) = parse_unmute_start_payload(payload) {
        return unmute_start_payload_reply(pool, tenant_id, telegram, user_id, group_id).await;
    }
    let Some((group_id, channel_id)) = parse_channel_start_payload(payload) else {
        return Ok((
            build_command_reply(&TelegramCommand::Start(Some(payload.to_string())), "朋友").text,
            None,
        ));
    };

    let config = sqlx::query(
        "SELECT verify_enabled, verify_type, verify_channel_id \
         FROM groups.tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .context("failed to load channel deep-link configuration")?;
    let valid_config = config.is_some_and(|row| {
        row.try_get::<bool, _>("verify_enabled").unwrap_or(false)
            && row.try_get::<String, _>("verify_type").ok().as_deref() == Some("channel")
            && row
                .try_get::<Option<String>, _>("verify_channel_id")
                .ok()
                .flatten()
                .and_then(|value| value.parse::<i64>().ok())
                == Some(channel_id)
    });
    if !valid_config {
        return Ok((
            "此验证链接已失效或不属于当前租户，请重新从群组获取验证链接。".into(),
            None,
        ));
    }

    let member = telegram
        .get_chat_member(channel_id, user_id)
        .await
        .context("failed to check channel membership for start payload")?;
    if telegram_member_is_subscribed(&member) {
        telegram
            .restrict_chat_member(
                group_id,
                user_id,
                json!({
                    "can_send_messages": true,
                    "can_send_polls": true,
                    "can_send_other_messages": true,
                    "can_add_web_page_previews": true,
                    "can_change_info": false,
                    "can_invite_users": true,
                    "can_pin_messages": false,
                }),
                None,
            )
            .await
            .context("failed to unrestrict member from start payload")?;
        sqlx::query(
            "UPDATE groups.group_members SET status = 'active', is_verified = true, \
             verified_at = now(), updated_at = now() \
             WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(group_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(pool)
        .await
        .context("failed to persist start payload verification")?;
        return Ok(("✅ 验证成功！您已被解禁，可以回到群组发言。".into(), None));
    }

    let channel_link = telegram.get_chat(channel_id).await.ok().and_then(|chat| {
        chat.get("username")
            .and_then(Value::as_str)
            .map(|name| format!("https://t.me/{}", name.trim_start_matches('@')))
    });
    let mut keyboard = Vec::new();
    if let Some(channel_link) = channel_link {
        keyboard.push(vec![json!({
            "text": "📢 直接关注",
            "url": channel_link,
        })]);
    }
    keyboard.push(vec![json!({
        "text": "✅ 已关注，重新验证",
        "callback_data": format!("subscribe_{group_id}_{channel_id}"),
    })]);
    Ok((
        "❌ 尚未检测到频道关注。请先关注频道，然后点击下方按钮重新验证。".into(),
        Some(json!({"inline_keyboard": keyboard})),
    ))
}

async fn unmute_start_payload_reply(
    pool: &PgPool,
    tenant_id: &str,
    telegram: &tgbot_integrations::telegram::TelegramClient,
    user_id: i64,
    group_id: i64,
) -> Result<(String, Option<Value>)> {
    let config = sqlx::query(
        "SELECT verify_enabled, verify_type, verify_channel_id \
         FROM groups.tracked_groups WHERE group_id = $1 AND tenant_id = $2",
    )
    .bind(group_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .context("failed to load unmute deep-link configuration")?;
    let Some(config) = config else {
        return Ok((
            "此解禁链接已失效或不属于当前租户，请重新从群组获取验证链接。".into(),
            None,
        ));
    };
    let valid_config = config.try_get::<bool, _>("verify_enabled").unwrap_or(false)
        && config.try_get::<String, _>("verify_type").ok().as_deref() == Some("channel");
    let channel_id = config
        .try_get::<Option<String>, _>("verify_channel_id")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value != 0);
    let Some(channel_id) = channel_id.filter(|_| valid_config) else {
        return Ok(("该群组未配置有效的频道验证，无法执行解禁。".into(), None));
    };

    let member_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM groups.group_members \
         WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3)",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .context("failed to verify group membership for unmute deep link")?;
    if !member_exists {
        return Ok((
            "未找到你的群组成员记录，请先重新加入群组后再试。".into(),
            None,
        ));
    }

    let member = telegram
        .get_chat_member(channel_id, user_id)
        .await
        .context("failed to check channel membership for unmute deep link")?;
    if !telegram_member_is_subscribed(&member) {
        let channel_link = telegram.get_chat(channel_id).await.ok().and_then(|chat| {
            chat.get("username")
                .and_then(Value::as_str)
                .map(|name| format!("https://t.me/{}", name.trim_start_matches('@')))
        });
        let mut keyboard = Vec::new();
        if let Some(channel_link) = channel_link {
            keyboard.push(vec![json!({
                "text": "📢 直接关注",
                "url": channel_link,
            })]);
        }
        keyboard.push(vec![json!({
            "text": "✅ 已关注，重新验证",
            "callback_data": format!("subscribe_{group_id}_{channel_id}"),
        })]);
        return Ok((
            "❌ 尚未检测到频道关注。请先关注频道，然后点击下方按钮重新验证。".into(),
            Some(json!({"inline_keyboard": keyboard})),
        ));
    }

    telegram
        .restrict_chat_member(
            group_id,
            user_id,
            json!({
                "can_send_messages": true,
                "can_send_polls": true,
                "can_send_other_messages": true,
                "can_add_web_page_previews": true,
                "can_change_info": false,
                "can_invite_users": true,
                "can_pin_messages": false,
            }),
            None,
        )
        .await
        .context("failed to unrestrict member from unmute deep link")?;
    sqlx::query(
        "UPDATE groups.group_members SET status = 'active', is_verified = true, \
         verified_at = now(), updated_at = now() \
         WHERE group_id = $1 AND user_id = $2 AND tenant_id = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .context("failed to persist unmute deep-link verification")?;
    Ok(("✅ 解禁成功！你已恢复在群组中的发言权限。".into(), None))
}

fn telegram_member_is_subscribed(member: &Value) -> bool {
    match member.get("status").and_then(Value::as_str) {
        Some("creator" | "administrator" | "member") => true,
        Some("restricted") => member
            .get("is_member")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        _ => false,
    }
}

fn parse_job_key(payload: &Value) -> Result<TelegramCommandJobKey> {
    let tenant_id = payload
        .get("tenant_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing tenant_id in telegram command payload"))?;
    let inbound_message_id = payload
        .get("inbound_message_id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("missing inbound_message_id in telegram command payload"))?;

    Ok(TelegramCommandJobKey {
        tenant_id: tenant_id.to_string(),
        inbound_message_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_tenant_scoped_job_key() {
        assert_eq!(
            parse_job_key(&json!({
                "tenant_id": "tenant-a",
                "inbound_message_id": 42
            }))
            .unwrap(),
            TelegramCommandJobKey {
                tenant_id: "tenant-a".into(),
                inbound_message_id: 42,
            }
        );
        assert!(parse_job_key(&json!({
            "tenant_id": "tenant-a",
            "inbound_message_id": 0
        }))
        .is_err());
        assert!(parse_job_key(&json!({"inbound_message_id": 42})).is_err());
    }
}
