use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tgbot_integrations::binance::BinanceClient;
use tgbot_integrations::telegram::{SendMessageParams, TelegramClient};
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
            TelegramCommand::Status => {
                system_status_reply(pool, &key.tenant_id, &locale).await?
            }
            TelegramCommand::Ping => (if locale == "en" { "pong 🏓".to_string() } else { "pong 🏓".to_string() }, None),
            TelegramCommand::Email => {
                // 对齐 Python /email：绑定或查看邮箱（存 auth.users.email）
                let email_arg: Option<String> = row
                    .try_get::<String, _>("content")
                    .ok()
                    .and_then(|text| {
                        let mut parts = text.split_whitespace();
                        parts.next();
                        parts.next().map(|arg| arg.trim().to_string())
                    });
                let email_opt: Option<String> = sqlx::query_scalar(
                    "SELECT email FROM auth.users WHERE tg_user_id = $1",
                )
                .bind(user_id)
                .fetch_optional(pool)
                .await?;
                match email_arg {
                    Some(email) => {
                        let valid = email.contains('@')
                            && email.split('@').nth(1).is_some_and(|d| d.contains('.'));
                        if !valid {
                            (if locale == "en" { "Invalid email address".into() } else { "邮箱格式无效".into() }, None)
                        } else {
                            sqlx::query(
                                "UPDATE auth.users SET email = $1, updated_at = now() WHERE tg_user_id = $2",
                            )
                            .bind(&email)
                            .bind(user_id)
                            .execute(pool)
                            .await?;
                            (if locale == "en" { format!("Email bound: {email}") } else { format!("邮箱已绑定: {email}") }, None)
                        }
                    }
                    None => match email_opt {
                        Some(email) if !email.trim().is_empty() => {
                            (if locale == "en" { format!("Current email: {email}") } else { format!("当前绑定邮箱: {email}") }, None)
                        }
                        _ => (if locale == "en" { "No email bound. Send /email <address> to bind.".into() } else { "尚未绑定邮箱。发送 /email <邮箱> 进行绑定。".into() }, None),
                    },
                }
            }
            TelegramCommand::Price(symbol) => {
                price_reply(pool, symbol.as_deref(), &locale).await?
            }
            TelegramCommand::Reply => {
                // 管理员指定用户回复：/reply <user_id> <text>
                let content = row.try_get::<String, _>("content").unwrap_or_default();
                let parts: Vec<&str> = content.splitn(3, char::is_whitespace).collect();
                if parts.len() >= 3 && parts[0] == "/reply" {
                    match parts[1].parse::<i64>() {
                        Ok(target) if target > 0 => {
                            let text = parts[2].trim().to_string();
                            if text.is_empty() {
                                (if locale == "en" { "Empty reply content".into() } else { "回复内容为空".into() }, None)
                            } else {
                                match admin_reply_to_user(pool, &telegram, &key.tenant_id, target, &text).await {
                                    Ok(()) => (if locale == "en" { "Reply sent".into() } else { "回复已发送".into() }, None),
                                    Err(e) => (format!("Reply failed: {e}"), None),
                                }
                            }
                        }
                        _ => (if locale == "en" { "Invalid user id. Usage: /reply <user_id> <text>".into() } else { "用户 ID 无效。用法：/reply <用户ID> <内容>".into() }, None),
                    }
                } else {
                    (if locale == "en" { "Usage: /reply <user_id> <text>".into() } else { "用法：/reply <用户ID> <内容>".into() }, None)
                }
            }
            TelegramCommand::RestartBot => {
                // 管理员重启：触发进程退出，由 systemd Restart=always 拉起
                let is_owner = is_tenant_owner(pool, &key.tenant_id, user_id).await.unwrap_or(false);
                if is_owner {
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        std::process::exit(0);
                    });
                    (if locale == "en" { "Restarting bot...".into() } else { "正在重启机器人...".into() }, None)
                } else {
                    (if locale == "en" { "Permission denied".into() } else { "无权限执行此命令".into() }, None)
                }
            }
            TelegramCommand::RegisterMerchant(name) => {
                register_merchant_reply(pool, user_id, name.as_deref(), &locale).await?
            }
            TelegramCommand::SyncPotential => {
                sync_potential_reply(pool, &key.tenant_id, user_id, &locale).await?
            }
            _ => {
                let command_name = match command {
                    TelegramCommand::Help => "help",
                    TelegramCommand::Wallets => "wallets",
                    TelegramCommand::Start(_) => "start",
                    TelegramCommand::WalletMonitor(_) => "wallet_monitor",
                    TelegramCommand::Menu => "menu",
                    TelegramCommand::Status => "status",
                    TelegramCommand::Clear => "clear",
                    TelegramCommand::Ping => "ping",
                    TelegramCommand::Email => "email",
                    TelegramCommand::Price(_) => "price",
                    TelegramCommand::Reply => "reply",
                    TelegramCommand::RestartBot => "restart_bot",
                    TelegramCommand::RegisterMerchant(_) => "register_merchant",
                    TelegramCommand::SyncPotential => "sync_potential",
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




/// 校验用户是否为租户 owner（管理员命令权限）。
async fn is_tenant_owner(
    pool: &PgPool,
    tenant_id: &str,
    user_id: i64,
) -> Result<bool, sqlx::Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM merchants WHERE tenant_id = $1 AND owner_tg_user_id = $2)",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

/// /register_merchant <商户名>：自助创建商户档案（对齐 Python merchant_onboarding.py 简化版）。
/// - 校验商户名 2-40 字符
/// - 已存在则提示
/// - 创建 merchants 记录（tenant_id = m-{user_id}-merchant，status = active_free）
/// - 付费套餐流程跳过（记录为简化版）
async fn register_merchant_reply(
    pool: &PgPool,
    user_id: i64,
    name: Option<&str>,
    locale: &str,
) -> Result<(String, Option<Value>), anyhow::Error> {
    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        return Ok((
            if locale == "en" {
                "Usage: /register_merchant <merchant name>".into()
            } else {
                "用法：/register_merchant <商户名>".into()
            },
            None,
        ));
    };
    let char_count = name.chars().count();
    if char_count < 2 {
        return Ok((
            if locale == "en" {
                "Name too short, at least 2 characters.".into()
            } else {
                "❌ 名称太短了，请至少输入 2 个字。".into()
            },
            None,
        ));
    }
    if char_count > 40 {
        return Ok((
            if locale == "en" {
                "Name too long, at most 40 characters.".into()
            } else {
                "❌ 名称太长了，请控制在 40 个字以内。".into()
            },
            None,
        ));
    }

    // 已存在则提示（对齐 Python get_merchant_by_owner）
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM merchants WHERE owner_tg_user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    if exists {
        return Ok((
            if locale == "en" {
                "You already have a merchant profile.".into()
            } else {
                "🏪 你已经有商户档案了".into()
            },
            None,
        ));
    }

    let tenant_id_new = format!("m-{}-merchant", user_id);
    let result = sqlx::query(
        "INSERT INTO merchants \
         (tenant_id, merchant_name, owner_tg_user_id, status, plan_type, subscription_status, onboarding_completed) \
         VALUES ($1, $2, $3, 'active_free', 'free', 'active', true) \
         ON CONFLICT (tenant_id) DO NOTHING",
    )
    .bind(&tenant_id_new)
    .bind(name)
    .bind(user_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Ok((
            if locale == "en" {
                "Merchant profile creation failed (may already exist).".into()
            } else {
                "⚠️ 商户档案创建失败（可能已存在）。".into()
            },
            None,
        ));
    }

    let text = if locale == "en" {
        format!(
            "✅ Merchant profile created\nName: {name}\nTenant: {tenant_id_new}\nPlan: Free\n\nNext: create a bot via @BotFather and bind its token in the console."
        )
    } else {
        format!(
            "✅ 商户档案已创建\n\n商户名称：{name}\n租户标识：{tenant_id_new}\n当前套餐：免费套餐\n\n接下来：1. 点击 @BotFather 创建机器人；2. 复制 Bot Token；3. 在商户控制台提交 Token 完成接入。"
        )
    };
    info!(user_id, tenant_id_new, merchant_name = name, "merchant registered via /register_merchant");
    Ok((text, None))
}

