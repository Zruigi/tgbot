//! 路由层（Dispatcher）— 从 core.rs 迁移的启发式工具匹配 + fallback 参数构造
//!
//! 职责：把用户文本路由到工具。
//! 优先级：确定性前置路由（core.rs prepare_*，处理删除/检查类）→ 本层 keyword_map/描述匹配 → LLM function calling。
//! 本层是纯逻辑、无 IO，可独立单测（`cargo test router`）。

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// 启发式路由调度器：持有关键词表 + 白名单，提供文本→工具匹配与 fallback 参数构造
pub struct Dispatcher {
    keyword_map: HashMap<&'static str, Vec<&'static str>>,
    whitelist: HashSet<&'static str>,
}

impl Dispatcher {
    pub fn new() -> Self {
        let keyword_map: HashMap<&'static str, Vec<&'static str>> = {
            let mut m = HashMap::new();
            m.insert(
                "finance_query",
                vec![
                    "查财务", "财务摘要", "财务查询", "财务报表", "财务分析", "查账目", "查流水",
                    "看账目", "看流水", "未核销", "待核销", "核销流水", "对账单", "对个账",
                ],
            );
            m.insert(
                "broadcast_message",
                vec![
                    "群发消息", "群发通知", "发送广播", "给所有人发", "群发公告", "批量发送",
                ],
            );
            m.insert(
                "detect_advertising",
                vec![
                    "检测广告", "是不是广告", "广告检测", "垃圾信息检测", "是否广告", "鉴别广告",
                    "这条是不是广告", "这段是不是广告", "鉴别这条", "检测这条消息",
                ],
            );
            m.insert(
                "generate_content",
                vec![
                    "帮我写", "生成文案", "写一段", "生成内容", "帮我生成", "写个文案", "写公告",
                ],
            );
            m.insert(
                "system_query",
                vec![
                    "系统状态", "系统概览", "机器人绑定", "绑定聊天", "绑定群", "绑定频道",
                    "有多少群", "群组统计", "系统信息", "活跃群组", "最近消息", "最近有谁",
                    "谁给我发", "谁给我发送", "最近发消息",
                ],
            );
            m.insert(
                "query_recent_ad_messages",
                vec![
                    "广告消息", "群广告", "疑似广告", "清理广告", "删除广告", "删广告", "最近广告",
                    "查广告消息", "广告清理", "检查广告", "查看广告", "广告检查", "广告情况",
                    "广告状态",
                ],
            );
            m.insert(
                "query_ad_clusters",
                vec![
                    "广告聚合", "广告簇", "广告聚类", "查广告簇", "查重复广告", "重复广告",
                    "聚合广告",
                ],
            );
            m.insert(
                "skill_manage",
                vec![
                    "新增skill", "增加skill", "添加skill", "技能管理", "列出技能", "查看技能",
                    "更新技能", "导入技能", "新增技能", "添加技能",
                ],
            );
            m
        };

        // 白名单工具 keyword 命中 >=8 即匹配；非白名单需 >=16（deb8eaf 确定性路由）
        let whitelist: HashSet<&'static str> = [
            "finance_query",
            "system_query",
            "detect_advertising",
            "query_ad_clusters",
            "query_recent_ad_messages",
        ]
        .into_iter()
        .collect();

