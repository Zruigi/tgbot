//! Wallet Monitor handler – migrated from handlers/wallet_monitor.py
//!
//! Handles USDT wallet monitoring for TRC20 and BSC networks.
//! Users input wallet addresses, the bot queries on-chain data and
//! registers the address for 24/7 real-time monitoring.

use std::fmt::Write;

use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};

use crate::handlers::HandlerResult;

// ---------------------------------------------------------------------------
// Conversation state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletMonitorState {
    WaitingAddress,
}

// ---------------------------------------------------------------------------
// Network detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Trc20,
    Bsc,
}

impl Network {
    fn from_address(addr: &str) -> Option<Self> {
        if addr.starts_with('T') {
            Some(Network::Trc20)
        } else if addr.starts_with("0x") {
            Some(Network::Bsc)
        } else {
            None
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Network::Trc20 => "TRC20",
            Network::Bsc => "BSC",
        }
    }
}

// ---------------------------------------------------------------------------
// Transaction record from chain API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TxRecord {
    pub direction: TxDirection,
    pub value: f64,
    pub timestamp_str: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxDirection {
    Incoming,
    Outgoing,
}

impl std::fmt::Display for TxDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxDirection::Incoming => write!(f, "📥 转入"),
            TxDirection::Outgoing => write!(f, "📤 转出"),
        }
    }
}

// ---------------------------------------------------------------------------
// Entry: callback-based
// ---------------------------------------------------------------------------

/// Start wallet monitor from callback query (button press).
pub async fn start_wallet_monitor(
    bot: &Bot,
    chat_id: ChatId,
    message_id: i32,
) -> HandlerResult<WalletMonitorState> {
    let text = "👛 <b>USDT 钱包监听</b>\n\n\
                请输入您要查询和监听的 USDT 地址：\n\
                支持 <code>TRC20</code> (T开头) 或 <code>BSC</code> (0x开头)";

    bot.edit_message_text(chat_id, MessageId(message_id), text)
        .parse_mode(ParseMode::Html)
        .await?;

    Ok(WalletMonitorState::WaitingAddress)
}

/// Start wallet monitor from a text message (direct command).
pub async fn start_wallet_monitor_from_text(
    bot: &Bot,
    chat_id: ChatId,
) -> HandlerResult<WalletMonitorState> {
    let text = "👛 <b>USDT 钱包监听</b>\n\n\
                请输入您要查询和监听的 USDT 地址：\n\
                支持 <code>TRC20</code> (T开头) 或 <code>BSC</code> (0x开头)";

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .await?;

    Ok(WalletMonitorState::WaitingAddress)
}

// ---------------------------------------------------------------------------
// Handle address input
// ---------------------------------------------------------------------------

