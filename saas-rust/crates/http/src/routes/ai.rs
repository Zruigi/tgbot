//! AI API routes — chat, ad detection, pipeline mode, usage
//!
//! Endpoints:
//!   POST /api/ai/chat             — AI chat with n8n/frontend compatibility
//!   POST /api/ai/detect_ad        — Ad detection
//!   GET  /api/ai/pipeline_mode    — Get AI pipeline mode
//!   POST /api/ai/pipeline_mode   — Set AI pipeline mode
//!   GET  /api/openai/models       — List available models
//!   POST /api/openai/chat/completions — OpenAI-compatible chat

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tgbot_agent::contextual_command::resolve_contextual_command;
use tgbot_agent::deletion_guard::{is_agent_cancellation_text, is_agent_confirmation_text};
use tgbot_agent::schema::AgentContext;
use tgbot_agent::session::{get_session_store, AgentSession};
use tgbot_domain::error::{ApiError, ApiResult};
use tgbot_domain::models::Role;
use tgbot_integrations::litellm::{ChatCompletionRequest, ChatMessage};
use tgbot_services::{
    ad_governance::{AdGovernanceCluster, AdGovernanceService, AdScanJob, GroupInfo},
    ad_pipeline,
    ad_text::ad_text_hash,
};

use crate::auth::resolve_scope;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AiChatRequest {
    #[serde(alias = "input", alias = "message")]
    pub content: Option<String>,
    #[serde(alias = "userId")]
    pub user_id: Option<i64>,
    pub model: Option<String>,
    pub feature_key: Option<String>,
    #[serde(alias = "tenantId")]
    pub tenant_id: Option<String>,
    #[serde(alias = "chatId")]
    pub chat_id: Option<i64>,
    pub group_id: Option<i64>,
    pub context: Option<AiChatContext>,
    pub agent_action: Option<AgentActionRequest>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AiChatContext {
    pub tenant_id: Option<String>,
    pub chat_id: Option<i64>,
    pub group_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AgentActionRequest {
    pub id: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
pub struct PipelineModeRequest {
    pub mode: String,
    #[serde(alias = "tenantId")]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PipelineModeQuery {
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillListQuery {
    #[serde(default, deserialize_with = "deserialize_truthy")]
    pub include_builtin: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSkillRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub prompt_template: String,
    pub source: Option<String>,
    pub is_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ToggleSkillRequest {
    pub name: String,
    pub is_enabled: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct AdPipelineQuery {
    pub status: Option<String>,
    pub tenant_id: Option<String>,
    pub group_id: Option<i64>,
    pub limit: Option<i64>,
    pub window_minutes: Option<i64>,
}

// ---------------------------------------------------------------------------
// POST /api/ai/chat — Structured Intent Router
// Calls LLM in JSON mode (temperature 0, 15s timeout) to analyze user intent
// Routes to ad query tool if needed, otherwise returns direct reply
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct IntentAnalysis {
    need_tool: bool,
    tool_name: Option<String>, // fixed to "query_ad_clusters"
    group_id: Option<String>,  // advisory only, overridden by system chat_id
    reply_content: String,     // direct reply or waiting prompt
}

fn normalized_nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn action_payload(action: &AgentActionRequest) -> ApiResult<&serde_json::Map<String, Value>> {
    action
        .payload
        .as_object()
        .ok_or_else(|| ApiError::BadRequest("agent_action.payload must be an object".into()))
}

fn payload_i64(payload: &serde_json::Map<String, Value>, key: &str) -> ApiResult<i64> {
    let value = payload.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|raw| raw.trim().parse().ok()))
    });
    value
        .filter(|value| *value != 0)
        .ok_or_else(|| ApiError::BadRequest(format!("{key} required")))
}

fn payload_string(payload: &serde_json::Map<String, Value>, key: &str) -> ApiResult<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ApiError::BadRequest(format!("{key} required")))
}

fn request_chat_id(body: &AiChatRequest) -> Option<i64> {
    body.chat_id.or(body.group_id).or_else(|| {
        body.context
            .as_ref()
            .and_then(|context| context.chat_id.or(context.group_id))
    })
}

fn explicit_tenant_id(body: &AiChatRequest) -> Option<String> {
    normalized_nonempty(body.tenant_id.as_deref())
        .or_else(|| {
            body.context
                .as_ref()
                .and_then(|context| normalized_nonempty(context.tenant_id.as_deref()))
        })
        .or_else(|| {
            body.agent_action.as_ref().and_then(|action| {
                action
                    .payload
                    .get("tenant_id")
                    .and_then(Value::as_str)
                    .and_then(|value| normalized_nonempty(Some(value)))
            })
        })
}

fn ad_scan_tenant_id(
    scoped_tenant_id: Option<&str>,
    role: &Role,
    body: &AiChatRequest,
) -> ApiResult<String> {
    if let Some(tenant_id) = normalized_nonempty(scoped_tenant_id) {
        return Ok(tenant_id);
    }
    if *role == Role::PlatformAdmin {
        return explicit_tenant_id(body)
            .ok_or_else(|| ApiError::BadRequest("tenant_id required for platform admin".into()));
    }
    Err(ApiError::Forbidden)
}

