//! OCR worker – processes ocr_process jobs from the queue.
//!
//! Handles OCR-related tasks:
//! - Image text extraction
//! - Document scanning
//! - QR code / address detection from images
//!
//! Uses the PaddleOCR integration for actual OCR processing.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::job_queue::ClaimedJob;
use crate::push_task_worker::TelegramClientResolver;
use crate::scheduler::Worker;

use tgbot_integrations::paddleocr::PaddleOCRClient;
use tgbot_integrations::telegram::GetFileParams;

// ---------------------------------------------------------------------------
// OCR worker
// ---------------------------------------------------------------------------

pub struct OcrWorker {
    ocr: Arc<PaddleOCRClient>,
    telegram_resolver: Arc<dyn TelegramClientResolver>,
}

impl OcrWorker {
    pub fn new(ocr: Arc<PaddleOCRClient>, telegram_resolver: Arc<dyn TelegramClientResolver>) -> Self {
        Self {
            ocr,
            telegram_resolver,
        }
    }
}

/// OCR task types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrTaskType {
    /// Extract text from a Telegram image (by file_id).
    ImageExtraction,
    /// Detect cryptocurrency addresses in an image.
    AddressDetection,
    /// Extract text from a document image.
    DocumentScan,
    Custom(String),
}

impl OcrTaskType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "image_extraction" => OcrTaskType::ImageExtraction,
            "address_detection" => OcrTaskType::AddressDetection,
            "document_scan" => OcrTaskType::DocumentScan,
            other => OcrTaskType::Custom(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            OcrTaskType::ImageExtraction => "image_extraction",
            OcrTaskType::AddressDetection => "address_detection",
            OcrTaskType::DocumentScan => "document_scan",
            OcrTaskType::Custom(s) => s,
        }
    }
}

#[async_trait]
impl Worker for OcrWorker {
    fn job_type(&self) -> &str {
        "ocr_process"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;

        let task_type_str = payload
            .get("ocr_task_type")
            .and_then(|v| v.as_str())
            .unwrap_or("image_extraction");
        let task_type = OcrTaskType::from_str(task_type_str);

        info!(task_type = task_type.as_str(), "processing OCR job");

        let result = match task_type {
            OcrTaskType::ImageExtraction => self.process_image_extraction(pool, payload).await,
            OcrTaskType::AddressDetection => self.process_address_detection(pool, payload).await,
            OcrTaskType::DocumentScan => self.process_document_scan(pool, payload).await,
            OcrTaskType::Custom(ref t) => {
                warn!(custom_type = t, "unknown OCR task type, skipping");
                Ok(json!({"status": "skipped", "reason": "unknown_task_type"}))
            }
        };

        match result {
            Ok(output) => {
                info!(task_type = task_type.as_str(), "OCR processing completed");
                // Store OCR result for downstream use
                if let Err(e) = self.store_result(pool, claimed.job.id, &output).await {
                    warn!(error = %e, "failed to store OCR result");
                }
                Ok(())
            }
            Err(e) => {
                warn!(task_type = task_type.as_str(), error = %e, "OCR processing failed");
                Err(e)
            }
        }
    }
}

impl OcrWorker {
    /// Extract text from a Telegram image using file_id.
    async fn process_image_extraction(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let file_id = payload
            .get("file_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing file_id in payload"))?;
        let tenant_id = payload
            .get("tenant_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        let telegram = self.telegram_resolver.resolve(pool, tenant_id).await?;

        // Download the file from Telegram
        let file_info = telegram
            .get_file(GetFileParams {
                file_id: file_id.to_string(),
            })
            .await?;

        // Download file bytes
        // Note: In a production system, we'd download to a temp file or buffer.
        // For now, we use a temp path.
        let temp_path = format!("/tmp/ocr_{}", uuid::Uuid::new_v4());
        telegram.download_file(&file_info, &temp_path).await?;

        // Read file bytes
        let image_bytes = tokio::fs::read(&temp_path).await?;

        // Run OCR
        let lang = payload.get("lang").and_then(|v| v.as_str());
        let ocr_response = self.ocr.extract_text(&image_bytes, lang).await?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_path).await;