/// /sync_potential：管理员立即同步群组潜在用户（对齐 Python admin_panel.py 简化版）。
/// 仅租户 owner 可触发；直接执行一次潜客 upsert（ad_score >= 40 的群消息）。
async fn sync_potential_reply(
    pool: &PgPool,
    tenant_id: &str,
    user_id: i64,
    locale: &str,
) -> Result<(String, Option<Value>), anyhow::Error> {
    let is_owner = is_tenant_owner(pool, tenant_id, user_id).await.unwrap_or(false);
    if !is_owner {
        return Ok((
            if locale == "en" {
                "Permission denied".into()
            } else {
                "无权限执行此命令".into()
            },
            None,
        ));
    }
    // 简化版潜客同步：将 monitored 群内 ad_score >= 40 的未捕获消息 upsert 到 potential_users
    let result = sqlx::query(
        "INSERT INTO tenant_saas.potential_users \
         (user_id, username, first_name, last_name, message_text, keywords, profile_tags, \
          group_id, group_title, tenant_id, captured_at) \
         SELECT DISTINCT ON (gm.tenant_id, gm.user_id) \
                gm.user_id, gm.username, member.first_name, member.last_name, \
                left(gm.content, 2000), '', '', \
                gm.group_id, tracked.group_title, gm.tenant_id, gm.timestamp \
         FROM groups.group_messages gm \
         JOIN groups.tracked_groups tracked \
           ON tracked.group_id = gm.group_id AND tracked.tenant_id = gm.tenant_id \
         LEFT JOIN groups.group_members member \
           ON member.group_id = gm.group_id AND member.user_id = gm.user_id \
          AND member.tenant_id = gm.tenant_id \
         WHERE gm.tenant_id = $1 \
           AND tracked.is_monitored = true \
           AND gm.sender_type = 'user' \
           AND gm.ad_score >= 40 \
           AND gm.tg_msg_id IS NOT NULL \
           AND NULLIF(btrim(gm.content), '') IS NOT NULL \
         ORDER BY gm.tenant_id, gm.user_id, gm.timestamp DESC \
         ON CONFLICT (tenant_id, user_id) DO UPDATE \
           SET username = EXCLUDED.username, first_name = EXCLUDED.first_name, \
               last_name = EXCLUDED.last_name, message_text = EXCLUDED.message_text, \
               group_id = EXCLUDED.group_id, group_title = EXCLUDED.group_title, \
               updated_at = now()",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    let affected = result.rows_affected();
    let text = if locale == "en" {
        format!(
            "✅ Potential user sync triggered (Beta simplified). Captured/updated {affected} users."
        )
    } else {
        format!("✅ 已触发潜客同步（Beta 简化版），本次捕获/更新 {affected} 位潜客。")
    };
    info!(tenant_id, affected, "potential user sync triggered via /sync_potential");
    Ok((text, None))
}

/// /price 行情查询（Binance 公共 API，对齐 Python crypto_price.py）。
async fn price_reply(
    pool: &PgPool,
    symbol: Option<&str>,
    locale: &str,
) -> Result<(String, Option<Value>), anyhow::Error> {
    let symbol = symbol.map(str::trim).filter(|s| !s.is_empty());
    let Some(symbol) = symbol else {
        return Ok((
            if locale == "en" {
                "Usage: /price <SYMBOL>  e.g. /price BTC".into()
            } else {
                "用法：/price <币种>  例如 /price BTC".into()
            },
            None,
        ));
    };
    let sym = if symbol.contains('/') {
        symbol.replace('/', "")
    } else if symbol.to_uppercase().ends_with("USDT") {
        symbol.to_string()
    } else {
        format!("{symbol}USDT")
    };
    let client = BinanceClient::from_env().context("failed to init binance client")?;
    match client.get_24hr_ticker(&sym).await {
        Ok(t) => {
            let text = if locale == "en" {
                format!(
                    "💰 {}\nPrice: {}\n24h: {}%\nRange: {} ~ {}",
                    t.symbol, t.last_price, t.price_change_percent, t.low_price, t.high_price
                )
            } else {
                format!(
                    "💰 {}\n最新价: {}\n24h: {}%\n区间: {} ~ {}",
                    t.symbol, t.last_price, t.price_change_percent, t.low_price, t.high_price
                )
            };
            Ok((text, None))
        }
        Err(error) => Ok((
            format!(
                "{} {}",
                if locale == "en" { "Query failed:" } else { "查询失败:" },
                error
            ),
            None,
        )),
    }
}

/// 管理员回复指定用户（/reply）：发送 + 入库。
async fn admin_reply_to_user(
    pool: &PgPool,
    telegram: &TelegramClient,
    tenant_id: &str,
    target: i64,
    text: &str,
) -> Result<(), String> {
    let bot_id: Option<i64> = sqlx::query_scalar("SELECT bot_id FROM merchants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    let bot_id = bot_id.unwrap_or(0);
    let sent = telegram
        .send_message(SendMessageParams {
            chat_id: target,
            text: text.to_string(),
            parse_mode: None,
            reply_markup: None,
            reply_to_message_id: None,
        })
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO chat.messages          (trace_id, from_userid, to_userid, content, msg_type, is_read, tg_msg_id, status, tenant_id)          VALUES (gen_random_uuid()::text, $1, $2, $3, 'text', true, $4, 'sent', $5)",
    )
    .bind(bot_id)
    .bind(target)
    .bind(text)
    .bind(sent.message_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// /status 真实运行报告（对齐 Python status.py 的健康报告，简化版）。
/// 返回：消息总数、24h 消息、未读用户数、监听群数、地址监听数。
async fn system_status_reply(
    pool: &PgPool,
    tenant_id: &str,
    locale: &str,
) -> Result<(String, Option<Value>), anyhow::Error> {
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.messages WHERE tenant_id = $1 AND is_deleted = false",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    let today: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.messages WHERE tenant_id = $1 AND is_deleted = false          AND timestamp >= now() - interval '24 hours'",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    let unread_users: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.admin_inbox WHERE tenant_id = $1 AND unread_count > 0",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    let groups: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM groups.tracked_groups WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    let monitors: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM address_monitors WHERE tenant_id = $1 AND is_active = true",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    let text = if locale == "en" {
        format!(
            "📊 System Status

Messages (total): {total}
Messages (24h): {today}
Unread users: {unread_users}
Tracked groups: {groups}
Active monitors: {monitors}"
        )
    } else {
        format!(
            "📊 系统运行状态

消息总数: {total}
24 小时消息: {today}
未读会话: {unread_users}
监听群数: {groups}
地址监听: {monitors}"
        )
    };
    Ok((text, None))
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