fn require_ad_scan_write(role: &Role) -> ApiResult<()> {
    if matches!(
        role,
        Role::PlatformAdmin | Role::MerchantOwner | Role::MerchantAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn is_ad_scan_intent(content: &str) -> bool {
    let normalized = content
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "扫描历史广告" | "全量扫描广告" | "全量检查广告" | "检查广告"
    )
}

fn ad_scan_group_actions(groups: &[GroupInfo]) -> Vec<Value> {
    groups
        .iter()
        .take(20)
        .map(|group| {
            json!({
                "id": "select_ad_scan_group",
                "label": if group.title.trim().is_empty() {
                    group.group_id.to_string()
                } else {
                    group.title.clone()
                },
                "icon": "groups",
                "payload": {"group_id": group.group_id},
                "description": format!("{} 条已落库消息", group.message_count),
            })
        })
        .collect()
}

fn tool_result(message: &str, data: Value, action_taken: bool) -> Value {
    json!({
        "success": true,
        "message": message,
        "data": data,
        "action_taken": action_taken,
    })
}

fn successful_agent_response(
    reply: &str,
    tool_name: &str,
    result: Value,
    ui_actions: Vec<Value>,
    job: Option<&AdScanJob>,
    action_taken: bool,
) -> Json<Value> {
    let mut tool_results = serde_json::Map::new();
    tool_results.insert(tool_name.to_string(), result);
    Json(json!({
        "status": "ok",
        "reply": reply,
        "tool_calls_made": [tool_name],
        "tool_results": tool_results,
        "is_action_taken": action_taken,
        "requires_confirmation": false,
        "ui_actions": ui_actions,
        "job": job,
        "data": {
            "reply": reply,
            "executed": true,
            "action_taken": action_taken,
        },
    }))
}

fn fail_closed_tool_response(message: &str, reply: &str) -> Json<Value> {
    Json(json!({
        "status": "error",
        "message": message,
        "reply": reply,
        "tool_calls_made": [],
        "tool_results": {},
        "is_action_taken": false,
        "data": {"executed": false},
    }))
}

fn queued_or_running(status: &str) -> bool {
    matches!(
        status,
        "queued" | "scanning" | "classifying" | "completed" | "partial"
    )
}

fn cluster_lines(clusters: &[AdGovernanceCluster]) -> String {
    if clusters.is_empty() {
        return "该层暂无记录。".into();
    }
    clusters
        .iter()
        .map(|cluster| {
            let text = cluster
                .representative_text
                .as_deref()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect::<String>();
            format!(
                "{}: {}条 / {}人 - {}",
                cluster.action_tier, cluster.recent_count, cluster.sender_count, text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ad_scan_result_text(job: &AdScanJob) -> String {
    let completed_label = if job.status == "completed" {
        "完成"
    } else {
        "部分完成"
    };
    format!(
        "{} 扫描{}。\n总消息：{}\n候选簇：{}\n确认广告簇：{}\n仅建立历史指纹：{} 条\n计划删除：{} 条\n警告发送者：{} 人\n禁言发送者：{} 人\n失败：{}",
        job.group_title.as_deref().unwrap_or("未命名群"),
        completed_label,
        job.total_messages,
        job.candidate_clusters,
        job.confirmed_clusters,
        job.fingerprint_only_count,
        job.planned_delete_count,
        job.warning_sender_count,
        job.muted_sender_count,
        job.failure_count,
    )
}

async fn handle_ad_scan_agent_action(
    state: &AppState,
    tenant_id: &str,
    requested_by: &str,
    role: &Role,
    action: &AgentActionRequest,
) -> ApiResult<Json<Value>> {
    let action_id = action.id.trim();
    if action_id.is_empty() {
        return Err(ApiError::BadRequest("agent_action.id required".into()));
    }
    let payload = action_payload(action)?;
    let service = AdGovernanceService::new(Arc::new(state.db()?.clone()));

    match action_id {
        "select_ad_scan_group" => {
            require_ad_scan_write(role)?;
            let group_id = payload_i64(payload, "group_id")?;
            let groups = service.list_groups(tenant_id).await?;
            let group = groups
                .iter()
                .find(|group| group.group_id == group_id)
                .ok_or_else(|| ApiError::BadRequest("group not found in current tenant".into()))?;
            let job = service
                .create_job(
                    tenant_id,
                    group_id,
                    requested_by,
                    Some(&group.title),
                    true,
                    false,
                    "agent",
                )
                .await?;
            let title = job
                .group_title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| group_id.to_string());
            let reply = format!(
                "已选择 {title}，共 {} 条待扫描记录。\n首次执行为 dry-run：会完成全历史聚类和 LLM 簇级判断，但不会删除、警告或禁言。",
                job.total_messages
            );
            let ui_actions = vec![
                json!({
                    "id": "confirm_ad_scan",
                    "label": "确认 dry-run",
                    "icon": "play_arrow",
                    "style": "primary",
                    "payload": {"job_id": job.job_id},
                }),
                json!({
                    "id": "cancel_ad_scan",
                    "label": "取消",
                    "icon": "close",
                    "style": "secondary",
                    "payload": {"job_id": job.job_id},
                }),
            ];
            let result = tool_result("广告扫描任务已真实创建，等待确认", json!(&job), true);
            let Json(mut response) = successful_agent_response(
                &reply,
                "prepare_ad_scan",
                result,
                ui_actions,
                Some(&job),
                true,
            );
            response["requires_confirmation"] = Value::Bool(true);
            response["action_summary"] = Value::String(format!("准备扫描群 {group_id}"));
            Ok(Json(response))
        }
        "confirm_ad_scan" => {
            require_ad_scan_write(role)?;
            let job_id = payload_string(payload, "job_id")?;
            let configured = service
                .configure_job(tenant_id, &job_id, true, false)
                .await?;
            if !matches!(configured.status.as_str(), "prepared" | "queued") {
                return Err(ApiError::Conflict(format!(
                    "ad scan job cannot be started from status {}",
                    configured.status
                )));
            }
            let job = service.start_job(tenant_id, &job_id).await?;
            if !queued_or_running(&job.status) {
                return Err(ApiError::Conflict(format!(
                    "ad scan job was not queued (status {})",
                    job.status
                )));
            }
            let title = job
                .group_title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| job.group_id.to_string());
            let reply = format!("已启动 {title} 的全历史广告 dry-run。");
            let result = tool_result("广告扫描任务已真实入队", json!(&job), true);
            Ok(successful_agent_response(
                &reply,
                "start_ad_scan",
                result,
                vec![],
                Some(&job),
                true,
            ))
        }
        "cancel_ad_scan" => {
            require_ad_scan_write(role)?;
            let job_id = payload_string(payload, "job_id")?;
            let before = service.get_job(tenant_id, &job_id).await?;
            let job = service.cancel_job(tenant_id, &job_id).await?;
            if job.status != "cancelled" {
                return Err(ApiError::Conflict(format!(
                    "ad scan job was not cancelled (status {})",
                    job.status
                )));
            }
            let action_taken = before.status != "cancelled";
            let reply = if action_taken {
                "已取消广告扫描，没有执行治理动作。"
            } else {
                "广告扫描此前已取消，没有执行治理动作。"
            };
            let result = tool_result(reply, json!(&job), action_taken);
            Ok(successful_agent_response(
                reply,
                "cancel_ad_scan",
                result,
                vec![],
                Some(&job),
                action_taken,
            ))
        }
        "activate_ad_policy" => {
            require_ad_scan_write(role)?;
            let group_id = payload_i64(payload, "group_id")?;
            let groups = service.list_groups(tenant_id).await?;
            let group = groups
                .iter()
                .find(|group| group.group_id == group_id)
                .ok_or_else(|| ApiError::BadRequest("group not found in current tenant".into()))?;
            let prepared = service
                .create_job(
                    tenant_id,
                    group_id,
                    requested_by,
                    Some(&group.title),
                    true,
                    true,
                    "agent_activation",
                )
                .await?;
            let configured = service
                .configure_job(tenant_id, &prepared.job_id, false, true)
                .await?;
            if configured.dry_run || !configured.activate_policy {
                return Err(ApiError::Conflict(
                    "formal governance job was not configured for activation".into(),
                ));
            }
            let job = service.start_job(tenant_id, &configured.job_id).await?;
            if !queued_or_running(&job.status) {
                return Err(ApiError::Conflict(format!(
                    "formal governance job was not queued (status {})",
                    job.status
                )));
            }
            let reply = "已启动正式治理扫描；本次完成后将同时启用该群的持续自动治理。";
            let result = tool_result("正式治理扫描任务已真实入队", json!(&job), true);
            Ok(successful_agent_response(
                reply,
                "activate_ad_policy",
                result,
                vec![],
                Some(&job),
                true,
            ))
        }
        "view_ad_scan_tier" => {
            let job_id = payload_string(payload, "job_id")?;
            let tier = payload_string(payload, "tier")?;
            if !matches!(tier.as_str(), "delete" | "warn" | "pending") {
                return Err(ApiError::BadRequest(
                    "tier must be delete, warn, or pending".into(),
                ));
            }
            let clusters = service
                .get_clusters(tenant_id, &job_id, Some(&tier), 50)
                .await?;
            let reply = cluster_lines(&clusters);
            let result = tool_result(
                "广告扫描分层明细已查询",
                json!({"tier": tier, "clusters": clusters}),
                false,
            );
            Ok(successful_agent_response(
                &reply,
                "view_ad_scan_tier",
                result,
                vec![],
                None,
                false,
            ))
        }
        "get_ad_scan_result" => {
            let job_id = payload_string(payload, "job_id")?;
            let job = service.get_job(tenant_id, &job_id).await?;
            let reply = ad_scan_result_text(&job);
            let mut ui_actions = vec![
                json!({"id": "view_ad_scan_tier", "label": "查看删除明细", "icon": "delete_sweep", "payload": {"job_id": job.job_id, "tier": "delete"}}),
                json!({"id": "view_ad_scan_tier", "label": "查看警告明细", "icon": "warning", "payload": {"job_id": job.job_id, "tier": "warn"}}),
                json!({"id": "view_ad_scan_tier", "label": "查看未确认簇", "icon": "help", "payload": {"job_id": job.job_id, "tier": "pending"}}),
            ];
            if job.dry_run && matches!(job.status.as_str(), "completed" | "partial") {
                ui_actions.insert(
                    0,
                    json!({
                        "id": "activate_ad_policy",
                        "label": "启用自动治理",
                        "icon": "verified_user",
                        "style": "danger",
                        "payload": {"group_id": job.group_id},
                    }),
                );
            }
            let result = tool_result("广告扫描结果已查询", json!(&job), false);
            Ok(successful_agent_response(
                &reply,
                "get_ad_scan_result",
                result,
                ui_actions,
                Some(&job),
                false,
            ))
        }
        _ => Err(ApiError::BadRequest(
            "unsupported Agent ad scan action".into(),
        )),
    }
}

pub async fn ai_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AiChatRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let session_tenant_id = scope
        .tenant_id
        .clone()
        .or_else(|| {
            if scope.role == Role::PlatformAdmin {
                explicit_tenant_id(&body)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "platform".into());
    let tenant_id = session_tenant_id.as_str();

    if let Some(action) = body.agent_action.as_ref() {
        let action_tenant_id = ad_scan_tenant_id(scope.tenant_id.as_deref(), &scope.role, &body)?;
        let requested_by = scope
            .user_id
            .map(|user_id| user_id.to_string())
            .unwrap_or_else(|| scope.role.as_str().to_string());
        return handle_ad_scan_agent_action(
            &state,
            &action_tenant_id,
            &requested_by,
            &scope.role,
            action,
        )
        .await;
    }

    let content = body.content.as_deref().unwrap_or("").trim();
    if content.is_empty() {
        return Ok(Json(json!({
            "status": "ok",
            "reply": "请输入消息内容",
            "tool_calls_made": [],
            "tool_results": {},
            "data": { "reply": "请输入消息内容" },
        })));
    }

    let user_id = body.user_id.unwrap_or(0);
    let real_chat_id = request_chat_id(&body);

    // Input truncation (500 chars, SPEC-2.2b)
    let truncated_input = content.chars().take(500).collect::<String>();

    if is_ad_scan_intent(&truncated_input) {
        let action_tenant_id = ad_scan_tenant_id(scope.tenant_id.as_deref(), &scope.role, &body)?;
        let service = AdGovernanceService::new(Arc::new(state.db()?.clone()));
        let groups = service.list_groups(&action_tenant_id).await?;
        let ui_actions = ad_scan_group_actions(&groups);
        let reply = if groups.is_empty() {
            "当前租户没有可扫描的已绑定群。"
        } else {
            "请选择要扫描的群。扫描只处理该群已落库消息，不会跨群执行。"
        };
        let result = tool_result(
            "可扫描群列表已从当前租户数据库查询",
            json!({"groups": groups}),
            false,
        );
        return Ok(successful_agent_response(
            reply,
            "list_ad_scan_groups",
            result,
            ui_actions,
            None,
            false,
        ));
    }

    // =========================================================================
    // SESSION MANAGEMENT — Behavior 4 & 2 (session metadata + cancellation)
    // =========================================================================

    // 1. Get or create session
    let session_store = get_session_store();
    let merchant_id_str = scope.merchant_id.as_ref().map(|m| m.to_string());
    let session_key = AgentSession::session_key(tenant_id, user_id, merchant_id_str.as_deref());

    let mut session = session_store.get(&session_key).unwrap_or_else(|| {
        let mut ctx = AgentContext::new(user_id);
        // Initialize metadata
        ctx.metadata.insert("tenant_id".into(), json!(tenant_id));
        ctx.metadata.insert(
            "merchant_id".into(),
            scope
                .merchant_id
                .as_ref()
                .map(|m| json!(m.to_string()))
                .unwrap_or(Value::Null),
        );
        ctx.metadata
            .insert("role".into(), json!(format!("{:?}", scope.role)));
        ctx.metadata.insert("chat_id".into(), json!(real_chat_id));
        ctx.metadata.insert("group_id".into(), json!(real_chat_id));
        AgentSession::new(ctx)
    });

    session.update_timestamp();

    // =========================================================================
    // CANCELLATION CHECK — Behavior 2 (must check before confirmation)
    // =========================================================================
    if let Some(ref _pending) = session.pending {
        if is_agent_cancellation_text(&truncated_input) {
            // Clear pending and return cancellation response
            session.clear_pending();
            session_store.update(&session_key, session);

            return Ok(Json(json!({
                "status": "ok",
                "reply": "已取消待确认操作，没有执行任何动作。",
                "tool_calls_made": [],
                "tool_results": {},
                "data": { "reply": "已取消待确认操作，没有执行任何动作。" },
            })));
        }
    }

    // =========================================================================
    // CONFIRMATION CHECK — Behavior 1 (confirm_and_execute reply fix)
    // =========================================================================
    if let Some(pending) = session.pending.take() {
        if is_agent_confirmation_text(&truncated_input) {
            // This route does not own an initialized AIAgent/tool registry yet. Keep the
            // operation pending and fail closed instead of claiming it was executed.
            session.pending = Some(pending);
            session_store.update(&session_key, session);

            return Ok(Json(json!({
                "status": "error",
                "reply": "确认执行链路暂不可用，本次未执行任何操作。待确认操作仍已保留，可输入“取消”撤销。",
                "tool_calls_made": [],
                "tool_results": {},
                "data": {
                    "reply": "确认执行链路暂不可用，本次未执行任何操作。待确认操作仍已保留，可输入“取消”撤销。",
                    "executed": false,
                    "pending_preserved": true
                },
            })));
        } else {
            // Not a confirmation, restore pending and continue
            session.pending = Some(pending);
        }
    }

    // =========================================================================
    // CONTEXTUAL COMMAND RESOLUTION — Behavior 3
    // =========================================================================
    let last_assistant_text = session
        .ctx
        .conversation_history
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .and_then(|m| m.content.clone())
        .unwrap_or_default();

    let routed_input = resolve_contextual_command(
        &truncated_input,
        &mut session.ctx.metadata,
        &last_assistant_text,
    );

    // Save session with any metadata updates
    session_store.update(&session_key, session.clone());

    // =========================================================================
    // STRUCTURED INTENT ANALYSIS — Original flow
    // =========================================================================

    // Structured intent analysis via LLM (JSON mode, temperature 0, 15s timeout)
    // System prompt guides LLM to output JSON with specific structure
    let intent_prompt = format!(
        r#"分析用户意图，返回 JSON 对象：
{{"need_tool": false|true, "tool_name": "query_ad_clusters"|null, "group_id": "群ID"|null, "reply_content": "直接回复或等待提示"}}

用户消息: {}"#,
        routed_input // Use routed input (expanded contextual commands)
    );

    // Structured intent analysis: LLM in JSON mode, temperature 0, 15s timeout
    let intent_request = ChatCompletionRequest {
        model: "gemini-2.5-flash-lite".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: intent_prompt,
        }],
        temperature: Some(0.0), // Deterministic output (temperature 0, spec-2.2b)
        max_tokens: None,
        response_format: None,
        extra: Value::Null,
    };

    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        state.llm.chat_completion(&intent_request),
    )
    .await
    {
        Ok(Ok(resp)) => resp,
        Ok(Err(_)) | Err(_) => {
            // Timeout or LLM error → degrade gracefully
            return Ok(Json(json!({
                "status": "ok",
                "reply": "抱歉，处理过程中出现了问题。请重新尝试。",
                "tool_calls_made": [],
                "tool_results": {},
                "data": { "reply": "抱歉，处理过程中出现了问题。请重新尝试。" },
            })));
        }
    };

    let reply_text = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default();

    // Parse IntentAnalysis JSON (fail-closed on parse error)
    let intent: IntentAnalysis = match serde_json::from_str(&reply_text) {
        Ok(parsed) => parsed,
        Err(_) => {
            // JSON parse failure → friendly degraded reply, no tool execution
            return Ok(Json(json!({
                "status": "ok",
                "reply": "抱歉，我在处理您的请求时遇到了问题。请尝试用更清晰的方式重新表述。",
                "tool_calls_made": [],
                "tool_results": {},
                "data": { "reply": "抱歉，我在处理您的请求时遇到了问题。请尝试用更清晰的方式重新表述。" },
            })));
        }
    };

    // Log usage
    let prompt_tokens = response
        .usage
        .as_ref()
        .and_then(|usage| usage.prompt_tokens)
        .and_then(|value| i64::try_from(value).ok())
        .unwrap_or_else(|| (truncated_input.len() / 4) as i64);
    let completion_tokens = response
        .usage
        .as_ref()
        .and_then(|usage| usage.completion_tokens)
        .and_then(|value| i64::try_from(value).ok())
        .unwrap_or_else(|| (reply_text.len() / 4) as i64);
    let total_tokens = response
        .usage
        .as_ref()
        .and_then(|usage| usage.total_tokens)
        .and_then(|value| i64::try_from(value).ok())
        .unwrap_or(prompt_tokens + completion_tokens);
    let usage_model = response.model.as_deref().unwrap_or(&intent_request.model);
    let _ = sqlx::query(
        "INSERT INTO ai_usage_logs (channel, user_id, tenant_id, feature_key, result_status, endpoint, model, prompt_tokens, completion_tokens, total_tokens, estimated_cost_usd) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
    )
    .bind("web_chat")
    .bind(user_id)
    .bind(tenant_id)
    .bind(body.feature_key.as_deref().unwrap_or("general"))
    .bind("success")
    .bind("ai_chat")
    .bind(usage_model)
    .bind(prompt_tokens)
    .bind(completion_tokens)
    .bind(total_tokens)
    .bind(0.0f64)
    .execute(state.db()?)
    .await;

    // Route based on intent analysis
    if !intent.need_tool {
        // Direct reply, no tool execution
        return Ok(Json(json!({
            "status": "ok",
            "reply": intent.reply_content,
            "tool_calls_made": [],
            "tool_results": {},
            "data": { "reply": intent.reply_content },
        })));
    }

    let Some(effective_tool_name) = intent.tool_name.as_deref() else {
        return Ok(fail_closed_tool_response(
            "AI requested a tool without naming one; no tool was executed",
            "需要调用工具，但未提供有效工具名；本次未执行任何操作。",
        ));
    };

    if effective_tool_name != "query_ad_clusters" {
        return Ok(fail_closed_tool_response(
            &format!("unsupported AI tool: {effective_tool_name}"),
            "请求的工具未注册，本次未执行任何操作。",
        ));
    }

    // Override group_id with real system chat_id (prompt injection defense)
    let effective_group_id =
        real_chat_id.or_else(|| intent.group_id.as_ref().and_then(|s| s.parse::<i64>().ok()));
    let action_tenant_id = ad_scan_tenant_id(scope.tenant_id.as_deref(), &scope.role, &body)?;
    let result = ad_pipeline::query_clusters(
        state.db()?,
        Some(&action_tenant_id),
        effective_group_id,
        "pending",
        20,
        60,
    )
    .await?;
    let tool_message = result
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("广告聚合查询已执行")
        .to_string();
    let reply = if intent.reply_content.trim().is_empty() {
        tool_message.as_str()
    } else {
        intent.reply_content.trim()
    };
    let tool_result = tool_result(&tool_message, result, false);
    Ok(successful_agent_response(
        reply,
        effective_tool_name,
        tool_result,
        vec![],
        None,
        false,
    ))
}

// ---------------------------------------------------------------------------
// POST /api/ai/detect_ad
// ---------------------------------------------------------------------------

pub async fn detect_ad(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;

    let text = body
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if text.is_empty() {
        return Err(ApiError::BadRequest("text is required".into()));
    }
    let context_type = body
        .get("context_type")
        .and_then(Value::as_str)
        .unwrap_or("group_message");
    let result = tgbot_agent::ad_detection::detect_advertising_signals(text, context_type);

    Ok(Json(json!({
        "status": "ok",
        "is_ad": result.is_advertising,
        "is_advertising": result.is_advertising,
        "score": result.ad_score,
        "ad_score": result.ad_score,
        "confidence": result.confidence,
        "risk_level": result.risk_level,
        "findings": result.findings,
        "matched_keywords": result.matched_keywords,
        "url_count": result.url_count,
        "context_type": result.context_type,
        "recommendation": result.recommendation,
    })))
}

// ---------------------------------------------------------------------------
// GET /api/ai/pipeline_mode
// ---------------------------------------------------------------------------

pub async fn get_pipeline_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PipelineModeQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = if scope.role == Role::PlatformAdmin {
        query.tenant_id.as_deref().unwrap_or("platform").trim()
    } else {
        scope.tenant_id.as_deref().ok_or(ApiError::Forbidden)?
    };
    let tenant_id = if tenant_id.is_empty() {
        "platform"
    } else {
        tenant_id
    };
    let setting_key = pipeline_setting_key(tenant_id);

    let mode: Option<String> =
        sqlx::query_scalar::<_, String>("SELECT value FROM platform.admin_settings WHERE key = $1")
            .bind(&setting_key)
            .fetch_optional(state.db()?)
            .await?;
    let mode = mode.unwrap_or_else(|| "agent".to_string());

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "tenantId": tenant_id,
            "mode": mode,
            "modes": ["agent", "legacy"],
        },
    })))
}

