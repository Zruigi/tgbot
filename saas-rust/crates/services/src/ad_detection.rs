// 广告识别核心逻辑（可被群消息管道与 AI Tool 复用）
// 对应 Python: services/ai_agent/ad_detection.py

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// 广告关键词
// ---------------------------------------------------------------------------

static AD_KEYWORDS: &[&str] = &[
    // 直接推广
    "加我",
    "扫码",
    "微信",
    "vx:",
    "VX:",
    "v信",
    "免费领取",
    "红包",
    "返利",
    "佣金",
    "代理",
    "拉人头",
    "邀请码",
    "注册送",
    // 投资理财诈骗
    "稳赚",
    "翻倍",
    "百倍",
    "暴富",
    "零风险",
    "内部消息",
    "内幕",
    "必涨",
    "抄底",
    // 违规平台
    "电报群",
    "telegram群",
    "飞机群",
    "梯子",
    // 黑产/灰产关键词
    "黑u",
    "黒u",
    "假钞",
    "接码",
    "料子",
    "一手料",
    "出料",
    "数据",
    "ssn",
    "护照",
    "驾照",
    "身份证",
    "企业套件",
    "实名",
    "三件套",
    "苹果pay",
    "apple pay",
    "绑好的",
    "网银",
    "鱼",
    "菠菜",
    "开云",
    "招商",
    "注册赠送",
    "彩金",
    "日入",
    "收益日结",
    "代刷",
    "广告截图",
];

// ---------------------------------------------------------------------------
// 正则模式
// ---------------------------------------------------------------------------

static URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
static PHONE_LIKE_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{5,11}").unwrap());

static TELEGRAM_CONTACT_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[^A-Za-z0-9_.])(@[A-Za-z0-9_]{4,32})\b").unwrap());

static PROMO_PHRASES: &[&str] = &[
    "长期",
    "稳定",
    "源头",
    "货源",
    "需要的",
    "滴滴",
    "主营",
    "出售",
    "可挑",
    "全网",
];

fn repeated_character_run_count(text: &str, minimum_run_length: usize) -> usize {
    let mut previous = None;
    let mut current_run_length = 0;
    let mut run_count = 0;

    for character in text.chars() {
        if previous == Some(character) {
            current_run_length += 1;
        } else {
            previous = Some(character);
            current_run_length = 1;
        }

        if current_run_length == minimum_run_length {
            run_count += 1;
        }
    }

    run_count
}

// ---------------------------------------------------------------------------
// AdDetectionResult
// ---------------------------------------------------------------------------

/// 广告检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdDetectionResult {
    pub is_advertising: bool,
    pub ad_score: u32,
    pub confidence: String,
    pub risk_level: String,
    pub findings: Vec<String>,
    pub matched_keywords: Vec<String>,
    pub url_count: usize,
    pub context_type: String,
    pub recommendation: String,
}

// ---------------------------------------------------------------------------
// 核心检测函数
// ---------------------------------------------------------------------------

