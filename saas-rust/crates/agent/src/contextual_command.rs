// Contextual Command Resolution — short follow-up instruction expansion
// 对应 Python: services/ai_agent/core.py::AIAgent._resolve_contextual_command()

use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 汉字数字映射表（一 - 十）
fn chinese_to_number(s: &str) -> Option<usize> {
    match s {
        "一" => Some(1),
        "二" => Some(2),
        "三" => Some(3),
        "四" => Some(4),
        "五" => Some(5),
        "六" => Some(6),
        "七" => Some(7),
        "八" => Some(8),
        "九" => Some(9),
        "十" => Some(10),
        _ => None,
    }
}

/// 字符串归一化：转小写、去空格
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
}

/// 根据对话历史上下文展开简短跟进指令
///
/// 实现两类短命令的展开：
/// 1. 删除简写（"删除"等）→ 展开为完整删除指令
/// 2. 群序号跟进（"第N个"）→ 解析为具体群ID
///
/// 返回展开后的意图字符串或原文本
pub fn resolve_contextual_command(
    user_text: &str,
    metadata: &mut HashMap<String, Value>,
    previous_assistant_text: &str,
) -> String {
    // 用户显式指定群ID → 记录 active_group_id（side-effect，不改变返回文本）
    // Python: re.search(r"(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})", text, re.I)
    if let Ok(explicit_group_re) =
        Regex::new(r"(?i)(?:group_id|chat_id|群|群组)\s*[:：=]?\s*(-?\d{5,})")
    {
        if let Some(caps) = explicit_group_re.captures(user_text) {
            if let Some(group_id) = caps.get(1).and_then(|m| m.as_str().parse::<i64>().ok()) {
                metadata.insert("active_group_id".into(), json!(group_id));
            }
        }
    }

    // 情况 A: 删除指令简写
    if let Some(expanded) = expand_terse_delete(user_text, metadata, previous_assistant_text) {
        return expanded;
    }

    // 情况 B: 群序号跟进指令
    if let Some(expanded) = expand_ordinal_group_selection(user_text, metadata, previous_assistant_text) {
        return expanded;
    }

    // 默认：返回原文本
    user_text.to_string()
}

/// 情况 A: 删除指令简写处理
/// 检查是否需要将"删除"等简短指令展开为"删除最近24小时的广告 群 XXX"
fn expand_terse_delete(
    user_text: &str,
    metadata: &mut HashMap<String, Value>,
    previous_assistant_text: &str,
) -> Option<String> {
    let normalized = normalize(user_text);

    // 检查输入是否为删除简写之一
    let delete_keywords = ["删除", "删掉", "执行删除", "清理", "处理掉"];
    if !delete_keywords.iter().any(|kw| normalized == *kw) {
        return None;
    }

    // 检查上一轮回复中是否包含广告标志
    let ad_markers = ["条合格广告", "具备删除权限", "广告记录"];
    if !ad_markers.iter().any(|marker| previous_assistant_text.contains(marker)) {
        return None;
    }

    // 获取 last_ad_query_group_id
    // Python: str(last_ad_group_id or "").strip() — None/0 均视为无群ID（不加后缀）
    let last_group_id = metadata
        .get("last_ad_query_group_id")
        .and_then(|v| v.as_i64())
        .filter(|v| *v != 0);

    // 构造返回文本
    let result = if let Some(group_id) = last_group_id {
        format!("删除最近24小时的广告 群 {}", group_id)
    } else {
        "删除最近24小时的广告".to_string()
    };

    Some(result)
}

