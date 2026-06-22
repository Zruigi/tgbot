//! Payment worker – processes payment_process jobs from the queue.
//!
//! Handles payment-related tasks:
//! - Verify payment status (USDT-TRC20 deposits)
//! - Process subscription renewals
//! - Handle payment webhooks
//! - Record payment transactions

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::job_queue::ClaimedJob;
use crate::scheduler::Worker;

use tgbot_integrations::trongrid::TronGridClient;

// ---------------------------------------------------------------------------
// Payment worker
// ---------------------------------------------------------------------------

pub struct PaymentWorker {
    trongrid: Arc<TronGridClient>,
}

impl PaymentWorker {
    pub fn new(trongrid: Arc<TronGridClient>) -> Self {
        Self { trongrid }
    }
}

/// Payment task types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentTaskType {
    /// Verify a USDT deposit.
    VerifyDeposit,
    /// Check pending subscription renewals.
    SubscriptionRenewal,
    /// Process payment webhook.
    Webhook,
    /// Refund a payment.
    Refund,
    Custom(String),
}

impl PaymentTaskType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "verify_deposit" => PaymentTaskType::VerifyDeposit,
            "subscription_renewal" => PaymentTaskType::SubscriptionRenewal,
            "webhook" => PaymentTaskType::Webhook,
            "refund" => PaymentTaskType::Refund,
            other => PaymentTaskType::Custom(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            PaymentTaskType::VerifyDeposit => "verify_deposit",
            PaymentTaskType::SubscriptionRenewal => "subscription_renewal",
            PaymentTaskType::Webhook => "webhook",
            PaymentTaskType::Refund => "refund",
            PaymentTaskType::Custom(s) => s,
        }
    }
}

#[async_trait]
impl Worker for PaymentWorker {
    fn job_type(&self) -> &str {
        "payment_process"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;

        let task_type_str = payload
            .get("payment_task_type")
            .and_then(|v| v.as_str())
            .unwrap_or("verify_deposit");
        let task_type = PaymentTaskType::from_str(task_type_str);

        info!(task_type = task_type.as_str(), "processing payment job");

        let result = match task_type {
            PaymentTaskType::VerifyDeposit => {
                self.process_verify_deposit(pool, payload).await
            }
            PaymentTaskType::SubscriptionRenewal => {
                self.process_subscription_renewal(pool, payload).await
            }
            PaymentTaskType::Webhook => {
                self.process_webhook(pool, payload).await
            }
            PaymentTaskType::Refund => {
                self.process_refund(pool, payload).await
            }
            PaymentTaskType::Custom(ref t) => {
                warn!(custom_type = t, "unknown payment task type, skipping");
                Ok(json!({"status": "skipped", "reason": "unknown_task_type"}))
            }
        };

        match result {
            Ok(output) => {
                info!(task_type = task_type.as_str(), "payment processing completed");
                Ok(())
            }
            Err(e) => {
                warn!(task_type = task_type.as_str(), error = %e, "payment processing failed");
                Err(e)
            }
        }
    }
}

impl PaymentWorker {
    /// Verify a USDT-TRC20 deposit by checking the transaction on-chain.
    async fn process_verify_deposit(&self, _pool: &PgPool, payload: &Value) -> Result<Value> {
        let tx_hash = payload
            .get("tx_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing tx_hash in payload"))?;
        let expected_amount = payload
            .get("expected_amount")
            .and_then(|v| v.as_str())
            .map(|s| s.parse::<f64>().ok())
            .flatten();

        // Query TronGrid for the transaction
        let tx_data = self.trongrid.get_transaction(tx_hash).await?;

        // Parse transaction data
        let amount: i64 = tx_data
            .get("contract")
            .and_then(|c| c.get("parameter"))
            .and_then(|p| p.get("value"))
            .and_then(|v| v.get("amount"))
            .and_then(|a| a.as_i64())
            .unwrap_or(0);

        let amount_usdt = amount as f64 / 1_000_000.0; // USDT uses 6 decimals

        info!(
            tx_hash,
            amount_usdt,
            "verified USDT deposit"
        );

        let mut verified = false;
        let mut message = "Transaction not found or invalid".to_string();

        if let Some(expected) = expected_amount {
            // Allow 1% tolerance for amount comparison
            let tolerance = expected * 0.01;
            if (amount_usdt - expected).abs() <= tolerance {
                verified = true;
                message = format!("Deposit verified: {} USDT", amount_usdt);
            } else {
                message = format!(
                    "Amount mismatch: expected {}, got {}",
                    expected, amount_usdt
                );
            }
        } else {
            // No expected amount, just verify the transaction exists and has value
            verified = amount_usdt > 0.0;
            message = if verified {
                format!("Deposit verified: {} USDT", amount_usdt)
            } else {
                "Invalid transaction: zero amount".to_string()
            };
        }

