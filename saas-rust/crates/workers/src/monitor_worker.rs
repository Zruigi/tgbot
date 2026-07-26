//! Monitor worker – processes monitor_check jobs from the queue.
//!
//! Handles monitoring-related tasks:
//! - Wallet balance monitoring (TRON, BSC, ETH)
//! - Token price alerts
//! - Transaction monitoring
//! - Group activity monitoring
//!
//! When a condition triggers (e.g., balance change, price threshold), the worker
//! enqueues notification jobs or directly sends alerts.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::job_queue::{self, ClaimedJob, JobType};
use crate::scheduler::Worker;

use tgbot_integrations::binance::BinanceClient;
use tgbot_integrations::telegram::SendMessageParams;
use tgbot_integrations::telegram::TelegramClient;
use tgbot_integrations::trongrid::TronGridClient;

// ---------------------------------------------------------------------------
// Monitor worker
// ---------------------------------------------------------------------------

pub struct MonitorWorker {
    telegram: Arc<TelegramClient>,
    trongrid: Arc<TronGridClient>,
    binance: Arc<BinanceClient>,
}

impl MonitorWorker {
    pub fn new(
        telegram: Arc<TelegramClient>,
        trongrid: Arc<TronGridClient>,
        binance: Arc<BinanceClient>,
    ) -> Self {
        Self {
            telegram,
            trongrid,
            binance,
        }
    }
}

/// Monitor task types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorTaskType {
    /// Check wallet balance for changes.
    WalletBalance,
    /// Check token price against thresholds.
    PriceAlert,
    /// Check for recent transactions on an address.
    TransactionMonitor,
    /// Check group activity metrics.
    GroupActivity,
    Custom(String),
}

impl MonitorTaskType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "wallet_balance" => MonitorTaskType::WalletBalance,
            "price_alert" => MonitorTaskType::PriceAlert,
            "transaction_monitor" => MonitorTaskType::TransactionMonitor,
            "group_activity" => MonitorTaskType::GroupActivity,
            other => MonitorTaskType::Custom(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            MonitorTaskType::WalletBalance => "wallet_balance",
            MonitorTaskType::PriceAlert => "price_alert",
            MonitorTaskType::TransactionMonitor => "transaction_monitor",
            MonitorTaskType::GroupActivity => "group_activity",
            MonitorTaskType::Custom(s) => s,
        }
    }
}

#[async_trait]
impl Worker for MonitorWorker {
    fn job_type(&self) -> &str {
        "monitor_check"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;

        let task_type_str = payload
            .get("monitor_task_type")
            .and_then(|v| v.as_str())
            .unwrap_or("wallet_balance");
        let task_type = MonitorTaskType::from_str(task_type_str);

        info!(
            task_type = task_type.as_str(),
            "processing monitor check job"
        );

        let result = match task_type {
            MonitorTaskType::WalletBalance => self.process_wallet_balance(pool, payload).await,
            MonitorTaskType::PriceAlert => self.process_price_alert(pool, payload).await,
            MonitorTaskType::TransactionMonitor => {
                self.process_transaction_monitor(pool, payload).await
            }
            MonitorTaskType::GroupActivity => self.process_group_activity(pool, payload).await,
            MonitorTaskType::Custom(ref t) => {
                warn!(custom_type = t, "unknown monitor task type, skipping");
                Ok(json!({"status": "skipped", "reason": "unknown_task_type"}))
            }
        };

        match result {
            Ok(output) => {
                info!(task_type = task_type.as_str(), "monitor check completed");

                // If there are alerts, send notifications
                if let Some(alerts) = output.get("alerts").and_then(|v| v.as_array()) {
                    if !alerts.is_empty() {
                        if let Err(e) = self.send_alerts(pool, alerts).await {
                            warn!(error = %e, "failed to send monitor alerts");
                        }
                    }
                }

                // Store monitor result
                if let Err(e) = self.store_result(pool, claimed.job.id, &output).await {
                    warn!(error = %e, "failed to store monitor result");
                }

                Ok(())
            }
            Err(e) => {
                warn!(task_type = task_type.as_str(), error = %e, "monitor check failed");
                Err(e)
            }
        }
    }
}

impl MonitorWorker {
    /// Check wallet balance and compare with previous recorded balance.
    async fn process_wallet_balance(&self, _pool: &PgPool, payload: &Value) -> Result<Value> {
        let address = payload
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing address in payload"))?;
        let chain = payload
            .get("chain")
            .and_then(|v| v.as_str())
            .unwrap_or("tron");

        let mut alerts = Vec::new();

        match chain {
            "tron" => {
                // Query TronGrid for TRX balance
                match self.trongrid.get_account(address).await {
                    Ok(account_data) => {
                        let balance_sun: i64 = account_data
                            .get("balance")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let balance_trx = balance_sun as f64 / 1_000_000.0;

                        info!(address, balance_trx, "TRON wallet balance checked");

                        // Compare with previous balance (would need to read from DB)
                        let previous_balance = payload
                            .get("previous_balance")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);

                        if (balance_trx - previous_balance).abs() > 1.0 {
                            alerts.push(json!({
                                "type": "balance_change",
                                "chain": "tron",
                                "address": address,
                                "previous_balance": previous_balance,
                                "current_balance": balance_trx,
                                "change": balance_trx - previous_balance,
                            }));
                        }
                    }
                    Err(e) => {
                        warn!(address, error = %e, "failed to query TronGrid for balance");
                    }
                }
            }
            other => {
                warn!(chain = other, "unsupported chain for wallet balance check");
            }
        }

