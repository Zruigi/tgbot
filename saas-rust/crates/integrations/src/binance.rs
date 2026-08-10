//! Binance API client — market data, prices, and ticker info.
//!
//! Provides methods for:
//! - Fetching current price for a symbol
//! - Getting 24hr ticker statistics
//! - Fetching recent trades
//! - K-line (candlestick) data
//! - Health check

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for connecting to Binance API.
#[derive(Clone, Debug)]
pub struct BinanceConfig {
    /// Base URL of the Binance API (use `https://api.binance.com` for production,
    /// or testnet `https://testnet.binance.vision`).
    pub base_url: String,
    /// API key for authenticated endpoints (optional for public data).
    pub api_key: Option<String>,
    /// Secret key for signed requests (optional for public data).
    pub secret_key: Option<String>,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl BinanceConfig {
    /// Build config from environment variables:
    /// - `BINANCE_BASE_URL` (default `https://api.binance.com`)
    /// - `BINANCE_API_KEY` (optional)
    /// - `BINANCE_SECRET_KEY` (optional)
    /// - `BINANCE_TIMEOUT_SECS` (default `30`)
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            base_url: std::env::var("BINANCE_BASE_URL")
                .unwrap_or_else(|_| "https://data-api.binance.vision".into()),
            api_key: std::env::var("BINANCE_API_KEY").ok(),
            secret_key: std::env::var("BINANCE_SECRET_KEY").ok(),
            timeout_secs: std::env::var("BINANCE_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        })
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Current price response.
#[derive(Debug, Deserialize)]
pub struct PriceTicker {
    pub symbol: String,
    pub price: String,
}

/// 24hr ticker statistics.
#[derive(Debug, Deserialize)]
pub struct Ticker24hr {
    #[serde(rename = "symbol")]
    pub symbol: String,
    #[serde(rename = "priceChange")]
    pub price_change: String,
    #[serde(rename = "priceChangePercent")]
    pub price_change_percent: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "highPrice")]
    pub high_price: String,
    #[serde(rename = "lowPrice")]
    pub low_price: String,
    #[serde(rename = "volume")]
    pub volume: String,
    #[serde(rename = "quoteVolume")]
    pub quote_volume: String,
}

/// Recent trade.
#[derive(Debug, Deserialize)]
pub struct Trade {
    #[serde(rename = "id")]
    pub id: i64,
    #[serde(rename = "price")]
    pub price: String,
    #[serde(rename = "qty")]
    pub qty: String,
    #[serde(rename = "time")]
    pub time: i64,
    #[serde(rename = "isBuyerMaker")]
    pub is_buyer_maker: bool,
}

/// K-line (candlestick) data.
#[derive(Debug, Deserialize)]
pub struct KLine {
    #[serde(rename = "0")]
    pub open_time: i64,
    #[serde(rename = "1")]
    pub open: String,
    #[serde(rename = "2")]
    pub high: String,
    #[serde(rename = "3")]
    pub low: String,
    #[serde(rename = "4")]
    pub close: String,
    #[serde(rename = "5")]
    pub volume: String,
    #[serde(rename = "6")]
    pub close_time: i64,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Binance HTTP client.
#[derive(Clone, Debug)]
pub struct BinanceClient {
    http: Client,
    config: BinanceConfig,
}

impl BinanceClient {
    /// Create a new client from the given config.
    pub fn new(config: BinanceConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("failed to build reqwest client for Binance")?;
        Ok(Self { http, config })
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self> {
        Self::new(BinanceConfig::from_env()?)
    }

    // -- Public endpoints -----------------------------------------------------

    /// Get current price for a symbol.
    #[instrument(skip(self), fields(symbol = %symbol))]
    pub async fn get_price(&self, symbol: &str) -> Result<f64> {
        let url = format!(
            "{}/api/v3/ticker/price?symbol={}",
            self.config.base_url, symbol
        );
        debug!(url, "querying Binance price");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Binance price request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "Binance price error");
            return Err(anyhow!(
                "Binance price request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let ticker: PriceTicker = resp
            .json()
            .await
            .context("failed to parse Binance price response")?;

        ticker
            .price
            .parse::<f64>()
            .context("failed to parse price as f64")
    }

    /// Get 24hr ticker statistics for a symbol.
    #[instrument(skip(self))]
    pub async fn get_24hr_ticker(&self, symbol: &str) -> Result<Ticker24hr> {
        let url = format!(
            "{}/api/v3/ticker/24hr?symbol={}",
            self.config.base_url, symbol
        );
        debug!(url, "querying Binance 24hr ticker");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Binance 24hr ticker request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "Binance 24hr ticker error");
            return Err(anyhow!(
                "Binance 24hr ticker request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json()
            .await
            .context("failed to parse Binance 24hr ticker response")
    }

    /// Get recent trades for a symbol.
    #[instrument(skip(self))]
    pub async fn get_recent_trades(&self, symbol: &str, limit: usize) -> Result<Vec<Trade>> {
        let url = format!(
            "{}/api/v3/trades?symbol={}&limit={}",
            self.config.base_url,
            symbol,
            limit.min(1000)
        );
        debug!(url, "querying Binance recent trades");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Binance trades request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Binance trades request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json()
            .await
            .context("failed to parse Binance trades response")
    }

    /// Get k-line (candlestick) data.
    #[instrument(skip(self))]
    pub async fn get_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<KLine>> {
        let url = format!(
            "{}/api/v3/klines?symbol={}&interval={}&limit={}",
            self.config.base_url,
            symbol,
            interval,
            limit.min(1000)
        );
        debug!(url, "querying Binance klines");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Binance klines request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Binance klines request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        // Binance returns array of arrays; serde_json can handle this
        let raw: Vec<Vec<serde_json::Value>> = resp
            .json()
            .await
            .context("failed to parse Binance klines response")?;

        let klines: Vec<KLine> = raw
            .into_iter()
            .filter_map(|arr| {
                if arr.len() >= 7 {
                    Some(KLine {
                        open_time: arr[0].as_i64()?,
                        open: arr[1].as_str()?.to_string(),
                        high: arr[2].as_str()?.to_string(),
                        low: arr[3].as_str()?.to_string(),
                        close: arr[4].as_str()?.to_string(),
                        volume: arr[5].as_str()?.to_string(),
                        close_time: arr[6].as_i64()?,
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(klines)
    }

    // -- Health check ---------------------------------------------------------

    /// Check if Binance API is reachable.
    pub async fn health(&self) -> Result<bool> {
        let url = format!("{}/api/v3/ping", self.config.base_url);
        let resp = self.http.get(&url).send().await;
        match resp {
            Ok(r) => Ok(r.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        std::env::remove_var("BINANCE_API_KEY");
        std::env::remove_var("BINANCE_SECRET_KEY");
        let cfg = BinanceConfig::from_env().unwrap();
        assert_eq!(cfg.base_url, "https://api.binance.com");
        assert!(cfg.api_key.is_none());
        assert!(cfg.secret_key.is_none());
    }
}
