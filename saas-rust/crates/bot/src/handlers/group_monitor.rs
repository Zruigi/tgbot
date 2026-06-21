//! Group Monitor Handler
//! Handles group message tracking, keyword monitoring, address detection, and channel verification.
//! Migrated from handlers/group_monitor.py

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{ChatPermissions, ChatType, Message};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::telegram_client::TelegramClient;

// ---------------------------------------------------------------------------
// Configuration Constants
// ---------------------------------------------------------------------------

lazy_static! {
    /// Default profile tags mapping keywords to user profile tags
    pub static ref KEYWORD_PROFILE_TAGS: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        m.insert("加入|申请|想进", "拉新意向");
        m.insert("邀请|拉我", "拉新意向");
        m.insert("合作|合伙|商务", "合作意向");
        m.insert("推荐|分享", "推荐");
        m.insert("招聘|找人|兼职", "招聘");
        m.insert("项目|开发|进度", "项目相关");
        m.insert("融资|资金|天使轮", "投融资");
        m.insert("投资|理财|赚钱", "投融资");
        m
    };

    /// Compiled regex patterns for profile tags
    pub static ref COMPILED_PROFILE_PATTERNS: Vec<(Regex, String)> = {
        KEYWORD_PROFILE_TAGS
            .iter()
            .map(|(k, v)| (Regex::new(k).unwrap(), v.to_string()))
            .collect()
    };

    /// Default keywords extracted from profile tags
    pub static ref DEFAULT_KEYWORDS: Vec<String> = {
        let mut keywords = Vec::new();
        for k in KEYWORD_PROFILE_TAGS.keys() {
            for part in k.split('|') {
                keywords.push(part.to_string());
            }
        }
        keywords
    };

    /// Address regex pattern for detecting crypto addresses
    pub static ref ADDR_REGEX: Regex = Regex::new(
        r"(?:[13][a-km-zA-HJ-NP-Z1-9]{25,34}|0x[a-fA-F0-9]{40}|T[A-Za-z0-9]{33}|4[0-9a-zA-Z]{94})"
    ).unwrap();
}

/// Group configuration cache entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    pub is_monitored: bool,
    pub custom_keywords: Vec<String>,
    pub verify_enabled: bool,
    pub verify_type: Option<String>,
    pub verify_channel_id: Option<i64>,
    pub verify_msg_template: Option<String>,
}

impl Default for GroupConfig {
    fn default() -> Self {
        Self {
            is_monitored: false,
            custom_keywords: Vec::new(),
            verify_enabled: false,
            verify_type: None,
            verify_channel_id: None,
            verify_msg_template: None,
        }
    }
}

/// Group address hit record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressSnapshot {
    pub address: String,
    pub network: String,
    pub tx_count_24h: i32,
    pub inflow_24h: f64,
    pub outflow_24h: f64,
    pub latest_tx: Option<String>,
    pub account_type: Option<String>,
    pub created_at: Option<String>,
    pub last_active: Option<String>,
    pub trx_balance: Option<f64>,
    pub usdt_balance: Option<f64>,
    pub energy_used: Option<i64>,
    pub energy_limit: Option<i64>,
    pub bandwidth_used: Option<i64>,
    pub bandwidth_limit: Option<i64>,
    pub free_bandwidth_used: Option<i64>,
    pub free_bandwidth_limit: Option<i64>,
    pub total_tx_in: Option<i64>,
    pub total_tx_out: Option<i64>,
    pub recent_txs: Vec<TransactionRecord>,
    pub permissions: Vec<PermissionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub time: String,
    pub direction: String,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRecord {
    #[serde(rename = "type")]
    pub perm_type: String,
    pub address: String,
    pub weight: i32,
}

// ---------------------------------------------------------------------------
// Cache Structures
// ---------------------------------------------------------------------------

/// Thread-safe group config cache
pub type GroupConfigCache = Arc<RwLock<HashMap<i64, GroupConfig>>>;

/// Thread-safe avatar cache
pub type AvatarCache = Arc<RwLock<HashMap<i64, String>>>;

/// Thread-safe verification cooldown cache
pub type VerifyCooldownCache = Arc<RwLock<HashMap<(i64, i64), f64>>>;

