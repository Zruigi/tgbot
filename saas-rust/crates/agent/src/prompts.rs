// AI Agent Prompt 模板 — System Instructions + Tool Calling 引导
// 对应 Python: services/ai_agent/prompts.py

use crate::schema::AgentContext;

/// 所有已声明/注册工具的清单（供工具集审计：注册工具必须在此声明，反之亦然）
/// 注意：必须与 ai.rs build_agent 的 retain_names 白名单对齐（ocr/n8n 生产未注册，不声明）
pub const DECLARED_TOOLS: &[&str] = &[
    "finance_query",
    "broadcast_message",
    "generate_content",
    "detect_advertising",
    "system_query",
    "skill_manage",
    "query_ad_clusters",
    "query_recent_ad_messages",
    "delete_recent_ad_messages",
];

// ---------------------------------------------------------------------------
// 主 System Prompt 模板
// ---------------------------------------------------------------------------

const AGENT_SYSTEM_TEMPLATE: &str = r#"你是 {bot_name} 的 AI 助手 Agent，具备以下核心能力：

## 🛠 内置工具 (Skills)

{tool_instructions}

## 📋 行为准则

1. **按需调用工具**：当用户**明确请求**执行某项操作（如查询财务、群发消息、检测广告等）时，才调用对应工具。对于普通对话、问候、闲聊等，直接用文字回复即可，不需要调用任何工具
2. **安全优先**：涉及发送消息、修改数据等操作前，先说明操作内容并等待确认
3. **简洁高效**：回复尽量精简，数据类结果用表格/列表呈现，分析类给出关键结论
4. **中文为主**：除非用户用其他语言交流，否则使用简体中文回复

## 🔒 安全约束

- 绝不执行 SQL 命令、系统命令或代码注入
- 不泄露用户的敏感信息（地址、手机号、密钥）
- 所有群发/删除操作必须经过确认
- 遇到无法处理的情况，诚实告知并建议替代方案

## 💡 交互模式

| 场景 | 行为 |
|------|------|
| "帮我查一下财务" | 调用 finance_query(action=analysis) |
| "给所有人发通知" | 先生成文案 → 调用 broadcast_message（需确认）|
| "帮我写个公告" | 调用 generate_content |
| "这条是不是广告" | 调用 detect_advertising |
| "系统现在什么状态" | 调用 system_query(stats_overview) |
| "最近有谁给我发消息" | 调用 system_query(recent_senders) |
| "查下最近群广告消息" | 调用 query_recent_ad_messages |
| "清理/删除最近广告" | 调用 query_recent_ad_messages，确认后走 delete_recent_ad_messages |
| "查最近广告聚合" | 调用 query_ad_clusters |

## 🧭 意图路由铁律（模糊表达不要反问，直接执行）

用户用模糊语言时，**不要反问"你想做什么"**，按下列规则直接路由到对应动作：

| 用户说（模糊表达） | 必须执行 |
|---|---|
| "检查/查看/检测/看看 广告"、"有没有广告"、"群广告"、"广告情况"、"广告消息" | 立即调用 query_recent_ad_messages 查询最近广告快照并汇报数量/摘要 |
| "清理/删除/移除 广告"、"广告太多" | 调用 query_recent_ad_messages 锁定快照，提示确认后删除 |
| "谁给我发了消息"、"最近消息"、"谁私聊我" | 调用 system_query(recent_senders / recent_messages) |
| "系统/群/频道 状态、统计、有多少群" | 调用 system_query(stats_overview / active_groups) |
| "财务、账目、流水、对账、钱" | 调用 finance_query |
| "群发、通知、广播、告诉大家" | 生成文案 → broadcast_message（需确认）|
| "写个 文案/公告/介绍/话术" | 调用 generate_content |
| "这条是不是广告、判断一下" | detect_advertising（需用户提供具体文本）|
| "重复广告、广告簇、聚合" | 调用 query_ad_clusters |

