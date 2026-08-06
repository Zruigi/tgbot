use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{info, instrument};

use crate::job_queue::ClaimedJob;
use crate::scheduler::Worker;
use tgbot_integrations::bscscan::BscScanClient;
use tgbot_integrations::telegram::SendMessageParams;
use tgbot_integrations::trongrid::TronGridClient;

use crate::push_task_worker::TelegramClientResolver;

const TRON_USDT_CONTRACT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
const BSC_USDT_CONTRACT: &str = "0x55d398326f99059ff775485246999027b3197955";

#[derive(Debug, Clone)]
struct ChainTransfer {
    tx_id: String,
    from: String,
    to: String,
    raw_amount: String,
    decimals: i32,
    token_symbol: String,
    timestamp_ms: i64,
}

pub struct AddressMonitorWorker {
    trongrid: Arc<TronGridClient>,
    bscscan: Arc<BscScanClient>,
}

impl AddressMonitorWorker {
    pub fn new(trongrid: Arc<TronGridClient>, bscscan: Arc<BscScanClient>) -> Self {
        Self { trongrid, bscscan }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonitorAlertJobKey {
    tenant_id: String,
    monitor_id: i64,
    tx_id: String,
}

pub struct AddressMonitorAlertWorker {
    telegram_resolver: Arc<dyn TelegramClientResolver>,
}

impl AddressMonitorAlertWorker {
    pub fn new(telegram_resolver: Arc<dyn TelegramClientResolver>) -> Self {
        Self { telegram_resolver }
    }
}

#[async_trait]
impl Worker for AddressMonitorWorker {
    fn job_type(&self) -> &str {
        "monitor_check"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let tenant_id = claimed
            .job
            .payload
            .get("tenant_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("monitor job has no tenant_id"))?;
        if tenant_id != claimed.job.tenant_id {
            return Err(anyhow!("monitor job tenant mismatch"));
        }
        let monitor_id = claimed
            .job
            .payload
            .get("monitor_id")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow!("monitor job has no monitor_id"))?;

        let monitor = sqlx::query(
            "SELECT monitor_id, user_id, address, network, last_tx_timestamp \
             FROM address_monitors \
             WHERE monitor_id = $1 AND tenant_id = $2 AND is_active = true",
        )
        .bind(monitor_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;
        let Some(monitor) = monitor else {
            return Ok(());
        };
        let user_id: i64 = monitor.try_get("user_id")?;
        let address: String = monitor.try_get("address")?;
        let network: String = monitor.try_get("network")?;
        let last_tx_timestamp: i64 = monitor
            .try_get::<Option<i64>, _>("last_tx_timestamp")?
            .unwrap_or(0);

        let transfers = match network.as_str() {
            "TRC20" => {
                let data = self
                    .trongrid
                    .get_trc20_transactions(&address, TRON_USDT_CONTRACT, 100)
                    .await
                    .context("failed to query TRC20 transactions")?;
                parse_tron_transfers(&data, &address)
            }
            "BSC" => {
                let data = self
                    .bscscan
                    .get_token_transactions(&address, BSC_USDT_CONTRACT, 100)
                    .await
                    .context("failed to query BSC transactions")?;
                parse_bsc_transfers(&data, &address)
            }
            other => return Err(anyhow!("unsupported monitor network: {other}")),
        };

        let mut new_count = 0usize;
        let mut latest_timestamp = last_tx_timestamp;
        let mut transaction = pool.begin().await?;
        for transfer in transfers {
            latest_timestamp = latest_timestamp.max(transfer.timestamp_ms);
            if transfer.timestamp_ms < last_tx_timestamp {
                continue;
            }
            let inserted = sqlx::query(
                "INSERT INTO monitor_transactions \
                 (tx_id, monitor_id, user_id, tenant_id, address, network, direction, amount, \
                  token_symbol, tx_timestamp, counterparty) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, \
                         $8::numeric / power(10::numeric, $9::int), $10, $11, $12) \
                 ON CONFLICT (tenant_id, tx_id) DO NOTHING",
            )
            .bind(&transfer.tx_id)
            .bind(monitor_id)
            .bind(user_id)
            .bind(tenant_id)
            .bind(&address)
            .bind(&network)
            .bind(direction_for(&transfer, &address, &network))
            .bind(&transfer.raw_amount)
            .bind(transfer.decimals)
            .bind(&transfer.token_symbol)
            .bind(transfer.timestamp_ms)
            .bind(counterparty_for(&transfer, &address, &network))
            .execute(&mut *transaction)
            .await?
            .rows_affected()
                > 0;
            if inserted {
                sqlx::query(
                    "INSERT INTO job_queue \
                     (tenant_id, job_type, payload, priority, max_attempts, run_at, status) \
                     VALUES ($1, 'monitor_alert', \
                             jsonb_build_object('tenant_id', $1, 'monitor_id', $2, 'tx_id', $3), \
                             4, 5, now(), 'pending') \
                     ON CONFLICT DO NOTHING",
                )
                .bind(tenant_id)
                .bind(monitor_id)
                .bind(&transfer.tx_id)
                .execute(&mut *transaction)
                .await?;
                new_count += 1;
            }
        }

        sqlx::query(
            "UPDATE address_monitors SET last_tx_timestamp = $3, updated_at = now() \
             WHERE monitor_id = $1 AND tenant_id = $2 AND is_active = true",
        )
        .bind(monitor_id)
        .bind(tenant_id)
        .bind(latest_timestamp)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        info!(
            monitor_id,
            tenant_id, new_count, "address monitor check completed"
        );
        Ok(())
    }
}

#[async_trait]
impl Worker for AddressMonitorAlertWorker {
    fn job_type(&self) -> &str {
        "monitor_alert"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let key = parse_alert_job_key(&claimed.job.payload)?;
        if key.tenant_id != claimed.job.tenant_id {
            return Err(anyhow!("monitor alert job tenant mismatch"));
        }

        let row = sqlx::query(
            "SELECT transaction_row.user_id, transaction_row.address, \
                    transaction_row.network, transaction_row.direction, \
                    transaction_row.amount::text AS amount, transaction_row.token_symbol \
             FROM monitor_transactions transaction_row \
             JOIN address_monitors monitor \
               ON monitor.monitor_id = transaction_row.monitor_id \
              AND monitor.tenant_id = transaction_row.tenant_id \
             WHERE transaction_row.tenant_id = $1 \
               AND transaction_row.monitor_id = $2 \
               AND transaction_row.tx_id = $3",
        )
        .bind(&key.tenant_id)
        .bind(key.monitor_id)
        .bind(&key.tx_id)
        .fetch_optional(pool)
        .await?;
        let Some(row) = row else {
            return Ok(());
        };

        let user_id: i64 = row.try_get("user_id")?;
        let address: String = row.try_get("address")?;
        let network: String = row.try_get("network")?;
        let direction: String = row.try_get("direction")?;
        let amount: String = row.try_get("amount")?;
        let token_symbol: String = row.try_get("token_symbol")?;
        let telegram = self
            .telegram_resolver
            .resolve(pool, &key.tenant_id)
            .await
            .context("failed to resolve monitor alert Telegram client")?;
        telegram
            .send_message(SendMessageParams {
                chat_id: user_id,
                text: format_persisted_transfer_alert(
                    &network,
                    &address,
                    &direction,
                    &amount,
                    &token_symbol,
                    &key.tx_id,
                ),
                parse_mode: None,
                reply_markup: None,
                reply_to_message_id: None,
            })
            .await
            .context("failed to send wallet monitor alert")?;

        info!(
            tenant_id = key.tenant_id,
            monitor_id = key.monitor_id,
            tx_id = key.tx_id,
            "wallet monitor alert delivered"
        );
        Ok(())
    }
}

fn parse_alert_job_key(payload: &Value) -> Result<MonitorAlertJobKey> {
    let tenant_id = payload
        .get("tenant_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("monitor alert job has no tenant_id"))?;
    let monitor_id = payload
        .get("monitor_id")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("monitor alert job has no monitor_id"))?;
    let tx_id = payload
        .get("tx_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("monitor alert job has no tx_id"))?;
    Ok(MonitorAlertJobKey {
        tenant_id: tenant_id.to_string(),
        monitor_id,
        tx_id: tx_id.to_string(),
    })
}

fn parse_tron_transfers(data: &Value, address: &str) -> Vec<ChainTransfer> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let tx_id = value
                .get("transaction_id")
                .or_else(|| value.get("txID"))
                .and_then(Value::as_str)?
                .to_string();
            let from = value.get("from").and_then(Value::as_str)?.to_string();
            let to = value.get("to").and_then(Value::as_str)?.to_string();
            let raw_amount = value.get("value").and_then(value_as_string)?;
            let timestamp_ms = value
                .get("block_timestamp")
                .and_then(value_as_i64)
                .filter(|value| *value > 0)?;
            let decimals = value
                .get("token_info")
                .and_then(|token| token.get("decimals"))
                .and_then(value_as_i64)
                .unwrap_or(6)
                .clamp(0, 30) as i32;
            let token_symbol = value
                .get("token_info")
                .and_then(|token| token.get("symbol"))
                .and_then(Value::as_str)
                .unwrap_or("USDT")
                .to_string();
            if !address_matches(&from, address, "TRC20") && !address_matches(&to, address, "TRC20")
            {
                return None;
            }
            Some(ChainTransfer {
                tx_id,
                from,
                to,
                raw_amount,
                decimals,
                token_symbol,
                timestamp_ms,
            })
        })
        .collect()
}

fn parse_bsc_transfers(data: &Value, address: &str) -> Vec<ChainTransfer> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let tx_id = value.get("hash").and_then(Value::as_str)?.to_string();
            let from = value.get("from").and_then(Value::as_str)?.to_string();
            let to = value.get("to").and_then(Value::as_str)?.to_string();
            let raw_amount = value.get("value").and_then(value_as_string)?;
            let timestamp_seconds = value
                .get("timeStamp")
                .and_then(value_as_i64)
                .filter(|value| *value > 0)?;
            let timestamp_ms = timestamp_seconds.checked_mul(1000)?;
            let decimals = value
                .get("tokenDecimal")
                .and_then(value_as_i64)
                .unwrap_or(18)
                .clamp(0, 30) as i32;
            let token_symbol = value
                .get("tokenSymbol")
                .and_then(Value::as_str)
                .unwrap_or("USDT")
                .to_string();
            if !address_matches(&from, address, "BSC") && !address_matches(&to, address, "BSC") {
                return None;
            }
            Some(ChainTransfer {
                tx_id,
                from,
                to,
                raw_amount,
                decimals,
                token_symbol,
                timestamp_ms,
            })
        })
        .collect()
}

