// Telegram 群/频道广告聚类管线
// 对应 Python: services/ad_pipeline.py
//
// Python 生产环境在未配置 pgvector sidecar 时，走 "sqlite_fallback"：对 bot
// 已落库的 group_messages 按归一化 MD5 精确去重聚合，并叠加实时广告评分。
// Rust 侧消息存储在 Postgres（groups.group_messages），因此这里实现的是同一
// 套聚合语义，source/mode 标记为 postgres。
//
// 安全闸（PORTING-13-COMMITS.md #5/#6）：
// - message_refs 永远由服务端按 cluster_id 重查得出，绝不信任客户端提交值；
// - 仅 llm_status=="ad"（含 is_advertising=1 消息推导 confirmed_for_deletion）
//   的 cluster 可删除；
// - 单批删除硬上限 50 条；逐 chat 校验 bot can_delete_messages；
// - 缺 sender 身份时 fail-closed（记错误、不删除）；
// - AD_DELETE_ENABLED / AD_KICK_ENABLED 默认关闭，dry_run 默认开启。

use anyhow::Result;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use tgbot_integrations::telegram::TelegramClient;
use tracing::warn;

use crate::ad_detection::detect_advertising_signals;
use crate::ad_text::ad_text_hash;

// ---------------------------------------------------------------------------
// 配置开关（与 Python config.py 同名同默认值）
// ---------------------------------------------------------------------------

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .trim()
            .to_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// AD_DELETE_ENABLED 总闸，默认 false（关闭 = 一律 dry-run）。
pub fn ad_delete_enabled() -> bool {
    env_flag("AD_DELETE_ENABLED")
}

/// AD_KICK_ENABLED 封禁开关，默认 false。
pub fn ad_kick_enabled() -> bool {
    env_flag("AD_KICK_ENABLED")
}

/// AD_MIN_DUPLICATE_COUNT 聚类最小重复次数，默认 2。
pub fn ad_min_duplicate_count() -> i64 {
    std::env::var("AD_MIN_DUPLICATE_COUNT")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(2)
        .max(1)
}

/// 实际生效的 dry_run：请求 dry_run 或删除总闸未开启时均为模拟执行。
pub fn effective_dry_run(requested_dry_run: bool, delete_enabled: bool) -> bool {
    requested_dry_run || !delete_enabled
}

// ---------------------------------------------------------------------------
// 消息行 + 聚类（纯函数，可单测）
// ---------------------------------------------------------------------------

/// groups.group_messages 中参与聚合的一行消息。
#[derive(Debug, Clone)]
pub struct AdMessageRow {
    pub id: i64,
    pub group_id: i64,
    pub tg_msg_id: Option<i64>,
    pub sender_id: i64,
    pub sender_name: String,
    pub text: String,
    pub msg_ts: Option<i64>,
    pub is_advertising: bool,
    pub ad_score: i32,
}

/// 按归一化 MD5 精确聚合消息，叠加实时广告评分。
/// 对应 Python `AdPipeline._cluster_sqlite_messages`。
pub fn cluster_messages(rows: &[AdMessageRow], limit: i64, min_duplicate_count: i64) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut by_hash: std::collections::HashMap<String, Value> = std::collections::HashMap::new();

    for row in rows {
        let hash = ad_text_hash(&row.text);
        let cluster = by_hash.entry(hash.clone()).or_insert_with(|| {
            order.push(hash.clone());
            json!({
                "cluster_id": format!("pg-md5:{hash}"),
                "representative_hash": hash,
                "representative_text": row.text,
                "occurrences": 0,
                "sender_ids": [],
                "message_ids": [],
                "message_refs": [],
                "chat_ids": [],
                "llm_status": "pending",
                "confirmed_for_deletion": false,
                "action_status": "none",
                "source": "postgres.group_messages",
            })
        });

        cluster["occurrences"] = json!(cluster["occurrences"].as_i64().unwrap_or(0) + 1);
        if row.is_advertising {
            cluster["llm_status"] = json!("ad");
            cluster["confirmed_for_deletion"] = json!(true);
        }
        for (key, value) in [
            ("sender_ids", json!(row.sender_id)),
            ("message_ids", json!(row.tg_msg_id)),
            ("chat_ids", json!(row.group_id)),
        ] {
            let list = cluster[key].as_array_mut().expect("array field");
            if !list.contains(&value) {
                list.push(value);
            }
        }
        cluster["message_refs"]
            .as_array_mut()
            .expect("message_refs array")
            .push(json!({
                "msg_id": row.id,
                "chat_id": row.group_id,
                "group_id": row.group_id,
                "tg_msg_id": row.tg_msg_id,
                "sender_id": row.sender_id,
                "sender_name": row.sender_name,
                "text": row.text,
                "msg_ts": row.msg_ts,
            }));
    }

    let min_dup = min_duplicate_count.max(1);
    let mut clusters: Vec<Value> = Vec::new();
    for hash in order {
        let mut cluster = by_hash.remove(&hash).expect("cluster present");
        let representative = cluster["representative_text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let flags = detect_advertising_signals(&representative, "group_message");
        let score = flags.ad_score as i64;
        let is_live_ad = flags.is_advertising;
        cluster["live_detection"] = serde_json::to_value(&flags).unwrap_or(Value::Null);
        let occurrences = cluster["occurrences"].as_i64().unwrap_or(0);
        if occurrences >= min_dup || is_live_ad || score >= 40 {
            let dup_score = if occurrences >= min_dup { 55 } else { 0 };
            cluster["risk_score"] = json!(score.max(dup_score));
            clusters.push(cluster);
        }
    }

    clusters.sort_by_key(|cluster| {
        std::cmp::Reverse((
            cluster["risk_score"].as_i64().unwrap_or(0),
            cluster["occurrences"].as_i64().unwrap_or(0),
        ))
    });
    clusters.truncate(limit.clamp(1, 100) as usize);
    clusters
}