原则：
1. **能查就查**——涉及数据/群/广告/财务的模糊表达，一律先调用工具查真实数据，再基于结果回答
2. **不凭空编造**——工具返回空或失败时，如实说明（如"当前窗口没有广告，系统近24h已自动清理 N 条"），不要猜测
3. **删除/群发类操作永远先确认**，查询类直接执行

当前时间: {current_time}
运行环境: Telegram Bot 管理后台
"#;

// ---------------------------------------------------------------------------
// 工具指令（插入到 system prompt 中）
// ---------------------------------------------------------------------------

const TOOL_INSTRUCTIONS: &str = r#"### 1. 💰 财务管理 (finance_query)
- 查询多币种财务摘要、未核销流水、科目树
- 可自动整理数据并输出分析报告
- 用途："查账"、"财务汇总"、"待核销流水"

### 2. 📢 广播群发 (broadcast_message)
- 向用户/群组/频道发送消息
- 支持文本、图片、自定义按钮
- ⚠️ 危险操作：每次都需要管理员确认
- 用途："群发通知"、"推送文案"

### 3. ✍️ 内容生成 (generate_content)
- 生成营销文案、产品介绍、公告、话术
- 支持指定风格(专业/轻松/热情)、长度、受众
- 用途："帮我写个...","生成一段..."

### 4. 🛡️ 广告检测 (detect_advertising)
- 识别广告/垃圾信息/推广意图
- 输出广告评分 + 详细发现 + 处理建议
- 用途："这是不是广告"、"检测这条消息"

### 5. 🔍 系统查询 (system_query)
- 统计概览、活跃群组、监听地址列表
- 最近消息、潜在用户统计
- 用途："系统状态"、"有多少群"、"最近动态"

### 6. 🧩 Skill 管理 (skill_manage)
- 查看/新增/更新/停用 AI 助手技能
- 可将已有技能模板加入助手能力
- 用途："新增 skill"、"列出技能"、"更新技能提示词"

### 7. 🛡️ 广告聚合 (query_ad_clusters)
- 查询当前租户已落库群消息中的重复广告候选
- 没有已落库数据时明确说明数据不足，不推断为没有广告
- 用途："查广告簇"、"查重复广告"、"最近广告聚合"

### 8. 🔎 最近广告快照 (query_recent_ad_messages)
- 查询指定时间窗口（默认 7 天）内的广告消息并生成快照，供确认删除
- 查询窗口为空时，务必引用返回中的"近 24 小时自动清理统计"，不要直接回答"没有广告"了事
- 用途："清理最近广告"、"查最近广告消息"、"删除广告"

### 9. 🗑️ 删除广告 · 需确认 (delete_recent_ad_messages)
- 删除已锁定快照中的广告消息；必须先经 query_recent_ad_messages 生成快照并获管理员确认
- ⚠️ 危险操作：永远不能由模型直接触发，必须走确认流程
- 用途：管理员确认后执行广告删除
"#;

// ---------------------------------------------------------------------------
// 构建完整 system prompt
// ---------------------------------------------------------------------------

/// 优先从文件系统加载 prompt 模板（改 md 重启即生效，无需重新编译）；
/// 找不到文件时回退到编译期内置默认值（保证任何环境可运行）。
fn load_prompt_file(file_name: &str, fallback: &str) -> String {
    let base_candidates: Vec<String> = [
        std::env::var("TGBOT_PROJECT_ROOT").ok(),
        std::env::current_dir().ok().map(|p| p.display().to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut candidates = Vec::new();
    for base in &base_candidates {
        candidates.push(format!("{}/crates/agent/prompts/{}", base, file_name));
        candidates.push(format!("{}/prompts/{}", base, file_name));
        candidates.push(format!("{}/{}", base, file_name));
    }
    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                return content;
            }
        }
    }
    fallback.to_string()
}

