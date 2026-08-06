//! Telegram Bot API client — typed wrapper around the Bot API.
//!
//! Provides convenient async methods for common Bot API operations while
//! preserving low-level access to any endpoint via `request`.

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{instrument, warn};

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

#[derive(Clone, Debug, Serialize)]
pub struct EditMessageTextParams {
    pub chat_id: i64,
    pub message_id: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<Value>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<Value>,
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

/// One entry of `setMyCommands` (BotCommand).
#[derive(Clone, Debug, Serialize)]
pub struct BotCommandItem {
    pub command: String,
    pub description: String,
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
        let resp = self
            .http
            .post(&url)
            .json(&params)
            .send()
            .await
            .map_err(|_| anyhow!("Telegram API {method} request failed"))?;

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
        #[derive(Serialize, Debug)]
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

    pub async fn edit_message_text(&self, params: EditMessageTextParams) -> Result<Message> {
        self.request("editMessageText", params).await
    }

    /// Send a photo.
    pub async fn send_photo(&self, params: SendPhotoParams) -> Result<Message> {
        self.request("sendPhoto", params).await
    }

    pub async fn get_chat(&self, chat_id: i64) -> Result<Value> {
        self.request("getChat", json!({ "chat_id": chat_id })).await
    }

    pub async fn get_chat_member(&self, chat_id: i64, user_id: i64) -> Result<Value> {
        self.request(
            "getChatMember",
            json!({ "chat_id": chat_id, "user_id": user_id }),
        )
        .await
    }

    pub async fn get_chat_member_count(&self, chat_id: i64) -> Result<i64> {
        self.request("getChatMemberCount", json!({ "chat_id": chat_id }))
            .await
    }

    pub async fn get_chat_administrators(&self, chat_id: i64) -> Result<Vec<Value>> {
        self.request("getChatAdministrators", json!({ "chat_id": chat_id }))
            .await
    }

    /// Forward a message to another chat (used e.g. for message-existence probes).
    pub async fn forward_message(
        &self,
        chat_id: i64,
        from_chat_id: i64,
        message_id: i64,
    ) -> Result<Message> {
        self.request(
            "forwardMessage",
            json!({
                "chat_id": chat_id,
                "from_chat_id": from_chat_id,
                "message_id": message_id,
            }),
        )
        .await
    }

    pub async fn ban_chat_member(&self, chat_id: i64, user_id: i64) -> Result<bool> {
        self.request(
            "banChatMember",
            json!({ "chat_id": chat_id, "user_id": user_id }),
        )
        .await
    }

    pub async fn unban_chat_member(&self, chat_id: i64, user_id: i64) -> Result<bool> {
        self.request(
            "unbanChatMember",
            json!({
                "chat_id": chat_id,
                "user_id": user_id,
                "only_if_banned": true,
            }),
        )
        .await
    }

    pub async fn restrict_chat_member(
        &self,
        chat_id: i64,
        user_id: i64,
        permissions: Value,
        until_date: Option<i64>,
    ) -> Result<bool> {
        let mut params = json!({
            "chat_id": chat_id,
            "user_id": user_id,
            "permissions": permissions,
        });
        if let Some(until_date) = until_date {
            params["until_date"] = until_date.into();
        }
        self.request("restrictChatMember", params).await
    }

    pub async fn promote_chat_member(
        &self,
        chat_id: i64,
        user_id: i64,
        permissions: Value,
    ) -> Result<bool> {
        let mut params = permissions.as_object().cloned().unwrap_or_default();
        params.insert("chat_id".into(), chat_id.into());
        params.insert("user_id".into(), user_id.into());
        self.request("promoteChatMember", Value::Object(params))
            .await
    }

    pub async fn delete_message(&self, chat_id: i64, message_id: i64) -> Result<bool> {
        self.request(
            "deleteMessage",
            json!({ "chat_id": chat_id, "message_id": message_id }),
        )
        .await
    }

    pub async fn send_photo_bytes(
        &self,
        chat_id: i64,
        bytes: Vec<u8>,
        filename: &str,
        mime_type: &str,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        reply_markup: Option<&Value>,
    ) -> Result<Message> {
        let url = format!(
            "{}/bot{}/sendPhoto",
            self.config.api_base_url, self.config.bot_token
        );
        let photo = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(mime_type)
            .context("invalid photo MIME type")?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", photo);
        if let Some(caption) = caption.filter(|value| !value.is_empty()) {
            form = form.text("caption", caption.to_string());
        }
        if let Some(reply_to_message_id) = reply_to_message_id {
            form = form.text("reply_to_message_id", reply_to_message_id.to_string());
        }
        if let Some(reply_markup) = reply_markup {
            form = form.text("reply_markup", reply_markup.to_string());
        }

        let response = self
            .http
            .post(url)
            .multipart(form)
            .send()
            .await
            .map_err(|_| anyhow!("Telegram photo upload failed"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Telegram photo upload failed (HTTP {}): {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }
        let response: BotResponse<Message> = response
            .json()
            .await
            .context("failed to parse Telegram photo response")?;
        if !response.ok {
            return Err(anyhow!(
                "Telegram API error (code {}): {}",
                response.error_code.unwrap_or(0),
                response.description.unwrap_or_else(|| "unknown".into())
            ));
        }
        response
            .result
            .ok_or_else(|| anyhow!("Telegram API returned no result"))
    }

    /// Send a document.
    pub async fn send_document(&self, params: SendDocumentParams) -> Result<Message> {
        self.request("sendDocument", params).await
    }

    /// Answer a callback query (dismiss the loading animation).
    pub async fn answer_callback_query(&self, params: AnswerCallbackQueryParams) -> Result<()> {
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
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|_| anyhow!("Telegram file download request failed"))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("Telegram file download failed (HTTP {})", status));
        }