// ---------------------------------------------------------------------------
// POST /api/ai/pipeline_mode
// ---------------------------------------------------------------------------

pub async fn set_pipeline_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PipelineModeRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    scope.require_platform()?;

    let valid_modes = ["agent", "legacy"];
    if !valid_modes.contains(&body.mode.as_str()) {
        return Err(ApiError::BadRequest(
            format!("mode must be one of: {:?}", valid_modes).into(),
        ));
    }
    let tenant_id = body.tenant_id.as_deref().unwrap_or("platform").trim();
    let tenant_id = if tenant_id.is_empty() {
        "platform"
    } else {
        tenant_id
    };
    let setting_key = pipeline_setting_key(tenant_id);

    sqlx::query(
        "INSERT INTO platform.admin_settings (key, value) VALUES ($1, $2) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(&setting_key)
    .bind(&body.mode)
    .execute(state.db()?)
    .await?;

    Ok(Json(json!({
        "status": "ok",
        "data": { "tenantId": tenant_id, "mode": body.mode },
    })))
}

// ---------------------------------------------------------------------------
// GET /api/openai/models
// ---------------------------------------------------------------------------

pub async fn list_openai_models(
    State(_state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&_state, &headers).await?;

    Ok(Json(json!({
        "status": "ok",
        "data": [
            { "id": "gemini-2.5-flash-lite", "object": "model", "created": 1700000000, "owned_by": "google" },
            { "id": "gemini-2.5-pro", "object": "model", "created": 1700000000, "owned_by": "google" },
            { "id": "gpt-4o-mini", "object": "model", "created": 1700000000, "owned_by": "openai" },
        ],
    })))
}