/// Address query cooldown to prevent duplicate queries within 30 minutes
pub type AddressQueryCooldown = Arc<RwLock<HashMap<(i64, String), f64>>>;

impl GroupConfigCache {
    pub fn new() -> Self {
        Arc::new(RwLock::new(HashMap::new()))
    }

    pub async fn get(&self, group_id: i64) -> Option<GroupConfig> {
        let cache = self.read().await;
        cache.get(&group_id).cloned()
    }

    pub async fn set(&self, group_id: i64, config: GroupConfig) {
        let mut cache = self.write().await;
        // Keep cache size bounded
        if cache.len() > 500 {
            // Remove oldest entries (simple strategy: clear half)
            let keys: Vec<_> = cache.keys().take(250).cloned().collect();
            for key in keys {
                cache.remove(&key);
            }
        }
        cache.insert(group_id, config);
    }

    pub async fn clear(&self, group_id: Option<i64>) {
        let mut cache = self.write().await;
        if let Some(gid) = group_id {
            cache.remove(&gid);
        } else {
            cache.clear();
        }
    }
}

impl AvatarCache {
    pub fn new() -> Self {
        Arc::new(RwLock::new(HashMap::new()))
    }

    pub async fn get(&self, user_id: i64) -> Option<String> {
        let cache = self.read().await;
        cache.get(&user_id).cloned()
    }

    pub async fn set(&self, user_id: i64, avatar: String) {
        let mut cache = self.write().await;
        if cache.len() > 2000 {
            // Clear oldest half
            let keys: Vec<_> = cache.keys().take(1000).cloned().collect();
            for key in keys {
                cache.remove(&key);
            }
        }
        cache.insert(user_id, avatar);
    }
}

// ---------------------------------------------------------------------------
// Handler State
// ---------------------------------------------------------------------------

pub struct GroupMonitorState {
    pub group_config_cache: GroupConfigCache,
    pub avatar_cache: AvatarCache,
    pub verify_cooldown: VerifyCooldownCache,
    pub address_query_cooldown: AddressQueryCooldown,
    pub telegram_client: TelegramClient,
}

