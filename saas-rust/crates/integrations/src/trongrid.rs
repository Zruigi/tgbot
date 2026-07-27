//! TronGrid API client — TRON network data via TronGrid HTTP API.
//!
//! Provides methods for:
//! - Querying account balances (TRX, USDT-TRC20, etc.)
//! - Fetching transaction history
//! - Getting token balances
//! - Health check

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for connecting to TronGrid API.
#[derive(Clone, Debug)]
pub struct TronGridConfig {
    /// Base URL of the TronGrid API (e.g., `https://api.trongrid.io`).
    pub base_url: String,
    /// API key for TronGrid.
    pub api_key: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl TronGridConfig {
    /// Build config from environment variables:
    /// - `TRONGRID_BASE_URL` (default `https://api.trongrid.io`)
    /// - `TRONGRID_API_KEY` (required)
    /// - `TRONGRID_TIMEOUT_SECS` (default `30`)
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("TRONGRID_API_KEY").unwrap_or_default();
        if api_key.is_empty() {
            return Err(anyhow!("TRONGRID_API_KEY environment variable is required"));
        }
        Ok(Self {
            base_url: std::env::var("TRONGRID_BASE_URL")
                .unwrap_or_else(|_| "https://api.trongrid.io".into()),
            api_key,
            timeout_secs: std::env::var("TRONGRID_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        })
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Account info response from TronGrid.
#[derive(Debug, Deserialize)]
pub struct AccountInfo {
    #[serde(rename = "address")]
    pub address: Option<String>,
    #[serde(rename = "balance")]
    pub balance: Option<i64>,
    #[serde(rename = "create_time")]
    pub create_time: Option<i64>,
    #[serde(rename = "latest_operation_time")]
    pub latest_operation_time: Option<i64>,
    #[serde(rename = "owner_permission")]
    pub owner_permission: Option<Value>,
    #[serde(rename = "active_permission")]
    pub active_permission: Option<Vec<Value>>,
    #[serde(rename = "frozen")]
    pub frozen: Option<Vec<FrozenBalance>>,
    #[serde(rename = "trc20")]
    pub trc20: Option<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Deserialize)]
pub struct FrozenBalance {
    #[serde(rename = "frozen_balance")]
    pub frozen_balance: Option<i64>,
    #[serde(rename = "expire_time")]
    pub expire_time: Option<i64>,
}

/// Transaction info from TronGrid.
#[derive(Debug, Deserialize)]
pub struct TransactionInfo {
    #[serde(rename = "txID")]
    pub tx_id: Option<String>,
    #[serde(rename = "block_timestamp")]
    pub block_timestamp: Option<i64>,
    #[serde(rename = "owner_address")]
    pub owner_address: Option<String>,
    #[serde(rename = "to_address")]
    pub to_address: Option<String>,
    #[serde(rename = "contract_type")]
    pub contract_type: Option<String>,
    #[serde(rename = "amount", default)]
    pub amount: Option<i64>,
    #[serde(rename = "contract")]
    pub contract: Option<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

/// Transfer contract details.
#[derive(Debug, Deserialize)]
pub struct TransferContract {
    #[serde(rename = "type_url")]
    pub type_url: Option<String>,
    #[serde(rename = "value")]
    pub value: Option<Value>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// TronGrid HTTP client.
#[derive(Clone, Debug)]
pub struct TronGridClient {
    http: Client,
    config: TronGridConfig,
}

impl TronGridClient {
    /// Create a new client from the given config.
    pub fn new(config: TronGridConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("failed to build reqwest client for TronGrid")?;
        Ok(Self { http, config })
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self> {
        Self::new(TronGridConfig::from_env()?)
    }

    // -- Account queries ------------------------------------------------------

    /// Get account information by address (base58 or hex format).
    #[instrument(skip(self), fields(address = %address))]
    pub async fn get_account(&self, address: &str) -> Result<Value> {
        let url = format!("{}/v1/accounts/{}", self.config.base_url, address);
        debug!(url, "querying TronGrid account");

        let resp = self
            .http
            .get(&url)
            .header("TRON-PRO-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("TronGrid account request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "TronGrid account error");
            return Err(anyhow!(
                "TronGrid account request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let json: Value = resp
            .json()
            .await
            .context("failed to parse TronGrid account response")?;

        // Return the data array's first item or an empty object
        if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            if let Some(first) = data.first() {
                return Ok(first.clone());
            }
        }

        Ok(Value::Null)
    }

    /// Get TRC20 token balance for an address and token contract.
    #[instrument(skip(self))]
    pub async fn get_trc20_balance(&self, address: &str, token_contract: &str) -> Result<String> {
        let url = format!(
            "{}/v1/accounts/{}/trc20/{}",
            self.config.base_url, address, token_contract
        );
        debug!(url, "querying TronGrid TRC20 balance");

        let resp = self
            .http
            .get(&url)
            .header("TRON-PRO-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("TronGrid TRC20 balance request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "TronGrid TRC20 balance error");
            return Err(anyhow!(
                "TronGrid TRC20 balance request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let json: Value = resp
            .json()
            .await
            .context("failed to parse TronGrid TRC20 balance response")?;

        // Return the balance from the data array
        if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            if let Some(first) = data.first() {
                if let Some(balance) = first.get("balance") {
                    return Ok(balance.to_string().trim_matches('"').to_string());
                }
            }
        }

        Ok("0".to_string())
    }

    // -- Transaction queries --------------------------------------------------

    /// Get transaction history for an address.
    #[instrument(skip(self), fields(address = %address))]
    pub async fn get_account_transactions(&self, address: &str, limit: usize) -> Result<Value> {
        let url = format!(
            "{}/v1/accounts/{}/transactions?limit={}",
            self.config.base_url, address, limit
        );
        debug!(url, "querying TronGrid transactions");

        let resp = self
            .http
            .get(&url)
            .header("TRON-PRO-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("TronGrid transactions request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "TronGrid transactions error");
            return Err(anyhow!(
                "TronGrid transactions request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let json: Value = resp
            .json()
            .await
            .context("failed to parse TronGrid transactions response")?;

        Ok(json.get("data").cloned().unwrap_or(Value::Array(vec![])))
    }

    /// Get a specific transaction by ID.
    pub async fn get_transaction(&self, tx_id: &str) -> Result<Value> {
        let url = format!("{}/v1/transactions/{}", self.config.base_url, tx_id);
        debug!(url, "querying TronGrid transaction");

        let resp = self
            .http
            .get(&url)
            .header("TRON-PRO-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("TronGrid transaction request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "TronGrid transaction request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let json: Value = resp
            .json()
            .await
            .context("failed to parse TronGrid transaction response")?;

        Ok(json.get("data").cloned().unwrap_or(Value::Null))
    }

    // -- Utility --------------------------------------------------------------

    /// Check if TronGrid API is reachable.
    pub async fn health(&self) -> Result<bool> {
        let url = format!("{}/v1/node_info", self.config.base_url);
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
    fn test_config_requires_api_key() {
        std::env::remove_var("TRONGRID_API_KEY");
        let cfg = TronGridConfig::from_env();
        assert!(cfg.is_err());
    }

    #[test]
    fn test_transaction_info_contract_type_and_amount() {
        // TronGrid transaction info with both contract_type and amount at top level
        let json = r#"{
            "txID": "abc123",
            "block_timestamp": 1700000000,
            "owner_address": "TWallet1",
            "to_address": "TWallet2",
            "contract_type": "Transfer",
            "amount": 1000000,
            "contract": {"type_url": "TransferContract", "value": {"amount": 1000000}}
        }"#;
        let tx: TransactionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(tx.tx_id.as_deref(), Some("abc123"));
        assert_eq!(tx.contract_type.as_deref(), Some("Transfer"));
        assert_eq!(tx.amount, Some(1_000_000));
    }

    #[test]
    fn test_transaction_info_amount_absent() {
        // TronGrid transaction info without top-level amount (e.g., non-transfer contracts)
        let json = r#"{
            "txID": "def456",
            "block_timestamp": 1700000001,
            "owner_address": "TWallet1",
            "to_address": "TWallet2",
            "contract_type": "TriggerSmartContract",
            "contract": {"type_url": "TriggerSmartContract", "value": {}}
        }"#;
        let tx: TransactionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(tx.contract_type.as_deref(), Some("TriggerSmartContract"));
        // amount is absent → default to None
        assert_eq!(tx.amount, None);
    }

    #[test]
    fn test_transaction_info_amount_zero() {
        // Transaction with amount = 0
        let json = r#"{
            "txID": "ghi789",
            "contract_type": "Transfer",
            "amount": 0
        }"#;
        let tx: TransactionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(tx.contract_type.as_deref(), Some("Transfer"));
        assert_eq!(tx.amount, Some(0));
    }
}
