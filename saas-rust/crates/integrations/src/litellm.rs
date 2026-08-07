//! LiteLLM API client — AI gateway for chat completions, embeddings, etc.
//!
//! Wraps the LiteLLM proxy HTTP API (OpenAI-compatible) so the rest of the
//! codebase can call any LLM through a single endpoint.

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for connecting to a LiteLLM proxy instance.
#[derive(Clone, Debug)]
pub struct LiteLLMConfig {
    /// Base URL of the LiteLLM proxy, e.g. `http://127.0.0.1:4000`.
    pub base_url: String,
    /// API key forwarded in the `Authorization: Bearer <key>` header.
    pub api_key: String,
    /// Default model to use when none is specified (e.g. `gemini-2.5-flash-lite`).
    pub default_model: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl LiteLLMConfig {
    /// Build config from environment variables:
    /// - `LITELLM_BASE_URL` (default `http://127.0.0.1:4000`)
    /// - `LITELLM_API_KEY`
    /// - `LITELLM_DEFAULT_MODEL` (default `gemini-2.5-flash-lite`)
    /// - `LITELLM_TIMEOUT_SECS` (default `120`)
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            base_url: std::env::var("LITELLM_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:4000".into()),
            api_key: std::env::var("LITELLM_API_KEY").unwrap_or_default(),
            default_model: std::env::var("LITELLM_DEFAULT_MODEL")
                .unwrap_or_else(|_| "gpt-5.6-luna".into()),
            timeout_secs: std::env::var("LITELLM_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120),
        })
    }
}

/// Join a base URL with an API path, tolerating base URLs that already carry
/// a trailing `/v1` (gateway style) as well as bare hosts.
fn join_endpoint(base_url: &str, path: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let cleaned_path = path.trim_start_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{}/{}", trimmed, cleaned_path)
    } else {
        format!("{}/v1/{}", trimmed, cleaned_path)
    }
}

// ---------------------------------------------------------------------------
// Request / Response types (OpenAI-compatible)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
}

/// Chat completion request (mirrors OpenAI `/v1/chat/completions`).
#[derive(Clone, Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    /// Arbitrary extra fields forwarded to the LiteLLM proxy
    /// (e.g. `metadata`, `stream`, `tools`, …).
    #[serde(flatten)]
    pub extra: Value,
    /// Per-request endpoint override (tenant-scoped base URL). Not serialized.
    #[serde(skip)]
    pub override_base_url: Option<String>,
    /// Per-request API key override (tenant-scoped). Not serialized.
    #[serde(skip)]
    pub override_api_key: Option<String>,
}

impl ChatCompletionRequest {
    /// Convenience builder for a simple text-only request.
    pub fn simple(model: &str, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.to_string(),
            messages,
            temperature: None,
            max_tokens: None,
            response_format: None,
            extra: Value::Null,
            override_base_url: None,
            override_api_key: None,
        }
    }

    /// Attach tenant-scoped endpoint credentials to this request.
    pub fn with_endpoint(mut self, base_url: Option<String>, api_key: Option<String>) -> Self {
        self.override_base_url = base_url;
        self.override_api_key = api_key;
        self
    }
}

/// Chat completion response (mirrors OpenAI).
#[derive(Clone, Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: Option<String>,
    pub object: Option<String>,
    pub created: Option<i64>,
    pub model: Option<String>,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Choice {
    pub index: Option<u32>,
    pub message: ChoiceMessage,
    pub finish_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChoiceMessage {
    pub role: Option<String>,
    pub content: Option<String>,
    /// Tool calls returned by the model (if any).
    pub tool_calls: Option<Vec<Value>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Embedding types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: EmbeddingInput,
}

/// Supports single string or array of strings for batch embedding.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Single(String),
    Batch(Vec<String>),
}

