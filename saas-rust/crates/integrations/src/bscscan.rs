use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tracing::warn;

#[derive(Clone, Debug)]
pub struct BscScanConfig {
    pub base_url: String,
    pub api_key: String,
    pub timeout_secs: u64,
}

impl BscScanConfig {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("BSCSCAN_API_KEY").unwrap_or_default();
        if api_key.is_empty() {
            return Err(anyhow!("BSCSCAN_API_KEY environment variable is required"));
        }
        Ok(Self {
            base_url: std::env::var("BSCSCAN_BASE_URL")
                .unwrap_or_else(|_| "https://api.bscscan.com/api".to_string()),
            api_key,
            timeout_secs: std::env::var("BSCSCAN_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(30),
        })
    }
}

#[derive(Clone, Debug)]
pub struct BscScanClient {
    http: Client,
    config: BscScanConfig,
}

impl BscScanClient {
    pub fn new(config: BscScanConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("failed to build BscScan HTTP client")?;
        Ok(Self { http, config })
    }

    pub fn from_env() -> Result<Self> {
        Self::new(BscScanConfig::from_env()?)
    }

    pub async fn get_token_transactions(
        &self,
        address: &str,
        token_contract: &str,
        limit: usize,
    ) -> Result<Value> {
        let response = self
            .http
            .get(&self.config.base_url)
            .query(&[
                ("module", "account"),
                ("action", "tokentx"),
                ("address", address),
                ("contractaddress", token_contract),
                ("page", "1"),
                ("offset", &limit.clamp(1, 100).to_string()),
                ("sort", "desc"),
                ("apikey", &self.config.api_key),
            ])
            .send()
            .await
            .context("BscScan token transaction request failed")?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .context("failed to parse BscScan token transaction response")?;
        if !status.is_success() {
            warn!(status = %status, "BscScan token transaction request returned an error");
            return Err(anyhow!("BscScan token transaction request failed"));
        }
        if body.get("status").and_then(Value::as_str) == Some("0")
            && body.get("message").and_then(Value::as_str) != Some("No transactions found")
        {
            return Err(anyhow!("BscScan token transaction query failed"));
        }
        Ok(body
            .get("result")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])))
    }
}
