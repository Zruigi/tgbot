// AI Agent 安全约束层 — 防止越权、注入、误操作
// 对应 Python: services/ai_agent/safety.py

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// 危险操作白名单（需二次确认）
// ---------------------------------------------------------------------------

const DANGEROUS_ACTIONS: &[&str] = &[
    "send_broadcast",
    "send_broadcast_with_image",
    "broadcast_message",
    "delete_data",
    "reverse_ledger",
    "reconcile_ledger",
    "skill_manage",
    "ban_user",
];

// ---------------------------------------------------------------------------
// 完全禁止的操作正则
// ---------------------------------------------------------------------------

fn forbidden_patterns() -> Vec<Regex> {
    vec![
        Regex::new(r"(?i)(?:exec|eval|__import__|os\.system|subprocess)").unwrap(),
        Regex::new(r"(?i)(?:DROP\s+TABLE|DELETE\s+FROM|TRUNCATE)").unwrap(),
        Regex::new(r"(?i)(?:rm\s+-rf|chmod|chown)\b").unwrap(),
    ]
}

// ---------------------------------------------------------------------------
// 敏感信息脱敏正则
// ---------------------------------------------------------------------------

struct SensitivePattern {
    regex: Regex,
    replacement: &'static str,
}

fn sensitive_patterns() -> Vec<SensitivePattern> {
    vec![
        SensitivePattern {
            regex: Regex::new(r"\b\d{15,19}\b").unwrap(),
            replacement: "[银行卡号]",
        },
        SensitivePattern {
            regex: Regex::new(r"[Tt][1-9A-HJ-NP-Za-km-z]{33}").unwrap(),
            replacement: "[TRON地址]",
        },
        SensitivePattern {
            regex: Regex::new(r"0x[0-9a-fA-F]{40}").unwrap(),
            replacement: "[ETH地址]",
        },
        SensitivePattern {
            regex: Regex::new(r"\d{4}[-\s]?\d{4}[-\s]?\d{4}\s?\d{3,5}").unwrap(),
            replacement: "[手机号]",
        },
    ]
}

// ---------------------------------------------------------------------------
// 速率限制配置
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub calls_per_minute: u32,
    pub calls_per_hour: u32,
}

fn default_rate_limit() -> RateLimitConfig {
    RateLimitConfig {
        calls_per_minute: 30,
        calls_per_hour: 200,
    }
}

fn dangerous_rate_limit() -> RateLimitConfig {
    RateLimitConfig {
        calls_per_minute: 2,
        calls_per_hour: 10,
    }
}

// ---------------------------------------------------------------------------
// SafetyCheckResult
// ---------------------------------------------------------------------------

/// 安全检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyCheckResult {
    pub passed: bool,
    #[serde(default)]
    pub reason: String,
    /// safe / warning / danger / forbidden
    #[serde(default = "default_risk_level")]
    pub risk_level: String,
    #[serde(default)]
    pub needs_confirmation: bool,
    pub sanitized_params: Option<serde_json::Value>,
    /// 速率限制等待时间（秒）
    #[serde(default)]
    pub wait_seconds: f64,
}

fn default_risk_level() -> String {
    "safe".into()
}

impl SafetyCheckResult {
    pub fn passed(sanitized: serde_json::Value, needs_confirmation: bool, risk_level: &str) -> Self {
        Self {
            passed: true,
            reason: String::new(),
            risk_level: risk_level.into(),
            needs_confirmation,
            sanitized_params: Some(sanitized),
            wait_seconds: 0.0,
        }
    }

    pub fn blocked(reason: &str, risk_level: &str) -> Self {
        Self {
            passed: false,
            reason: reason.into(),
            risk_level: risk_level.into(),
            needs_confirmation: false,
            sanitized_params: None,
            wait_seconds: 0.0,
        }
    }