/// 情况 B: 群序号跟进指令处理
/// 检查是否需要将"第N个"展开为"查看群 XXX 最近群消息"
fn expand_ordinal_group_selection(
    user_text: &str,
    metadata: &mut HashMap<String, Value>,
    previous_assistant_text: &str,
) -> Option<String> {
    let normalized = normalize(user_text);

    // 编译正则表达式以匹配群序号模式
    let ordinal_pattern = Regex::new(
        r"第\s*(\d{1,2}|一|二|三|四|五|六|七|八|九|十)\s*个"
    ).ok()?;

    // 检查是否包含"第N个"模式
    let ordinal_match = ordinal_pattern.captures(&normalized)?;
    let token = ordinal_match.get(1)?.as_str();

    // 判断是否需要查询群消息
    // 极简形式("第一个"等) 或 包含关键词("群", "消息", "查看", "打开", "看看")
    let is_terse_ordinal = ["第一个", "第二个", "第三个", "第四个", "第五个"]
        .iter()
        .any(|s| normalized == *s);

    let has_context_word = ["群", "消息", "查看", "打开", "看看"]
        .iter()
        .any(|w| normalized.contains(w));

    if !is_terse_ordinal && !has_context_word {
        return None;
    }

    // 检查上一轮回复中是否包含"群组："标志
    if !previous_assistant_text.contains("群组：") {
        return None;
    }

    // 解析群ID序列
    // 1. 截取"群组："和"频道："之间的部分
    let group_section = if let Some(start) = previous_assistant_text.find("群组：") {
        let after_marker = &previous_assistant_text[start + "群组：".len()..];
        if let Some(end) = after_marker.find("频道：") {
            &after_marker[..end]
        } else {
            after_marker
        }
    } else {
        return None;
    };

    // 2. 从文本中提取所有 (-?\d{5,}) 格式的群ID（与 Python 正则 r"\((-?\d{5,})\)" 保持 bit-level 一致）
    let group_id_pattern = Regex::new(r"\((-?\d{5,})\)").ok()?;
    let group_ids: Vec<i64> = group_id_pattern
        .captures_iter(group_section)
        .filter_map(|cap| {
            cap.get(1)
                .and_then(|m| m.as_str().parse::<i64>().ok())
        })
        .collect();

    if group_ids.is_empty() {
        return None;
    }

    // 3. 将 token（数字或汉字）映射到索引
    let index = if let Ok(num) = token.parse::<usize>() {
        if num >= 1 && num <= 99 {
            num
        } else {
            return None;
        }
    } else if let Some(num) = chinese_to_number(token) {
        num
    } else {
        return None;
    };

    // 4. 检查索引是否有效
    if index < 1 || index > group_ids.len() {
        return None;
    }

    // 5. 获取选中的群ID
    let selected_group_id = group_ids[index - 1];

    // 6. 更新 metadata 中的 active_group_id
    metadata.insert("active_group_id".into(), json!(selected_group_id));

    // 7. 返回展开后的指令
    Some(format!("查看群 {} 最近群消息", selected_group_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terse_delete_with_ad_context() {
        let mut metadata = HashMap::new();
        metadata.insert("last_ad_query_group_id".into(), json!(-1002222222222i64));

        let previous = "最近找到2条合格广告";
        let result = resolve_contextual_command("删除", &mut metadata, previous);
        assert_eq!(result, "删除最近24小时的广告 群 -1002222222222");
    }

    #[test]
    fn test_terse_delete_without_context() {
        let mut metadata = HashMap::new();
        let previous = "无历史";
        let result = resolve_contextual_command("删除", &mut metadata, previous);
        assert_eq!(result, "删除");
    }

    #[test]
    fn test_terse_delete_without_marker() {
        let mut metadata = HashMap::new();
        metadata.insert("last_ad_query_group_id".into(), json!(-1002222222222i64));

        let previous = "其他内容";
        let result = resolve_contextual_command("删除", &mut metadata, previous);
        assert_eq!(result, "删除");
    }

    #[test]
    fn test_ordinal_chinese_numbers() {
        let mut metadata = HashMap::new();
        let previous = "群组：\n1. A (-1002222222221)\n2. B (-1002222222222)\n3. C (-1002222222223)\n频道：";

        let result = resolve_contextual_command("第二个", &mut metadata, previous);
        assert_eq!(result, "查看群 -1002222222222 最近群消息");
        assert_eq!(
            metadata.get("active_group_id").and_then(|v| v.as_i64()),
            Some(-1002222222222)
        );

        // 汉字"第三个"、"第十个"边界
        let mut metadata2 = HashMap::new();
        let result2 = resolve_contextual_command("第三个", &mut metadata2, previous);
        assert_eq!(result2, "查看群 -1002222222223 最近群消息");
    }

    #[test]
    fn test_ordinal_out_of_range() {
        let mut metadata = HashMap::new();
        let previous = "群组：\n1. A (-1002222222221)\n2. B (-1002222222222)\n频道：";

        let result = resolve_contextual_command("第五个", &mut metadata, previous);
        assert_eq!(result, "第五个");
        assert!(metadata.get("active_group_id").is_none());
    }

    #[test]
    fn test_ordinal_with_keyword() {
        let mut metadata = HashMap::new();
        let previous = "群组：\n1. A (-1002222222221)\n2. B (-1002222222222)\n频道：";

        let result = resolve_contextual_command("第2个 群消息", &mut metadata, previous);
        assert_eq!(result, "查看群 -1002222222222 最近群消息");
    }

    #[test]
    fn test_ordinal_short_ids_not_matched_python_parity() {
        // Python regex r"\((-?\d{5,})\)" 要求至少 5 位数字 — 短ID不匹配（保持 bit-level parity）
        let mut metadata = HashMap::new();
        let previous = "群组：\n1. A (-1001)\n2. B (-1002)\n频道：";

        let result = resolve_contextual_command("第二个", &mut metadata, previous);
        assert_eq!(result, "第二个");
        assert!(metadata.get("active_group_id").is_none());
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("删 除"), "删除");
        assert_eq!(normalize("第 一 个"), "第一个");
        assert_eq!(normalize("CANCEL"), "cancel");
    }

    #[test]
    fn test_chinese_to_number() {
        assert_eq!(chinese_to_number("一"), Some(1));
        assert_eq!(chinese_to_number("十"), Some(10));
        assert_eq!(chinese_to_number("11"), None);
    }

    #[test]
    fn test_explicit_group_sets_active_group_id() {
        let mut metadata = HashMap::new();
        let result = resolve_contextual_command("查看群 -1002222222222 的消息", &mut metadata, "");
        // 显式群ID被记录为 active_group_id（side-effect），文本不展开
        assert_eq!(
            metadata.get("active_group_id").and_then(|v| v.as_i64()),
            Some(-1002222222222)
        );
        assert_eq!(result, "查看群 -1002222222222 的消息");
    }

    #[test]
    fn test_terse_delete_zero_group_id_no_suffix() {
        // Python: str(0 or "") == "" → 无群后缀
        let mut metadata = HashMap::new();
        metadata.insert("last_ad_query_group_id".into(), json!(0i64));
        let previous = "共 3 条广告记录";
        let result = resolve_contextual_command("删除", &mut metadata, previous);
        assert_eq!(result, "删除最近24小时的广告");
    }
}
