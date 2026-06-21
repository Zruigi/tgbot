//! AI worker – processes ai_process jobs from the queue.
//!
//! Handles various AI-related tasks:
//! - Content analysis (keyword detection, spam detection)
//! - Auto-translation
//! - Message summarization
//! - Ad detection
//!
//! Uses the LiteLLM integration to call LLMs.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::job_queue::ClaimedJob;
use crate::scheduler::Worker;

use tgbot_integrations::litellm::{ChatMessage, LiteLLMClient};
use tgbot_repos::ai_usage_repo;

// ---------------------------------------------------------------------------
// AI worker
// ---------------------------------------------------------------------------

pub struct AiWorker {
    litellm: Arc<LiteLLMClient>,
}

impl AiWorker {
    pub fn new(litellm: Arc<LiteLLMClient>) -> Self {
        Self { litellm }
    }
}

/// Analysis types supported by the AI worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisType {
    KeywordMonitoring,
    SpamDetection,
    Translation,
    Summarization,
    AdDetection,
    Custom(String),
}

impl AnalysisType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "keyword_monitoring" => AnalysisType::KeywordMonitoring,
            "spam_detection" => AnalysisType::SpamDetection,
            "translation" => AnalysisType::Translation,
            "summarization" => AnalysisType::Summarization,
            "ad_detection" => AnalysisType::AdDetection,
            other => AnalysisType::Custom(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            AnalysisType::KeywordMonitoring => "keyword_monitoring",
            AnalysisType::SpamDetection => "spam_detection",
            AnalysisType::Translation => "translation",
            AnalysisType::Summarization => "summarization",
            AnalysisType::AdDetection => "ad_detection",
            AnalysisType::Custom(s) => s,
        }
    }
}

#[async_trait]
impl Worker for AiWorker {
    fn job_type(&self) -> &str {
        "ai_process"
    }

    #[instrument(skip(self, pool, claimed), fields(job_id = claimed.job.id))]
    async fn process(&self, pool: &PgPool, claimed: &ClaimedJob) -> Result<()> {
        let payload = &claimed.job.payload;

        let analysis_type_str = payload
            .get("analysis_type")
            .and_then(|v| v.as_str())
            .unwrap_or("custom");
        let analysis_type = AnalysisType::from_str(analysis_type_str);
        let tenant_id = payload
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        info!(analysis_type = analysis_type.as_str(), tenant_id, "processing AI job");

        let result = match analysis_type {
            AnalysisType::KeywordMonitoring => {
                self.process_keyword_monitoring(pool, payload).await
            }
            AnalysisType::SpamDetection => {
                self.process_spam_detection(pool, payload).await
            }
            AnalysisType::Translation => {
                self.process_translation(pool, payload).await
            }
            AnalysisType::Summarization => {
                self.process_summarization(pool, payload).await
            }
            AnalysisType::AdDetection => {
                self.process_ad_detection(pool, payload).await
            }
            AnalysisType::Custom(ref t) => {
                warn!(custom_type = t, "unknown AI analysis type, skipping");
                Ok(json!({"status": "skipped", "reason": "unknown_analysis_type"}))
            }
        };

        match result {
            Ok(output) => {
                info!(analysis_type = analysis_type.as_str(), "AI processing completed");
                // Record AI usage (non-blocking)
                if let Err(e) = self.record_usage(pool, tenant_id, analysis_type.as_str(), &output).await {
                    warn!(error = %e, "failed to record AI usage");
                }
                Ok(())
            }
            Err(e) => {
                warn!(analysis_type = analysis_type.as_str(), error = %e, "AI processing failed");
                Err(e)
            }
        }
    }
}