/// Process user-submitted wallet address, query chain data, and register monitoring.
pub async fn handle_address_input(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    address: &str,
    db: &sqlx::PgPool,
    trongrid_api_key: &str,
    bscscan_api_key: &str,
) -> HandlerResult<()> {
    let network = match Network::from_address(address) {
        Some(n) => n,
        None => {
            bot.send_message(
                chat_id,
                "❌ 无效的地址格式。请发送正确的 TRC20 或 BSC 地址。",
            )
            .await?;
            return Ok(());
        }
    };

    let processing_msg = bot
        .send_message(chat_id, "🔍 正在查询链上数据，请稍候...")
        .await?;

    match query_chain_data(address, network, trongrid_api_key, bscscan_api_key).await {
        Ok((txs, latest_ts)) => {
            let mut tx_text = String::new();
            if txs.is_empty() {
                tx_text.push_str("暂无近期 USDT 转账记录");
            } else {
                for (i, tx) in txs.iter().enumerate() {
                    write!(
                        tx_text,
                        "{}. {} {:.2} USDT ({})\n",
                        i + 1,
                        tx.direction,
                        tx.value,
                        tx.timestamp_str
                    )
                    .unwrap();
                }
            }

            let msg = format!(
                "📊 <b>地址近期 USDT 账单</b>\n\
                 网络: <code>{net}</code>\n\
                 地址: <code>{address}</code>\n\n\
                 {tx_text}\n\n\
                 ✅ 该地址已自动加入 24/7 实时监听队列！有新账单将自动推送。",
                net = network.as_str(),
            );

            let scan_url = match network {
                Network::Trc20 => format!("https://tronscan.org/#/address/{address}"),
                Network::Bsc => format!("https://bscscan.com/address/{address}"),
            };
            let scan_label = match network {
                Network::Trc20 => "TronScan",
                Network::Bsc => "BscScan",
            };

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::url(
                    format!("🔗 前往 {scan_label} 查看完整账单"),
                    scan_url
                        .parse()
                        .unwrap_or_else(|_| "https://example.com".parse().unwrap()),
                )],
                vec![InlineKeyboardButton::callback(
                    "🔙 返回主菜单",
                    "back_to_start",
                )],
            ]);

            let _ = bot.delete_message(chat_id, processing_msg.id).await;
            bot.send_message(chat_id, &msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;

            // Save to DB with initial timestamp to avoid pushing historical txs
            save_wallet_monitor(db, user_id, address, network.as_str(), latest_ts).await;
        }
        Err(e) => {
            tracing::error!("Wallet query failed for {address}: {e}");
            bot.edit_message_text(
                chat_id,
                MessageId(processing_msg.id.0 as i32),
                "❌ 查询失败：无法连接至链上节点。地址已尝试保存至监听列表。",
            )
            .await?;

            // Still save the monitor entry (without timestamp)
            save_wallet_monitor(db, user_id, address, network.as_str(), None).await;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Chain data queries
// ---------------------------------------------------------------------------

/// Query on-chain USDT transactions for the given address.
/// Returns (recent transactions, latest timestamp in ms).
async fn query_chain_data(
    address: &str,
    network: Network,
    trongrid_api_key: &str,
    bscscan_api_key: &str,
) -> anyhow::Result<(Vec<TxRecord>, Option<i64>)> {
    match network {
        Network::Trc20 => query_trc20(address, trongrid_api_key).await,
        Network::Bsc => query_bsc(address, bscscan_api_key).await,
    }
}

async fn query_trc20(address: &str, api_key: &str) -> anyhow::Result<(Vec<TxRecord>, Option<i64>)> {
    let url = format!(
        "https://api.trongrid.io/v1/accounts/{address}/transactions/trc20?limit=20&order_by=block_timestamp,desc"
    );

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(&url)
        .header("TRON-PRO-API-KEY", api_key)
        .send()
        .await?
        .json()
        .await?;

    let mut txs = Vec::new();
    let mut latest_ts: Option<i64> = None;

    if resp
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        if let Some(data) = resp.get("data").and_then(|v| v.as_array()) {
            // Filter for USDT (TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t)
            let usdt_txs: Vec<&serde_json::Value> = data
                .iter()
                .filter(|tx| {
                    tx.get("token_info")
                        .and_then(|t| t.get("address"))
                        .and_then(|a| a.as_str())
                        .map(|a| a == "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")
                        .unwrap_or(false)
                })
                .collect();

            if let Some(first) = usdt_txs.first() {
                latest_ts = first.get("block_timestamp").and_then(|v| v.as_i64());
            }

            for tx in usdt_txs.iter().take(10) {
                let value: f64 = tx
                    .get("value")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 1_000_000.0;

                let to_addr = tx.get("to").and_then(|v| v.as_str()).unwrap_or("");
                let direction = if to_addr == address {
                    TxDirection::Incoming
                } else {
                    TxDirection::Outgoing
                };

                let ts_ms = tx
                    .get("block_timestamp")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let ts_secs = ts_ms / 1000;
                let datetime = chrono::DateTime::from_timestamp(ts_secs, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                txs.push(TxRecord {
                    direction,
                    value,
                    timestamp_str: datetime,
                });
            }
        }
    }

    Ok((txs, latest_ts))
}

async fn query_bsc(address: &str, api_key: &str) -> anyhow::Result<(Vec<TxRecord>, Option<i64>)> {
    let url = format!(
        "https://api.bscscan.com/api?module=account&action=tokentx\
         &contractaddress=0x55d398326f99059fF775485246999027B3197955\
         &address={address}&sort=desc&page=1&offset=20&apikey={api_key}"
    );

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client.get(&url).send().await?.json().await?;

    let mut txs = Vec::new();
    let mut latest_ts: Option<i64> = None;

    if resp.get("status").and_then(|v| v.as_str()) == Some("1") {
        if let Some(result) = resp.get("result").and_then(|v| v.as_array()) {
            if let Some(first) = result.first() {
                latest_ts = first
                    .get("timeStamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|t| t * 1000);
            }

            for tx in result.iter().take(10) {
                let value: f64 = tx
                    .get("value")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 1_000_000_000_000_000_000.0;

                let to_addr = tx
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let direction = if to_addr == address.to_lowercase() {
                    TxDirection::Incoming
                } else {
                    TxDirection::Outgoing
                };

                let ts = tx
                    .get("timeStamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0);
                let datetime = chrono::DateTime::from_timestamp(ts, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                txs.push(TxRecord {
                    direction,
                    value,
                    timestamp_str: datetime,
                });
            }
        }
    }

    Ok((txs, latest_ts))
}

// ---------------------------------------------------------------------------
// DB persistence for wallet monitors
// ---------------------------------------------------------------------------

/// Save a wallet monitor entry. `initial_timestamp` is the latest chain
/// timestamp (ms) at registration time, used to skip historical pushes.
pub async fn save_wallet_monitor(
    pool: &sqlx::PgPool,
    user_id: i64,
    address: &str,
    network: &str,
    initial_timestamp: Option<i64>,
) {
    let _ = sqlx::query(
        r#"
        INSERT INTO wallet_monitors (user_id, address, network, last_seen_ts, active, created_at, updated_at)
        VALUES ($1, $2, $3, $4, true, now(), now())
        ON CONFLICT (user_id, address) DO UPDATE SET
            last_seen_ts = EXCLUDED.last_seen_ts,
            active       = true,
            updated_at   = now()
        "#,
    )
    .bind(user_id)
    .bind(address)
    .bind(network)
    .bind(initial_timestamp)
    .execute(pool)
    .await;

    tracing::info!(
        "Wallet monitor saved: {} for user {} with ts {:?}",
        address,
        user_id,
        initial_timestamp
    );
}
