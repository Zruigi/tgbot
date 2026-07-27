// AI Agent 核心引擎 — 对话管理 + Function Calling 循环 + 安全约束集成
// 对应 Python: services/ai_agent/core.py

use crate::ad_detection::detect_advertising_signals;
use crate::prompts::build_system_prompt;
use crate::safety::SafetyGuardrail;
use crate::schema::{
    AgentContext, AgentResponse, ChatMessage, LlmCallResult, SingleToolResult, Tool, ToolCall,
    ToolCallFunction, ToolExecuteResult,
};
use crate::tools::broadcast_message::BroadcastMessageTool;
use crate::tools::delete_recent_ad_messages::DeleteRecentAdMessagesTool;
use crate::tools::detect_advertising::DetectAdvertisingTool;
use crate::tools::finance_query::FinanceQueryTool;
use crate::tools::n8n_workflow::N8NWorkflowTool;
use crate::tools::ocr::OcrTool;
use crate::tools::query_recent_ad_messages::QueryRecentAdMessagesTool;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, warn};

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/// 最大循环次数（防无限递归）
const MAX_TOOL_CALL_ROUNDS: usize = 5;

/// 最大对话历史条数
const MAX_HISTORY: usize = 40;

/// 默认 LLM 模型
const DEFAULT_MODEL: &str = "gemini-2.5-flash-lite";

// ---------------------------------------------------------------------------
// LlmCaller trait — 抽象 LLM 调用
// ---------------------------------------------------------------------------

/// LLM 调用接口
/// 签名: async fn call(messages, model, tools) -> (text, tool_calls)
#[async_trait]
pub trait LlmCaller: Send + Sync {
    /// 调用 LLM，返回文本和可选的 tool_calls
    async fn call(
        &self,
        messages: &[ChatMessage],
        model: &str,
        tools: &[Value],
    ) -> Result<LlmCallResult, anyhow::Error>;
}

// ---------------------------------------------------------------------------
// ToolRegistry — 工具注册表
// ---------------------------------------------------------------------------