fn value_as_string(value: &Value) -> Option<String> {
    let result = value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|number| number.to_string()))?;
    (!result.is_empty() && result.chars().all(|character| character.is_ascii_digit()))
        .then_some(result)
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

fn address_matches(left: &str, right: &str, network: &str) -> bool {
    if network == "BSC" {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn direction_for(transfer: &ChainTransfer, address: &str, network: &str) -> &'static str {
    if address_matches(&transfer.to, address, network) {
        "in"
    } else {
        "out"
    }
}

fn counterparty_for<'a>(transfer: &'a ChainTransfer, address: &str, network: &str) -> &'a str {
    if address_matches(&transfer.to, address, network) {
        &transfer.from
    } else {
        &transfer.to
    }
}

fn format_persisted_transfer_alert(
    network: &str,
    address: &str,
    direction: &str,
    amount: &str,
    token_symbol: &str,
    tx_id: &str,
) -> String {
    let direction = if direction == "in" { "📥" } else { "📤" };
    format!(
        "📡 {network} address {address} has a new transfer:\n{direction} {amount} {token_symbol} ({tx_id})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_tron_transfer_and_uses_milliseconds() {
        let transfers = parse_tron_transfers(
            &json!([{
                "transaction_id": "tron-tx",
                "from": "TFrom",
                "to": "TWallet",
                "value": "1234500",
                "block_timestamp": 1700000000123i64,
                "token_info": {"decimals": 6, "symbol": "USDT"}
            }]),
            "TWallet",
        );
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].timestamp_ms, 1700000000123);
        assert_eq!(direction_for(&transfers[0], "TWallet", "TRC20"), "in");
        assert_eq!(transfers[0].raw_amount, "1234500");
        assert_eq!(transfers[0].decimals, 6);
    }

    #[test]
    fn parses_bsc_transfer_timestamp_and_case_insensitive_address() {
        let transfers = parse_bsc_transfers(
            &json!([{
                "hash": "bsc-tx",
                "from": "0xfrom",
                "to": "0xABCDEF",
                "value": "1000000000000000000",
                "timeStamp": "1700000000",
                "tokenDecimal": "18",
                "tokenSymbol": "USDT"
            }]),
            "0xabcdef",
        );
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].timestamp_ms, 1700000000000);
        assert_eq!(transfers[0].raw_amount, "1000000000000000000");
        assert_eq!(transfers[0].decimals, 18);
    }

    #[test]
    fn rejects_non_numeric_chain_values() {
        assert!(parse_tron_transfers(
            &json!([{
                "transaction_id": "bad",
                "from": "TFrom",
                "to": "TWallet",
                "value": "1.2",
                "block_timestamp": 1700000000123i64
            }]),
            "TWallet",
        )
        .is_empty());
    }

    #[test]
    fn parses_tenant_scoped_alert_job_and_formats_message() {
        let key = parse_alert_job_key(&json!({
            "tenant_id": "tenant-a",
            "monitor_id": 42,
            "tx_id": "tx-1"
        }))
        .unwrap();
        assert_eq!(
            key,
            MonitorAlertJobKey {
                tenant_id: "tenant-a".into(),
                monitor_id: 42,
                tx_id: "tx-1".into()
            }
        );
        assert!(
            format_persisted_transfer_alert("TRC20", "TWallet", "in", "1.25", "USDT", "tx-1")
                .contains("📥 1.25 USDT (tx-1)")
        );
    }
}
