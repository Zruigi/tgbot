//! 客服消息自动翻译（用户 → 管理员，译为中文）
//!
//! 移植自 Python `services/translator.py` + `services/security_shield.py`：
//! 启发式判断是否需要翻译 → 地址脱敏 → LLM 结构化翻译（【译文】【术语注解】）
//! → 指令泄露防御 → 地址回填 + 一致性校验。
//! 与 Python 老逻辑保持一致：不依赖 `global_auto_translate` 之外的前端开关，
//! 开关默认开启（get_config 默认 true），关闭后本模块直接跳过。

use regex::Regex;
use sqlx::{PgPool, Row};
use std::sync::OnceLock;
use tracing::warn;
use tgbot_integrations::litellm::LiteLLMClient;

// ---------------------------------------------------------------------------
// 启发式判断（对齐 translator.py should_translate_user_to_admin）
// ---------------------------------------------------------------------------

fn is_chinese(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 常见英文聊天填充词/语气词（中文用户混用时多为寒暄，不触发机翻）
#[allow(dead_code)]
const CASUAL_ENGLISH_WORDS: &[&str] = &[
    "a", "an", "the", "i", "u", "im", "you", "we", "it", "is", "are", "am", "be", "to", "do",
    "so", "my", "me", "and", "or", "but", "also", "too", "very", "really", "just", "only", "not",
    "all", "any", "some", "that", "this", "ok", "okay", "kk", "k", "yes", "no", "yeah", "yep",
    "nope", "nah", "yup", "ya", "yea", "good", "nice", "cool", "fine", "great", "wow", "omg",
    "lol", "haha", "hehe", "rofl", "lmao", "hi", "hey", "hello", "bye", "pls", "please", "thx",
    "thanks", "thank", "ty", "sry", "sorry", "gm", "gn", "morning", "night", "afternoon",
    "evening", "sure", "maybe", "well", "hmm", "um", "uh", "emm", "mmm", "perfect", "awesome",
    "super", "cute", "love", "wtf", "idk", "ikr", "tbh", "imo", "fyi", "np", "gl", "hf", "wait",
    "hold", "stop", "go", "see", "look", "got", "get", "come", "back", "bro", "dude", "man",
    "miss", "sir", "mam", "babe",
];

/// 出现任一即视为可能含专业/业务语义，需要翻译
#[allow(dead_code)]
const PROFESSIONAL_HINTS: &[&str] = &[
    "withdraw", "deposit", "wallet", "blockchain", "crypto", "bitcoin", "ethereum", "usdt", "btc",
    "eth", "bnb", "trx", "fee", "gas", "staking", "contract", "liquidat", "margin", "leverage",
    "kyc", "aml", "verify", "verification", "pending", "confirm", "network", "chain", "token",
    "defi", "nft", "mining", "validator", "address", "hash", "transaction", "transfer", "balance",
    "order", "api", "key", "seed", "phrase", "private", "public", "sign", "message", "support",
    "ticket", "account", "bind", "unbind", "freeze", "risk", "audit", "interest", "apy", "apr",
    "yield", "farm", "pool", "swap", "bridge", "mainnet", "testnet", "rpc", "node", "sync",
    "error", "failed", "success",
];

/// okkk→ok、yesss→yes：连续 3 个及以上相同字母压成 1 个
#[allow(dead_code)]
fn normalize_repeated_letters(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut run = 0u32;
    let mut prev: Option<char> = None;
    for c in word.chars().flat_map(|c| c.to_lowercase()) {
        if Some(c) == prev {
            run += 1;
            if run >= 3 {
                continue;
            }
        } else {
            run = 1;
            prev = Some(c);
        }
        out.push(c);
    }
    out
}

#[allow(dead_code)]
fn looks_like_stretched_casual(word: &str) -> bool {
    let w = word.to_lowercase();
    ["^o+k+$", "^g+o+d+$", "^n+i+c+e+$", "^y+e+s+$"]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .any(|r| r.is_match(&w))
}

#[allow(dead_code)]
fn word_is_casual_filler(word: &str) -> bool {
    let w = word.trim_matches(&['\'', '"'][..]).to_lowercase();
    if w.is_empty() {
        return true;
    }
    if looks_like_stretched_casual(&w) {
        return true;
    }
    if CASUAL_ENGLISH_WORDS.contains(&w.as_str()) {
        return true;
    }
    let w2 = normalize_repeated_letters(&w);
    if CASUAL_ENGLISH_WORDS.contains(&w2.as_str()) {
        return true;
    }
    if w.chars().count() <= 2 && w.chars().all(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    false
}

/// 提取 [a-zA-Z']+ 词元（对齐 Python re.findall）
#[allow(dead_code)]
fn extract_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, c) in text.char_indices() {
        let is_word_char = c.is_ascii_alphabetic() || c == '\'';
        if is_word_char && start.is_none() {
            start = Some(idx);
        } else if !is_word_char && start.is_some() {
            words.push(&text[start.unwrap()..idx]);
            start = None;
        }
    }
    if let Some(s) = start {
        words.push(&text[s..]);
    }
    words
}

