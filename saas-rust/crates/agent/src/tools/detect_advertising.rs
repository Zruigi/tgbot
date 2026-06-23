// 广告/垃圾消息识别工具
// 对应 Python: services/ai_agent/tools.py :: AdDetectorTool
// 底层逻辑复用 ad_detection.rs 的 detect_advertising_signals

use crate::ad_detection::detect_advertising_signals;
use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use tracing::info;

// ---------------------------------------------------------------------------
// DetectAdvertisingTool
// ---------------------------------------------------------------------------

/// 广告/垃圾消息识别工具
///
/// 支持：
/// - 广告关键词匹配
/// - 可疑链接检测
/// - 重复内容判断
/// - 推广意图评分
pub struct DetectAdvertisingTool;

#[async_trait]
impl Tool for DetectAdvertisingTool {
    fn name(&self) -> &str {
        "detect_advertising"
    }

    fn description(&self) -> &str {
        "检测文本中是否包含广告、垃圾信息、违规内容。支持：\
         广告关键词匹配、可疑链接检测、重复内容判断、推广意图评分。"
    }

    fn category(&self) -> &str {
        "security"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "text": {
                    "type": "string",
                    "description": "需要检测的文本内容"
                },
                "context_type": {
                    "type": "string",
                    "enum": ["group_message", "private_message", "bio", "username"],
                    "description": "上下文类型：群消息/私聊/简介/用户名"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, params: &Value, _context: &AgentContext) -> ToolExecuteResult {
        let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let context_type = params
            .get("context_type")
            .and_then(|v| v.as_str())
            .unwrap_or("group_message");

        if text.trim().is_empty() {
            return ToolExecuteResult::fail("检测文本不能为空");
        }

        info!(
            "DetectAdvertisingTool: 检测文本 (len={}, context={})",
            text.len(),
            context_type
        );

        let result = detect_advertising_signals(text, context_type);
        let is_ad = result.is_advertising;
        let ad_score = result.ad_score;
        let confidence = &result.confidence;

        let message = if is_ad {
            format!(
                "检测到广告内容 (得分: {}/100, 置信度: {})",
                ad_score, confidence
            )
        } else {
            format!(
                "未检出广告 (得分: {}/100, 置信度: {})",
                ad_score, confidence
            )
        };

        ToolExecuteResult::ok_with_data(
            serde_json::to_value(&result).unwrap_or_default(),
            &message,
        )
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
        let tool = DetectAdvertisingTool;
        assert_eq!(tool.name(), "detect_advertising");
        assert_eq!(tool.category(), "security");
    }

    #[test]
    fn test_parameters_schema() {
        let tool = DetectAdvertisingTool;
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("text").is_some());
        assert!(props.get("context_type").is_some());
    }

    #[tokio::test]
    async fn test_execute_empty_text() {
        let tool = DetectAdvertisingTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({"text": "", "context_type": "group_message"});
        let result = tool.execute(&params, &ctx).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_execute_ad_text() {
        let tool = DetectAdvertisingTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({
            "text": "加我微信免费领取红包返利佣金",
            "context_type": "group_message"
        });
        let result = tool.execute(&params, &ctx).await;
        assert!(result.success);
        let data = result.data.unwrap();
        assert!(data.get("is_advertising").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_execute_normal_text() {
        let tool = DetectAdvertisingTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({
            "text": "今天天气不错，大家好",
            "context_type": "group_message"
        });
        let result = tool.execute(&params, &ctx).await;
        assert!(result.success);
        let data = result.data.unwrap();
        assert!(!data.get("is_advertising").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_execute_with_suspicious_urls() {
        let tool = DetectAdvertisingTool;
        let ctx = AgentContext::new(1);
        let params = serde_json::json!({
            "text": "点击链接 https://t.me/joinchat/abc 和 https://bit.ly/xyz 赚钱",
            "context_type": "group_message"
        });
        let result = tool.execute(&params, &ctx).await;
        assert!(result.success);
        let data = result.data.unwrap();
        let findings = data.get("findings").unwrap().as_array().unwrap();
        assert!(findings.iter().any(|f| f.as_str().unwrap().contains("可疑链接")));
    }
}