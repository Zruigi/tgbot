//! 核心层（Core Abstractions）— 基础 Trait 定义
//!
//! 借鉴 ADK-Rust 的 5 核心 Trait：Agent / Llm / Tool / Session / Toolset。
//! 本项目：`Tool`（schema.rs）与 `Llm`（core::LlmCaller）已存在，
//! 本模块补齐 `Agent` / `Session` / `Toolset` 三个抽象并让现有类型实现，
//! 为阶段 5（Streaming / Toolset 动态工具集 / 多 Agent 编排）铺路。

use crate::core::{AIAgent, ToolRegistry};
use crate::schema::{AgentContext, AgentResponse, ChatMessage, Tool};
use crate::session::AgentSession;
use serde_json::Value;
use std::sync::Arc;

/// Agent 抽象：可执行的智能体单元（当前为 LLM 驱动单 Agent；未来可扩展顺序/并行/图 Agent）
pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// 处理一条用户消息，返回响应
    async fn run(&self, ctx: &mut AgentContext, message: &str, model: &str) -> AgentResponse;
}

impl Agent for AIAgent {
    fn name(&self) -> &str {
        "llm_agent"
    }
    fn description(&self) -> &str {
        "LLM 驱动的单 Agent：理解意图、调用工具、总结回复"
    }
    async fn run(&self, ctx: &mut AgentContext, message: &str, model: &str) -> AgentResponse {
        self.process_message(message, ctx, model).await
    }
}

/// Session 抽象：会话状态与历史（内存实现 AgentSession；未来可接持久化）
pub trait Session: Send + Sync {
    fn history(&self) -> Vec<ChatMessage>;
    fn push_user(&mut self, content: &str);
    fn push_assistant(&mut self, content: &str);
}

impl Session for AgentSession {
    fn history(&self) -> Vec<ChatMessage> {
        self.ctx.conversation_history.clone()
    }
    fn push_user(&mut self, content: &str) {
        self.ctx
            .conversation_history
            .push(ChatMessage::user(content));
    }
    fn push_assistant(&mut self, content: &str) {
        self.ctx
            .conversation_history
            .push(ChatMessage::assistant(content));
    }
}

/// Toolset 抽象：工具集（支持组合/动态启用子集，解决"工具互相打架"）
pub trait Toolset: Send + Sync {
    fn names(&self) -> Vec<String>;
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;
    fn schemas(&self) -> Vec<Value>;
}

impl Toolset for ToolRegistry {
    fn names(&self) -> Vec<String> {
        ToolRegistry::names(self)
    }
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        ToolRegistry::get(self, name).cloned()
    }
    fn schemas(&self) -> Vec<Value> {
        ToolRegistry::schemas_for_llm(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ToolExecuteResult;

    /// 测试用最小工具
    struct PingTool;

    #[async_trait::async_trait]
    impl Tool for PingTool {
        fn name(&self) -> &str {
            "ping"
        }
        fn description(&self) -> &str {
            "测试工具：返回 pong"
        }
        fn category(&self) -> &str {
            "system"
        }
        async fn execute(
            &self,
            _params: &Value,
            _context: &AgentContext,
        ) -> ToolExecuteResult {
            ToolExecuteResult {
                success: true,
                data: Some(serde_json::json!({"pong": true})),
                message: Some("pong".into()),
                action_taken: false,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            }
        }
    }

    #[test]
    fn agent_trait_name_and_description() {
        // Agent trait 的静态契约
        let _ = AIAgent::name;
        let _ = AIAgent::description;
        // 直接验证实现存在（编译期已保证 impl Agent for AIAgent）
        fn assert_agent<T: Agent>() {}
        assert_agent::<AIAgent>();
    }

    #[test]
    fn session_trait_push_and_history() {
        let mut session = AgentSession::new(AgentContext::new(0));
        session.push_user("你好");
        session.push_assistant("你好呀");
        let history = session.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content.as_deref(), Some("你好"));
        assert_eq!(history[1].content.as_deref(), Some("你好呀"));
    }

    #[test]
    fn toolset_trait_names_get_schemas() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(PingTool));
        let toolset = &registry as &dyn Toolset;
        assert_eq!(toolset.names(), vec!["ping".to_string()]);
        assert!(toolset.get("ping").is_some());
        assert!(toolset.get("nope").is_none());
        let schemas = toolset.schemas();
        assert!(!schemas.is_empty());
        // schema 包含工具名
        let joined = serde_json::to_string(&schemas).unwrap_or_default();
        assert!(joined.contains("ping"));
    }
}