// ---------------------------------------------------------------------------
// POST /api/openai/chat/completions
// ---------------------------------------------------------------------------

pub async fn openai_chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("gemini-2.5-flash-lite");
    let messages: Vec<ChatMessage> = serde_json::from_value(
        body.get("messages")
            .cloned()
            .ok_or_else(|| ApiError::BadRequest("messages is required".into()))?,
    )
    .map_err(|error| ApiError::BadRequest(format!("invalid messages: {error}")))?;
    if messages.is_empty() {
        return Err(ApiError::BadRequest("messages must not be empty".into()));
    }
    let mut request = ChatCompletionRequest::simple(model, messages);
    request.temperature = body.get("temperature").and_then(Value::as_f64);
    request.max_tokens = body
        .get("max_tokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let response = state
        .llm
        .chat_completion(&request)
        .await
        .map_err(ApiError::Anyhow)?;
    let choices: Vec<Value> = response
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index.unwrap_or(0),
                "message": {
                    "role": choice.message.role.as_deref().unwrap_or("assistant"),
                    "content": choice.message.content,
                    "tool_calls": choice.message.tool_calls,
                },
                "finish_reason": choice.finish_reason,
            })
        })
        .collect();
    let usage = response.usage.as_ref().map(|usage| {
        json!({
            "prompt_tokens": usage.prompt_tokens.unwrap_or(0),
            "completion_tokens": usage.completion_tokens.unwrap_or(0),
            "total_tokens": usage.total_tokens.unwrap_or(0),
        })
    });
    Ok(Json(json!({
        "id": response.id.unwrap_or_else(|| format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis())),
        "object": response.object.unwrap_or_else(|| "chat.completion".into()),
        "created": response.created.unwrap_or_else(|| chrono::Utc::now().timestamp()),
        "model": response.model.unwrap_or_else(|| model.into()),
        "choices": choices,
        "usage": usage,
    })))
}