        Ok(json!({
            "file_id": file_id,
            "extracted_text": ocr_response,
            "text_length": ocr_response.len(),
        }))
    }

    /// Detect cryptocurrency addresses in an image.
    async fn process_address_detection(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let file_id = payload
            .get("file_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing file_id in payload"))?;
        let tenant_id = payload
            .get("tenant_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        let telegram = self.telegram_resolver.resolve(pool, tenant_id).await?;

        // Download image from Telegram
        let file_info = telegram
            .get_file(GetFileParams {
                file_id: file_id.to_string(),
            })
            .await?;

        let temp_path = format!("/tmp/ocr_addr_{}", uuid::Uuid::new_v4());
        telegram.download_file(&file_info, &temp_path).await?;
        let image_bytes = tokio::fs::read(&temp_path).await?;

        // Run OCR
        let ocr_text = self.ocr.extract_text(&image_bytes, Some("en")).await?;

        // Clean up
        let _ = tokio::fs::remove_file(&temp_path).await;

        // Extract potential crypto addresses from OCR text
        let addresses = extract_crypto_addresses(&ocr_text);

        Ok(json!({
            "file_id": file_id,
            "ocr_text": ocr_text,
            "detected_addresses": addresses,
        }))
    }

    /// Extract text from a document image (higher accuracy mode).
    async fn process_document_scan(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let file_id = payload
            .get("file_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing file_id in payload"))?;
        let tenant_id = payload
            .get("tenant_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        let telegram = self.telegram_resolver.resolve(pool, tenant_id).await?;

        let file_info = telegram
            .get_file(GetFileParams {
                file_id: file_id.to_string(),
            })
            .await?;

        let temp_path = format!("/tmp/ocr_doc_{}", uuid::Uuid::new_v4());
        telegram.download_file(&file_info, &temp_path).await?;
        let image_bytes = tokio::fs::read(&temp_path).await?;

        let lang = payload.get("lang").and_then(|v| v.as_str()).unwrap_or("ch");
        let ocr_text = self.ocr.extract_text(&image_bytes, Some(lang)).await?;

        let _ = tokio::fs::remove_file(&temp_path).await;

        Ok(json!({
            "file_id": file_id,
            "document_text": ocr_text,
            "char_count": ocr_text.len(),
        }))
    }

    /// Store OCR results in the database for later retrieval.
    async fn store_result(&self, pool: &PgPool, job_id: i64, result: &Value) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO ocr_results (job_id, result_data, created_at)
            VALUES ($1, $2, now())
            ON CONFLICT (job_id) DO UPDATE SET result_data = $2
            "#,
        )
        .bind(job_id)
        .bind(serde_json::to_string(result)?)
        .execute(pool)
        .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract potential cryptocurrency addresses from OCR text.
/// Supports TRON (T...), Ethereum (0x...), and Bitcoin (1.../3.../bc1...) addresses.
fn extract_crypto_addresses(text: &str) -> Vec<Value> {
    let mut addresses = Vec::new();

    for word in text.split_whitespace() {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());

        // TRON address: starts with 'T', 34 chars
        if word.starts_with('T') && word.len() == 34 {
            addresses.push(json!({
                "address": word,
                "chain": "tron",
                "confidence": "medium",
            }));
        }

        // Ethereum address: starts with '0x', 42 chars
        if word.starts_with("0x") && word.len() == 42 {
            addresses.push(json!({
                "address": word,
                "chain": "ethereum",
                "confidence": "high",
            }));
        }

        // Bitcoin address: starts with 1, 3, or bc1
        if (word.starts_with('1') || word.starts_with('3')) && word.len() >= 26 && word.len() <= 35
        {
            addresses.push(json!({
                "address": word,
                "chain": "bitcoin",
                "confidence": "low",
            }));
        }
        if word.starts_with("bc1") && word.len() >= 42 {
            addresses.push(json!({
                "address": word,
                "chain": "bitcoin",
                "confidence": "medium",
            }));
        }
    }

    addresses
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ocr_task_type_roundtrip() {
        let types = vec![
            OcrTaskType::ImageExtraction,
            OcrTaskType::AddressDetection,
            OcrTaskType::DocumentScan,
            OcrTaskType::Custom("my_ocr".into()),
        ];
        for t in types {
            assert_eq!(OcrTaskType::from_str(t.as_str()), t);
        }
    }

    #[test]
    fn test_extract_crypto_addresses() {
        let text = "Send to T9yD14tF5Wm7v1B3VvBz8k8Lm6nNpQrRs or 0x742d35Cc6634C0532925a3b844Bc9e7595f2bD18";
        let addrs = extract_crypto_addresses(text);
        assert!(!addrs.is_empty());

        // Check that we found at least the Ethereum address
        let has_eth = addrs.iter().any(|a| a["chain"] == "ethereum");
        assert!(has_eth);
    }

    #[test]
    fn test_extract_tron_address() {
        let text = "TRON wallet: TN2YqTv5G1YyXYg55hEY6gQy1RnFGBwMnj";
        let addrs = extract_crypto_addresses(text);
        let has_tron = addrs.iter().any(|a| a["chain"] == "tron");
        assert!(has_tron);
    }
}
