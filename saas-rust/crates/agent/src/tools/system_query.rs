// System Query Tool - 查询系统状态和数据统计
// 对应 Python: services/ai_agent/tools.py 中的 SystemQueryTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{error, info};

/// 系统查询工具
pub struct SystemQueryTool {
    pool: Arc<PgPool>,
}

impl SystemQueryTool {
    /// 创建新的系统查询工具实例
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Tool for SystemQueryTool {
    fn name(&self) -> &str {
        "system_query"
    }

    fn description(&self) -> &str {
        "查询系统状态和数据统计。支持：统计概览、活跃群组、监听地址列表、最近发消息用户、最近消息、广告消息列表、潜在用户等。"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "query_type": {
                    "type": "string",
                    "enum": [
                        "stats_overview",
                        "active_groups",
                        "monitors_list",
                        "recent_senders",
                        "recent_messages",
                        "group_ad_messages",
                        "user_stats",
                        "potential_users"
                    ],
                    "description": "查询类型：stats_overview=统计概览, active_groups=活跃群组, monitors_list=监听列表, recent_senders=最近发消息用户, recent_messages=最近消息, group_ad_messages=群广告消息, user_stats=用户统计, potential_users=潜在用户"
                },
                "limit": {
                    "type": "integer",
                    "description": "返回数量限制（默认10）"
                },
                "group_id": {
                    "type": "integer",
                    "description": "群组ID（可选，用于筛选特定群组）"
                }
            },
            "required": ["query_type"]
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let query_type = params
            .get("query_type")
            .and_then(|v| v.as_str())
            .unwrap_or("stats_overview");
        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(10)
            .clamp(1, 100) as i32;
        let group_id = params.get("group_id").and_then(|v| v.as_i64());

        let tenant_id = context
            .metadata
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|tenant| !tenant.is_empty());

        let Some(tenant_id) = tenant_id else {
            return ToolExecuteResult::fail(
                "system_query requires an explicit tenant scope; refusing an unscoped query",
            );
        };

        info!("系统查询: query_type={}, limit={}", query_type, limit);

        let result = match query_type {
            "stats_overview" => self.get_stats_overview(&self.pool, tenant_id).await,
            "active_groups" => self.get_active_groups(&self.pool, tenant_id, limit).await,
            "monitors_list" => self.get_monitors_list(&self.pool, tenant_id, limit).await,
            "recent_senders" => self.get_recent_senders(&self.pool, tenant_id, limit).await,
            "recent_messages" => self.get_recent_messages(&self.pool, tenant_id, limit).await,
            "group_ad_messages" => {
                self.get_group_ad_messages(&self.pool, tenant_id, group_id, limit)
                    .await
            }
            "potential_users" => self.get_potential_users(&self.pool, tenant_id, limit).await,
            _ => Err(anyhow::anyhow!("未知查询类型: {}", query_type)),
        };

        match result {
            Ok(data) => ToolExecuteResult::ok_with_data(data.0, &data.1),
            Err(e) => {
                error!("系统查询失败: {}", e);
                ToolExecuteResult::fail(&format!("查询失败: {}", e))
            }
        }
    }
}

impl SystemQueryTool {
    /// 获取统计概览
    async fn get_stats_overview(
        &self,
        pool: &PgPool,
        tenant_id: &str,
    ) -> Result<(Value, String), anyhow::Error> {
        // 获取群组数量
        let group_count: i64 = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM groups.tracked_groups WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;

        // 获取用户数量（来自 admin_inbox）
        let user_count: i64 = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM chat.admin_inbox WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;

        // 获取活跃监听地址数量
        let monitor_count: i64 = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::bigint FROM address_monitors WHERE tenant_id = $1 AND is_active = true",
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;

        let data = serde_json::json!({
            "group_count": group_count,
            "user_count": user_count,
            "monitor_count": monitor_count,
            "ai_enabled": true,
        });

        Ok((data, "系统统计概览".to_string()))
    }

