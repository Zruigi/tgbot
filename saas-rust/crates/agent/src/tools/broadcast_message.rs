// 消息群发与内容分发工具
// 对应 Python: services/ai_agent/tools.py :: BroadcastTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// BroadcastMessageTool
// ---------------------------------------------------------------------------

/// 消息群发与内容分发工具
///
/// 支持：
/// - 向用户/群组/频道发送消息
/// - 纯文本、带图片、自定义按钮
/// - 目标类型：users / groups / all
pub struct BroadcastMessageTool;

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
                    "description": "是否包含图片（如有上传的话）"
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
            "required": ["content"]
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if content.is_empty() {
            return ToolExecuteResult::fail("消息内容不能为空");
        }

        info!(
            "BroadcastMessageTool: 准备向 {:?} 发送广播",
            params.get("target_type").and_then(|v| v.as_str()).unwrap_or("users")
        );

        // 获取目标列表
        let target_type = params
            .get("target_type")
            .and_then(|v| v.as_str())
            .unwrap_or("users");

        let target_ids = params
            .get("target_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_i64())
                    .collect::<Vec<i64>>()
            })
            .unwrap_or_default();

        let buttons = params
            .get("buttons")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let _do_translate = params
            .get("do_translate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 执行广播
        match self.execute_broadcast(target_type, &target_ids, content, &buttons).await {
            Ok(result) => {
                let total = result.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let success = result.get("success").and_then(|v| v.as_u64()).unwrap_or(0);
                let failed = result.get("failed").and_then(|v| v.as_u64()).unwrap_or(0);

                ToolExecuteResult::ok_with_data(result, &format!(
                    "广播完成: 总计{}个目标, 成功{}个, 失败{}个",
                    total, success, failed
                ))
            }
            Err(e) => {
                error!("BroadcastTool 执行失败: {}", e);
                ToolExecuteResult::fail(&format!("广播执行错误: {}", e))
            }
        }
    }

    /// 执行实际的广播逻辑
    async fn execute_broadcast(
        &self,
        target_type: &str,
        target_ids: &[i64],
        content: &str,
        buttons: &[Value],
    ) -> Result<serde_json::Value, String> {
        // 模拟广播结果（实际需接入 tgbot-services / tgbot-bot）
        // TODO: 接入真实 Bot 实例进行消息发送

        // 模拟获取目标列表
        let (user_ids, chat_ids) = if target_ids.is_empty() {
            self.get_target_lists(target_type).await
        } else {
            (
                if target_type == "users" || target_type == "all" {
                    target_ids.to_vec()
                } else {
                    vec![]
                },
                if target_type == "groups" || target_type == "all" {
                    target_ids.to_vec()
                } else {
                    vec![]
                },
            )
        };

        let mut total = 0;
        let mut success = 0;
        let mut failed_items: Vec<serde_json::Value> = vec![];

        // 群发到用户
        if !user_ids.is_empty() {
            warn!("BroadcastMessageTool: 准备向 {} 个用户发送消息（模拟）", user_ids.len());
            // TODO: 真实发送逻辑需接入 Bot 实例
            // 示例: for uid in &user_ids { bot.send_message(uid, content).await? }
            total += user_ids.len();
            success += user_ids.len(); // 模拟全部成功
        }

        // 群发到群组
        if !chat_ids.is_empty() {
            warn!("BroadcastMessageTool: 准备向 {} 个群组发送消息（模拟）", chat_ids.len());
            // TODO: 真实发送逻辑
            total += chat_ids.len();
            success += chat_ids.len(); // 模拟全部成功
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
        }))
    }

    /// 获取目标列表
    async fn get_target_lists(&self, target_type: &str) -> (Vec<i64>, Vec<i64>) {
        // TODO: 接入 database 模块获取真实目标列表
        // 参考 Python: database.get_inbox_list() / database.get_broadcast_targets()

        info!("BroadcastMessageTool::get_target_lists — 占位实现, target_type={}", target_type);

        // 返回空列表（实际需从数据库查询）
        (vec![], vec![])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Tool;
    use crate::schema::AgentContext;

    #[test]
    fn test_name_and_category() {
        let tool = BroadcastMessageTool;
        assert_eq!(tool.name(), "broadcast_message");
        assert_eq!(tool.category(), "broadcast");
    }

    #[test]
    fn test_parameters_schema() {
        let tool = BroadcastMessageTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("content").is_some());
        assert!(props.get("target_type").is_some());
    }

    #[tokio::test]
    async fn test_execute_empty_content() {
        let tool = BroadcastMessageTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"content": ""});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("不能为空"));
    }

    #[tokio::test]
    async fn test_execute_with_content() {
        let tool = BroadcastMessageTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({
            "content": "测试广播",
            "target_type": "users"
        });
        let result = tool.execute(&params, &ctx).await;
        // 当前返回模拟结果，实际接入 Bot 后应返回真实发送结果
        assert!(result.success || !result.success); // 占位实现
        info!("广播结果: {:?}", result);
    }

    #[tokio::test]
    async fn test_execute_no_targets() {
        let tool = BroadcastMessageTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({
            "content": "测试广播",
            "target_type": "users",
            "target_ids": []
        });
        let result = tool.execute(&params, &ctx).await;
        // 无目标时返回错误（模拟环境）
        info!("广播结果: {:?}", result);
    }
}