//! 提示词/工具审计 — 防止"工具打架"与"提示词脱节"
//!
//! 检查项：
//! 1. 工具名唯一
//! 2. keyword_map 词条不跨工具重叠（同一关键词不得路由到多个工具）
//! 3. 每个注册工具都在 TOOL_INSTRUCTIONS / skill 声明中有描述（prompt 覆盖完整）
//!
//! 用法：单测/CI 中调用 `audit_toolset` 断言 `report.is_clean()`；
//! 开发环境可在 AIAgent::new 用 debug_assert 触发。

use crate::schema::Tool;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 审计报告
#[derive(Debug, Default)]
pub struct AuditReport {
    pub issues: Vec<String>,
}

impl AuditReport {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
    pub fn push(&mut self, issue: String) {
        self.issues.push(issue);
    }
}

/// 对工具集做一致性审计
///
/// `strict_declared`：为 true 时还检查"声明了但未注册"（死引用）。
/// 运行时/测试环境（工具集可能不全）传 false；完整工具集回归传 true。
pub fn audit_toolset(
    tools: &[Arc<dyn Tool>],
    keyword_map: &HashMap<&'static str, Vec<&'static str>>,
    declared_tool_names: &[&str],
    strict_declared: bool,
) -> AuditReport {
    let mut report = AuditReport::default();

    // 1. 工具名唯一
    let mut seen_names: HashSet<&str> = HashSet::new();
    for tool in tools {
        if !seen_names.insert(tool.name()) {
            report.push(format!("工具名重复: {}", tool.name()));
        }
    }

    // 2. 关键词互斥：同一关键词不得映射到多个工具
    let mut kw_owner: HashMap<&str, &str> = HashMap::new();
    for (tool_name, keywords) in keyword_map {
        for kw in keywords {
            if let Some(prev) = kw_owner.insert(kw, tool_name) {
                if prev != *tool_name {
                    report.push(format!(
                        "关键词重叠: \"{}\" 同时路由到 {} 与 {}",
                        kw, prev, tool_name
                    ));
                }
            }
        }
    }

    // 3. prompt/声明覆盖：每个注册工具都要有描述
    let declared: HashSet<&str> = declared_tool_names.iter().copied().collect();
    for tool in tools {
        if !declared.contains(tool.name()) {
            report.push(format!("工具 {} 缺少 prompt/声明描述", tool.name()));
        }
    }
    // 4. 反向（严格模式）：声明了但没注册（死引用，如曾出现的 telegram_sdk_query）
    if strict_declared {
        for name in declared {
            if !seen_names.contains(name) {
                report.push(format!("声明了但未注册的工具: {}", name));
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{AgentContext, ToolExecuteResult};
    use serde_json::Value;

    struct MockTool(String);

    #[async_trait::async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.0
        }
        fn description(&self) -> &str {
            "mock tool"
        }
        async fn execute(
            &self,
            _params: &Value,
            _context: &AgentContext,
        ) -> ToolExecuteResult {
            ToolExecuteResult {
                success: true,
                data: None,
                message: None,
                action_taken: false,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            }
        }
    }

    fn tools(names: &[&str]) -> Vec<Arc<dyn Tool>> {
        names
            .iter()
            .map(|n| Arc::new(MockTool(n.to_string())) as Arc<dyn Tool>)
            .collect()
    }

    #[test]
    fn clean_toolset_passes() {
        let t = tools(&["finance_query", "system_query"]);
        let mut km = HashMap::new();
        km.insert("finance_query", vec!["查财务"]);
        km.insert("system_query", vec!["系统状态"]);
        let report = audit_toolset(&t, &km, &["finance_query", "system_query"], true);
        assert!(report.is_clean(), "{:?}", report.issues);
    }

    #[test]
    fn detects_keyword_overlap() {
        let t = tools(&["detect_advertising", "query_recent_ad_messages"]);
        let mut km = HashMap::new();
        // 同一关键词"检测广告"映射到两个工具 → 必须被审计发现
        km.insert("detect_advertising", vec!["检测广告"]);
        km.insert("query_recent_ad_messages", vec!["检测广告"]);
        let report = audit_toolset(&t, &km, &["detect_advertising", "query_recent_ad_messages"], true);
        assert!(!report.is_clean());
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.contains("关键词重叠") && i.contains("检测广告")),
            "{:?}",
            report.issues
        );
    }

    #[test]
    fn detects_unregistered_declared_tool() {
        let t = tools(&["finance_query"]);
        let km = HashMap::new();
        // telegram_sdk_query 已声明但未注册（历史死路由）→ 必须被审计发现
        let report = audit_toolset(&t, &km, &["finance_query", "telegram_sdk_query"], true);
        assert!(!report.is_clean());
        assert!(
            report.issues.iter().any(|i| i.contains("未注册")),
            "{:?}",
            report.issues
        );
    }

    #[test]
    fn detects_missing_prompt_declaration() {
        let t = tools(&["finance_query", "new_tool_no_prompt"]);
        let mut km = HashMap::new();
        km.insert("finance_query", vec!["查财务"]);
        km.insert("new_tool_no_prompt", vec!["新功能"]);
        let report = audit_toolset(&t, &km, &["finance_query"], true);
        assert!(!report.is_clean());
        assert!(
            report.issues.iter().any(|i| i.contains("缺少")),
            "{:?}",
            report.issues
        );
    }

    #[test]
    fn audit_production_registry_no_overlap() {
        // 真实工具集 + 真实 keyword_map 回归：注册工具都需在声明中、关键词互斥。
        // 模拟生产注册集（ai.rs build_agent 的 retain_names 剔除了 ocr/n8n 等未声明工具）
        let registry = crate::core::default_tool_registry();
        let tools: Vec<Arc<dyn Tool>> = crate::prompts::DECLARED_TOOLS
            .iter()
            .filter_map(|name| registry.get(name).cloned())
            .collect();
        let report = audit_toolset(
            &tools,
            crate::router::dispatcher().keyword_map(),
            crate::prompts::DECLARED_TOOLS,
            false,
        );
        assert!(report.is_clean(), "{:?}", report.issues);
    }
}
