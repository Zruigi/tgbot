// 消息群发与内容分发工具
// 对应 Python: services/ai_agent/tools.py :: BroadcastTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tgbot_integrations::{
    litellm::LiteLLMClient,
    telegram::{SendMessageParams, SendPhotoParams, TelegramClient},
};
use tracing::{error, info, warn};

const MAX_BROADCAST_TARGETS: usize = 500;
const MAX_BROADCAST_TEXT_CHARS: usize = 4096;
const MAX_IMAGE_SOURCE_CHARS: usize = 2048;

// ---------------------------------------------------------------------------
// BroadcastMessageTool
// ---------------------------------------------------------------------------

/// 消息群发与内容分发工具
///
/// 支持：
/// - 向用户/群组/频道发送消息
/// - 纯文本、带图片、自定义按钮
/// - 目标类型：users / groups / all
pub struct BroadcastMessageTool {
    pool: Option<Arc<PgPool>>,
    telegram_client: Option<Arc<TelegramClient>>,
    llm: Option<Arc<LiteLLMClient>>,
}

impl BroadcastMessageTool {
    /// Create a new tool without Telegram client (for testing)
    pub fn new() -> Self {
        Self {
            pool: None,
            telegram_client: None,
            llm: None,
        }
    }

    /// Create a tool with Telegram client for real message sending
    pub fn with_telegram(client: Arc<TelegramClient>) -> Self {
        Self {
            pool: None,
            telegram_client: Some(client),
            llm: None,
        }
    }

    pub fn with_pool_and_telegram(pool: Arc<PgPool>, client: Arc<TelegramClient>) -> Self {
        Self {
            pool: Some(pool),
            telegram_client: Some(client),
            llm: None,
        }
    }

    pub fn with_telegram_and_llm(client: Arc<TelegramClient>, llm: Arc<LiteLLMClient>) -> Self {
        Self {
            pool: None,
            telegram_client: Some(client),
            llm: Some(llm),
        }
    }

    pub fn with_pool_telegram_and_llm(
        pool: Arc<PgPool>,
        client: Arc<TelegramClient>,
        llm: Arc<LiteLLMClient>,
    ) -> Self {
        Self {
            pool: Some(pool),
            telegram_client: Some(client),
            llm: Some(llm),
        }
    }
}

impl Default for BroadcastMessageTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BroadcastMessageTool {
    fn name(&self) -> &str {
        "broadcast_message"
    }

    fn description(&self) -> &str {
        "向用户或群组发送广播消息。支持纯文本、带图片、自定义按钮。\
         可用于一键群发通知、营销文案推送、活动公告等。"
    }