/// 识别文本中的广告/推广特征并返回结构化评分结果
pub fn detect_advertising_signals(text: &str, context_type: &str) -> AdDetectionResult {
    let raw = text.trim();
    if raw.is_empty() {
        return AdDetectionResult {
            is_advertising: false,
            ad_score: 0,
            confidence: "low".into(),
            risk_level: "low".into(),
            findings: vec![],
            matched_keywords: vec![],
            url_count: 0,
            context_type: context_type.into(),
            recommendation: "文本为空，无法判定".into(),
        };
    }

    let raw_lower = raw.to_lowercase();
    let mut findings: Vec<String> = Vec::new();
    let mut ad_score: u32 = 0;

    // 1. 广告关键词匹配
    let matched_keywords: Vec<String> = AD_KEYWORDS
        .iter()
        .filter(|kw| raw_lower.contains(&kw.to_lowercase()))
        .map(|kw| kw.to_string())
        .collect();

    if !matched_keywords.is_empty() {
        ad_score += std::cmp::min((matched_keywords.len() as u32) * 15, 45);
        findings.push(format!("命中广告关键词: {}", matched_keywords.join(", ")));
    }

    // 2. URL 检测
    let urls: Vec<&str> = URL_PATTERN.find_iter(raw).map(|m| m.as_str()).collect();
    let url_count = urls.len();

    if urls.len() >= 3 {
        ad_score += 20;
        findings.push(format!("包含 {} 个链接（可能为批量推广）", urls.len()));
    } else if !urls.is_empty() {
        let suspicious_urls: Vec<&&str> = urls
            .iter()
            .filter(|u| {
                let u_lower = u.to_lowercase();
                u_lower.contains("t.me/")
                    || u_lower.contains("shorturl")
                    || u_lower.contains("bit.ly")
            })
            .collect();
        if !suspicious_urls.is_empty() {
            ad_score += 15;
            findings.push(format!("发现可疑链接: {:?}", suspicious_urls));
        }
    }

    // 3. 重复字符检测
    if repeated_character_run_count(raw, 6) > 3 {
        ad_score += 10;
        findings.push("存在大量重复字符（刷屏特征）".into());
    }

    // 4. 电话号码检测
    let phone_like: Vec<regex::Match<'_>> = PHONE_LIKE_PATTERN.find_iter(raw).collect();
    if phone_like.len() >= 2 {
        ad_score += 15;
        findings.push("发现多个疑似电话号码/ID".into());
    }

    // 5. Telegram 联系方式检测
    let telegram_contacts: Vec<&str> = TELEGRAM_CONTACT_PATTERN
        .captures_iter(raw)
        .filter_map(|captures| captures.get(1).map(|matched| matched.as_str()))
        .collect();
    if telegram_contacts.len() == 1 {
        ad_score += 20;
        findings.push(format!("发现 Telegram 联系方式: {}", telegram_contacts[0]));
    } else if telegram_contacts.len() >= 2 {
        ad_score += 30;
        findings.push(format!(
            "发现多个 Telegram 联系方式: {}",
            telegram_contacts.join(", ")
        ));
    }

    // 6. 推广话术检测
    let promo_count = PROMO_PHRASES
        .iter()
        .filter(|phrase| raw_lower.contains(**phrase))
        .count();
    if promo_count >= 2 {
        ad_score += 20;
        findings.push(format!(
            "命中推广话术 ({} 条): 包含多个推广常用词",
            promo_count
        ));
    }

    // 综合评估
    let is_ad = ad_score >= 40;
    let confidence = if ad_score >= 60 {
        "high"
    } else if ad_score >= 40 {
        "medium"
    } else {
        "low"
    };
    let risk_level = confidence; // 与 confidence 保持一致

    let recommendation = if is_ad {
        if ad_score >= 60 {
            "建议拦截该消息（高置信度广告）".into()
        } else {
            "疑似广告，建议人工复核".into()
        }
    } else if ad_score < 20 {
        "暂未检测到明显广告特征".into()
    } else {
        "有轻微推广特征，建议持续观察".into()
    };

    AdDetectionResult {
        is_advertising: is_ad,
        ad_score,
        confidence: confidence.into(),
        risk_level: risk_level.into(),
        findings,
        matched_keywords,
        url_count,
        context_type: context_type.into(),
        recommendation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_text() {
        let result = detect_advertising_signals("", "group_message");
        assert!(!result.is_advertising);
        assert_eq!(result.ad_score, 0);
    }

    #[test]
    fn test_ad_keywords() {
        let result = detect_advertising_signals("加我微信免费领取红包返利佣金", "group_message");
        assert!(result.is_advertising);
        assert!(result.ad_score >= 40);
        assert!(!result.matched_keywords.is_empty());
    }

    #[test]
    fn test_normal_text() {
        let result = detect_advertising_signals("今天天气不错，大家好", "group_message");
        assert!(!result.is_advertising);
    }

    #[test]
    fn test_suspicious_urls() {
        let result = detect_advertising_signals(
            "点击链接 https://t.me/joinchat/abc 和 https://bit.ly/xyz 赚钱",
            "group_message",
        );
        assert!(result.findings.iter().any(|f| f.contains("可疑链接")));
    }

    #[test]
    fn test_many_urls() {
        let text = "https://a.com https://b.com https://c.com https://d.com";
        let result = detect_advertising_signals(text, "group_message");
        assert!(result.ad_score >= 20);
    }

    #[test]
    fn test_telegram_contact_single() {
        let result = detect_advertising_signals("联系我 @username123 详谈", "group_message");
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.contains("Telegram 联系方式")),
            "应检测到单个 Telegram 联系方式"
        );
        assert_eq!(result.ad_score, 20);
    }

    #[test]
    fn test_telegram_contact_multiple() {
        let result =
            detect_advertising_signals("加 @user1 或 @user2 或 @user3 都可以", "group_message");
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.contains("多个 Telegram 联系方式")),
            "应检测到多个 Telegram 联系方式"
        );
        assert!(result.ad_score >= 30);
    }

    #[test]
    fn test_telegram_contact_no_false_positive() {
        let result =
            detect_advertising_signals("请发送邮件到 support@example.com", "group_message");
        let has_telegram = result.findings.iter().any(|f| f.contains("Telegram"));
        assert!(!has_telegram, "普通邮箱不应被误判为 Telegram 联系方式");
    }

    #[test]
    fn test_promo_phrases() {
        let result =
            detect_advertising_signals("长期稳定货源，全国可发，需要的滴滴", "group_message");
        let promo_finding = result.findings.iter().find(|f| f.contains("推广话术"));
        assert!(promo_finding.is_some(), "预期命中推广话术，但未找到");
        assert!(result.ad_score >= 20);
    }

    #[test]
    fn test_promo_phrases_insufficient() {
        let result = detect_advertising_signals("长期供应优质产品", "group_message");
        let promo_finding = result.findings.iter().find(|f| f.contains("推广话术"));
        assert!(promo_finding.is_none(), "仅一个推广词不应触发话术检测");
    }
}
