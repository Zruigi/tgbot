// AI Agent 类型定义与数据模型
// 对应 Python: services/ai_agent/schema.py

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// AgentRole
// ---------------------------------------------------------------------------

/// Agent 角色类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Assistant,
    Finance,
    Broadcast,
    Content,
    Security,
}

impl AgentRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRole::Assistant => "assistant",
            AgentRole::Finance => "finance",
            AgentRole::Broadcast => "broadcast",
            AgentRole::Content => "content",
            AgentRole::Security => "security",
        }
    }
}

impl Default for AgentRole {
    fn default() -> Self {
        AgentRole::Assistant
    }
}

// ---------------------------------------------------------------------------
// PermissionLevel
// ---------------------------------------------------------------------------

/// 操作权限级别
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    ReadOnly,
    SafeWrite,
    DangerousWrite,
}

impl PermissionLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionLevel::ReadOnly => "read_only",
            PermissionLevel::SafeWrite => "safe_write",
            PermissionLevel::DangerousWrite => "dangerous",
        }
    }
}

// ---------------------------------------------------------------------------
// ToolDefinition
// ---------------------------------------------------------------------------

/// 工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub permission: PermissionLevel,
    pub category: String,
}

impl ToolDefinition {
    /// 转换为 LLM 可用的 function calling schema
    pub fn schema_for_llm(&self) -> serde_json::Value {
        let params = &self.parameters;
        let properties = params.get("properties").cloned().unwrap_or(serde_json::json!({}));
        let required = params.get("required").cloned().unwrap_or(serde_json::json!([]));

        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// ChatMessage
// ---------------------------------------------------------------------------

/// 对话消息（用于 conversation_history）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 保留扩展字段（如 tool_calls 等）
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ChatMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            extra: HashMap::new(),
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            extra: HashMap::new(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(content.into()),
            extra: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// AgentContext
// ---------------------------------------------------------------------------

/// Agent 上下文（每次对话会话）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    pub user_id: i64,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default = "default_sandbox_id")]
    pub sandbox_id: String,
    #[serde(default)]
    pub conversation_history: Vec<ChatMessage>,
    #[serde(default)]
    pub active_role: AgentRole,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_channel() -> String {
    "web_ai_chat".into()
}

fn default_sandbox_id() -> String {
    "default".into()
}

impl AgentContext {
    pub fn new(user_id: i64) -> Self {
        Self {
            user_id,
            channel: default_channel(),
            sandbox_id: default_sandbox_id(),
            conversation_history: Vec::new(),
            active_role: AgentRole::Assistant,
            metadata: HashMap::new(),
        }
    }

    /// 保持历史在合理范围内（默认 40 条）
    pub fn trim_history(&mut self, max: usize) {
        if self.conversation_history.len() > max {
            let start = self.conversation_history.len() - max;
            self.conversation_history = self.conversation_history.split_off(start);
        }
    }
}

// ---------------------------------------------------------------------------
// AgentResponse
// ---------------------------------------------------------------------------

/// Agent 响应结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// 最终回复文本
    pub text: String,
    /// 本次调用的工具列表
    #[serde(default)]
    pub tool_calls_made: Vec<String>,
    /// 工具执行结果
    #[serde(default)]
    pub tool_results: HashMap<String, serde_json::Value>,
    /// 是否执行了实际动作
    #[serde(default)]
    pub is_action_taken: bool,
    /// 动作摘要
    #[serde(default)]
    pub action_summary: String,
    /// 是否需要用户确认
    #[serde(default)]
    pub requires_confirmation: bool,
    /// 置信度 0-1
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// 原始 LLM 响应
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<serde_json::Value>,
}

fn default_confidence() -> f64 {
    1.0
}

impl AgentResponse {
    /// 转换为 JSON 字典
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::json!({}))
    }
}

// ---------------------------------------------------------------------------
// ToolCall (解析 LLM 返回的 function calling 指令)
// ---------------------------------------------------------------------------

/// LLM 返回的单个 tool_call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// arguments 为 JSON 字符串
    #[serde(default)]
    pub arguments: String,
}

impl ToolCallFunction {
    /// 解析 arguments JSON 字符串为 Value
    pub fn parse_arguments(&self) -> serde_json::Value {
        if self.arguments.is_empty() {
            return serde_json::json!({});
        }
        serde_json::from_str(&self.arguments).unwrap_or(serde_json::json!({}))
    }
}

// ---------------------------------------------------------------------------
// LLM Call Result
// ---------------------------------------------------------------------------

/// LLM 调用返回
#[derive(Debug, Clone)]
pub struct LlmCallResult {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

// ---------------------------------------------------------------------------
// Tool Execute Result (统一返回格式)
// ---------------------------------------------------------------------------

/// 工具执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecuteResult {
    #[serde(default = "default_true")]
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub action_taken: bool,
    #[serde(default)]
    pub needs_confirmation: bool,
    /// 标记需要 LLM 继续生成内容
    #[serde(default)]
    pub requires_llm_generation: bool,
    /// 生成的文本内容（由引擎在 requires_llm_generation 后填入）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_content: Option<String>,
}

fn default_true() -> bool {
    true
}

impl ToolExecuteResult {
    pub fn ok(message: &str) -> Self {
        Self {
            success: true,
            data: None,
            message: Some(message.into()),
            action_taken: false,
            needs_confirmation: false,
            requires_llm_generation: false,
            generated_content: None,
        }
    }

    pub fn ok_with_data(data: serde_json::Value, message: &str) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: Some(message.into()),
            action_taken: false,
            needs_confirmation: false,
            requires_llm_generation: false,
            generated_content: None,
        }
    }

    pub fn fail(message: &str) -> Self {
        Self {
            success: false,
            data: None,
            message: Some(message.into()),
            action_taken: false,
            needs_confirmation: false,
            requires_llm_generation: false,
            generated_content: None,
        }
    }

    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::json!({
            "success": false,
            "message": "序列化失败"
        }))
    }
}

// ---------------------------------------------------------------------------
// Single Tool Result in the loop
// ---------------------------------------------------------------------------

/// 循环中单个工具的结果记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleToolResult {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub needs_confirmation: bool,
}

// ---------------------------------------------------------------------------
// Tool trait — 抽象工具执行（定义在此供所有工具实现使用）
// ---------------------------------------------------------------------------

use async_trait::async_trait;

/// 工具执行接口
/// 所有 AI Agent 工具必须实现此 trait
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具名
    fn name(&self) -> &str;

    /// 工具描述
    fn description(&self) -> &str;

    /// 工具分类
    fn category(&self) -> &str {
        "system"
    }

    /// JSON Schema 参数定义
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"properties": {}, "required": []})
    }

    /// 执行工具逻辑
    async fn execute(&self, params: &serde_json::Value, context: &AgentContext) -> ToolExecuteResult;

    /// 转换为 ToolDefinition
    fn definition(&self) -> ToolDefinition {
        let permission = if self.category() == "broadcast" || self.category() == "security" {
            PermissionLevel::DangerousWrite
        } else if self.category() == "finance" || self.category() == "system" {
            PermissionLevel::ReadOnly
        } else {
            PermissionLevel::SafeWrite
        };

        ToolDefinition {
            name: self.name().into(),
            description: self.description().into(),
            parameters: self.parameters_schema(),
            permission,
            category: self.category().into(),
        }
    }

    /// 转换为 LLM function calling schema
    fn schema_for_llm(&self) -> serde_json::Value {
        self.definition().schema_for_llm()
    }
}
