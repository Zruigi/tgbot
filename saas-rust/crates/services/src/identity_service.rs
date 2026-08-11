//! Identity service — 基于官方 Telegram Bot API 的智能身份识别与绑定
//!
//! 背景：系统目前"识别不到群聊/频道/联系人"，Telegram 用户（对象 A）处于
//! "暂时未绑定"状态。本模块利用官方 Bot API 的能力（getChatMember /
//! getChatAdministrators / getChat），遍历某租户 bot 名下已跟踪的群/频道
//! （groups.tracked_groups），判定 A 在哪些群、是哪些群的管理员、有哪些频道，
//! 并把"是管理员/creator"的群绑定给 A：
//!   - 该租户 merchants.owner_tg_user_id 为空且 A 是某 chat 的 creator → merchant_owner
//!   - 否则 A 是某 chat 的 administrator/creator → merchant_admin
//!   - 成员身份持久化到 groups.group_members.status
//!
//! 限制：Bot API 只能查询 bot 自己已加入的 chat；getChatMember 在大超群里
//! 对非管理员 bot 可能失败 → 统一用 getChatAdministrators 判定管理员（对任何
//! bot 已加入的 chat 都可用），成员判定失败时记入 skipped 不阻断。

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::time::Duration;
use tgbot_integrations::telegram::TelegramClient;
use tracing::{info, warn};

/// 单个 chat 的识别结果
#[derive(Debug, Clone)]
pub struct ChatIdentity {
    pub chat_id: i64,
    pub chat_title: Option<String>,
    pub chat_type: Option<String>, // group / supergroup / channel
    pub is_member: bool,
    pub is_admin: bool,
    pub is_creator: bool,
    pub member_status: Option<String>,
}

impl ChatIdentity {
    fn to_json(&self) -> Value {
        json!({
            "chat_id": self.chat_id,
            "chat_title": self.chat_title,
            "chat_type": self.chat_type,
            "is_member": self.is_member,
            "is_admin": self.is_admin,
            "is_creator": self.is_creator,
            "member_status": self.member_status,
        })
    }
}

/// 识别报告
#[derive(Debug, Clone)]
pub struct IdentityReport {
    pub user_id: i64,
    pub chats: Vec<ChatIdentity>,
    pub skipped: Vec<Value>, // {chat_id, reason}
}

impl IdentityReport {
    pub fn admin_chats(&self) -> Vec<&ChatIdentity> {
        self.chats.iter().filter(|c| c.is_admin).collect()
    }
    pub fn creator_chats(&self) -> Vec<&ChatIdentity> {
        self.chats.iter().filter(|c| c.is_creator).collect()
    }
    pub fn member_chats(&self) -> Vec<&ChatIdentity> {
        self.chats.iter().filter(|c| c.is_member).collect()
    }
    fn to_json(&self) -> Value {
        json!({
            "user_id": self.user_id,
            "chats": self.chats.iter().map(|c| c.to_json()).collect::<Vec<_>>(),
            "admin_chats": self.admin_chats().iter().map(|c| c.to_json()).collect::<Vec<_>>(),
            "creator_chats": self.creator_chats().iter().map(|c| c.to_json()).collect::<Vec<_>>(),
            "skipped": self.skipped,
        })
    }
}

/// 绑定结果
#[derive(Debug, Clone)]
pub struct BindResult {
    pub user_id: i64,
    pub bound_as: String, // merchant_owner | merchant_admin | none
    pub bound_chats: Vec<Value>,
    pub skipped: Vec<Value>,
}

/// 识别对象 A 在租户 bot 名下各群/频道的身份（只读）
pub async fn identify_identity(
    pool: &PgPool,
    telegram: &TelegramClient,
    tenant_id: &str,
    user_id: i64,
    limit: Option<i64>,
) -> Result<IdentityReport> {
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let rows = sqlx::query(
        "SELECT group_id, group_title \
         FROM groups.tracked_groups \
         WHERE tenant_id = $1 \
         ORDER BY group_id \
         LIMIT $2",
    )
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("failed to list tracked groups")?;

    let mut chats = Vec::with_capacity(rows.len());
    let mut skipped: Vec<Value> = Vec::new();
    for row in &rows {
        let chat_id: i64 = row
            .try_get("group_id")
            .context("tracked_groups.group_id missing")?;
        let chat_title: Option<String> = row.try_get("group_title").unwrap_or(None);
        match identify_chat(telegram, chat_id, user_id).await {
            Ok(identity) => chats.push(identity),
            Err(err) => {
                warn!(chat_id, err = %err, "identity identify chat skipped");
                skipped.push(json!({
                    "chat_id": chat_id,
                    "chat_title": chat_title,
                    "reason": err.to_string(),
                }));
            }
        }
    }
    info!(
        user_id,
        tenant_id,
        total = rows.len(),
        matched = chats.len(),
        "identity identify completed"
    );

    Ok(IdentityReport {
        user_id,
        chats,
        skipped,
    })
}