// ---------------------------------------------------------------------------
// POST /api/openai/exec
// ---------------------------------------------------------------------------

pub async fn openai_exec(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    let input = body
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if input.is_empty() {
        return Err(ApiError::BadRequest("input is required".into()));
    }
    let model = body.get("model").and_then(Value::as_str);
    let sandbox_id = body
        .get("sandbox_id")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let response = state
        .llm
        .chat(
            vec![ChatMessage {
                role: "user".into(),
                content: input.into(),
            }],
            model,
        )
        .await
        .map_err(ApiError::Anyhow)?;
    let output = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .ok_or_else(|| ApiError::Anyhow(anyhow::anyhow!("LLM returned no content")))?;
    Ok(Json(json!({
        "status": "ok",
        "output": output,
        "output_text": output,
        "result": output,
        "model": response.model.unwrap_or_else(|| model.unwrap_or("gemini-2.5-flash-lite").into()),
        "sandbox_id": sandbox_id,
        "usage": {
            "total_tokens": response.usage.and_then(|usage| usage.total_tokens).unwrap_or(0)
        }
    })))
}

// ---------------------------------------------------------------------------
// AI Skills API — frontend chat/skills compatibility
// ---------------------------------------------------------------------------

pub async fn list_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SkillListQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = skill_tenant_id(&scope);
    let rows = sqlx::query(
        "SELECT id, name, description, prompt_template, source, is_enabled, created_at, updated_at \
         FROM tenant_saas.ai_custom_skills \
         WHERE tenant_id = $1 AND ($2 OR source != 'builtin_fs') \
         ORDER BY updated_at DESC, id DESC LIMIT 100",
    )
    .bind(tenant_id)
    .bind(query.include_builtin)
    .fetch_all(state.db()?)
    .await?;
    let skills: Vec<Value> = rows.iter().map(skill_row_json).collect::<Result<_, _>>()?;
    Ok(Json(
        json!({"status": "ok", "data": skills, "total": skills.len()}),
    ))
}