impl GroupMonitorState {
    pub fn new(telegram_client: TelegramClient) -> Self {
        Self {
            group_config_cache: GroupConfigCache::new(),
            avatar_cache: AvatarCache::new(),
            verify_cooldown: VerifyCooldownCache::new(RwLock::new(HashMap::new())),
            address_query_cooldown: AddressQueryCooldown::new(RwLock::new(HashMap::new())),
            telegram_client,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper Functions
// ---------------------------------------------------------------------------

/// Guess blockchain network by address prefix
pub fn guess_network_by_address(address: &str) -> Option<&'static str> {
    if address.starts_with('T') && address.len() == 34 {
        Some("trc20")
    } else if address.starts_with("0x") && address.len() == 42 {
        Some("bsc")
    } else {
        None
    }
}

/// Extract profile tags from matched keywords and text content
pub fn profile_tags_from_keywords(matched_keywords: &[String], text: &str) -> String {
    let mut tags = std::collections::HashSet::new();

    // 1. Simple keyword mapping
    for kw in matched_keywords {
        for (pattern_str, tag) in KEYWORD_PROFILE_TAGS.iter() {
            if pattern_str.split('|').any(|p| p.eq_ignore_ascii_case(kw)) {
                tags.insert(tag.to_string());
            }
        }
    }

    // 2. Enhanced regex matching
    for (pattern, tag) in COMPILED_PROFILE_PATTERNS.iter() {
        if pattern.is_match(text) {
            tags.insert(tag.clone());
        }
    }

    let mut tags_vec: Vec<String> = tags.into_iter().collect();
    tags_vec.sort();
    tags_vec.join(",")
}

/// Detect crypto addresses in text
pub fn detect_addresses(text: &str) -> Vec<String> {
    ADDR_REGEX
        .find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Message Handlers
// ---------------------------------------------------------------------------

/// Track group message - main entry point for all group messages
pub async fn track_group_message(
    bot: &Bot,
    state: &GroupMonitorState,
    message: &Message,
) -> Option<()> {
    let chat = message.chat.clone();

    // Only process group and supergroup messages
    if !matches!(chat.type, ChatType::Group | ChatType::Supergroup) {
        return None;
    }

    // Get user info
    let user = message.from.as_ref()?;

    info!(
        "[GroupMonitor] Message from {} in {} ({})",
        user.id, chat.title.as_deref().unwrap_or("Unknown"), chat.id
    );

    // Track group seen (update group info in database)
    track_group_seen(bot, state, message).await;

    // Check channel verification if enabled
    check_channel_verify(bot, state, message).await;

    // Handle address auto-detection
    handle_group_address_auto_query(bot, state, message).await;

    Some(())
}

/// Track group seen - update group info in database
async fn track_group_seen(bot: &Bot, state: &GroupMonitorState, message: &Message) {
    let chat = message.chat.clone();

    // In Rust, we would call repository functions here
    // For now, this is a placeholder that logs the activity
    debug!(
        "[GroupMonitor] Tracking group: {} ({})",
        chat.title.as_deref().unwrap_or("Unknown"),
        chat.id
    );

    // Check if group is monitored
    if let Some(config) = state.group_config_cache.get(chat.id).await {
        if config.is_monitored {
            // Record message to group_messages table
            // In production, this would call member_repo::save_group_message
            debug!("[GroupMonitor] Group {} is monitored, recording message", chat.id);
        }
    }
}

/// Channel verification checker
async fn check_channel_verify(bot: &Bot, state: &GroupMonitorState, message: &Message) {
    let chat = message.chat.clone();
    let user = message.from.as_ref();

    // Skip if no user or if it's a sender_chat (channelanonymous)
    if user.is_none() || message.sender_chat.is_some() {
        return;
    }

    let user = user.unwrap();

    // Get group config
    let config = match state.group_config_cache.get(chat.id).await {
        Some(c) => c,
        None => return,
    };

    // Only process if verify is enabled and type is channel
    if !config.verify_enabled || config.verify_type.as_deref() != Some("channel") {
        return;
    }

    let channel_id = match config.verify_channel_id {
        Some(id) => id,
        None => return,
    };

    let user_id = user.id;

    // Skip Telegram system accounts
    if user_id.0 == 777000 {
        return;
    }

    // Check if user is admin/creator
    if let Ok(member) = bot.get_chat_member(chat.id, user_id).await {
        if matches!(
            member.status(),
            teloxide::types::ChatMemberStatus::Administrator
                | teloxide::types::ChatMemberStatus::Creator
        ) {
            return;
        }
    }

    // Check cooldown
    {
        let cooldown = state.verify_cooldown.read().await;
        let now = chrono::Utc::now().timestamp() as f64;
        if let Some(last_check) = cooldown.get(&(chat.id.0, user_id.0)) {
            if now - last_check < 30.0 {
                // Within 30 seconds cooldown
                return;
            }
        }
    }

    // Update cooldown
    {
        let mut cooldown = state.verify_cooldown.write().await;
        cooldown.insert((chat.id.0, user_id.0), chrono::Utc::now().timestamp() as f64);
    }

    // Check if user is member of the channel
    let is_member = check_channel_membership(bot, user_id, channel_id).await;

    if !is_member {
        // Delete user's message
        let _ = bot.delete_message(chat.id, message.id).await;

        // Send verification prompt
        let user_name = user
            .first_name
            .as_deref()
            .or(user.username.as_deref())
            .unwrap_or(&format!("User{}", user_id.0));

        let template = config
            .verify_msg_template
            .as_deref()
            .unwrap_or(&format!(
                "🚫 {}, please follow channel {} before speaking!",
                user_name, channel_id
            ));

        let text = template
            .replace("{user}", &format!(r#"<a href="tg://user?id={}">{}</a>"#, user_id.0, user_name))
            .replace("{channel}", &channel_id.to_string());

        if let Ok(msg) = bot.send_message(chat.id, text).parse_mode(teloxide::types::ParseMode::Html).await {
            // Auto-delete after 5 seconds
            let bot_clone = bot.clone();
            let chat_id = chat.id;
            let msg_id = msg.id;
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                let _ = bot_clone.delete_message(chat_id, msg_id).await;
            });
        }
    }
}

/// Check if user is member of a channel
async fn check_channel_membership(bot: &Bot, user_id: UserId, channel_id: i64) -> bool {
    match bot.get_chat_member(ChatId(channel_id), user_id).await {
        Ok(member) => matches!(
            member.status(),
            teloxide::types::ChatMemberStatus::Member
                | teloxide::types::ChatMemberStatus::Administrator
                | teloxide::types::ChatMemberStatus::Creator
        ),
        Err(e) => {
            warn!("[ChannelVerify] Failed to check membership: {}", e);
            false
        }
    }
}

/// Handle address auto-detection and query
async fn handle_group_address_auto_query(
    bot: &Bot,
    state: &GroupMonitorState,
    message: &Message,
) {
    let text = message.text().or(message.caption()).unwrap_or("");

    let addresses = detect_addresses(text);
    if addresses.is_empty() {
        return;
    }

    let group_id = message.chat.id.0;
    let user_id = message.from.as_ref().map(|u| u.id.0).unwrap_or(0);

    // Process up to 3 addresses
    for addr in addresses.iter().take(3) {
        let network = match guess_network_by_address(addr) {
            Some(n) => n,
            None => continue,
        };

        // Check cooldown (30 minutes)
        {
            let cooldown = state.address_query_cooldown.read().await;
            let now = chrono::Utc::now().timestamp() as f64;
            if let Some(last_query) = cooldown.get(&(group_id, addr.clone())) {
                if now - last_query < 1800.0 {
                    // 30 minutes
                    continue;
                }
            }
        }

        // Update cooldown
        {
            let mut cooldown = state.address_query_cooldown.write().await;
            cooldown.insert((group_id, addr.clone()), chrono::Utc::now().timestamp() as f64);
        }

        // Query address snapshot
        match query_address_snapshot(addr, network).await {
            Ok(snapshot) => {
                // Send address info to group
                let tip_lines = build_address_tip_lines(addr, &snapshot);
                let tip = tip_lines.join("\n");

                if let Err(e) = bot.send_message(message.chat.id, tip).parse_mode(teloxide::types::ParseMode::Html).await {
                    warn!("[GroupMonitor] Failed to send address tip: {}", e);
                }
            }
            Err(e) => {
                warn!("[GroupMonitor] Address query failed for {}: {}", addr, e);
            }
        }
    }
}

/// Build address tip message lines
fn build_address_tip_lines(address: &str, snapshot: &AddressSnapshot) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push(format!(
        "👤账户类型: {}",
        snapshot.account_type.as_deref().unwrap_or("未知")
    ));
    lines.push(format!("🔍查询地址: <code>{}</code>", address));

    if let Some(ref created) = snapshot.created_at {
        lines.push(format!("⏰创建时间: {}", created));
    }
    if let Some(ref last_active) = snapshot.last_active {
        lines.push(format!("🌟最后活跃: {}", last_active));
    }

    lines.push("➖➖➖➖资源➖➖➖➖".to_string());

    if let Some(trx) = snapshot.trx_balance {
        lines.push(format!("💰 TRX  余额：  {} TRX", trx));
    }
    lines.push(format!(
        "💰USDT余额： {} USDT",
        snapshot.usdt_balance.map(|v| v.to_string()).unwrap_or_else(|| "0.00".to_string())
    ));

    if let (Some(eu), Some(el)) = (snapshot.energy_used, snapshot.energy_limit) {
        lines.push(format!("🔋能量： {} / {}", eu, el));
    }
    if let (Some(bu), Some(bl)) = (snapshot.bandwidth_used, snapshot.bandwidth_limit) {
        lines.push(format!("📡质押带宽： {} / {}", bu, bl));
    }
    if let (Some(fbu), Some(fbl)) = (snapshot.free_bandwidth_used, snapshot.free_bandwidth_limit) {
        lines.push(format!("📡免费带宽： {} / {}", fbu, fbl));
    }

    if !snapshot.permissions.is_empty() {
        lines.push("➖➖➖➖权限➖➖➖➖".to_string());
        let owner_perms: Vec<_> = snapshot
            .permissions
            .iter()
            .filter(|p| p.perm_type == "owner")
            .collect();
        let active_perms: Vec<_> = snapshot
            .permissions
            .iter()
            .filter(|p| p.perm_type == "active")
            .collect();

        if !owner_perms.is_empty() {
            lines.push("拥有者 (Owner) 权限".to_string());
            for p in owner_perms {
                lines.push(format!("<code>{}</code>(权重: {})", p.address, p.weight));
            }
        }
        if !active_perms.is_empty() {
            lines.push("活跃 (Active) 权限".to_string());
            for p in active_perms {
                lines.push(format!("<code>{}</code>(权重: {})", p.address, p.weight));
            }
        }
    }

    lines.push("➖➖➖➖最近交易➖➖➖➖".to_string());

    if let (Some(ti), Some(to)) = (snapshot.total_tx_in, snapshot.total_tx_out) {
        lines.push(format!("⤴️USDT支出笔数：{}    ⤵️USDT收入笔数：{}", to, ti));
    } else {
        lines.push(format!("24h 笔数: <b>{}</b>", snapshot.tx_count_24h));
        lines.push(format!(
            "24h 转入: <b>{:.2} USDT</b>",
            snapshot.inflow_24h
        ));
        lines.push(format!(
            "24h 转出: <b>{:.2} USDT</b>",
            snapshot.outflow_24h
        ));
    }

    for tx in snapshot.recent_txs.iter().take(5) {
        lines.push(format!("{} {} {:.2} USDT", tx.time, tx.direction, tx.amount));
    }

    lines
}

/// Query address snapshot from blockchain APIs
pub async fn query_address_snapshot(address: &str, network: &str) -> Result<AddressSnapshot, Box<dyn std::error::Error + Send + Sync>> {
    let mut result = AddressSnapshot {
        address: address.to_string(),
        network: network.to_uppercase(),
        tx_count_24h: 0,
        inflow_24h: 0.0,
        outflow_24h: 0.0,
        latest_tx: None,
        account_type: None,
        created_at: None,
        last_active: None,
        trx_balance: None,
        usdt_balance: None,
        energy_used: None,
        energy_limit: None,
        bandwidth_used: None,
        bandwidth_limit: None,
        free_bandwidth_used: None,
        free_bandwidth_limit: None,
        total_tx_in: None,
        total_tx_out: None,
        recent_txs: Vec::new(),
        permissions: Vec::new(),
    };

    // Network-specific querying would be implemented here
    // For now, return a placeholder

    Ok(result)
}

/// Monitor group keywords - detect potential users based on keywords
pub async fn monitor_group_keywords(
    bot: &Bot,
    state: &GroupMonitorState,
    message: &Message,
) -> Option<()> {
    let text = message.text()?;
    let user = message.from.as_ref()?;
    let chat = &message.chat;

    // Skip admin messages
    // In production, check against config.ADMIN_ID

    // Get group config
    let config = state.group_config_cache.get(chat.id).await?;

    if !config.is_monitored {
        return None;
    }

    // Build keyword pool
    let mut keywords_pool = DEFAULT_KEYWORDS.clone();
    keywords_pool.extend(config.custom_keywords.clone());

    // Match keywords
    let text_lower = text.to_lowercase();
    let matched_keywords: Vec<String> = keywords_pool
        .iter()
        .filter(|kw| text_lower.contains(&kw.to_lowercase()))
        .cloned()
        .collect();

    // Get profile tags
    let profile_tags = profile_tags_from_keywords(&matched_keywords, text);

    if profile_tags.is_empty() && matched_keywords.is_empty() {
        return None;
    }

    // Save to potential user database
    info!(
        "[GroupMonitor] Potential user: {} (@{}) - Keywords: {:?}, Tags: {}",
        user.id.0,
        user.username.as_deref().unwrap_or("None"),
        matched_keywords,
        profile_tags
    );

    // Send auto-invite to user
    let invite_text = format!(
        "👋 <b>你好 {}！</b>\n\n我们在群组 <b>{}</b> 注意到你提到了关于 <code>{}</code> 的内容。\n\n欢迎直接咨询我们的官方客服，我们将为您提供 1对1 的专业解答和支持！",
        user.first_name.as_deref().unwrap_or("朋友"),
        chat.title.as_deref().unwrap_or("Unknown"),
        matched_keywords.first().unwrap_or(&"相关内容".to_string())
    );

    // Try to send invite (may fail if user hasn't started the bot)
    let _ = bot.send_message(user.id, &invite_text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .await;

    Some(())
}

/// Handle new group member
pub async fn handle_new_group_member(
    bot: &Bot,
    state: &GroupMonitorState,
    message: &Message,
) -> Option<()> {
    let new_members = message.new_chat_members()?;
    let chat = &message.chat;

    for member in new_members {
        if member.is_bot {
            continue;
        }

        info!(
            "[GroupMonitor] New member: {} (@{}) joined {}",
            member.id.0,
            member.username.as_deref().unwrap_or("None"),
            chat.title.as_deref().unwrap_or("Unknown")
        );

        // Save member to database
        // In production: call member_repo::upsert_member
    }

    Some(())
}

/// Handle member left group
pub async fn handle_group_left_member(
    bot: &Bot,
    state: &GroupMonitorState,
    message: &Message,
) -> Option<()> {
    let left_member = message.left_chat_member()?;
    let chat = &message.chat;

    if left_member.is_bot {
        return None;
    }

    info!(
        "[GroupMonitor] Member left: {} (@{}) left {}",
        left_member.id.0,
        left_member.username.as_deref().unwrap_or("None"),
        chat.title.as_deref().unwrap_or("Unknown")
    );

    // Update member status in database
    // In production: call member_repo::update_member_status

    Some(())
}

/// Handle channel seen - track channels for broadcast
pub async fn track_channel_seen(
    bot: &Bot,
    state: &GroupMonitorState,
    message: &Message,
) -> Option<()> {
    let chat = message.chat.clone();

    if !matches!(chat.type, ChatType::Channel) {
        return None;
    }

    info!(
        "[GroupMonitor] Channel seen: {} ({})",
        chat.title.as_deref().unwrap_or("Unknown"),
        chat.id
    );

    // Save channel as broadcast target
    // In production: call broadcast_repo::upsert_broadcast_target

    Some(())
}

// ---------------------------------------------------------------------------
// Mute/Unmute/Kick Utilities (used by join_verify)
// ---------------------------------------------------------------------------

/// Mute a user in a group
pub async fn mute_user(bot: &Bot, chat_id: i64, user_id: i64) -> Result<(), teloxide::RequestError> {
    info!("[Verify] Attempting to mute user {} in chat {}", user_id, chat_id);
    bot.restrict_chat_member(
        chat_id,
        user_id,
        ChatPermissions::default()
            .can_send_messages(false)
            .can_send_polls(false)
            .can_send_other_messages(false)
            .can_add_web_page_previews(false),
    )
    .await?;
    info!("[Verify] Muted user {} in chat {}", user_id, chat_id);
    Ok(())
}

/// Unmute a user in a group
pub async fn unmute_user(bot: &Bot, chat_id: i64, user_id: i64) -> Result<(), teloxide::RequestError> {
    bot.restrict_chat_member(
        chat_id,
        user_id,
        ChatPermissions::default()
            .can_send_messages(true)
            .can_send_polls(true)
            .can_send_other_messages(true)
            .can_add_web_page_previews(true)
            .can_change_info(false)
            .can_invite_users(true)
            .can_pin_messages(false),
    )
    .await?;
    Ok(())
}

/// Kick a user from a group (ban + unban to allow rejoin)
pub async fn kick_user(bot: &Bot, chat_id: i64, user_id: i64) -> Result<(), teloxide::RequestError> {
    info!("[Verify] Kicking user {} from chat {}", user_id, chat_id);
    bot.ban_chat_member(chat_id, user_id).await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    bot.unban_chat_member(chat_id, user_id).await?;
    info!("[Verify] Kicked user {} from chat {}", user_id, chat_id);
    Ok(())
}