#[derive(Clone, Debug, Deserialize)]
pub struct EmbeddingResponse {
    pub object: Option<String>,
    pub data: Vec<EmbeddingData>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EmbeddingData {
    pub object: Option<String>,
    pub index: Option<u32>,
    pub embedding: Vec<f64>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// LiteLLM HTTP client.
#[derive(Clone, Debug)]
pub struct LiteLLMClient {
    http: Client,
    config: LiteLLMConfig,
}

impl LiteLLMClient {
    /// Create a new client from the given config.
    pub fn new(config: LiteLLMConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("failed to build reqwest client for LiteLLM")?;
        Ok(Self { http, config })
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self> {
        Self::new(LiteLLMConfig::from_env()?)
    }

    // -- Chat completions ---------------------------------------------------

    /// Send a chat completion request.
    ///
    /// If the request carries `override_base_url`/`override_api_key` (tenant
    /// scoped), those are used instead of the client-level config.
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn chat_completion(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let base_url = req
            .override_base_url
            .as_deref()
            .unwrap_or(&self.config.base_url);
        let api_key = req
            .override_api_key
            .as_deref()
            .unwrap_or(&self.config.api_key);
        let url = join_endpoint(base_url, "/chat/completions");
        debug!(url, "sending chat completion request");

        let resp = self
            .http
            .post(&url)
            .bearer_auth(api_key)
            .json(req)
            .send()
            .await
            .context("LiteLLM chat completion request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "LiteLLM chat completion error");
            return Err(anyhow!(
                "LiteLLM chat completion failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<ChatCompletionResponse>()
            .await
            .context("failed to parse LiteLLM chat completion response")
    }

    // -- Claude Messages format ---------------------------------------------

    /// Send a request using the Anthropic `/v1/messages` format.
    ///
    /// The gateway may expose a Claude-compatible endpoint; when `base_url`
    /// override is present it is used instead of the client-level config.
    #[instrument(skip(self), fields(model = %model))]
    pub async fn claude_messages(
        &self,
        model: &str,
        system: Option<&str>,
        messages: &[ChatMessage],
        max_tokens: u32,
        base_url_override: Option<String>,
        api_key_override: Option<String>,
    ) -> Result<Value> {
        let base_url = base_url_override.unwrap_or_else(|| self.config.base_url.clone());
        let api_key = api_key_override.unwrap_or_else(|| self.config.api_key.clone());
        let url = join_endpoint(&base_url, "/messages");
        debug!(url, "sending Claude messages request");

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
        });
        if let Some(system) = system {
            body["system"] = Value::String(system.to_string());
        }

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .context("Claude messages request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %err_body, "Claude messages error");
            return Err(anyhow!(
                "Claude messages failed (HTTP {}): {}",
                status,
                err_body.chars().take(500).collect::<String>()
            ));
        }
        resp.json::<Value>()
            .await
            .context("failed to parse Claude messages response")
    }

    /// Extract text content from a Claude `/v1/messages` response.
    pub fn claude_text(response: &Value) -> Option<String> {
        response
            .get("content")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("type")
                            .and_then(Value::as_str)
                            .filter(|t| *t == "text")
                            .and_then(|_| item.get("text").and_then(Value::as_str))
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .next()
            })
            .map(str::to_string)
    }

    // -- OpenAI Responses format --------------------------------------------

    /// Send a request using the OpenAI `/v1/responses` format.
    #[instrument(skip(self), fields(model = %model))]
    pub async fn responses(
        &self,
        model: &str,
        input: Value,
        base_url_override: Option<String>,
        api_key_override: Option<String>,
    ) -> Result<Value> {
        let base_url = base_url_override.unwrap_or_else(|| self.config.base_url.clone());
        let api_key = api_key_override.unwrap_or_else(|| self.config.api_key.clone());
        let url = join_endpoint(&base_url, "/responses");
        debug!(url, "sending OpenAI Responses request");

        let body = serde_json::json!({
            "model": model,
            "input": input,
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .context("OpenAI Responses request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %err_body, "OpenAI Responses error");
            return Err(anyhow!(
                "OpenAI Responses failed (HTTP {}): {}",
                status,
                err_body.chars().take(500).collect::<String>()
            ));
        }
        resp.json::<Value>()
            .await
            .context("failed to parse OpenAI Responses response")
    }

    /// Extract text content from an OpenAI `/v1/responses` response.
    pub fn responses_text(response: &Value) -> Option<String> {
        response
            .get("output")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("type")
                            .and_then(Value::as_str)
                            .filter(|t| *t == "message")
                            .and_then(|_| {
                                item.get("content").and_then(Value::as_array)
                            })
                            .and_then(|contents| {
                                contents
                                    .iter()
                                    .filter_map(|c| {
                                        c.get("type")
                                            .and_then(Value::as_str)
                                            .filter(|t| *t == "output_text")
                                            .and_then(|_| c.get("text").and_then(Value::as_str))
                                    })
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .next()
                            })
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .next()
            })
            .map(str::to_string)
    }

    /// Convenience: single-turn chat with the default model.
    pub async fn simple_chat(&self, system_prompt: &str, user_message: &str) -> Result<String> {        let req = ChatCompletionRequest::simple(
            &self.config.default_model,
            vec![
                ChatMessage {
                    role: "system".into(),
                    content: system_prompt.into(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: user_message.into(),
                },
            ],
        );
        let resp = self.chat_completion(&req).await?;
        let content = resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("LiteLLM returned no content"))?;
        Ok(content)
    }

    /// Multi-turn chat: send a full message history with the default model.
    pub async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        model: Option<&str>,
    ) -> Result<ChatCompletionResponse> {
        let req = ChatCompletionRequest {
            model: model
                .map(|m| m.to_string())
                .unwrap_or_else(|| self.config.default_model.clone()),
            messages,
            temperature: None,
            max_tokens: None,
            response_format: None,
            extra: Value::Null,
            override_base_url: None,
            override_api_key: None,
        };
        self.chat_completion(&req).await
    }

    // -- Embeddings ----------------------------------------------------------

    /// Request text embeddings.
    #[instrument(skip(self, req), fields(model = %req.model))]
    pub async fn embedding(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse> {
        let url = join_endpoint(&self.config.base_url, "/embeddings");
        debug!(url, "sending embedding request");

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(req)
            .send()
            .await
            .context("LiteLLM embedding request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "LiteLLM embedding error");
            return Err(anyhow!(
                "LiteLLM embedding failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        resp.json::<EmbeddingResponse>()
            .await
            .context("failed to parse LiteLLM embedding response")
    }

    /// Convenience: embed a single text string.
    pub async fn embed(&self, text: &str, model: Option<&str>) -> Result<Vec<f64>> {
        let model = model
            .map(|m| m.to_string())
            .unwrap_or_else(|| "text-embedding-3-small".into());
        let req = EmbeddingRequest {
            model,
            input: EmbeddingInput::Single(text.to_string()),
        };
        let resp = self.embedding(&req).await?;
        resp.data
            .first()
            .map(|d| d.embedding.clone())
            .ok_or_else(|| anyhow!("LiteLLM returned no embedding data"))
    }

    // -- Health check --------------------------------------------------------

    /// Check if the LiteLLM proxy is reachable.
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
    fn test_chat_completion_request_serialization() {
        let req = ChatCompletionRequest::simple(
            "gpt-4",
            vec![ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
        );
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "gpt-4");
        assert_eq!(json["messages"][0]["role"], "user");
        // extra is Null and should not appear
        assert!(json.get("temperature").is_none());
    }

    #[test]
    fn test_embedding_input_serialization() {
        let single = EmbeddingInput::Single("hello".into());
        let v = serde_json::to_value(&single).unwrap();
        assert_eq!(v, Value::String("hello".into()));

        let batch = EmbeddingInput::Batch(vec!["a".into(), "b".into()]);
        let v = serde_json::to_value(&batch).unwrap();
        assert!(v.is_array());
    }
}