pub async fn upsert_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertSkillRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_skill_write(&scope)?;
    let tenant_id = skill_tenant_id(&scope);
    let name = validate_skill_text("name", &body.name, 100, false)?;
    let description = validate_skill_text("description", &body.description, 2_000, true)?;
    let prompt_template =
        validate_skill_text("prompt_template", &body.prompt_template, 20_000, true)?;
    let source = match body.source.as_deref().unwrap_or("custom").trim() {
        "custom" => "custom",
        _ => return Err(ApiError::BadRequest("source must be custom".into())),
    };
    let row = sqlx::query(
        "INSERT INTO tenant_saas.ai_custom_skills \
         (tenant_id, name, description, prompt_template, source, is_enabled) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (tenant_id, name) DO UPDATE SET \
           description = EXCLUDED.description, prompt_template = EXCLUDED.prompt_template, \
           source = EXCLUDED.source, is_enabled = EXCLUDED.is_enabled, updated_at = now() \
         RETURNING id, name, description, prompt_template, source, is_enabled, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(name)
    .bind(description)
    .bind(prompt_template)
    .bind(source)
    .bind(body.is_enabled.unwrap_or(true))
    .fetch_one(state.db()?)
    .await?;
    Ok(Json(json!({"status": "ok", "data": skill_row_json(&row)?})))
}

pub async fn toggle_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ToggleSkillRequest>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_skill_write(&scope)?;
    let tenant_id = skill_tenant_id(&scope);
    let name = validate_skill_text("name", &body.name, 100, false)?;
    let row = sqlx::query(
        "UPDATE tenant_saas.ai_custom_skills SET is_enabled = $1, updated_at = now() \
         WHERE tenant_id = $2 AND name = $3 \
         RETURNING id, name, description, prompt_template, source, is_enabled, created_at, updated_at",
    )
    .bind(body.is_enabled)
    .bind(tenant_id)
    .bind(name)
    .fetch_optional(state.db()?)
    .await?
    .ok_or_else(|| ApiError::NotFound("skill not found".into()))?;
    Ok(Json(json!({"status": "ok", "data": skill_row_json(&row)?})))
}

pub async fn import_builtin_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    require_skill_write(&scope)?;
    let tenant_id = skill_tenant_id(&scope);
    let discovered = discover_project_skills()?;
    let mut imported = 0u64;
    for skill in &discovered {
        imported += sqlx::query(
            "INSERT INTO tenant_saas.ai_custom_skills \
             (tenant_id, name, description, prompt_template, source, is_enabled) \
             VALUES ($1, $2, $3, $4, 'builtin_fs', true) \
             ON CONFLICT (tenant_id, name) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(
            skill
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .bind(
            skill
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .bind(format!(
            "引用项目内技能: {}",
            skill
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ))
        .execute(state.db()?)
        .await?
        .rows_affected();
    }
    Ok(Json(json!({
        "status": "ok",
        "data": {"imported": imported, "detected": discovered.len()},
        "message": format!("已导入 {}/{} 个本地 Skills", imported, discovered.len()),
    })))
}

pub async fn list_discovered_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let _scope = resolve_scope(&state, &headers).await?;
    let skills = discover_project_skills()?;
    Ok(Json(
        json!({"status": "ok", "data": skills, "total": skills.len()}),
    ))
}

fn deserialize_truthy<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn pipeline_setting_key(tenant_id: &str) -> String {
    if tenant_id == "platform" {
        "ai_pipeline_mode".to_string()
    } else {
        format!("ai_pipeline_mode:{tenant_id}")
    }
}

fn skill_tenant_id(scope: &tgbot_domain::scope::Scope) -> &str {
    scope.tenant_id.as_deref().unwrap_or("platform")
}

fn require_skill_write(scope: &tgbot_domain::scope::Scope) -> ApiResult<()> {
    if matches!(
        scope.role,
        Role::PlatformAdmin | Role::MerchantOwner | Role::MerchantAdmin
    ) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn validate_skill_text<'a>(
    field: &str,
    value: &'a str,
    max_chars: usize,
    allow_empty: bool,
) -> ApiResult<&'a str> {
    let value = value.trim();
    if !allow_empty && value.is_empty() {
        return Err(ApiError::BadRequest(format!("{field} is required")));
    }
    if value.chars().count() > max_chars {
        return Err(ApiError::BadRequest(format!("{field} is too long")));
    }
    Ok(value)
}

fn skill_row_json(row: &sqlx::postgres::PgRow) -> Result<Value, sqlx::Error> {
    Ok(json!({
        "id": row.try_get::<i64, _>("id")?,
        "name": row.try_get::<String, _>("name")?,
        "description": row.try_get::<String, _>("description")?,
        "prompt_template": row.try_get::<String, _>("prompt_template")?,
        "source": row.try_get::<String, _>("source")?,
        "is_enabled": row.try_get::<bool, _>("is_enabled")?,
        "created_at": row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")?,
        "updated_at": row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")?,
    }))
}

