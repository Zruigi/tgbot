//! Telegram Bot API client — typed wrapper around the Bot API.
//!
//! Provides convenient async methods for common Bot API operations while
//! preserving low-level access to any endpoint via `request`.

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the Telegram Bot API.
#[derive(Clone, Debug)]
pub struct TelegramConfig {
    /// Bot API token (obtained from @BotFather).
    pub bot_token: String,
    /// Base URL of the Bot API (use `https://api.telegram.org` for production,
    /// or a local proxy for development).
    pub api_base_url: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl TelegramConfig {
    /// Build config from environment variables:
    /// - `BOT_TOKEN` (required)
    /// - `TELEGRAM_API_BASE_URL` (default `https://api.telegram.org`)
    /// - `TELEGRAM_TIMEOUT_SECS` (default `30`)
    pub fn from_env() -> Result<Self> {
        let bot_token = std::env::var("BOT_TOKEN").unwrap_or_default();
        if bot_token.is_empty() {
            return Err(anyhow!("BOT_TOKEN environment variable is required"));
        }
        Ok(Self {
            bot_token,
            api_base_url: std::env::var("TELEGRAM_API_BASE_URL")
                .unwrap_or_else(|_| "https://api.telegram.org".into()),
            timeout_secs: std::env::var("TELEGRAM_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        })
    }
}

// ---------------------------------------------------------------------------
// Common types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub edited_message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub from: Option<User>,
    pub chat: Chat,
    pub date: i64,
    pub text: Option<String>,
    pub photo: Option<Vec<PhotoSize>>,
    pub document: Option<Document>,
    pub caption: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
    pub language_code: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PhotoSize {
    pub file_id: String,
    pub width: i32,
    pub height: i32,
    pub file_size: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub file_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub thumb: Option<PhotoSize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<Message>,
    pub data: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct File {
    pub file_id: String,
    pub file_unique_id: String,
    pub file_size: Option<i64>,
    pub file_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Bot API method request/response types
// ---------------------------------------------------------------------------

/// Generic request wrapper for Bot API calls.
#[derive(Clone, Debug, Serialize)]
pub struct BotRequest<M: Serialize> {
    #[serde(flatten)]
    pub method: M,
}

/// Generic Bot API response wrapper.
#[derive(Clone, Debug, Deserialize)]
pub struct BotResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub error_code: Option<i32>,
    pub description: Option<String>,
}

/// Parameters for `sendMessage`.
#[derive(Clone, Debug, Serialize)]
pub struct SendMessageParams {
    pub chat_id: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
}

/// Parameters for `sendPhoto`.
#[derive(Clone, Debug, Serialize)]
pub struct SendPhotoParams {
    pub chat_id: i64,
    pub photo: String, // file_id or URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
}

/// Parameters for `sendDocument`.
#[derive(Clone, Debug, Serialize)]
pub struct SendDocumentParams {
    pub chat_id: i64,
    pub document: String, // file_id or URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
}

/// Parameters for `answerCallbackQuery`.
#[derive(Clone, Debug, Serialize)]
pub struct AnswerCallbackQueryParams {
    pub callback_query_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_alert: Option<bool>,
}

/// Parameters for `getFile`.
#[derive(Clone, Debug, Serialize)]
pub struct GetFileParams {
    pub file_id: String,
}

/// Parameters for `getUpdates`.
#[derive(Clone, Debug, Serialize)]
pub struct GetUpdatesParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i32>,
}

/// Parameters for `setWebhook`.
#[derive(Clone, Debug, Serialize)]
pub struct SetWebhookParams {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_updates: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Telegram Bot API HTTP client.
#[derive(Clone, Debug)]
pub struct TelegramClient {
    http: Client,
    config: TelegramConfig,
}

impl TelegramClient {
    /// Create a new client from the given config.
    pub fn new(config: TelegramConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("failed to build reqwest client for Telegram")?;
        Ok(Self { http, config })
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Result<Self> {
        Self::new(TelegramConfig::from_env()?)
    }

    // -- Low-level request ---------------------------------------------------

    /// Make a raw Bot API request.
    ///
    /// Example: `client.request::<Value>("getMe", Value::Null).await`
    #[instrument(skip(self), fields(method = %method))]
    pub async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: impl Serialize + std::fmt::Debug,
    ) -> Result<T> {
        let url = format!(
            "{}/bot{}/{}",
            self.config.api_base_url, self.config.bot_token, method
        );
        debug!(url, "calling Telegram Bot API");

        let resp = self
            .http
            .post(&url)
            .json(&params)
            .send()
            .await
            .context("Telegram API request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body, "Telegram API error");
            return Err(anyhow!(
                "Telegram API request failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let bot_resp: BotResponse<T> = resp
            .json()
            .await
            .context("failed to parse Telegram API response")?;

        if !bot_resp.ok {
            return Err(anyhow!(
                "Telegram API error (code {}): {}",
                bot_resp.error_code.unwrap_or(0),
                bot_resp.description.unwrap_or_else(|| "unknown".into())
            ));
        }

        bot_resp
            .result
            .ok_or_else(|| anyhow!("Telegram API returned no result"))
    }

    // -- Convenience methods -------------------------------------------------

    /// Get basic information about the bot.
    pub async fn get_me(&self) -> Result<User> {
        #[derive(Serialize)]
        struct Empty {}
        self.request("getMe", Empty {}).await
    }

    /// Get updates from the bot.
    pub async fn get_updates(&self, params: GetUpdatesParams) -> Result<Vec<Update>> {
        self.request("getUpdates", params).await
    }

    /// Send a text message.
    pub async fn send_message(&self, params: SendMessageParams) -> Result<Message> {
        self.request("sendMessage", params).await
    }

    /// Send a photo.
    pub async fn send_photo(&self, params: SendPhotoParams) -> Result<Message> {
        self.request("sendPhoto", params).await
    }

    /// Send a document.
    pub async fn send_document(&self, params: SendDocumentParams) -> Result<Message> {
        self.request("sendDocument", params).await
    }

    /// Answer a callback query (dismiss the loading animation).
    pub async fn answer_callback_query(&self, params: AnswerCallbackQueryParams) -> Result<()> {
        #[derive(Serialize)]
        struct Empty {}
        self.request("answerCallbackQuery", params)
            .await
            .map(|_: Value| ())
    }

    /// Get file information (for downloading).
    pub async fn get_file(&self, params: GetFileParams) -> Result<File> {
        self.request("getFile", params).await
    }

    /// Download a file to local storage.
    pub async fn download_file(&self, file: &File, dest_path: &str) -> Result<()> {
        let file_path = file
            .file_path
            .as_ref()
            .ok_or_else(|| anyhow!("File path not available"))?;

        let url = format!(
            "{}/file/bot{}/{}",
            self.config.api_base_url, self.config.bot_token, file_path
        );
        debug!(url, "downloading file from Telegram");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Telegram file download request failed")?;

        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!(
                "Telegram file download failed (HTTP {})",
                status
            ));
        }

        let bytes = resp
            .bytes()
            .await
            .context("failed to read file content")?;

        std::fs::write(dest_path, bytes).context("failed to write file to disk")?;
        Ok(())
    }

    /// Set a webhook.
    pub async fn set_webhook(&self, params: SetWebhookParams) -> Result<()> {
        #[derive(Deserialize)]
        struct WebhookResponse {
            ok: bool,
            result: bool,
            description: Option<String>,
        }
        let resp: WebhookResponse = self.request("setWebhook", params).await?;
        if resp.result {
            Ok(())
        } else {
            Err(anyhow!(
                "setWebhook failed: {}",
                resp.description.unwrap_or_else(|| "unknown".into())
            ))
        }
    }

    /// Delete the webhook.
    pub async fn delete_webhook(&self) -> Result<()> {
        #[derive(Serialize)]
        struct Empty {}
        #[derive(Deserialize)]
        struct WebhookResponse {
            ok: bool,
            result: bool,
        }
        let resp: WebhookResponse = self.request("deleteWebhook", Empty {}).await?;
        if resp.result {
            Ok(())
        } else {
            Err(anyhow!("deleteWebhook failed"))
        }
    }

    // -- Health check --------------------------------------------------------

    /// Check if the Telegram API is reachable (via `getMe`).
    pub async fn health(&self) -> Result<bool> {
        match self.get_me().await {
            Ok(_) => Ok(true),
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
    fn test_config_requires_bot_token() {
        // Reset env to test error handling
        std::env::remove_var("BOT_TOKEN");
        let cfg = TelegramConfig::from_env();
        assert!(cfg.is_err());
    }

    #[test]
    fn test_send_message_params_serialization() {
        let params = SendMessageParams {
            chat_id: 12345,
            text: "Hello".into(),
            parse_mode: Some("Markdown".into()),
            reply_markup: None,
            reply_to_message_id: None,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["chat_id"], 12345);
        assert_eq!(json["text"], "Hello");
        assert_eq!(json["parse_mode"], "Markdown");
    }
}