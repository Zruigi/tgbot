//! Crypto Price handler – migrated from handlers/crypto_price.py
//!
//! Provides spot and futures price queries via Binance API,
//! with inline keyboard for adding price alerts.

use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};

use crate::handlers::HandlerResult;

// ---------------------------------------------------------------------------
// Conversation state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoPriceState {
    WaitingSymbol,
}

// ---------------------------------------------------------------------------
// Price type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PriceType {
    #[default]
    Spot,
    Futures,
}

impl PriceType {
    pub fn label(&self) -> &'static str {
        match self {
            PriceType::Spot => "现货",
            PriceType::Futures => "合约",
        }
    }
}

/// Per-user data carried through the conversation.
#[derive(Debug, Clone, Default)]
pub struct CryptoPriceUserData {
    pub price_type: PriceType,
}

impl CryptoPriceUserData {
    pub fn new(price_type: PriceType) -> Self {
        Self { price_type }
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Start price query from callback (spot_price / futures_price).
pub async fn start_price_query(
    bot: &Bot,
    chat_id: ChatId,
    message_id: i32,
    price_type: PriceType,
) -> HandlerResult<CryptoPriceState> {
    let text = "🔍 <b>价格查询</b>\n\n请输入您要查询的币种（例如：<code>BTC/USDT</code> 或 <code>ETH/USDT</code>）：";
    bot.edit_message_text(chat_id, MessageId(message_id), text)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(CryptoPriceState::WaitingSymbol)
}

/// Start price query from a text message.
pub async fn start_price_query_from_text(
    bot: &Bot,
    chat_id: ChatId,
    price_type: PriceType,
) -> HandlerResult<CryptoPriceState> {
    let text = "🔍 <b>价格查询</b>\n\n请输入您要查询的币种（例如：<code>BTC/USDT</code> 或 <code>ETH/USDT</code>）：";
    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(CryptoPriceState::WaitingSymbol)
}

// ---------------------------------------------------------------------------
// Handle symbol input
// ---------------------------------------------------------------------------

/// Process user-submitted trading pair symbol and fetch price from Binance.
pub async fn handle_symbol_input(
    bot: &Bot,
    chat_id: ChatId,
    symbol: &str,
    price_type: PriceType,
) -> HandlerResult<()> {
    let mut symbol = symbol.trim().to_uppercase();

    // Auto-append /USDT if slash is missing
    if !symbol.contains('/') {
        symbol.push_str("/USDT");
    }

    match fetch_ticker(&symbol, price_type).await {
        Ok(ticker) => {
            let change_sign = if ticker.change > 0.0 { "+" } else { "" };
            let msg = format!(
                "📈 <b>{symbol} {}行情</b>\n\n\
                 最新价格: <code>{}</code>\n\
                 24h 涨跌: <code>{change_sign}{:.2}%</code>\n\n\
                 💡 点击下方按钮可添加实时价格报警。",
                price_type.label(),
                ticker.last_price,
                ticker.change,
            );

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "🔔 添加价格提醒",
                    "add_price_alert",
                )],
                vec![InlineKeyboardButton::callback(
                    "🔙 返回主菜单",
                    "back_to_start",
                )],
            ]);

            bot.send_message(chat_id, &msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        Err(e) => {
            tracing::error!("Price query failed for {symbol}: {e}");
            bot.send_message(
                chat_id,
                format!("❌ 查询失败：未能找到交易对 {symbol}。请检查输入是否正确。"),
            )
            .await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Binance API integration
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TickerResult {
    pub last_price: f64,
    pub change: f64,
}

/// Fetch ticker from Binance for the given symbol.
/// Uses public API – no API key required.
pub async fn fetch_ticker(symbol: &str, price_type: PriceType) -> anyhow::Result<TickerResult> {
    let market_type = match price_type {
        PriceType::Spot => "spot",
        PriceType::Futures => "futures",
    };

    // Binance API: /api/v3/ticker/24hr for spot, /fapi/v1/ticker/24hr for futures
    let url = match price_type {
        PriceType::Spot => format!(
            "https://api.binance.com/api/v3/ticker/24hr?symbol={}",
            symbol.replace('/', "")
        ),
        PriceType::Futures => format!(
            "https://fapi.binance.com/fapi/v1/ticker/24hr?symbol={}",
            symbol.replace('/', "")
        ),
    };

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client.get(&url).send().await?.json().await?;

    let last_price = resp
        .get("lastPrice")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| resp.get("lastPrice").and_then(|v| v.as_f64()))
        .ok_or_else(|| anyhow::anyhow!("missing lastPrice in response"))?;

    let change = resp
        .get("priceChangePercent")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| resp.get("priceChangePercent").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);

    Ok(TickerResult { last_price, change })
}