/// 用户 -> 管理员：是否值得调用翻译（纯中文/无字母不翻）
pub fn should_translate_user_to_admin(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    // 纯中文不翻译（管理员直接可读）
    if is_chinese(text) {
        return false;
    }
    // 无英文字母（纯符号/数字/emoji）不翻译
    if !text.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // 客服场景：用户发外语（含英文）一律翻译给管理员，不做闲聊词过滤。
    // （Python 启发式同样会翻 "Are you OK?" 这类客服寒暄）
    true
}

// ---------------------------------------------------------------------------
// 地址脱敏（对齐 security_shield.py）
// ---------------------------------------------------------------------------

/// 加密地址正则：BTC / ETH-BSC / TRON / Monero
const ADDR_PATTERN: &str = r"(?:[13][a-km-zA-HJ-NP-Z1-9]{25,34}|0x[a-fA-F0-9]{40}|T[A-Za-z0-9]{33}|4[0-9a-zA-Z]{94})";

fn addr_regex() -> &'static Regex {
    static ADDR: OnceLock<Regex> = OnceLock::new();
    ADDR.get_or_init(|| Regex::new(ADDR_PATTERN).expect("addr regex"))
}

/// 提取地址并替换为占位符，返回（脱敏文本, 占位符→地址映射）
fn shield_text(text: &str) -> (String, Vec<(String, String)>) {
    let mut mapping = Vec::new();
    let mut shielded = text.to_string();
    for (i, m) in addr_regex().find_iter(text).enumerate() {
        let placeholder = format!("[[ADDR_{i}]]");
        mapping.push((placeholder.clone(), m.as_str().to_string()));
        shielded = shielded.replace(m.as_str(), &placeholder);
    }
    (shielded, mapping)
}

/// 回填占位符
fn unshield_text(shielded: &str, mapping: &[(String, String)]) -> String {
    let mut final_text = shielded.to_string();
    for (placeholder, addr) in mapping {
        final_text = final_text.replace(placeholder, addr);
    }
    final_text
}

/// 强制一致性校验：原文与最终文本中的地址集合必须完全一致
fn verify_address_consistency(original: &str, final_text: &str) -> bool {
    let orig: Vec<String> = addr_regex()
        .find_iter(original)
        .map(|m| m.as_str().to_string())
        .collect();
    let final_addrs: Vec<String> = addr_regex()
        .find_iter(final_text)
        .map(|m| m.as_str().to_string())
        .collect();
    let (mut a, mut b) = (orig, final_addrs);
    a.sort();
    b.sort();
    a == b
}

// ---------------------------------------------------------------------------
// LLM 翻译（对齐 translator.py translate_text_with_shield）
// ---------------------------------------------------------------------------

const YIWEN_TAG: &str = "【译文】";
const ZHUSHI_TAG: &str = "【术语注解】";

fn parse_llm_translation_output(raw: &str) -> String {
    let t = raw.trim();
    if !t.contains(YIWEN_TAG) {
        return t.to_string();
    }
    let body_start = t.find(YIWEN_TAG).unwrap() + YIWEN_TAG.len();
    let body_end = t.find(ZHUSHI_TAG).unwrap_or(t.len());
    let body = t[body_start..body_end.min(t.len())].trim().to_string();
    let mut note = String::new();
    if let Some(ni) = t.find(ZHUSHI_TAG) {
        let note_start = ni + ZHUSHI_TAG.len();
        if note_start <= t.len() {
            note = t[note_start..].trim().to_string();
        }
    }
    if !note.is_empty() && !["无", "无。", "none", "None", "N/A"].contains(&note.as_str()) {
        return format!("{body}\n\n{ZHUSHI_TAG}\n{note}");
    }
    body
}

fn build_system_prompt_for_user_to_admin() -> String {
    "你是加密货币客服场景的专业翻译。将用户消息译为流畅的简体中文。\n\
     输出格式（必须严格遵守，方便系统解析）：\n\
     【译文】\n\
     （这里只写中文译文正文，保留占位符 [[ADDR_N]] 不变）\n\
     【术语注解】\n\
     若存在专业术语、项目名、缩写（如 KYC、DeFi、爆仓、链上确认），用一行或多行简短解释；\
     若没有需要解释的术语，只写一个字：无\n\
     不要输出任何其它前言、Markdown 代码块或角色扮演。\n"
        .to_string()
}