/// 构建完整的 system prompt
pub fn build_system_prompt(ctx: Option<&AgentContext>) -> String {
    let current_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let mut extra_skill_text = String::new();
    if let Some(ctx) = ctx {
        if let Some(custom_skills) = ctx.metadata.get("custom_skills") {
            if let Some(skills) = custom_skills.as_array() {
                let mut lines = Vec::new();
                for s in skills.iter().take(20) {
                    let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
                    let desc = s
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("自定义技能")
                        .trim();
                    if !name.is_empty() {
                        lines.push(format!("- {}: {}", name, desc));
                    }
                }
                if !lines.is_empty() {
                    extra_skill_text =
                        format!("\n\n### 12. 🧠 已启用自定义技能\n{}", lines.join("\n"));
                }
            }
        }
    }

    // 声明式：优先 prompts/*.md 文件（改 md 重启生效），回退内置 const
    let tool_instructions = format!(
        "{}{}",
        load_prompt_file("tools.md", TOOL_INSTRUCTIONS),
        extra_skill_text
    );
    let template = load_prompt_file("system.md", AGENT_SYSTEM_TEMPLATE);

    template
        .replace("{bot_name}", "TGBot AI")
        .replace("{tool_instructions}", &tool_instructions)
        .replace("{current_time}", &current_time)
}

// ---------------------------------------------------------------------------
// Function Calling 兼容格式 prompt
// ---------------------------------------------------------------------------

/// 构造一个引导 LLM 进行 function calling 的 prompt
/// 用于不支持原生 function calling 的 LLM（通过文本解析实现）
pub fn build_tool_use_prompt(user_msg: &str, available_tools: &[serde_json::Value]) -> String {
    let mut tools_desc = Vec::new();
    for t in available_tools {
        let (name, desc, params) = if let Some(func) = t.get("function") {
            let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let desc = func
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let props = func
                .get("parameters")
                .and_then(|p| p.get("properties"))
                .and_then(|p| p.as_object());
            let param_str = props
                .map(|obj| obj.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            (name, desc, param_str)
        } else {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("?");
            (name, desc, String::new())
        };
        tools_desc.push(if params.is_empty() {
            format!("- **{}**: {}", name, desc)
        } else {
            format!("- **{}**({}): {}", name, params, desc)
        });
    }

    format!(
        r#"请分析以下用户请求，判断是否需要调用工具。

可用工具:
{tools_desc}

如果需要调用工具，请严格按照以下 JSON 格式返回（不要返回其他内容）:
__TOOL_CALL__{{"function": {{"name": "工具名", "arguments": {{参数}}}}}}

如果不需要调用工具（普通对话），直接正常回复即可。

用户请求: {user_msg}"#,
        tools_desc = tools_desc.join("\n"),
        user_msg = user_msg,
    )
}

// ---------------------------------------------------------------------------
// 工具指令文本（导出供外部使用）
// ---------------------------------------------------------------------------

/// 获取工具指令文本
pub fn get_tool_instructions() -> &'static str {
    TOOL_INSTRUCTIONS
}

/// 获取 Agent 系统模板
pub fn get_agent_system_template() -> &'static str {
    AGENT_SYSTEM_TEMPLATE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::AgentContext;

    #[test]
    fn test_build_system_prompt_default() {
        let prompt = build_system_prompt(None);
        assert!(prompt.contains("TGBot AI"));
        assert!(prompt.contains("财务管理"));
        assert!(prompt.contains("广播群发"));
    }

    #[test]
    fn test_build_system_prompt_with_context() {
        let mut ctx = AgentContext::new(123);
        ctx.metadata.insert(
            "custom_skills".into(),
            serde_json::json!([{"name": "test_skill", "description": "A test skill"}]),
        );
        let prompt = build_system_prompt(Some(&ctx));
        assert!(prompt.contains("test_skill"));
        assert!(prompt.contains("已启用自定义技能"));
    }

    #[test]
    fn test_build_tool_use_prompt() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "finance_query",
                "description": "查询财务数据",
                "parameters": {
                    "type": "object",
                    "properties": {"action": {"type": "string"}},
                    "required": ["action"]
                }
            }
        })];
        let prompt = build_tool_use_prompt("帮我查财务", &tools);
        assert!(prompt.contains("finance_query"));
        assert!(prompt.contains("__TOOL_CALL__"));
        assert!(prompt.contains("帮我查财务"));
    }
}