/// 仅已确认为广告的 cluster 才允许删除（安全闸，对应 Python `cluster_is_confirmed_ad`）。
pub fn cluster_is_confirmed_ad(cluster: Option<&Value>) -> bool {
    let Some(cluster) = cluster else {
        return false;
    };
    cluster
        .get("confirmed_for_deletion")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || cluster.get("llm_status").and_then(Value::as_str) == Some("ad")
}

// ---------------------------------------------------------------------------
// 查询入口
// ---------------------------------------------------------------------------

/// 拉取窗口内已落库的群/频道消息（对应 Python `_fetch_sqlite_messages`）。
pub async fn fetch_recent_group_messages(
    pool: &PgPool,
    tenant_id: Option<&str>,
    group_id: Option<i64>,
    window_minutes: i64,
    limit: i64,
) -> Result<Vec<AdMessageRow>> {
    let cutoff = chrono::Utc::now().timestamp() - window_minutes.max(1) * 60;
    let limit = limit.clamp(1, 2000);

    let rows = sqlx::query(
        r#"
        SELECT id, group_id, tg_msg_id, user_id, COALESCE(username, '') AS username,
               COALESCE(content, '') AS content,
               COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) AS msg_ts,
               is_advertising, ad_score
        FROM groups.group_messages
        WHERE COALESCE(content, '') != ''
          AND COALESCE(sender_type, 'user') IN ('user', 'channel', 'bot')
          AND COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) >= $1
          AND NOT COALESCE(is_deleted, false)
          AND ($2::bigint IS NULL OR group_id = $2)
          AND ($3::text IS NULL OR tenant_id = $3)
        ORDER BY COALESCE(message_ts::bigint, EXTRACT(EPOCH FROM timestamp)::bigint) DESC, id DESC
        LIMIT $4
        "#,
    )
    .bind(cutoff)
    .bind(group_id)
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| AdMessageRow {
            id: row.get::<i64, _>("id"),
            group_id: row.get::<i64, _>("group_id"),
            tg_msg_id: row.get::<Option<i64>, _>("tg_msg_id"),
            sender_id: row.get::<i64, _>("user_id"),
            sender_name: row.get::<String, _>("username"),
            text: row.get::<String, _>("content"),
            msg_ts: row.get::<Option<i64>, _>("msg_ts"),
            is_advertising: row.get::<bool, _>("is_advertising"),
            ad_score: row.get::<i32, _>("ad_score"),
        })
        .collect())
}

/// 查询广告聚合候选（对应 Python `AdPipeline.query_clusters` 的 fallback 分支）。
///
/// 注意：与 Python fallback 一致，`_status` 过滤仅在 pgvector sidecar 模式下生效，
/// 这里接受并忽略该参数；cluster 的 llm_status 由 is_advertising 消息推导。
pub async fn query_clusters(
    pool: &PgPool,
    tenant_id: Option<&str>,
    group_id: Option<i64>,
    _status: &str,
    limit: i64,
    window_minutes: i64,
) -> Result<Value> {
    let fetch_limit = std::cmp::max(200, limit.max(1) * 20);
    let rows =
        fetch_recent_group_messages(pool, tenant_id, group_id, window_minutes, fetch_limit).await?;
    let clusters = cluster_messages(&rows, limit, ad_min_duplicate_count());

    Ok(json!({
        "success": true,
        "source": "postgres.group_messages",
        "mode": "postgres_fallback",
        "data_complete": !rows.is_empty(),
        "analysis_allowed": !rows.is_empty(),
        "evidence_count": rows.len(),
        "data": {"clusters": clusters, "raw_message_count": rows.len()},
        "count": clusters.len(),
        "message": format!("广告聚合查询完成：{} 个候选 cluster，来源 Postgres 已落库消息", clusters.len()),
        "limitation": if rows.is_empty() {
            json!("当前没有 bot 已收到并落库的群/频道消息，不能判断没有广告。")
        } else {
            Value::Null
        },
    }))
}