/// 读取全局翻译开关（默认开启）
async fn global_translate_enabled(pool: &PgPool) -> bool {
    match sqlx::query_scalar::<_, String>(
        "SELECT value FROM platform.admin_settings WHERE key = 'global_auto_translate'",
    )
    .fetch_optional(pool)
    .await
    {
        Ok(Some(value)) => matches!(
            value.trim().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        _ => true,
    }
}

/// 翻译入口：返回译文（与原文不同才有值），不需要翻译/失败返回 None。
pub async fn translate_to_admin(
    llm: &LiteLLMClient,
    pool: &PgPool,
    text: &str,
) -> Option<String> {
    if !global_translate_enabled(pool).await {
        return None;
    }
    if !should_translate_user_to_admin(text) {
        return None;
    }

    // 地址脱敏
    let (shielded, mapping) = shield_text(text);
    if shielded.trim().is_empty() {
        return None;
    }

    let system_prompt = build_system_prompt_for_user_to_admin();
    let user_prompt = format!("请翻译以下用户消息：\n{shielded}");

    let raw = match llm.simple_chat(&system_prompt, &user_prompt).await {
        Ok(raw) => raw,
        Err(error) => {
            warn!(%error, "translate_to_admin: LLM call failed, skip translation");
            return None;
        }
    };

    let mut translated = parse_llm_translation_output(&raw);

    // 过滤 AI 可能复读的提示词标签
    for label in [
        "Text to translate:",
        "Translated text:",
        "Simplified Chinese:",
        "### TRANSLATION:",
    ] {
        translated = translated.replace(label, "").trim().to_string();
    }

    // 指令泄露防御：出现翻译指令关键词 → 丢弃译文
    let lower = translated.to_lowercase();
    let leaked = [
        "dịch sang",
        "translate to",
        "翻译成",
        "占位符",
        "保持不变",
        "output only",
        "original text",
        "target language",
    ]
    .iter()
    .any(|kw| lower.contains(*kw));
    if leaked {
        return None;
    }

    let translated = translated.trim_matches(&['"', '\''][..]).to_string();

    // 地址回填 + 一致性校验
    let final_text = unshield_text(&translated, &mapping);
    if !verify_address_consistency(text, &final_text) {
        return None;
    }
    let final_text = final_text.trim().to_string();
    if final_text.is_empty() || final_text == text.trim() {
        return None;
    }
    Some(final_text)
}

/// 简单语言检测（对齐 Python langdetect 的轻量替代）：
/// 返回 ISO-639-1 语言码 + 用于 LLM 的目标语言描述。
/// 按字符集启发式判断，覆盖中文/俄文/阿拉伯文/日韩文，默认英文。
pub fn detect_language(text: &str) -> &'static str {
    // 日文假名 / 韩文谚文优先（日文常混用汉字，需在 CJK 判断前检测）
    let jp = text
        .chars()
        .filter(|c| ('\u{3040}'..='\u{30ff}').contains(c))
        .count();
    if jp > 0 {
        return "ja";
    }
    let kr = text
        .chars()
        .filter(|c| ('\u{ac00}'..='\u{d7af}').contains(c))
        .count();
    if kr > 0 {
        return "ko";
    }
    let cjk = text
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    if cjk > 0 {
        return "zh-CN";
    }
    let cyrillic = text
        .chars()
        .filter(|c| ('\u{0400}'..='\u{04ff}').contains(c))
        .count();
    if cyrillic > 0 {
        return "ru";
    }
    let arabic = text
        .chars()
        .filter(|c| ('\u{0600}'..='\u{06ff}').contains(c))
        .count();
    if arabic > 0 {
        return "ar";
    }
    "en"
}

/// 管理员 → 用户翻译（双向翻译的回复方向）。
/// 用户消息语言非中文、且管理员回复含中文时，译为用户语言；
/// 否则返回 None（不翻译）。带地址脱敏/回填/一致性校验。