        let bytes = resp.bytes().await.context("failed to read file content")?;

        std::fs::write(dest_path, bytes).context("failed to write file to disk")?;
        Ok(())
    }

    pub async fn download_file_bytes(&self, file: &File, max_bytes: usize) -> Result<Vec<u8>> {
        let file_path = file
            .file_path
            .as_ref()
            .ok_or_else(|| anyhow!("File path not available"))?;
        if file
            .file_size
            .is_some_and(|size| size < 0 || size as usize > max_bytes)
        {
            return Err(anyhow!("Telegram file exceeds download limit"));
        }
        let url = format!(
            "{}/file/bot{}/{}",
            self.config.api_base_url, self.config.bot_token, file_path
        );
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| anyhow!("Telegram file download request failed"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(anyhow!("Telegram file download failed (HTTP {})", status));
        }
        if response
            .content_length()
            .is_some_and(|size| size > max_bytes as u64)
        {
            return Err(anyhow!("Telegram file exceeds download limit"));
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("failed to read Telegram file content")?
        {
            if bytes.len() + chunk.len() > max_bytes {
                return Err(anyhow!("Telegram file exceeds download limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// Set a webhook.
    pub async fn set_webhook(&self, params: SetWebhookParams) -> Result<()> {
        let result: bool = self.request("setWebhook", params).await?;
        if result {
            Ok(())
        } else {
            Err(anyhow!("setWebhook returned false"))
        }
    }

    /// Delete the webhook.
    pub async fn delete_webhook(&self) -> Result<()> {
        #[derive(Serialize, Debug)]
        struct Empty {}
        let result: bool = self.request("deleteWebhook", Empty {}).await?;
        if result {
            Ok(())
        } else {
            Err(anyhow!("deleteWebhook failed"))
        }
    }

    /// Set the bot command menu (`setMyCommands`).
    ///
    /// `commands` is a list of `{ command, description }` objects; `scope`
    /// is the optional `BotCommandScope` JSON object (e.g. `{"type":"chat",
    /// "chat_id": 123}`). When `scope` is `None` the default (all chats)
    /// scope is used.
    pub async fn set_my_commands(
        &self,
        commands: Vec<BotCommandItem>,
        scope: Option<Value>,
    ) -> Result<()> {
        let mut params = json!({ "commands": commands });
        if let Some(scope) = scope {
            params["scope"] = scope;
        }
        let result: bool = self.request("setMyCommands", params).await?;
        if result {
            Ok(())
        } else {
            Err(anyhow!("setMyCommands returned false"))
        }
    }

    /// Set the menu button (`setChatMenuButton`).
    ///
    /// `chat_id` scopes the button to one chat when provided; `menu_button`
    /// is the `MenuButton` JSON object (e.g. `{"type":"web_app",
    /// "text":"控制台","web_app":{"url": "https://…/miniapp?entry=admin_console"}}`).
    pub async fn set_chat_menu_button(
        &self,
        chat_id: Option<i64>,
        menu_button: Value,
    ) -> Result<()> {
        let mut params = json!({ "menu_button": menu_button });
        if let Some(chat_id) = chat_id {
            params["chat_id"] = chat_id.into();
        }
        let result: bool = self.request("setChatMenuButton", params).await?;
        if result {
            Ok(())
        } else {
            Err(anyhow!("setChatMenuButton returned false"))
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

    #[test]
    fn test_edit_message_text_params_serialization() {
        let params = EditMessageTextParams {
            chat_id: 12345,
            message_id: 678,
            text: "Updated".into(),
            parse_mode: None,
            reply_markup: Some(json!({
                "inline_keyboard": [[{
                    "text": "Refresh",
                    "callback_data": "wallet_my_list"
                }]]
            })),
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["chat_id"], 12345);
        assert_eq!(json["message_id"], 678);
        assert_eq!(json["text"], "Updated");
        assert_eq!(
            json["reply_markup"]["inline_keyboard"][0][0]["callback_data"],
            "wallet_my_list"
        );
    }
}