    fn category(&self) -> &str {
        "broadcast"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "content": {
                    "type": "string",
                    "description": "要发送的消息正文（支持 HTML 格式）"
                },
                "target_type": {
                    "type": "string",
                    "enum": ["users", "groups", "all"],
                    "description": "目标类型：users=私聊用户, groups=群组频道, all=全部目标"
                },
                "target_ids": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "指定目标 ID 列表（可选，不传则使用全部）"
                },
                "include_image": {
                    "type": "boolean",
                    "description": "是否包含图片；需要同时提供 image_file_id、image_url 或 image"
                },
                "image_file_id": {
                    "type": "string",
                    "description": "Telegram 图片 file_id"
                },
                "image_url": {
                    "type": "string",
                    "description": "Telegram 可访问的图片 URL"
                },
                "image": {
                    "type": "string",
                    "description": "图片 file_id 或 URL（image_file_id/image_url 的兼容别名）"
                },
                "do_translate": {
                    "type": "boolean",
                    "description": "是否自动翻译为多语言"
                },
                "buttons": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": {"type": "string"},
                            "url": {"type": "string"}
                        },
                        "required": ["text", "url"]
                    },
                    "description": "自定义跳转按钮列表"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let mut content = params
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let image_source = match parse_image_source(params) {
            Ok(source) => source,
            Err(error) => return ToolExecuteResult::fail(&error),
        };

        if content.is_empty() && image_source.is_none() {
            return ToolExecuteResult::fail("消息内容或图片不能为空");
        }
        if content.chars().count() > MAX_BROADCAST_TEXT_CHARS {
            return ToolExecuteResult::fail("消息内容不能超过 4096 个字符");
        }

        if self.telegram_client.is_none() {
            return ToolExecuteResult::fail("Telegram 客户端未配置，广播未执行");
        }

        let do_translate = params
            .get("do_translate")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if do_translate {
            if content.is_empty() {
                return ToolExecuteResult::fail("图片广播没有可翻译的正文");
            }
            let Some(llm) = self.llm.as_ref() else {
                return ToolExecuteResult::fail("翻译服务未配置，广播未执行");
            };
            content = match llm
                .simple_chat(
                    "Translate the message into English when it is not English, otherwise into Simplified Chinese. Preserve Telegram HTML tags and return only the translation.",
                    &content,
                )
                .await
            {
                Ok(translated) => translated.trim().to_string(),
                Err(error) => return ToolExecuteResult::fail(&format!("广播翻译失败: {error}")),
            };
            if content.is_empty() || content.chars().count() > MAX_BROADCAST_TEXT_CHARS {
                return ToolExecuteResult::fail("翻译后的消息长度无效，广播未执行");
            }
        }

        info!(
            "BroadcastMessageTool: 准备向 {:?} 发送广播",
            params
                .get("target_type")
                .and_then(|v| v.as_str())
                .unwrap_or("users")
        );

        // 获取目标列表
        let target_type = params
            .get("target_type")
            .and_then(|v| v.as_str())
            .unwrap_or("users");
        if !matches!(target_type, "users" | "groups" | "all") {
            return ToolExecuteResult::fail("target_type 必须是 users、groups 或 all");
        }

        let target_ids = match parse_target_ids(params.get("target_ids")) {
            Ok(ids) => ids,
            Err(error) => return ToolExecuteResult::fail(&error),
        };

        let buttons = params
            .get("buttons")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // 执行广播
        match self
            .execute_broadcast(
                target_type,
                &target_ids,
                &content,
                &buttons,
                image_source.as_deref(),
                context,
            )
            .await
        {
            Ok(result) => {
                let total = result.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let success = result.get("success").and_then(|v| v.as_u64()).unwrap_or(0);
                let failed = result.get("failed").and_then(|v| v.as_u64()).unwrap_or(0);
                ToolExecuteResult {
                    success: success > 0,
                    data: Some(result),
                    message: Some(format!(
                        "广播完成: 总计{}个目标, 成功{}个, 失败{}个",
                        total, success, failed
                    )),
                    action_taken: success > 0,
                    needs_confirmation: false,
                    requires_llm_generation: false,
                    generated_content: None,
                }
            }
            Err(e) => {
                error!("BroadcastTool 执行失败: {}", e);
                ToolExecuteResult::fail(&format!("广播执行错误: {}", e))
            }
        }
    }
}

impl BroadcastMessageTool {
    /// 执行实际的广播逻辑
    async fn execute_broadcast(
        &self,
        target_type: &str,
        target_ids: &[i64],
        content: &str,
        buttons: &[Value],
        image_source: Option<&str>,
        context: &AgentContext,
    ) -> Result<serde_json::Value, String> {
        let (user_ids, chat_ids) = self
            .get_target_lists(target_type, target_ids, context)
            .await?;

        if user_ids.len() + chat_ids.len() > MAX_BROADCAST_TARGETS {
            return Err(format!(
                "广播目标超过单次上限 {MAX_BROADCAST_TARGETS}，广播未执行"
            ));
        }

        let mut total = 0;
        let mut success = 0;
        let mut failed_items: Vec<serde_json::Value> = vec![];
        let reply_markup = build_reply_markup(buttons)?;

        // 群发到用户
        if !user_ids.is_empty() {
            if let Some(ref client) = self.telegram_client {
                info!("BroadcastMessageTool: 向 {} 个用户发送消息", user_ids.len());
                for uid in &user_ids {
                    match send_broadcast_message(
                        client,
                        *uid,
                        content,
                        image_source,
                        reply_markup.as_ref(),
                    )
                    .await
                    {
                        Ok(_) => success += 1,
                        Err(e) => {
                            warn!("Failed to send to user {}: {}", uid, e);
                            failed_items.push(serde_json::json!({
                                "target_id": uid,
                                "error": e.to_string()
                            }));
                        }
                    }
                }
            }
            total += user_ids.len();
        }

        // 群发到群组
        if !chat_ids.is_empty() {
            if let Some(ref client) = self.telegram_client {
                info!("BroadcastMessageTool: 向 {} 个群组发送消息", chat_ids.len());
                for gid in &chat_ids {
                    match send_broadcast_message(
                        client,
                        *gid,
                        content,
                        image_source,
                        reply_markup.as_ref(),
                    )
                    .await
                    {
                        Ok(_) => success += 1,
                        Err(e) => {
                            warn!("Failed to send to group {}: {}", gid, e);
                            failed_items.push(serde_json::json!({
                                "target_id": gid,
                                "error": e.to_string()
                            }));
                        }
                    }
                }
            }
            total += chat_ids.len();
        }

        if total == 0 {
            return Err("没有有效的发送目标".to_string());
        }

        Ok(serde_json::json!({
            "total": total,
            "success": success,
            "failed": failed_items.len(),
            "failed_items": failed_items,
            "buttons": buttons,
            "has_image": image_source.is_some(),
        }))
    }