impl AiWorker {
    /// Process keyword monitoring: check recent messages in a group against keywords.
    async fn process_keyword_monitoring(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let group_id = payload
            .get("group_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("missing group_id"))?;
        let keywords = payload
            .get("keywords")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if keywords.is_empty() {
            return Ok(json!({"status": "skipped", "reason": "no_keywords"}));
        }

        // Fetch recent messages from the group (last 50)
        let messages = sqlx::query(
            r#"
            SELECT message_id, text, from_user_id, from_username, created_at
            FROM messages
            WHERE group_id = $1
              AND created_at > now() - interval '1 hour'
            ORDER BY created_at DESC
            LIMIT 50
            "#,
        )
        .bind(group_id)
        .fetch_all(pool)
        .await?;

        let msg_texts: Vec<String> = messages
            .iter()
            .filter_map(|row| row.try_get::<String, _>("text").ok())
            .filter(|t| !t.is_empty())
            .collect();

        if msg_texts.is_empty() {
            return Ok(json!({"status": "no_recent_messages", "group_id": group_id}));
        }

        // Use LLM to analyze messages against keywords
        let messages_text = msg_texts.join("\n---\n");
        let system_prompt = format!(
            "You are a content moderation assistant. Analyze the following messages from a Telegram group \
             and identify any that match or relate to these keywords: [{}]. \
             For each match, explain why it matched. Respond in JSON format with a 'matches' array.",
            keywords
        );

        let response = self
            .litellm
            .simple_chat(&system_prompt, &messages_text)
            .await?;

        Ok(json!({
            "group_id": group_id,
            "messages_analyzed": msg_texts.len(),
            "llm_response": response,
        }))
    }

    /// Process spam detection: analyze messages for spam patterns.
    async fn process_spam_detection(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let group_id = payload.get("group_id").and_then(|v| v.as_i64());
        let message_text = payload
            .get("message_text")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if message_text.is_empty() {
            return Ok(json!({"status": "skipped", "reason": "empty_message"}));
        }

        let system_prompt = "You are a spam detection assistant. Analyze the given message and determine \
                              if it is spam. Respond in JSON format with fields: is_spam (boolean), \
                              confidence (0.0-1.0), reason (string).";

        let response = self
            .litellm
            .simple_chat(system_prompt, message_text)
            .await?;

        Ok(json!({
            "group_id": group_id,
            "llm_response": response,
        }))
    }

    /// Process translation: translate message content.
    async fn process_translation(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let target_lang = payload
            .get("target_lang")
            .and_then(|v| v.as_str())
            .unwrap_or("en");

        if text.is_empty() {
            return Ok(json!({"status": "skipped", "reason": "empty_text"}));
        }

        let system_prompt = format!(
            "You are a translation assistant. Translate the following text to {}. \
             Respond with only the translated text.",
            target_lang
        );

        let translated = self.litellm.simple_chat(&system_prompt, text).await?;

        Ok(json!({
            "original": text,
            "translated": translated,
            "target_lang": target_lang,
        }))
    }

    /// Process summarization: summarize a batch of messages.
    async fn process_summarization(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let group_id = payload
            .get("group_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("missing group_id"))?;

        // Fetch recent messages
        let messages = sqlx::query(
            r#"
            SELECT text, from_username, created_at
            FROM messages
            WHERE group_id = $1
              AND created_at > now() - interval '24 hours'
            ORDER BY created_at ASC
            LIMIT 200
            "#,
        )
        .bind(group_id)
        .fetch_all(pool)
        .await?;

        let msg_texts: Vec<String> = messages
            .iter()
            .filter_map(|row| {
                let text = row.try_get::<String, _>("text").ok()?;
                let user = row.try_get::<Option<String>, _>("from_username").ok()??;
                Some(format!("[{}]: {}", user, text))
            })
            .collect();

        if msg_texts.is_empty() {
            return Ok(json!({"status": "no_messages", "group_id": group_id}));
        }

        let messages_text = msg_texts.join("\n");
        let system_prompt = "You are a summarization assistant. Summarize the following group chat messages \
                              into a concise summary highlighting key topics and decisions. \
                              Respond in a structured format.";

        let summary = self.litellm.simple_chat(system_prompt, &messages_text).await?;

        Ok(json!({
            "group_id": group_id,
            "messages_count": msg_texts.len(),
            "summary": summary,
        }))
    }

    /// Process ad detection: detect advertisements in messages.
    async fn process_ad_detection(&self, pool: &PgPool, payload: &Value) -> Result<Value> {
        let message_text = payload
            .get("message_text")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if message_text.is_empty() {
            return Ok(json!({"status": "skipped", "reason": "empty_message"}));
        }

        let system_prompt = "You are an advertisement detection assistant. Analyze the given message and \
                              determine if it contains promotional or advertising content. \
                              Respond in JSON format with fields: is_ad (boolean), confidence (0.0-1.0), \
                              ad_type (string: commercial/self-promo/affiliate/spam/none).";

        let response = self.litellm.simple_chat(system_prompt, message_text).await?;

        Ok(json!({
            "llm_response": response,
        }))
    }

    /// Record AI usage for billing/quota tracking.
    async fn record_usage(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        analysis_type: &str,
        result: &Value,
    ) -> Result<()> {
        // Best-effort recording; the ai_usage_repo may be a stub.
        sqlx::query(
            r#"
            INSERT INTO ai_usage_logs (tenant_id, analysis_type, result_summary, created_at)
            VALUES ($1, $2, $3, now())
            "#,
        )
        .bind(tenant_id)
        .bind(analysis_type)
        .bind(serde_json::to_string(result)?)
        .execute(pool)
        .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analysis_type_roundtrip() {
        let types = vec![
            AnalysisType::KeywordMonitoring,
            AnalysisType::SpamDetection,
            AnalysisType::Translation,
            AnalysisType::Summarization,
            AnalysisType::AdDetection,
            AnalysisType::Custom("my_analysis".into()),
        ];
        for at in types {
            assert_eq!(AnalysisType::from_str(at.as_str()), at);
        }
    }
}