/// 取用户最近的对话上下文（管理员/用户交替，最多 limit 条），供翻译理解语境。
async fn recent_dialog_context(
    pool: &PgPool,
    user_id: i64,
    tenant_id: &str,
    limit: i64,
) -> String {
    let Ok(rows) = sqlx::query(
        "SELECT from_userid, content FROM chat.messages          WHERE (from_userid = $1 OR to_userid = $1) AND tenant_id = $2            AND is_deleted = false AND msg_type = 'text'            AND NULLIF(btrim(content), '') IS NOT NULL          ORDER BY id DESC LIMIT $3",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    else {
        return String::new();
    };
    rows.iter()
        .rev()
        .filter_map(|row| {
            let from: Option<i64> = row.try_get("from_userid").ok();
            let content: Option<String> = row.try_get("content").ok();
            match (from, content) {
                (Some(from), Some(content)) if from == user_id => {
                    Some(format!("用户: {}", content.chars().take(120).collect::<String>()))
                }
                (Some(_), Some(content)) => {
                    Some(format!("客服: {}", content.chars().take(120).collect::<String>()))
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn translate_to_customer(
    llm: &LiteLLMClient,
    pool: &PgPool,
    content: &str,
    customer_msg: &str,
    user_id: i64,
    tenant_id: &str,
) -> Option<String> {
    if !global_translate_enabled(pool).await {
        return None;
    }
    // 用户消息是中文 → 无需翻译
    if detect_language(customer_msg) == "zh-CN" {
        return None;
    }
    // 管理员回复本身非中文（已是用户语言或无需翻译）→ 不翻
    if !is_chinese(content) {
        return None;
    }

    let target_lang = detect_language(customer_msg);
    let system_prompt = format!(
        "你是专业翻译。将管理员回复译为目标语言：{target_lang}。\n\
         只输出译文，不要解释。保留占位符 [[ADDR_N]] 不变。\n\
         若原文已是目标语言，保持原文。\n\
         结合下方最近的对话上下文理解语境，保持术语与语气一致。"
    );

    let (shielded, mapping) = shield_text(content);
    if shielded.trim().is_empty() {
        return None;
    }
    // 最近对话上下文（最多 6 条）：让翻译理解上下文关联（对齐用户诉求）
    let context = recent_dialog_context(pool, user_id, tenant_id, 6).await;
    let user_prompt = if context.is_empty() {
        shielded.clone()
    } else {
        format!("最近的对话上下文（仅供理解语境）：\n{context}\n\n请翻译这条管理员回复：\n{shielded}")
    };
    let raw = match llm.simple_chat(&system_prompt, &user_prompt).await {
        Ok(raw) => raw,
        Err(error) => {
            warn!(%error, "translate_to_customer: LLM call failed, skip translation");
            return None;
        }
    };
    let translated = raw.trim_matches(&['"', '\''][..]).trim().to_string();
    let final_text = unshield_text(&translated, &mapping);
    if !verify_address_consistency(content, &final_text) {
        return None;
    }
    let final_text = final_text.trim().to_string();
    if final_text.is_empty() || final_text == content.trim() {
        return None;
    }
    Some(final_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_skips_translation() {
        assert!(!should_translate_user_to_admin("你好，请问怎么充值？"));
    }

    #[test]
    fn symbols_and_numbers_skip_translation() {
        assert!(!should_translate_user_to_admin("👍👍"));
        assert!(!should_translate_user_to_admin("123456"));
        assert!(!should_translate_user_to_admin("###"));
    }

    #[test]
    fn short_english_always_translates() {
        assert!(should_translate_user_to_admin("Are you OK?"));
        assert!(should_translate_user_to_admin("ok thanks"));
        assert!(should_translate_user_to_admin("hello"));
        assert!(should_translate_user_to_admin("I want to withdraw usdt please"));
    }

    #[test]
    fn professional_hint_triggers_translation() {
        assert!(should_translate_user_to_admin("I want to withdraw usdt please"));
    }

    #[test]
    fn long_sentence_triggers_translation() {
        let long = "this is a fairly long english sentence that contains more than fifty two characters in total";
        assert!(should_translate_user_to_admin(long));
    }

    #[test]
    fn address_is_shielded_and_restored() {
        let addr = "0x1234567890abcdef1234567890abcdef12345678";
        let text = format!("send to {addr} please");
        let (shielded, mapping) = shield_text(&text);
        assert!(shielded.contains("[[ADDR_0]]"));
        assert!(!shielded.contains(addr));
        let restored = unshield_text(&shielded, &mapping);
        assert_eq!(restored, text);
        assert!(verify_address_consistency(&text, &restored));
    }

    #[test]
    fn parse_translation_output_with_note() {
        let raw = "【译文】\n请把资金转到我的钱包\n【术语注解】\nKYC：身份验证";
        let parsed = parse_llm_translation_output(raw);
        assert!(parsed.contains("钱包"));
        assert!(parsed.contains("KYC"));
    }

    #[test]
    fn detects_languages() {
        assert_eq!(detect_language("你好，请问怎么充值？"), "zh-CN");
        assert_eq!(detect_language("I want to withdraw usdt"), "en");
        assert_eq!(detect_language("Как вывести средства?"), "ru");
        assert_eq!(detect_language("كيف يمكنني السحب؟"), "ar");
        assert_eq!(detect_language("お金を引き出したいです"), "ja");
        assert_eq!(detect_language("출금하고 싶습니다"), "ko");
    }
}