    async fn get_target_lists(
        &self,
        target_type: &str,
        requested_ids: &[i64],
        context: &AgentContext,
    ) -> Result<(Vec<i64>, Vec<i64>), String> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| "未配置广播目标数据源，广播未执行".to_string())?;
        let tenant_id = context
            .metadata
            .get("tenant_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "缺少 tenant_id，广播未执行".to_string())?;

        if target_type == "all" && !requested_ids.is_empty() {
            return Err("target_type=all 时不能使用未分类的 target_ids".into());
        }

        let user_ids = if matches!(target_type, "users" | "all") {
            let bot_id = sqlx::query_scalar::<_, i64>(
                "SELECT bot_id FROM merchants WHERE tenant_id = $1 AND bot_id IS NOT NULL",
            )
            .bind(tenant_id)
            .fetch_optional(pool.as_ref())
            .await
            .map_err(|error| format!("读取租户 Bot 失败: {error}"))?
            .ok_or_else(|| "当前租户未绑定 Bot，广播未执行".to_string())?;
            if requested_ids.is_empty() {
                sqlx::query_scalar::<_, i64>(
                    "SELECT DISTINCT CASE WHEN from_userid = $2 THEN to_userid ELSE from_userid END \
                     FROM chat.messages \
                     WHERE tenant_id = $1 AND (from_userid = $2 OR to_userid = $2) \
                       AND is_deleted = false \
                     ORDER BY 1",
                )
                .bind(tenant_id)
                .bind(bot_id)
                .fetch_all(pool.as_ref())
                .await
            } else {
                sqlx::query_scalar::<_, i64>(
                    "SELECT DISTINCT CASE WHEN from_userid = $2 THEN to_userid ELSE from_userid END \
                     FROM chat.messages \
                     WHERE tenant_id = $1 AND (from_userid = $2 OR to_userid = $2) \
                       AND is_deleted = false \
                       AND (CASE WHEN from_userid = $2 THEN to_userid ELSE from_userid END) = ANY($3) \
                     ORDER BY 1",
                )
                .bind(tenant_id)
                .bind(bot_id)
                .bind(requested_ids)
                .fetch_all(pool.as_ref())
                .await
            }
            .map_err(|error| format!("读取租户广播用户失败: {error}"))?
        } else {
            Vec::new()
        };
        let chat_ids = if matches!(target_type, "groups" | "all") {
            if requested_ids.is_empty() {
                sqlx::query_scalar::<_, i64>(
                    "SELECT chat_id FROM broadcast.broadcast_targets \
                     WHERE tenant_id = $1 AND can_broadcast = true ORDER BY chat_id",
                )
                .bind(tenant_id)
                .fetch_all(pool.as_ref())
                .await
            } else {
                sqlx::query_scalar::<_, i64>(
                    "SELECT chat_id FROM broadcast.broadcast_targets \
                     WHERE tenant_id = $1 AND chat_id = ANY($2) AND can_broadcast = true \
                     ORDER BY chat_id",
                )
                .bind(tenant_id)
                .bind(requested_ids)
                .fetch_all(pool.as_ref())
                .await
            }
            .map_err(|error| format!("读取租户广播群组失败: {error}"))?
        } else {
            Vec::new()
        };

        if !requested_ids.is_empty() {
            let requested = requested_ids
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>();
            let allowed = user_ids
                .iter()
                .chain(chat_ids.iter())
                .copied()
                .collect::<std::collections::HashSet<_>>();
            if requested != allowed {
                return Err("广播目标包含未绑定到当前租户的 ID，广播未执行".into());
            }
        }

        Ok((user_ids, chat_ids))
    }
}

async fn send_broadcast_message(
    client: &TelegramClient,
    chat_id: i64,
    content: &str,
    image_source: Option<&str>,
    reply_markup: Option<&Value>,
) -> anyhow::Result<tgbot_integrations::telegram::Message> {
    match image_source {
        Some(photo) => {
            client
                .send_photo(SendPhotoParams {
                    chat_id,
                    photo: photo.to_string(),
                    caption: (!content.is_empty()).then(|| content.to_string()),
                    parse_mode: Some("HTML".to_string()),
                    reply_to_message_id: None,
                    reply_markup: reply_markup.cloned(),
                })
                .await
        }
        None => {
            client
                .send_message(SendMessageParams {
                    chat_id,
                    text: content.to_string(),
                    parse_mode: Some("HTML".to_string()),
                    reply_markup: reply_markup.cloned(),
                    reply_to_message_id: None,
                })
                .await
        }
    }
}