fn discover_project_skills() -> ApiResult<Vec<Value>> {
    let configured_root = std::env::var_os("TGBOT_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|error| ApiError::Anyhow(error.into()))?);
    let project_root = configured_root
        .canonicalize()
        .map_err(|error| ApiError::Anyhow(error.into()))?;
    let configured_dirs = std::env::var("AI_SKILL_LOCAL_DIRS").unwrap_or_else(|_| "skills".into());
    let mut skills = BTreeMap::new();

    for relative in configured_dirs
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let path = Path::new(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|component| component == Component::ParentDir)
        {
            continue;
        }
        let candidate = project_root.join(path);
        let Ok(candidate) = candidate.canonicalize() else {
            continue;
        };
        if !candidate.starts_with(&project_root) || !candidate.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(candidate) else {
            continue;
        };
        for entry in entries.flatten() {
            let skill_directory = entry.path();
            let skill_file = skill_directory.join("SKILL.md");
            if !skill_directory.is_dir() || !skill_file.is_file() {
                continue;
            }
            let Some(name) = skill_directory.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 100 {
                continue;
            }
            let description = read_skill_description(&skill_file);
            skills.entry(name.to_string()).or_insert_with(|| {
                json!({
                    "name": name,
                    "description": description,
                    "source": "builtin_scan",
                })
            });
        }
    }

    Ok(skills.into_values().collect())
}

fn read_skill_description(path: &Path) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return "项目内技能模板".into();
    };
    content
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("description")
                .then(|| value.trim().trim_matches(['\'', '"']).to_string())
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "项目内技能模板".into())
}

// ---------------------------------------------------------------------------
// POST /api/ad-pipeline/events/group-message — n8n AdCluster compatibility
// ---------------------------------------------------------------------------

fn pipeline_tenant_id(
    scope: &tgbot_domain::scope::Scope,
    requested_tenant: Option<&str>,
) -> ApiResult<String> {
    if let Some(tenant_id) = scope
        .tenant_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Ok(tenant_id.to_owned());
    }
    if scope.role == Role::PlatformAdmin {
        return requested_tenant
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| ApiError::BadRequest("tenant_id required for platform admin".into()));
    }
    Err(ApiError::Forbidden)
}

fn value_i64(body: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        body.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|raw| raw.trim().parse().ok()))
        })
    })
}

fn value_string(body: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        body.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

pub async fn ad_pipeline_group_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = pipeline_tenant_id(&scope, body.get("tenant_id").and_then(Value::as_str))?;
    if body
        .get("compute_embedding")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(ApiError::BadRequest(
            "compute_embedding is not available in the PostgreSQL pipeline".into(),
        ));
    }

    let group_id = value_i64(&body, &["group_id", "chat_id"])
        .filter(|value| *value != 0)
        .ok_or_else(|| ApiError::BadRequest("group_id required".into()))?;
    let tg_msg_id = value_i64(&body, &["tg_msg_id", "message_id"])
        .filter(|value| *value > 0)
        .ok_or_else(|| ApiError::BadRequest("tg_msg_id required".into()))?;
    let user_id = value_i64(&body, &["sender_user_id", "sender_id", "user_id"])
        .filter(|value| *value != 0)
        .ok_or_else(|| ApiError::BadRequest("sender_user_id required".into()))?;
    let content = value_string(&body, &["content", "text"]);
    let media_unique_id = value_string(&body, &["media_unique_id", "file_unique_id"]);
    if content.is_none() && media_unique_id.is_none() {
        return Err(ApiError::BadRequest(
            "content or media_unique_id required".into(),
        ));
    }
    let sender_type = value_string(&body, &["sender_type"])
        .unwrap_or_else(|| "user".to_string())
        .to_lowercase();
    if !matches!(sender_type.as_str(), "user" | "channel" | "bot") {
        return Err(ApiError::BadRequest("invalid sender_type".into()));
    }
    let msg_type =
        value_string(&body, &["msg_type", "message_type"]).unwrap_or_else(|| "text".to_string());
    let username = value_string(&body, &["username", "sender_username"]);
    let file_id = value_string(&body, &["file_id"]);
    let message_ts = value_i64(&body, &["message_ts", "timestamp", "date"])
        .filter(|value| *value > 0)
        .map(|value| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
    let text_hash = content.as_deref().map(ad_text_hash);
    let pool = state.db()?;

    let tracked = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM groups.tracked_groups WHERE tenant_id = $1 AND group_id = $2)",
    )
    .bind(&tenant_id)
    .bind(group_id)
    .fetch_one(pool)
    .await?;
    if !tracked {
        return Err(ApiError::NotFound("group_not_found".into()));
    }

    let existing_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM groups.group_messages WHERE tenant_id = $1 AND group_id = $2 AND tg_msg_id = $3 LIMIT 1",
    )
    .bind(&tenant_id)
    .bind(group_id)
    .bind(tg_msg_id)
    .fetch_optional(pool)
    .await?;
    let (id, created) = if let Some(id) = existing_id {
        sqlx::query(
            "UPDATE groups.group_messages
             SET user_id = $1, username = $2, sender_type = $3, content = $4,
                 msg_type = $5, file_id = $6, message_ts = $7, timestamp = COALESCE(to_timestamp($7::double precision), timestamp),
                 media_unique_id = $8, ad_text_hash = $9
             WHERE id = $10 AND tenant_id = $11",
        )
        .bind(user_id)
        .bind(username.as_deref())
        .bind(&sender_type)
        .bind(content.as_deref())
        .bind(&msg_type)
        .bind(file_id.as_deref())
        .bind(message_ts)
        .bind(media_unique_id.as_deref())
        .bind(text_hash.as_deref())
        .bind(id)
        .bind(&tenant_id)
        .execute(pool)
        .await?;
        (id, false)
    } else {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO groups.group_messages
             (group_id, user_id, username, tg_msg_id, sender_type, content, msg_type, file_id,
              timestamp, message_ts, tenant_id, media_unique_id, ad_text_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, COALESCE(to_timestamp($9::double precision), now()), $9, $10, $11, $12)
             RETURNING id",
        )
        .bind(group_id)
        .bind(user_id)
        .bind(username.as_deref())
        .bind(tg_msg_id)
        .bind(&sender_type)
        .bind(content.as_deref())
        .bind(&msg_type)
        .bind(file_id.as_deref())
        .bind(message_ts)
        .bind(&tenant_id)
        .bind(media_unique_id.as_deref())
        .bind(text_hash.as_deref())
        .fetch_one(pool)
        .await?;
        (id, true)
    };

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "id": id,
            "tenant_id": tenant_id,
            "group_id": group_id,
            "tg_msg_id": tg_msg_id,
            "created": created,
            "mode": "postgres.group_messages"
        }
    })))
}

// ---------------------------------------------------------------------------
// POST /api/ad-pipeline/backfill — n8n AdCluster compatibility
// ---------------------------------------------------------------------------

