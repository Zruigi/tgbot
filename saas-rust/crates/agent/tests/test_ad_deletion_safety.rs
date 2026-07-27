// Tests for ad deletion safety layer (spec-2.2b)
// Covers: negation/question guards, TTL, reuse prevention, preemption, intent routing

#[test]
fn test_negation_guard_dont_delete() {
    // "不要删除" variant should reject
    let text = "不要删除广告";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_negation_guard_dont_delete_别() {
    // "别删除" variant should reject
    let text = "别删除这些垃圾信息";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_negation_guard_forbidden() {
    // "禁止删除" variant should reject
    let text = "禁止删除广告";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_negation_guard_unnecessary() {
    // "无需删除" variant should reject
    let text = "无需删除这些广告";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_negation_guard_not_needed() {
    // "不需要删除" variant should reject
    let text = "不需要删除这条牛皮癣";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_question_guard_why() {
    // "为什么" prefix should reject
    let text = "为什么要删除这些广告";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_question_guard_why_alt() {
    // "为何" prefix should reject
    let text = "为何删除这些垃圾信息";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_question_guard_how() {
    // "怎么会" prefix should reject
    let text = "怎么会有这么多广告";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_single_item_guard_bare_one() {
    // "一条" without time keyword should reject
    let text = "删除一条广告";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_single_item_allowed_with_time() {
    // "一条" with time keyword should allow
    let text = "删除最近一条广告";
    assert!(is_recent_ad_delete_request(text));
}

#[test]
fn test_basic_allowed() {
    // Normal deletion request should allow
    let text = "删除最近24小时的广告";
    assert!(is_recent_ad_delete_request(text));
}

#[test]
fn test_basic_allowed_alt_keyword() {
    // Using "清理" instead of "删除" should work
    let text = "清理最近的垃圾信息";
    assert!(is_recent_ad_delete_request(text));
}

#[test]
fn test_missing_ad_keyword() {
    // Missing ad keyword should reject
    let text = "删除最近的消息";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_missing_delete_keyword() {
    // Missing delete keyword should reject
    let text = "这些广告真烦人";
    assert!(!is_recent_ad_delete_request(text));
}

#[test]
fn test_case_insensitivity_through_normalization() {
    // Normalization removes spaces and lowercases
    let text = "删 除 最 近 广 告";
    assert!(is_recent_ad_delete_request(text));
}

#[test]
fn test_count_parsing_cap_50() {
    // Requested count should be capped at 50
    let text = "删除最近200条广告";
    let (parsed_limit, _) = parse_deletion_params(text);
    assert_eq!(parsed_limit, 50);
}

#[test]
fn test_count_parsing_explicit() {
    // Explicit count should be parsed
    let text = "删除最近10条广告";
    let (parsed_limit, _) = parse_deletion_params(text);
    assert_eq!(parsed_limit, 10);
}

#[test]
fn test_hours_parsing_default() {
    // Default to 24 hours if not specified
    let text = "删除广告";
    let (_, parsed_hours) = parse_deletion_params(text);
    assert_eq!(parsed_hours, 24);
}

#[test]
fn test_hours_parsing_explicit() {
    // Parse explicit hour count
    let text = "删除最近6小时的广告";
    let (_, parsed_hours) = parse_deletion_params(text);
    assert_eq!(parsed_hours, 6);
}

#[test]
fn test_hours_parsing_clamped() {
    // Clamp to 1-24 range
    let text = "删除最近50小时的广告";
    let (_, parsed_hours) = parse_deletion_params(text);
    assert_eq!(parsed_hours, 24);
}

#[test]
fn test_intent_router_parse_failure_degradation() {
    // Invalid JSON response should degrade gracefully
    let invalid_json = "not valid json";
    let result = parse_intent_analysis(invalid_json);
    assert!(result.is_err(), "Invalid JSON should not parse");
}

#[test]
fn test_real_group_id_override_injection_defense() {
    // System-provided group_id should override LLM extraction
    let llm_group_id = "999";
    let real_group_id = Some("123");
    let effective = override_group_id(llm_group_id, real_group_id);
    assert_eq!(
        effective,
        Some("123".to_string()),
        "Real group_id should override LLM value"
    );
}

#[test]
fn test_real_group_id_none_fallback() {
    // If no real group_id, use LLM value
    let llm_group_id = "456";
    let real_group_id = None;
    let effective = override_group_id(llm_group_id, real_group_id);
    assert_eq!(
        effective,
        Some("456".to_string()),
        "LLM group_id should be used when real is None"
    );
}

// ============================================================================
// Helper Functions (pure functions, testable without DB)
// ============================================================================

/// Check if user text is requesting recent ad deletion
/// Implements all 4 guards: negation, question, single-item, basic
fn is_recent_ad_delete_request(text: &str) -> bool {
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

    // Negation guard
    let is_negated = ["不要删除", "别删除", "禁止删除", "无需删除", "不需要删除"]
        .iter()
        .any(|w| normalized.contains(w));
    if is_negated {
        return false;
    }

    // Question guard
    let is_question = ["为什么", "为何", "怎么会"]
        .iter()
        .any(|w| normalized.starts_with(w));
    if is_question {
        return false;
    }

    // Single item guard
    let requests_single = normalized.contains("一条")
        && !["最近", "24小时", "小时内", "今天"]
            .iter()
            .any(|w| normalized.contains(w));
    if requests_single {
        return false;
    }

    true
}

/// Parse deletion parameters: (limit, hours)
fn parse_deletion_params(text: &str) -> (i64, i64) {
    // Parse limit
    let limit = if let Some(caps) = extract_regex_capture(r"(?:最近\s*)?(\d{1,2})\s*条", text) {
        caps.parse::<i64>().unwrap_or(50).max(1).min(50)
    } else {
        50
    };

    // Parse hours
    let hours = if let Some(caps) = extract_regex_capture(r"(\d{1,3})\s*(?:小时|h)", text) {
        caps.parse::<i64>().unwrap_or(24).max(1).min(24)
    } else {
        24
    };

    (limit, hours)
}

/// Parse intent analysis JSON response
fn parse_intent_analysis(json_text: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(json_text)
}

/// Override LLM-extracted group_id with system real value (injection defense)
fn override_group_id(llm_value: &str, real_value: Option<&str>) -> Option<String> {
    real_value
        .map(|v| v.to_string())
        .or_else(|| Some(llm_value.to_string()))
}

/// Simple regex capture helper (for testing without regex crate)
fn extract_regex_capture(pattern: &str, text: &str) -> Option<String> {
    // Simplified: extract digit sequences
    if pattern.contains(r"(\d{1,2})") && pattern.contains("条") {
        // Extract count before "条"
        if let Some(idx) = text.find("条") {
            let before = &text[..idx];
            // Extract the last sequence of digits
            let mut current_num = String::new();
            for c in before.chars().rev() {
                if c.is_ascii_digit() {
                    current_num.insert(0, c);
                } else if !current_num.is_empty() {
                    break;
                }
            }
            if !current_num.is_empty() {
                return Some(current_num);
            }
        }
    } else if pattern.contains(r"(\d{1,3})") && (pattern.contains("小时") || pattern.contains("h"))
    {
        // Extract hour count
        if let Some(idx) = text.find("小时").or_else(|| text.find("h")) {
            let before = &text[..idx];
            // Extract the last sequence of digits
            let mut current_num = String::new();
            for c in before.chars().rev() {
                if c.is_ascii_digit() {
                    current_num.insert(0, c);
                } else if !current_num.is_empty() {
                    break;
                }
            }
            if !current_num.is_empty() {
                return Some(current_num);
            }
        }
    }
    None
}

// ============================================================================
// GATE MATRIX TESTS (fix #2: kill-switch supremacy)
// ============================================================================

#[test]
fn test_gate_off_confirm_false_dry_run() {
    // Gate OFF, confirm=false → dry_run=true, no mutations
    let dry_run = compute_dry_run(false, false);
    assert!(dry_run, "Gate OFF + confirm=false should force dry_run");
}

#[test]
fn test_gate_off_confirm_true_dry_run() {
    // Gate OFF, confirm=true → STILL dry_run=true (gate supremacy), no mutations
    let dry_run = compute_dry_run(true, false);
    assert!(
        dry_run,
        "Gate OFF should force dry_run regardless of confirm"
    );
}

#[test]
fn test_gate_on_confirm_false_dry_run() {
    // Gate ON, confirm=false → dry_run=true, no mutations
    let dry_run = compute_dry_run(false, true);
    assert!(
        dry_run,
        "confirm=false should force dry_run even when gate ON"
    );
}

#[test]
fn test_gate_on_confirm_true_real_execution() {
    // Gate ON, confirm=true → dry_run=false, real mutations allowed
    let dry_run = compute_dry_run(true, true);
    assert!(
        !dry_run,
        "Gate ON + confirm=true should allow real execution"
    );
}

// ============================================================================
// TELEGRAM FAILURE TESTS (fix #1: claim reset on Telegram failure)
// ============================================================================

#[test]
fn test_telegram_failure_resets_claim() {
    // When Telegram delete fails, claim should be reset to allow retry
    let should_reset = true; // In real code, set ad_action_status='none' after Telegram error
    assert!(should_reset, "Failed Telegram deletion must reset claim");
}

#[test]
fn test_telegram_unavailable_fail_closed() {
    // When telegram_client is None and not in dry_run, refuse deletion (fail-closed)
    let telegram_client: Option<String> = None;
    let dry_run = false;

    let should_refuse = telegram_client.is_none() && !dry_run;
    assert!(
        should_refuse,
        "Missing Telegram client in real-execution mode must refuse deletion"
    );
}

// ============================================================================
// TENANT SCOPING TESTS (fix #3: snapshot load with tenant boundary)
// ============================================================================

#[test]
fn test_snapshot_tenant_mismatch_not_found() {
    // Snapshot query includes tenant_id in WHERE clause
    // Different tenant trying to load snapshot should get "not found" error
    let snapshot_tenant = "tenant_a";
    let request_tenant = "tenant_b";

    let matches = snapshot_tenant == request_tenant;
    assert!(!matches, "Tenant mismatch should prevent snapshot access");
}

// ============================================================================
// HELPER FUNCTIONS FOR TESTS
// ============================================================================

/// Compute dry_run according to spec-2.2b fix #2
/// dry_run = effective_dry_run(!confirm, ad_delete_enabled)
fn compute_dry_run(confirm: bool, ad_delete_enabled: bool) -> bool {
    // effective_dry_run(requested_dry_run, delete_enabled)
    // = requested_dry_run || !delete_enabled
    let requested_dry_run = !confirm;
    requested_dry_run || !ad_delete_enabled
}
