use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::BTreeSet;

use crate::ad_detection::detect_advertising_signals;

const LEAD_PROFILE_RULES: &[(&[&str], &str)] = &[
    (&["加入", "申请", "想进", "邀请", "拉我"], "拉新意向"),
    (&["合作", "合伙", "商务"], "合作意向"),
    (&["推荐", "分享"], "推荐"),
    (&["招聘", "找人", "兼职"], "招聘"),
    (&["项目", "开发", "进度"], "项目相关"),
    (
        &["融资", "资金", "天使轮", "投资", "理财", "赚钱"],
        "投融资",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupLeadMatch {
    pub matched_keywords: Vec<String>,
    pub profile_tags: Vec<String>,
}

pub struct GroupMessagePolicyInput<'a> {
    pub tenant_id: &'a str,
    pub group_id: i64,
    pub group_title: &'a str,
    pub telegram_message_id: i64,
    pub user_id: i64,
    pub username: Option<&'a str>,
    pub first_name: Option<&'a str>,
    pub last_name: Option<&'a str>,
    pub content: &'a str,
    pub custom_keywords: &'a [String],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GroupMessagePolicySummary {
    pub message_updated: bool,
    pub is_advertising: bool,
    pub lead_captured: bool,
}

pub fn analyze_group_lead(text: &str, custom_keywords: &[String]) -> Option<GroupLeadMatch> {
    let normalized_text = text.to_lowercase();
    if normalized_text.trim().is_empty() {
        return None;
    }

    let mut keyword_pool = BTreeSet::new();
    for (keywords, _) in LEAD_PROFILE_RULES {
        for keyword in *keywords {
            keyword_pool.insert((*keyword).to_string());
        }
    }
    for keyword in custom_keywords {
        let keyword = keyword.trim();
        if !keyword.is_empty() {
            keyword_pool.insert(keyword.to_string());
        }
    }

    let matched_keywords = keyword_pool
        .into_iter()
        .filter(|keyword| normalized_text.contains(&keyword.to_lowercase()))
        .collect::<Vec<_>>();
    if matched_keywords.is_empty() {
        return None;
    }

    let matched_lower = matched_keywords
        .iter()
        .map(|keyword| keyword.to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut profile_tags = BTreeSet::new();
    for (keywords, tag) in LEAD_PROFILE_RULES {
        if keywords.iter().any(|keyword| {
            let keyword = keyword.to_lowercase();
            matched_lower.contains(&keyword) || normalized_text.contains(&keyword)
        }) {
            profile_tags.insert((*tag).to_string());
        }
    }

    Some(GroupLeadMatch {
        matched_keywords,
        profile_tags: profile_tags.into_iter().collect(),
    })
}

pub fn parse_custom_keywords(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub async fn apply_group_message_policy(
    pool: &PgPool,
    input: GroupMessagePolicyInput<'_>,
) -> Result<GroupMessagePolicySummary, sqlx::Error> {
    let advertising = detect_advertising_signals(input.content, "group_message");
    let findings = serde_json::to_string(&advertising.findings).unwrap_or_else(|_| "[]".into());
    let updated = sqlx::query(
        "UPDATE groups.group_messages \
         SET is_advertising = $4, ad_score = $5, ad_confidence = $6, ad_findings = $7 \
         WHERE tenant_id = $1 AND group_id = $2 AND tg_msg_id = $3",
    )
    .bind(input.tenant_id)
    .bind(input.group_id)
    .bind(input.telegram_message_id)
    .bind(advertising.is_advertising)
    .bind(i32::try_from(advertising.ad_score).unwrap_or(i32::MAX))
    .bind(&advertising.confidence)
    .bind(findings)
    .execute(pool)
    .await?
    .rows_affected();

    if updated == 0 {
        return Ok(GroupMessagePolicySummary::default());
    }

    let lead = analyze_group_lead(input.content, input.custom_keywords);
    if let Some(lead) = lead.as_ref() {
        let keywords = join_nonempty(&lead.matched_keywords);
        let profile_tags = join_nonempty(&lead.profile_tags);
        sqlx::query(
            r#"
            INSERT INTO tenant_saas.potential_users AS potential
              (tenant_id, user_id, username, first_name, last_name, message_text, keywords,
               profile_tags, group_id, group_title, captured_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now(), now())
            ON CONFLICT (tenant_id, user_id) DO UPDATE SET
              username = COALESCE(EXCLUDED.username, potential.username),
              first_name = COALESCE(EXCLUDED.first_name, potential.first_name),
              last_name = COALESCE(EXCLUDED.last_name, potential.last_name),
              message_text = EXCLUDED.message_text,
              keywords = (
                SELECT string_agg(value, ',' ORDER BY value)
                FROM (
                  SELECT DISTINCT btrim(item) AS value
                  FROM unnest(string_to_array(concat_ws(',', potential.keywords, EXCLUDED.keywords), ',')) item
                  WHERE btrim(item) <> ''
                ) merged_keywords
              ),
              profile_tags = (
                SELECT string_agg(value, ',' ORDER BY value)
                FROM (
                  SELECT DISTINCT btrim(item) AS value
                  FROM unnest(string_to_array(concat_ws(',', potential.profile_tags, EXCLUDED.profile_tags), ',')) item
                  WHERE btrim(item) <> ''
                ) merged_tags
              ),
              group_id = EXCLUDED.group_id,
              group_title = EXCLUDED.group_title,
              captured_at = now(),
              updated_at = now()
            "#,
        )
        .bind(input.tenant_id)
        .bind(input.user_id)
        .bind(input.username)
        .bind(input.first_name)
        .bind(input.last_name)
        .bind(input.content)
        .bind(keywords)
        .bind(profile_tags)
        .bind(input.group_id)
        .bind(input.group_title)
        .execute(pool)
        .await?;
    }

    Ok(GroupMessagePolicySummary {
        message_updated: true,
        is_advertising: advertising.is_advertising,
        lead_captured: lead.is_some(),
    })
}

fn join_nonempty(values: &[String]) -> Option<String> {
    let joined = values.join(",");
    (!joined.is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_default_keywords_and_tags() {
        let result = analyze_group_lead("想谈商务合作，也在找投资", &[]).unwrap();
        assert!(result.matched_keywords.contains(&"合作".to_string()));
        assert!(result.matched_keywords.contains(&"商务".to_string()));
        assert!(result.matched_keywords.contains(&"投资".to_string()));
        assert_eq!(result.profile_tags, vec!["合作意向", "投融资"]);
    }

    #[test]
    fn detects_custom_keywords_without_inventing_tags() {
        let custom_keywords = vec!["VIP 通道".to_string()];
        let result = analyze_group_lead("需要 vip 通道", &custom_keywords).unwrap();
        assert_eq!(result.matched_keywords, vec!["VIP 通道"]);
        assert!(result.profile_tags.is_empty());
    }

    #[test]
    fn ignores_empty_or_unmatched_messages() {
        assert_eq!(analyze_group_lead("", &[]), None);
        assert_eq!(analyze_group_lead("今天天气不错", &[]), None);
        assert_eq!(
            parse_custom_keywords(Some(" 合作,合作,  定制 ")),
            vec!["合作", "定制"]
        );
    }

    #[test]
    fn joins_only_nonempty_collections() {
        assert_eq!(join_nonempty(&[]), None);
        assert_eq!(
            join_nonempty(&["合作".to_string(), "投资".to_string()]),
            Some("合作,投资".to_string())
        );
    }
}