    pub fn rate_limited(reason: &str, wait_seconds: f64) -> Self {
        Self {
            passed: false,
            reason: reason.into(),
            risk_level: "warning".into(),
            needs_confirmation: false,
            sanitized_params: None,
            wait_seconds,
        }
    }

    pub fn needs_confirmation(tool_name: &str, sanitized: serde_json::Value) -> Self {
        Self {
            passed: true,
            reason: format!("'{}' 属于重要操作，需要您确认后执行", tool_name),
            risk_level: "danger".into(),
            needs_confirmation: true,
            sanitized_params: Some(sanitized),
            wait_seconds: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// SafetyGuardrail
// ---------------------------------------------------------------------------

/// AI Agent 安全护栏
/// 多层防护：模式匹配、权限分级校验、速率限制、输出脱敏、操作审计日志
pub struct SafetyGuardrail {
    /// 调用历史 (timestamp, action_type)
    call_history: Mutex<Vec<(Instant, String)>>,
    /// 编译后的禁止正则
    forbidden_regexes: Vec<Regex>,
    /// 编译后的敏感正则
    sensitive_regexes: Vec<SensitivePattern>,
}

impl SafetyGuardrail {
    pub fn new() -> Self {
        Self {
            call_history: Mutex::new(Vec::new()),
            forbidden_regexes: forbidden_patterns(),
            sensitive_regexes: sensitive_patterns(),
        }
    }

    /// 对工具调用进行安全检查
    pub async fn check_tool_call(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        context_user_id: i64,
    ) -> SafetyCheckResult {
        // 1. 检查输入参数中是否有危险模式
        let params_str = params.to_string();
        for pattern in &self.forbidden_regexes {
            if pattern.is_match(&params_str) {
                warn!("安全拦截: 工具 {} 包含禁止模式", tool_name);
                return SafetyCheckResult::blocked("请求内容包含不允许的指令模式", "forbidden");
            }
        }

        // 2. 判断是否为危险操作（需要确认）
        let is_dangerous = DANGEROUS_ACTIONS.iter().any(|d| {
            tool_name.contains(d) || tool_name.replace('_', "").contains(&d.replace('_', ""))
        });

        if is_dangerous {
            return SafetyCheckResult::needs_confirmation(tool_name, params.clone());
        }

        // 3. 速率限制检查
        let limit_config = if is_dangerous {
            dangerous_rate_limit()
        } else {
            default_rate_limit()
        };
        if let Some(wait) = self.check_rate_limit(tool_name, &limit_config) {
            return SafetyCheckResult::rate_limited(
                &format!("操作过于频繁，请等待 {:.0} 秒后再试", wait),
                wait,
            );
        }

        // 4. 参数基本校验
        match self.sanitize_params(params, tool_name) {
            Ok(sanitized) => SafetyCheckResult::passed(sanitized, false, "safe"),
            Err(e) => SafetyCheckResult::blocked(&format!("参数格式校验失败: {}", e), "warning"),
        }
    }

    /// 对输出文本进行敏感信息脱敏
    pub async fn sanitize_output(&self, text: &str) -> String {
        let mut result = text.to_string();
        for sp in &self.sensitive_regexes {
            result = sp.regex.replace_all(&result, sp.replacement).to_string();
        }
        result
    }

    /// 记录操作审计日志
    pub fn log_action(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        result: &serde_json::Value,
        user_id: i64,
    ) {
        let params_preview = format!("{:.100}", params.to_string());
        let status = result
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        info!(
            "Audit | user={} | tool={} | status={} | summary={}",
            user_id, tool_name, status, params_preview
        );
    }

    // ---- 内部方法 ----

    /// 清理参数中的潜在风险
    fn sanitize_params(
        &self,
        params: &serde_json::Value,
        _tool_name: &str,
    ) -> Result<serde_json::Value, String> {
        Ok(Self::sanitize_value(params))
    }

    /// 递归清理 Value
    fn sanitize_value(val: &serde_json::Value) -> serde_json::Value {
        match val {
            serde_json::Value::String(s) => {
                let mut cleaned = s.clone();
                // 截断过长字符串（防注入长 payload）
                if cleaned.len() > 5000 {
                    cleaned.truncate(5000);
                    cleaned.push_str("... [已截断]");
                }
                // 去除控制字符
                cleaned = cleaned
                    .chars()
                    .filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
                    .collect();
                serde_json::Value::String(cleaned)
            }
            serde_json::Value::Object(map) => {
                let cleaned: serde_json::Map<String, serde_json::Value> = map
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::sanitize_value(v)))
                    .collect();
                serde_json::Value::Object(cleaned)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::sanitize_value).collect())
            }
            other => other.clone(),
        }
    }

    /// 速率限制检查，返回需要等待的秒数（None 表示不限速）
    fn check_rate_limit(&self, action_type: &str, limits: &RateLimitConfig) -> Option<f64> {
        let now = Instant::now();
        let mut history = self.call_history.lock().unwrap();

        // 清理过期记录（1 小时以上）
        let cutoff_1hour = now - std::time::Duration::from_secs(3600);
        history.retain(|(t, _)| *t > cutoff_1hour);

        // 统计 1 分钟内调用
        let cutoff_1min = now - std::time::Duration::from_secs(60);
        let recent_1min: Vec<_> = history.iter().filter(|(t, _)| *t > cutoff_1min).collect();

        if recent_1min.len() >= limits.calls_per_minute as usize {
            if let Some((earliest, _)) = recent_1min.first() {
                let wait = earliest.duration_since(Instant::now()).as_secs_f64()
                    + 60.0
                    - earliest.elapsed().as_secs_f64();
                return Some(wait.max(0.0));
            }
        }

        if history.len() >= limits.calls_per_hour as usize {
            if let Some((earliest, _)) = history.first() {
                let wait = earliest.elapsed().as_secs_f64();
                let remaining = 3600.0 - wait;
                return Some(remaining.max(0.0));
            }
        }

        // 记录本次调用
        history.push((now, action_type.to_string()));
        None
    }
}

