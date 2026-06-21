//! PaddleOCR HTTP client — text recognition via PaddleOCR service.
//!
//! Wraps a PaddleOCR HTTP service (typically `paddleocr-server` or a custom
//! FastAPI wrapper) for:
//! - Image OCR (text extraction)
//! - PDF OCR (multi-page text extraction)
//! - Health check

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for connecting to a PaddleOCR HTTP service.
#[derive(Clone, Debug)]
pub struct PaddleOCRConfig {
    /// Base URL of the PaddleOCR service, e.g. `http://127.0.0.1:8868`.
    pub base_url: String,
    /// Request timeout in seconds (OCR can be slow on large images).
    pub timeout_secs: u64,
    /// Default language for OCR (e.g. `ch`, `en`).
    pub default_lang: String,
}

impl PaddleOCRConfig {
    /// Build config from environment variables:
    /// - `PADDLEOCR_BASE_URL` (default `http://127.0.0.1:8868`)
    /// - `PADDLEOCR_TIMEOUT_SECS` (default `60`)
    /// - `PADDLEOCR_DEFAULT_LANG` (default `ch`)
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            base_url: std::env::var("PADDLEOCR_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8868".into()),
            timeout_secs: std::env::var("PADDLEOCR_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            default_lang: std::env::var("PADDLEOCR_DEFAULT_LANG")
                .unwrap_or_else(|_| "ch".into()),
        })
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// OCR request parameters.
#[derive(Clone, Debug, Serialize)]
pub struct OcrRequest {
    /// Base64-encoded image data.
    pub image: String,
    /// Language hint (e.g. `ch`, `en`, `japan`, `korean`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Whether to return bounding box coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_angle_cls: Option<bool>,
    /// Whether to use GPU acceleration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_gpu: Option<bool>,
}

/// A single OCR result line with text and bounding box.
#[derive(Clone, Debug, Deserialize)]
pub struct OcrTextLine {
    /// Recognized text.
    pub text: String,
    /// Confidence score (0.0 - 1.0).
    pub confidence: Option<f64>,
    /// Bounding box as `[[x1,y1],[x2,y2],[x3,y3],[x4,y4]]`.
    pub text_region: Option<Vec<Vec<f64>>>,
}

/// OCR response from the PaddleOCR service.
#[derive(Clone, Debug, Deserialize)]
pub struct OcrResponse {
    /// Recognized text lines.
    pub result: Option<Vec<OcrTextLine>>,
    /// Full text joined (some service implementations return this).
    pub text: Option<String>,
    /// Raw response from the PaddleOCR service (for forward-compatibility).
    #[serde(flatten)]
    pub extra: Value,
}

/// PDF OCR request parameters.
#[derive(Clone, Debug, Serialize)]
pub struct PdfOcrRequest {
    /// Base64-encoded PDF data.
    pub pdf: String,
    /// Language hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Maximum number of pages to process (0 = all).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<u32>,
}

/// PDF OCR response — results per page.
#[derive(Clone, Debug, Deserialize)]
pub struct PdfOcrResponse {
    /// Results per page.
    pub pages: Vec<PageOcrResult>,
    /// Total pages processed.
    pub total_pages: Option<u32>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PageOcrResult {
    /// Page number (1-based).
    pub page: Option<u32>,
    /// Recognized text lines for this page.
    pub lines: Option<Vec<OcrTextLine>>,
    /// Full text of the page.
    pub text: Option<String>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// PaddleOCR HTTP client.
#[derive(Clone, Debug)]
pub struct PaddleOCRClient {
    http: Client,
    config: PaddleOCRConfig,
}

impl PaddleOCRClient {
    /// Create a new client from the given config.
    pub fn new(config: PaddleOCRConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("failed to build reqwest client for PaddleOCR")?;
        Ok(Self { http, config })
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self> {
        Self::new(PaddleOCRConfig::from_env()?)
    }

    // -- Image OCR -----------------------------------------------------------

    /// Perform OCR on a base64-encoded image.
    #[instrument(skip(self, req))]
    pub async fn ocr(&self, req: &OcrRequest) -> Result<OcrResponse> {
        let url = format!("{}/v1/ocr", self.config.base_url);
        debug!(url, "sending PaddleOCR request");

        let resp = self
            .http
            .post(&url)
            .json(req)
            .send()
            .await
            .context("PaddleOCR request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "PaddleOCR error");
            return Err(anyhow!(
                "PaddleOCR request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<OcrResponse>()
            .await
            .context("failed to parse PaddleOCR response")
    }

    /// Convenience: OCR on raw image bytes (auto base64-encoded).
    pub async fn ocr_image_bytes(
        &self,
        image_bytes: &[u8],
        lang: Option<&str>,
    ) -> Result<OcrResponse> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(image_bytes);
        self.ocr(&OcrRequest {
            image: encoded,
            lang: lang.map(|l| l.to_string()).or(Some(self.config.default_lang.clone())),
            use_angle_cls: Some(true),
            use_gpu: None,
        })
        .await
    }

    /// Convenience: extract plain text from an image.
    pub async fn extract_text(
        &self,
        image_bytes: &[u8],
        lang: Option<&str>,
    ) -> Result<String> {
        let resp = self.ocr_image_bytes(image_bytes, lang).await?;
        // Prefer the joined `text` field if present
        if let Some(text) = &resp.text {
            if !text.is_empty() {
                return Ok(text.clone());
            }
        }
        // Otherwise join from result lines
        let text = resp
            .result
            .unwrap_or_default()
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(text)
    }

    // -- PDF OCR -------------------------------------------------------------

    /// Perform OCR on a base64-encoded PDF.
    #[instrument(skip(self, req))]
    pub async fn pdf_ocr(&self, req: &PdfOcrRequest) -> Result<PdfOcrResponse> {
        let url = format!("{}/v1/ocr/pdf", self.config.base_url);
        debug!(url, "sending PaddleOCR PDF request");

        let resp = self
            .http
            .post(&url)
            .json(req)
            .send()
            .await
            .context("PaddleOCR PDF request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "PaddleOCR PDF error");
            return Err(anyhow!(
                "PaddleOCR PDF request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<PdfOcrResponse>()
            .await
            .context("failed to parse PaddleOCR PDF response")
    }

    // -- Health check --------------------------------------------------------

    /// Check if the PaddleOCR service is reachable.
    pub async fn health(&self) -> Result<bool> {
        let url = format!("{}/health", self.config.base_url);
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
    fn test_ocr_request_serialization() {
        let req = OcrRequest {
            image: "base64data".into(),
            lang: Some("ch".into()),
            use_angle_cls: Some(true),
            use_gpu: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["image"], "base64data");
        assert_eq!(json["lang"], "ch");
        assert!(json.get("use_gpu").is_none()); // skipped
    }

    #[test]
    fn test_ocr_response_deserialization() {
        let raw = r#"{
            "result": [
                {"text": "Hello", "confidence": 0.99, "text_region": [[1,2],[3,4],[5,6],[7,8]]},
                {"text": "World", "confidence": 0.95}
            ]
        }"#;
        let resp: OcrResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.result.unwrap().len(), 2);
    }
}