    /// 获取活跃群组列表
    async fn get_active_groups(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        limit: i32,
    ) -> Result<(Value, String), anyhow::Error> {
        let rows = sqlx::query(
            "SELECT group_id, group_title, is_monitored, 0::integer AS member_count, updated_at \
             FROM groups.tracked_groups \
             WHERE tenant_id = $1 \
             ORDER BY updated_at DESC \
             LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        let groups: Vec<Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "group_id": row.get::<i64, _>("group_id"),
                    "group_title": row.get::<Option<String>, _>("group_title").unwrap_or_default(),
                    "is_monitored": row.get::<Option<bool>, _>("is_monitored").unwrap_or(false),
                    "member_count": row.get::<Option<i32>, _>("member_count").unwrap_or(0),
                    "updated_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("updated_at"),
                })
            })
            .collect();

        Ok((
            serde_json::json!(groups),
            format!("活跃群组列表 (前{}个)", groups.len()),
        ))
    }

    /// 获取监听地址列表
    async fn get_monitors_list(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        limit: i32,
    ) -> Result<(Value, String), anyhow::Error> {
        // 先检查表结构
        let has_inflow = sqlx::query_scalar::<_, Option<String>>(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'address_monitors' AND column_name = 'inflow_20'",
        )
        .fetch_optional(pool)
        .await?
        .is_some();

        let inflow_expr = if has_inflow {
            "inflow_20"
        } else {
            "0 AS inflow_20"
        };
        let outflow_expr = if has_inflow {
            "outflow_20"
        } else {
            "0 AS outflow_20"
        };
        let profit_expr = if has_inflow {
            "profit_20"
        } else {
            "0 AS profit_20"
        };

        let query = format!(
            "SELECT monitor_id, address, network, remark, {} as inflow, {} as outflow, {} as profit \
             FROM address_monitors \
             WHERE tenant_id = $1 AND is_active = true \
             ORDER BY monitor_id DESC \
             LIMIT $2",
            inflow_expr, outflow_expr, profit_expr
        );

        let rows = sqlx::query(&query)
            .bind(tenant_id)
            .bind(limit)
            .fetch_all(pool)
            .await?;

        let monitors: Vec<Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "monitor_id": row.get::<i64, _>("monitor_id"),
                    "address": row.get::<Option<String>, _>("address").unwrap_or_default(),
                    "network": row.get::<Option<String>, _>("network").unwrap_or_default(),
                    "remark": row.get::<Option<String>, _>("remark").unwrap_or_default(),
                    "inflow": row.get::<Option<f64>, _>("inflow").unwrap_or(0.0),
                    "outflow": row.get::<Option<f64>, _>("outflow").unwrap_or(0.0),
                    "profit": row.get::<Option<f64>, _>("profit").unwrap_or(0.0),
                })
            })
            .collect();

        Ok((
            serde_json::json!(monitors),
            format!("监听地址列表 (前{}个)", monitors.len()),
        ))
    }

    /// 获取最近发消息的用户
    async fn get_recent_senders(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        limit: i32,
    ) -> Result<(Value, String), anyhow::Error> {
        let rows = sqlx::query(
            "SELECT user_id, username, summary, unread_count, last_active \
             FROM chat.admin_inbox \
             WHERE tenant_id = $1 \
             ORDER BY last_active DESC \
             LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        let senders: Vec<Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "user_id": row.get::<i64, _>("user_id"),
                    "username": row.get::<Option<String>, _>("username").unwrap_or_else(|| format!("User {}", row.get::<i64, _>("user_id"))),
                    "summary": row.get::<Option<String>, _>("summary").unwrap_or_default(),
                    "unread_count": row.get::<Option<i32>, _>("unread_count").unwrap_or(0),
                    "last_active": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_active"),
                })
            })
            .collect();

        Ok((
            serde_json::json!(senders),
            format!("最近发消息用户 {} 人", senders.len()),
        ))
    }

    /// 获取最近消息
    async fn get_recent_messages(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        limit: i32,
    ) -> Result<(Value, String), anyhow::Error> {
        let rows = sqlx::query(
            "SELECT id as msg_id, from_userid, to_userid, content, msg_type, timestamp \
             FROM chat.messages \
             WHERE is_deleted = false AND tenant_id = $1 \
             ORDER BY timestamp DESC \
             LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        let messages: Vec<Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "msg_id": row.get::<i64, _>("msg_id"),
                    "from_userid": row.get::<i64, _>("from_userid"),
                    "to_userid": row.get::<i64, _>("to_userid"),
                    "content": row.get::<Option<String>, _>("content").unwrap_or_default(),
                    "msg_type": row.get::<Option<String>, _>("msg_type").unwrap_or_else(|| "text".to_string()),
                    "timestamp": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("timestamp"),
                })
            })
            .collect();

        Ok((
            serde_json::json!(messages),
            format!("最近 {} 条消息", messages.len()),
        ))
    }

    /// 获取群广告消息
    async fn get_group_ad_messages(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        group_id: Option<i64>,
        limit: i32,
    ) -> Result<(Value, String), anyhow::Error> {
        // 先检查表结构是否有 is_advertising 字段
        let has_ad_field: bool = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM information_schema.columns \
             WHERE table_schema = 'groups' AND table_name = 'group_messages' AND column_name = 'is_advertising')",
        )
        .fetch_one(pool)
        .await?;

        if !has_ad_field {
            return Ok((
                serde_json::json!([]),
                "当前数据库尚未启用广告标记字段".to_string(),
            ));
        }

        let mut query = String::from(
            "SELECT tg_msg_id, group_id, user_id, username, content, msg_type, \
             COALESCE(is_advertising, false) as is_advertising, COALESCE(ad_score, 0) as ad_score, timestamp \
             FROM groups.group_messages \
             WHERE tenant_id = $1 AND COALESCE(is_advertising, false) = true",
        );

        if group_id.is_some() {
            query.push_str(" AND group_id = $2");
        }

        query.push_str(if group_id.is_some() {
            " ORDER BY timestamp DESC LIMIT $3"
        } else {
            " ORDER BY timestamp DESC LIMIT $2"
        });

        let mut request = sqlx::query(&query).bind(tenant_id);
        if let Some(group_id) = group_id {
            request = request.bind(group_id);
        }
        let rows = request.bind(limit).fetch_all(pool).await?;

        let ads: Vec<Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "msg_id": row.get::<Option<i64>, _>("tg_msg_id"),
                    "group_id": row.get::<i64, _>("group_id"),
                    "user_id": row.get::<Option<i64>, _>("user_id").unwrap_or(0),
                    "username": row.get::<Option<String>, _>("username").unwrap_or_default(),
                    "content": row.get::<Option<String>, _>("content").unwrap_or_default(),
                    "msg_type": row.get::<Option<String>, _>("msg_type").unwrap_or_else(|| "text".to_string()),
                    "is_advertising": row.get::<bool, _>("is_advertising"),
                    "ad_score": row.get::<i32, _>("ad_score"),
                    "timestamp": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("timestamp"),
                })
            })
            .collect();

        Ok((
            serde_json::json!(ads),
            format!("广告消息列表 (前{}条)", ads.len()),
        ))
    }

    /// 获取潜在用户
    async fn get_potential_users(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        limit: i32,
    ) -> Result<(Value, String), anyhow::Error> {
        // 先检查表是否存在
        let table_exists: bool = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'tenant_saas' AND table_name = 'potential_users')",
        )
        .fetch_one(pool)
        .await?;

        if !table_exists {
            return Ok((serde_json::json!([]), "潜在用户表不存在".to_string()));
        }

        let rows = sqlx::query(
            "SELECT user_id, username, group_id AS source_group, profile_tags, created_at AS first_seen, updated_at AS last_active \
             FROM tenant_saas.potential_users \
             WHERE tenant_id = $1 \
             ORDER BY updated_at DESC \
             LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        let users: Vec<Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "user_id": row.get::<i64, _>("user_id"),
                    "username": row.get::<Option<String>, _>("username").unwrap_or_default(),
                    "source_group": row.get::<Option<i64>, _>("source_group").unwrap_or(0),
                    "profile_tags": row.get::<Option<String>, _>("profile_tags").unwrap_or_default(),
                    "first_seen": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("first_seen"),
                    "last_active": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_active"),
                })
            })
            .collect();

        Ok((
            serde_json::json!(users),
            format!("潜在用户列表 (前{}个)", users.len()),
        ))
    }
}