impl Default for SafetyGuardrail {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_forbidden_pattern_blocked() {
        let guard = SafetyGuardrail::new();
        let params = serde_json::json!({"cmd": "rm -rf /"});
        let result = guard.check_tool_call("test_tool", &params, 1).await;
        assert!(!result.passed);
        assert_eq!(result.risk_level, "forbidden");
    }

    #[tokio::test]
    async fn test_dangerous_action_needs_confirmation() {
        let guard = SafetyGuardrail::new();
        let params = serde_json::json!({"content": "hello"});
        let result = guard.check_tool_call("broadcast_message", &params, 1).await;
        assert!(result.passed);
        assert!(result.needs_confirmation);
        assert_eq!(result.risk_level, "danger");
    }

    #[tokio::test]
    async fn test_safe_action_passes() {
        let guard = SafetyGuardrail::new();
        let params = serde_json::json!({"action": "summary"});
        let result = guard.check_tool_call("finance_query", &params, 1).await;
        assert!(result.passed);
        assert!(!result.needs_confirmation);
        assert_eq!(result.risk_level, "safe");
    }

    #[tokio::test]
    async fn test_sanitize_output() {
        let guard = SafetyGuardrail::new();
        let text = "卡号 1234567890123456 地址 TN2YqTv5Gz3q7Bm6xXb7fVb7R3Kf5e8q5q";
        let result = guard.sanitize_output(text).await;
        assert!(result.contains("[银行卡号]"));
        assert!(result.contains("[TRON地址]"));
    }

    #[tokio::test]
    async fn test_sanitize_params_truncation() {
        let long_str = "a".repeat(6000);
        let params = serde_json::json!({"content": long_str});
        let sanitized = SafetyGuardrail::sanitize_value(&params);
        let content = sanitized.get("content").unwrap().as_str().unwrap();
        assert!(content.len() < 5100);
        assert!(content.contains("已截断"));
    }
}