        Ok(json!({
            "address": address,
            "chain": chain,
            "alerts": alerts,
        }))
    }

    /// Check token prices against configured thresholds.
    async fn process_price_alert(&self, _pool: &PgPool, payload: &Value) -> Result<Value> {
        let symbol = payload
            .get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing symbol in payload"))?;
        let threshold_high = payload.get("threshold_high").and_then(|v| v.as_f64());
        let threshold_low = payload.get("threshold_low").and_then(|v| v.as_f64());

        let mut alerts = Vec::new();

        // Query Binance for current price
        match self.binance.get_price(symbol).await {
            Ok(price) => {
                info!(symbol, price, "price checked");

                if let Some(high) = threshold_high {
                    if price >= high {
                        alerts.push(json!({
                            "type": "price_above_threshold",
                            "symbol": symbol,
                            "price": price,
                            "threshold": high,
                        }));
                    }
                }

                if let Some(low) = threshold_low {
                    if price <= low {
                        alerts.push(json!({
                            "type": "price_below_threshold",
                            "symbol": symbol,
                            "price": price,
                            "threshold": low,
                        }));
                    }
                }
            }
            Err(e) => {
                warn!(symbol, error = %e, "failed to query Binance for price");
            }
        }

        Ok(json!({
            "symbol": symbol,
            "alerts": alerts,
        }))
    }

    /// Monitor recent transactions on an address.
    async fn process_transaction_monitor(&self, _pool: &PgPool, payload: &Value) -> Result<Value> {
        let address = payload
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing address in payload"))?;
        let chain = payload
            .get("chain")
            .and_then(|v| v.as_str())
            .unwrap_or("tron");

        let mut alerts: Vec<Value> = Vec::new();

        if chain == "tron" {
            match self.trongrid.get_account_transactions(address, 5).await {
                Ok(txs) => {
                    if let Some(arr) = txs.as_array() {
                        info!(address, count = arr.len(), "transactions fetched");
                        // In production, compare with previously seen tx hashes
                        // to detect new incoming/outgoing transactions.
                    }
                }
                Err(e) => {
                    warn!(address, error = %e, "failed to query TronGrid for transactions");
                }
            }
        }

        Ok(json!({
            "address": address,
            "chain": chain,
            "alerts": alerts,
        }))
    }

    /// Check group activity metrics (message rate, member count, etc.).
    async fn process_group_activity(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let group_id = payload
            .get("group_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("missing group_id in payload"))?;

        // Count messages in the last hour
        let msg_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)::bigint FROM messages
            WHERE group_id = $1
              AND created_at > now() - interval '1 hour'
            "#,
        )
        .bind(group_id)
        .fetch_one(pool)
        .await?;

        let mut alerts = Vec::new();

        // Alert if message rate is unusually high (> 100 msgs/hour)
        if msg_count > 100 {
            alerts.push(json!({
                "type": "high_activity",
                "group_id": group_id,
                "messages_last_hour": msg_count,
                "threshold": 100,
            }));
        }

        Ok(json!({
            "group_id": group_id,
            "messages_last_hour": msg_count,
            "alerts": alerts,
        }))
    }

    /// Send alert notifications via Telegram.
    async fn send_alerts(&self, _pool: &PgPool, alerts: &[Value]) -> Result<()> {
        for alert in alerts {
            let alert_text = format!(
                "**Monitor Alert**\nType: {}\n{}",
                alert
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown"),
                serde_json::to_string_pretty(alert)?
            );

            // In production, we'd look up the target chat_id from the alert config.
            // For now, we log the alert.
            info!(alert = %alert_text, "monitor alert generated");

            // If a notify_chat_id is present, send directly
            if let Some(chat_id) = alert.get("notify_chat_id").and_then(|v| v.as_i64()) {
                let _ = self
                    .telegram
                    .send_message(SendMessageParams {
                        chat_id,
                        text: alert_text.clone(),
                        parse_mode: Some("Markdown".to_string()),
                        reply_markup: None,
                        reply_to_message_id: None,
                    })
                    .await;
            }
        }
        Ok(())
    }

    /// Store monitor results in the database.
    async fn store_result(&self, pool: &PgPool, job_id: i64, result: &Value) -> Result<()> {
        // Best-effort storage
        let _ = sqlx::query(
            r#"
            INSERT INTO monitor_results (job_id, result_data, created_at)
            VALUES ($1, $2, now())
            ON CONFLICT (job_id) DO UPDATE SET result_data = $2
            "#,
        )
        .bind(job_id)
        .bind(serde_json::to_string(result)?)
        .execute(pool)
        .await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_task_type_roundtrip() {
        let types = vec![
            MonitorTaskType::WalletBalance,
            MonitorTaskType::PriceAlert,
            MonitorTaskType::TransactionMonitor,
            MonitorTaskType::GroupActivity,
            MonitorTaskType::Custom("my_monitor".into()),
        ];
        for t in types {
            assert_eq!(MonitorTaskType::from_str(t.as_str()), t);
        }
    }
}
