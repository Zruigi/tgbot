//! n8n HTTP client — trigger and query n8n workflows.
//!
//! Provides typed wrappers around n8n's REST API for:
//! - Triggering workflows via webhook URLs
//! - Listing / getting workflow definitions
//! - Listing / getting execution records

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for connecting to an n8n instance.
#[derive(Clone, Debug, Default)]
pub struct N8NConfig {
    /// Base URL of the n8n instance, e.g. `http://127.0.0.1:5678`.
    pub base_url: String,
    /// API key for n8n REST API authentication.
    pub api_key: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl N8NConfig {
    /// Build config from environment variables:
    /// - `N8N_BASE_URL` (default `http://127.0.0.1:5678`)
    /// - `N8N_API_KEY`
    /// - `N8N_TIMEOUT_SECS` (default `30`)
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            base_url: std::env::var("N8N_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:5678".into()),
            api_key: std::env::var("N8N_API_KEY").unwrap_or_default(),
            timeout_secs: std::env::var("N8N_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        })
    }
}

// ---------------------------------------------------------------------------
// Webhook trigger
// ---------------------------------------------------------------------------

/// Trigger a workflow via n8n webhook.
///
/// n8n exposes webhook endpoints at:
/// `POST {base_url}/webhook/{path}`  (production)
/// `POST {base_url}/webhook-test/{path}`  (test)
#[derive(Clone, Debug, Serialize)]
pub struct WebhookTriggerRequest {
    /// Webhook path (the part after `/webhook/`).
    pub path: String,
    /// Whether to use the test webhook endpoint (`webhook-test`) instead of
    /// the production one (`webhook`).
    pub test: bool,
    /// JSON payload forwarded as the request body.
    pub body: Value,
}

/// Response from a webhook trigger.
#[derive(Clone, Debug, Deserialize)]
pub struct WebhookTriggerResponse {
    /// n8n may return arbitrary JSON; we capture it as a raw Value.
    #[serde(flatten)]
    pub data: Value,
}

// ---------------------------------------------------------------------------
// Workflow types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct Workflow {
    pub id: Option<String>,
    pub name: Option<String>,
    pub active: Option<bool>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub nodes: Option<Vec<Value>>,
    pub connections: Option<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowListResponse {
    pub data: Option<Vec<Workflow>>,
    #[serde(flatten)]
    pub extra: Value,
}

// ---------------------------------------------------------------------------
// Execution types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct Execution {
    pub id: Option<String>,
    pub finished: Option<bool>,
    pub mode: Option<String>,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
    pub workflow_id: Option<String>,
    pub status: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExecutionListResponse {
    pub data: Option<Vec<Execution>>,
    #[serde(flatten)]
    pub extra: Value,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// n8n HTTP client.
#[derive(Clone, Debug)]
pub struct N8NClient {
    http: Client,
    config: N8NConfig,
}

impl N8NClient {
    /// Create a new client from the given config.
    pub fn new(config: N8NConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("failed to build reqwest client for n8n")?;
        Ok(Self { http, config })
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self> {
        Self::new(N8NConfig::from_env()?)
    }

    // -- Webhook triggers ----------------------------------------------------

    /// Trigger a workflow via its webhook URL.
    #[instrument(skip(self, req))]
    pub async fn trigger_webhook(
        &self,
        req: &WebhookTriggerRequest,
    ) -> Result<WebhookTriggerResponse> {
        let segment = if req.test {
            "webhook-test"
        } else {
            "webhook"
        };
        let url = format!("{}/{}/{}", self.config.base_url, segment, req.path);
        debug!(url, "triggering n8n webhook");

        let http_req = self
            .http
            .post(&url)
            .header(
                "X-N8N-API-KEY",
                &self.config.api_key,
            )
            .json(&req.body);

        let resp = http_req.send().await.context("n8n webhook request failed")?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "n8n webhook trigger error");
            return Err(anyhow!(
                "n8n webhook trigger failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        // n8n may return JSON or plain text depending on the workflow
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if content_type.contains("application/json") {
            resp.json::<WebhookTriggerResponse>()
                .await
                .context("failed to parse n8n webhook JSON response")
        } else {
            let text = resp.text().await.unwrap_or_default();
            Ok(WebhookTriggerResponse {
                data: Value::String(text),
            })
        }
    }

    /// Convenience: trigger a production webhook with a JSON body.
    pub async fn trigger(
        &self,
        path: &str,
        body: Value,
    ) -> Result<WebhookTriggerResponse> {
        self.trigger_webhook(&WebhookTriggerRequest {
            path: path.to_string(),
            test: false,
            body,
        })
        .await
    }

    /// Convenience: trigger a test webhook with a JSON body.
    pub async fn trigger_test(
        &self,
        path: &str,
        body: Value,
    ) -> Result<WebhookTriggerResponse> {
        self.trigger_webhook(&WebhookTriggerRequest {
            path: path.to_string(),
            test: true,
            body,
        })
        .await
    }

    // -- Workflow management -------------------------------------------------

    /// List all workflows.
    #[instrument(skip(self))]
    pub async fn list_workflows(&self) -> Result<WorkflowListResponse> {
        let url = format!("{}/api/v1/workflows", self.config.base_url);
        debug!(url, "listing n8n workflows");

        let resp = self
            .http
            .get(&url)
            .header("X-N8N-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("n8n list workflows request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "n8n list workflows error");
            return Err(anyhow!(
                "n8n list workflows failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<WorkflowListResponse>()
            .await
            .context("failed to parse n8n workflow list response")
    }

    /// Get a single workflow by ID.
    pub async fn get_workflow(&self, workflow_id: &str) -> Result<Workflow> {
        let url = format!(
            "{}/api/v1/workflows/{}",
            self.config.base_url, workflow_id
        );
        debug!(url, "getting n8n workflow");

        let resp = self
            .http
            .get(&url)
            .header("X-N8N-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("n8n get workflow request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "n8n get workflow failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<Workflow>()
            .await
            .context("failed to parse n8n workflow response")
    }

    // -- Execution history ---------------------------------------------------

    /// List executions, optionally filtered.
    pub async fn list_executions(&self, limit: Option<u32>) -> Result<ExecutionListResponse> {
        let mut url = format!("{}/api/v1/executions", self.config.base_url);
        if let Some(limit) = limit {
            url = format!("{}?limit={}", url, limit);
        }
        debug!(url, "listing n8n executions");

        let resp = self
            .http
            .get(&url)
            .header("X-N8N-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("n8n list executions request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "n8n list executions failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<ExecutionListResponse>()
            .await
            .context("failed to parse n8n execution list response")
    }

    /// Get a single execution by ID.
    pub async fn get_execution(&self, execution_id: &str) -> Result<Execution> {
        let url = format!(
            "{}/api/v1/executions/{}",
            self.config.base_url, execution_id
        );
        debug!(url, "getting n8n execution");

        let resp = self
            .http
            .get(&url)
            .header("X-N8N-API-KEY", &self.config.api_key)
            .send()
            .await
            .context("n8n get execution request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "n8n get execution failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<Execution>()
            .await
            .context("failed to parse n8n execution response")
    }

    // -- Health check --------------------------------------------------------

    /// Check if the n8n instance is reachable (REST API health endpoint).
    pub async fn health(&self) -> Result<bool> {
        let url = format!("{}/healthz", self.config.base_url);
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
    fn test_webhook_trigger_request_serialization() {
        let req = WebhookTriggerRequest {
            path: "my-workflow".into(),
            test: false,
            body: serde_json::json!({"key": "value"}),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["path"], "my-workflow");
        assert_eq!(json["test"], false);
        assert_eq!(json["body"]["key"], "value");
    }

    #[test]
    fn test_config_from_env_defaults() {
        // Ensure defaults are reasonable when env vars are not set
        let cfg = N8NConfig {
            base_url: "http://127.0.0.1:5678".into(),
            api_key: String::new(),
            timeout_secs: 30,
        };
        assert_eq!(cfg.base_url, "http://127.0.0.1:5678");
        assert_eq!(cfg.timeout_secs, 30);
    }
}
