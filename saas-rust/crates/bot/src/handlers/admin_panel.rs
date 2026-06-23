//! Admin Panel handler – migrated from handlers/admin_panel.py
//!
//! Provides the admin configuration panel for managing translation engines,
//! LLM model selection, API key verification, and system settings.

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode, MessageId};
use teloxide::payloads::SendMessageSetters;

use crate::handlers::HandlerResult;

// ---------------------------------------------------------------------------
// 2026 Model Matrix (mirrors Python MODEL_MATRIX)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub base_url: &'static str,
    pub models: &'static [&'static str],
}

pub static MODEL_MATRIX: &[(&str, ProviderInfo)] = &[
    (
        "DeepSeek",
        ProviderInfo {
            base_url: "https://api.deepseek.com/v1",
            models: &["deepseek-chat", "deepseek-reasoner"],
        },
    ),
    (
        "Gemini",
        ProviderInfo {
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
            models: &["gemini-3.1-pro", "gemini-3.1-flash"],
        },
    ),
    (
        "OpenAI",
        ProviderInfo {
            base_url: "https://api.openai.com/v1",
            models: &["gpt-5.3-pro", "gpt-5.3-mini"],
        },
    ),
];

fn find_provider(name: &str) -> Option<&'static ProviderInfo> {
    MODEL_MATRIX
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, info)| info)
}

// ---------------------------------------------------------------------------
// Conversation states (mirrors Python WAITING_*)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminState {
    WaitingCustomModel,
    WaitingToken,
    WaitingSystemPrompt,
}

// ---------------------------------------------------------------------------
// Admin panel main display
// ---------------------------------------------------------------------------