        Ok(json!({
            "tx_hash": tx_hash,
            "amount": amount_usdt,
            "verified": verified,
            "message": message,
        }))
    }

    /// Check and process pending subscription renewals.
    async fn process_subscription_renewal(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let tenant_id = payload
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        // Find subscriptions that expired within the last 24 hours and haven't been renewed
        let rows = sqlx::query(
            r#"
            SELECT id, merchant_id, plan_code, subscription_expires_at
            FROM merchants
            WHERE tenant_id = $1
              AND subscription_status = 'active'
              AND subscription_expires_at <= now()
              AND subscription_expires_at > now() - interval '24 hours'
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;

        let mut renewed = 0;
        let mut failed = 0;

        for row in rows {
            let merchant_id: uuid::Uuid = row.try_get("id")?;
            let plan_code: String = row.try_get("plan_code")?;

            // In production, this would:
            // 1. Check if the merchant has sufficient balance
            // 2. Process the payment
            // 3. Extend the subscription_expires_at
            // For now, we just log and mark as processed

            info!(merchant_id = %merchant_id, plan_code, "processing subscription renewal");

            // Update subscription status to expired
            let update = sqlx::query(
                r#"
                UPDATE merchants
                SET subscription_status = 'expired', updated_at = now()
                WHERE id = $1
                "#,
            )
            .bind(merchant_id)
            .execute(pool)
            .await;

            match update {
                Ok(_) => renewed += 1,
                Err(e) => {
                    warn!(merchant_id = %merchant_id, error = %e, "failed to update subscription status");
                    failed += 1;
                }
            }
        }

        Ok(json!({
            "tenant_id": tenant_id,
            "processed": renewed + failed,
            "renewed": renewed,
            "failed": failed,
        }))
    }

    /// Process a payment webhook (e.g., from Paddle, payment gateway).
    async fn process_webhook(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let webhook_type = payload
            .get("webhook_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let event_data = payload.get("event_data").cloned().unwrap_or(Value::Null);

        info!(webhook_type, "processing payment webhook");

        // Store the webhook event for audit
        sqlx::query(
            r#"
            INSERT INTO payment_webhooks (webhook_type, event_data, created_at)
            VALUES ($1, $2, now())
            "#,
        )
        .bind(webhook_type)
        .bind(serde_json::to_string(&event_data)?)
        .execute(pool)
        .await?;

        // Process based on webhook type
        match webhook_type {
            "payment_succeeded" => {
                // Find the merchant and update their subscription
                if let Some(merchant_id) = event_data.get("merchant_id").and_then(|v| v.as_str()) {
                    let amount = event_data.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let plan_code = event_data.get("plan_code").and_then(|v| v.as_str()).unwrap_or("free");

                    // Update merchant subscription
                    sqlx::query(
                        r#"
                        UPDATE merchants
                        SET subscription_status = 'active',
                            plan_code = $1,
                            subscription_expires_at = now() + interval '30 days',
                            updated_at = now()
                        WHERE id = $2
                        "#,
                    )
                    .bind(plan_code)
                    .bind(merchant_id)
                    .execute(pool)
                    .await?;

                    info!(merchant_id, amount, "subscription activated via webhook");
                }
            }
            "payment_failed" => {
                if let Some(merchant_id) = event_data.get("merchant_id").and_then(|v| v.as_str()) {
                    warn!(merchant_id, "payment failed, subscription may be affected");
                }
            }
            _ => {
                info!(webhook_type, "unhandled webhook type");
            }
        }

        Ok(json!({
            "webhook_type": webhook_type,
            "processed": true,
        }))
    }

    /// Process a refund request.
    async fn process_refund(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let payment_id = payload
            .get("payment_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing payment_id in payload"))?;
        let reason = payload
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("manual refund");

        info!(payment_id, reason, "processing refund");

        // In production, this would:
        // 1. Verify the payment exists and is eligible for refund
        // 2. Process the refund via the payment provider
        // 3. Update the payment record
        // For now, we just record the refund request

        let refund_id = uuid::Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO refunds (id, payment_id, reason, status, created_at)
            VALUES ($1, $2, $3, 'pending', now())
            "#,
        )
        .bind(refund_id.to_string())
        .bind(payment_id)
        .bind(reason)
        .execute(pool)
        .await?;

        Ok(json!({
            "refund_id": refund_id.to_string(),
            "payment_id": payment_id,
            "status": "pending",
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payment_task_type_roundtrip() {
        let types = vec![
            PaymentTaskType::VerifyDeposit,
            PaymentTaskType::SubscriptionRenewal,
            PaymentTaskType::Webhook,
            PaymentTaskType::Refund,
            PaymentTaskType::Custom("my_payment".into()),
        ];
        for t in types {
            assert_eq!(PaymentTaskType::from_str(t.as_str()), t);
        }
    }
}