fn parse_image_source(params: &Value) -> Result<Option<String>, String> {
    let source = ["image_file_id", "image_url", "image"]
        .iter()
        .find_map(|key| {
            params
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        });
    let include_image = params
        .get("include_image")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if include_image && source.is_none() {
        return Err("include_image=true 时必须提供图片 file_id 或 URL".into());
    }
    let Some(source) = source else {
        return Ok(None);
    };
    if source.chars().count() > MAX_IMAGE_SOURCE_CHARS {
        return Err("图片 file_id 或 URL 过长".into());
    }
    Ok(Some(source.to_string()))
}

fn build_reply_markup(buttons: &[Value]) -> Result<Option<Value>, String> {
    if buttons.is_empty() {
        return Ok(None);
    }
    let mut row = Vec::with_capacity(buttons.len());
    for button in buttons {
        let text = button
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "广播按钮缺少 text".to_string())?;
        let url = button
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| value.starts_with("https://") || value.starts_with("http://"))
            .ok_or_else(|| "广播按钮 url 必须是 http(s) 地址".to_string())?;
        row.push(serde_json::json!({"text": text, "url": url}));
    }
    Ok(Some(serde_json::json!({"inline_keyboard": [row]})))
}

fn parse_target_ids(value: Option<&Value>) -> Result<Vec<i64>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| "target_ids 必须是整数数组".to_string())?;
    if values.len() > MAX_BROADCAST_TARGETS {
        return Err(format!("target_ids 不能超过 {MAX_BROADCAST_TARGETS} 个"));
    }
    let mut ids = std::collections::HashSet::with_capacity(values.len());
    for value in values {
        let id = value
            .as_i64()
            .filter(|id| *id != 0)
            .ok_or_else(|| "target_ids 只能包含非零整数".to_string())?;
        if !ids.insert(id) {
            return Err("target_ids 不能包含重复 ID".to_string());
        }
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort_unstable();
    Ok(ids)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AgentContext, Tool};

    #[test]
    fn test_name_and_category() {
        let tool = BroadcastMessageTool::new();
        assert_eq!(tool.name(), "broadcast_message");
        assert_eq!(tool.category(), "broadcast");
    }

    #[test]
    fn test_parameters_schema() {
        let tool = BroadcastMessageTool::new();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("content").is_some());
        assert!(props.get("target_type").is_some());
        assert!(props.get("image_file_id").is_some());
        assert!(props.get("image_url").is_some());
    }

    #[tokio::test]
    async fn test_execute_empty_content() {
        let tool = BroadcastMessageTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"content": ""});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("不能为空"));
    }

    #[tokio::test]
    async fn test_image_flag_requires_a_source() {
        let tool = BroadcastMessageTool::new();
        let ctx = AgentContext::new(1);
        let result = tool
            .execute(
                &serde_json::json!({
                    "content": "图片广播",
                    "include_image": true,
                }),
                &ctx,
            )
            .await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("必须提供图片"));
    }

    #[tokio::test]
    async fn test_translation_requires_an_llm_client() {
        let telegram = Arc::new(
            TelegramClient::new(tgbot_integrations::telegram::TelegramConfig {
                bot_token: "beta-test".into(),
                api_base_url: "http://127.0.0.1:1".into(),
                timeout_secs: 1,
            })
            .unwrap(),
        );
        let tool = BroadcastMessageTool::with_telegram(telegram);
        let ctx = AgentContext::new(1);
        let result = tool
            .execute(
                &serde_json::json!({
                    "content": "translate me",
                    "do_translate": true,
                }),
                &ctx,
            )
            .await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("翻译服务未配置"));
    }

    #[tokio::test]
    async fn test_execute_with_content() {
        let tool = BroadcastMessageTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({
            "content": "测试广播",
            "target_type": "users"
        });
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        info!("广播结果: {:?}", result);
    }

    #[tokio::test]
    async fn test_execute_no_targets() {
        let tool = BroadcastMessageTool::new();
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({
            "content": "测试广播",
            "target_type": "users",
            "target_ids": []
        });
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn invalid_target_ids_fail_before_any_send() {
        let tool = BroadcastMessageTool::new();
        let ctx = AgentContext::new(1);
        for target_ids in [
            serde_json::json!("1"),
            serde_json::json!([1, "2"]),
            serde_json::json!([1, 1]),
            serde_json::json!([0]),
        ] {
            let result = tool
                .execute(
                    &serde_json::json!({"content": "测试广播", "target_ids": target_ids}),
                    &ctx,
                )
                .await;
            assert!(!result.success);
            assert!(!result.action_taken);
        }
    }

    #[test]
    fn valid_target_ids_are_deduplicated_only_by_rejection() {
        assert_eq!(
            parse_target_ids(Some(&serde_json::json!([42, -100]))).unwrap(),
            vec![-100, 42]
        );
        assert!(parse_target_ids(Some(&serde_json::json!([42, 42]))).is_err());
    }
}
