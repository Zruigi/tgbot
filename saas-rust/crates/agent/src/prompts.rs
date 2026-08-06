// AI Agent Prompt 模板 — System Instructions + Tool Calling 引导
// 对应 Python: services/ai_agent/prompts.py

use crate::schema::AgentContext;

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
| "查下最近群广告消息" | 调用 system_query(group_ad_messages) |
| "查最近广告聚合" | 调用 query_ad_clusters |

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
- 最近消息、潜在用户统计、群广告消息
- 用途："系统状态"、"有多少群"、"最近动态"

### 6. 🧩 Skill 管理 (skill_manage)
- 查看/新增/更新/停用 AI 助手技能
- 可将已有技能模板加入助手能力
- 用途："新增 skill"、"列出技能"、"更新技能提示词"

### 7. 🛡️ 广告聚合 (query_ad_clusters)
- 查询当前租户已落库群消息中的重复广告候选
- 没有已落库数据时明确说明数据不足，不推断为没有广告
- 用途："查广告簇"、"查重复广告"、"最近广告聚合"
"#;

// ---------------------------------------------------------------------------
// 构建完整 system prompt
// ---------------------------------------------------------------------------

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
                        format!("\n\n### 7. 🧠 已启用自定义技能\n{}", lines.join("\n"));
                }
            }
        }
    }

    let tool_instructions = format!("{}{}", TOOL_INSTRUCTIONS, extra_skill_text);

    AGENT_SYSTEM_TEMPLATE
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