/// 识别单个 chat 中 A 的身份；429 限流时按 retry_after 重试一次
async fn identify_chat(
    telegram: &TelegramClient,
    chat_id: i64,
    user_id: i64,
) -> Result<ChatIdentity> {
    let mut retried = false;
    loop {
        match identify_chat_once(telegram, chat_id, user_id).await {
            Ok(identity) => return Ok(identity),
            Err(err) => {
                let msg = err.to_string();
                if !retried && msg.contains("429") {
                    retried = true;
                    info!(chat_id, "telegram 429, retrying after 2s");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                return Err(err);
            }
        }
    }
}

async fn identify_chat_once(
    telegram: &TelegramClient,
    chat_id: i64,
    user_id: i64,
) -> Result<ChatIdentity> {
    let mut identity = ChatIdentity {
        chat_id,
        chat_title: None,
        chat_type: None,
        is_member: false,
        is_admin: false,
        is_creator: false,
        member_status: None,
    };

    // 1) 管理员列表 —— 对 bot 已加入的任何 chat 都可用，作为管理员判定依据
    let admins = telegram.get_chat_administrators(chat_id).await?;
    let mut admin_ids: Vec<i64> = Vec::new();
    for admin in &admins {
        let id = admin
            .get("user")
            .and_then(|u| u.get("id"))
            .and_then(Value::as_i64);
        if let Some(id) = id {
            admin_ids.push(id);
            if id == user_id {
                let status = admin.get("status").and_then(Value::as_str).unwrap_or("");
                identity.is_admin = true;
                identity.is_creator = status == "creator";
                identity.member_status = Some(status.to_string());
                identity.is_member = true;
            }
        }
    }
    let _ = admin_ids;

    // 2) getChat 补全 chat 类型/标题
    if let Ok(chat) = telegram.get_chat(chat_id).await {
        identity.chat_type = chat.get("type").and_then(Value::as_str).map(str::to_string);
        identity.chat_title = chat.get("title").and_then(Value::as_str).map(str::to_string);
    }

    // 3) getChatMember 判定普通成员身份（大超群非管理员 bot 可能失败 → 容错）
    if !identity.is_admin {
        match telegram.get_chat_member(chat_id, user_id).await {
            Ok(member) => {
                let status = member.get("status").and_then(Value::as_str).unwrap_or("");
                identity.member_status = Some(status.to_string());
                identity.is_member = matches!(status, "creator" | "administrator" | "member" | "restricted");
                identity.is_admin = matches!(status, "creator" | "administrator");
                identity.is_creator = status == "creator";
            }
            Err(err) => {
                // 非成员/无法查询 → 记录但不阻断；is_member 保持 false
                warn!(chat_id, user_id, err = %err, "getChatMember failed (assumed not member)");
            }
        }
    }

    Ok(identity)
}

/// 智能绑定：把 A 在租户内"是管理员"的群/频道绑定给 A
///
/// 写库（同一事务）：
///   1. auth.users 按 tg_user_id upsert → 取 uuid
///   2. merchants.owner_tg_user_id 为空 且 A 是某 chat creator → A 升级为 merchant_owner
///   3. 否则 A 是某 chat administrator/creator → tenant_memberships role=merchant_admin
///   4. groups.group_members 持久化成员身份（管理员写 admin，普通成员 active）
pub async fn bind_identity(
    pool: &PgPool,
    tenant_id: &str,
    user_id: i64,
    report: &IdentityReport,
) -> Result<BindResult> {
    // 1) 查询/创建 auth.users
    let user_uuid = upsert_auth_user(pool, user_id).await?;

    // 2) 取商户信息
    let merchant = sqlx::query_as::<_, (uuid::Uuid, Option<i64>, Option<String>)>(
        "SELECT id, owner_tg_user_id, owner_username FROM merchants WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .context("failed to load merchant")?
    .ok_or_else(|| anyhow!("tenant {tenant_id} has no merchant record"))?;
    let (merchant_id, current_owner_tg, _current_owner_name) = merchant;

    let admin_chats = report.admin_chats();
    let creator_chats = report.creator_chats();
    let member_chats = report.member_chats();

    let mut bound_chats: Vec<Value> = Vec::new();
    for chat in admin_chats.iter().chain(member_chats.iter()) {
        let seen = bound_chats
            .iter()
            .any(|v| v.get("chat_id").and_then(Value::as_i64) == Some(chat.chat_id));
        if seen {
            continue;
        }
        bound_chats.push(json!({
            "chat_id": chat.chat_id,
            "chat_title": chat.chat_title,
            "chat_type": chat.chat_type,
            "role": if chat.is_admin { "admin" } else { "member" },
        }));
    }

    // 3) 决定绑定角色
    let can_own = current_owner_tg.is_none() && !creator_chats.is_empty();
    let bound_as = if can_own {
        "merchant_owner"
    } else if !admin_chats.is_empty() {
        "merchant_admin"
    } else {
        "none"
    };

    let mut tx = pool.begin().await.context("failed to begin tx")?;

    // 4) 商家 owner 升级
    if can_own {
        let owner_username: Option<String> = sqlx::query_scalar(
            "SELECT username FROM chat.users WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .ok()
        .flatten();
        sqlx::query(
            "UPDATE merchants \
             SET owner_tg_user_id = $1, owner_username = $2, owner_user_id = $3, updated_at = now() \
             WHERE tenant_id = $4 AND owner_tg_user_id IS NULL",
        )
        .bind(user_id)
        .bind(owner_username)
        .bind(user_uuid)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await
        .context("failed to promote merchant owner")?;
    }

    // 5) 租户成员关系
    if bound_as != "none" {
        sqlx::query(
            "INSERT INTO tenant_memberships (user_id, tenant_id, merchant_id, role) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (user_id, tenant_id) DO UPDATE SET \
               role = CASE WHEN tenant_memberships.role = 'merchant_owner' THEN 'merchant_owner' ELSE EXCLUDED.role END, \
               merchant_id = EXCLUDED.merchant_id",
        )
        .bind(user_uuid)
        .bind(tenant_id)
        .bind(merchant_id)
        .bind(bound_as)
        .execute(&mut *tx)
        .await
        .context("failed to upsert tenant membership")?;
    }

    // 6) 群成员身份持久化（管理员不降级）
    for chat in &member_chats {
        let status = if chat.is_admin { "admin" } else { "active" };
        let username: Option<String> = sqlx::query_scalar("SELECT username FROM chat.users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await
            .ok()
            .flatten();
        sqlx::query(
            "INSERT INTO groups.group_members \
               (user_id, username, first_name, last_name, group_id, group_title, tenant_id, status) \
             VALUES ($1, $2, NULL, NULL, $3, $4, $5, $6) \
             ON CONFLICT (group_id, user_id) DO UPDATE SET \
               status = CASE WHEN groups.group_members.status = 'admin' THEN 'admin' ELSE EXCLUDED.status END, \
               group_title = EXCLUDED.group_title, \
               username = COALESCE(EXCLUDED.username, groups.group_members.username), \
               updated_at = now()",
        )
        .bind(user_id)
        .bind(username)
        .bind(chat.chat_id)
        .bind(chat.chat_title.as_deref())
        .bind(tenant_id)
        .bind(status)
        .execute(&mut *tx)
        .await
        .context("failed to upsert group member")?;
    }

    tx.commit().await.context("failed to commit bind tx")?;

    info!(
        user_id,
        tenant_id, bound_as, chats = bound_chats.len(),
        "identity bind completed"
    );

    Ok(BindResult {
        user_id,
        bound_as: bound_as.to_string(),
        bound_chats,
        skipped: report.skipped.clone(),
    })
}

/// 按 tg_user_id upsert auth.users，返回 uuid
async fn upsert_auth_user(pool: &PgPool, user_id: i64) -> Result<uuid::Uuid> {
    let row = sqlx::query(
        "INSERT INTO auth.users (tg_user_id, username, first_name) \
         VALUES ($1, \
                 (SELECT username FROM chat.users WHERE user_id = $1), \
                 (SELECT first_name FROM chat.users WHERE user_id = $1)) \
         ON CONFLICT (tg_user_id) DO UPDATE SET \
           username = COALESCE(EXCLUDED.username, auth.users.username), \
           first_name = COALESCE(EXCLUDED.first_name, auth.users.first_name), \
           updated_at = now() \
         RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("failed to upsert auth.users")?;
    Ok(row.try_get("id")?)
}

/// 供路由层使用的 JSON 输出
pub fn identify_report_json(report: &IdentityReport) -> Value {
    report.to_json()
}

pub fn bind_result_json(result: &BindResult) -> Value {
    json!({
        "user_id": result.user_id,
        "bound_as": result.bound_as,
        "bound_chats": result.bound_chats,
        "skipped": result.skipped,
    })
}