pub async fn ad_pipeline_backfill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = pipeline_tenant_id(&scope, body.get("tenant_id").and_then(Value::as_str))?;
    let days = body
        .get("days")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(1)
        .clamp(1, 30);
    let limit = body
        .get("limit")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(100)
        .clamp(1, 2000);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    let scanned = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM groups.group_messages
         WHERE timestamp >= $1 AND tenant_id = $2 AND NOT COALESCE(is_deleted, false)",
    )
    .bind(cutoff)
    .bind(&tenant_id)
    .fetch_one(state.db()?)
    .await?;
    let mut result = ad_pipeline::query_clusters(
        state.db()?,
        Some(&tenant_id),
        None,
        "all",
        limit.min(100),
        days * 24 * 60,
    )
    .await
    .map_err(ApiError::Anyhow)?;
    if let Some(object) = result.as_object_mut() {
        object.insert("status".into(), json!("ok"));
        object.insert(
            "data".into(),
            json!({
                "scanned": scanned,
                "candidates": object.get("count").cloned().unwrap_or_else(|| json!(0)),
                "clusters": object.get("data").and_then(|data| data.get("clusters")).cloned().unwrap_or_else(|| json!([])),
                "days": days,
                "limit": limit
            }),
        );
    }
    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// GET /api/advertisements — n8n AdCrud compatibility
// ---------------------------------------------------------------------------

pub async fn list_advertisements(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdPipelineQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = pipeline_tenant_id(&scope, query.tenant_id.as_deref())?;
    let result = ad_pipeline::query_clusters(
        state.db()?,
        Some(&tenant_id),
        query.group_id,
        query.status.as_deref().unwrap_or("all"),
        query.limit.unwrap_or(100).clamp(1, 100),
        query
            .window_minutes
            .unwrap_or(24 * 60)
            .clamp(1, 30 * 24 * 60),
    )
    .await
    .map_err(ApiError::Anyhow)?;
    let mut result = result;
    if let Some(object) = result.as_object_mut() {
        object.insert("status".into(), json!("ok"));
    }
    Ok(Json(result))
}

pub async fn get_advertisement(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(ad_id): AxumPath<String>,
    Query(query): Query<AdPipelineQuery>,
) -> ApiResult<Json<Value>> {
    let scope = resolve_scope(&state, &headers).await?;
    let tenant_id = pipeline_tenant_id(&scope, query.tenant_id.as_deref())?;
    let result = ad_pipeline::query_clusters(
        state.db()?,
        Some(&tenant_id),
        query.group_id,
        query.status.as_deref().unwrap_or("all"),
        query.limit.unwrap_or(100).clamp(1, 100),
        query
            .window_minutes
            .unwrap_or(24 * 60)
            .clamp(1, 30 * 24 * 60),
    )
    .await
    .map_err(ApiError::Anyhow)?;
    let cluster = result
        .get("data")
        .and_then(|data| data.get("clusters"))
        .and_then(Value::as_array)
        .and_then(|clusters| {
            clusters.iter().find(|cluster| {
                cluster.get("cluster_id").and_then(Value::as_str) == Some(ad_id.as_str())
            })
        })
        .cloned()
        .ok_or_else(|| ApiError::NotFound("advertisement_not_found".into()))?;
    Ok(Json(json!({"status": "ok", "data": cluster})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_chat_request_deserializes_real_agent_action_and_chat_context() {
        let request: AiChatRequest = serde_json::from_value(json!({
            "userId": 700000001,
            "content": "确认 dry-run",
            "chatId": -1001234567890_i64,
            "agent_action": {
                "id": "confirm_ad_scan",
                "payload": {"job_id": "scan_123"}
            }
        }))
        .expect("request should deserialize");

        assert_eq!(request_chat_id(&request), Some(-1001234567890));
        let action = request
            .agent_action
            .expect("agent_action should be retained");
        assert_eq!(action.id, "confirm_ad_scan");
        assert_eq!(action.payload["job_id"], "scan_123");
    }

    #[test]
    fn platform_ad_action_requires_explicit_tenant_and_merchant_scope_wins() {
        let request: AiChatRequest = serde_json::from_value(json!({
            "content": "执行操作",
            "tenant_id": "body-tenant",
            "agent_action": {"id": "get_ad_scan_result", "payload": {"job_id": "scan_1"}}
        }))
        .expect("request should deserialize");

        assert_eq!(
            ad_scan_tenant_id(None, &Role::PlatformAdmin, &request).unwrap(),
            "body-tenant"
        );
        assert_eq!(
            ad_scan_tenant_id(Some("trusted-tenant"), &Role::MerchantAdmin, &request).unwrap(),
            "trusted-tenant"
        );

        let no_tenant: AiChatRequest = serde_json::from_value(json!({
            "content": "执行操作",
            "agent_action": {"id": "get_ad_scan_result", "payload": {"job_id": "scan_1"}}
        }))
        .expect("request should deserialize");
        assert!(ad_scan_tenant_id(None, &Role::PlatformAdmin, &no_tenant).is_err());
        assert!(require_ad_scan_write(&Role::MerchantAdmin).is_ok());
        assert!(require_ad_scan_write(&Role::MerchantViewer).is_err());
    }

    #[test]
    fn need_tool_without_execution_is_an_explicit_error() {
        let Json(response) =
            fail_closed_tool_response("missing tool", "请求的工具未注册，本次未执行任何操作。");

        assert_eq!(response["status"], "error");
        assert_eq!(response["data"]["executed"], false);
        assert_eq!(response["is_action_taken"], false);
        assert_eq!(response["tool_calls_made"], json!([]));
        assert_eq!(response["tool_results"], json!({}));
        assert!(!response["reply"]
            .as_str()
            .unwrap_or_default()
            .contains("操作已执行完毕"));
    }

    #[test]
    fn successful_agent_response_records_the_actual_dynamic_tool_result() {
        let Json(response) = successful_agent_response(
            "查询完成",
            "query_ad_clusters",
            tool_result("真实查询完成", json!({"count": 2}), false),
            vec![],
            None,
            false,
        );

        assert_eq!(response["status"], "ok");
        assert_eq!(response["tool_calls_made"], json!(["query_ad_clusters"]));
        assert_eq!(
            response["tool_results"]["query_ad_clusters"]["success"],
            true
        );
        assert!(response["tool_results"].get("tool_name").is_none());
    }

    #[test]
    fn ad_scan_intent_is_deterministic_and_does_not_match_unrelated_chat() {
        assert!(is_ad_scan_intent(" 全量扫描广告 "));
        assert!(is_ad_scan_intent("检查广告"));
        assert!(!is_ad_scan_intent("请介绍一下广告治理功能"));
    }
}
