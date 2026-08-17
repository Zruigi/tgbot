//! 评估层（Eval）— 真实场景回归集
//!
//! 把"联合测试"变成可重复跑的自动化断言：每个 case 是一条真实用户表达，
//! 期望的"工具调用轨迹"是验收标准。跑法：`cargo test -p tgbot-agent eval`。
//!
//! 覆盖四大业务域：广告（检查/删除）、财务、系统、群发/技能。
//! 新改动上线前必须保证 eval 全绿——否则说明功能线路被破坏。

use crate::core::LlmCaller;
use crate::core::AIAgent;
use crate::schema::{AgentContext, ChatMessage, LlmCallResult};
use serde_json::Value;

/// 一个评估用例：真实用户表达 → 期望的工具调用轨迹
pub struct EvalCase {
    pub name: &'static str,
    pub input: &'static str,
    /// 期望工具轨迹（顺序不敏感，必须全部出现）
    pub expect_tools: &'static [&'static str],
}

/// 真实场景回归集（源自生产用户的实际输入）
pub const EVAL_CASES: &[EvalCase] = &[
    // 广告域
    EvalCase { name: "ad_check_fuzzy", input: "检查大鹅群的广告", expect_tools: &["query_recent_ad_messages"] },
    EvalCase { name: "ad_check_detect", input: "检测一下广告", expect_tools: &["query_recent_ad_messages"] },
    EvalCase { name: "ad_delete", input: "清理最近广告", expect_tools: &["query_recent_ad_messages", "delete_recent_ad_messages"] },
    EvalCase { name: "ad_inspect_2", input: "广告情况", expect_tools: &["query_recent_ad_messages"] },
    EvalCase { name: "ad_clusters", input: "查重复广告", expect_tools: &["query_ad_clusters"] },
    // 财务域
    EvalCase { name: "finance_check", input: "帮我查财务", expect_tools: &["finance_query"] },
    EvalCase { name: "finance_unreconciled", input: "未核销流水", expect_tools: &["finance_query"] },
    // 系统域
    EvalCase { name: "system_status", input: "系统状态", expect_tools: &["system_query"] },
    EvalCase { name: "system_groups", input: "有多少群", expect_tools: &["system_query"] },
    EvalCase { name: "system_senders", input: "最近有谁给我发了消息", expect_tools: &["system_query"] },
    EvalCase { name: "system_recent", input: "最近消息", expect_tools: &["system_query"] },
    // 群发/内容域（非白名单工具：需 ≥2 关键词或 LLM 决策）
    EvalCase { name: "broadcast", input: "群发通知给所有人发", expect_tools: &["broadcast_message"] },
    EvalCase { name: "generate", input: "帮我写生成文案", expect_tools: &["generate_content"] },
    EvalCase { name: "skill", input: "列出技能管理", expect_tools: &["skill_manage"] },
];

/// 测试用 LLM：总是返回"无工具调用"，逼 process_message 走确定性路由/关键词兜底
pub(crate) struct NoToolLlm;

#[async_trait::async_trait]
impl LlmCaller for NoToolLlm {
    async fn call(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _tools: &[Value],
    ) -> anyhow::Result<LlmCallResult> {
        Ok(LlmCallResult {
            text: String::new(),
            tool_calls: vec![],
        })
    }
}

/// 跑一条 case，返回实际工具轨迹
pub async fn run_case(agent: &AIAgent, case: &EvalCase) -> Vec<String> {
    let mut ctx = AgentContext::new(0);
    let resp = agent
        .process_message(case.input, &mut ctx, crate::core::DEFAULT_MODEL)
        .await;
    resp.tool_calls_made
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{default_tool_registry, ToolRegistry, DEFAULT_MODEL};
    use crate::schema::ToolExecuteResult;
    use std::sync::Arc;

    /// 依赖 DB 的工具在测试环境用同名 mock 替代（execute 返回固定结果）——
    /// eval 测的是"路由 + 确定性流程 + 工具轨迹"，不依赖真实数据库
    struct MockToolImpl {
        name: &'static str,
        confirm: bool,
    }

    #[async_trait::async_trait]
    impl crate::schema::Tool for MockToolImpl {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "eval mock tool"
        }
        async fn execute(
            &self,
            _params: &Value,
            _context: &AgentContext,
        ) -> ToolExecuteResult {
            let mut data = serde_json::json!({"mock": true});
            // 广告快照 mock：携带 snapshot_id（prepare_recent_ad_deletion 据此注入删除确认）
            if self.name == "query_recent_ad_messages" {
                data = serde_json::json!({
                    "snapshot_id": "snap-eval-test",
                    "candidate_count": 5,
                    "message_refs": [],
                });
            }
            ToolExecuteResult {
                success: true,
                data: Some(data),
                message: Some("mock ok".into()),
                action_taken: false,
                needs_confirmation: self.confirm,
                requires_llm_generation: false,
                generated_content: None,
            }
        }
    }

    fn build_eval_registry() -> ToolRegistry {
        let mut reg = default_tool_registry();
        for name in [
            "system_query",
            "query_ad_clusters",
            "skill_manage",
        ] {
            reg.register(Arc::new(MockToolImpl {
                name,
                confirm: false,
            }));
        }
        // 广告快照工具模拟"有快照需确认"（触发删除链路注入）
        reg.register(Arc::new(MockToolImpl {
            name: "query_recent_ad_messages",
            confirm: true,
        }));
        reg.register(Arc::new(MockToolImpl {
            name: "delete_recent_ad_messages",
            confirm: false,
        }));
        reg
    }

    fn build_agent() -> AIAgent {
        AIAgent::new(Arc::new(NoToolLlm), Arc::new(build_eval_registry()))
    }

    #[tokio::test]
    async fn eval_all_cases_route_correctly() {
        let agent = build_agent();
        let mut failures = Vec::new();
        for case in EVAL_CASES {
            let tools = run_case(&agent, case).await;
            for expect in case.expect_tools {
                if !tools.iter().any(|t| t == expect) {
                    failures.push(format!(
                        "case [{}] input=\"{}\": 期望工具 {} 未出现在轨迹 {:?}",
                        case.name, case.input, expect, tools
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{} 条失败:\n{}", failures.len(), failures.join("\n"));
    }

    #[tokio::test]
    async fn eval_ad_delete_requires_confirmation() {
        // 删除类必须触发确认（requires_confirmation=true），不能直接执行
        let agent = build_agent();
        let mut ctx = AgentContext::new(0);
        let resp = agent
            .process_message("清理最近广告", &mut ctx, DEFAULT_MODEL)
            .await;
        assert!(resp.requires_confirmation, "删除类操作必须要求确认");
        assert!(
            resp.tool_calls_made.contains(&"delete_recent_ad_messages".to_string()),
            "应进入删除链路"
        );
    }

    #[tokio::test]
    async fn eval_normal_chat_no_tool_calls() {
        // 闲聊不应触发任何工具
        let agent = build_agent();
        let tools = run_case(
            &agent,
            &EvalCase { name: "chat", input: "你好", expect_tools: &[] },
        )
        .await;
        assert!(tools.is_empty(), "闲聊不应调用工具，实际: {:?}", tools);
    }
}
