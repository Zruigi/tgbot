//! 编排层（Runner）— 上下文组装、记忆压缩等编排逻辑
//!
//! 与 Agent 决策分离：Agent 负责"理解意图 + 调工具"，Runner 负责
//! "如何组织上下文 + 何时压缩记忆 + 如何进入 Agent"。本层可独立单测。

use crate::core::LlmCaller;
use crate::schema::{AgentContext, ChatMessage};
use serde_json::Value;
use std::sync::Arc;

/// 编排器：持有 LLM 引用（用于摘要压缩），提供上下文组装与记忆管理
pub struct Runner {
    llm: Arc<dyn LlmCaller>,
}

impl Runner {
    pub fn new(llm: Arc<dyn LlmCaller>) -> Self {
        Self { llm }
    }

    /// 组装发给 LLM 的完整 messages：system + [历史摘要] + 会话历史 + 当前消息
    ///
    /// 纯函数（不修改 ctx），可单测。
    pub fn build_messages(
        &self,
        system_prompt: &str,
        ctx: &AgentContext,
        user_message: &str,
    ) -> Vec<ChatMessage> {
        let mut messages = vec![ChatMessage::system(system_prompt)];
        if let Some(summary) = ctx
            .metadata
            .get("summary")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            messages.push(ChatMessage::system(&format!("[历史对话摘要]\n{}", summary)));
        }
        messages.extend(ctx.conversation_history.iter().cloned());
        messages.push(ChatMessage::user(user_message));
        messages
    }

    /// 历史超阈值时，把最旧的一半压缩成摘要（LLM 失败则降级，不阻塞主流程）
    pub async fn maybe_summarize_history(&self, ctx: &mut AgentContext) {
        const SUMMARY_THRESHOLD: usize = 30;
        if ctx.conversation_history.len() < SUMMARY_THRESHOLD {
            return;
        }
        if ctx.metadata.contains_key("summary") {
            return;
        }
        let split = ctx.conversation_history.len() - SUMMARY_THRESHOLD / 2;
        let to_summarize = ctx.conversation_history[..split].to_vec();
        let summary = self.summarize_history(&to_summarize).await;
        if let Some(s) = summary {
            ctx.metadata
                .insert("summary".into(), Value::String(s));
            ctx.conversation_history = ctx.conversation_history[split..].to_vec();
        }
    }

    /// 用 LLM 压缩一段历史为摘要；失败返回 None（降级，不影响主流程）
    async fn summarize_history(&self, history: &[ChatMessage]) -> Option<String> {
        if history.is_empty() {
            return None;
        }
        let mut msgs = vec![ChatMessage::system(
            "请将以下对话压缩为一份简洁的中文摘要，保留：用户身份、已完成的工具操作与结果、待确认事项、用户偏好。直接输出摘要内容，不要任何前缀。",
        )];
        // 截断避免摘要输入过长（最多取 200 条）
        let tail = if history.len() > 200 {
            &history[history.len() - 200..]
        } else {
            history
        };
        msgs.extend(tail.iter().cloned());
        let Ok(call) = self.llm.call(&msgs, crate::core::DEFAULT_MODEL, &[]).await else {
            return None;
        };
        let text = call.text.trim();
        if text.is_empty() || text.contains("[LLM 错误]") {
            None
        } else {
            Some(text.chars().take(800).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::LlmCallResult;

    /// 测试用 LLM：不真正调用，返回固定结果/失败
    struct NoopLlmCaller;

    #[async_trait::async_trait]
    impl LlmCaller for NoopLlmCaller {
        async fn call(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _tools: &[Value],
        ) -> Result<LlmCallResult, anyhow::Error> {
            Err(anyhow::anyhow!("noop"))
        }
    }

    fn sample_ctx() -> AgentContext {
        AgentContext::new(0)
    }

    fn dummy_runner() -> Runner {
        Runner::new(Arc::new(NoopLlmCaller))
    }

    fn text(msg: &ChatMessage) -> &str {
        msg.content.as_deref().unwrap_or("")
    }

    #[test]
    fn build_messages_without_summary() {
        let mut ctx = sample_ctx();
        ctx.conversation_history = vec![ChatMessage::user("你好"), ChatMessage::assistant("你好呀")];
        let runner = dummy_runner();
        let msgs = runner.build_messages("SYSTEM", &ctx, "今天天气如何");
        assert_eq!(msgs.len(), 4);
        assert_eq!(text(&msgs[0]), "SYSTEM");
        assert_eq!(text(&msgs[1]), "你好");
        assert_eq!(text(&msgs[2]), "你好呀");
        assert_eq!(text(&msgs[3]), "今天天气如何");
    }

    #[test]
    fn build_messages_with_summary() {
        let mut ctx = sample_ctx();
        ctx.conversation_history = vec![ChatMessage::user("旧消息")];
        ctx.metadata.insert("summary".into(), Value::String("摘要内容".into()));
        let runner = dummy_runner();
        let msgs = runner.build_messages("SYSTEM", &ctx, "新问题");
        assert_eq!(msgs.len(), 4);
        assert!(text(&msgs[1]).contains("[历史对话摘要]"));
        assert!(text(&msgs[1]).contains("摘要内容"));
    }

    #[test]
    fn build_messages_keeps_history_order() {
        let mut ctx = sample_ctx();
        ctx.conversation_history = vec![
            ChatMessage::user("q1"),
            ChatMessage::assistant("a1"),
            ChatMessage::user("q2"),
        ];
        let runner = dummy_runner();
        let msgs = runner.build_messages("SYSTEM", &ctx, "q3");
        assert_eq!(msgs.len(), 5);
        assert_eq!(text(&msgs[1]), "q1");
        assert_eq!(text(&msgs[2]), "a1");
        assert_eq!(text(&msgs[3]), "q2");
        assert_eq!(text(&msgs[4]), "q3");
    }
}