/// Build and send the admin panel main menu.
/// Returns the next conversation state (normally None = end).
pub async fn admin_panel(
    bot: &Bot,
    chat_id: ChatId,
    message_id: Option<i32>,
    admin_id: i64,
    db: &sqlx::PgPool,
    user_data: &mut AdminUserData,
) -> HandlerResult<Option<AdminState>> {
    // Authorization check
    let caller_id = chat_id.0;
    if caller_id != admin_id {
        bot.send_message(chat_id, "⛔ 仅管理员可使用此命令。")
            .await?;
        return Ok(None);
    }

    // Concurrently read DB settings
    let engine = get_admin_setting(db, "translation_engine", "llm").await;
    let model = get_admin_setting(db, "llm_model", "gpt-3.5-turbo").await;
    let ocr_str = get_admin_setting(db, "ocr_force_enabled", "true").await;
    let ocr_enabled = ocr_str.to_lowercase() == "true";

    let engine_text = if engine == "llm" {
        "🤖 大模型"
    } else {
        "🌍 谷歌翻译"
    };
    let toggle_text = if engine == "llm" {
        "🔄 切换为: 🌍 谷歌"
    } else {
        "🔄 切换为: 🤖 大模型"
    };

    let text = format!(
        "🚀 <b>Admin Control Center 2026</b>\n\n\
         🔸 <b>当前引擎</b>: <code>{engine_text}</code>\n\
         🔸 <b>当前模型</b>: <code>{model}</code>\n\
         🔸 <b>安全核验</b>: <code>{}</code>\n\n\
         请选择操作：",
        if ocr_enabled { "开启 ✅" } else { "关闭 ❌" }
    );

    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(toggle_text, "adm:toggle_eng")],
        vec![InlineKeyboardButton::callback(
            "🧠 2026 模型矩阵",
            "adm:providers",
        )],
        vec![InlineKeyboardButton::callback(
            "⌨️ 手动输入模型名称",
            "adm:custom_mod",
        )],
        vec![InlineKeyboardButton::callback(
            "📝 翻译提示词",
            "adm:prompt",
        )],
        vec![InlineKeyboardButton::callback(
            "🔄 刷新潜在用户",
            "adm:sync_potential",
        )],
        vec![InlineKeyboardButton::callback(
            if ocr_enabled {
                "🛡️ OCR 强核验开关"
            } else {
                "🔓 OCR 强核验开关"
            },
            "adm:ocr_toggle",
        )],
        vec![
            InlineKeyboardButton::callback("🧹 清理缓存", "adm:clear_cache"),
            InlineKeyboardButton::callback("🔙 返回", "back_to_start"),
        ],
    ]);

    if let Some(mid) = message_id {
        bot.edit_message_text(chat_id, MessageId(mid), &text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    } else {
        bot.send_message(chat_id, &text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Callback dispatcher
// ---------------------------------------------------------------------------

/// Handle admin panel callback queries.
/// Returns Some(state) if the conversation should enter a waiting state,
/// otherwise None.
pub async fn handle_admin_callback(
    bot: &Bot,
    query: CallbackQuery,
    admin_id: i64,
    db: &sqlx::PgPool,
    user_data: &mut AdminUserData,
    web_base_url: &str,
    web_admin_token: &str,
) -> HandlerResult<Option<AdminState>> {
    let chat_id = match query.message.as_ref() {
        Some(msg) => ChatId(msg.chat().id.0),
        None => return Ok(None),
    };
    let message_id = query.message.as_ref().map(|m| m.id().0 as i32);

    // Authorization
    let from_id = query.from.id.0 as i64;
    if from_id != admin_id {
        bot.answer_callback_query(&query.id)
            .text("⛔ 仅管理员可使用此命令。")
            .show_alert(true)
            .await?;
        return Ok(None);
    }

    let data = match &query.data {
        Some(d) => d.clone(),
        None => return Ok(None),
    };

    // Answer the callback immediately
    bot.answer_callback_query(&query.id).await?;

    match data.as_str() {
        "adm:toggle_eng" => {
            let current = get_admin_setting(db, "translation_engine", "llm").await;
            let new_eng = if current == "llm" { "google" } else { "llm" };
            save_admin_setting(db, "translation_engine", &new_eng).await;
            admin_panel(bot, chat_id, message_id, admin_id, db, user_data).await?;
        }
        "adm:providers" => {
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("DeepSeek", "adm:pv:DeepSeek"),
                    InlineKeyboardButton::callback("Gemini", "adm:pv:Gemini"),
                ],
                vec![InlineKeyboardButton::callback("OpenAI", "adm:pv:OpenAI")],
                vec![InlineKeyboardButton::callback("🔙 返回", "adm:main")],
            ]);
            if let Some(mid) = message_id {
                bot.edit_message_text(chat_id, MessageId(mid), "请选择 AI 厂商 (2026 规格)：")
                    .reply_markup(keyboard)
                    .await?;
            }
        }
        d if d.starts_with("adm:pv:") => {
            let provider = d.strip_prefix("adm:pv:").unwrap_or("");
            user_data.pending_provider = Some(provider.to_string());

            let models = find_provider(provider)
                .map(|p| p.models)
                .unwrap_or(&[]);

            let mut btns: Vec<Vec<InlineKeyboardButton>> = models
                .iter()
                .map(|m| {
                    vec![InlineKeyboardButton::callback(
                        m.to_string(),
                        format!("adm:set_mod:{m}"),
                    )]
                })
                .collect();
            btns.push(vec![InlineKeyboardButton::callback(
                "🔙 返回",
                "adm:providers",
            )]);

            if let Some(mid) = message_id {
                bot.edit_message_text(
                    chat_id,
                    MessageId(mid),
                    format!("选择 {provider} 旗舰型号："),
                )
                .reply_markup(InlineKeyboardMarkup::new(btns))
                .await?;
            }
        }
        d if d.starts_with("adm:set_mod:") => {
            let model_code = d.strip_prefix("adm:set_mod:").unwrap_or("");
            let provider = user_data
                .pending_provider
                .as_deref()
                .unwrap_or("OpenAI");
            let base_url = find_provider(provider)
                .map(|p| p.base_url)
                .unwrap_or("https://api.openai.com/v1");

            save_admin_setting(db, "llm_model", model_code).await;
            save_admin_setting(db, "llm_base_url", base_url).await;
            user_data.pending_model = Some(model_code.to_string());
            user_data.pending_base_url = Some(base_url.to_string());

            if let Some(mid) = message_id {
                bot.edit_message_text(
                    chat_id,
                    MessageId(mid),
                    format!(
                        "✅ 已锁定: <code>{model_code}</code>\n\
                         请输入 <b>API Key</b> 进行验证 (输入 /skip 保持原样)："
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
            return Ok(Some(AdminState::WaitingToken));
        }
        "adm:custom_mod" => {
            if let Some(mid) = message_id {
                bot.edit_message_text(
                    chat_id,
                    MessageId(mid),
                    "⌨️ <b>自定义模型接入</b>\n\n请输入完整的模型名称 (例如: <code>gpt-6.0</code>)：",
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
            return Ok(Some(AdminState::WaitingCustomModel));
        }
        "adm:prompt" => {
            let curr = get_admin_setting(db, "system_prompt", "Professional Translator").await;
            if let Some(mid) = message_id {
                bot.edit_message_text(
                    chat_id,
                    MessageId(mid),
                    format!(
                        "📝 <b>翻译补充提示词</b>\n\
                         当前: <code>{curr}</code>\n\n\
                         该段会<b>追加</b>在系统固定规则之后（含译文格式与术语注解），用于微调语气或业务偏好。\n\
                         请输入新内容："
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
            return Ok(Some(AdminState::WaitingSystemPrompt));
        }
        "adm:sync_potential" => {
            let result = sync_potential_users(web_base_url, web_admin_token).await;
            match result {
                Ok(summary) => {
                    bot.answer_callback_query(&query.id)
                        .text(&format!("✅ {summary}"))
                        .show_alert(true)
                        .await?;
                }
                Err(e) => {
                    bot.answer_callback_query(&query.id)
                        .text(&format!("❌ 同步失败: {e}"))
                        .show_alert(true)
                        .await?;
                }
            }
            admin_panel(bot, chat_id, message_id, admin_id, db, user_data).await?;
        }
        "adm:ocr_toggle" => {
            let ocr_str = get_admin_setting(db, "ocr_force_enabled", "true").await;
            let curr = ocr_str.to_lowercase() == "true";
            save_admin_setting(db, "ocr_force_enabled", &(!curr).to_string()).await;
            admin_panel(bot, chat_id, message_id, admin_id, db, user_data).await?;
        }
        "adm:clear_cache" => {
            user_data.clear();
            bot.answer_callback_query(&query.id)
                .text("🧹 缓存已清理")
                .await?;
            admin_panel(bot, chat_id, message_id, admin_id, db, user_data).await?;
        }
        "adm:main" => {
            admin_panel(bot, chat_id, message_id, admin_id, db, user_data).await?;
        }
        _ => {}
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Text message handlers for conversation states
// ---------------------------------------------------------------------------

/// Handle custom model name input.
pub async fn save_custom_model(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    db: &sqlx::PgPool,
    user_data: &mut AdminUserData,
) -> HandlerResult<AdminState> {
    let model = text.trim();
    save_admin_setting(db, "llm_model", model).await;
    user_data.pending_model = Some(model.to_string());

    bot.send_message(
        chat_id,
        format!(
            "✅ 模型名称已更新为: <code>{model}</code>\n请输入 API Key 或发送 /skip："
        ),
    )
    .parse_mode(ParseMode::Html)
    .await?;

    Ok(AdminState::WaitingToken)
}

/// Handle API key / token input.
pub async fn save_token(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    db: &sqlx::PgPool,
    user_data: &mut AdminUserData,
) -> HandlerResult<Option<AdminState>> {
    let token = text.trim();
    let model = user_data
        .pending_model
        .as_deref()
        .unwrap_or("Unknown");

    if token == "/skip" {
        bot.send_message(chat_id, "✅ 配置已保持。").await?;
    } else {
        let base_url = user_data
            .pending_base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1");

        let status_msg = bot
            .send_message(chat_id, "⏳ 正在验证 2026 规格链路...")
            .await?;

        // Attempt to verify token via a lightweight request
        let verified = verify_token(token, model, base_url).await;

        if verified {
            let key_field = if model.to_lowercase().contains("deepseek") {
                "llm_api_key_deepseek"
            } else if model.to_lowercase().contains("gemini") {
                "llm_api_key_gemini"
            } else {
                "llm_api_key_generic"
            };
            save_admin_setting(db, key_field, token).await;
            bot.edit_message_text(
                chat_id,
                MessageId(status_msg.id.0 as i32),
                format!("✅ 验证成功！{model} 已激活。"),
            )
            .await?;
        } else {
            bot.edit_message_text(
                chat_id,
                MessageId(status_msg.id.0 as i32),
                "❌ 验证失败！请检查 Token 或网络。",
            )
            .await?;
        }

        // Clean up status message after a short delay
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        let _ = bot.delete_message(chat_id, status_msg.id).await;
    }

    user_data.pending_model = None;
    user_data.pending_base_url = None;
    user_data.pending_provider = None;

    Ok(None)
}

/// Handle system prompt input.
pub async fn save_prompt(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    db: &sqlx::PgPool,
) -> HandlerResult<Option<AdminState>> {
    save_admin_setting(db, "system_prompt", text.trim()).await;
    let msg = bot.send_message(chat_id, "✅ 提示词已更新。").await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let _ = bot.delete_message(chat_id, msg.id).await;
    Ok(None)
}

// ---------------------------------------------------------------------------
// Admin user data (per-user state, replaces context.user_data)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct AdminUserData {
    pub pending_provider: Option<String>,
    pub pending_model: Option<String>,
    pub pending_base_url: Option<String>,
}

impl AdminUserData {
    pub fn clear(&mut self) {
        self.pending_provider = None;
        self.pending_model = None;
        self.pending_base_url = None;
    }
}

// ---------------------------------------------------------------------------
// DB helpers (wrapping admin_settings table access)
// ---------------------------------------------------------------------------

/// Get an admin setting from the database, with a default value.
pub async fn get_admin_setting(pool: &sqlx::PgPool, key: &str, default: &str) -> String {
    let result: Option<String> = sqlx::query_scalar(
        "SELECT value FROM admin_settings WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten();

    result.unwrap_or_else(|| default.to_string())
}

/// Save an admin setting to the database.
pub async fn save_admin_setting(pool: &sqlx::PgPool, key: &str, value: &str) {
    let _ = sqlx::query(
        r#"
        INSERT INTO admin_settings (key, value, updated_at)
        VALUES ($1, $2, now())
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()
        "#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await;
}

// ---------------------------------------------------------------------------
// External integration: sync potential users
// ---------------------------------------------------------------------------

async fn sync_potential_users(web_base_url: &str, web_admin_token: &str) -> anyhow::Result<String> {
    let url = format!(
        "{}/api/potential_users/sync?token={}",
        web_base_url, web_admin_token
    );
    let client = reqwest::Client::new();
    let resp = client.post(&url).send().await?;

    if resp.status().is_success() {
        let body: serde_json::Value = resp.json().await?;
        let summary = body
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("同步完成");
        Ok(summary.to_string())
    } else {
        anyhow::bail!("同步接口响应异常")
    }
}

// ---------------------------------------------------------------------------
// Token verification (lightweight HTTP call to LLM provider)
// ---------------------------------------------------------------------------

async fn verify_token(token: &str, model: &str, base_url: &str) -> bool {
    let client = reqwest::Client::new();
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await;

    match resp {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}