// ---------------------------------------------------------------------------
// 删除执行器
// ---------------------------------------------------------------------------

fn ref_i64(reference: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        reference.get(*key).and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        })
    })
}

/// 按 cluster 的 message_refs 批量删除 Telegram 消息（对应 Python `delete_cluster_messages`）。
///
/// refs 必须来自服务端 `query_clusters` 的重查结果（永不信任客户端提交值）。
pub async fn delete_cluster_messages(
    pool: &PgPool,
    telegram: &TelegramClient,
    refs: &[Value],
    dry_run: bool,
    ban_senders: bool,
) -> Value {
    let total_target_count = refs.len();
    // 安全闸：单批硬上限 50 条
    let refs = &refs[..refs.len().min(50)];
    let delete_enabled = ad_delete_enabled();
    let kick_enabled = ad_kick_enabled();
    let dry = effective_dry_run(dry_run, delete_enabled);

    let bot_id = match telegram.get_me().await {
        Ok(me) => me.id,
        Err(error) => {
            return json!({
                "success": false,
                "message": format!("无法校验 bot 身份: {}", truncate(&error.to_string(), 160)),
                "action_taken": false,
            });
        }
    };

    let mut errors: Vec<Value> = Vec::new();
    let mut deleted: Vec<Value> = Vec::new();
    let mut skipped_admins: Vec<Value> = Vec::new();
    let mut banned: Vec<i64> = Vec::new();
    let mut permission_cache: std::collections::HashMap<i64, bool> =
        std::collections::HashMap::new();

    for reference in refs {
        let chat_id = ref_i64(reference, &["chat_id", "group_id"]);
        let tg_msg_id = ref_i64(reference, &["tg_msg_id"]);
        let (Some(chat_id), Some(tg_msg_id)) = (chat_id, tg_msg_id) else {
            errors.push(json!({"ref": reference, "error": "缺少 chat_id/tg_msg_id"}));
            continue;
        };
        let sender_id = ref_i64(reference, &["sender_id"]).unwrap_or(0);

        // 安全闸：逐 chat 校验 bot 删除权限
        let allowed = match permission_cache.get(&chat_id) {
            Some(allowed) => *allowed,
            None => {
                let allowed = match telegram.get_chat_member(chat_id, bot_id).await {
                    Ok(member) => {
                        let status = member.get("status").and_then(Value::as_str).unwrap_or("");
                        status == "creator"
                            || (status == "administrator"
                                && member
                                    .get("can_delete_messages")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false))
                    }
                    Err(_) => false,
                };
                permission_cache.insert(chat_id, allowed);
                allowed
            }
        };
        if !allowed {
            errors.push(json!({"ref": reference, "error": "bot_missing_delete_permission"}));
            continue;
        }

        // 安全闸：缺 sender 身份 fail-closed
        if sender_id == 0 {
            errors.push(json!({"ref": reference, "error": "missing_sender_id"}));
            continue;
        }
        match telegram.get_chat_member(chat_id, sender_id).await {
            Ok(member) => {
                let status = member.get("status").and_then(Value::as_str).unwrap_or("");
                if status == "administrator" || status == "creator" {
                    skipped_admins.push(reference.clone());
                    continue;
                }
            }
            Err(error) => {
                errors.push(json!({
                    "ref": reference,
                    "error": format!("sender_check_failed:{}", truncate(&error.to_string(), 160)),
                }));
                continue;
            }
        }

        if !dry {
            if let Err(error) = telegram.delete_message(chat_id, tg_msg_id).await {
                errors.push(json!({"ref": reference, "error": truncate(&error.to_string(), 300)}));
                continue;
            }
            if let Err(error) = sqlx::query(
                "DELETE FROM groups.group_messages WHERE group_id = $1 AND tg_msg_id = $2",
            )
            .bind(chat_id)
            .bind(tg_msg_id)
            .execute(pool)
            .await
            {
                // Telegram 已删除，仅记录落库清理失败
                warn!("delete_cluster_messages: DB cleanup failed: {error}");
                errors.push(json!({"ref": reference, "error": truncate(&error.to_string(), 300)}));
                continue;
            }
        }
        deleted.push(reference.clone());

        if ban_senders && kick_enabled && !dry {
            if let Err(error) = telegram.ban_chat_member(chat_id, sender_id).await {
                errors.push(json!({
                    "ref": reference,
                    "ban_error": truncate(&error.to_string(), 300),
                }));
            } else if !banned.contains(&sender_id) {
                banned.push(sender_id);
            }
        }
    }

    json!({
        "success": errors.is_empty(),
        "action_taken": !deleted.is_empty() && !dry,
        "dry_run": dry,
        "data": {
            "total_target_count": total_target_count,
            "batch_target_count": refs.len(),
            "remaining_count": total_target_count.saturating_sub(refs.len()),
            "deleted_count": deleted.len(),
            "skipped_admin_count": skipped_admins.len(),
            "banned_sender_ids": banned,
            "errors": errors,
            "delete_enabled": delete_enabled,
            "kick_enabled": kick_enabled,
        },
        "message": format!(
            "{}处理 cluster：本批目标 {}/{} 条，处理 {} 条，跳过管理员 {} 条，错误 {} 条",
            if dry { "模拟" } else { "实际" },
            refs.len(),
            total_target_count,
            deleted.len(),
            skipped_admins.len(),
            errors.len()
        ),
    })
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, text: &str, sender: i64, is_ad: bool) -> AdMessageRow {
        AdMessageRow {
            id,
            group_id: -100200300,
            tg_msg_id: Some(1000 + id),
            sender_id: sender,
            sender_name: format!("user{sender}"),
            text: text.to_string(),
            msg_ts: Some(1_700_000_000 + id),
            is_advertising: is_ad,
            ad_score: 0,
        }
    }

    #[test]
    fn clusters_duplicate_messages_by_normalized_hash() {
        let rows = vec![
            row(1, "低价出料 联系 @seller_a", 11, false),
            row(2, "低价出料 联系 @seller_b", 12, false),
            row(3, "今天天气不错", 13, false),
        ];
        let clusters = cluster_messages(&rows, 20, 2);
        // 重复广告聚成一簇；普通闲聊单条且无广告特征被过滤
        assert_eq!(clusters.len(), 1);
        let cluster = &clusters[0];
        assert_eq!(cluster["occurrences"], 2);
        assert_eq!(cluster["message_refs"].as_array().unwrap().len(), 2);
        assert_eq!(cluster["sender_ids"].as_array().unwrap().len(), 2);
        assert!(cluster["cluster_id"]
            .as_str()
            .unwrap()
            .starts_with("pg-md5:"));
        assert_eq!(cluster["llm_status"], "pending");
        assert_eq!(cluster["confirmed_for_deletion"], false);
    }

    #[test]
    fn single_high_score_ad_kept_without_duplicates() {
        let rows = vec![row(1, "加我微信免费领取红包返利佣金", 5, false)];
        let clusters = cluster_messages(&rows, 20, 2);
        assert_eq!(clusters.len(), 1);
        assert!(clusters[0]["risk_score"].as_i64().unwrap() >= 40);
    }

    #[test]
    fn confirmed_ad_message_marks_cluster_confirmed() {
        let rows = vec![
            row(1, "普通重复内容", 5, true),
            row(2, "普通重复内容", 6, false),
        ];
        let clusters = cluster_messages(&rows, 20, 2);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0]["llm_status"], "ad");
        assert!(cluster_is_confirmed_ad(Some(&clusters[0])));
    }

    #[test]
    fn pending_cluster_is_not_confirmed() {
        let rows = vec![
            row(1, "重复但未确认", 5, false),
            row(2, "重复但未确认", 6, false),
        ];
        let clusters = cluster_messages(&rows, 20, 2);
        assert_eq!(clusters.len(), 1);
        assert!(!cluster_is_confirmed_ad(Some(&clusters[0])));
        assert!(!cluster_is_confirmed_ad(None));
    }

    #[test]
    fn cluster_limit_is_clamped() {
        let rows: Vec<AdMessageRow> = (0..10)
            .flat_map(|i| {
                let text = format!("重复内容第{i}组");
                vec![
                    row(i * 2, &text, 100 + i, false),
                    row(i * 2 + 1, &text, 200 + i, false),
                ]
            })
            .collect();
        let clusters = cluster_messages(&rows, 3, 2);
        assert_eq!(clusters.len(), 3);
    }

    #[test]
    fn effective_dry_run_requires_enabled_flag() {
        // 总闸关闭时，即使请求实删也必须落到 dry-run
        assert!(effective_dry_run(true, true));
        assert!(effective_dry_run(true, false));
        assert!(effective_dry_run(false, false));
        assert!(!effective_dry_run(false, true));
    }

    #[test]
    fn ref_i64_accepts_aliases_and_strings() {
        let reference = json!({"group_id": 42, "tg_msg_id": "77"});
        assert_eq!(ref_i64(&reference, &["chat_id", "group_id"]), Some(42));
        assert_eq!(ref_i64(&reference, &["tg_msg_id"]), Some(77));
        assert_eq!(ref_i64(&reference, &["sender_id"]), None);
    }
}