        Self {
            keyword_map,
            whitelist,
        }
    }

    /// 根据用户文本做启发式工具匹配（仅作为 LLM 未显式返回 tool_call 时的兜底）
    pub fn match_tool_from_text(&self, text: &str, tools: &[Value]) -> Option<String> {
        if text.is_empty() {
            return None;
        }

        let normalized = text.to_lowercase();

        let mut best_match: Option<String> = None;
        let mut best_score: u32 = 0;

        for tool in tools {
            let (name, desc) = if let Some(func) = tool.get("function") {
                let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let desc = func
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                (name.to_string(), desc)
            } else {
                continue;
            };

            let mut score: u32 = 0;

            if let Some(keywords) = self.keyword_map.get(name.as_str()) {
                for keyword in keywords {
                    if normalized.contains(keyword) {
                        score += 8;
                    }
                }
            }

            if !name.is_empty() && normalized.contains(&name) {
                score += 6;
            }

            if score == 0 {
                for token in normalized.split_whitespace() {
                    if !token.is_empty() && desc.contains(token) {
                        score += 1;
                    }
                }
            }

            if score > best_score {
                best_score = score;
                best_match = Some(name);
            }
        }

        match &best_match {
            Some(name) if self.whitelist.contains(name.as_str()) && best_score >= 8 => best_match,
            Some(_) if best_score >= 16 => best_match,
            _ => None,
        }
    }

    /// 当模型未返回参数时，基于用户原文构造一组可执行参数
    pub fn build_fallback_args(&self, tool_name: &str, user_text: &str) -> Value {
        let text = user_text.trim();
        if text.is_empty() {
            return serde_json::json!({});
        }

        match tool_name {
            "broadcast_message" => {
                let mut content = text.to_string();
                for kw in &[
                    "群发消息", "群发通知", "发送广播", "给所有人发", "群发公告", "批量发送", "测试",
                ] {
                    content = content.replace(kw, "");
                }
                let content = content
                    .trim_matches(|c: char| c.is_whitespace() || "：:，,。.!！？?".contains(c));
                let content = if content.is_empty() { text } else { content };
                serde_json::json!({"content": content, "target_type": "users"})
            }
            "finance_query" => {
                if text.contains("摘要") {
                    serde_json::json!({"action": "summary"})
                } else if text.contains("未核销") || text.contains("待核销") {
                    serde_json::json!({"action": "unreconciled"})
                } else if text.contains("科目") || text.contains("分类") {
                    serde_json::json!({"action": "categories"})
                } else {
                    serde_json::json!({"action": "analysis"})
                }
            }
            "detect_advertising" => {
                serde_json::json!({"text": text, "context_type": "private_message"})
            }
            "query_recent_ad_messages" => {
                serde_json::json!({"hours": 168, "limit": 50})
            }
            "query_ad_clusters" => {
                let mut params = serde_json::json!({
                    "status": "pending",
                    "limit": 20,
                    "window_minutes": 60,
                });
                if let Ok(group_pattern) =
                    regex::Regex::new(r"(?i)(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})")
                {
                    if let Some(captures) = group_pattern.captures(text) {
                        if let Some(group_id) = captures
                            .get(1)
                            .and_then(|value| value.as_str().parse::<i64>().ok())
                        {
                            params["group_id"] = serde_json::json!(group_id);
                        }
                    }
                }
                params
            }
            "telegram_sdk_query" => {
                // 4c3f1dc: 数字 group_id 优先于群名解析（该工具已从白名单/关键词表移除，分支保留防御）
                if let Ok(re) =
                    regex::Regex::new(r"(?i)(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})")
                {
                    if let Some(caps) = re.captures(text) {
                        if let Some(id_str) = caps.get(1) {
                            if let Ok(chat_id) = id_str.as_str().parse::<i64>() {
                                return serde_json::json!({
                                    "operation": "chat_capabilities",
                                    "chat_id": chat_id,
                                    "include_admins": false,
                                });
                            }
                        }
                    }
                }
                serde_json::json!({
                    "operation": "bound_chat_capabilities",
                    "limit": 50,
                    "include_admins": false,
                })
            }
            "generate_content" => {
                serde_json::json!({
                    "topic": text,
                    "style": "professional",
                    "length": "medium",
                    "purpose": "announcement"
                })
            }
            "system_query" => {
                if text.contains("最近有谁")
                    || text.contains("谁给我发")
                    || text.contains("谁给我发送")
                    || text.contains("最近发消息")
                    || text.contains("谁发了消息")
                {
                    serde_json::json!({"query_type": "recent_senders", "limit": 10})
                } else if text.contains("最近消息") {
                    serde_json::json!({"query_type": "recent_messages", "limit": 10})
                } else if text.contains("潜在用户") {
                    serde_json::json!({"query_type": "potential_users", "limit": 10})
                } else if text.contains("绑定")
                    && (text.contains("群") || text.contains("频道") || text.contains("聊天"))
                {
                    serde_json::json!({"query_type": "bound_chats", "limit": 50})
                } else if text.contains("监听")
                    || text.contains("地址列表")
                    || text.contains("监控地址")
                {
                    serde_json::json!({"query_type": "monitors_list", "limit": 10})
                } else if text.contains("群") || text.contains("群组") {
                    serde_json::json!({"query_type": "active_groups", "limit": 10})
                } else {
                    serde_json::json!({"query_type": "stats_overview"})
                }
            }
            "skill_manage" => {
                let lower = text.to_lowercase();
                if text.contains("导入") {
                    serde_json::json!({"action": "import_builtin"})
                } else if (lower.contains("新增")
                    || lower.contains("添加")
                    || lower.contains("增加"))
                    && lower.contains("skill")
                {
                    serde_json::json!({"action": "add"})
                } else if text.contains("启用") {
                    serde_json::json!({"action": "enable"})
                } else if text.contains("停用") || text.contains("禁用") {
                    serde_json::json!({"action": "disable"})
                } else if text.contains("更新") || text.contains("修改") {
                    serde_json::json!({"action": "update"})
                } else {
                    serde_json::json!({"action": "list"})
                }
            }
            _ => serde_json::json!({}),
        }
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    /// 暴露关键词表（供审计/调试）
    pub fn keyword_map(&self) -> &HashMap<&'static str, Vec<&'static str>> {
        &self.keyword_map
    }
}