/// 工具注册表
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// 注册工具
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// 按名称获取工具
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// 获取所有已注册的工具
    pub fn all(&self) -> Vec<&Arc<dyn Tool>> {
        self.tools.values().collect()
    }

    /// 获取所有工具名
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// 获取所有工具的 LLM function calling schema
    pub fn schemas_for_llm(&self) -> Vec<Value> {
        self.tools.values().map(|t| t.schema_for_llm()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// AIAgent — 核心引擎
// ---------------------------------------------------------------------------

/// AI Agent 核心引擎
///
/// 功能：
/// 1. 维护对话上下文
/// 2. 解析 LLM 的 function calling 指令
/// 3. 安全检查后执行工具
/// 4. 将结果反馈给 LLM 形成最终回复
pub struct AIAgent {
    llm_caller: Arc<dyn LlmCaller>,
    safety: SafetyGuardrail,
    tools: Arc<ToolRegistry>,
}

impl AIAgent {
    /// 创建新的 Agent 实例
    pub fn new(llm_caller: Arc<dyn LlmCaller>, tools: Arc<ToolRegistry>) -> Self {
        Self {
            llm_caller,
            safety: SafetyGuardrail::new(),
            tools,
        }
    }

    /// 获取所有可用工具名
    pub fn available_tools(&self) -> Vec<String> {
        self.tools.names()
    }

    /// 处理一条用户消息，返回 Agent 响应
    ///
    /// 主入口函数：
    /// 1. 构建含 system prompt + tool definitions 的 messages
    /// 2. 调用 LLM
    /// 3. 如果 LLM 返回 tool_calls -> 安全检查 -> 执行 -> 结果回传 LLM -> 直到无 tool_calls
    /// 4. 返回最终回复
    pub async fn process_message(
        &self,
        user_message: &str,
        ctx: &mut AgentContext,
        model: &str,
    ) -> AgentResponse {
        // Deterministic pre-route: check for ad deletion request (no LLM)
        if let Some(deletion_response) = self.prepare_recent_ad_deletion(user_message, ctx).await {
            ctx.conversation_history
                .push(ChatMessage::user(user_message));
            ctx.conversation_history
                .push(ChatMessage::assistant(&deletion_response.text));
            if ctx.conversation_history.len() > MAX_HISTORY {
                ctx.conversation_history = ctx.conversation_history
                    [ctx.conversation_history.len() - MAX_HISTORY..]
                    .to_vec();
            }
            return deletion_response;
        }

        // 构建初始消息
        let system_prompt = build_system_prompt(Some(ctx));

        let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(&system_prompt)];
        messages.extend(ctx.conversation_history.iter().cloned());
        messages.push(ChatMessage::user(user_message));

        // 获取工具 schemas
        let tool_schemas = self.tools.schemas_for_llm();

        // Function Calling 循环
        let mut round_num = 0;
        let mut tool_calls_made: Vec<String> = Vec::new();
        let mut tool_results: HashMap<String, Value> = HashMap::new();
        let mut action_taken = false;
        let mut needs_confirmation = false;
        let mut final_text = String::new();

        while round_num < MAX_TOOL_CALL_ROUNDS {
            round_num += 1;

            let result = match self
                .call_llm(&messages, model, &tool_schemas, round_num)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("AIAgent process_message 第{}轮异常: {}", round_num, e);
                    final_text = format!(
                        "抱歉，处理过程中出现了问题：{}",
                        &format!("{}", e).chars().take(200).collect::<String>()
                    );
                    break;
                }
            };

            // 如果没有 tool_calls，这就是最终回复
            if result.tool_calls.is_empty() {
                final_text = result.text;
                break;
            }

            // 处理 tool_calls
            messages.push(ChatMessage::assistant(&result.text));

            let mut all_results: Vec<SingleToolResult> = Vec::new();
            let mut has_dangerous = false;

            for tc in &result.tool_calls {
                let tool_name = &tc.function.name;
                let args = tc.function.parse_arguments();

                // HARD BLOCK: refuse LLM-emitted delete_recent_ad_messages calls (safety gate c816441 item a)
                // Deletion may only enter via deterministic pre-route
                if tool_name == "delete_recent_ad_messages" {
                    all_results.push(SingleToolResult {
                        tool_name: tool_name.clone(),
                        result: None,
                        success: false,
                        error: Some(
                            "Ad deletion can only be performed via deterministic pre-route, not LLM emission".into(),
                        ),
                        blocked: true,
                        needs_confirmation: false,
                    });
                    continue;
                }

                let tool_instance = match self.tools.get(tool_name) {
                    Some(t) => t,
                    None => {
                        all_results.push(SingleToolResult {
                            tool_name: tool_name.clone(),
                            result: None,
                            success: false,
                            error: Some(format!("未知工具: {}", tool_name)),
                            blocked: false,
                            needs_confirmation: false,
                        });
                        continue;
                    }
                };

                tool_calls_made.push(tool_name.clone());

                // 安全检查
                let safety_result = self
                    .safety
                    .check_tool_call(tool_name, &args, ctx.user_id)
                    .await;

                if !safety_result.passed {
                    all_results.push(SingleToolResult {
                        tool_name: tool_name.clone(),
                        result: None,
                        success: false,
                        error: Some(safety_result.reason.clone()),
                        blocked: true,
                        needs_confirmation: false,
                    });
                    continue;
                }

                if safety_result.needs_confirmation {
                    // 危险操作先挂起，不在此轮执行
                    has_dangerous = true;
                    needs_confirmation = true;
                    let exec_params = safety_result
                        .sanitized_params
                        .clone()
                        .unwrap_or(args.clone());
                    let pending_payload = serde_json::json!({
                        "needs_confirmation": true,
                        "_params": exec_params,
                        "message": "操作待确认",
                        "success": true,
                        "action_taken": false,
                    });
                    tool_results.insert(tool_name.clone(), pending_payload.clone());
                    all_results.push(SingleToolResult {
                        tool_name: tool_name.clone(),
                        result: Some(pending_payload),
                        success: true,
                        error: None,
                        blocked: false,
                        needs_confirmation: true,
                    });
                    continue;
                }

                // 执行工具
                let exec_params = safety_result
                    .sanitized_params
                    .clone()
                    .unwrap_or(args.clone());

                let exec_result = match tool_instance.execute(&exec_params, ctx).await {
                    r => r,
                };

                // 检查执行结果是否失败
                if !exec_result.success {
                    all_results.push(SingleToolResult {
                        tool_name: tool_name.clone(),
                        result: None,
                        success: false,
                        error: Some(
                            exec_result
                                .message
                                .clone()
                                .unwrap_or_else(|| "工具执行失败".into()),
                        ),
                        blocked: false,
                        needs_confirmation: false,
                    });
                    continue;
                }

                if exec_result.action_taken {
                    action_taken = true;
                }

                // 如果是内容生成工具的特殊请求
                let mut result_value = exec_result.to_json_value();
                let mut generation_error: Option<String> = None;
                if exec_result.requires_llm_generation {
                    let gen_prompt = exec_result
                        .data
                        .as_ref()
                        .and_then(|data| data.get("generate_prompt"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|prompt| !prompt.is_empty());
                    let Some(gen_prompt) = gen_prompt else {
                        let error = "工具请求内容生成但未提供有效提示词".to_string();
                        // Keep the error visible to callers instead of returning
                        // the tool's optimistic success flag.
                        if let Some(object) = result_value.as_object_mut() {
                            object.insert("success".into(), Value::Bool(false));
                        }
                        all_results.push(SingleToolResult {
                            tool_name: tool_name.clone(),
                            result: Some(result_value),
                            success: false,
                            error: Some(error),
                            blocked: false,
                            needs_confirmation: false,
                        });
                        continue;
                    };
                    match self
                        .call_llm(&[ChatMessage::user(gen_prompt)], model, &[], round_num)
                        .await
                    {
                        Ok(gen_result) => {
                            result_value.as_object_mut().map(|obj| {
                                obj.insert(
                                    "generated_content".into(),
                                    Value::String(gen_result.text),
                                );
                                obj.remove("requires_llm_generation");
                            });
                        }
                        Err(e) => {
                            let message = format!("LLM generation for tool failed: {e}");
                            warn!("{}", message);
                            generation_error = Some(message);
                            if let Some(object) = result_value.as_object_mut() {
                                object.insert("success".into(), Value::Bool(false));
                            }
                        }
                    }
                }

                if let Some(error) = generation_error {
                    all_results.push(SingleToolResult {
                        tool_name: tool_name.clone(),
                        result: Some(result_value),
                        success: false,
                        error: Some(error),
                        blocked: false,
                        needs_confirmation: false,
                    });
                    continue;
                }

                // 只有执行成功且需要确认时才走确认流程
                let tool_needs_confirm = exec_result.needs_confirmation && exec_result.success;

                all_results.push(SingleToolResult {
                    tool_name: tool_name.clone(),
                    result: Some(result_value.clone()),
                    success: true,
                    error: None,
                    blocked: false,
                    needs_confirmation: tool_needs_confirm,
                });

                // 保存参数，便于确认后重放
                let mut stored_result = result_value.clone();
                if let Some(obj) = stored_result.as_object_mut() {
                    obj.insert("_params".into(), exec_params.clone());
                    obj.insert("needs_confirmation".into(), Value::Bool(tool_needs_confirm));
                }
                tool_results.insert(tool_name.clone(), stored_result);

                // 记录审计日志
                self.safety
                    .log_action(tool_name, &args, &result_value, ctx.user_id);
            }

            // 将工具结果加入 messages，让 LLM 总结回复
            let results_summary = self.format_tool_results(&all_results);

            if has_dangerous {
                messages.push(ChatMessage::user(&format!(
                    "[系统提示: 工具返回结果]\n{}\n\n请根据以上结果生成回复。如果有危险操作待确认，请提醒用户确认。",
                    results_summary
                )));
            } else {
                messages.push(ChatMessage::user(&format!(
                    "[工具执行结果]\n{}\n\n请基于以上结果简洁回复用户。",
                    results_summary
                )));
            }

            // 如果有危险操作需要确认，提前退出让用户确认
            if has_dangerous {
                final_text = format!(
                    "{}\n\n⚠️ 以上操作需要您确认后才能执行。请回复「确认」继续，或取消操作。",
                    results_summary
                );
                break;
            }
        }

        if final_text.is_empty() {
            final_text = "我已收到您的请求，但未能生成合适的回复。".into();
        }

        // 输出脱敏
        final_text = self.safety.sanitize_output(&final_text).await;

        // 更新对话历史
        ctx.conversation_history
            .push(ChatMessage::user(user_message));
        ctx.conversation_history
            .push(ChatMessage::assistant(&final_text));

        // 保持历史在合理范围内
        ctx.trim_history(MAX_HISTORY);

        let confidence = if needs_confirmation { 0.6 } else { 0.9 };

        let tcm_summary = tool_calls_made.join(", ");

        AgentResponse {
            text: final_text,
            tool_calls_made,
            tool_results,
            is_action_taken: action_taken,
            action_summary: tcm_summary,
            requires_confirmation: needs_confirmation,
            confidence,
            raw_response: None,
        }
    }

    /// 用户确认后执行之前被暂停的危险操作
    pub async fn confirm_and_execute(
        &self,
        pending_response: &AgentResponse,
        ctx: &AgentContext,
    ) -> AgentResponse {
        if !pending_response.requires_confirmation {
            return pending_response.clone();
        }

        let mut executed_results: HashMap<String, Value> = HashMap::new();
        let mut action_taken = false;
        let mut has_failure = false;
        let mut failure_messages: Vec<String> = Vec::new();
        let mut confirmation_items = 0usize;

        for (tool_name, prev_result) in &pending_response.tool_results {
            let needs_confirm = prev_result
                .get("needs_confirmation")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if !needs_confirm {
                continue;
            }

            confirmation_items += 1;

            let tool_instance = match self.tools.get(tool_name) {
                Some(t) => t,
                None => {
                    has_failure = true;
                    let message = format!("{}: 待确认工具未注册，本次未执行", tool_name);
                    failure_messages.push(message.clone());
                    executed_results.insert(
                        tool_name.clone(),
                        serde_json::json!({
                            "success": false,
                            "action_taken": false,
                            "message": message,
                        }),
                    );
                    continue;
                }
            };

            let params = prev_result
                .get("_params")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            match tool_instance.execute(&params, ctx).await {
                result => {
                    let mut result_value = result.to_json_value();

                    if result.action_taken {
                        action_taken = true;
                    }

                    if !result.success {
                        has_failure = true;
                        failure_messages.push(format!(
                            "{}: {}",
                            tool_name,
                            result.message.unwrap_or_else(|| "执行失败".into())
                        ));
                    } else if !result.action_taken {
                        // A dangerous confirmation is only successful when the tool reports
                        // an actual side effect. A successful no-op must never be presented as
                        // "操作已执行完毕".
                        has_failure = true;
                        failure_messages.push(format!(
                            "{}: 工具未报告实际动作，本次按未执行处理",
                            tool_name
                        ));
                        if let Some(result_object) = result_value.as_object_mut() {
                            result_object.insert("success".into(), Value::Bool(false));
                            result_object.insert("fail_closed".into(), Value::Bool(true));
                        }
                    }
                    executed_results.insert(tool_name.clone(), result_value);
                }
            }
        }

        if confirmation_items == 0 {
            has_failure = true;
            failure_messages.push("没有找到可执行的待确认工具，本次未执行任何操作".into());
        }

        // Build result text based on execution status
        // Behavior 1: Only return THIS execution's result, never re-append old confirmation prompt
        let result_text = if has_failure && action_taken {
            // Partial execution: some actions succeeded but some failed
            format!(
                "操作已部分执行，但有失败项：\n{}",
                failure_messages
                    .iter()
                    .map(|msg| format!("- {}", msg))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        } else if has_failure {
            // All failed: no actions executed
            format!(
                "操作未执行：\n{}",
                failure_messages
                    .iter()
                    .map(|msg| format!("- {}", msg))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        } else {
            // All succeeded
            let mut text = "操作已执行完毕。".to_string();

            // Append individual tool result messages if any
            let result_messages: Vec<String> = executed_results
                .values()
                .filter_map(|result| {
                    result
                        .get("message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .collect();

            if !result_messages.is_empty() {
                text.push('\n');
                for msg in result_messages {
                    text.push_str(&format!("- {}\n", msg));
                }
                text.pop(); // Remove trailing newline
            }

            text
        };

        let confidence = if has_failure { 0.7 } else { 1.0 };

        AgentResponse {
            text: result_text, // ← 关键：仅返回执行结果，不拼接 pending_response.text
            tool_calls_made: pending_response.tool_calls_made.clone(),
            tool_results: executed_results,
            is_action_taken: action_taken,
            action_summary: pending_response.action_summary.clone(),
            requires_confirmation: false,
            confidence,
            raw_response: None,
        }
    }

    // ---- 内部方法 ----

    /// 调用 LLM 并解析响应
    async fn call_llm(
        &self,
        messages: &[ChatMessage],
        model: &str,
        tools: &[Value],
        round_num: usize,
    ) -> Result<LlmCallResult, anyhow::Error> {
        let model = if model.is_empty() {
            DEFAULT_MODEL
        } else {
            model
        };

        match self.llm_caller.call(messages, model, tools).await {
            Ok(mut result) => {
                // 尝试从返回文本中解析 function calling（显式标记模式）
                // 2.2d: 先 trim 再检查前缀（容忍前导/尾随空白，deb8eaf）
                let mut tool_calls: Vec<ToolCall> = Vec::new();
                let stripped_text = result.text.trim();
                if stripped_text.starts_with("__TOOL_CALL__") {
                    let json_str = &stripped_text["__TOOL_CALL__".len()..];
                    match serde_json::from_str::<Value>(json_str) {
                        Ok(parsed) => {
                            if let Some(arr) = parsed.as_array() {
                                for item in arr {
                                    if let Some(tc) = parse_tool_call_value(item) {
                                        tool_calls.push(tc);
                                    }
                                }
                            } else if let Some(tc) = parse_tool_call_value(&parsed) {
                                tool_calls.push(tc);
                            }
                        }
                        Err(e) => {
                            warn!("Tool call JSON 解析失败，当作纯文本: {}", e);
                        }
                    }
                }

                // 兜底：当模型没按约定返回 tool_call 时，用"最新用户消息"做启发式匹配
                if tools.is_empty() == false && tool_calls.is_empty() && round_num == 1 {
                    let latest_user_text = messages
                        .iter()
                        .rev()
                        .find_map(|m| {
                            if m.role == "user" {
                                m.content.as_deref()
                            } else {
                                None
                            }
                        })
                        .unwrap_or("");

                    let stripped = latest_user_text.trim();
                    if stripped.len() >= 4 {
                        if let Some(matched_name) = self.match_tool_from_text(stripped, tools) {
                            let fallback_args = self.build_fallback_args(&matched_name, stripped);
                            tool_calls.push(ToolCall {
                                function: ToolCallFunction {
                                    name: matched_name,
                                    arguments: serde_json::to_string(&fallback_args)
                                        .unwrap_or_else(|_| "{}".into()),
                                },
                            });
                        }
                    }
                }

                result.tool_calls = tool_calls;
                Ok(result)
            }
            Err(e) => {
                error!("LLM 调用异常: {}", e);
                Ok(LlmCallResult {
                    text: format!(
                        "[LLM 错误] {}",
                        &format!("{}", e).chars().take(200).collect::<String>()
                    ),
                    tool_calls: vec![],
                })
            }
        }
    }

    /// 根据用户文本做启发式工具匹配（仅作为 LLM 未显式返回 tool_call 时的兜底）
    fn match_tool_from_text(&self, text: &str, tools: &[Value]) -> Option<String> {
        if text.is_empty() {
            return None;
        }

        let normalized = text.to_lowercase();

        let keyword_map: HashMap<&str, Vec<&str>> = {
            let mut m = HashMap::new();
            m.insert(
                "finance_query",
                vec![
                    "查财务",
                    "财务摘要",
                    "财务查询",
                    "财务报表",
                    "财务分析",
                    "查账目",
                    "查流水",
                    "看账目",
                    "看流水",
                    "未核销",
                    "待核销",
                    "核销流水",
                    "对账单",
                    "对个账",
                ],
            );
            m.insert(
                "broadcast_message",
                vec![
                    "群发消息",
                    "群发通知",
                    "发送广播",
                    "给所有人发",
                    "群发公告",
                    "批量发送",
                ],
            );
            m.insert(
                "detect_advertising",
                vec![
                    "检测广告",
                    "是不是广告",
                    "广告检测",
                    "垃圾信息检测",
                    "是否广告",
                    "鉴别广告",
                    "这条是不是广告",
                    "这段是不是广告",
                    "鉴别这条",
                    "检测这条消息",
                ],
            );
            m.insert(
                "telegram_sdk_query",
                vec![
                    "机器人权限",
                    "bot权限",
                    "删除权限",
                    "群权限",
                    "频道权限",
                    "机器人能不能删除",
                    "为什么删除失败",
                    "telegram sdk",
                ],
            );
            m.insert(
                "generate_content",
                vec![
                    "帮我写",
                    "生成文案",
                    "写一段",
                    "生成内容",
                    "帮我生成",
                    "写个文案",
                    "写公告",
                ],
            );
            m.insert(
                "system_query",
                vec![
                    "系统状态",
                    "系统概览",
                    "有多少群",
                    "群组统计",
                    "系统信息",
                    "活跃群组",
                    "最近消息",
                    "最近有谁",
                    "谁给我发",
                    "谁给我发送",
                    "最近发消息",
                    "广告消息",
                    "群广告",
                    "疑似广告",
                ],
            );
            m.insert(
                "skill_manage",
                vec![
                    "新增skill",
                    "增加skill",
                    "添加skill",
                    "技能管理",
                    "列出技能",
                    "查看技能",
                    "更新技能",
                    "导入技能",
                    "新增技能",
                    "添加技能",
                ],
            );
            m
        };

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

            if let Some(keywords) = keyword_map.get(name.as_str()) {
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

        // 白名单工具 >= 8 即匹配,其他工具需 >= 16（deb8eaf 确定性路由）
        const WHITELIST: &[&str] = &[
            "finance_query",
            "system_query",
            "telegram_sdk_query",
            "detect_advertising",
            "query_ad_clusters",
            "query_recent_ad_messages",
        ];
        match &best_match {
            Some(name) if WHITELIST.contains(&name.as_str()) && best_score >= 8 => best_match,
            Some(_) if best_score >= 16 => best_match,
            _ => None,
        }
    }

    /// 当模型未返回参数时，基于用户原文构造一组可执行参数
    fn build_fallback_args(&self, tool_name: &str, user_text: &str) -> Value {
        let text = user_text.trim();
        if text.is_empty() {
            return serde_json::json!({});
        }

        match tool_name {
            "broadcast_message" => {
                let mut content = text.to_string();
                for kw in &[
                    "群发消息",
                    "群发通知",
                    "发送广播",
                    "给所有人发",
                    "群发公告",
                    "批量发送",
                    "测试",
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
            "telegram_sdk_query" => {
                // 4c3f1dc: 数字 group_id 优先于群名解析
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
                // 42bafb0: bound_chats SDK 验证 limit 50
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
                let _lower = text.to_lowercase();
                if text.contains("最近有谁")
                    || text.contains("谁给我发")
                    || text.contains("谁给我发送")
                    || text.contains("最近发消息")
                    || text.contains("谁发了消息")
                {
                    serde_json::json!({"query_type": "recent_senders", "limit": 10})
                } else if text.contains("最近消息") {
                    serde_json::json!({"query_type": "recent_messages", "limit": 10})
                } else if text.contains("广告消息")
                    || text.contains("群广告")
                    || text.contains("疑似广告")
                    || text.contains("广告群消息")
                {
                    serde_json::json!({"query_type": "group_ad_messages", "limit": 20})
                } else if text.contains("潜在用户") {
                    serde_json::json!({"query_type": "potential_users", "limit": 10})
                } else if text.contains("监听")
                    || text.contains("地址列表")
                    || text.contains("监控地址")
                {
                    serde_json::json!({"query_type": "monitors_list", "limit": 10})
                } else if text.contains("群") || text.contains("群组") {
                    serde_json::json!({"query_type": "active_groups", "limit": 10})
                } else if text.contains("统计") || text.contains("概览") || text.contains("状态")
                {
                    serde_json::json!({"query_type": "stats_overview"})
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

    /// Check if user text is requesting recent ad deletion
    /// Applies negation/question guards from spec-2.2c
    fn is_recent_ad_delete_request(&self, text: &str) -> bool {
        let normalized = text
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();

        // Must have both delete and ad keywords
        let has_delete = ["删除", "清理", "移除", "删掉", "执行删除"]
            .iter()
            .any(|w| normalized.contains(w));
        let has_ad = ["广告", "垃圾信息", "牛皮癣"]
            .iter()
            .any(|w| normalized.contains(w));

        if !has_delete || !has_ad {
            return false;
        }

        // Negation guard: reject if contains negation phrases
        let is_negated = ["不要删除", "别删除", "禁止删除", "无需删除", "不需要删除"]
            .iter()
            .any(|w| normalized.contains(w));
        if is_negated {
            return false;
        }

        // Question guard: reject if starts with question phrases
        let is_question = ["为什么", "为何", "怎么会"]
            .iter()
            .any(|w| normalized.starts_with(w));
        if is_question {
            return false;
        }

        // Single item guard: reject if contains "一条" without time keywords
        let requests_single = normalized.contains("一条")
            && !["最近", "24小时", "小时内", "今天"]
                .iter()
                .any(|w| normalized.contains(w));
        if requests_single {
            return false;
        }

        true
    }

    /// Deterministic pre-route for ad deletion: if user requests deletion, query and snapshot
    /// instead of letting LLM handle it. Returns Some(AgentResponse) to short-circuit LLM
    async fn prepare_recent_ad_deletion(
        &self,
        user_text: &str,
        ctx: &mut AgentContext,
    ) -> Option<AgentResponse> {
        if !self.is_recent_ad_delete_request(user_text) {
            return None;
        }

        // Parse hours from text (default 24, clamp to 1-24)
        let hours = if let Some(caps) = regex::Regex::new(r"(\d{1,3})\s*(?:小时|h)")
            .ok()
            .and_then(|re| re.captures(user_text))
        {
            caps.get(1)
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .unwrap_or(24)
                .max(1)
                .min(24)
        } else {
            24
        };

        // Parse requested limit from text (default 50, clamp to 1-50)
        let requested_limit = if let Some(caps) = regex::Regex::new(r"(?:最近\s*)?(\d{1,2})\s*条")
            .ok()
            .and_then(|re| re.captures(user_text))
        {
            caps.get(1)
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .unwrap_or(50)
                .max(1)
                .min(50)
        } else {
            50
        };

        // Parse group ID if present
        let group_match = if let Some(caps) =
            regex::Regex::new(r"(?:group_id|群|群组)\s*[:：=]?\s*(-?\d{5,})")
                .ok()
                .and_then(|re| re.captures_iter(user_text).next())
        {
            caps.get(1).and_then(|m| m.as_str().parse::<i64>().ok())
        } else {
            None
        };

        let params = serde_json::json!({
            "hours": hours,
            "limit": requested_limit,
            "group_id": group_match,
        });
        let Some(tool) = self.tools.get("query_recent_ad_messages") else {
            return Some(AgentResponse {
                text: "广告查询工具未注册，本次没有执行任何查询。".into(),
                tool_calls_made: vec![],
                tool_results: HashMap::new(),
                is_action_taken: false,
                action_summary: "query_recent_ad_messages unavailable".into(),
                requires_confirmation: false,
                confidence: 1.0,
                raw_response: None,
            });
        };
        let exec_result = tool.execute(&params, ctx).await;
        let result_value = exec_result.to_json_value();
        let message = exec_result
            .message
            .clone()
            .unwrap_or_else(|| "广告查询未完成".into());
        let text = if exec_result.success && exec_result.needs_confirmation {
            format!("{}，请确认后再执行删除。", message)
        } else if exec_result.success {
            message.clone()
        } else {
            format!("广告查询失败：{}", message)
        };
        let mut tool_results = HashMap::new();
        tool_results.insert("query_recent_ad_messages".into(), result_value);
        Some(AgentResponse {
            text,
            tool_calls_made: vec!["query_recent_ad_messages".into()],
            tool_results,
            is_action_taken: false,
            action_summary: format!(
                "查询 {} 小时广告（limit {}，{}group_id {}）: {}",
                hours,
                requested_limit,
                if group_match.is_some() { "" } else { "no " },
                group_match.unwrap_or(0),
                message
            ),
            requires_confirmation: exec_result.success && exec_result.needs_confirmation,
            confidence: if exec_result.success { 1.0 } else { 0.0 },
            raw_response: None,
        })
    }

    /// 将工具结果格式化为 LLM 可读的总结
    fn format_tool_results(&self, all_results: &[SingleToolResult]) -> String {
        let mut parts: Vec<String> = Vec::new();

        for r in all_results {
            let name = &r.tool_name;
            if r.blocked {
                parts.push(format!(
                    "- [{}] ❌ 已拦截: {}",
                    name,
                    r.error.as_deref().unwrap_or("安全原因")
                ));
            } else if r.error.is_some() {
                if !r.success {
                    parts.push(format!(
                        "- [{}] ❌ 执行失败: {}",
                        name,
                        r.error.as_deref().unwrap_or("")
                    ));
                } else {
                    parts.push(format!(
                        "- [{}] ⚠️ 执行出错: {}",
                        name,
                        r.error.as_deref().unwrap_or("")
                    ));
                }
            } else if !r.success {
                parts.push(format!("- [{}] ❌ 执行失败", name));
            } else {
                let result = r.result.as_ref();
                let msg = result
                    .and_then(|v| v.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("完成");
                let data_preview: String = result
                    .and_then(|v| v.get("data"))
                    .map(|d| {
                        let s = d.to_string();
                        s.chars().take(200).collect()
                    })
                    .unwrap_or_default();
                let conf_note = if r.needs_confirmation {
                    " ⚠️ 需确认"
                } else {
                    ""
                };
                parts.push(format!(
                    "- [{}] ✅ {}{}\n  数据: {}",
                    name, msg, conf_note, data_preview
                ));
            }
        }

        format!("工具执行结果:\n{}", parts.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// 辅助：解析 ToolCall JSON Value
// ---------------------------------------------------------------------------

fn parse_tool_call_value(val: &Value) -> Option<ToolCall> {
    let func = val.get("function")?;
    let name = func.get("name")?.as_str()?.to_string();
    let arguments = func
        .get("arguments")
        .map(|v| {
            if v.is_string() {
                v.as_str().unwrap_or("{}").to_string()
            } else {
                v.to_string()
            }
        })
        .unwrap_or_else(|| "{}".into());

    Some(ToolCall {
        function: ToolCallFunction { name, arguments },
    })
}

// ---------------------------------------------------------------------------
// 全局单例管理
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

static GLOBAL_AGENT: OnceLock<Arc<AIAgent>> = OnceLock::new();

/// 获取全局 Agent 实例（单例）
/// 首次调用时必须通过 init_global_agent 初始化
pub fn get_agent() -> Option<Arc<AIAgent>> {
    GLOBAL_AGENT.get().cloned()
}

/// 初始化全局 Agent 实例
pub fn init_global_agent(llm_caller: Arc<dyn LlmCaller>, tools: Arc<ToolRegistry>) -> Arc<AIAgent> {
    let agent = Arc::new(AIAgent::new(llm_caller, tools));
    let _ = GLOBAL_AGENT.set(agent.clone());
    agent
}

// ---------------------------------------------------------------------------
// 内置工具实现（广告检测，纯计算不依赖外部服务）
// ---------------------------------------------------------------------------

/// 广告检测工具（内置，纯逻辑无外部依赖）
pub struct AdDetectorTool;

#[async_trait]
impl Tool for AdDetectorTool {
    fn name(&self) -> &str {
        "detect_advertising"
    }

    fn description(&self) -> &str {
        "检测文本中是否包含广告、垃圾信息、违规内容。支持：广告关键词匹配、可疑链接检测、重复内容判断、推广意图评分。"
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

        let result = detect_advertising_signals(text, context_type);
        let is_ad = result.is_advertising;
        let ad_score = result.ad_score;
        let confidence = &result.confidence;

        let message = if is_ad {
            format!(
                "⚠️ 检测到广告内容 (得分: {}/100, 置信度: {})",
                ad_score, confidence
            )
        } else {
            format!(
                "✅ 未检出广告 (得分: {}/100, 置信度: {})",
                ad_score, confidence
            )
        };

        ToolExecuteResult::ok_with_data(serde_json::to_value(&result).unwrap_or_default(), &message)
    }
}

/// 内容生成工具（返回 LLM 生成 prompt，由引擎调度 LLM 生成）
pub struct ContentGeneratorTool;

#[async_trait]
impl Tool for ContentGeneratorTool {
    fn name(&self) -> &str {
        "generate_content"
    }

    fn description(&self) -> &str {
        "生成各类文案内容。支持：营销文案、产品介绍、公告通知、社群话术、回复模板等。可指定风格、长度、目标受众。"
    }

    fn category(&self) -> &str {
        "content"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "文案主题或核心信息"
                },
                "style": {
                    "type": "string",
                    "enum": ["professional", "casual", "enthusiastic", "urgent", "friendly"],
                    "description": "文案风格：专业正式/轻松随意/热情洋溢/紧急通知/亲切友好"
                },
                "length": {
                    "type": "string",
                    "enum": ["short", "medium", "long"],
                    "description": "长度：短(100字内)/中(300字内)/长(500字内)"
                },
                "target_audience": {
                    "type": "string",
                    "description": "目标受众描述（如'新用户''VIP客户'）"
                },
                "purpose": {
                    "type": "string",
                    "enum": ["announcement", "marketing", "notification", "greeting", "reply_template"],
                    "description": "用途：公告/营销/通知/问候/回复模板"
                }
            },
            "required": ["topic"]
        })
    }

    async fn execute(&self, params: &Value, _context: &AgentContext) -> ToolExecuteResult {
        let topic = params.get("topic").and_then(|v| v.as_str()).unwrap_or("");
        let style = params
            .get("style")
            .and_then(|v| v.as_str())
            .unwrap_or("professional");
        let length = params
            .get("length")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        let purpose = params
            .get("purpose")
            .and_then(|v| v.as_str())
            .unwrap_or("announcement");
        let audience = params
            .get("target_audience")
            .and_then(|v| v.as_str())
            .unwrap_or("通用用户");

        let style_map: HashMap<&str, &str> = [
            ("professional", "专业正式的语气，用词精准，结构清晰"),
            ("casual", "轻松随意的口语化表达，像朋友聊天一样自然"),
            ("enthusiastic", "充满热情活力的语气，多用感叹号和表情符号"),
            ("urgent", "紧急重要感，强调时效性和行动必要性"),
            ("friendly", "亲切友好的语气，温暖且富有同理心"),
        ]
        .iter()
        .cloned()
        .collect();

        let length_map: HashMap<&str, &str> = [
            ("short", "控制在100字以内"),
            ("medium", "控制在200-300字左右"),
            ("long", "控制在400-600字左右"),
        ]
        .iter()
        .cloned()
        .collect();

        let purpose_map: HashMap<&str, &str> = [
            ("announcement", "官方公告/通知"),
            ("marketing", "产品推广/营销文案"),
            ("notification", "日常提醒/状态通知"),
            ("greeting", "欢迎语/开场白"),
            ("reply_template", "客服回复模板/FAQ 回答"),
        ]
        .iter()
        .cloned()
        .collect();

        let style_text = style_map.get(style).unwrap_or(&"");
        let length_text = length_map.get(length).unwrap_or(&"");
        let purpose_text = purpose_map.get(purpose).unwrap_or(&"");

        let generate_prompt = format!(
            "请生成一篇{}文案。\n\n要求：\n- 主题：{}\n- 风格：{}\n- 字数要求：{}\n- 目标受众：{}\n\n请直接输出文案内容，不需要额外说明。",
            purpose_text, topic, style_text, length_text, audience
        );

        let mut result = ToolExecuteResult::ok(&format!("正在生成{}文案...", purpose_text));
        result.requires_llm_generation = true;
        result.data = Some(serde_json::json!({
            "generate_prompt": generate_prompt,
            "meta": {
                "topic": topic,
                "style": style,
                "length": length,
                "purpose": purpose,
                "audience": audience,
            }
        }));
        result
    }
}

// ---------------------------------------------------------------------------
// 注册内置工具到 ToolRegistry
// ---------------------------------------------------------------------------

/// 创建包含内置工具的默认 ToolRegistry (无 Telegram 客户端，测试用)
pub fn default_tool_registry() -> ToolRegistry {
    default_tool_registry_with_telegram(None)
}

/// 创建包含内置工具的 ToolRegistry，可选传入 Telegram 客户端
pub fn default_tool_registry_with_telegram(
    telegram_client: Option<Arc<tgbot_integrations::telegram::TelegramClient>>,
) -> ToolRegistry {
    default_tool_registry_with_pool_and_telegram(None, telegram_client)
}

/// 创建包含内置工具的 ToolRegistry，包含 pool 和可选的 Telegram 客户端（用于需要数据库的工具）
pub fn default_tool_registry_with_pool_and_telegram(
    pool: Option<Arc<::sqlx::PgPool>>,
    telegram_client: Option<Arc<tgbot_integrations::telegram::TelegramClient>>,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    // 从独立模块加载的工具
    if let Some(ref pool) = pool {
        registry.register(Arc::new(FinanceQueryTool::with_pool(pool.clone())));
    } else {
        registry.register(Arc::new(FinanceQueryTool::new()));
    }
    // 注册 BroadcastMessageTool，传入可选的 Telegram 客户端
    if let (Some(pool), Some(client)) = (pool.as_ref(), telegram_client.clone()) {
        registry.register(Arc::new(BroadcastMessageTool::with_pool_and_telegram(
            pool.clone(),
            client,
        )));
    } else if let Some(client) = telegram_client.clone() {
        registry.register(Arc::new(BroadcastMessageTool::with_telegram(client)));
    } else {
        registry.register(Arc::new(BroadcastMessageTool::new()));
    }
    registry.register(Arc::new(DetectAdvertisingTool));
    registry.register(Arc::new(ContentGeneratorTool));
    // SUB-015: AI Tools - ocr/n8n/system_query/skill_manage
    registry.register(Arc::new(OcrTool::with_default_config().unwrap_or_else(
        |_| {
            // 如果配置失败，创建一个使用默认配置的 client
            let config = tgbot_integrations::paddleocr::PaddleOCRConfig::default();
            let client = tgbot_integrations::paddleocr::PaddleOCRClient::new(config).unwrap();
            OcrTool::new(Arc::new(client))
        },
    )));
    registry.register(Arc::new(
        N8NWorkflowTool::with_default_config().unwrap_or_else(|_| {
            let config = tgbot_integrations::n8n::N8NConfig::default();
            let client = tgbot_integrations::n8n::N8NClient::new(config).unwrap();
            N8NWorkflowTool::new(Arc::new(client))
        }),
    ));

    // Ad deletion tools (spec-2.2b): require pool
    if let Some(pool) = pool {
        if let Some(client) = telegram_client {
            registry.register(Arc::new(QueryRecentAdMessagesTool::new(pool.clone())));
            registry.register(Arc::new(DeleteRecentAdMessagesTool::with_telegram(
                pool, client,
            )));
        } else {
            registry.register(Arc::new(QueryRecentAdMessagesTool::new(pool.clone())));
            registry.register(Arc::new(DeleteRecentAdMessagesTool::new(pool)));
        }
    }

    // 注意：SystemQueryTool 和 SkillManageTool 需要数据库连接
    // 在实际使用时应通过依赖注入传入正确的 pool
    // 这里暂时不注册，调用方需要手动注入
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock LLM Caller for tests
    struct MockLlmCaller;

    #[async_trait]
    impl LlmCaller for MockLlmCaller {
        async fn call(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _tools: &[Value],
        ) -> Result<LlmCallResult, anyhow::Error> {
            Ok(LlmCallResult {
                text: "这是一个模拟回复".into(),
                tool_calls: vec![],
            })
        }
    }

    struct ConfirmNoActionTool;

    #[async_trait]
    impl Tool for ConfirmNoActionTool {
        fn name(&self) -> &str {
            "confirm_no_action"
        }

        fn description(&self) -> &str {
            "test tool that completes without a side effect"
        }

        async fn execute(&self, _params: &Value, _context: &AgentContext) -> ToolExecuteResult {
            ToolExecuteResult::ok("没有执行实际动作")
        }
    }

    struct ConfirmActionTool;

    #[async_trait]
    impl Tool for ConfirmActionTool {
        fn name(&self) -> &str {
            "confirm_action"
        }

        fn description(&self) -> &str {
            "test tool that reports a real side effect"
        }

        async fn execute(&self, _params: &Value, _context: &AgentContext) -> ToolExecuteResult {
            ToolExecuteResult {
                success: true,
                data: None,
                message: Some("实际动作已执行".into()),
                action_taken: true,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            }
        }
    }

    struct ConfirmPartialTool;

    #[async_trait]
    impl Tool for ConfirmPartialTool {
        fn name(&self) -> &str {
            "confirm_partial"
        }

        fn description(&self) -> &str {
            "test tool that reports a partial side effect"
        }

        async fn execute(&self, _params: &Value, _context: &AgentContext) -> ToolExecuteResult {
            ToolExecuteResult {
                success: false,
                data: None,
                message: Some("部分目标失败".into()),
                action_taken: true,
                needs_confirmation: false,
                requires_llm_generation: false,
                generated_content: None,
            }
        }
    }

    fn pending_confirmation(tool_name: Option<&str>) -> AgentResponse {
        let mut tool_results = HashMap::new();
        if let Some(tool_name) = tool_name {
            tool_results.insert(
                tool_name.to_string(),
                serde_json::json!({
                    "needs_confirmation": true,
                    "_params": {"confirm": true},
                }),
            );
        }

        AgentResponse {
            text: "请确认操作".into(),
            tool_calls_made: tool_name
                .map(|name| vec![name.to_string()])
                .unwrap_or_default(),
            tool_results,
            is_action_taken: false,
            action_summary: "待确认".into(),
            requires_confirmation: true,
            confidence: 0.6,
            raw_response: None,
        }
    }

    #[tokio::test]
    async fn test_process_message_basic() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);

        let mut ctx = AgentContext::new(1);
        let response = agent.process_message("你好", &mut ctx, DEFAULT_MODEL).await;

        assert!(!response.text.is_empty());
        assert!(!response.requires_confirmation);
    }

    #[test]
    fn test_match_tool_from_text() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);

        let tool_schemas = agent.tools.schemas_for_llm();

        let result = agent.match_tool_from_text("帮我查一下财务分析", &tool_schemas);
        assert_eq!(result, Some("finance_query".to_string()));

        let result = agent.match_tool_from_text("你好", &tool_schemas);
        assert!(result.is_none());
    }

    #[test]
    fn test_build_fallback_args() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);

        let args = agent.build_fallback_args("finance_query", "帮我查财务分析");
        assert_eq!(args["action"], "analysis");

        let args = agent.build_fallback_args("detect_advertising", "检测这条消息是不是广告");
        assert_eq!(args["context_type"], "private_message");
    }

    #[test]
    fn test_format_tool_results() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);

        let results = vec![SingleToolResult {
            tool_name: "finance_query".into(),
            result: Some(serde_json::json!({"message": "查询完成"})),
            success: true,
            error: None,
            blocked: false,
            needs_confirmation: false,
        }];

        let formatted = agent.format_tool_results(&results);
        assert!(formatted.contains("finance_query"));
        assert!(formatted.contains("查询完成"));
    }

    #[tokio::test]
    async fn test_confirm_and_execute_no_confirm() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);

        let response = AgentResponse {
            text: "回复内容".into(),
            tool_calls_made: vec![],
            tool_results: HashMap::new(),
            is_action_taken: false,
            action_summary: String::new(),
            requires_confirmation: false,
            confidence: 0.9,
            raw_response: None,
        };

        let ctx = AgentContext::new(1);
        let result = agent.confirm_and_execute(&response, &ctx).await;
        assert!(!result.requires_confirmation);
        assert_eq!(result.text, "回复内容");
    }

    #[tokio::test]
    async fn test_confirm_and_execute_fails_closed_without_pending_tool() {
        let agent = AIAgent::new(Arc::new(MockLlmCaller), Arc::new(ToolRegistry::new()));
        let result = agent
            .confirm_and_execute(&pending_confirmation(None), &AgentContext::new(1))
            .await;

        assert!(result.text.contains("未执行"));
        assert!(!result.text.contains("操作已执行完毕"));
        assert!(!result.is_action_taken);
        assert!(result.tool_results.is_empty());
    }

    #[tokio::test]
    async fn test_confirm_and_execute_fails_closed_when_tool_is_missing() {
        let agent = AIAgent::new(Arc::new(MockLlmCaller), Arc::new(ToolRegistry::new()));
        let result = agent
            .confirm_and_execute(
                &pending_confirmation(Some("missing_tool")),
                &AgentContext::new(1),
            )
            .await;

        assert!(result.text.contains("未执行"));
        assert!(!result.text.contains("操作已执行完毕"));
        assert!(!result.is_action_taken);
        assert_eq!(result.tool_results["missing_tool"]["success"], false);
    }

    #[tokio::test]
    async fn test_confirm_and_execute_fails_closed_on_successful_noop() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ConfirmNoActionTool));
        let agent = AIAgent::new(Arc::new(MockLlmCaller), Arc::new(tools));
        let result = agent
            .confirm_and_execute(
                &pending_confirmation(Some("confirm_no_action")),
                &AgentContext::new(1),
            )
            .await;

        assert!(result.text.contains("未执行"));
        assert!(!result.text.contains("操作已执行完毕"));
        assert!(!result.is_action_taken);
        assert_eq!(result.tool_results["confirm_no_action"]["success"], false);
        assert_eq!(
            result.tool_results["confirm_no_action"]["fail_closed"],
            true
        );
    }

    #[tokio::test]
    async fn test_confirm_and_execute_reports_success_only_after_action() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ConfirmActionTool));
        let agent = AIAgent::new(Arc::new(MockLlmCaller), Arc::new(tools));
        let result = agent
            .confirm_and_execute(
                &pending_confirmation(Some("confirm_action")),
                &AgentContext::new(1),
            )
            .await;

        assert!(result.text.starts_with("操作已执行完毕"));
        assert!(result.is_action_taken);
        assert_eq!(result.tool_results["confirm_action"]["success"], true);
    }

    #[tokio::test]
    async fn test_confirm_and_execute_reports_partial_side_effects() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ConfirmPartialTool));
        let agent = AIAgent::new(Arc::new(MockLlmCaller), Arc::new(tools));
        let result = agent
            .confirm_and_execute(
                &pending_confirmation(Some("confirm_partial")),
                &AgentContext::new(1),
            )
            .await;

        assert!(result.text.contains("部分执行"));
        assert!(result.is_action_taken);
        assert_eq!(result.tool_results["confirm_partial"]["success"], false);
    }

    #[test]
    fn test_default_tool_registry() {
        let registry = default_tool_registry();
        assert!(registry.get("detect_advertising").is_some());
        assert!(registry.get("generate_content").is_some());
    }

    // === 2.2d 确定性路由测试 ===

    #[test]
    fn test_tool_call_trim_leading_whitespace() {
        // __TOOL_CALL__ 前导空白应被容忍（deb8eaf）
        let input = "  \n__TOOL_CALL__[{\"name\":\"system_query\",\"arguments\":{}}]";
        let stripped = input.trim();
        assert!(stripped.starts_with("__TOOL_CALL__"));
        let json_str = &stripped["__TOOL_CALL__".len()..];
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(json_str);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_whitelist_threshold_finance_at_8() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);
        let tool_schemas: Vec<serde_json::Value> = agent.tools.schemas_for_llm();
        // "查财务" 命中一个关键词 = score 8, 白名单工具应匹配
        let result = agent.match_tool_from_text("查财务", &tool_schemas);
        assert_eq!(result, Some("finance_query".to_string()));
    }

    #[test]
    fn test_whitelist_threshold_non_whitelist_needs_16() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);
        let tool_schemas: Vec<serde_json::Value> = agent.tools.schemas_for_llm();
        // "群发消息" 命中 broadcast_message 一个关键词 = score 8, 非白名单需 >= 16 → 不匹配
        let result = agent.match_tool_from_text("群发消息", &tool_schemas);
        assert_eq!(result, None);
    }

    #[test]
    fn test_finance_summary_priority() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);
        // "摘要" 应优先于 "未核销"
        let args = agent.build_fallback_args("finance_query", "财务摘要和未核销");
        assert_eq!(args["action"], "summary");
    }

    #[test]
    fn test_telegram_sdk_fallback_with_group_id() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);
        let args = agent.build_fallback_args("telegram_sdk_query", "查看群 -1002222222222 的权限");
        assert_eq!(args["operation"], "chat_capabilities");
        assert_eq!(args["chat_id"], -1002222222222i64);
    }

    #[test]
    fn test_telegram_sdk_fallback_bound_chats_limit_50() {
        let llm = Arc::new(MockLlmCaller);
        let tools = Arc::new(default_tool_registry());
        let agent = AIAgent::new(llm, tools);
        let args = agent.build_fallback_args("telegram_sdk_query", "查看机器人权限");
        assert_eq!(args["operation"], "bound_chat_capabilities");
        assert_eq!(args["limit"], 50);
    }
}