/// 全局单例（keyword_map/whitelist 为静态数据，只构建一次）
pub fn dispatcher() -> &'static Dispatcher {
    static INSTANCE: OnceLock<Dispatcher> = OnceLock::new();
    INSTANCE.get_or_init(Dispatcher::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造与生产一致的模拟工具 schema（供封闭单测）
    fn sample_tools() -> Vec<Value> {
        let mut tools = Vec::new();
        for (name, desc) in [
            ("finance_query", "查询财务摘要、未核销流水、科目树，输出分析报告"),
            ("broadcast_message", "向用户/群组/频道发送消息，支持文本图片按钮，危险操作需确认"),
            ("detect_advertising", "识别广告/垃圾信息/推广意图，输出广告评分与处理建议"),
            ("generate_content", "生成营销文案、公告、话术，支持风格长度受众"),
            ("system_query", "系统统计概览、活跃群组、监听地址、最近消息、潜在用户统计"),
            ("query_recent_ad_messages", "查询指定时间窗口内的广告消息并生成快照供确认删除"),
            ("query_ad_clusters", "查询已落库群消息中的重复广告候选"),
            ("skill_manage", "查看/新增/更新/停用 AI 助手技能"),
        ] {
            tools.push(json!({
                "type": "function",
                "function": { "name": name, "description": desc, "parameters": {"type": "object", "properties": {}} }
            }));
        }
        tools
    }

    // ---------- 单点测试：模糊/明确表达 → 路由 ----------

    #[test]
    fn route_finance_expressions() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        // finance_query 在白名单：单关键词 8 分即匹配
        for expr in ["帮我查财务", "查流水", "未核销", "对个账", "财务分析报告"] {
            assert_eq!(
                d.match_tool_from_text(expr, &tools).as_deref(),
                Some("finance_query"),
                "expr={expr}"
            );
        }
    }

    #[test]
    fn route_broadcast_expressions() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        // broadcast_message 非白名单：需 ≥2 个关键词（16 分）
        for expr in ["群发通知给所有人发", "批量发送群发公告", "给所有人发群发消息"] {
            assert_eq!(
                d.match_tool_from_text(expr, &tools).as_deref(),
                Some("broadcast_message"),
                "expr={expr}"
            );
        }
    }

    #[test]
    fn route_detect_expressions() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        for expr in ["这条是不是广告", "检测这条消息", "广告检测", "这段是不是广告"] {
            assert_eq!(
                d.match_tool_from_text(expr, &tools).as_deref(),
                Some("detect_advertising"),
                "expr={expr}"
            );
        }
    }

    #[test]
    fn route_system_query_expressions() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        for expr in ["系统状态", "有多少群", "最近有谁给我发了消息", "活跃群组"] {
            assert_eq!(
                d.match_tool_from_text(expr, &tools).as_deref(),
                Some("system_query"),
                "expr={expr}"
            );
        }
    }

    #[test]
    fn route_ad_query_expressions() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        for expr in ["最近广告消息", "群广告", "检查广告", "广告情况", "删除最近广告"] {
            assert_eq!(
                d.match_tool_from_text(expr, &tools).as_deref(),
                Some("query_recent_ad_messages"),
                "expr={expr}"
            );
        }
    }

    #[test]
    fn route_ad_clusters_expressions() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        for expr in ["查广告簇", "重复广告", "广告聚合"] {
            assert_eq!(
                d.match_tool_from_text(expr, &tools).as_deref(),
                Some("query_ad_clusters"),
                "expr={expr}"
            );
        }
    }

    #[test]
    fn route_generate_and_skill() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        // generate_content / skill_manage 非白名单：需 ≥2 个关键词
        assert_eq!(
            d.match_tool_from_text("帮我写生成文案", &tools).as_deref(),
            Some("generate_content")
        );
        assert_eq!(
            d.match_tool_from_text("列出技能管理", &tools).as_deref(),
            Some("skill_manage")
        );
    }

    #[test]
    fn route_normal_chat_returns_none() {
        let d = Dispatcher::new();
        let tools = sample_tools();
        for expr in ["你好", "谢谢", "今天天气不错"] {
            assert!(d.match_tool_from_text(expr, &tools).is_none(), "expr={expr}");
        }
        assert!(d.match_tool_from_text("", &tools).is_none());
    }

    #[test]
    fn route_dead_tool_not_reachable() {
        // telegram_sdk_query 已从 keyword_map/白名单移除，不应被路由命中
        let d = Dispatcher::new();
        let tools = sample_tools();
        assert_eq!(d.match_tool_from_text("机器人权限", &tools), None);
        assert_eq!(d.match_tool_from_text("为什么删除失败", &tools), None);
    }

    // ---------- 单点测试：fallback 参数构造 ----------

    #[test]
    fn fallback_args_finance() {
        let d = Dispatcher::new();
        assert_eq!(d.build_fallback_args("finance_query", "查财务摘要")["action"], "summary");
        assert_eq!(d.build_fallback_args("finance_query", "未核销流水")["action"], "unreconciled");
        assert_eq!(d.build_fallback_args("finance_query", "科目树")["action"], "categories");
        assert_eq!(d.build_fallback_args("finance_query", "帮我查账")["action"], "analysis");
    }

    #[test]
    fn fallback_args_system_query() {
        let d = Dispatcher::new();
        let a = d.build_fallback_args("system_query", "最近有谁给我发消息");
        assert_eq!(a["query_type"], "recent_senders");
        let b = d.build_fallback_args("system_query", "系统状态");
        assert_eq!(b["query_type"], "stats_overview");
        let c = d.build_fallback_args("system_query", "有多少群");
        assert_eq!(c["query_type"], "active_groups");
    }

    #[test]
    fn fallback_args_broadcast_and_content() {
        let d = Dispatcher::new();
        let a = d.build_fallback_args("broadcast_message", "群发通知：今晚维护");
        assert_eq!(a["content"], "今晚维护");
        assert_eq!(a["target_type"], "users");
        let b = d.build_fallback_args("generate_content", "写个产品介绍");
        assert_eq!(b["topic"], "写个产品介绍");
    }

    #[test]
    fn fallback_args_ad_tools() {
        let d = Dispatcher::new();
        let a = d.build_fallback_args("query_recent_ad_messages", "清理广告");
        assert_eq!(a["hours"], 168);
        assert_eq!(a["limit"], 50);
        let b = d.build_fallback_args("detect_advertising", "这条是广告吗");
        assert_eq!(b["text"], "这条是广告吗");
    }
}
